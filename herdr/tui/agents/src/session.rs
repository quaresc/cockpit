//! Agent sessions cockpit drives itself. The agent runs under `--input-format stream-json`, so what it says, the tools
//! it calls and the permissions it asks for arrive as events instead of as a screen to read, and the person's words go
//! back the same way.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use cockpit_core::agent;
use cockpit_core::seen::now_ms;
use cockpit_core::transcripts::arg_line;
use serde_json::{Value, json};

/// What a live session holds: the reader thread fills it, the view reads it.
#[derive(Clone, Debug, Default)]
pub struct State {
    /// The agent's own id for the session, once it says: what `--resume` takes.
    pub session_id: String,
    pub agent: String,
    pub cwd: String,
    /// The conversation in the shape the chat draws: `{role, at, text, tool}`.
    pub messages: Vec<Value>,
    /// The slash commands the agent published, and republishes when they change.
    pub commands: Vec<Value>,
    /// The model the agent answers with, as it names it ("claude-opus-5-5").
    pub model: String,
    /// Where each tool call sits in `messages`, so its result can say whether it worked.
    calls: HashMap<String, usize>,
    pub cost: Option<f64>,
    pub usage: Value,
    pub rate_limit: Value,
    pub mode: String,
    /// True between a prompt and its result.
    pub working: bool,
    /// Why it is over, when it is.
    pub ended: Option<String>,
    /// What the agent is asking permission for, oldest first.
    pub pending: Vec<Value>,
    /// True between an interrupt and the result it cuts short, which is not a failure.
    pub interrupted: bool,
}

impl State {
    /// The session as the hub serves it.
    pub fn json(&self) -> Value {
        json!({
            "session": self.session_id, "agent": self.agent, "cwd": self.cwd, "mode": self.mode, "model": self.model,
            "messages": self.messages, "commands": self.commands, "cost": self.cost,
            "usage": self.usage, "rate_limit": self.rate_limit, "working": self.working, "ended": self.ended,
            "pending": self.pending,
        })
    }
}

pub struct Session {
    pub state: Arc<Mutex<State>>,
    stdin: Mutex<Option<ChildStdin>>,
    child: Mutex<Child>,
}

fn live() -> MutexGuard<'static, HashMap<String, Arc<Session>>> {
    static LIVE: OnceLock<Mutex<HashMap<String, Arc<Session>>>> = OnceLock::new();
    LIVE.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner())
}

/// What the person answered, by the id the host was given.
fn answers() -> MutexGuard<'static, HashMap<String, Value>> {
    static ANSWERS: OnceLock<Mutex<HashMap<String, Value>>> = OnceLock::new();
    ANSWERS.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner())
}

static TOUCH: OnceLock<fn()> = OnceLock::new();

/// The hub asks to be told when a session moves, so its view is built again.
pub fn on_change(touch: fn()) {
    let _ = TOUCH.set(touch);
}

fn moved() {
    if let Some(t) = TOUCH.get() {
        t();
    }
}

/// Starts an agent under the streaming protocol in `cwd`, and returns the key it is kept under.
pub fn start(agent_id: &str, cwd: &str, mode: &str) -> Result<String, String> {
    let found = agent::all().into_iter().find(|a| a.id == agent_id).ok_or_else(|| format!("no agent {agent_id:?}"))?;
    if found.transcripts != cockpit_core::transcripts::Transcripts::Claude {
        return Err(format!("{} has no streaming protocol cockpit speaks yet", found.name));
    }
    let key = format!("{}:{}", found.id, now_ms());
    // The agent asks cockpit for permission, over an MCP server that is this session's alone.
    let host = cockpit_core::files::cache().join("sessions").join(format!("{key}.json"));
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let config = json!({"mcpServers": {"cockpit-permission": {"command": me, "args": ["session", "host", &key]}}});
    cockpit_core::files::write(&host, &config).map_err(|e| e.to_string())?;
    let mut child = Command::new(found.path())
        .args(["-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose", "--permission-mode", mode])
        .args(["--mcp-config", &host.to_string_lossy()])
        .args(["--permission-prompt-tool", "mcp__cockpit-permission__permission"])
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("{}: {e}", found.path()))?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let stdin = child.stdin.take();
    let state = Arc::new(Mutex::new(State { agent: found.id.clone(), cwd: cwd.to_string(), mode: mode.to_string(), ..Default::default() }));
    let session = Arc::new(Session { state: Arc::clone(&state), stdin: Mutex::new(stdin), child: Mutex::new(child) });
    live().insert(key.clone(), Arc::clone(&session));
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            let Ok(event) = serde_json::from_str::<Value>(&line) else { continue };
            take(&mut state.lock().unwrap_or_else(|e| e.into_inner()), &event);
            moved();
        }
        let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
        s.working = false;
        s.ended.get_or_insert_with(|| "the agent stopped".into());
        moved();
    });
    Ok(key)
}

