//! A palette jump to another app: its pinned space comes into view, then the app is asked to bring the item into view.
//! A past Claude session opens instead as a fork in a tab of the actions space.

use std::path::Path;

use cockpit_core::{actions, files, focus};
use serde_json::{Value, json};

use crate::value::{cut, join, list};
use crate::{config, herdr};

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
    if app == "session" {
        return session(target);
    }
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

/// A fork of the past session, from its own folder, asked where it stood; its tab comes back into view while it runs.
fn session(target: &Value) -> Value {
    let id = target["key"].as_str().unwrap_or_default();
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return json!({"error": format!("not a session id: {id:?}")});
    }
    let Some(cwd) = target["cwd"].as_str().filter(|c| Path::new(c).is_dir()) else {
        return json!({"error": format!("the folder of session {id} is gone")});
    };
    let key = format!("session {id}");
    if let Some(pane) = actions::pane_of(&key)
        && actions::focus(&pane)
    {
        return json!({"ok": true, "note": "the session is already open: focused its tab"});
    }
    let title = target["title"].as_str().map(str::trim).filter(|t| !t.is_empty()).unwrap_or(id);
    let label = format!("↩ {}", cut(title, 28));
    let prompt = "This is a fork of a past session, opened from cockpit to look back at it. Sum up in a few lines where it \
                  stood: what was done, what was decided, what was left open. Then wait for my questions. Change nothing (no \
                  edits, commits, pushes or ticket updates) unless I ask.";
    let line = join(&["claude", "--resume", id, "--fork-session", prompt].map(str::to_string));
    let pane = actions::open_tab(&config::load().space_names(), cwd, &label, Some(&key)).or_else(|| {
        let created = herdr::run(&["workspace", "create", "--cwd", cwd, "--label", &label, "--focus"]);
        herdr::result_of(&created).and_then(|r| r["root_pane"]["pane_id"].as_str().map(str::to_string))
    });
    let Some(pane) = pane else {
        return json!({"error": "herdr opened no tab for the session"});
    };
    herdr::run(&["pane", "run", &pane, &line]);
    json!({"ok": true, "note": format!("opened a fork of \"{title}\"")})
}
