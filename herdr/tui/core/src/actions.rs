//! The pinned actions space: a tab per one-off agent session the inbox or the runbook starts, remembered by key in
//! actions.json so pressing the same action again brings its tab back. The hub and the `cockpit` CLI both open them.

use std::path::PathBuf;

use serde_json::{Map, Value};

use crate::{files, frontend};

/// The spaces pinned when the config names none.
const DEFAULT_SPACES: [&str; 6] = ["inbox", "terminal", "actions", "to-do", "runbook", "meetings"];

fn panes_path() -> PathBuf {
    files::cache().join("actions.json")
}

/// The remembered panes in the order the file lists them, so the open ones are checked with herdr in that order.
fn panes() -> Vec<(String, Value)> {
    std::fs::read_to_string(panes_path()).ok().and_then(|text| serde_json::from_str::<Ordered>(&text).ok()).map(|o| o.0).unwrap_or_default()
}

/// A JSON object's entries in file order.
struct Ordered(Vec<(String, Value)>);

impl<'de> serde::Deserialize<'de> for Ordered {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visit;
        impl<'de> serde::de::Visitor<'de> for Visit {
            type Value = Ordered;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("an object")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Ordered, A::Error> {
                let mut out = Vec::new();
                while let Some(entry) = map.next_entry::<String, Value>()? {
                    out.push(entry);
                }
                Ok(Ordered(out))
            }
        }
        d.deserialize_map(Visit)
    }
}

fn pane_id(value: &Value) -> Option<&str> {
    value.as_str().filter(|p| !p.is_empty())
}

/// A herdr call's `result`, 15 seconds at most; None when it fails, answers no JSON or a null result.
pub fn herdr(args: &[&str]) -> Option<Value> {
    frontend::herdr(args)
}

/// The pinned spaces' names, from the config the mod resolved.
pub fn pinned() -> Vec<String> {
    let config = files::config();
    let Some(spaces) = config.get("spaces").and_then(Value::as_array) else {
        return DEFAULT_SPACES.iter().map(|s| s.to_string()).collect();
    };
    spaces.iter().map(|s| s.get("name").unwrap_or(s).as_str().unwrap_or_default().to_string()).collect()
}

/// The herdr TUI's actions space id; None when it is not among the pinned spaces or not open.
pub fn space(pinned: &[String]) -> Option<String> {
    if !pinned.iter().any(|n| n == "actions") {
        return None;
    }
    let listed = herdr(&["workspace", "list"])?;
    listed["workspaces"]
        .as_array()?
        .iter()
        .find(|w| w["label"].as_str().is_some_and(|l| l.ends_with("  actions")))
        .and_then(|w| w["workspace_id"].as_str().map(str::to_string))
}

/// The pane or terminal where the action's agent session still runs; None once it has ended or its tab closed.
pub fn pane_of(key: &str) -> Option<String> {
    let handle = panes().into_iter().find(|(k, _)| k == key).and_then(|(_, p)| pane_id(&p).map(str::to_string))?;
    frontend::alive(&handle).then_some(handle)
}

/// Brings the pane's tab into view, with its space.
pub fn focus(handle: &str) -> bool {
    frontend::focus(handle)
}

/// A new tab in the actions space, in view: its handle, kept under `key`; None without the space.
pub fn open_tab(pinned: &[String], cwd: &str, label: &str, key: Option<&str>) -> Option<String> {
    let handle = frontend::actions_tab(pinned, cwd, label)?;
    if let Some(key) = key {
        // Only the panes still open are kept, so the file does not grow with every action.
        let mut live: Map<String, Value> = panes().into_iter().filter(|(_, p)| pane_id(p).is_some_and(frontend::exists)).collect();
        live.insert(key.to_string(), Value::from(handle.clone()));
        let _ = files::write(&panes_path(), &Value::Object(live));
    }
    Some(handle)
}

/// Moves the action's tab out to a space of its own, in view, once it turns into work; false when it no longer runs.
pub fn promote(key: &str, label: &str) -> bool {
    let Some(handle) = pane_of(key) else {
        return false;
    };
    if !frontend::promote(&handle, label) {
        return false;
    }
    let kept: Map<String, Value> = panes().into_iter().filter(|(k, _)| k != key).collect();
    let _ = files::write(&panes_path(), &Value::Object(kept));
    true
}
