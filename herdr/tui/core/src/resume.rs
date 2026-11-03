//! Agents a restart stopped, brought back where they were. While they run, the hub notes which Claude session each
//! terminal holds (Claude Code's own `~/.claude/sessions/<pid>.json` in the Herdr app, herdr's `agent_session` in the
//! TUI) in `~/.cache/cockpit/resume/agents.json`; after a reboot or a herdr-core restart, the ones not running again
//! are resumed in their terminal, or in a new space when it is gone.

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

use crate::files;
use crate::frontend::{self, APP, Frontend};

/// A lost agent stays offered this long, then goes.
const KEEP_LOST_MS: u64 = 7 * 86_400_000;

/// One Claude session running in a terminal now.
#[derive(Clone, Debug, PartialEq)]
pub struct Seen {
    /// `app:t3` or a herdr pane id.
    pub handle: String,
    pub session: String,
    pub cwd: String,
    pub name: String,
    /// When the terminal started (the app's `created_ms`), which changes when herdr-core gives it a new shell.
    pub created: u64,
}

pub fn dir() -> PathBuf {
    files::cache().join("resume")
}

fn path() -> PathBuf {
    dir().join("agents.json")
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// When this machine booted, in seconds (0 when unknown).
pub fn boot_time() -> u64 {
    if cfg!(target_os = "macos") {
        let out = std::process::Command::new("sysctl").args(["-n", "kern.boottime"]).output().ok();
        let text = out.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
        return text.split("sec = ").nth(1).and_then(|r| r.split(|c: char| !c.is_ascii_digit()).next()).and_then(|n| n.parse().ok()).unwrap_or(0);
    }
    let stat = std::fs::read_to_string("/proc/stat").unwrap_or_default();
    stat.lines().find_map(|l| l.strip_prefix("btime ")).and_then(|n| n.trim().parse().ok()).unwrap_or(0)
}

/// A Claude session id, the only kind of id ever typed into a shell.
pub fn is_session_id(id: &str) -> bool {
    id.len() == 36 && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

/// The line that resumes the session in a shell, its folder quoted.
pub fn resume_line(cwd: &str, session: &str) -> String {
    let quoted = cwd.replace('\'', "'\\''");
    if cwd.is_empty() { format!("claude --resume {session}") } else { format!("cd '{quoted}' && claude --resume {session}") }
}

// ---------------------------------------------------------------------------------------------------------------------
// What runs now

/// The Claude sessions running in terminals now, per frontend; None for a frontend that could not be asked (its
/// terminals are then left as they were).
pub fn seen_now() -> (Option<Vec<Seen>>, Option<Vec<Seen>>) {
    let app = frontend::app_home().join("herdr-app.sock").exists().then(seen_in_app).flatten();
    let tui = seen_in_tui();
    (app, tui)
}

/// The name of the terminal where the hub keeps a Claude session for the MCP-only sources (`cockpit-hub`'s fetcher).
pub const FETCHER: &str = "cockpit-fetch";

/// Claude Code's registry of running sessions: pid → {sessionId, cwd, name}.
fn claude_sessions() -> Map<String, Value> {
    let dir = files::home().join(".claude/sessions");
    let mut out = Map::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "json")
            && let Some(v) = files::load(&path)
            && let Some(pid) = v["pid"].as_u64()
        {
            out.insert(pid.to_string(), v);
        }
    }
    out
}

fn seen_in_app() -> Option<Vec<Seen>> {
    let listed = frontend::hc(&["ls"])?;
    let registry = claude_sessions();
    let mut out = Vec::new();
    for t in listed["terminals"].as_array().into_iter().flatten() {
        let address = t["address"].as_str().unwrap_or_default();
        // The hub's own fetching session is not an agent to bring back: the hub starts it again.
        if t["state"] != "live" || address.is_empty() || !t["agent"].as_str().unwrap_or_default().contains("claude") || t["name"] == FETCHER {
            continue;
        }
        let Some(got) = frontend::hc(&["get", address]) else { continue };
        let pid = got["foreground"]["pid"].as_u64().map(|p| p.to_string()).unwrap_or_default();
        let Some(session) = registry.get(&pid) else { continue };
        let id = session["sessionId"].as_str().unwrap_or_default();
        if !is_session_id(id) {
            continue;
        }
        out.push(Seen {
            handle: format!("{APP}{address}"),
            session: id.to_string(),
            cwd: session["cwd"].as_str().or_else(|| got["cwd"].as_str()).unwrap_or_default().to_string(),
            name: session["name"].as_str().unwrap_or_default().to_string(),
            created: got["created_ms"].as_u64().unwrap_or(0),
        });
    }
    Some(out)
}

