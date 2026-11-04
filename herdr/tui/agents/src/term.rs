//! The agent terminals of both frontends, and what is done to one: its screen read, keys pressed, text pasted; a handle
//! is `app:t3` for a Herdr app terminal, a herdr pane id (`w9:p1`) for the TUI. With COCKPIT_AGENTS_DEMO naming a JSON
//! file, the terminals are that file's made-up ones (the sandbox, the screenshots), answered in memory.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::channel;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use cockpit_core::agent::{self, Agent};
use cockpit_core::files;
use cockpit_core::frontend::{self, APP};
use cockpit_core::transcripts::{self, Transcripts, Turn};
use serde_json::{Value, json};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Live {
    pub handle: String,
    pub address: String,
    pub frontend: &'static str,
    pub space_id: String,
    pub space_name: String,
    pub tab_id: String,
    pub tab_name: String,
    pub title: String,
    pub program: String,
    /// Its agent's id in `cockpit_core::agent` (`claude`, `codex`), empty for a program cockpit does not know.
    pub agent: String,
    /// working | blocked | done | idle
    pub status: String,
    pub cwd: String,
    pub session: Option<String>,
    /// The session's transcript, when found.
    pub transcript: Option<String>,
    /// The process its agent runs under (the terminal's), when the frontend says.
    pub pid: Option<u32>,
    pub context: Option<f64>,
    /// When its status began, when the source knows (the demo).
    pub since: Option<u64>,
}

/// A program's stdout when it exits 0 within `secs`, else None.
pub fn output(argv: &[String], secs: u64) -> Option<String> {
    // hc would tag what it sends with the terminal this runs in.
    let child =
        Command::new(&argv[0]).args(&argv[1..]).env_remove("HC_TERMINAL_ID").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let pid = child.id();
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    let Ok(Ok(out)) = rx.recv_timeout(Duration::from_secs(secs)) else {
        let _ = Command::new("kill").args(["-9", &pid.to_string()]).stderr(Stdio::null()).status();
        return None;
    };
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `hc <args>`'s stdout, 10 seconds at most.
pub fn hc_text(args: &[&str]) -> Option<String> {
    hc(args)
}

fn hc(args: &[&str]) -> Option<String> {
    let mut argv = vec![frontend::program("hc", "HC_BIN")];
    argv.extend(args.iter().map(|a| a.to_string()));
    output(&argv, 10)
}

fn herdr(args: &[&str]) -> Option<String> {
    let mut argv = vec![std::env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".into())];
    argv.extend(args.iter().map(|a| a.to_string()));
    output(&argv, 10)
}

fn json_of(text: Option<String>) -> Value {
    text.and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null)
}

// ---------------------------------------------------------------------------------------------------------------------
// Demo

pub fn demo_path() -> Option<PathBuf> {
    std::env::var_os("COCKPIT_AGENTS_DEMO").filter(|p| !p.is_empty()).map(PathBuf::from)
}

/// The demo's made-up world, read once and changed in memory by the actions.
pub fn demo() -> Option<MutexGuard<'static, Value>> {
    static DEMO: Mutex<Value> = Mutex::new(Value::Null);
    let path = demo_path()?;
    let mut world = DEMO.lock().unwrap_or_else(|e| e.into_inner());
    if world.is_null() {
        *world = files::load(&path).unwrap_or(json!({"agents": []}));
        // Ages are written in minutes before now, so the world looks the same whenever it starts.
        let now = cockpit_core::seen::now_ms();
        for a in world["agents"].as_array_mut().into_iter().flatten() {
            let mins = a["since_min"].as_f64().unwrap_or(1.0);
            a["since"] = json!(now.saturating_sub((mins * 60_000.0) as u64));
        }
    }
    Some(world)
}

