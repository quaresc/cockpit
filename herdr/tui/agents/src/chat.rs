//! The `chat` view: one agent's conversation as a reader shows it, read from the transcript its terminal holds —
//! the messages in order, the tool calls between them, and the question it is waiting on.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use cockpit_core::transcripts::{self, Detail, Follow, Role, Transcripts, Usage};
use serde_json::{Value, json};

/// The most recent messages a chat holds unless it is asked for more; `more` says there are older ones.
const WINDOW: usize = 400;
/// However much is asked for, a chat never reads past this.
const CEILING: usize = 20_000;
/// How many transcripts are followed at once: the chats open in a window, and a few just left.
const KEPT: usize = 8;

/// The transcripts chats follow, the last read first: each is read on from where it stopped, not again whole.
static FOLLOWED: Mutex<Vec<(PathBuf, Follow)>> = Mutex::new(Vec::new());

/// How the hub builds the chat view again, set once by the hub.
static GROWN: OnceLock<fn()> = OnceLock::new();

/// Whether the last chat built shows an agent in a terminal at work: the step it is on is read off its screen, which
/// moves while its transcript waits on a long call.
static ON_A_STEP: AtomicBool = AtomicBool::new(false);

/// Builds the chat view again whenever a transcript it follows grows, and every other second while it shows an agent's
/// step: a long turn writes for minutes while its agent's status, which is what moves the agents view, stays the same.
pub fn on_growth(touch: fn()) {
    if GROWN.set(touch).is_err() {
        return;
    }
    std::thread::spawn(|| {
        let mut stamps: HashMap<PathBuf, (u64, SystemTime)> = HashMap::new();
        let mut odd = false;
        loop {
            odd = !odd;
            std::thread::sleep(Duration::from_secs(1));
            let paths: Vec<PathBuf> = FOLLOWED.lock().unwrap_or_else(|e| e.into_inner()).iter().map(|(p, _)| p.clone()).collect();
            let grew = grown(&mut stamps, &paths);
            let step = odd && ON_A_STEP.load(Ordering::Relaxed);
            if (grew || step)
                && let Some(touch) = GROWN.get()
            {
                touch();
            }
        }
    });
}

/// Whether any of these files changed since the last look; a file seen for the first time has not.
fn grown(stamps: &mut HashMap<PathBuf, (u64, SystemTime)>, paths: &[PathBuf]) -> bool {
    let mut grew = false;
    for path in paths {
        let Ok(meta) = std::fs::metadata(path) else { continue };
        let now = (meta.len(), meta.modified().unwrap_or(SystemTime::UNIX_EPOCH));
        grew |= stamps.insert(path.clone(), now).is_some_and(|before| before != now);
    }
    stamps.retain(|p, _| paths.contains(p));
    grew
}

/// The folders the view reads: none of its own, it follows the agents poller.
pub fn dirs() -> Vec<PathBuf> {
    Vec::new()
}

