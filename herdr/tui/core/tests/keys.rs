//! Every app's buttons against the shared keys: a shared verb is on its key, and an app's own verb never takes one.

use std::path::Path;

use cockpit_core::keys::{VERBS, key_of};
use regex::Regex;

const APPS: [&str; 5] = ["inbox", "runbook", "todo", "meetings", "show"];

/// Each button written with a literal label: (file, label, key).
fn buttons() -> Vec<(String, String, char)> {
    let pattern = Regex::new(r#"(?:RowAct::new|RowAct::pick|Act::new)\(\s*"([^"]+)",\s*'(.)'"#).unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut found = Vec::new();
    for app in APPS {
        for entry in std::fs::read_dir(root.join(app).join("src")).unwrap().flatten() {
            let text = std::fs::read_to_string(entry.path()).unwrap();
            let file = format!("{app}/src/{}", entry.file_name().to_string_lossy());
            for c in pattern.captures_iter(&text) {
                found.push((file.clone(), c[1].to_string(), c[2].chars().next().unwrap()));
            }
        }
    }
    found
}

#[test]
fn shared_verbs_keep_their_keys_and_no_app_takes_one() {
    let found = buttons();
    assert!(found.len() > 30, "the scan found only {} buttons", found.len());
    let mut wrong = Vec::new();
    for (file, label, key) in &found {
        match key_of(label) {
            Some(shared) if shared != *key => wrong.push(format!("{file}: \"{label}\" on {key}, the shared key is {shared}")),
            None if VERBS.iter().any(|(k, _)| k == key) => wrong.push(format!("{file}: \"{label}\" takes {key}, a shared key")),
            _ => {}
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
