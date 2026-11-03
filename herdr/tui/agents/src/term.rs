//! The agent terminals of both frontends, and what is done to one: its screen read, keys pressed, text pasted; a handle
//! is `app:t3` for a Herdr app terminal, a herdr pane id (`w9:p1`) for the TUI. With COCKPIT_AGENTS_DEMO naming a JSON
//! file, the terminals are that file's made-up ones (the sandbox, the screenshots), answered in memory.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::channel;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use cockpit_core::files;
use cockpit_core::frontend::{self, APP};
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
    /// working | blocked | done | idle
    pub status: String,
    pub cwd: String,
    pub session: Option<String>,
    pub context: Option<f64>,
    /// When its status began, when the source knows (the demo).
    pub since: Option<u64>,
}

/// A program's stdout when it exits 0 within `secs`, else None.
pub fn output(argv: &[String], secs: u64) -> Option<String> {
    // hc would tag what it sends with the terminal this runs in.
    let child = Command::new(&argv[0]).args(&argv[1..]).env_remove("HC_TERMINAL_ID").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
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
                status: s("status"),
                cwd: s("cwd"),
                session: a["session"].as_str().map(str::to_string),
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
    let cwd = json_of(hc(&["ls", "--json"]));
    let cwd_of = |t: &str| -> String {
        cwd["terminals"].as_array().into_iter().flatten().find(|x| x["address"] == t).and_then(|x| x["cwd"].as_str()).unwrap_or_default().to_string()
    };
    let mut out = Vec::new();
    for space in ls["spaces"].as_array().into_iter().flatten() {
        for tab in space["tabs"].as_array().into_iter().flatten() {
            for t in tab["terminals"].as_array().into_iter().flatten() {
                let address = t["terminal"].as_str().unwrap_or_default().to_string();
                // Herdr reports no status for a Claude Code session attached from the background (`claude --bg`, a job):
                // its program and its title's spinner say whether it is one and works.
                let (status, title, program) = match t["status"].as_str().filter(|s| !s.is_empty()) {
                    Some(status) => (status_word(status), t["name"].as_str().map(str::to_string).unwrap_or_default(), t["app"].as_str().unwrap_or("agent").to_string()),
                    None => match attached_claude(&address) {
                        Some((status, title)) => (status, title, "claude".to_string()),
                        None => continue,
                    },
                };
                let s = |v: &Value| v.as_str().unwrap_or_default().to_string();
                out.push(Live {
                    handle: format!("{APP}{address}"),
                    cwd: cwd_of(&address),
                    address,
                    frontend: "app",
                    space_id: s(&space["id"]),
                    space_name: s(&space["name"]),
                    tab_id: s(&tab["id"]),
                    tab_name: s(&tab["name"]),
                    title,
                    program,
                    status,
                    session: None,
                    context: None,
                    since: None,
                });
            }
        }
    }
    out
}

/// A terminal whose foreground is Claude Code that Herdr does not track: working while its title spins (Claude puts ✳
/// there when it waits), with the title past the glyph.
fn attached_claude(address: &str) -> Option<(String, String)> {
    let t = json_of(hc(&["get", address, "--json"]));
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
        let l = spaces["result"]["workspaces"].as_array().into_iter().flatten().find(|w| w["workspace_id"] == id).and_then(|w| w["label"].as_str()).unwrap_or_default();
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
            status: status_word(p["agent_status"].as_str().unwrap_or("idle")),
            cwd: p["foreground_cwd"].as_str().or(p["cwd"].as_str()).unwrap_or_default().to_string(),
            session: p["agent_session"]["value"].as_str().map(str::to_string),
            context,
            since: None,
        });
    }
    out
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