/// One event from the agent, folded into the state.
fn take(state: &mut State, event: &Value) {
    let at = chrono::Utc::now().to_rfc3339();
    match event["type"].as_str().unwrap_or_default() {
        "system" => match event["subtype"].as_str().unwrap_or_default() {
            "init" => {
                state.session_id = event["session_id"].as_str().unwrap_or_default().to_string();
                if let Some(cwd) = event["cwd"].as_str() {
                    state.cwd = cwd.to_string();
                }
                if let Some(model) = event["model"].as_str() {
                    state.model = model.to_string();
                }
                // The names the agent opens with. An agent run with an --mcp-config publishes no commands_changed, so
                // this is the only list some sessions ever get.
                let named: Vec<Value> = event["slash_commands"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|c| c.as_str())
                    .map(|name| json!({"name": name, "description": ""}))
                    .collect();
                if !named.is_empty() {
                    state.commands = named;
                }
            }
            "commands_changed" => {
                let sent = event["commands"].as_array().cloned().unwrap_or_default();
                if !sent.is_empty() {
                    state.commands = sent;
                }
            }
            _ => {}
        },
        "rate_limit_event" => state.rate_limit = event["rate_limit_info"].clone(),
        // The agent confirms a control request by answering it; the mode it reports back is the one that took.
        "control_response" => {
            if let Some(mode) = event["response"]["response"]["mode"].as_str() {
                state.mode = mode.to_string();
            }
            // Only a success names the model it took; an error names the one it kept, under a different key.
            if let Some(model) = event["response"]["response"]["model"].as_str() {
                state.model = model.to_string();
            }
            if event["response"]["response"]["still_queued"].is_array() {
                state.working = false;
            }
        }
        "assistant" => {
            state.working = true;
            for block in event["message"]["content"].as_array().into_iter().flatten() {
                match block["type"].as_str().unwrap_or_default() {
                    "text" => {
                        let text = block["text"].as_str().unwrap_or_default();
                        if !text.trim().is_empty() {
                            state.messages.push(json!({"role": "assistant", "at": at, "text": text, "tool": null}));
                        }
                    }
                    "tool_use" => {
                        let name = block["name"].as_str().unwrap_or_default();
                        let tool = json!({"name": name, "summary": arg_line(&block["input"]), "ok": null});
                        if let Some(id) = block["id"].as_str() {
                            state.calls.insert(id.to_string(), state.messages.len());
                        }
                        state.messages.push(json!({"role": "assistant", "at": at, "text": "", "tool": tool}));
                    }
                    _ => {}
                }
            }
        }
        "user" => {
            for block in event["message"]["content"].as_array().into_iter().flatten() {
                match block["type"].as_str().unwrap_or_default() {
                    "text" => {
                        let text = block["text"].as_str().unwrap_or_default();
                        if !text.trim().is_empty() {
                            state.messages.push(json!({"role": "user", "at": at, "text": text, "tool": null}));
                        }
                    }
                    "tool_result" => {
                        let ok = block["is_error"] != true;
                        if let Some(id) = block["tool_use_id"].as_str() {
                            // The call is over, however it went: an ask still waiting on it is one nobody can answer.
                            state.pending.retain(|p| p["tool_use_id"] != id);
                            if let Some(at) = state.calls.get(id).copied()
                                && let Some(m) = state.messages.get_mut(at)
                            {
                                m["tool"]["ok"] = json!(ok);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        "result" => {
            state.working = false;
            state.cost = event["total_cost_usd"].as_f64().or(state.cost);
            state.usage = event["usage"].clone();
            // A turn that failed says why in `result` as well as in its words, so this records it without saying it twice.
            // An interrupted turn reports the same error and is not one: the session takes the next prompt.
            let interrupted = std::mem::take(&mut state.interrupted);
            if event["is_error"] == true && !interrupted {
                state.ended = Some(event["result"].as_str().unwrap_or("the turn failed").to_string());
            }
        }
        _ => {}
    }
}

/// The permission modes a one-key cycle offers; the rest are set on purpose or not at all.
pub const CYCLE: [&str; 3] = ["default", "acceptEdits", "plan"];

/// The mode after this one, in the cycle.
pub fn next_mode(now: &str) -> &'static str {
    let at = CYCLE.iter().position(|m| *m == now).unwrap_or(0);
    CYCLE[(at + 1) % CYCLE.len()]
}

/// A control request down the same pipe the words go: the agent answers it without asking the model.
pub fn control(key: &str, request: Value) -> Result<(), String> {
    let session = live().get(key).cloned().ok_or("no such session")?;
    if request["subtype"] == "interrupt" {
        session.state.lock().unwrap_or_else(|e| e.into_inner()).interrupted = true;
    }
    let line = json!({"type": "control_request", "request_id": format!("c{}", now_ms()), "request": request});
    let mut held = session.stdin.lock().unwrap_or_else(|e| e.into_inner());
    let stdin = held.as_mut().ok_or("the session takes no more")?;
    writeln!(stdin, "{line}").map_err(|e| e.to_string())?;
    stdin.flush().map_err(|e| e.to_string())
}

/// The person's words, as the protocol takes them.
pub fn send(key: &str, text: &str) -> Result<(), String> {
    let session = live().get(key).cloned().ok_or("no such session")?;
    let line = json!({"type": "user", "message": {"role": "user", "content": [{"type": "text", "text": text}]}});
    let mut held = session.stdin.lock().unwrap_or_else(|e| e.into_inner());
    let stdin = held.as_mut().ok_or("the session takes no more")?;
    writeln!(stdin, "{line}").map_err(|e| e.to_string())?;
    stdin.flush().map_err(|e| e.to_string())?;
    let mut state = session.state.lock().unwrap_or_else(|e| e.into_inner());
    state.messages.push(json!({"role": "user", "at": chrono::Utc::now().to_rfc3339(), "text": text, "tool": null}));
    state.working = true;
    drop(state);
    moved();
    Ok(())
}

/// Ends a session and forgets it.
pub fn stop(key: &str) {
    let Some(session) = live().remove(key) else { return };
    session.stdin.lock().unwrap_or_else(|e| e.into_inner()).take();
    let _ = session.child.lock().unwrap_or_else(|e| e.into_inner()).kill();
    moved();
}

/// One session as the hub serves it.
pub fn snapshot(key: &str) -> Option<Value> {
    let session = live().get(key).cloned()?;
    let state = session.state.lock().unwrap_or_else(|e| e.into_inner());
    let mut out = state.json();
    out["key"] = json!(key);
    Some(out)
}

/// Every live session, newest first.
pub fn keys() -> Vec<String> {
    let mut all: Vec<String> = live().keys().cloned().collect();
    all.sort();
    all.reverse();
    all
}

// ---------------------------------------------------------------------------------------------------------------------
// The hub's view

/// The folders the view reads: none of its own, the sessions tell it when they move.
pub fn dirs() -> Vec<std::path::PathBuf> {
    Vec::new()
}

/// Every live session, or the one `key` names: `{sessions: [...], session: {...}}`.
pub fn view(args: &Value, _doc: &dyn Fn(&str) -> Value) -> Value {
    let all: Vec<Value> = keys().iter().filter_map(|k| snapshot(k)).collect();
    let wanted = args["key"].as_str().filter(|k| !k.is_empty());
    let one = match wanted {
        Some(k) => all.iter().find(|s| s["key"] == k).cloned(),
        None => all.first().cloned(),
    };
    json!({"sessions": all, "session": one})
}

/// `start {agent, cwd, mode}`, `send {key, text}`, `stop {key}`.
pub fn act(action: &str, args: &[Value]) -> Option<Result<String, String>> {
    let arg = args.first().cloned().unwrap_or(Value::Null);
    let key = arg["key"].as_str().unwrap_or_default().to_string();
    Some(match action {
        "start" => {
            let agent = arg["agent"].as_str().unwrap_or("claude");
            let cwd = arg["cwd"].as_str().unwrap_or_default();
            let mode = arg["mode"].as_str().unwrap_or("default");
            if cwd.is_empty() {
                return Some(Err("start where? pass {cwd}".into()));
            }
            start(agent, cwd, mode).map(|k| format!("started {k}"))
        }
        "send" => {
            let text = arg["text"].as_str().unwrap_or_default().trim().to_string();
            if text.is_empty() {
                return Some(Err("nothing to send".into()));
            }
            send(&key, &text).map(|()| String::new())
        }
        "stop" => {
            stop(&key);
            Ok("the session is over".into())
        }
        // The permission host asks on the agent's behalf; the note it gets back is the id to wait on.
        "mode" => {
            let want = arg["mode"].as_str().map(str::to_string).unwrap_or_else(|| {
                let now = snapshot(&key).map(|s| s["mode"].as_str().unwrap_or("default").to_string()).unwrap_or_default();
                next_mode(&now).to_string()
            });
            control(&key, json!({"subtype": "set_permission_mode", "mode": want.clone()})).map(|()| want)
        }
        "model" => {
            let want = arg["model"].as_str().unwrap_or_default().to_string();
            if want.is_empty() {
                return Some(Err("which model? pass {model}".into()));
            }
            control(&key, json!({"subtype": "set_model", "model": want.clone()})).map(|()| want)
        }
        "interrupt" => control(&key, json!({"subtype": "interrupt"})).map(|()| "stopped".into()),
        "ask" => {
            let Some(session) = live().get(&key).cloned() else { return Some(Err("no such session".into())) };
            let id = format!("p{}", now_ms());
            let ask = json!({
                "id": id, "at": now_ms(),
                "tool_name": arg["tool_name"].clone(), "input": arg["input"].clone(), "tool_use_id": arg["tool_use_id"].clone(),
            });
            session.state.lock().unwrap_or_else(|e| e.into_inner()).pending.push(ask);
            moved();
            Ok(id)
        }
        // Empty while nobody has answered: the host asks again in a moment.
        "decision" => {
            let id = arg["id"].as_str().unwrap_or_default();
            Ok(answers().get(id).map(|a| a.to_string()).unwrap_or_default())
        }
        "answer" => {
            let id = arg["id"].as_str().unwrap_or_default().to_string();
            let allow = arg["allow"] == true;
            // An allow carries the arguments back unchanged, so they are taken from the ask rather than from the caller.
            let mut input = arg["input"].clone();
            for session in live().values() {
                let mut state = session.state.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(ask) = state.pending.iter().find(|p| p["id"] == id.as_str()) {
                    input = ask["input"].clone();
                }
                state.pending.retain(|p| p["id"] != id.as_str());
            }
            let answer = match allow {
                true => json!({"behavior": "allow", "updatedInput": input}),
                false => json!({"behavior": "deny", "message": arg["why"].as_str().unwrap_or("not this time")}),
            };
            answers().insert(id.clone(), answer);
            moved();
            Ok(if allow { "allowed".into() } else { "denied".into() })
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fold(events: &[Value]) -> State {
        let mut state = State::default();
        for e in events {
            take(&mut state, e);
        }
        state
    }

    #[test]
    fn a_session_with_no_commands_changed_still_knows_the_slash_commands() {
        let state = fold(&[json!({"type": "system", "subtype": "init", "slash_commands": ["review", "ship"]})]);
        assert_eq!(
            state.commands.iter().map(|c| c["name"].as_str().unwrap_or("")).collect::<Vec<_>>(),
            ["review", "ship"],
            "an agent run with an --mcp-config publishes them only on init"
        );
    }

    #[test]
    fn the_stream_becomes_the_conversation_the_chat_draws() {
        let state = fold(&[
            json!({"type": "system", "subtype": "init", "session_id": "s-1", "cwd": "/r/api"}),
            json!({"type": "system", "subtype": "commands_changed", "commands": [{"name": "review", "description": "Review it"}]}),
            json!({"type": "assistant", "message": {"content": [{"type": "thinking", "thinking": "hm"}, {"type": "text", "text": "Reading it."}]}}),
            json!({"type": "assistant", "message": {"content": [{"type": "tool_use", "id": "t1", "name": "Bash", "input": {"command": "mix test"}}]}}),
            json!({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": "t1", "is_error": true}]}}),
            json!({"type": "rate_limit_event", "rate_limit_info": {"rateLimitType": "five_hour"}}),
            json!({"type": "result", "total_cost_usd": 0.25, "usage": {"output_tokens": 9}}),
        ]);
        assert_eq!((state.session_id.as_str(), state.cwd.as_str()), ("s-1", "/r/api"));
        assert_eq!(state.commands.len(), 1, "the agent's own commands are kept");
        assert_eq!(
            state.messages.iter().map(|m| (m["role"].as_str().unwrap_or(""), m["text"].as_str().unwrap_or(""), m["tool"]["name"].as_str())).collect::<Vec<_>>(),
            [("assistant", "Reading it.", None), ("assistant", "", Some("Bash"))],
            "thinking is left out, words and calls are kept in order"
        );
        assert_eq!(state.messages[1]["tool"]["summary"], "mix test");
        assert_eq!(state.messages[1]["tool"]["ok"], json!(false), "the result says the call failed");
        assert_eq!((state.working, state.cost), (false, Some(0.25)));
        assert_eq!(state.rate_limit["rateLimitType"], "five_hour");
        assert!(state.ended.is_none());
    }

    #[test]
    fn an_ask_whose_call_came_back_stops_waiting() {
        let mut state = State::default();
        take(
            &mut state,
            &json!({"type": "assistant", "message": {"content": [{"type": "tool_use", "id": "t1", "name": "Write", "input": {"file_path": "/r/x"}}]}}),
        );
        state.pending.push(json!({"id": "p1", "tool_use_id": "t1", "tool_name": "Write"}));
        take(&mut state, &json!({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": "t1", "is_error": true}]}}));
        assert!(state.pending.is_empty(), "the agent gave up on it, so the card goes with it");
    }

    #[test]
    fn an_interrupted_turn_leaves_the_session_open() {
        let mut state = State { interrupted: true, ..Default::default() };
        take(&mut state, &json!({"type": "result", "is_error": true, "subtype": "error_during_execution"}));
        assert_eq!(state.ended, None, "the person stopped the turn, the agent takes the next prompt");
        take(&mut state, &json!({"type": "result", "is_error": true, "result": "credit balance too low"}));
        assert_eq!(state.ended.as_deref(), Some("credit balance too low"), "a real failure still ends it");
    }

    #[test]
    fn the_model_is_the_one_the_agent_settled_on() {
        let state = fold(&[
            json!({"type": "system", "subtype": "init", "model": "claude-opus-5-5"}),
            json!({"type": "control_response", "response": {"subtype": "success", "request_id": "c1", "response": {"model": "claude-sonnet-5-5"}}}),
        ]);
        assert_eq!(state.model, "claude-sonnet-5-5");
    }

    #[test]
    fn the_mode_cycles_and_a_control_answer_is_believed() {
        assert_eq!(next_mode("default"), "acceptEdits");
        assert_eq!(next_mode("acceptEdits"), "plan");
        assert_eq!(next_mode("plan"), "default");
        assert_eq!(next_mode("bypassPermissions"), "acceptEdits", "a mode outside the cycle starts it again");
        let state = fold(&[json!({"type": "control_response", "response": {"subtype": "success", "request_id": "c1", "response": {"mode": "plan"}}})]);
        assert_eq!(state.mode, "plan");
    }

    #[test]
    fn a_turn_that_fails_says_why_without_saying_it_twice() {
        let state = fold(&[
            json!({"type": "assistant", "message": {"content": [{"type": "text", "text": "Not logged in"}]}}),
            json!({"type": "result", "is_error": true, "result": "Not logged in"}),
        ]);
        assert_eq!(state.messages.len(), 1, "the words are already in the stream");
        assert_eq!(state.ended.as_deref(), Some("Not logged in"));
    }
}
