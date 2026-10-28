//! The duties of the app running in the runbook space: the sidebar count, notification clicks, the mod's refresh
//! requests and the notification a finished background refresh sends.

use std::process::Command;
use std::time::SystemTime;

use cockpit_core::files;
use serde_json::Value;

use crate::book::Book;

pub const ICON: &str = "\u{f405}";

/// Run by `cockpit space runbook` in the runbook space: these duties are this app's then.
pub fn is_pinned() -> bool {
    std::env::var("COCKPIT_SPACE").is_ok_and(|s| !s.is_empty())
}

fn herdr_bin() -> String {
    std::env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".into())
}

/// The count on the runbook space's sidebar row, sent off the key path.
pub fn report(books: &[Book]) {
    let Ok(workspace) = std::env::var("HERDR_WORKSPACE_ID") else { return };
    let left: usize = books.iter().map(Book::todo).sum();
    let text = if books.is_empty() { format!("{ICON}  no runbook") } else { format!("{ICON}  {} runbooks · {left} to do", books.len()) };
    std::thread::spawn(move || {
        let _ = Command::new(herdr_bin())
            .args(["workspace", "report-metadata", &workspace, "--source", "cockpit-runbook", "--token", &format!("runbook={text}")])
            .output();
    });
}

/// A click on a runbook notification since `since`: the runbook's slug and the steps to play, all its done ones by default.
pub fn take_focus(since: &mut SystemTime) -> Option<(String, Vec<String>)> {
    let path = files::cache().join("runbook-focus.json");
    let at = std::fs::metadata(&path).and_then(|m| m.modified()).ok()?;
    if at <= *since {
        return None;
    }
    *since = at;
    let request = files::load(&path).unwrap_or(Value::Null);
    let steps = request["steps"].as_array().map(|s| s.iter().filter_map(|n| n.as_str().map(str::to_string)).collect()).unwrap_or_default();
    Some((request["slug"].as_str()?.to_string(), steps))
}

/// Refreshes the mod asks for, as (slug, reason); each file is renamed before it is read, so one written meanwhile waits.
pub fn take_requests() -> Vec<(String, String)> {
    let dir = files::cache().join("runbook-refresh");
    let Ok(entries) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut names: Vec<String> = entries.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".json")).collect();
    names.sort();
    let mut requests = Vec::new();
    for name in names {
        let taken = dir.join(format!("{name}.taken"));
        if std::fs::rename(dir.join(&name), &taken).is_err() {
            continue;
        }
        let request = files::load(&taken).unwrap_or(Value::Null);
        let _ = std::fs::remove_file(&taken);
        if let Some(slug) = request["slug"].as_str() {
            requests.push((slug.to_string(), request["reason"].as_str().unwrap_or_default().to_string()));
        }
    }
    requests
}

/// Whether the runbook space is the one herdr shows now.
fn is_in_view() -> bool {
    let Some(spaces) = files::herdr(&["workspace", "list"]) else { return false };
    spaces["result"]["workspaces"]
        .as_array()
        .is_some_and(|all| all.iter().any(|w| w["focused"].as_bool() == Some(true) && w["label"].as_str().is_some_and(|l| l.ends_with("  runbook"))))
}

/// A finished background refresh: a notification whose click opens the runbook and plays its new done steps.
pub fn announce(book: &Book, newly: Vec<String>, reason: String) {
    let (title, slug, todo) = (if book.title.is_empty() { book.slug.clone() } else { book.title.clone() }, book.slug.clone(), book.todo());
    std::thread::spawn(move || {
        if is_in_view() {
            return;
        }
        let mut body = if newly.is_empty() {
            format!("no change · {todo} to do")
        } else {
            format!("{} step{} done · {todo} to do", newly.len(), if newly.len() != 1 { "s" } else { "" })
        };
        if !reason.is_empty() {
            body = format!("{reason} · {body}");
        }
        let action = if newly.is_empty() { format!("runbook:{slug}") } else { format!("runbook:{slug}:{}", newly.join(",")) };
        cockpit_core::notify::post(&format!("📋 {title}"), &body, &action, &[]);
    });
}
