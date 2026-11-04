//! An agent's recap: what its session said, read from its transcript (`cockpit_core::transcripts`, any agent), summed up
//! by an agent run headless and kept out of that agent's history, one at a time per session in the background. Cached
//! per session with the transcript's size and time, current while the transcript has not changed.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, UNIX_EPOCH};

use cockpit_core::agent::{self, Agent};
use cockpit_core::files;
use cockpit_core::transcripts::{self, Role, Transcripts};
use serde_json::{Value, json};

use crate::term::output;

static RUNNING: Mutex<Option<HashSet<String>>> = Mutex::new(None);
static ERRORS: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

fn dir() -> PathBuf {
    files::cache().join("agents/recaps")
}

/// The session's transcript, in whichever agent's format holds it.
pub fn transcript(session: &str, cwd: &str) -> Option<PathBuf> {
    transcripts::locate(session, cwd, None).map(|(_, p)| p)
}

/// The most of a session a recap reads: its first prompt, then its latest messages up to this.
const DIGEST_CAP: usize = 24 * 1024;

/// The session as the recap reads it: who said what, the task first and then the latest exchanges, oldest first.
pub fn digest(kind: Transcripts, path: &Path) -> Option<String> {
    let t = transcripts::read(kind, path)?;
    let line = |m: &transcripts::Message| format!("{}: {}", if m.role == Role::User { "Person" } else { "Agent" }, m.text.trim());
    let first = t.messages.iter().position(|m| m.role == Role::User)?;
    let head = line(&t.messages[first]);
    let mut tail: Vec<String> = Vec::new();
    let mut size = head.len();
    for m in t.messages[first + 1..].iter().rev() {
        let l = line(m);
        if size + l.len() > DIGEST_CAP {
            break;
        }
        size += l.len() + 2;
        tail.push(l);
    }
    tail.reverse();
    let gap = if tail.len() + 1 < t.messages.len() - first { "\n\n[…]" } else { "" };
    Some(format!("{head}{gap}\n\n{}", tail.join("\n\n")).trim_end().to_string())
}

fn stamp(path: &Path) -> (u64, u64) {
    let Ok(m) = std::fs::metadata(path) else { return (0, 0) };
    let modified = m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64).unwrap_or(0);
    (m.len(), modified)
}

pub fn running(session: &str) -> bool {
    RUNNING.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|r| r.contains(session))
}

fn error(session: &str) -> Option<String> {
    ERRORS.lock().unwrap_or_else(|e| e.into_inner()).iter().rev().find(|(s, _)| s == session).map(|(_, e)| e.clone())
}

/// The cached recap as the view shows it: {text, at, fresh, running, error}, or null when none was asked yet.
pub fn shown(session: &str, transcript: Option<&Path>) -> Value {
    let cached = files::load(&dir().join(format!("{session}.json")));
    let (running, error) = (running(session), error(session));
    let Some(c) = cached else {
        return if running || error.is_some() { json!({"text": "", "at": null, "fresh": false, "running": running, "error": error}) } else { Value::Null };
    };
    let fresh = transcript.is_some_and(|t| {
        let (size, modified) = stamp(t);
        c["size"].as_u64() == Some(size) && c["modified"].as_u64() == Some(modified)
    });
    json!({"text": c["text"], "at": c["at"], "fresh": fresh, "running": running, "error": error})
}

/// Whether the cached recap still describes the transcript.
pub fn fresh(session: &str, transcript: &Path) -> bool {
    shown(session, Some(transcript))["fresh"] == true
}

/// Starts a recap in the background unless one runs, on `model` when given (else the session's own); `done` runs when
/// it ends (the view is built again).
pub fn start(session: &str, cwd: &str, transcript: PathBuf, model: Option<&'static str>, done: fn()) -> bool {
    {
        let mut guard = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
        let set = guard.get_or_insert_with(HashSet::new);
        if !set.insert(session.to_string()) {
            return false;
        }
    }
    let (session, cwd) = (session.to_string(), cwd.to_string());
    std::thread::spawn(move || {
        let result = run(&session, &cwd, &transcript, model);
        let mut errors = ERRORS.lock().unwrap_or_else(|e| e.into_inner());
        errors.retain(|(s, _)| *s != session);
        if let Err(e) = result {
            errors.push((session.clone(), e));
        }
        drop(errors);
        if let Some(set) = RUNNING.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            set.remove(&session);
        }
        done();
    });
    true
}

const ASK: &str = "Below are the messages of a coding agent's session with a person (tool calls and their output left out). \
Recap it in two or three sentences for the person coming back to it: what was done, where it stands, and what is left or \
what the agent waits for. Plain prose, no preamble, no heading.";

