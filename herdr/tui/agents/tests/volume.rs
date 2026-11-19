//! The agents view at the plan's volume, 50 agents of the demo world in the Herdr app: the view is built again on every
//! change, so no build runs a process for each row.

use std::path::PathBuf;
use std::time::Instant;

use cockpit_agents::gui;
use serde_json::{Value, json};

#[test]
fn a_build_of_fifty_agents_runs_no_process_for_each_row() {
    let home = tempfile::tempdir().unwrap();
    let demo: Value = serde_json::from_str(&std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("demo.json")).unwrap()).unwrap();
    let base = demo["agents"].as_array().unwrap();
    let statuses = ["working", "blocked", "done", "idle", "working", "failed", "interrupted"];
    let agents: Vec<Value> = (0..50)
        .map(|i| {
            let mut a = base[i % base.len()].clone();
            a["terminal"] = json!(format!("app:t{}", 100 + i));
            a["status"] = json!(statuses[i % statuses.len()]);
            a["session"] = json!(format!("vol-{i}"));
            a
        })
        .collect();
    let world = home.path().join("world.json");
    std::fs::write(&world, json!({"agents": agents}).to_string()).unwrap();
    // hc notes each time it is asked, and knows none of the demo's made-up terminals.
    let asked = home.path().join("hc.log");
    let hc = home.path().join("hc");
    std::fs::write(&hc, format!("#!/bin/sh\necho \"$*\" >> '{}'\nexit 1\n", asked.display())).unwrap();
    std::fs::set_permissions(&hc, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    // SAFETY: this test binary holds this one test, so nothing reads the environment meanwhile.
    unsafe {
        std::env::set_var("HOME", home.path());
        std::env::set_var("COCKPIT_AGENTS_DEMO", &world);
        std::env::set_var("HC_BIN", &hc);
    }
    let calls = || std::fs::read_to_string(&asked).map(|t| t.lines().filter(|l| l.starts_with("get ")).count()).unwrap_or(0);
    let mut took = Vec::new();
    for _ in 0..3 {
        let started = Instant::now();
        let view = gui::view(&json!({}), &|_| Value::Null);
        took.push(started.elapsed());
        assert_eq!(view["agents"].as_array().map(Vec::len), Some(50));
    }
    eprintln!("agents view, 50 agents: {:?} the first build, then {:?} and {:?}", took[0], took[1], took[2]);
    assert_eq!(calls(), 0, "hc is never asked about a made-up terminal");
}