fn demo_lives(world: &Value) -> Vec<Live> {
    world["agents"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|a| {
            let s = |k: &str| a[k].as_str().unwrap_or_default().to_string();
            Live {
                handle: s("terminal"),
                address: s("terminal").trim_start_matches(APP).to_string(),
                frontend: "app",
                space_id: a["space"]["id"].as_str().unwrap_or_default().to_string(),
                space_name: a["space"]["name"].as_str().unwrap_or_default().to_string(),
                tab_id: a["tab"]["id"].as_str().unwrap_or_default().to_string(),
                tab_name: a["tab"]["name"].as_str().unwrap_or_default().to_string(),
                title: s("title"),
                program: a["program"].as_str().unwrap_or("claude").to_string(),
                agent: agent_id(a["program"].as_str().unwrap_or("claude")),
                status: s("status"),
                cwd: s("cwd"),
                session: a["session"].as_str().map(str::to_string),
                transcript: None,
                pid: None,
                context: a["context"].as_f64(),
                since: a["since"].as_u64(),
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------------------------------
// Listing

/// Every agent terminal: the Herdr app's when it runs, herdr's panes when it runs (or the demo's alone).
pub fn list(only: Option<&str>) -> Vec<Live> {
    if let Some(world) = demo() {
        return demo_lives(&world);
    }
    let mut out = Vec::new();
    if only != Some("tui") && frontend::app_home().join("herdr-app.sock").exists() {
        out.extend(app_list());
    }
    if only != Some("app") && tui_running() {
        out.extend(tui_list());
    }
    out
}

fn tui_running() -> bool {
    Command::new("pgrep").args(["-x", "herdr"]).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

fn app_list() -> Vec<Live> {
    let ls = json_of(output(&[frontend::program("herdr-app", "HERDR_APP_BIN"), "ls".to_string()], 10));
    let core = json_of(hc(&["ls", "--json"]));
    let in_core = |t: &str| core["terminals"].as_array().into_iter().flatten().find(|x| x["address"] == t).cloned().unwrap_or(Value::Null);
    let mut out = Vec::new();
    for space in ls["spaces"].as_array().into_iter().flatten() {
        for tab in space["tabs"].as_array().into_iter().flatten() {
            for t in tab["terminals"].as_array().into_iter().flatten() {
                let address = t["terminal"].as_str().unwrap_or_default().to_string();
                let core = in_core(&address);
                let cwd = core["cwd"].as_str().unwrap_or_default().to_string();
                // herdr-core's own word for the agent it runs, reported or not.
                let detected = core["agent"].as_str().filter(|a| !a.is_empty());
                let reported = t["status"].as_str().filter(|s| !s.is_empty());
                let program = t["app"].as_str().filter(|a| !a.is_empty()).or(detected).unwrap_or("agent").to_string();
                let agent = agent::of_program(&program);
                let handle = format!("{APP}{address}");
                // A terminal that reports no status is looked at on every poll (its title may say it works); an agent's
                // session is looked for again every LOOK_AGAIN.
                let found = if reported.is_none() || agent.is_some() {
                    app_session(&handle, &address, agent.as_ref(), &cwd, reported.is_none())
                } else {
                    Found::default()
                };
                // Herdr reports no status for a Claude Code session attached from the background (`claude --bg`, a job):
                // its program and its title's spinner say whether it is one and works. An agent with no Herdr status
                // otherwise takes it from its transcript's last turn.
                let (status, title, program) = match reported {
                    Some(status) => (status_word(status), t["name"].as_str().map(str::to_string).unwrap_or_default(), program),
                    None => match attached_claude(&found.got) {
                        Some((status, title)) => (status, title, "claude".to_string()),
                        None => match unreported(agent.as_ref(), &found) {
                            Some(status) => (status, found.got["title"].as_str().unwrap_or_default().to_string(), program),
                            None => continue,
                        },
                    },
                };
                let s = |v: &Value| v.as_str().unwrap_or_default().to_string();
                out.push(Live {
                    handle,
                    cwd: if cwd.is_empty() { s(&found.got["cwd"]) } else { cwd },
                    address,
                    frontend: "app",
                    space_id: s(&space["id"]),
                    space_name: s(&space["name"]),
                    tab_id: s(&tab["id"]),
                    tab_name: s(&tab["name"]),
                    title,
                    agent: agent_id(&program),
                    program,
                    status,
                    context: context_of(agent.as_ref(), found.transcript.as_deref()),
                    session: found.session,
                    transcript: found.transcript,
                    pid: found.got["pid"].as_u64().map(|p| p as u32),
                    since: None,
                });
            }
        }
    }
    out
}

/// What is known of an app terminal's session, and the `hc get` it was found with.
#[derive(Clone, Debug, Default)]
struct Found {
    got: Value,
    session: Option<String>,
    transcript: Option<String>,
}

/// How long what a terminal holds is trusted before it is looked at again (a /clear or a resume starts another session).
const LOOK_AGAIN: Duration = Duration::from_secs(10);

/// An app terminal's `hc get` (again when `fresh`, else every LOOK_AGAIN) and its session, found again every
/// LOOK_AGAIN: Herdr app terminals report none.
fn app_session(handle: &str, address: &str, agent: Option<&Agent>, cwd: &str, fresh: bool) -> Found {
    static KNOWN: Mutex<Option<HashMap<String, (Instant, Found)>>> = Mutex::new(None);
    let cached = KNOWN
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(HashMap::new)
        .get(handle)
        .filter(|(at, _)| at.elapsed() < LOOK_AGAIN)
        .map(|(_, f)| f.clone());
    if let Some(found) = &cached
        && !fresh
    {
        return found.clone();
    }
    let got = json_of(hc(&["get", address, "--json"]));
    if let Some(found) = cached {
        return Found { got, ..found };
    }
    let argv: Vec<String> = got["foreground"]["argv"].as_array().into_iter().flatten().filter_map(|a| a.as_str().map(str::to_string)).collect();
    let pid = got["foreground"]["pid"].as_u64();
    let cwd = if cwd.is_empty() { got["cwd"].as_str().unwrap_or_default() } else { cwd };
    let reported = got["agent_session"].as_str().or(got["agent_session"]["value"].as_str());
    let (session, transcript) = match agent {
        Some(a) => which(a, reported, None, pid, &argv, cwd, got["created_ms"].as_u64().unwrap_or(0)),
        None => (None, None),
    };
    let found = Found { got, session, transcript };
    let mut known = KNOWN.lock().unwrap_or_else(|e| e.into_inner());
    let known = known.get_or_insert_with(HashMap::new);
    known.retain(|_, (at, _)| at.elapsed() < LOOK_AGAIN * 30);
    known.insert(handle.to_string(), (Instant::now(), found.clone()));
    found
}

/// Which session a terminal's agent holds, and its transcript: what Herdr reports (an id or the transcript's path),
/// else Claude Code's own registry by the agent's process, else a session id on its command line (`codex resume
/// <id>`), else the newest session the agent started in the terminal's folder since the terminal did.
pub fn which(
    agent: &Agent,
    reported: Option<&str>,
    path: Option<&str>,
    pid: Option<u64>,
    argv: &[String],
    cwd: &str,
    since: u64,
) -> (Option<String>, Option<String>) {
    let kind = agent.transcripts;
    let shown = |p: PathBuf| p.to_string_lossy().into_owned();
    if let Some(p) = path.filter(|p| Path::new(p).is_file()) {
        let id = reported.map(str::to_string).or_else(|| transcripts::id_of(transcripts::kind_of(Path::new(p)).unwrap_or(kind), Path::new(p)));
        return (id, Some(p.to_string()));
    }
    if kind == Transcripts::None {
        return (reported.map(str::to_string), None);
    }
    if let Some(id) = reported {
        return (Some(id.to_string()), transcripts::find(kind, id, cwd).map(shown));
    }
    if kind == Transcripts::Claude
        && let Some(id) = pid.and_then(claude_session_of)
    {
        let transcript = transcripts::find(kind, &id, cwd).map(shown);
        return (Some(id), transcript);
    }
    let ids = argv.iter().skip(1).filter(|a| a.len() >= 8 && a.chars().all(|c| c.is_ascii_hexdigit() || c == '-'));
    if let Some((id, p)) = ids.into_iter().find_map(|id| Some((id.clone(), transcripts::find(kind, id, cwd)?))) {
        return (Some(id), Some(shown(p)));
    }
    // Without the terminal's start, the newest session in its folder could be any older one.
    match (since > 0).then(|| transcripts::latest(kind, cwd, since)).flatten() {
        Some(p) => (transcripts::id_of(kind, &p), Some(shown(p))),
        None => (None, None),
    }
}

/// The session Claude Code notes for one of its processes: `~/.claude/sessions/<pid>.json`.
fn claude_session_of(pid: u64) -> Option<String> {
    let v = files::load(&transcripts::claude_home().join(format!("sessions/{pid}.json")))?;
    v["sessionId"].as_str().filter(|s| !s.is_empty()).map(str::to_string)
}

fn agent_id(program: &str) -> String {
    agent::of_program(program).map(|a| a.id).unwrap_or_default()
}

/// The status of an agent that reports none to Herdr, from its transcript's last turn; None when it cannot be told.
fn unreported(agent: Option<&Agent>, found: &Found) -> Option<String> {
    let agent = agent?;
    let path = found.transcript.as_deref()?;
    let word = match transcripts::turn(agent.transcripts, Path::new(path)) {
        Some(Turn::Working) => "working",
        Some(Turn::Blocked) => "blocked",
        Some(Turn::Done) => "done",
        None => "idle",
    };
    Some(word.to_string())
}

/// How full its context is, in percent, from its transcript where the format gives the window; read again only when
/// the file changed.
fn context_of(agent: Option<&Agent>, transcript: Option<&str>) -> Option<f64> {
    type Seen = HashMap<String, ((u64, std::time::SystemTime), Option<f64>)>;
    static SEEN: Mutex<Option<Seen>> = Mutex::new(None);
    let (agent, path) = (agent?, transcript?);
    let meta = std::fs::metadata(path).ok()?;
    let stamp = (meta.len(), meta.modified().ok()?);
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    let seen = seen.get_or_insert_with(HashMap::new);
    if let Some((s, v)) = seen.get(path)
        && *s == stamp
    {
        return *v;
    }
    let v = transcripts::usage(agent.transcripts, Path::new(path)).and_then(|u| u.context_percent()).map(f64::round);
    if seen.len() > 200 {
        seen.clear();
    }
    seen.insert(path.to_string(), (stamp, v));
    v
}

/// A terminal whose foreground is Claude Code that Herdr does not track: working while its title spins (Claude puts ✳
/// there when it waits), with the title past the glyph.
fn attached_claude(t: &Value) -> Option<(String, String)> {
    let name = t["foreground"]["name"].as_str().unwrap_or_default();
    if !name.contains("claude") {
        return None;
    }
    let title = t["title"].as_str().unwrap_or_default().trim();
    let (glyph, rest) = title.split_once(' ').unwrap_or(("", title));
    let spins = !glyph.is_empty() && glyph.chars().all(|c| !c.is_alphanumeric()) && glyph != "✳";
    let shown = if glyph.chars().all(|c| !c.is_alphanumeric()) { rest } else { title };
    Some((if spins { "working".into() } else { "idle".into() }, shown.to_string()))
}

fn status_word(s: &str) -> String {
    match s {
        "error" => "blocked".into(),
        "working" | "blocked" | "done" | "idle" => s.into(),
        _ => "idle".into(),
    }
}

fn tui_list() -> Vec<Live> {
    let panes = json_of(herdr(&["pane", "list"]));
    let spaces = json_of(herdr(&["workspace", "list"]));
    let label = |id: &str| -> String {
        let l = spaces["result"]["workspaces"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|w| w["workspace_id"] == id)
            .and_then(|w| w["label"].as_str())
            .unwrap_or_default();
        match l.strip_prefix('[').and_then(|r| r.split_once(']')) {
            Some((n, rest)) if n.chars().all(|c| c.is_ascii_digit()) => rest.trim().to_string(),
            _ => l.to_string(),
        }
    };
    let mut out = Vec::new();
    for p in panes["result"]["panes"].as_array().into_iter().flatten() {
        let Some(agent) = p["agent"].as_str() else { continue };
        let s = |k: &str| p[k].as_str().unwrap_or_default().to_string();
        let context = p["tokens"].as_object().and_then(|t| ["ctx_ok", "ctx_warn", "ctx_hot"].iter().find_map(|k| t.get(*k)?.as_str())).and_then(percent_in);
        let cwd = p["foreground_cwd"].as_str().or(p["cwd"].as_str()).unwrap_or_default().to_string();
        let found = agent::of_program(agent);
        // Herdr's integrations report a session as its id or its transcript's path.
        let reported = &p["agent_session"];
        let path = reported["path"].as_str().or_else(|| (reported["kind"] == "path").then(|| reported["value"].as_str()).flatten());
        let id = (reported["kind"] != "path").then(|| reported["value"].as_str()).flatten();
        let pid = ["foreground_pid", "pid"].iter().find_map(|k| p[*k].as_u64());
        let (session, transcript) = match &found {
            Some(a) => tui_session(&s("pane_id"), a, id, path, pid, &cwd),
            None => (id.map(str::to_string), path.map(str::to_string)),
        };
        out.push(Live {
            handle: s("pane_id"),
            address: s("pane_id"),
            frontend: "tui",
            space_name: label(p["workspace_id"].as_str().unwrap_or_default()),
            space_id: s("workspace_id"),
            tab_id: s("tab_id"),
            tab_name: s("terminal_title_stripped"),
            title: s("terminal_title_stripped"),
            program: agent.to_string(),
            agent: found.as_ref().map(|a| a.id.clone()).unwrap_or_default(),
            status: status_word(p["agent_status"].as_str().unwrap_or("idle")),
            context: context.or_else(|| context_of(found.as_ref(), transcript.as_deref())),
            cwd,
            session,
            transcript,
            pid: pid.map(|p| p as u32),
            since: None,
        });
    }
    out
}

/// A pane's session as `which` finds it, found again when what Herdr reports changes or every LOOK_AGAIN.
fn tui_session(pane: &str, agent: &Agent, id: Option<&str>, path: Option<&str>, pid: Option<u64>, cwd: &str) -> (Option<String>, Option<String>) {
    type Known = HashMap<String, (Instant, String, (Option<String>, Option<String>))>;
    static KNOWN: Mutex<Option<Known>> = Mutex::new(None);
    let key = format!("{}|{}|{cwd}", id.unwrap_or_default(), path.unwrap_or_default());
    let mut guard = KNOWN.lock().unwrap_or_else(|e| e.into_inner());
    let known = guard.get_or_insert_with(HashMap::new);
    if let Some((at, k, found)) = known.get(pane)
        && *k == key
        && (at.elapsed() < LOOK_AGAIN || found.1.is_some())
    {
        return found.clone();
    }
    let found = which(agent, id, path, pid, &[], cwd, 0);
    known.insert(pane.to_string(), (Instant::now(), key, found.clone()));
    found
}

fn percent_in(text: &str) -> Option<f64> {
    text.split_whitespace().find_map(|w| w.strip_suffix('%')?.parse().ok())
}

// ---------------------------------------------------------------------------------------------------------------------
// Acting on one

/// Its screen as text, `lines` of history above it.
pub fn screen(handle: &str, lines: u32) -> Option<String> {
    if let Some(world) = demo() {
        return world["agents"].as_array()?.iter().find(|a| a["terminal"] == handle).map(|a| a["screen"].as_str().unwrap_or_default().to_string());
    }
    let n = lines.to_string();
    match handle.strip_prefix(APP) {
        Some(t) => hc(&["read", t, "--lines", &n]),
        None => herdr(&["pane", "read", handle, "--source", "recent", "--lines", &(lines + 60).to_string()]),
    }
}

/// Presses named keys (enter, up, down, 1, esc).
pub fn keys(handle: &str, keys: &[String]) -> Result<(), String> {
    if keys.is_empty() {
        return Ok(());
    }
    if let Some(mut world) = demo() {
        demo_answered(&mut world, handle, keys.last().map(String::as_str) == Some("enter"));
        return Ok(());
    }
    let mut args: Vec<&str> = Vec::new();
    let ok = match handle.strip_prefix(APP) {
        Some(t) => {
            args.extend(["keys", t]);
            args.extend(keys.iter().map(String::as_str));
            hc(&args).is_some()
        }
        None => {
            args.extend(["pane", "send-keys", handle]);
            args.extend(keys.iter().map(String::as_str));
            herdr(&args).is_some()
        }
    };
    if ok { Ok(()) } else { Err(format!("could not press {} in {handle}", keys.join(" "))) }
}

/// Pastes text and presses Enter (accents and all: key names cannot type them).
pub fn paste(handle: &str, text: &str) -> Result<(), String> {
    if let Some(mut world) = demo() {
        demo_answered(&mut world, handle, true);
        return Ok(());
    }
    let ok = match handle.strip_prefix(APP) {
        Some(t) => hc(&["send", "--force", t, text]).is_some(),
        None => herdr(&["pane", "send-text", handle, text]).is_some() && herdr(&["pane", "send-keys", handle, "enter"]).is_some(),
    };
    if ok { Ok(()) } else { Err(format!("could not type into {handle}")) }
}

/// A demo agent that got its answer goes back to work.
fn demo_answered(world: &mut Value, handle: &str, done: bool) {
    let now = cockpit_core::seen::now_ms();
    for a in world["agents"].as_array_mut().into_iter().flatten() {
        if a["terminal"] == handle && done {
            a["status"] = json!("working");
            a["since"] = json!(now);
            a["screen"] = json!("⏺ Thanks: on it.");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rollout(dir: &Path, id: &str, cwd: &str, done: bool) -> PathBuf {
        let day = chrono::Local::now().format("%Y/%m/%d").to_string();
        let path = dir.join(format!("sessions/{day}/rollout-2026-11-03T10-00-00-{id}.jsonl"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut lines = vec![
            json!({"timestamp": "2026-11-03T10:00:00Z", "type": "session_meta", "payload": {"id": id, "cwd": cwd, "source": "cli", "git": {"branch": "me/eng-412-daily-quotas"}}}),
            json!({"timestamp": "2026-11-03T10:00:01Z", "type": "event_msg", "payload": {"type": "task_started", "model_context_window": 100_000}}),
            json!({"timestamp": "2026-11-03T10:00:02Z", "type": "response_item", "payload": {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "Enforce the daily quota"}]}}),
            json!({"timestamp": "2026-11-03T10:00:03Z", "type": "event_msg", "payload": {"type": "token_count", "info": {"total_token_usage": {"input_tokens": 9000, "output_tokens": 10}, "last_token_usage": {"total_tokens": 25_000}, "model_context_window": 100_000}}}),
        ];
        if done {
            lines.push(json!({"timestamp": "2026-11-03T10:00:04Z", "type": "event_msg", "payload": {"type": "task_complete"}}));
        }
        std::fs::write(&path, lines.iter().map(Value::to_string).collect::<Vec<_>>().join("\n")).unwrap();
        path
    }

    #[test]
    fn a_codex_terminal_finds_its_session_status_and_context_without_herdr_saying() {
        let dir = tempfile::tempdir().unwrap();
        let work = dir.path().join("api");
        std::fs::create_dir_all(&work).unwrap();
        let cwd = work.to_string_lossy().into_owned();
        let codex = dir.path().join("codex");
        let (a, b) = ("0aa1b2c3-d4e5-7f60-8a9b-0c1d2e3f4a01", "0aa1b2c3-d4e5-7f60-8a9b-0c1d2e3f4a02");
        let old = rollout(&codex, a, &cwd, true);
        // SAFETY: the only test of this crate that reads CODEX_HOME.
        unsafe { std::env::set_var("CODEX_HOME", &codex) };
        let agent = agent::builtin().into_iter().find(|x| x.id == "codex").unwrap();
        let shown = |p: &Path| Some(p.to_string_lossy().into_owned());
        let started = std::fs::metadata(&old).unwrap().modified().unwrap().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
        assert_eq!(which(&agent, None, None, None, &[], &cwd, started - 1000), (Some(a.to_string()), shown(&old)), "the session it started there");
        assert_eq!(which(&agent, None, None, None, &[], &cwd, 0), (None, None), "no start time: no guess");
        let newer = rollout(&codex, b, &cwd, false);
        let resumed = ["codex".to_string(), "resume".to_string(), a.to_string()];
        assert_eq!(which(&agent, None, None, None, &resumed, &cwd, started - 1000), (Some(a.to_string()), shown(&old)), "the one it resumed");
        assert_eq!(which(&agent, Some(b), None, None, &[], "", 0), (Some(b.to_string()), shown(&newer)), "the one Herdr reports");
        assert_eq!(which(&agent, None, newer.to_str(), None, &[], "", 0), (Some(b.to_string()), shown(&newer)), "a path Herdr reports");
        let found = Found { got: Value::Null, session: Some(b.into()), transcript: shown(&newer) };
        assert_eq!(unreported(Some(&agent), &found).as_deref(), Some("working"));
        assert_eq!(unreported(Some(&agent), &Found { transcript: shown(&old), ..found.clone() }).as_deref(), Some("done"));
        assert_eq!(context_of(Some(&agent), newer.to_str()), Some(25.0));
        unsafe { std::env::remove_var("CODEX_HOME") };
    }

    #[test]
    fn a_claude_terminal_in_the_background_reads_its_title() {
        let t = json!({"foreground": {"name": "claude"}, "title": "⠂ Fixing quotas"});
        assert_eq!(attached_claude(&t), Some(("working".to_string(), "Fixing quotas".to_string())));
        assert_eq!(attached_claude(&json!({"foreground": {"name": "codex"}, "title": "⠂ x"})), None);
    }
}