/// One agent's conversation: `{agent, session: {id, cwd, branch, model, effort, fast, mode, focus, switch, repos, task, detail,
/// context, cost}, messages, question, more, next}`, for the agent `terminal` names (a handle, an address or a driven
/// session's key); empty when it names none the agents view lists. `limit` is how many of the latest messages to send, `WINDOW` by default; `agent` is the agents
/// view's row, so a chat needs no second subscription.
pub fn view(args: &Value, doc: &dyn Fn(&str) -> Value) -> Value {
    let agents = crate::gui::view(&json!({"frontend": args["frontend"].clone()}), doc);
    let rows = agents["agents"].as_array().cloned().unwrap_or_default();
    // The chat's subject, and only that: another agent's conversation is never shown in its place.
    let wanted = args["terminal"].as_str().filter(|t| !t.is_empty());
    let Some(row) = wanted.and_then(|t| rows.iter().find(|r| r["terminal"] == t || r["address"] == t)) else {
        ON_A_STEP.store(false, Ordering::Relaxed);
        return json!({"agent": Value::Null, "session": Value::Null, "messages": [], "question": Value::Null, "more": false, "next": Value::Null});
    };
    let session = row["session"].as_str().unwrap_or_default();
    let cwd = row["cwd"].as_str().unwrap_or_default();
    let found = (!session.is_empty()).then(|| transcripts::locate(session, cwd, None)).flatten();
    let window = args["limit"].as_u64().map(|n| (n as usize).clamp(1, CEILING)).unwrap_or(WINDOW);
    let (messages, more, usage, ran) = found.and_then(|(kind, path)| latest(kind, &path, window)).unwrap_or_default();
    let next = crate::next::of(row, &messages, &cockpit_core::files::config(), ran.model.as_deref(), GROWN.get().copied());
    // What its terminal shows now: the step it is on while it works, and its mode, which the transcript says a turn late.
    let screen = row["terminal"].as_str().filter(|_| row["driven"] != true).and_then(|t| crate::term::screen(t, 0));
    let shown = screen.map(|s| crate::prompt::activity(&s, &crate::prompt::Shapes::of(row["agent"].as_str().unwrap_or_default()))).unwrap_or_default();
    let working = row["status"] == "working";
    // What a click in the chat may change in its terminal: the agent's mode cycle and focus view, where measured.
    let program = row["program"].as_str().unwrap_or_default();
    let known = cockpit_core::agent::of_program(program).or_else(|| cockpit_core::agent::get(row["agent"].as_str().unwrap_or_default()));
    let switch = json!({
        "mode": row["driven"] != true && known.as_ref().is_some_and(|a| a.cycle.is_some()),
        "focus": row["driven"] != true && known.as_ref().is_some_and(|a| a.focus.is_some()),
    });
    ON_A_STEP.store(working && row["driven"] != true, Ordering::Relaxed);
    json!({
        "agent": row,
        "session": {
            "id": session, "cwd": cwd, "branch": row["branch"].clone(),
            "model": ran.model.map(Value::from).unwrap_or(Value::Null),
            "effort": ran.effort.map(Value::from).unwrap_or(Value::Null),
            "fast": ran.speed.map(|s| Value::from(s == "fast")).unwrap_or(Value::Null),
            "mode": shown.mode.or(ran.mode).map(Value::from).unwrap_or(Value::Null),
            "focus": ran.focus.map(Value::from).unwrap_or(Value::Null),
            "repos": repos(cwd, &messages),
            "switch": switch,
            "task": shown.task.filter(|_| working).map(Value::from).unwrap_or(Value::Null),
            "detail": shown.detail.filter(|_| working).map(Value::from).unwrap_or(Value::Null),
            "context": usage.context_percent(), "cost": usage.cost,
        },
        "messages": messages,
        "question": row["question"].clone(),
        "more": more,
        "next": next,
    })
}

/// The most repositories a chat names as worked in.
const REPOS: usize = 8;

