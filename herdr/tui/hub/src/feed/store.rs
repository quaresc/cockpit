//! Where the feed's data lands. inbox.json: the hub writes the GitHub fields (and `tickets` given a Linear token) plus
//! its own `hub` record, the mod the rest, each through the same lock and an atomic rename; `errors` joins both sides'.
//! ci.json: whole, marked `by: "hub"`. While the hub's part is under three minutes old the mod leaves it alone.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde_json::{Value, json};

use crate::value::list;

/// The shape of inbox.json the apps read, and of ci.json.
pub const INBOX_VERSION: i64 = 1;
pub const CI_VERSION: i64 = 3;
/// The mod's lock is touched every 30 s while held: an older one was left by a dead session.
const LOCK_STALE: Duration = Duration::from_secs(120);

/// The cross-process lock the mod takes too: a directory, created or not in one step.
pub struct Lock(PathBuf);

impl Lock {
    pub fn take(cache: &Path, name: &str) -> Option<Lock> {
        let path = cache.join(format!("{name}.lock"));
        let age = std::fs::metadata(&path).ok().and_then(|m| m.modified().ok()).and_then(|t| SystemTime::now().duration_since(t).ok());
        if age.is_some_and(|age| age > LOCK_STALE) {
            // Renamed away first, so a single process takes a stale lock over.
            let stale = cache.join(format!("{name}.lock.stale-hub{}", std::process::id()));
            if std::fs::rename(&path, &stale).is_ok() {
                let _ = std::fs::remove_dir(&stale);
            }
        }
        let _ = std::fs::create_dir_all(cache);
        std::fs::create_dir(&path).ok().map(|_| Lock(path))
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.0);
    }
}

/// Written beside it, then moved in place, so no reader sees half of it.
pub fn write(path: &Path, value: &Value) -> std::io::Result<()> {
    let nanos = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
    let tmp = path.with_file_name(format!("{}.hub{}-{nanos}.tmp", path.file_name().and_then(|n| n.to_str()).unwrap_or("data"), std::process::id()));
    std::fs::write(&tmp, serde_json::to_string(value)?)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// What one round of the feed brings to inbox.json: the parts it fetched, and its errors (the latest of each source).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InboxPart {
    pub github: Option<[Value; 5]>,
    pub tickets: Option<Vec<Value>>,
    pub errors: Vec<String>,
}

impl InboxPart {
    /// A part not written yet (the lock was taken) followed by a newer one.
    pub fn then(self, newer: InboxPart) -> InboxPart {
        InboxPart { github: newer.github.or(self.github), tickets: newer.tickets.or(self.tickets), errors: newer.errors }
    }
}

const GITHUB_FIELDS: [&str; 5] = ["direct", "team", "botRepos", "mine", "merged"];

pub fn github_fields(github: &super::github::Github) -> [Value; 5] {
    [json!(github.direct), json!(github.team), json!(github.bot_repos), json!(github.mine), json!(github.merged)]
}

/// Whether the inbox holds GitHub data someone fetched, to compare a new fetch against.
pub fn has_github(inbox: &Value) -> bool {
    inbox["version"].as_i64() == Some(INBOX_VERSION) && (inbox["hub"]["github"].is_number() || inbox["modAt"].is_number() || (inbox["hub"].is_null() && inbox["at"].is_number()))
}

/// The inbox with the hub's part merged in; another version's file, or none, starts over.
pub fn merge_inbox(file: &Value, part: &InboxPart, now: u64) -> Value {
    let mut inbox = if file["version"].as_i64() == Some(INBOX_VERSION) {
        file.clone()
    } else {
        json!({"version": INBOX_VERSION, "direct": [], "team": [], "botRepos": [], "mine": [], "tickets": [], "merged": [], "errors": []})
    };
    let mut hub = if inbox["hub"].is_object() { inbox["hub"].clone() } else { json!({}) };
    let theirs: Vec<Value> = list(&inbox["errors"]).iter().filter(|e| !list(&hub["errors"]).contains(e)).cloned().collect();
    if let Some(fields) = &part.github {
        for (name, value) in GITHUB_FIELDS.iter().zip(fields) {
            inbox[*name] = value.clone();
        }
        hub["github"] = json!(now);
    }
    if let Some(tickets) = &part.tickets {
        inbox["tickets"] = json!(tickets);
        hub["tickets"] = json!(now);
    }
    if part.github.is_some() || part.tickets.is_some() || !inbox["at"].is_number() {
        inbox["at"] = json!(now);
    }
    hub["at"] = json!(now);
    hub["errors"] = json!(part.errors);
    inbox["errors"] = json!(theirs.into_iter().chain(part.errors.iter().map(|e| json!(e))).collect::<Vec<_>>());
    inbox["hub"] = hub;
    inbox
}

/// ci.json from the hub's runs, or its error beside the runs it had.
pub fn merge_ci(file: &Value, runs: Result<Vec<Value>, String>, now: u64) -> Value {
    match runs {
        Ok(runs) => json!({"at": now, "runs": runs, "version": CI_VERSION, "by": "hub"}),
        Err(error) => {
            let mut ci = json!({"at": file["at"].as_u64().unwrap_or(0), "runs": file["runs"].as_array().cloned().unwrap_or_default(), "error": error, "by": "hub"});
            if file["version"].as_i64() == Some(CI_VERSION) {
                ci["version"] = json!(CI_VERSION);
            }
            ci
        }
    }
}

