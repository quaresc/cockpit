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

/// Why the published version could not be read, in a line a person reads.
fn why(failure: &http::Failure) -> String {
    match failure {
        http::Failure::Limited(_) => "GitHub is rate limiting this machine: try again in a minute".to_string(),
        http::Failure::Other(message) => match message.strip_prefix(http::UNREACHABLE) {
            Some(detail) => format!("GitHub could not be reached ({})", detail.trim()),
            None => format!("GitHub did not give the published version ({message})"),
        },
    }
}

/// The state once a look failed: when it was, and why, until a look that works.
fn failed(state: &Value, current: &str, now: u64, message: &str) -> Value {
    let mut next = state.clone();
    next["at"] = json!(now);
    next["current"] = json!(current);
    next["error"] = json!({"message": message, "at": now});
    next
}

/// One look at what is published, written to update.json: the state it leaves and the version to announce, or why the
/// look failed. A failed look keeps the last version read.
fn look(state: &Value, now: u64) -> (Value, Result<Option<String>, String>) {
    let current = env!("CARGO_PKG_VERSION");
    let base = http::base("COCKPIT_PUBLISHED_URL", PUBLISHED);
    let latest = match http::get(&base, &[]) {
        Ok(published) => published["version"].as_str().map(str::to_string).ok_or_else(|| "what GitHub published names no version".to_string()),
        Err(failure) => Err(why(&failure)),
    };
    let latest = match latest {
        Ok(latest) => latest,
        Err(message) => return (failed(state, current, now, &message), Err(message)),
    };
    let (mut next, announce) = next(state, Some(&latest), current, now);
    if let Some(found) = next.as_object_mut() {
        found.remove("error");
    }
    // A checkout running its own build updates by building again, so the sidebar is told to leave its tag out.
    next["dev"] = json!(cockpit_update::dev_build(&cockpit_core::files::herdr_dir()));
    next["notes"] = match is_newer(&latest, current) {
        true => json!(http::text(&http::base("COCKPIT_CHANGELOG_URL", CHANGELOG), &[]).map(|text| notes(&text, current)).unwrap_or_default()),
        false => json!([]),
    };
    (next, Ok(announce))
}

fn path() -> std::path::PathBuf {
    cockpit_core::files::cache().join("update.json")
}

pub fn check(config: &Config, now: u64) {
    let state = cockpit_core::files::load(&path()).unwrap_or(json!({"at": 0}));
    if now.saturating_sub(state["at"].as_u64().unwrap_or(0)) < EVERY_MS {
        return;
    }
    if http::base("COCKPIT_PUBLISHED_URL", PUBLISHED) == "off" {
        return;
    }
    let (next, looked) = look(&state, now);
    let _ = cockpit_core::files::write(&path(), &next);
    if let Ok(Some(latest)) = looked {
        news::notify(config, &format!("⬆️ Cockpit {latest}"), &format!("You run {}: click for the update.", env!("CARGO_PKG_VERSION")), "update", &[]);
    }
}

/// What a look asked for says: the version that is out, or that this one is the latest.
pub fn said(latest: &str, current: &str) -> String {
    if is_newer(latest, current) { format!("{latest} is out") } else { "Up to date".to_string() }
}

/// How long the answer of a look asked for is given again rather than asking GitHub again.
const FLOOR_MS: u64 = 45_000;

/// The answer of a look asked for under `FLOOR_MS` ago: what it found, or why it failed.
fn recent(state: &Value, now: u64) -> Option<Result<String, String>> {
    let looked = state["looked_at"].as_u64().filter(|at| now.saturating_sub(*at) < FLOOR_MS)?;
    if state["error"]["at"].as_u64() == Some(looked) {
        return Some(Err(state["error"]["message"].as_str().unwrap_or("the last check failed").to_string()));
    }
    Some(Ok(said(state["latest"].as_str()?, env!("CARGO_PKG_VERSION"))))
}

/// The state after a look asked for: when it ran is `looked_at`, `at` stays the six-hourly look's, and the version is
/// noted as announced only when a person asked, so an agent's look leaves the announcement to come.
fn asked_look(before: &Value, mut next: Value, now: u64, person: bool) -> Value {
    next["at"] = before.get("at").cloned().unwrap_or(json!(0));
    next["looked_at"] = json!(now);
    if next.get("error").is_some() {
        next["error"]["at"] = json!(now);
    }
    if !person && let Some(state) = next.as_object_mut() {
        match before.get("notified") {
            Some(notified) => state.insert("notified".into(), notified.clone()),
            None => state.remove("notified"),
        };
    }
    next
}