/// The git repositories the agent works in, the latest touched first, as `[{path, name}]`: its folder's own, and those its
/// calls changed or went into (a file written, a `cd`, a path under its folder in a command), never one it only read.
fn repos(cwd: &str, messages: &[Value]) -> Value {
    let home = std::env::var("HOME").unwrap_or_default();
    let full = |p: &str| -> Option<PathBuf> {
        let p = p.trim_matches(|c: char| matches!(c, '"' | '\'' | '`' | ',' | ';' | ')' | '('));
        let p = match p.strip_prefix("~/") {
            Some(rest) if !home.is_empty() => format!("{home}/{rest}"),
            _ => p.to_string(),
        };
        p.starts_with('/').then(|| PathBuf::from(p))
    };
    let mut found: Vec<PathBuf> = Vec::new();
    let mut note = |top: Option<PathBuf>| {
        if let Some(top) = top {
            found.retain(|t| *t != top);
            found.push(top);
        }
    };
    note(toplevel(Path::new(cwd)));
    for m in messages {
        let Some(summary) = m["tool"]["summary"].as_str() else { continue };
        let name = m["tool"]["name"].as_str().unwrap_or_default();
        if matches!(name, "Edit" | "Write" | "MultiEdit" | "NotebookEdit") {
            note(full(summary).and_then(|p| toplevel(&p)));
            continue;
        }
        // Reading a project is no work in it.
        if matches!(name, "Read" | "Grep" | "Glob" | "LS" | "NotebookRead" | "WebFetch" | "WebSearch") {
            continue;
        }
        let words: Vec<&str> = summary.split_whitespace().collect();
        for (i, w) in words.iter().enumerate() {
            let Some(p) = full(w) else { continue };
            let went = i > 0 && words[i - 1] == "cd";
            if went || (!cwd.is_empty() && p.starts_with(cwd)) {
                note(toplevel(&p));
            }
        }
    }
    let list: Vec<Value> = found
        .iter()
        .rev()
        .take(REPOS)
        .map(|t| json!({"path": t.to_string_lossy(), "name": t.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()}))
        .collect();
    Value::from(list)
}

/// The top of the git repository `path` is in, found by its `.git` (a folder, or a worktree's file); None outside one.
fn toplevel(path: &Path) -> Option<PathBuf> {
    static SEEN: Mutex<Option<HashMap<PathBuf, Option<PathBuf>>>> = Mutex::new(None);
    if path.as_os_str().is_empty() {
        return None;
    }
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    let seen = seen.get_or_insert_with(HashMap::new);
    if let Some(top) = seen.get(path) {
        return top.clone();
    }
    let top = path.ancestors().find(|dir| dir.join(".git").exists()).filter(|dir| dir.parent().is_some()).map(Path::to_path_buf);
    if seen.len() > 4096 {
        seen.clear();
    }
    seen.insert(path.to_path_buf(), top.clone());
    top
}

/// What a transcript says its last turn ran with.
#[derive(Default)]
struct Ran {
    model: Option<String>,
    effort: Option<String>,
    speed: Option<String>,
    mode: Option<String>,
    focus: Option<bool>,
}

/// The latest `window` messages of a transcript, whether it holds older ones, its token use and what it ran with.
fn latest(kind: Transcripts, path: &Path, window: usize) -> Option<(Vec<Value>, bool, Usage, Ran)> {
    let mut followed = FOLLOWED.lock().unwrap_or_else(|e| e.into_inner());
    let mut entry = match followed.iter().position(|(p, _)| p == path) {
        Some(at) => followed.remove(at),
        None => (path.to_path_buf(), Follow::new(kind, path, Detail::Tools)),
    };
    let out = entry.1.read().map(|t| {
        let from = t.messages.len().saturating_sub(window);
        let ran = Ran { model: t.model.clone(), effort: t.effort.clone(), speed: t.speed.clone(), mode: t.mode.clone(), focus: t.focus };
        (t.messages[from..].iter().map(message).collect(), from > 0, t.usage.clone(), ran)
    });
    followed.insert(0, entry);
    followed.truncate(KEPT);
    out
}

/// A message as the chat draws it: words, or the tool call that stood there.
pub(crate) fn message(m: &transcripts::Message) -> Value {
    json!({
        "role": if m.role == Role::User { "user" } else { "assistant" },
        "at": m.at,
        "text": m.text,
        "tool": m.tool.as_ref().map(|t| json!({"name": t.name, "summary": t.summary, "ok": t.ok, "about": t.about})).unwrap_or(Value::Null),
    })
}

