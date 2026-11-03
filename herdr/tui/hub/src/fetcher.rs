//! A Claude session the hub keeps for the sources it cannot reach itself: Linear, CircleCI or the calendar without a
//! token, the platform and the monitor's Datadog. Their data comes from MCP connectors, which only a Claude Code session
//! can call, so the hub runs one in a hidden herdr-core terminal (no space, no pane): idle at its prompt, it spends no
//! tokens, and the plugin in it refreshes the inbox, CI, meetings and monitor like any session. `fetcher: false` in the
//! config turns it off; it goes when nothing needs it any more.

use std::time::Duration;

use cockpit_core::files;
use cockpit_core::frontend;
use cockpit_core::resume::FETCHER;
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

/// The sources only an MCP connector reaches, as the config has them: empty when the hub's tokens cover everything.
pub fn mcp_sources() -> Vec<&'static str> {
    let c = config::load();
    let raw = files::load(&files::config_dir().join("config.json")).unwrap_or(Value::Null);
    let on = |section: &str| c.0[section]["enabled"] != false && c.0[section].is_object();
    let has = |section: &str, key: &str| raw[section][key].as_str().is_some_and(|t| !t.trim().is_empty());
    let server = |section: &str| c.0[section]["server"].as_str().is_some_and(|s| !s.is_empty());
    let mut out = Vec::new();
    if on("tickets") && server("tickets") && !has("tickets", "token") {
        out.push("tickets");
    }
    if on("ci") && server("ci") && !has("ci", "token") {
        out.push("ci");
    }
    if c.0["calendar"]["enabled"] == true && !has("calendar", "ics") {
        out.push("calendar");
    }
    if c.0["platform"]["enabled"] == true {
        out.push("platform");
    }
    if c.space_names().iter().any(|s| s == "monitor") {
        out.push("monitor");
    }
    out
}

fn hc(args: &[&str]) -> Option<Value> {
    frontend::hc(args)
}

/// The fetcher's terminal when it runs Claude: `hc get` by its name.
fn running() -> Option<Value> {
    let got = hc(&["get", FETCHER])?;
    (got["state"] == "live" && got["foreground"]["name"].as_str().unwrap_or_default().contains("claude")).then_some(got)
}

fn tend() {
    let wanted = config::load().0["fetcher"] != false && !mcp_sources().is_empty();
    match (wanted, running().is_some()) {
        (true, false) => launch(),
        (false, true) => {
            let _ = hc(&["kill", FETCHER]);
        }
        _ => {}
    }
}

fn launch() {
    // A finished terminal of that name holds it: it goes first.
    let _ = hc(&["rm", FETCHER]);
    let cwd = config::load().repos();
    let claude = files::claude();
    // Not through frontend::hc: its trailing --json would land in Claude's arguments.
    let _ = std::process::Command::new(frontend::program("hc", "HC_BIN"))
        .args(["run", "--json", "--bg", "--no-parent", "--no-notify", "--name", FETCHER, "--cwd", cwd.as_str(), "--label", "cockpit=fetcher"])
        .args([claude.as_str(), "--name", "cockpit fetcher", "--model", "haiku"])
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
