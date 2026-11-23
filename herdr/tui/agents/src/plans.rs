//! The plans an agent asked to carry out, kept as views of its session (`plan-<session>-<n>`) in the show folder: the
//! side panel lists them beside the agent's other views, numbered, and each stays there to read after it was carried out.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use cockpit_core::files;
use serde_json::{Value, json};

/// The view a plan is kept as for `owner` (an app terminal's address, or a driven session's key), writing it the first
/// time this plan is seen: the next number of the session's plans, titled after the plan's first heading.
pub fn record(owner: &str, session: &str, cwd: &str, plan: &str) -> Option<String> {
    record_in(&files::cache().join("show"), owner, session, cwd, plan, cockpit_core::seen::now_ms())
}

/// `record` in `dir`, at `now` (ms).
pub fn record_in(dir: &Path, owner: &str, session: &str, cwd: &str, plan: &str, now: u64) -> Option<String> {
    let plan = plan.trim();
    let own = short(session);
    if owner.is_empty() || own.is_empty() || plan.is_empty() {
        return None;
    }
    let key = format!("{}\u{1f}{owner}\u{1f}{own}\u{1f}{plan}", dir.display());
    let mut seen = SEEN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let seen = seen.get_or_insert_with(HashMap::new);
    if let Some(name) = seen.get(&key).filter(|name| dir.join(format!("{name}.json")).exists()) {
        return Some(name.clone());
    }
    let prefix = format!("plan-{own}-");
    let mut last = 0;
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let file = entry.file_name().to_string_lossy().into_owned();
        let Some(n) = file.strip_prefix(&prefix).and_then(|rest| rest.strip_suffix(".json")).and_then(|n| n.parse::<u32>().ok()) else { continue };
        last = last.max(n);
        let kept = files::load(&entry.path()).unwrap_or(Value::Null);
        if kept["terminal"] == owner && kept["blocks"][0]["text"].as_str().map(str::trim) == Some(plan) {
            let name = format!("{prefix}{n}");
            seen.insert(key, name.clone());
            return Some(name);
        }
    }
    let n = last + 1;
    let name = format!("{prefix}{n}");
    let spec = json!({
        "title": format!("Plan {n} · {}", title(plan)), "blocks": [{"type": "markdown", "text": plan}], "at": now,
        "session": session, "placement": "right", "cwd": cwd, "terminal": owner, "plan": n,
    });
    std::fs::create_dir_all(dir).ok()?;
    files::write(&dir.join(format!("{name}.json")), &spec).ok()?;
    seen.insert(key, name.clone());
    Some(name)
}

static SEEN: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

/// A session's id as a view name takes it: its first eight letters and digits, lowercased.
fn short(session: &str) -> String {
    session.to_lowercase().chars().filter(char::is_ascii_alphanumeric).take(8).collect()
}

/// What a plan is about: its first heading, else its first line, without Markdown marks, on one short line.
pub fn title(plan: &str) -> String {
    let lines = || plan.lines().map(str::trim).filter(|l| !l.is_empty());
    let line = lines().find(|l| l.starts_with('#')).or_else(|| lines().next()).unwrap_or_default();
    let words = line.trim_start_matches('#').replace(['*', '`'], "");
    let words = words.trim();
    match words.char_indices().nth(TITLE_CHARS) {
        Some((at, _)) => format!("{}…", words[..at].trim_end()),
        None => words.to_string(),
    }
}

/// How long a plan's title runs before it is cut.
const TITLE_CHARS: usize = 60;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_new_plan_of_a_session_is_kept_once_as_its_next_numbered_view() {
        let dir = tempfile::tempdir().unwrap();
        let session = "9D2E0000-0000-4000-8000-000000000412";
        let first = record_in(dir.path(), "t41", session, "/r/api", "# Daily quota\n\n1. Add the column", 1000).unwrap();
        assert_eq!(first, "plan-9d2e0000-1");
        let spec = files::load(&dir.path().join("plan-9d2e0000-1.json")).unwrap();
        assert_eq!(spec["title"], "Plan 1 · Daily quota");
        assert_eq!((spec["terminal"].clone(), spec["placement"].clone(), spec["at"].clone()), (json!("t41"), json!("right"), json!(1000)));
        assert_eq!(spec["blocks"], json!([{"type": "markdown", "text": "# Daily quota\n\n1. Add the column"}]));
        // Seen again on the next look: the same view, untouched.
        assert_eq!(record_in(dir.path(), "t41", session, "/r/api", "# Daily quota\n\n1. Add the column\n", 2000).as_deref(), Some("plan-9d2e0000-1"));
        assert_eq!(files::load(&dir.path().join("plan-9d2e0000-1.json")).unwrap()["at"], 1000);
        // A revised plan is the next one; the first stays to be read.
        assert_eq!(record_in(dir.path(), "t41", session, "/r/api", "# Daily quota, per plan\n\n1. Add the column", 3000).as_deref(), Some("plan-9d2e0000-2"));
        assert!(dir.path().join("plan-9d2e0000-1.json").exists());
        // Found from the folder too, after a hub restart forgot what it saw.
        SEEN.lock().unwrap().take();
        assert_eq!(record_in(dir.path(), "t41", session, "/r/api", "# Daily quota\n\n1. Add the column", 4000).as_deref(), Some("plan-9d2e0000-1"));
        // No owner, no session or no plan: nothing is kept.
        assert_eq!(record_in(dir.path(), "", session, "/r/api", "# P", 1), None);
        assert_eq!(record_in(dir.path(), "t41", "", "/r/api", "# P", 1), None);
        assert_eq!(record_in(dir.path(), "t41", session, "/r/api", "  ", 1), None);
    }

    #[test]
    fn a_plans_title_is_its_first_heading_on_one_short_line() {
        assert_eq!(title("Intro line\n\n## Must-have fixes & features\n\nText"), "Must-have fixes & features");
        assert_eq!(title("\n  Add a **daily_limit** column\n"), "Add a daily_limit column");
        let long = format!("# {}", "word ".repeat(30));
        assert!(title(&long).ends_with('…') && title(&long).chars().count() <= TITLE_CHARS + 1);
    }
}
