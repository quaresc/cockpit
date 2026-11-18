//! An agent's recap: what its session said, read from its transcript (`cockpit_core::transcripts`, any agent), summed up
//! by an agent run headless and kept out of that agent's history, one at a time per session in the background. Cached
//! per session with the transcript's size and time, current while the transcript has not changed.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, UNIX_EPOCH};

use cockpit_core::agent::{self, Agent, Headless};
use cockpit_core::files;
use cockpit_core::run;
use cockpit_core::transcripts::{self, Role, Transcripts};
use serde_json::{Value, json};

static RUNNING: Mutex<Option<HashSet<String>>> = Mutex::new(None);
static ERRORS: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

fn dir() -> PathBuf {
    files::cache().join("agents/recaps")
}

/// Where a session's recap is kept, its id made safe as a file name.
fn kept_at(dir: &Path, session: &str) -> PathBuf {
    dir.join(format!("{}.json", files::file_key(session)))
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
    let cached = files::load(&kept_at(&dir(), session));
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

/// Selects a verified text-only runner, preferring the session's own agent and then an installed safe fallback.
fn runner(kind: Transcripts, prompt: &str, model: Option<&str>) -> Result<Vec<String>, String> {
    let mut candidates = vec![agent::get(kind.agent()), Some(agent::default())];
    candidates.extend(agent::all().into_iter().map(Some));
    let a = candidates
        .into_iter()
        .flatten()
        .find(|a| a.installed() && !a.aside().is_empty())
        .ok_or("Recaps need an installed agent that can answer with no tools, such as Claude Code")?;
    runner_for(&a, prompt, model)
}

fn runner_for(a: &Agent, prompt: &str, model: Option<&str>) -> Result<Vec<String>, String> {
    if a.aside().is_empty() {
        return Err(format!("{} has no verified text-only isolation for recaps", a.name));
    }
    a.headless(prompt, &Headless { model, aside: true, ..Default::default() }).ok_or_else(|| format!("{} has no headless mode for a recap", a.name))
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
    // The recap belongs to no terminal or parent agent session (`cockpit_core::run`).
    let out = run::output(&mut cmd, Duration::from_secs(180), 1 << 20, &Default::default())
        .map_err(|e| if e == "it took too long" { "the recap took too long".to_string() } else { format!("{}: {e}", argv[0]) })?;
    if !out.status.success() || out.stdout.is_empty() {
        return Err(String::from_utf8_lossy(&out.stderr).lines().next().unwrap_or("the agent failed").to_string());
    }
    let out = String::from_utf8_lossy(out.stdout.trim_ascii()).into_owned();
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
        &kept_at(&dir(), session),
        &json!({"session": session, "text": text, "at": at, "size": size, "modified": modified, "cost": reply["total_cost_usd"]}),
    )
    .map_err(|e| e.to_string())
}

/// A recap written by hand (the demo world): what `shown` reads afterwards.
pub fn put(session: &str, text: &str, at: u64) {
    let _ = std::fs::create_dir_all(dir());
    let _ = files::write(&kept_at(&dir(), session), &json!({"session": session, "text": text, "at": at, "size": 0, "modified": 0}));
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

    #[test]
    fn a_recap_is_kept_inside_its_folder_whatever_the_session_says() {
        let dir = Path::new("/cache/recaps");
        assert_eq!(kept_at(dir, "c-1"), dir.join("c-1.json"));
        assert_eq!(kept_at(dir, "../../.ssh/config"), dir.join("-------ssh-config.json"));
        assert_eq!(kept_at(dir, "a/b"), dir.join("a-b.json"));
        assert_eq!(kept_at(dir, ""), dir.join("_.json"));
    }

    #[test]
    fn recaps_require_verified_text_only_isolation() {
        let row = |id: &str| agent::builtin().into_iter().find(|a| a.id == id).unwrap();
        for id in ["codex", "pi", "gemini", "omp"] {
            assert!(runner_for(&row(id), "Recap.", None).unwrap_err().contains("text-only"));
        }
        let mut claude = row("claude");
        let argv = runner_for(&claude, "Recap.", Some("haiku")).unwrap();
        assert!(argv.windows(2).any(|w| w == ["--tools", ""]) && argv.contains(&"--safe-mode".into()));
        assert!(argv.windows(2).any(|w| w == ["--model", "haiku"]));
        // The prompt, the model, then the isolation: the command line every text-only run takes.
        let isolation = ["--no-session-persistence", "--setting-sources", "", "--safe-mode", "--tools", "", "--strict-mcp-config"];
        let rest = ["--disable-slash-commands", "--no-chrome", "--permission-mode", "plan", "--max-turns", "1"];
        assert_eq!(argv[1..], [&["-p", "Recap.", "--model", "haiku"][..], &isolation[..], &rest[..]].concat());
        claude.run = vec!["{bin}".into(), "{prompt}".into()];
        assert!(runner_for(&claude, "Recap.", None).is_err(), "overridden templates cannot inherit isolation");
    }
}
