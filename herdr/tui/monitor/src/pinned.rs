//! The duties of the app running in the monitor space: how production does on the space's sidebar row, and the app a
//! notification click asks for (monitor-focus.json, written by `cockpit focus monitor:<app>`).

use std::process::Command;
use std::time::SystemTime;

use cockpit_core::files;

use crate::data::{Doc, State};

pub const ICON: &str = "\u{f0e4}";

/// Run by `cockpit space monitor` in the monitor space: the sidebar row is this app's then.
pub fn is_pinned() -> bool {
    std::env::var("COCKPIT_SPACE").is_ok_and(|s| !s.is_empty())
}

fn plural(n: usize, one: &str) -> String {
    format!("{n} {one}{}", if n == 1 { "" } else { "s" })
}

/// The sidebar's line, and whether it says something is wrong: failed or moved releases and alerts, else the releases
/// still watched, else all good.
pub fn sidebar_text(doc: &Doc) -> (String, bool) {
    let states: Vec<State> = doc.apps.iter().flat_map(|a| a.releases.iter().map(|r| r.verdict.state)).collect();
    let of = |state: State| states.iter().filter(|s| **s == state).count();
    let alerts: u32 = doc.apps.iter().filter_map(|a| a.monitors.as_ref()).map(|m| m.alert).sum();
    let (failed, moved, watching) = (of(State::Failed), of(State::Moved), of(State::Watching));
    let mut parts = Vec::new();
    if failed > 0 {
        parts.push(format!("{} failed", plural(failed, "release")));
    }
    if moved > 0 {
        parts.push(if failed > 0 { format!("{moved} moved") } else { format!("{} moved", plural(moved, "release")) });
    }
    if alerts > 0 {
        parts.push(plural(alerts as usize, "alert"));
    }
    if !parts.is_empty() {
        let icon = if failed > 0 { State::Failed.icon() } else { State::Moved.icon() };
        return (format!("{icon} {}", parts.join(" · ")), true);
    }
    if watching > 0 {
        return (format!("{} {} watching", State::Watching.icon(), plural(watching, "release")), false);
    }
    (format!("{ICON}  all good"), false)
}

/// The text on the monitor space's sidebar row, sent off the key path: in red when something is wrong.
pub fn report(text: String, bad: bool) {
    let Ok(workspace) = std::env::var("HERDR_WORKSPACE_ID") else { return };
    let bin = std::env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".into());
    let (token, other) = if bad { ("monitor_bad", "monitor") } else { ("monitor", "monitor_bad") };
    std::thread::spawn(move || {
        let _ = Command::new(bin)
            .args(["workspace", "report-metadata", &workspace, "--source", "cockpit-monitor", "--token", &format!("{token}={text}"), "--clear-token", other])
            .output();
    });
}

/// The app a notification click asked for since `seen`, taken once.
pub fn take_focus(seen: &mut SystemTime) -> Option<String> {
    let path = files::cache().join("monitor-focus.json");
    let at = std::fs::metadata(&path).and_then(|m| m.modified()).ok()?;
    if at <= *seen {
        return None;
    }
    *seen = at;
    files::load(&path)?["app"].as_str().map(str::to_string)
}
