//! The duties of the app running in the routines space: how the routines do on the space's sidebar row.

use std::process::Command;

use crate::data::{Clock, Doc, State};

pub const ICON: &str = "\u{f017}";

/// Run by `cockpit space routines` in the routines space: the sidebar row is this app's then.
pub fn is_pinned() -> bool {
    std::env::var("COCKPIT_SPACE").is_ok_and(|s| !s.is_empty())
}

/// The sidebar's line, and whether it says something is wrong: what runs, else the routines whose last run failed,
/// else the next run.
pub fn sidebar_text(doc: &Doc, clock: &Clock) -> (String, bool) {
    let count = |state: State| doc.routines.iter().filter(|r| doc.state(r) == state).count();
    let (running, failed) = (count(State::Running), count(State::Failed) + count(State::Broken));
    if running > 0 {
        let failing = if failed > 0 { format!(" · {failed} failed") } else { String::new() };
        return (format!("\u{f04b}  {running} running{failing}"), failed > 0);
    }
    if failed > 0 {
        return (format!("\u{f00d}  {failed} failed"), true);
    }
    let next = doc.routines.iter().filter(|r| !r.paused).filter_map(|r| r.next).min();
    match next {
        Some(at) => (format!("{ICON}  next {}", clock.until(at)), false),
        None if doc.routines.is_empty() => (format!("{ICON}  none yet"), false),
        None => (format!("{ICON}  {} waiting", doc.routines.len()), false),
    }
}

/// The text on the routines space's sidebar row, sent off the key path: in red when something is wrong.
pub fn report(text: String, bad: bool) {
    let Ok(workspace) = std::env::var("HERDR_WORKSPACE_ID") else { return };
    let bin = std::env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".into());
    let (token, other) = if bad { ("routines_bad", "routines") } else { ("routines", "routines_bad") };
    std::thread::spawn(move || {
        let _ = Command::new(bin)
            .args(["workspace", "report-metadata", &workspace, "--source", "cockpit-routines", "--token", &format!("{token}={text}"), "--clear-token", other])
            .output();
    });
}
