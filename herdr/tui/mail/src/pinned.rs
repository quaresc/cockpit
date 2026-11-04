//! The duties of the app running in the mail space: how many messages need me, on the space's sidebar row.

use std::process::Command;

/// Run by `cockpit space mail` in the mail space: the sidebar row is this app's then.
pub fn is_pinned() -> bool {
    std::env::var("COCKPIT_SPACE").is_ok_and(|s| !s.is_empty())
}

/// The text on the mail space's sidebar row, sent off the key path.
pub fn report(text: String) {
    let Ok(workspace) = std::env::var("HERDR_WORKSPACE_ID") else { return };
    let bin = std::env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".into());
    std::thread::spawn(move || {
        let _ = Command::new(bin).args(["workspace", "report-metadata", &workspace, "--source", "cockpit-mail", "--token", &format!("mail={text}")]).output();
    });
}
