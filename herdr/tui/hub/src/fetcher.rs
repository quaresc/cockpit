//! A Claude Code session the hub keeps for the sources it cannot reach itself: Linear, CircleCI or the calendar without
//! a token or a server of the hub's own (`servers` in the config), the platform and the monitor's Datadog. Their data comes from MCP connectors that cockpit's Claude Code
//! plugin calls, so the hub runs one in a hidden herdr-core terminal (no space, no pane): idle at its prompt, it spends
//! no tokens, and the plugin in it refreshes the inbox, CI, meetings and monitor like any session. It runs only where
//! Claude Code is installed; `fetcher: false` in the config turns it off, and it goes when nothing needs it any more.
//! Without the Herdr app's `hc` it runs in a space of herdr's own, kept out of view, so herdr alone is enough.

use std::time::Duration;

use cockpit_core::frontend::{self, Frontend};
use cockpit_core::resume::FETCHER;
use cockpit_core::{agent, files};
use serde_json::Value;

use crate::config;

const EVERY: Duration = Duration::from_secs(60);

/// The label the fetcher's herdr space starts with, when there is no Herdr app to give it a terminal of its own. herdr
/// renames a space after the agent in it, so the space is known by the id kept in `fetcher.space`, never by this.
const SPACE: &str = "cockpit fetcher";

/// Starts the watch once, beside the feed.
pub fn start() {
    static STARTED: std::sync::Once = std::sync::Once::new();
    if std::env::var("COCKPIT_HUB_FEED").is_ok_and(|v| v == "off") {
        return;
    }
    STARTED.call_once(|| {
        std::thread::spawn(|| {
            loop {
                tend();
                std::thread::sleep(EVERY);
            }
        });
    });
}

/// The sources only an MCP connector reaches, as the config has them: empty when the hub's tokens and servers cover
/// everything.
pub fn mcp_sources() -> Vec<&'static str> {
    let c = config::load();
    let raw = files::load(&files::config_dir().join("config.json")).unwrap_or(Value::Null);
    sources(&c.0, &raw, &c.space_names())
}

fn sources(c: &Value, raw: &Value, spaces: &[String]) -> Vec<&'static str> {
    let on = |section: &str| c[section]["enabled"] != false && c[section].is_object();
    let has = |section: &str, key: &str| raw[section][key].as_str().is_some_and(|t| !t.trim().is_empty());
    let server = |section: &str| c[section]["server"].as_str().is_some_and(|s| !s.is_empty());
    let hubs = |section: &str| crate::feed::mcp::server(raw, c, section).is_some();
    let mut out = Vec::new();
    if on("tickets") && server("tickets") && !has("tickets", "token") && !hubs("tickets") {
        out.push("tickets");
    }
    if on("ci") && server("ci") && !has("ci", "token") && !hubs("ci") {
        out.push("ci");
    }
    if c["calendar"]["enabled"] == true && !has("calendar", "ics") && !hubs("calendar") {
        out.push("calendar");
    }
    if c["platform"]["enabled"] == true && !hubs("platform") {
        out.push("platform");
    }
    if c["mail"]["enabled"] == true && server("mail") && !hubs("mail") {
        out.push("mail");
    }
    if spaces.iter().any(|s| s == "monitor") && !(hubs("platform") && crate::feed::mcp::observability(raw).is_some()) {
        out.push("monitor");
    }
    out
}

fn hc(args: &[&str]) -> Option<Value> {
    frontend::hc(args)
}

/// How the fetcher is run on this machine.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Via {
    /// The Herdr app's `hc`: a hidden terminal of its own.
    Hc,
    /// herdr alone: a space kept out of view.
    Herdr,
}

/// Whether `hc`, the Herdr app's command line, can be run: `HC_BIN`, `~/.local/bin/hc` or one on the PATH.
fn hc_installed() -> bool {
    let program = frontend::program("hc", "HC_BIN");
    if program.contains('/') {
        return std::path::Path::new(&program).exists();
    }
    std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(&program).is_file()))
}

/// The way to run the fetcher here: `hc` when it is there, else herdr when its server answers, else none.
fn via() -> Option<Via> {
    if hc_installed() { Some(Via::Hc) } else { frontend::with(Some(Frontend::Tui), || frontend::herdr(&["workspace", "list"])).map(|_| Via::Herdr) }
}

/// Whether a pane of this herdr space runs Claude Code, in the answer of `herdr pane list`.
fn space_runs_claude(panes: &Value, space: &str) -> bool {
    panes["panes"].as_array().into_iter().flatten().any(|p| p["workspace_id"] == space && p["agent"].as_str() == Some("claude"))
}

