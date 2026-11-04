//! Which window cockpit drives: the herdr TUI (its `herdr` CLI) or the Herdr desktop app (`herdr-app do` and `hc`).
//! Every place that opens, focuses or runs something in a window goes through here, so both halves stay in step.
//!
//! A request names its frontend (`"frontend": "app"`), else the environment does: `COCKPIT_FRONTEND`, `HERDR_ENV=1`
//! (a herdr TUI pane), `HC_TERMINAL_ID` (a Herdr app terminal), the config's `frontend`, then whichever is running.
//! Handles for things already open carry their frontend: an app terminal is `app:t3`, anything else a herdr pane id.

use std::cell::Cell;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::channel;
use std::time::Duration;

use serde_json::{Value, json};

use crate::files;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Frontend {
    Tui,
    App,
}

/// The prefix of a handle that names a Herdr app terminal.
pub const APP: &str = "app:";

/// The stack in the cockpit layout that holds the work area and one block per app.
pub const STACK: &str = "center";

thread_local! {
    static CURRENT: Cell<Option<Frontend>> = const { Cell::new(None) };
}

impl Frontend {
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "app" | "gui" | "herdr-app" => Some(Self::App),
            "tui" | "herdr" => Some(Self::Tui),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Tui => "tui",
            Self::App => "app",
        }
    }
}

/// The frontend of the request this thread serves, else the detected one.
pub fn current() -> Frontend {
    CURRENT.with(Cell::get).unwrap_or_else(detect)
}

/// Runs `f` with this frontend as the thread's current one (None: detected).
pub fn with<T>(frontend: Option<Frontend>, f: impl FnOnce() -> T) -> T {
    let before = CURRENT.with(|c| c.replace(frontend));
    let out = f();
    CURRENT.with(|c| c.set(before));
    out
}

/// The frontend a notification goes to: the request's or the session's, else, with both halves running, the one in
/// front (the Herdr app when its window is the frontmost app, else herdr's terminal).
pub fn for_notice() -> Frontend {
    if let Some(f) = CURRENT.with(Cell::get) {
        return f;
    }
    let told = ["COCKPIT_FRONTEND", "HC_TERMINAL_ID", "HERDR_ENV"].iter().any(|v| std::env::var(v).is_ok_and(|v| !v.is_empty()))
        || files::config()["frontend"].is_string();
    if told || !app_home().join("herdr-app.sock").exists() || !tui_running() {
        return detect();
    }
    if app_in_front() { Frontend::App } else { Frontend::Tui }
}

/// Whether the Herdr app is the frontmost application (macOS; false elsewhere).
fn app_in_front() -> bool {
    let front = Command::new("lsappinfo").arg("front").output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    !front.is_empty()
        && Command::new("lsappinfo")
            .args(["info", "-only", "bundleid", &front])
            .output()
            .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("\"dev.herdr.app\""))
}

pub fn detect() -> Frontend {
    if let Some(found) = std::env::var("COCKPIT_FRONTEND").ok().as_deref().and_then(Frontend::parse) {
        return found;
    }
    let terminal = std::env::var("HC_TERMINAL_ID").ok().filter(|v| !v.is_empty());
    if std::env::var("HERDR_ENV").is_ok_and(|v| v == "1") {
        static UNDER: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let under = *UNDER.get_or_init(|| terminal.is_some_and(|t| descends_from_terminal(&t)));
        return if under { Frontend::App } else { Frontend::Tui };
    }
    if terminal.is_some() {
        return Frontend::App;
    }
    if let Some(found) = files::config()["frontend"].as_str().and_then(Frontend::parse) {
        return found;
    }
    if app_home().join("herdr-app.sock").exists() && !tui_running() { Frontend::App } else { Frontend::Tui }
}