fn seen_in_tui() -> Option<Vec<Seen>> {
    if std::env::var_os("COCKPIT_NO_TUI").is_some() {
        return None;
    }
    let listed = frontend::herdr(&["pane", "list"])?;
    let out = listed["panes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| p["agent_session"]["agent"] == "claude" && p["agent_session"]["kind"] == "id")
        .filter_map(|p| {
            let id = p["agent_session"]["value"].as_str()?;
            is_session_id(id).then(|| Seen {
                handle: p["pane_id"].as_str().unwrap_or_default().to_string(),
                session: id.to_string(),
                cwd: p["cwd"].as_str().unwrap_or_default().to_string(),
                name: p["terminal_title_stripped"].as_str().unwrap_or_default().to_string(),
                created: 0,
            })
        })
        .collect();
    Some(out)
}

// ---------------------------------------------------------------------------------------------------------------------
// The record

/// The record after looking: running sessions noted, those gone since a restart (another boot, or a terminal herdr-core
/// gave a new shell) marked lost, those that ended in this boot dropped. `asked` says which frontends answered: a
/// frontend that did not keeps its entries as they were.
pub fn merge(record: &Value, seen: &[Seen], asked: &dyn Fn(&str) -> bool, boot: u64, now: u64, created_now: &dyn Fn(&str) -> Option<u64>) -> Value {
    let mut agents = record["agents"].as_object().cloned().unwrap_or_default();
    for s in seen {
        agents.insert(
            s.handle.clone(),
            json!({"session": s.session, "cwd": s.cwd, "name": s.name, "at": now, "boot": boot, "created": s.created, "lost": false}),
        );
    }
    let running: Vec<&str> = seen.iter().map(|s| s.handle.as_str()).collect();
    agents.retain(|handle, entry| {
        if running.contains(&handle.as_str()) || !asked(handle) {
            return true;
        }
        if entry["lost"] == true {
            return now.saturating_sub(entry["at"].as_u64().unwrap_or(0)) < KEEP_LOST_MS;
        }
        let rebooted = entry["boot"].as_u64().unwrap_or(0) != boot;
        let respawned = entry["created"].as_u64().filter(|c| *c > 0).is_some_and(|c| created_now(handle).is_some_and(|now| now != c));
        if rebooted || respawned {
            entry["lost"] = json!(true);
            return true;
        }
        false
    });
    json!({"boot": boot, "agents": agents})
}

/// The terminals' start times in the app now, by handle.
fn app_created() -> Map<String, Value> {
    let listed = frontend::hc(&["ls"]).unwrap_or(Value::Null);
    let mut out = Map::new();
    for t in listed["terminals"].as_array().into_iter().flatten() {
        if let Some(address) = t["address"].as_str()
            && let Some(got) = frontend::hc(&["get", address])
        {
            out.insert(format!("{APP}{address}"), got["created_ms"].clone());
        }
    }
    out
}

/// Looks at what runs now and writes the record when it changed: the record, and how many agents became lost.
pub fn remember() -> (Value, usize) {
    let before = files::load(&path()).unwrap_or(json!({"agents": {}}));
    let (app, tui) = seen_now();
    let seen: Vec<Seen> = app.iter().chain(tui.iter()).flatten().cloned().collect();
    let (app_asked, tui_asked) = (app.is_some(), tui.is_some());
    let asked = |handle: &str| if handle.starts_with(APP) { app_asked } else { tui_asked };
    let had_app = before["agents"].as_object().into_iter().flatten().any(|(h, e)| h.starts_with(APP) && e["lost"] != true);
    let created = if app_asked && had_app { app_created() } else { Map::new() };
    // A herdr-core that started again gave every terminal a new shell.
    let core = if app_asked { frontend::hc(&["server", "status"]).and_then(|s| s["pid"].as_u64()) } else { None };
    let core_restarted = core.is_some_and(|pid| before["core"].as_u64().is_some_and(|was| was != pid));
    let created_now = |h: &str| if core_restarted && h.starts_with(APP) { Some(u64::MAX) } else { created.get(h).and_then(Value::as_u64) };
    let mut after = merge(&before, &seen, &asked, boot_time(), now_ms(), &created_now);
    after["core"] = core.map(Value::from).unwrap_or_else(|| before["core"].clone());
    let lost = |v: &Value| v["agents"].as_object().into_iter().flatten().filter(|(_, e)| e["lost"] == true).count();
    let newly = lost(&after).saturating_sub(lost(&before));
    if after["agents"] != before["agents"] || after["boot"] != before["boot"] || after["core"] != before["core"] {
        let _ = files::write(&path(), &after);
    }
    (after, newly)
}

/// The agents a restart stopped, newest first: {handle, session, cwd, name, at}.
pub fn lost() -> Vec<Value> {
    let record = files::load(&path()).unwrap_or(Value::Null);
    let mut out: Vec<Value> = record["agents"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(_, e)| e["lost"] == true)
        .map(|(handle, e)| {
            let mut e = e.clone();
            e["handle"] = json!(handle);
            e
        })
        .collect();
    out.sort_by_key(|e| std::cmp::Reverse(e["at"].as_u64().unwrap_or(0)));
    out
}

fn edit(change: impl FnOnce(&mut Map<String, Value>)) {
    let mut record = files::load(&path()).unwrap_or(json!({"agents": {}}));
    if let Some(agents) = record["agents"].as_object_mut() {
        change(agents);
    }
    let _ = files::write(&path(), &record);
}

const SHELLS: [&str; 6] = ["zsh", "bash", "fish", "sh", "dash", "nu"];

/// Whether the handle's terminal sits at a shell, ready for the line: a shell in the foreground of an app terminal (its
/// last status report outlives the agent), a pane with no agent in the TUI.
fn at_shell(handle: &str) -> bool {
    match handle.strip_prefix(APP) {
        Some(address) => frontend::hc(&["get", address]).is_some_and(|t| t["state"] == "live" && SHELLS.contains(&t["foreground"]["name"].as_str().unwrap_or_default().trim_start_matches('-'))),
        None => frontend::exists(handle) && !frontend::alive(handle),
    }
}

/// Leaves a lost agent stopped: it is offered no more.
pub fn dismiss(handle: &str) {
    edit(|agents| _ = agents.remove(handle));
}

/// Resumes one lost agent in its terminal when it still sits at a shell, else in a new space in its folder.
pub fn resume(handle: &str) -> Result<String, String> {
    let record = files::load(&path()).unwrap_or(Value::Null);
    let entry = record["agents"][handle].clone();
    let session = entry["session"].as_str().unwrap_or_default().to_string();
    if !is_session_id(&session) {
        return Err(format!("no stopped agent in {handle}"));
    }
    let cwd = entry["cwd"].as_str().unwrap_or_default().to_string();
    let line = resume_line(&cwd, &session);
    let which = if handle.starts_with(APP) { Frontend::App } else { Frontend::Tui };
    let name = entry["name"].as_str().filter(|n| !n.is_empty()).map(str::to_string).unwrap_or_else(|| folder_name(&cwd));
    let done = frontend::with(Some(which), || {
        if at_shell(handle) {
            Ok(frontend::run_in(handle, &line))
        } else if frontend::exists(handle) {
            Err(format!("{name}'s terminal runs something else now: resume it yourself with `{line}`"))
        } else {
            Ok(frontend::new_space(&cwd, &name).is_some_and(|h| frontend::run_in(&h, &line)))
        }
    })?;
    if !done {
        return Err(format!("could not type into {handle}"));
    }
    edit(|agents| {
        if let Some(e) = agents.get_mut(handle) {
            e["lost"] = json!(false);
            e["at"] = json!(now_ms());
            e["boot"] = json!(boot_time());
            e["created"] = json!(0);
        }
    });
    Ok(format!("resumed {name}"))
}

/// Resumes every lost agent: how many came back, and the first failure.
pub fn resume_all() -> Result<String, String> {
    let lost = lost();
    let mut done = 0;
    let mut failed = None;
    for e in &lost {
        match resume(e["handle"].as_str().unwrap_or_default()) {
            Ok(_) => done += 1,
            Err(err) => failed = failed.or(Some(err)),
        }
    }
    match failed {
        Some(err) if done == 0 => Err(err),
        _ => Ok(format!("resumed {done} agent{}", if done == 1 { "" } else { "s" })),
    }
}

fn folder_name(cwd: &str) -> String {
    cwd.trim_end_matches('/').rsplit('/').next().filter(|n| !n.is_empty()).unwrap_or("agent").to_string()
}

/// The hub's loop: looks every minute, the first time a minute after the hub starts (Herdr is back by then); when a
/// restart stopped agents, says so once, with Resume as the click.
pub fn keep() {
    loop {
        std::thread::sleep(Duration::from_secs(60));
        let (record, newly) = remember();
        if newly > 0 {
            let names: Vec<String> = lost().iter().take(4).map(|e| e["name"].as_str().filter(|n| !n.is_empty()).map(str::to_string).unwrap_or_else(|| folder_name(e["cwd"].as_str().unwrap_or_default()))).collect();
            let count = record["agents"].as_object().into_iter().flatten().filter(|(_, e)| e["lost"] == true).count();
            let title = format!("↻ {count} agent{} stopped by the restart", if count == 1 { "" } else { "s" });
            crate::notify::post(&title, &format!("Click to resume them where they were: {}", names.join(", ")), "resume", &[]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(handle: &str, session: &str, created: u64) -> Seen {
        Seen { handle: handle.into(), session: session.into(), cwd: "/code/api".into(), name: "api".into(), created }
    }

    const A: &str = "0f0e0d0c-0b0a-0908-0706-050403020100";
    const B: &str = "1f0e0d0c-0b0a-0908-0706-050403020100";

    #[test]
    fn a_session_that_ends_in_this_boot_is_forgotten_and_one_a_reboot_stopped_is_lost() {
        let all = |_: &str| true;
        let none = |_: &str| None;
        let running = merge(&json!({}), &[seen("app:t3", A, 5), seen("w1:p1", B, 0)], &all, 100, 1_000, &none);
        assert_eq!(running["agents"]["app:t3"]["session"], A);
        let ended = merge(&running, &[seen("w1:p1", B, 0)], &all, 100, 2_000, &none);
        assert!(ended["agents"].get("app:t3").is_none());
        let rebooted = merge(&ended, &[], &all, 200, 3_000, &none);
        assert_eq!(rebooted["agents"]["w1:p1"]["lost"], true);
        let back = merge(&rebooted, &[seen("w1:p1", B, 0)], &all, 200, 4_000, &none);
        assert_eq!(back["agents"]["w1:p1"]["lost"], false);
    }

    #[test]
    fn a_terminal_herdr_core_gave_a_new_shell_counts_as_lost_and_an_unasked_frontend_keeps_its_own() {
        let running = merge(&json!({}), &[seen("app:t3", A, 5)], &|_| true, 100, 1_000, &|_| None);
        let respawned = merge(&running, &[], &|_| true, 100, 2_000, &|_| Some(9));
        assert_eq!(respawned["agents"]["app:t3"]["lost"], true);
        let unasked = merge(&running, &[], &|h| !h.starts_with("app:"), 100, 2_000, &|_| None);
        assert_eq!(unasked["agents"]["app:t3"]["lost"], false);
        let old = merge(&respawned, &[], &|_| true, 100, 2_000 + KEEP_LOST_MS, &|_| None);
        assert!(old["agents"].get("app:t3").is_none());
    }

    #[test]
    fn only_a_session_id_is_typed_and_the_folder_is_quoted() {
        assert!(is_session_id(A));
        assert!(!is_session_id("x; rm -rf ~"));
        assert_eq!(resume_line("/code/it's", A), format!("cd '/code/it'\\''s' && claude --resume {A}"));
    }
}
