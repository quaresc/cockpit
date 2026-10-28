//! History for the apps' sparklines, one JSON line per point in ~/.cache/cockpit/history/<kind>.jsonl: `ticket_cost`
//! (what each ticket has cost so far, a point whenever it changes) and `ci_duration` (seconds from a CI run's start to
//! when the hub first saw it finished, a point per run), 60 points per key at most.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::value::{list, now, text};

const KEEP: usize = 60;
/// A run counts only when the hub saw it running this recently, so a run that ended while no hub ran is skipped.
const SEEN_RECENTLY: f64 = 600.0;

fn dir(cache: &Path) -> PathBuf {
    cache.join("history")
}

fn points(cache: &Path, kind: &str) -> Vec<Value> {
    let path = dir(cache).join(format!("{kind}.jsonl"));
    std::fs::read_to_string(path).unwrap_or_default().lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

/// The key's values, oldest first.
pub fn series(cache: &Path, kind: &str, key: &str) -> Vec<f64> {
    points(cache, kind).iter().filter(|p| p["key"].as_str() == Some(key)).filter_map(|p| p["value"].as_f64()).collect()
}

/// Adds points, then keeps the last 60 of each key.
fn append(cache: &Path, kind: &str, new: Vec<Value>) {
    if new.is_empty() {
        return;
    }
    let mut all = points(cache, kind);
    all.extend(new);
    let mut left: HashMap<String, usize> = HashMap::new();
    for p in &all {
        *left.entry(text(&p["key"])).or_default() += 1;
    }
    let kept: Vec<&Value> = all
        .iter()
        .filter(|p| {
            let count = left.get_mut(&text(&p["key"])).expect("counted above");
            let keep = *count <= KEEP;
            *count -= 1;
            keep
        })
        .collect();
    let body: String = kept.iter().map(|p| format!("{p}\n")).collect();
    let path = dir(cache).join(format!("{kind}.jsonl"));
    let _ = std::fs::create_dir_all(dir(cache));
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, body).is_ok() {
        let _ = std::fs::rename(tmp, path);
    }
}

/// What each ticket cost, summed over the sessions' cost files, like inbox.ticket_costs.
fn ticket_costs(cache: &Path) -> HashMap<String, f64> {
    let mut total = HashMap::new();
    for entry in std::fs::read_dir(cache.join("costs")).into_iter().flatten().flatten() {
        if !entry.file_name().to_string_lossy().ends_with(".json") {
            continue;
        }
        let costs = cockpit_core::files::load(&entry.path()).unwrap_or(Value::Null);
        for (ticket, usd) in costs.as_object().into_iter().flatten() {
            *total.entry(ticket.clone()).or_insert(0.0) += usd.as_f64().unwrap_or(0.0);
        }
    }
    total
}

fn record_costs(cache: &Path, at: f64) {
    let mut last: HashMap<String, f64> = HashMap::new();
    for p in points(cache, "ticket_cost") {
        if let Some(value) = p["value"].as_f64() {
            last.insert(text(&p["key"]), value);
        }
    }
    let mut costs: Vec<(String, f64)> = ticket_costs(cache).into_iter().collect();
    costs.sort_by(|a, b| a.0.cmp(&b.0));
    let new = costs
        .into_iter()
        .filter(|(ticket, usd)| last.get(ticket).is_none_or(|was| (was - usd).abs() > 1e-9))
        .map(|(ticket, usd)| json!({"key": ticket, "value": usd, "at": at}))
        .collect();
    append(cache, "ticket_cost", new);
}