/// Whether this process runs under the app terminal's shell. herdr's variables leak into app terminals when
/// herdr-core was started from a herdr pane, and HC_TERMINAL_ID into a herdr started in an app terminal, whose panes'
/// programs descend from the herdr server instead.
fn descends_from_terminal(terminal: &str) -> bool {
    let Some(shell) = hc(&["get", terminal]).and_then(|t| t["pid"].as_u64()) else { return false };
    let mut pid = std::process::id() as u64;
    for _ in 0..64 {
        if pid == shell {
            return true;
        }
        let out = Command::new("ps").args(["-o", "ppid=", "-p", &pid.to_string()]).stderr(Stdio::null()).output();
        match out.ok().and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<u64>().ok()) {
            Some(parent) if parent > 1 => pid = parent,
            _ => return false,
        }
    }
    false
}

fn tui_running() -> bool {
    Command::new("pgrep").args(["-x", "herdr"]).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

/// The Herdr app's home: HERDR_APP_HOME, else ~/.config/herdr-app.
pub fn app_home() -> PathBuf {
    std::env::var_os("HERDR_APP_HOME").filter(|h| !h.is_empty()).map(PathBuf::from).unwrap_or_else(|| files::home().join(".config/herdr-app"))
}

/// A program on PATH, else in ~/.local/bin where the Herdr installer puts it.
pub fn program(name: &str, env: &str) -> String {
    // COCKPIT_<env> too, which `cockpit app install` hands on to the mod's commands (the sandbox's wrappers).
    if let Some(path) = [env.to_string(), format!("COCKPIT_{env}")].iter().find_map(|v| std::env::var(v).ok().filter(|p| !p.is_empty())) {
        return path;
    }
    let local = files::home().join(".local/bin").join(name);
    if local.exists() { local.to_string_lossy().into_owned() } else { name.to_string() }
}

/// The command's stdout parsed as JSON, 15 seconds at most; None when it fails or prints no JSON.
fn json_of(argv: &[String]) -> Option<Value> {
    let mut command = Command::new(&argv[0]);
    // Run from a terminal, hc tags what it sends with that sender: a shell would run the tag, Claude reads it as a note.
    if argv[0] == program("hc", "HC_BIN") {
        command.env_remove("HC_TERMINAL_ID");
    }
    let child = command.args(&argv[1..]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let pid = child.id();
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    let Ok(Ok(out)) = rx.recv_timeout(Duration::from_secs(15)) else {
        let _ = Command::new("kill").args(["-9", &pid.to_string()]).stderr(Stdio::null()).status();
        return None;
    };
    let value: Value = serde_json::from_slice(&out.stdout).ok()?;
    (out.status.success() && value.get("error").is_none()).then_some(value)
}

// ---------------------------------------------------------------------------------------------------------------------
// The Herdr app

/// `herdr-app do <command> <args>`: what the command made, None when it failed.
pub fn app_do(command: &str, args: &Value) -> Option<Value> {
    json_of(&[program("herdr-app", "HERDR_APP_BIN"), "do".into(), command.into(), args.to_string()])
}

/// `herdr-app ls`: the spaces with their tabs and terminals.
pub fn app_ls() -> Option<Value> {
    json_of(&[program("herdr-app", "HERDR_APP_BIN"), "ls".into()])
}

/// `herdr-app status`: the shown layout, focus and overlays.
pub fn app_status() -> Option<Value> {
    json_of(&[program("herdr-app", "HERDR_APP_BIN"), "status".into()])
}

/// `hc <args> --json`.
pub fn hc(args: &[&str]) -> Option<Value> {
    let mut argv = vec![program("hc", "HC_BIN")];
    argv.extend(args.iter().map(|a| a.to_string()));
    argv.push("--json".into());
    json_of(&argv)
}

/// The app's spaces as `herdr-app ls` lists them.
pub fn app_spaces() -> Vec<Value> {
    app_ls().and_then(|l| l["spaces"].as_array().cloned()).unwrap_or_default()
}

/// The space and tab a terminal shows in, by its address.
pub fn app_place(terminal: &str) -> Option<(String, String)> {
    for space in app_spaces() {
        for tab in space["tabs"].as_array().into_iter().flatten() {
            if tab["terminals"].as_array().into_iter().flatten().any(|t| t["terminal"] == terminal) {
                return Some((space["id"].as_str()?.to_string(), tab["id"].as_str()?.to_string()));
            }
        }
    }
    None
}

/// The id of the app space with this name, if open.
pub fn app_space_named(name: &str) -> Option<String> {
    app_spaces().into_iter().find(|s| s["name"] == name).and_then(|s| s["id"].as_str().map(str::to_string))
}

/// Shows the cockpit layout's block for an app (`inbox`, `runbook`, `todo`, ...), switching to the cockpit layout first.
/// The mod's sidebar does it (`cockpit.go:<app>`), so it shows which one is in front.
pub fn app_show(child: &str) -> bool {
    if app_status().is_some_and(|s| s["layout"] != "cockpit") {
        app_do("layout.switch", &json!({"name": "cockpit"}));
    }
    app_do(&format!("cockpit.go:{child}"), &json!({})).is_some() || app_do("layout.select", &json!({"stack": STACK, "child": child})).is_some()
}

/// Shows the work area, where the spaces' tabs are.
pub fn app_show_work() {
    if app_status().is_some_and(|s| s["layout"] == "cockpit") {
        app_do("cockpit.go:work", &json!({}));
    }
}

/// The handle of a terminal a command made.
fn app_handle(made: &Value) -> Option<String> {
    made["terminal"].as_str().filter(|t| !t.is_empty()).map(|t| format!("{APP}{t}"))
}

// ---------------------------------------------------------------------------------------------------------------------
// herdr TUI

/// A herdr call's `result`, 15 seconds at most; None when it fails, answers no JSON or a null result.
pub fn herdr(args: &[&str]) -> Option<Value> {
    let mut argv = vec![std::env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".into())];
    argv.extend(args.iter().map(|a| a.to_string()));
    let child = Command::new(&argv[0]).args(&argv[1..]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let pid = child.id();
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    let Ok(Ok(out)) = rx.recv_timeout(Duration::from_secs(15)) else {
        let _ = Command::new("kill").args(["-9", &pid.to_string()]).stderr(Stdio::null()).status();
        return None;
    };
    if !out.status.success() {
        return None;
    }
    serde_json::from_slice::<Value>(&out.stdout).ok()?.get("result").filter(|r| !r.is_null()).cloned()
}

/// herdr's spaces, each label stripped of its "[N] " prefix.
fn tui_spaces() -> Vec<Value> {
    let listed = herdr(&["workspace", "list"]).and_then(|r| r["workspaces"].as_array().cloned()).unwrap_or_default();
    listed
        .into_iter()
        .map(|mut w| {
            let label = w["label"].as_str().unwrap_or_default();
            let stripped = match label.strip_prefix('[').and_then(|r| r.split_once(']')) {
                Some((n, rest)) if n.chars().all(|c| c.is_ascii_digit()) => rest.trim_start().to_string(),
                _ => label.to_string(),
            };
            w["label"] = json!(stripped);
            w
        })
        .collect()
}

fn root_pane(created: &Value) -> Option<String> {
    created["root_pane"]["pane_id"].as_str().map(str::to_string)
}

// ---------------------------------------------------------------------------------------------------------------------
// What callers use

/// Brings an app's place into view: its pinned space in the TUI, its block of the cockpit layout in the app.
pub fn show_app(app: &str, space: &str) -> bool {
    match current() {
        Frontend::App => app_show(app),
        Frontend::Tui => {
            let suffix = format!("  {space}");
            let found = tui_spaces().into_iter().find(|w| w["label"].as_str().is_some_and(|l| l.ends_with(&suffix)));
            found.and_then(|w| w["workspace_id"].as_str().map(str::to_string)).is_some_and(|id| herdr(&["workspace", "focus", &id]).is_some())
        }
    }
}

/// A new space in view with a shell in `cwd`: the handle of its first pane or terminal.
pub fn new_space(cwd: &str, label: &str) -> Option<String> {
    match current() {
        Frontend::App => {
            let made = app_do("space.new", &json!({"name": label, "folder": cwd}))?;
            app_show_work();
            app_handle(&made)
        }
        Frontend::Tui => root_pane(&herdr(&["workspace", "create", "--cwd", cwd, "--label", label, "--focus"])?),
    }
}

/// The space open under this label, brought into view; false when none is.
pub fn focus_space_named(label: &str) -> bool {
    match current() {
        Frontend::App => {
            let Some(id) = app_space_named(label) else { return false };
            app_show_work();
            app_do("space.select", &json!({"id": id})).is_some()
        }
        Frontend::Tui => {
            let found = tui_spaces().into_iter().find(|w| w["label"].as_str() == Some(label));
            found.and_then(|w| w["workspace_id"].as_str().map(str::to_string)).is_some_and(|id| herdr(&["workspace", "focus", &id]).is_some())
        }
    }
}

/// A new tab for a one-off action, in view: its handle. In the TUI it goes to the pinned `actions` space; the Herdr app
/// has none, so it goes beside the folder's own work: the space open on that folder, else a new one named after it.
pub fn actions_tab(pinned: &[String], cwd: &str, label: &str) -> Option<String> {
    match current() {
        Frontend::App => {
            let name = std::path::Path::new(cwd).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| label.to_string());
            let spaces = app_spaces();
            let found = spaces.iter().find(|s| s["folder"].as_str() == Some(cwd)).or_else(|| spaces.iter().find(|s| s["name"].as_str() == Some(name.as_str())));
            let space = match found.and_then(|s| s["id"].as_str()) {
                Some(id) => id.to_string(),
                None => return new_space(cwd, &name).inspect(|h| rename_tab(h, label)),
            };
            let made = app_do("tab.new", &json!({"space": space, "folder": cwd, "name": label}))?;
            app_show_work();
            app_do("space.select", &json!({"id": space}));
            app_handle(&made)
        }
        Frontend::Tui => {
            if !pinned.iter().any(|n| n == "actions") {
                return None;
            }
            let ws = tui_spaces()
                .into_iter()
                .find(|w| w["label"].as_str().is_some_and(|l| l.ends_with("actions")))
                .and_then(|w| w["workspace_id"].as_str().map(str::to_string))?;
            root_pane(&herdr(&["tab", "create", "--workspace", &ws, "--cwd", cwd, "--label", label, "--focus"])?)
        }
    }
}

fn rename_tab(handle: &str, name: &str) {
    if let Some((_, tab)) = handle.strip_prefix(APP).and_then(app_place) {
        app_do("tab.rename", &json!({"id": tab, "name": name}));
    }
}

/// Types a shell line into the handle's terminal and runs it.
pub fn run_in(handle: &str, line: &str) -> bool {
    match handle.strip_prefix(APP) {
        Some(terminal) => {
            let mut argv = vec![program("hc", "HC_BIN"), "send".into(), "--force".into(), terminal.into(), line.into()];
            argv.push("--json".into());
            // Sent from a terminal, hc tags the text with its sender: a shell would run the tag.
            Command::new(&argv[0])
                .args(&argv[1..])
                .env_remove("HC_TERMINAL_ID")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        }
        None => herdr(&["pane", "run", handle, line]).is_some(),
    }
}

/// Puts text at the prompt of an app terminal without sending it, through the clipboard, which gets its own text back
/// a moment later.
pub fn app_type(terminal: &str, text: &str) -> bool {
    let before = clipboard();
    if !files::copy(text) {
        return false;
    }
    app_show_work();
    let pasted = app_do("pane.focus", &json!({"terminal": terminal})).is_some() && app_do("terminal.paste", &json!({})).is_some();
    if let Some(before) = before {
        std::thread::sleep(Duration::from_millis(800));
        files::copy(&String::from_utf8_lossy(&before));
    }
    pasted
}

/// The clipboard's text, read with the same tools `files::copy` writes with: pbpaste, wl-paste, xclip.
fn clipboard() -> Option<Vec<u8>> {
    let tools: [&[&str]; 3] = [&["pbpaste"], &["wl-paste", "--no-newline"], &["xclip", "-selection", "clipboard", "-o"]];
    tools.iter().find_map(|t| {
        Command::new(t[0]).args(&t[1..]).stdin(Stdio::null()).stderr(Stdio::null()).output().ok().filter(|o| o.status.success()).map(|o| o.stdout)
    })
}

/// Sends text to an app terminal's agent as its next prompt.
pub fn app_prompt(terminal: &str, text: &str) -> bool {
    let argv = [program("hc", "HC_BIN"), "send".into(), terminal.into(), text.into(), "--json".into()];
    Command::new(&argv[0])
        .args(&argv[1..])
        .env_remove("HC_TERMINAL_ID")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Whether the handle's terminal still runs an agent (the TUI) or still lives (the app).
pub fn alive(handle: &str) -> bool {
    match handle.strip_prefix(APP) {
        Some(terminal) => {
            hc(&["ls"]).is_some_and(|l| l["terminals"].as_array().into_iter().flatten().any(|t| t["address"] == terminal && t["state"] == "live"))
        }
        None => herdr(&["pane", "get", handle]).is_some_and(|f| truthy(&f["pane"]["agent"])),
    }
}

/// Whether the handle's pane or terminal still exists.
pub fn exists(handle: &str) -> bool {
    match handle.strip_prefix(APP) {
        Some(terminal) => hc(&["ls"]).is_some_and(|l| l["terminals"].as_array().into_iter().flatten().any(|t| t["address"] == terminal)),
        None => herdr(&["pane", "get", handle]).is_some_and(|f| truthy(&f)),
    }
}

/// Brings the handle's tab into view, with its space.
pub fn focus(handle: &str) -> bool {
    match handle.strip_prefix(APP) {
        Some(terminal) => {
            app_show_work();
            app_do("pane.focus", &json!({"terminal": terminal})).is_some()
        }
        None => {
            let Some(found) = herdr(&["pane", "get", handle]).filter(truthy) else { return false };
            let tab = found["pane"]["tab_id"].as_str().unwrap_or_default().to_string();
            herdr(&["tab", "focus", &tab]).is_some()
        }
    }
}

/// The tab id the handle shows in (a herdr tab id, or the app's), None when it is gone.
pub fn tab_of(handle: &str) -> Option<String> {
    match handle.strip_prefix(APP) {
        Some(terminal) => app_place(terminal).map(|(_, tab)| tab),
        None => herdr(&["pane", "get", handle])?["pane"]["tab_id"].as_str().map(str::to_string),
    }
}

/// Moves the handle's tab out to a space of its own, in view; false when it cannot.
pub fn promote(handle: &str, label: &str) -> bool {
    match handle.strip_prefix(APP) {
        // The app moves no tab across spaces: the tab stays where it is, renamed, in view.
        Some(_) => {
            rename_tab(handle, label);
            focus(handle)
        }
        None => herdr(&["pane", "move", handle, "--new-workspace", "--label", label, "--focus"]).is_some_and(|r| truthy(&r)),
    }
}

/// Closes a tab by its id: `app:<tab id>` in the app, a herdr tab id in the TUI.
pub fn close_tab(tab: &str) -> bool {
    match tab.strip_prefix(APP) {
        Some(id) => app_do("tab.close", &json!({"id": id, "confirmed": true})).is_some(),
        None => herdr(&["tab", "close", tab]).is_some(),
    }
}

/// Closes a space by its id: `app:<space id>` in the app, a herdr workspace id in the TUI.
pub fn close_space(space: &str) -> bool {
    match space.strip_prefix(APP) {
        Some(id) => app_do("space.close", &json!({"id": id, "confirmed": true})).is_some(),
        None => herdr(&["workspace", "close", space]).is_some(),
    }
}

/// Whether an answer says something: herdr gives an empty result for a call that found or did nothing.
pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_names_its_frontend_for_its_thread_only() {
        let inner = with(Some(Frontend::App), current);
        assert_eq!(inner, Frontend::App);
        assert_eq!(with(Some(Frontend::Tui), current), Frontend::Tui);
        assert_eq!(Frontend::parse("app"), Some(Frontend::App));
        assert_eq!(Frontend::parse("nope"), None);
    }
}
