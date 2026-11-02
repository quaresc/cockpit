//! An agent's recap: Claude Code's own `/recap`, asked of an unsaved fork of its session so the conversation is only read
//! (after herdr-recap, MIT), one at a time per session in the background. Cached per session with the transcript's size
//! and time, current while the transcript has not changed.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, UNIX_EPOCH};

use cockpit_core::files;
use serde_json::{Value, json};

use crate::term::output;

static RUNNING: Mutex<Option<HashSet<String>>> = Mutex::new(None);
static ERRORS: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

fn dir() -> PathBuf {
    files::cache().join("agents/recaps")
}

/// The session's transcript: under ~/.claude/projects/<the cwd, non-alphanumerics as "-">/, else wherever it is.
pub fn transcript(session: &str, cwd: &str) -> Option<PathBuf> {
    let projects = files::home().join(".claude/projects");
    let name = format!("{session}.jsonl");
    let folder: String = cwd.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let direct = projects.join(folder).join(&name);
    if direct.exists() {
        return Some(direct);
    }
    std::fs::read_dir(&projects).ok()?.flatten().map(|e| e.path().join(&name)).find(|p| p.exists())
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

fn claude() -> String {
    let local = files::home().join(".local/bin/claude");
    std::env::var("COCKPIT_CLAUDE").ok().filter(|c| !c.is_empty()).unwrap_or_else(|| if local.exists() { local.to_string_lossy().into_owned() } else { "claude".into() })
}

fn run(session: &str, cwd: &str, transcript: &Path, model: Option<&str>) -> Result<(), String> {
    let (size, modified) = stamp(transcript);
    let mut cmd = Command::new(claude());
    cmd.args(["-p", "--resume", session, "--fork-session", "--no-session-persistence", "--output-format", "json"]);
    if let Some(model) = model {
        cmd.args(["--model", model]);
    }
    cmd.arg("/recap");
    if Path::new(cwd).is_dir() {
        cmd.current_dir(cwd);
    }
    // Without the terminal's variables, the fork is not reported to Herdr as an agent of that terminal.
    for (k, _) in std::env::vars() {
        if k.starts_with("HERDR_") || k.starts_with("HC_") {
            cmd.env_remove(k);
        }
    }
    let child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| format!("claude: {e}"))?;
    let out = wait(child, Duration::from_secs(180))?;
    let reply: Value = serde_json::from_slice(out.trim_ascii()).map_err(|_| "claude gave no recap".to_string())?;
    let text = reply["result"].as_str().unwrap_or_default().split_whitespace().collect::<Vec<_>>().join(" ");
    if reply["is_error"] == true || text.is_empty() || text.starts_with("Couldn't generate a recap") {
        return Err(if text.is_empty() { "claude gave an empty recap".into() } else { text });
    }
    if text.starts_with("Nothing to recap yet") {
        return Err("nothing to recap yet".into());
    }
    let _ = std::fs::create_dir_all(dir());
    let at = cockpit_core::seen::now_ms();
    files::write(&dir().join(format!("{session}.json")), &json!({"session": session, "text": text, "at": at, "size": size, "modified": modified, "cost": reply["total_cost_usd"]}))
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
        Ok(Ok(out)) => Err(String::from_utf8_lossy(&out.stderr).lines().next().unwrap_or("claude failed").to_string()),
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