/// A look now for whoever asks, the last answer again within `FLOOR_MS`: update.json is written, nothing is announced
/// (the answer is the news), and a failure says why.
pub fn check_now(now: u64) -> Result<String, String> {
    if http::base("COCKPIT_PUBLISHED_URL", PUBLISHED) == "off" {
        return Err("Update checks are turned off here (COCKPIT_PUBLISHED_URL=off)".into());
    }
    let state = cockpit_core::files::load(&path()).unwrap_or(json!({"at": 0}));
    if let Some(answer) = recent(&state, now) {
        return answer;
    }
    let (next, looked) = look(&state, now);
    let next = asked_look(&state, next, now, cockpit_core::peer::person("update check").is_ok());
    let _ = cockpit_core::files::write(&path(), &next);
    looked.map(|_| said(next["latest"].as_str().unwrap_or_default(), env!("CARGO_PKG_VERSION")))
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

    #[test]
    fn a_failed_look_keeps_the_last_version_and_says_why_until_one_works() {
        let state = json!({"at": 1, "latest": "0.41.0", "notified": "0.41.0", "notes": [{"version": "0.41.0"}]});
        let failed = failed(&state, "0.40.4", 9, "GitHub could not be reached (io: timeout)");
        assert_eq!((failed["at"].as_u64(), failed["latest"].as_str(), failed["error"]["at"].as_u64()), (Some(9), Some("0.41.0"), Some(9)));
        assert_eq!(failed["error"]["message"], "GitHub could not be reached (io: timeout)");
        assert_eq!(failed["notes"], state["notes"]);
        assert_eq!(why(&http::Failure::Other(format!("{} io: Connection refused", http::UNREACHABLE))), "GitHub could not be reached (io: Connection refused)");
        assert_eq!(why(&http::Failure::Other("HTTP 404: Not Found".into())), "GitHub did not give the published version (HTTP 404: Not Found)");
        assert!(why(&http::Failure::Limited(0)).contains("rate limiting"));
    }

    #[test]
    fn a_look_asked_for_says_what_is_out() {
        assert_eq!(said("0.41.0", "0.40.4"), "0.41.0 is out");
        assert_eq!(said("0.40.4", "0.40.4"), "Up to date");
        assert_eq!(said("0.40.3", "0.40.4"), "Up to date", "a build ahead of what is published is up to date");
    }

    #[test]
    fn a_look_asked_for_keeps_the_six_hourly_clock_and_announces_only_for_a_person() {
        let before = json!({"at": 100, "current": "0.40.4", "latest": "0.40.4"});
        let (found, _) = next(&before, Some("0.41.0"), "0.40.4", 5_000);
        let by_agent = asked_look(&before, found.clone(), 5_000, false);
        assert_eq!((by_agent["at"].as_u64(), by_agent["looked_at"].as_u64(), by_agent["latest"].as_str()), (Some(100), Some(5_000), Some("0.41.0")));
        assert!(by_agent.get("notified").is_none(), "an agent's look leaves the announcement to the six-hourly look");
        assert_eq!(asked_look(&before, found, 5_000, true)["notified"], "0.41.0");
        let failed = asked_look(&before, failed(&before, "0.40.4", 7_000, "GitHub could not be reached"), 7_000, false);
        assert_eq!((failed["at"].as_u64(), failed["error"]["at"].as_u64()), (Some(100), Some(7_000)));
    }

    #[test]
    fn a_look_asked_for_again_soon_gives_the_last_answer() {
        let current = env!("CARGO_PKG_VERSION");
        let state = json!({"at": 100, "looked_at": 10_000, "latest": "999.0.0"});
        assert_eq!(recent(&state, 10_000 + FLOOR_MS - 1), Some(Ok("999.0.0 is out".to_string())));
        assert_eq!(recent(&state, 10_000 + FLOOR_MS), None, "past the floor GitHub is asked again");
        assert_eq!(recent(&json!({"at": 100, "looked_at": 10_000, "latest": current}), 10_001), Some(Ok("Up to date".to_string())));
        let failed = json!({"at": 100, "looked_at": 10_000, "latest": current, "error": {"message": "GitHub could not be reached", "at": 10_000}});
        assert_eq!(recent(&failed, 10_001), Some(Err("GitHub could not be reached".to_string())));
        assert_eq!(recent(&json!({"at": 10_000, "latest": current}), 10_001), None, "only a look asked for is given again");
    }
}
