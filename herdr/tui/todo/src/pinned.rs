//! The duties of the app running in the to-do space: the open count on the space's sidebar row.

use std::process::Command;

pub const ICON_TODO: &str = "\u{f45e}";
pub const ICON_OPEN: &str = "\u{f10c}";
pub const ICON_DONE: &str = "\u{f058}";
pub const ICON_ADD: &str = "\u{f067}";
pub const ICON_EDIT: &str = "\u{f040}";
pub const ICON_FOLD: &str = "\u{f44a}";
pub const ICON_UNFOLD: &str = "\u{f47c}";

/// Run by todo-loop.sh in the to-do space: the sidebar count is this app's then.
pub fn is_pinned() -> bool {
    std::env::var("COCKPIT_SPACE").is_ok_and(|s| !s.is_empty())
}

/// The sidebar text for this many open items, as todo.py words it.
pub fn count_text(open: usize) -> String {
    if open > 0 { format!("{ICON_OPEN}  {open} open") } else { format!("{ICON_DONE}  all done") }
}

/// The count on the to-do space's sidebar row, sent off the key path.
pub fn report(text: String) {
    let Ok(workspace) = std::env::var("HERDR_WORKSPACE_ID") else { return };
    let bin = std::env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".into());
    std::thread::spawn(move || {
        let _ = Command::new(bin).args(["workspace", "report-metadata", &workspace, "--source", "cockpit-todo", "--token", &format!("todo={text}")]).output();
    });
}
