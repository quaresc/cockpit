//! A Claude Code session the hub keeps for the sources it cannot reach itself: Linear, CircleCI or the calendar without
//! a token or a server of the hub's own (`servers` in the config), the platform and the monitor's Datadog. Their data comes from MCP connectors that cockpit's Claude Code
//! plugin calls, so the hub runs one in a hidden herdr-core terminal (no space, no pane): idle at its prompt, it spends
//! no tokens, and the plugin in it refreshes the inbox, CI, meetings and monitor like any session. It runs only where
//! Claude Code is installed; `fetcher: false` in the config turns it off, and it goes when nothing needs it any more.

use std::time::Duration;

use cockpit_core::resume::FETCHER;
use cockpit_core::{agent, files, frontend};
use serde_json::Value;

use crate::config;

const EVERY: Duration = Duration::from_secs(60);

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
    if spaces.iter().any(|s| s == "monitor") && !(hubs("platform") && crate::feed::mcp::observability(raw).is_some()) {
        out.push("monitor");
    }
    out
}

fn hc(args: &[&str]) -> Option<Value> {
    frontend::hc(args)
}

/// The fetcher's terminal when it runs Claude Code: `hc get` by its name.
fn running() -> Option<Value> {
    let got = hc(&["get", FETCHER])?;
    (got["state"] == "live" && got["foreground"]["name"].as_str().unwrap_or_default().contains("claude")).then_some(got)
}

/// Claude Code's executable, when it is installed.
fn claude() -> Option<String> {
    agent::get("claude").filter(agent::Agent::installed).map(|a| a.path())
}

fn tend() {
    let claude = claude();
    let wanted = claude.is_some() && config::load().0["fetcher"] != false && !mcp_sources().is_empty();
    match (wanted, running().is_some()) {
        (true, false) => launch(&claude.unwrap_or_default()),
        (false, true) => {
            let _ = hc(&["kill", FETCHER]);
        }
        _ => {}
    }
}

fn launch(claude: &str) {
    // A finished terminal of that name holds it: it goes first.
    let _ = hc(&["rm", FETCHER]);
    let cwd = config::load().repos();
    // Not through frontend::hc: its trailing --json would land in Claude's arguments.
    let _ = std::process::Command::new(frontend::program("hc", "HC_BIN"))
        .args(["run", "--json", "--bg", "--no-parent", "--no-notify", "--name", FETCHER, "--cwd", cwd.as_str(), "--label", "cockpit=fetcher"])
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
    fn tickets_the_hub_serves_need_no_session() {
        let config = json!({"tickets": {"enabled": true, "server": "linear"}, "ci": {"enabled": true, "server": "circleci"}});
        assert_eq!(sources(&config, &json!({}), &[]), ["tickets", "ci"]);
        let raw = json!({"servers": {"linear": {"command": "linear-mcp"}}});
        assert_eq!(sources(&config, &raw, &[]), ["ci"]);
        let raw = json!({"servers": {"linear": {"command": "linear-mcp"}, "circleci": {"command": "circleci-mcp"}}});
        assert!(sources(&config, &raw, &[]).is_empty());
    }
}
