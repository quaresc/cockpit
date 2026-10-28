//! A palette jump to another app: its pinned space comes into view, then the app is asked to bring the item into view.

use cockpit_core::{files, focus};
use serde_json::{Value, json};

use crate::py::list;

/// The pinned space an app runs in; show views live beside their session, in no space of their own.
fn space_of(app: &str) -> Option<&'static str> {
    match app {
        "inbox" => Some("inbox"),
        "runbook" => Some("runbook"),
        "todo" | "to-do" => Some("to-do"),
        "meetings" => Some("meetings"),
        _ => None,
    }
}

pub fn go(target: &Value) -> Value {
    let Some(app) = target["app"].as_str().filter(|a| !a.is_empty()) else {
        return json!({"error": "the target names no app"});
    };
    focus::request(app, target);
    if let Some(space) = space_of(app)
        && let Some(listed) = files::herdr(&["workspace", "list"])
    {
        let suffix = format!("  {space}");
        let found = list(&listed["result"]["workspaces"])
            .iter()
            .find(|w| w["label"].as_str().is_some_and(|l| l.ends_with(&suffix)))
            .and_then(|w| w["workspace_id"].as_str());
        if let Some(id) = found {
            files::herdr(&["workspace", "focus", id]);
        }
    }
    json!({"ok": true})
}