fn record_ci(cache: &Path, at: f64) {
    let state_path = dir(cache).join("ci-running.json");
    let mut running: HashMap<String, f64> = cockpit_core::files::load(&state_path)
        .and_then(|v| v.as_object().map(|m| m.iter().filter_map(|(k, v)| Some((k.clone(), v.as_f64()?))).collect()))
        .unwrap_or_default();
    let known: Vec<String> = points(cache, "ci_duration").iter().filter_map(|p| p["id"].as_str().map(str::to_string)).collect();
    let mut new = Vec::new();
    let ci = cockpit_core::files::load(&cache.join("ci.json")).unwrap_or(Value::Null);
    for run in list(&ci["runs"]) {
        let Some(id) = run["runId"].as_str().or(run["workflowId"].as_str()) else {
            continue;
        };
        if run["isRunning"].as_bool() == Some(true) {
            running.insert(id.to_string(), at);
            continue;
        }
        let Some(seen) = running.remove(id) else {
            continue;
        };
        let started = run["createdAt"].as_str().and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok());
        if let Some(started) = started.filter(|_| at - seen <= SEEN_RECENTLY && !known.iter().any(|k| k == id)) {
            let seconds = at - started.timestamp_millis() as f64 / 1000.0;
            if seconds > 0.0 {
                new.push(json!({"key": format!("{}/{}", text(&run["repo"]), text(&run["branch"])), "value": seconds.round(), "at": at, "id": id}));
            }
        }
    }
    running.retain(|_, seen| at - *seen < 86_400.0);
    append(cache, "ci_duration", new);
    let _ = cockpit_core::files::write(&state_path, &json!(running));
}

/// Records whatever the caches say now; points only for what changed.
pub fn record(cache: &Path) {
    let at = now();
    record_costs(cache, at);
    record_ci(cache, at);
}

#[cfg(test)]
mod tests {
    use super::*;
    use cockpit_core::files::write;

    #[test]
    fn ticket_costs_change_points_only() {
        let cache = tempfile::tempdir().unwrap();
        let cache = cache.path();
        write(&cache.join("costs/s1.json"), &json!({"ENG-401": 1.5})).unwrap();
        write(&cache.join("costs/s2.json"), &json!({"ENG-401": 0.5, "ENG-402": 3})).unwrap();
        record(cache);
        record(cache);
        assert_eq!(series(cache, "ticket_cost", "ENG-401"), [2.0]);
        write(&cache.join("costs/s2.json"), &json!({"ENG-401": 1.0, "ENG-402": 3})).unwrap();
        record(cache);
        assert_eq!(series(cache, "ticket_cost", "ENG-401"), [2.0, 2.5]);
        assert_eq!(series(cache, "ticket_cost", "ENG-402"), [3.0]);
    }

    #[test]
    fn ci_duration_from_running_to_finished() {
        let cache = tempfile::tempdir().unwrap();
        let cache = cache.path();
        let started = chrono::Utc::now() - chrono::Duration::seconds(300);
        let run = |running: bool| json!({"runs": [{"repo": "api", "branch": "main", "runId": "r1", "createdAt": started.to_rfc3339(), "isRunning": running}]});
        write(&cache.join("ci.json"), &run(true)).unwrap();
        record(cache);
        assert!(series(cache, "ci_duration", "api/main").is_empty());
        write(&cache.join("ci.json"), &run(false)).unwrap();
        record(cache);
        record(cache);
        let values = series(cache, "ci_duration", "api/main");
        assert_eq!(values.len(), 1);
        assert!((299.0..=305.0).contains(&values[0]), "{values:?}");
    }

    #[test]
    fn a_run_never_seen_running_is_skipped() {
        let cache = tempfile::tempdir().unwrap();
        let cache = cache.path();
        write(
            &cache.join("ci.json"),
            &json!({"runs": [{"repo": "api", "branch": "x", "runId": "old", "createdAt": "2026-01-01T00:00:00Z", "isRunning": false}]}),
        )
        .unwrap();
        record(cache);
        assert!(series(cache, "ci_duration", "api/x").is_empty());
    }

    #[test]
    fn keeps_sixty_points_per_key() {
        let cache = tempfile::tempdir().unwrap();
        let cache = cache.path();
        append(cache, "ticket_cost", (0..70).map(|i| json!({"key": "A", "value": i})).chain([json!({"key": "B", "value": 1})]).collect());
        let a = series(cache, "ticket_cost", "A");
        assert_eq!((a.len(), a[0], a[59]), (60, 10.0, 69.0));
        assert_eq!(series(cache, "ticket_cost", "B"), [1.0]);
    }
}