/// Where the id of the fetcher's herdr space is kept: its label changes with the agent in it.
fn space_file() -> std::path::PathBuf {
    files::cache().join("fetcher.space")
}

/// The workspace a herdr pane handle (`w2Y:p1`) belongs to.
fn space_of(pane: &str) -> Option<&str> {
    pane.split_once(':').map(|(space, _)| space).filter(|space| !space.is_empty())
}

/// Whether a space with this id is among the ones `herdr workspace list` gives.
fn space_listed(spaces: &Value, space: &str) -> bool {
    spaces["workspaces"].as_array().into_iter().flatten().any(|w| w["workspace_id"] == space)
}

/// The fetcher's herdr space, by the id kept when it was made, and whether Claude Code runs in it; None once the space
/// is gone.
fn herdr_space() -> Option<(String, bool)> {
    let space = std::fs::read_to_string(space_file()).ok()?.trim().to_string();
    if space.is_empty() {
        return None;
    }
    frontend::with(Some(Frontend::Tui), || {
        if !space_listed(&frontend::herdr(&["workspace", "list"])?, &space) {
            return None;
        }
        let alive = frontend::herdr(&["pane", "list"]).is_some_and(|panes| space_runs_claude(&panes, &space));
        Some((space, alive))
    })
}

/// Whether the fetcher runs Claude Code: its `hc` terminal by name, or a pane of its herdr space.
fn running(via: Via) -> bool {
    match via {
        Via::Hc => hc(&["get", FETCHER]).is_some_and(|got| got["state"] == "live" && got["foreground"]["name"].as_str().unwrap_or_default().contains("claude")),
        Via::Herdr => herdr_space().is_some_and(|(_, alive)| alive),
    }
}

/// Stops the fetcher: its `hc` terminal, or its herdr space with whatever is left in it.
fn kill(via: Via) {
    match via {
        Via::Hc => {
            let _ = hc(&["kill", FETCHER]);
        }
        Via::Herdr => {
            if let Some((space, _)) = herdr_space() {
                frontend::with(Some(Frontend::Tui), || frontend::close_space(&space));
            }
            let _ = std::fs::remove_file(space_file());
        }
    }
}

/// Claude Code's executable, when it is installed.
fn claude() -> Option<String> {
    agent::get("claude").filter(agent::Agent::installed).map(|a| a.path())
}

/// Where the version of the hub that launched the fetcher is kept: a fetcher launched by another runs an older plugin.
fn launched_by() -> std::path::PathBuf {
    files::cache().join("fetcher.version")
}

fn tend() {
    let claude = claude();
    let wanted = claude.is_some() && config::load().0["fetcher"] != false && !mcp_sources().is_empty();
    let current = std::fs::read_to_string(launched_by()).is_ok_and(|v| v.trim() == env!("CARGO_PKG_VERSION"));
    // Nothing to ask herdr when nothing is wanted and no `hc` terminal or herdr space could hold a fetcher.
    let Some(via) = (if wanted || hc_installed() || space_file().exists() { via() } else { None }) else { return };
    match (wanted, running(via)) {
        (true, false) => launch(via, &claude.unwrap_or_default()),
        // The next round launches it again, on this hub's plugin.
        (true, true) if !current => kill(via),
        (false, true) => kill(via),
        // A space left without Claude Code, or a stale id, goes too.
        (false, false) if via == Via::Herdr => kill(via),
        _ => {}
    }
}

fn launch(via: Via, claude: &str) {
    let _ = std::fs::write(launched_by(), env!("CARGO_PKG_VERSION"));
    let cwd = config::load().repos();
    match via {
        Via::Hc => launch_hc(claude, &cwd),
        Via::Herdr => launch_herdr(claude, &cwd),
    }
}

/// The shell line that starts the fetcher in a herdr pane: an ordinary session at its prompt, not a job's.
fn herdr_line(claude: &str) -> String {
    let quoted = format!("'{}'", claude.replace('\'', "'\\''"));
    format!(
        "env -u CLAUDE_JOB_DIR -u CLAUDE_CODE_SESSION_KIND -u CLAUDECODE -u CLAUDE_CODE_ENTRYPOINT COCKPIT_FETCHER=1 {quoted} --name 'cockpit fetcher' --model haiku"
    )
}

