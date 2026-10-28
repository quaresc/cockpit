//! actions.py: the pinned actions space, a tab per one-off Claude session, remembered by key in actions.json so pressing
//! the same action again brings its tab back.

use std::path::PathBuf;

use cockpit_core::files;
use serde_json::{Map, Value};

use crate::config::Config;
use crate::herdr;
use crate::py::{ordered, text, truthy};

fn panes_path() -> PathBuf {
    files::cache().join("actions.json")
}

fn panes() -> Vec<(String, Value)> {
    std::fs::read_to_string(panes_path()).map(|t| ordered(&t)).unwrap_or_default()
}

fn write_panes(entries: Vec<(String, Value)>) {
    let _ = files::write(&panes_path(), &Value::Object(entries.into_iter().collect::<Map<_, _>>()));
}

/// The actions space's id; None when it is not among the pinned spaces or not open.
pub fn space(config: &Config) -> Option<String> {
    if !config.space_names().iter().any(|n| n == "actions") {
        return None;
    }
    let listed = herdr::result(&["workspace", "list"])?;
    crate::py::list(&listed["workspaces"]).iter().find(|w| w["label"].as_str().is_some_and(|l| l.ends_with("  actions"))).map(|w| text(&w["workspace_id"]))
}

/// The pane where the action's Claude session still runs; None once it has ended or its tab closed.
pub fn pane_of(key: &str) -> Option<String> {
    let pane = panes().into_iter().find(|(k, _)| k == key).map(|(_, p)| p).filter(truthy)?;
    let pane = text(&pane);
    let found = herdr::result(&["pane", "get", &pane]).filter(truthy)?;
    truthy(&found["pane"]["agent"]).then_some(pane)
}

/// Brings the pane's tab into view, with its space.
pub fn focus(pane: &str) -> bool {
    let Some(found) = herdr::result(&["pane", "get", pane]).filter(truthy) else {
        return false;
    };
    herdr::result(&["tab", "focus", &text(&found["pane"]["tab_id"])]).is_some()
}

/// A new tab in the actions space, in view: its pane id, kept under `key`; None without the space.
pub fn open_tab(config: &Config, cwd: &str, label: &str, key: Option<&str>) -> Option<String> {
    let ws = space(config)?;
    let created = herdr::result(&["tab", "create", "--workspace", &ws, "--cwd", cwd, "--label", label, "--focus"]).filter(truthy)?;
    let pane = text(&created["root_pane"]["pane_id"]);
    if let Some(key) = key {
        let mut live: Vec<(String, Value)> =
            panes().into_iter().filter(|(_, p)| herdr::result(&["pane", "get", &text(p)]).is_some_and(|f| truthy(&f))).collect();
        match live.iter_mut().find(|(k, _)| k == key) {
            Some(entry) => entry.1 = Value::from(pane.clone()),
            None => live.push((key.to_string(), Value::from(pane.clone()))),
        }
        write_panes(live);
    }
    Some(pane)
}

/// Moves the action's tab out to a space of its own, in view, once it turns into work; false when it no longer runs.
pub fn promote(key: &str, label: &str) -> bool {
    let Some(pane) = pane_of(key) else {
        return false;
    };
    if !herdr::result(&["pane", "move", &pane, "--new-workspace", "--label", label, "--focus"]).is_some_and(|r| truthy(&r)) {
        return false;
    }
    write_panes(panes().into_iter().filter(|(k, _)| k != key).collect());
    true
}
