//! A newer cockpit announced once, whatever the agent: every six hours the hub reads the published version and the
//! changelog's sections above the running one, and a click on the notification runs the update. It shares update.json
//! with the Claude plugin's check, so none is told twice, and the Herdr app's sidebar draws its tag from the same file.

use serde_json::{Value, json};

use super::{http, news};
use crate::config::Config;

const EVERY_MS: u64 = 6 * 3600 * 1000;
const PUBLISHED: &str = "https://raw.githubusercontent.com/quaresc/cockpit/main/plugin/.claude-plugin/plugin.json";
const CHANGELOG: &str = "https://raw.githubusercontent.com/quaresc/cockpit/main/CHANGELOG.md";

/// Whether `a` is a later version than `b`, number by number.
pub fn is_newer(a: &str, b: &str) -> bool {
    let parts = |v: &str| v.trim_start_matches('v').split('.').map(|p| p.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    parts(a) > parts(b)
}

/// The state after a look at the published version: notified once per newer version.
pub fn next(state: &Value, latest: Option<&str>, current: &str, now: u64) -> (Value, Option<String>) {
    let mut next = state.clone();
    next["at"] = json!(now);
    next["current"] = json!(current);
    let Some(latest) = latest else { return (next, None) };
    next["latest"] = json!(latest);
    if is_newer(latest, current) && state["notified"].as_str() != Some(latest) {
        next["notified"] = json!(latest);
        return (next, Some(latest.to_string()));
    }
    (next, None)
}

/// The changelog's sections above `current`, newest first, as `{version, date, changes}`; a bullet wrapped over
/// several lines comes back as one.
pub fn notes(changelog: &str, current: &str) -> Vec<Value> {
    let mut sections: Vec<(String, String, Vec<String>)> = Vec::new();
    let mut open = false;
    for line in changelog.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            let (version, date) = heading.split_once('(').map_or((heading, ""), |(v, d)| (v, d.trim().trim_end_matches(')')));
            open = is_newer(version.trim(), current);
            if open {
                sections.push((version.trim().to_string(), date.to_string(), Vec::new()));
            }
        } else if open && let Some((.., changes)) = sections.last_mut() {
            if let Some(bullet) = line.strip_prefix("- ") {
                changes.push(bullet.trim().to_string());
            } else if !line.trim().is_empty()
                && let Some(last) = changes.last_mut()
            {
                last.push(' ');
                last.push_str(line.trim());
            }
        }
    }
    sections.into_iter().map(|(version, date, changes)| json!({"version": version, "date": date, "changes": changes})).collect()
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
    let (mut next, announce) = next(&state, latest.as_deref(), current, now);
    // A checkout running its own build updates by building again, so the sidebar is told to leave its tag out.
    next["dev"] = json!(cockpit_update::dev_build(&cockpit_core::files::herdr_dir()));
    next["notes"] = match latest.as_deref().filter(|l| is_newer(l, current)) {
        Some(_) => json!(http::text(&http::base("COCKPIT_CHANGELOG_URL", CHANGELOG), &[]).map(|text| notes(&text, current)).unwrap_or_default()),
        None => json!([]),
    };
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

    #[test]
    fn the_notes_are_the_sections_above_the_running_version() {
        let changelog = "# Changelog\n\nA line about the file.\n\n## 0.38.0 (2026-11-09)\n\n- A chart keeps its unit\n- A bullet that wraps\n  onto a second line\n\n## 0.37.1 (2026-11-08)\n\n- Older\n\n## 0.37.0 (2026-11-01)\n\n- Older still\n";
        let notes = super::notes(changelog, "0.37.0");
        assert_eq!(notes.len(), 2);
        assert_eq!((notes[0]["version"].as_str(), notes[0]["date"].as_str()), (Some("0.38.0"), Some("2026-11-09")));
        assert_eq!(notes[0]["changes"], json!(["A chart keeps its unit", "A bullet that wraps onto a second line"]));
        assert_eq!(notes[1]["changes"], json!(["Older"]), "the section after a skipped one keeps its own bullets");
        assert!(super::notes("## 0.37.0 (2026-11-01)\n\n- Nothing new\n", "0.37.0").is_empty());
    }
}
