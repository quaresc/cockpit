//! The duties of the app running in the meetings space: the next meeting on the space's sidebar row, and the meeting a
//! notification click asks for (meetings-focus.json, written by focus.sh).

use std::process::Command;
use std::time::SystemTime;

use cockpit_core::files;

use crate::week::{Clock, Week, until_text};

pub const ICON_CALENDAR: &str = "\u{f455}";
pub const ICON_NOW: &str = "\u{f444}";
pub const ICON_PEOPLE: &str = "\u{f4fd}";
pub const ICON_JOIN: &str = "\u{f0567}";

/// A response's glyph.
pub fn response_icon(response: &str) -> &'static str {
    match response {
        "tentative" => "\u{f420}",
        "needsAction" => "\u{f444}",
        _ => "\u{f42e}",
    }
}

/// Run by meetings-loop.sh in the meetings space: the sidebar row is this app's then.
pub fn is_pinned() -> bool {
    std::env::var("COCKPIT_SPACE").is_ok_and(|s| !s.is_empty())
}

/// The next meeting, as the sidebar shows it: now, in N min today, else its day and time.
pub fn next_text(week: &Week, clock: &Clock) -> String {
    let Some(next) = week.upcoming(clock).into_iter().next() else { return format!("{ICON_CALENDAR}  no meeting this week") };
    let when = if next.start <= clock.now {
        "now".to_string()
    } else if clock.local(next.start).date() == clock.today() {
        until_text(next.start, clock)
    } else {
        clock.local(next.start).format("%a %H:%M").to_string()
    };
    let title: String = next.title.chars().take(20).collect();
    format!("{ICON_CALENDAR}  {when} · {title}")
}

/// The text on the meetings space's sidebar row, sent off the key path.
pub fn report(text: String) {
    let Ok(workspace) = std::env::var("HERDR_WORKSPACE_ID") else { return };
    let bin = std::env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".into());
    std::thread::spawn(move || {
        let _ = Command::new(bin)
            .args(["workspace", "report-metadata", &workspace, "--source", "cockpit-meetings", "--token", &format!("meetings={text}")])
            .output();
    });
}

/// The meeting a notification click asked for since `seen`, taken once.
pub fn take_focus(seen: &mut SystemTime) -> Option<String> {
    let path = files::cache().join("meetings-focus.json");
    let at = std::fs::metadata(&path).and_then(|m| m.modified()).ok()?;
    if at <= *seen {
        return None;
    }
    *seen = at;
    files::load(&path)?["id"].as_str().map(str::to_string)
}