/// Starts the fetcher in a new herdr space kept out of view, after the old one, which a crashed run leaves behind.
fn launch_herdr(claude: &str, cwd: &str) {
    frontend::with(Some(Frontend::Tui), || {
        if let Some((old, _)) = herdr_space() {
            frontend::close_space(&old);
        }
        let _ = std::fs::remove_file(space_file());
        if let Some(pane) = frontend::new_space_behind(cwd, SPACE) {
            // The id first: a space that is not written down could not be found again, and each round would make another.
            if let Some(space) = space_of(&pane) {
                let _ = std::fs::write(space_file(), space);
            }
            frontend::run_in(&pane, &herdr_line(claude));
        }
    });
}

fn launch_hc(claude: &str, cwd: &str) {
    // A finished terminal of that name holds it: it goes first.
    let _ = hc(&["rm", FETCHER]);
    // Not through frontend::hc: its trailing --json would land in Claude's arguments.
    let _ = std::process::Command::new(frontend::program("hc", "HC_BIN"))
        .args(["run", "--json", "--bg", "--no-parent", "--no-notify", "--name", FETCHER, "--cwd", cwd, "--label", "cockpit=fetcher"])
        .args([claude, "--name", "cockpit fetcher", "--model", "haiku"])
        .env_remove("HC_TERMINAL_ID")
        // A hub started from inside a Claude job would pass the job on: the fetcher is an ordinary session at its prompt.
        .env_remove("CLAUDE_JOB_DIR")
        .env_remove("CLAUDE_CODE_SESSION_KIND")
        .env_remove("CLAUDECODE")
        .env_remove("CLAUDE_CODE_ENTRYPOINT")
        .env("COCKPIT_FETCHER", "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_space_runs_claude_when_one_of_its_panes_does() {
        let panes = json!({"panes": [
            {"pane_id": "w1:p1", "workspace_id": "w1", "agent": "claude"},
            {"pane_id": "w2:p1", "workspace_id": "w2", "agent": null},
            {"pane_id": "w3:p1", "workspace_id": "w3", "agent": "codex"}
        ]});
        assert!(space_runs_claude(&panes, "w1"));
        assert!(!space_runs_claude(&panes, "w2"), "a bare shell is not the fetcher");
        assert!(!space_runs_claude(&panes, "w3"), "another agent is not either");
        assert!(!space_runs_claude(&panes, "w9"));
        assert!(!space_runs_claude(&json!({}), "w1"));
    }

    #[test]
    fn the_space_is_known_by_its_id_whatever_herdr_calls_it() {
        assert_eq!(space_of("w2Y:p1"), Some("w2Y"));
        assert_eq!(space_of("w2Y"), None);
        assert_eq!(space_of(":p1"), None);
        let listed = json!({"workspaces": [{"workspace_id": "w2Y", "label": "claude"}, {"workspace_id": "w2H", "label": "  terminal"}]});
        assert!(space_listed(&listed, "w2Y"), "a space renamed after its agent is still the one");
        assert!(!space_listed(&listed, "w9"));
        assert!(!space_listed(&json!({}), "w2Y"));
    }

    #[test]
    fn the_herdr_line_starts_an_ordinary_session_and_quotes_the_path() {
        let line = herdr_line("/home/me/.local/bin/claude");
        assert!(
            line.starts_with(
                "env -u CLAUDE_JOB_DIR -u CLAUDE_CODE_SESSION_KIND -u CLAUDECODE -u CLAUDE_CODE_ENTRYPOINT COCKPIT_FETCHER=1 '/home/me/.local/bin/claude' "
            ),
            "{line}"
        );
        assert!(line.ends_with("--name 'cockpit fetcher' --model haiku"));
        assert!(herdr_line("/odd path/it's").contains("'/odd path/it'\\''s'"), "a quote in the path is closed, escaped and reopened");
        assert!(!frontend::switch_like(&line), "herdr would not read the line as one of its switches");
    }

    #[test]
    fn tickets_the_hub_serves_need_no_session() {
        let config = json!({"tickets": {"enabled": true, "server": "linear"}, "ci": {"enabled": true, "server": "circleci"}});
        assert_eq!(sources(&config, &json!({}), &[]), ["tickets", "ci"]);
        let raw = json!({"servers": {"linear": {"command": "linear-mcp"}}});
        assert_eq!(sources(&config, &raw, &[]), ["ci"]);
        let raw = json!({"servers": {"linear": {"command": "linear-mcp"}, "circleci": {"command": "circleci-mcp"}}});
        assert!(sources(&config, &raw, &[]).is_empty());
    }
}
