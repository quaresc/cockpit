//! `cockpit routine run <slug>`: a routine run now through the hub, waited for, and how it went, for an agent session
//! trying out a routine it just wrote.

use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::proc;

const USAGE: &str = "A routine run now, waited for.

    cockpit routine run <slug>    runs it through the hub, then prints how it went, what it called and its output
                                  (what its precheck said when that skipped it); exits 1 when the run failed";

fn ask(request: Value) -> Value {
    cockpit_core::hub::request(&request, Duration::from_secs(30)).unwrap_or_else(|error| proc::fail(&format!("cockpit routine: {error}")))
}

fn doc() -> Value {
    ask(json!({"op": "store.get", "name": "routines"}))["value"].clone()
}

pub fn main(args: &[String]) {
    let [verb, slug] = args else { proc::fail(USAGE) };
    if verb != "run" {
        proc::fail(USAGE);
    }
    let before = doc()["runs"][slug.as_str()][0].clone();
    let answer = ask(json!({"op": "act", "app": "routines", "action": "run", "args": [slug]}));
    if let Some(error) = answer["error"].as_str() {
        proc::fail(&format!("cockpit routine: {error}"));
    }
    let timeout = doc()["routines"].as_array().into_iter().flatten().find(|r| r["slug"] == slug.as_str()).and_then(|r| r["timeout"].as_u64()).unwrap_or(30);
    let start = Instant::now();
    // A run that calls none of its tools runs once more: the answer is the one no run follows.
    let run = loop {
        std::thread::sleep(Duration::from_secs(2));
        let doc = doc();
        let last = &doc["runs"][slug.as_str()][0];
        let busy = !doc["running"][slug.as_str()].is_null() || doc["queued"].as_array().into_iter().flatten().any(|q| q["slug"] == slug.as_str());
        if !busy && is_new(&before, last) {
            break last.clone();
        }
        // Neither running nor waiting, and no run of it ended: it was dropped.
        if !busy {
            proc::fail(&format!("cockpit routine: {slug} did not run; the routines space says why"));
        }
        if start.elapsed() > Duration::from_secs(timeout * 120 + 60) {
            proc::fail(&format!("cockpit routine: {slug} still runs after {} min", start.elapsed().as_secs() / 60));
        }
    };
    let output = cockpit_core::files::cache().join("routines").join(slug).join(format!("{}.txt", run["id"].as_str().unwrap_or_default()));
    for line in report(&run, std::fs::read_to_string(&output).ok().as_deref()) {
        println!("{line}");
    }
    if run["ok"] != true {
        std::process::exit(1);
    }
}

/// Whether the routine's latest run is not the one it had before: a skip after a skip updates the same record, so its
/// end tells them apart.
fn is_new(before: &Value, last: &Value) -> bool {
    !last.is_null() && (last["id"] != before["id"] || last["ended"] != before["ended"] || last["times"] != before["times"])
}

/// How the run went: ok, failed, or skipped by its precheck, what it called and cost, the sentence saying why, what it
/// said, and its whole output when that says more.
fn report(run: &Value, output: Option<&str>) -> Vec<String> {
    let ok = run["ok"] == true;
    let mut facts = vec![match run["skipped"].as_str() {
        Some(by) => format!("skipped ({by})"),
        None if ok => "ok".to_string(),
        None => "failed".to_string(),
    }];
    if let Some(cost) = run["cost"].as_f64() {
        facts.push(format!("~${cost:.4} (Claude's estimate)"));
    }
    let called: Vec<&str> = run["called"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    if run["skipped"].is_null() {
        facts.push(if called.is_empty() { "called no tool".into() } else { format!("called {}", called.join(", ")) });
    }
    let mut lines = vec![facts.join(" · ")];
    let summary = run["summary"].as_str().unwrap_or_default();
    lines.extend(run["reason"].as_str().filter(|r| *r != summary).map(str::to_string));
    lines.push(summary.to_string());
    if let Some(text) = output.map(str::trim_end).filter(|t| !t.is_empty() && !summary.ends_with(*t)) {
        lines.push(format!("\n{text}"));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_skip_after_a_skip_is_a_new_end_and_reads_as_skipped() {
        let first = json!({"id": "1", "ended": 10, "times": 1, "ok": true, "skipped": "precheck"});
        let again = json!({"id": "1", "ended": 20, "times": 2, "ok": true, "skipped": "precheck", "summary": "nothing waits",
                           "reason": "Skipped: its precheck found nothing to do (exit 1)."});
        assert!(is_new(&first, &again) && !is_new(&again, &again) && is_new(&Value::Null, &first));
        assert!(!is_new(&first, &Value::Null), "no run yet");
        assert_eq!(report(&again, Some("nothing waits\n")), ["skipped (precheck)", "Skipped: its precheck found nothing to do (exit 1).", "nothing waits"]);
        let failed = json!({"ok": false, "summary": "ci_failing u1", "reason": "Ended with exit 3.", "called": ["Read"], "cost": 0.0123});
        assert_eq!(report(&failed, None), ["failed · ~$0.0123 (Claude's estimate) · called Read", "Ended with exit 3.", "ci_failing u1"]);
    }
}