/// The agent that writes the recap: the session's own when it runs headless, else the default one, with what keeps
/// the run out of its history.
fn runner(kind: Transcripts, prompt: &str, model: Option<&str>) -> Result<Vec<String>, String> {
    let own = agent::get(kind.agent()).filter(Agent::can_run);
    let a = own.unwrap_or_else(agent::default);
    let model = model.filter(|_| a.id == "claude");
    let mut argv = a.run_argv(prompt, model).ok_or_else(|| format!("{} has no headless mode for a recap", a.name))?;
    match a.id.as_str() {
        "claude" => argv.extend(["--no-session-persistence", "--tools", "", "--strict-mcp-config"].map(str::to_string)),
        "codex" => argv.extend(["--ephemeral", "--skip-git-repo-check"].map(str::to_string)),
        _ => {}
    }
    Ok(argv)
}

fn run(session: &str, cwd: &str, transcript: &Path, model: Option<&str>) -> Result<(), String> {
    let (size, modified) = stamp(transcript);
    let kind = transcripts::kind_of(transcript).unwrap_or(Transcripts::Claude);
    let digest = digest(kind, transcript).ok_or("nothing to recap yet")?;
    let argv = runner(kind, &format!("{ASK}\n\n<session>\n{digest}\n</session>"), model)?;
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    if Path::new(cwd).is_dir() {
        cmd.current_dir(cwd);
    }
    // Without the terminal's variables, the run is not reported to Herdr as an agent of that terminal.
    for (k, _) in std::env::vars() {
        if k.starts_with("HERDR_") || k.starts_with("HC_") || k == "CLAUDECODE" {
            cmd.env_remove(k);
        }
    }
    let child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| format!("{}: {e}", argv[0]))?;
    let out = wait(child, Duration::from_secs(180))?;
    let out = String::from_utf8_lossy(out.trim_ascii()).into_owned();
    // An agent printing JSON (claude --output-format json) has its answer under `result`.
    let reply: Value = serde_json::from_str(&out).unwrap_or(Value::Null);
    let said = reply["result"].as_str().map(str::to_string).unwrap_or(out);
    let text = said.split_whitespace().collect::<Vec<_>>().join(" ");
    if reply["is_error"] == true || text.is_empty() {
        return Err(if text.is_empty() { "the agent gave an empty recap".into() } else { text });
    }
    let _ = std::fs::create_dir_all(dir());
    let at = cockpit_core::seen::now_ms();
    files::write(
        &dir().join(format!("{session}.json")),
        &json!({"session": session, "text": text, "at": at, "size": size, "modified": modified, "cost": reply["total_cost_usd"]}),
    )
    .map_err(|e| e.to_string())
}

fn wait(child: std::process::Child, limit: Duration) -> Result<Vec<u8>, String> {
    let pid = child.id();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    match rx.recv_timeout(limit) {
        Ok(Ok(out)) if !out.stdout.is_empty() => Ok(out.stdout),
        Ok(Ok(out)) => Err(String::from_utf8_lossy(&out.stderr).lines().next().unwrap_or("the agent failed").to_string()),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => {
            let _ = output(&["kill".into(), "-9".into(), pid.to_string()], 2);
            Err("the recap took too long".into())
        }
    }
}

/// A recap written by hand (the demo world): what `shown` reads afterwards.
pub fn put(session: &str, text: &str, at: u64) {
    let _ = std::fs::create_dir_all(dir());
    let _ = files::write(&dir().join(format!("{session}.json")), &json!({"session": session, "text": text, "at": at, "size": 0, "modified": 0}));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_digest_keeps_the_task_and_the_latest_exchanges() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c-1.jsonl");
        let line = |kind: &str, text: &str| json!({"type": kind, "message": {"role": kind, "content": [{"type": "text", "text": text}]}}).to_string();
        let mut lines = vec![line("user", "Enforce the daily quota for ENG-412")];
        for i in 0..400 {
            lines.push(line("assistant", &format!("step {i}: {}", "x".repeat(100))));
        }
        lines.push(line("assistant", "Done: the limiter counts per day."));
        std::fs::write(&path, lines.join("\n")).unwrap();
        let d = digest(Transcripts::Claude, &path).unwrap();
        assert!(d.starts_with("Person: Enforce the daily quota for ENG-412\n\n[…]\n\nAgent: step "), "{}", &d[..120]);
        assert!(d.ends_with("Agent: Done: the limiter counts per day."));
        assert!(d.len() <= DIGEST_CAP + 200, "{}", d.len());
        assert!(!d.contains("step 0:"));
    }
}
