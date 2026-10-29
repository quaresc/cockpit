//! `cockpit routine run <slug>`: a routine run now through the hub, waited for, and how it went, for a Claude session
//! trying out a routine it just wrote.

use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::proc;

const USAGE: &str = "A routine run now, waited for.

    cockpit routine run <slug>    runs it through the hub, then prints how it went, what it called and its output;
                                  exits 1 when the run failed";

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
    let before = doc()["runs"][slug.as_str()][0]["id"].clone();
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
        if !busy && !last.is_null() && last["id"] != before {
            break last.clone();
        }
        if start.elapsed() > Duration::from_secs(timeout * 120 + 60) {
            proc::fail(&format!("cockpit routine: {slug} still runs after {} min", start.elapsed().as_secs() / 60));
        }
    };
    let ok = run["ok"] == true;
    let mut facts = vec![if ok { "ok".to_string() } else { "failed".to_string() }];
    if let Some(cost) = run["cost"].as_f64() {
        facts.push(format!("~${cost:.4} (Claude's estimate)"));
    }
    let called: Vec<&str> = run["called"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    facts.push(if called.is_empty() { "called no tool".into() } else { format!("called {}", called.join(", ")) });
    println!("{}", facts.join(" · "));
    let summary = run["summary"].as_str().unwrap_or_default();
    println!("{summary}");
    let output = cockpit_core::files::cache().join("routines").join(slug).join(format!("{}.txt", run["id"].as_str().unwrap_or_default()));
    if let Some(text) = std::fs::read_to_string(&output).ok().map(|t| t.trim_end().to_string()).filter(|t| !t.is_empty() && !summary.ends_with(t.as_str())) {
        println!("\n{text}");
    }
    if !ok {
        std::process::exit(1);
    }
}
