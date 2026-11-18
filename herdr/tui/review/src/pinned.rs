//! The duties of the app running in the review space: what waits for review on the space's sidebar row.

use std::process::Command;

use crate::app::App;

pub const ICON: &str = "\u{f4af}";

/// Run by `cockpit space review` in the review space: the sidebar row is this app's then.
pub fn is_pinned() -> bool {
    std::env::var("COCKPIT_SPACE").is_ok_and(|s| !s.is_empty())
}

/// The sidebar's line: the comments waiting to be sent, else the scope's changes, else that there is nothing to review.
pub fn sidebar_text(app: &App) -> String {
    let Some(agent) = app.subject() else { return format!("{ICON}  no agent yet") };
    let here = app.here();
    let notes = here.diff.notes().len();
    if notes > 0 {
        return format!("{ICON}  {notes} comment{} to send", if notes == 1 { "" } else { "s" });
    }
    let files = here.diff.files();
    if here.error.is_some() || here.read_at.is_none() {
        return format!("{ICON}  {}", agent.repo_name());
    }
    if files.is_empty() {
        return format!("{ICON}  {} clean", agent.repo_name());
    }
    let (added, removed) = files.iter().fold((0, 0), |(a, r), f| (a + f.added, r + f.removed));
    format!("{ICON}  {} · {} file{} +{added} −{removed}", agent.repo_name(), files.len(), if files.len() == 1 { "" } else { "s" })
}

/// The text on the review space's sidebar row, sent off the key path.
pub fn report(text: String) {
    let Ok(workspace) = std::env::var("HERDR_WORKSPACE_ID") else { return };
    let bin = std::env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".into());
    std::thread::spawn(move || {
        let _ = Command::new(bin)
            .args(["workspace", "report-metadata", &workspace, "--source", "cockpit-review", "--token", &format!("review_space={text}")])
            .output();
    });
}