/// It has no actions of its own: a chat answers, replies and snoozes through the `agents` view.
pub fn act(_action: &str, _args: &[Value]) -> Option<Result<String, String>> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use cockpit_core::transcripts::{Message, Tool};

    #[test]
    fn a_message_carries_its_words_or_the_call_that_stood_there() {
        let said = Message { role: Role::User, text: "Fix the rounding".into(), at: "2026-11-03T10:00:00Z".into(), tool: None };
        assert_eq!(message(&said), json!({"role": "user", "at": "2026-11-03T10:00:00Z", "text": "Fix the rounding", "tool": null}));

        let ran = Message {
            role: Role::Assistant,
            text: String::new(),
            at: "2026-11-03T10:01:00Z".into(),
            tool: Some(Tool { name: "Bash".into(), summary: "mix test".into(), ok: Some(false), about: None }),
        };
        assert_eq!(
            message(&ran),
            json!({"role": "assistant", "at": "2026-11-03T10:01:00Z", "text": "", "tool": {"name": "Bash", "summary": "mix test", "ok": false, "about": null}})
        );
    }

    /// The view's build on a 5,000-message transcript: a whole read as it was, then a followed one, untouched and
    /// after a turn more. `cargo test -p cockpit-agents --release chat::tests::measure -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn measure_a_5000_message_chat() {
        use std::io::Write;
        use std::time::Instant;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("long.jsonl");
        let turn = |n: usize| {
            let at = format!("2026-11-03T10:{:02}:{:02}Z", (n / 60) % 60, n % 60);
            let call = |id: String, command: String| json!({"type": "assistant", "timestamp": at, "message": {"content": [{"type": "tool_use", "id": id, "name": "Bash", "input": {"command": command}}]}});
            let result = |id: String| json!({"type": "user", "timestamp": at, "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": id, "content": "ok"}]}});
            [
                json!({"type": "user", "cwd": "/r/api", "sessionId": "long", "timestamp": at, "message": {"role": "user", "content": format!("Turn {n}: make the daily quota configurable per plan, and run the quota tests when it builds.")}}),
                call(format!("a{n}"), format!("mix test test/api/quota_test.exs:{n}")),
                result(format!("a{n}")),
                call(format!("b{n}"), "rg daily_limit lib".into()),
                result(format!("b{n}")),
                json!({"type": "assistant", "timestamp": at, "message": {"content": [{"type": "text", "text": format!("Turn {n} done: the limit now reads from each plan's settings, with the current value as the default, and the quota tests pass.")}]}}),
            ]
            .map(|l| format!("{l}\n"))
            .concat()
        };
        std::fs::write(&path, (0..1250).map(turn).collect::<String>()).unwrap();
        let time = |what: &str, runs: u32, f: &mut dyn FnMut() -> usize| {
            let start = Instant::now();
            let mut n = 0;
            for _ in 0..runs {
                n = f();
            }
            println!("{what}: {:.2} ms ({n} messages sent)", start.elapsed().as_secs_f64() * 1000.0 / f64::from(runs));
        };
        time("whole read, as every build was", 5, &mut || {
            let t = transcripts::read_detail(Transcripts::Claude, &path, Detail::Tools).unwrap();
            assert_eq!(t.messages.len(), 5000);
            t.messages[t.messages.len() - WINDOW..].iter().map(message).count()
        });
        time("followed, first read", 1, &mut || latest(Transcripts::Claude, &path, WINDOW).unwrap().0.len());
        time("followed, nothing new", 20, &mut || latest(Transcripts::Claude, &path, WINDOW).unwrap().0.len());
        let mut n = 1250;
        time("followed, one turn more each time", 20, &mut || {
            let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
            file.write_all(turn(n).as_bytes()).unwrap();
            n += 1;
            latest(Transcripts::Claude, &path, WINDOW).unwrap().0.len()
        });
    }

    #[test]
    fn a_transcript_that_grows_has_its_chat_built_again_within_about_a_second() {
        use std::io::Write;
        use std::sync::atomic::{AtomicUsize, Ordering};
        static TOUCHED: AtomicUsize = AtomicUsize::new(0);
        on_growth(|| {
            TOUCHED.fetch_add(1, Ordering::SeqCst);
        });
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("g.jsonl");
        let line = |n: usize| {
            format!(
                "{}\n",
                json!({"type": "user", "cwd": "/r", "sessionId": "g", "timestamp": "2026-11-03T10:00:00Z", "message": {"role": "user", "content": format!("Turn {n}")}})
            )
        };
        std::fs::write(&path, line(0)).unwrap();
        assert!(latest(Transcripts::Claude, &path, 10).is_some(), "followed now");
        // The watcher's first look sees the file; nothing in the agents view changes after that.
        std::thread::sleep(Duration::from_millis(1500));
        let before = TOUCHED.load(Ordering::SeqCst);
        std::fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(line(1).as_bytes()).unwrap();
        let started = std::time::Instant::now();
        while TOUCHED.load(Ordering::SeqCst) == before && started.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(TOUCHED.load(Ordering::SeqCst) > before, "built again as the transcript grew");
        assert!(started.elapsed() < Duration::from_millis(2100), "within about a second: {:?}", started.elapsed());
    }

    #[test]
    fn a_followed_transcript_that_grows_is_seen_to_grow_and_one_seen_first_is_not() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        std::fs::write(&path, "{}\n").unwrap();
        let mut stamps = HashMap::new();
        assert!(!grown(&mut stamps, std::slice::from_ref(&path)), "seen for the first time");
        assert!(!grown(&mut stamps, std::slice::from_ref(&path)), "unchanged");
        std::fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(b"{}\n").unwrap();
        assert!(grown(&mut stamps, std::slice::from_ref(&path)), "a line more");
        assert!(!grown(&mut stamps, &[]), "no longer followed, no longer looked at");
        assert!(stamps.is_empty());
    }

    #[test]
    fn a_chat_sends_the_latest_messages_and_says_when_there_are_older_ones() {
        let longer = |n: usize| (0..n).map(|i| Message { role: Role::User, text: format!("m{i}"), at: String::new(), tool: None }).collect::<Vec<_>>();
        let cut = |all: &[Message], window: usize| (all.len() > window, all.len().saturating_sub(window));
        assert_eq!(cut(&longer(10), 400), (false, 0), "a short session is sent whole");
        assert_eq!(cut(&longer(500), 400), (true, 100), "a long one keeps its latest 400 and says so");
        assert_eq!(cut(&longer(500), 500), (false, 0), "asking for more reaches the start");
    }

    #[test]
    fn the_repositories_an_agent_works_in_are_the_ones_it_changed_or_went_into_the_latest_first() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("work");
        for repo in ["api", "web", "docs"] {
            std::fs::create_dir_all(root.join(repo).join(".git")).unwrap();
        }
        let at = |p: &str| root.join(p).to_string_lossy().into_owned();
        let call = |name: &str, summary: String| json!({"role": "assistant", "text": "", "tool": {"name": name, "summary": summary, "ok": true}});
        let messages = [
            call("Edit", at("api/lib/quota.ex")),
            call("Bash", format!("cd {} && mix test", at("web"))),
            call("Read", at("docs/README.md")),
            call("Bash", format!("grep -rn quota {}", at("api/lib"))),
        ];
        let names = |v: Value| v.as_array().unwrap().iter().map(|r| r["name"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        assert_eq!(
            names(repos(&root.to_string_lossy(), &messages)),
            ["api", "web"],
            "a file only read is no work; a command naming a path under the folder is"
        );
        assert_eq!(names(repos(&at("web"), &messages[..1])), ["api", "web"], "its own folder's repository, then the one it changed");
    }

    #[test]
    fn a_chat_for_an_agent_nobody_knows_still_answers() {
        let chat = view(&json!({"terminal": "t404", "frontend": "nothing"}), &|_| Value::Null);
        for key in ["agent", "session", "messages", "question", "more"] {
            assert!(chat.get(key).is_some(), "the view always has {key}");
        }
        assert_eq!(chat["messages"].as_array().map(Vec::len), Some(0));
        let none = view(&json!({"frontend": "nothing"}), &|_| Value::Null);
        assert_eq!(none["agent"], Value::Null, "no subject: no one else's conversation in its place");
    }
}
