//! A newer cockpit announced once, whatever the agent: every six hours the hub reads the published version, and a click
//! on the notification runs the update. It shares update.json with the Claude plugin's check, so none is told twice.

use serde_json::{Value, json};

use super::{http, news};
use crate::config::Config;

const EVERY_MS: u64 = 6 * 3600 * 1000;
const PUBLISHED: &str = "https://raw.githubusercontent.com/quaresc/cockpit/main/plugin/.claude-plugin/plugin.json";

/// Whether `a` is a later version than `b`, number by number.
pub fn is_newer(a: &str, b: &str) -> bool {
    let parts = |v: &str| v.trim_start_matches('v').split('.').map(|p| p.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    parts(a) > parts(b)
}

/// The state after a look at the published version: notified once per newer version.
pub fn next(state: &Value, latest: Option<&str>, current: &str, now: u64) -> (Value, Option<String>) {
    let mut next = state.clone();
    next["at"] = json!(now);
    let Some(latest) = latest else { return (next, None) };
    next["latest"] = json!(latest);
    if is_newer(latest, current) && state["notified"].as_str() != Some(latest) {
        next["notified"] = json!(latest);
        return (next, Some(latest.to_string()));
    }
    (next, None)
}

pub fn check(config: &Config, now: u64) {
    let path = cockpit_core::files::cache().join("update.json");
    let state = cockpit_core::files::load(&path).unwrap_or(json!({"at": 0}));
    if now.saturating_sub(state["at"].as_u64().unwrap_or(0)) < EVERY_MS {
        return;
    }
    let base = http::base("COCKPIT_PUBLISHED_URL", PUBLISHED);
    if base == "off" {
        return;
    }
    let latest = http::get(&base, &[]).ok().and_then(|v| v["version"].as_str().map(str::to_string));
    let current = env!("CARGO_PKG_VERSION");
    let (next, announce) = next(&state, latest.as_deref(), current, now);
    let _ = cockpit_core::files::write(&path, &next);
    if let Some(latest) = announce {
        news::notify(config, &format!("⬆️ Cockpit {latest}"), &format!("You run {current}. Click to update cockpit and reload what runs it."), "update", &[]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_newer_version_is_announced_once() {
        assert!(is_newer("0.37.0", "0.36.1") && !is_newer("0.36.1", "0.36.1") && is_newer("1.0.0", "0.99.9"));
        let (state, said) = next(&json!({"at": 0}), Some("0.37.0"), "0.36.1", 5);
        assert_eq!((said.as_deref(), &state["notified"]), (Some("0.37.0"), &json!("0.37.0")));
        let (_, again) = next(&state, Some("0.37.0"), "0.36.1", 9);
        assert_eq!(again, None);
        assert_eq!(next(&json!({}), Some("0.36.0"), "0.36.1", 1).1, None);
    }
}