/// Reads, merges and writes a cache file under its lock; None when the lock is held (the caller tries again later).
pub fn update(cache: &Path, name: &str, merge: impl FnOnce(&Value) -> Value) -> Option<(Value, Value)> {
    let _lock = Lock::take(cache, name)?;
    let path = cache.join(format!("{name}.json"));
    let before = cockpit_core::files::load(&path).unwrap_or(Value::Null);
    let after = merge(&before);
    write(&path, &after).ok()?;
    Some((before, after))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(errors: &[&str]) -> InboxPart {
        InboxPart {
            github: Some([json!([{"url": "d"}]), json!([]), json!(["api"]), json!([{"url": "m"}]), json!([])]),
            tickets: None,
            errors: errors.iter().map(|e| e.to_string()).collect(),
        }
    }

    #[test]
    fn the_hub_writes_only_its_fields_and_keeps_the_mods() {
        let file = json!({"version": 1, "at": 5, "modAt": 5, "direct": [], "team": [], "botRepos": [], "mine": [], "merged": [],
            "tickets": [{"id": "ENG-1"}], "deploys": {"ENG-1": []}, "previews": [], "errors": ["Previews failed"]});
        let inbox = merge_inbox(&file, &part(&["GitHub: HTTP 502"]), 100);
        assert_eq!(inbox["direct"], json!([{"url": "d"}]));
        assert_eq!(inbox["botRepos"], json!(["api"]));
        assert_eq!((&inbox["tickets"], &inbox["deploys"], &inbox["modAt"]), (&file["tickets"], &file["deploys"], &json!(5)));
        assert_eq!(inbox["errors"], json!(["Previews failed", "GitHub: HTTP 502"]));
        assert_eq!(inbox["hub"], json!({"at": 100, "github": 100, "errors": ["GitHub: HTTP 502"]}));
        assert_eq!(inbox["at"], 100);
        let again = merge_inbox(&inbox, &InboxPart { errors: vec![], ..InboxPart::default() }, 200);
        assert_eq!(again["errors"], json!(["Previews failed"]));
        assert_eq!((&again["at"], &again["hub"]["github"], &again["hub"]["at"]), (&json!(100), &json!(100), &json!(200)));
    }

    #[test]
    fn another_version_or_no_file_starts_over() {
        let inbox = merge_inbox(&json!({"version": 0, "mine": "old"}), &part(&[]), 7);
        assert_eq!(inbox["version"], 1);
        assert_eq!(inbox["tickets"], json!([]));
        assert_eq!(inbox["mine"], json!([{"url": "m"}]));
        assert!(!has_github(&Value::Null));
        assert!(has_github(&inbox));
        assert!(!has_github(&merge_inbox(&Value::Null, &InboxPart::default(), 7)));
        assert!(has_github(&json!({"version": 1, "at": 3})));
    }

    #[test]
    fn tickets_are_the_hubs_only_when_it_fetched_them() {
        let inbox = merge_inbox(&Value::Null, &InboxPart { tickets: Some(vec![json!({"id": "ENG-2"})]), ..InboxPart::default() }, 9);
        assert_eq!(inbox["hub"]["tickets"], 9);
        assert!(inbox["hub"].get("github").is_none());
        let later = InboxPart { github: None, tickets: None, errors: vec!["x".into()] };
        assert_eq!(part(&[]).then(later.clone()).github, part(&[]).github);
        assert_eq!(part(&["old"]).then(later).errors, ["x"]);
    }

    #[test]
    fn a_ci_error_keeps_the_runs_and_their_time() {
        let file = json!({"at": 50, "runs": [{"repo": "api"}], "version": 3});
        assert_eq!(merge_ci(&file, Err("GitHub HTTP 502".into()), 99), json!({"at": 50, "runs": [{"repo": "api"}], "error": "GitHub HTTP 502", "version": 3, "by": "hub"}));
        assert_eq!(merge_ci(&file, Ok(vec![]), 99), json!({"at": 99, "runs": [], "version": 3, "by": "hub"}));
    }

    #[test]
    fn a_held_lock_waits_and_a_stale_one_is_taken_over() {
        let dir = tempfile::tempdir().unwrap();
        let held = Lock::take(dir.path(), "inbox").unwrap();
        assert!(Lock::take(dir.path(), "inbox").is_none());
        assert!(update(dir.path(), "inbox", |_| json!({})).is_none());
        drop(held);
        let (before, after) = update(dir.path(), "inbox", |v| json!({"was": v})).unwrap();
        assert_eq!((before, after), (Value::Null, json!({"was": null})));
        std::fs::create_dir(dir.path().join("ci.lock")).unwrap();
        let old = SystemTime::now() - Duration::from_secs(300);
        std::fs::File::open(dir.path().join("ci.lock")).unwrap().set_modified(old).unwrap();
        assert!(Lock::take(dir.path(), "ci").is_some());
    }
}
