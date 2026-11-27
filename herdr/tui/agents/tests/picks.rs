//! A model, an effort level and fast mode for a driven session, against `tests/fixtures/stub-claude`: what a start passes
//! on, what its catalog offers, and what changes live over its stream, refused where its agent takes no such value.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use cockpit_agents::session::{self, Picks};
use serde_json::{Value, json};

fn wait_for(key: &str, what: &str, done: impl Fn(&Value) -> bool) -> Value {
    let started = Instant::now();
    loop {
        let now = session::summary(key).expect("the session lives");
        if done(&now) {
            return now;
        }
        assert!(started.elapsed() < Duration::from_secs(15), "waited too long for {what}: {now:#}");
        std::thread::sleep(Duration::from_millis(30));
    }
}

/// Each start's arguments, as the stub noted them.
fn starts(file: &Path) -> Vec<Vec<String>> {
    std::fs::read_to_string(file).unwrap_or_default().lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

fn has(args: &[String], pair: [&str; 2]) -> bool {
    args.windows(2).any(|w| w[0] == pair[0] && w[1] == pair[1])
}

#[test]
fn a_session_starts_with_what_was_picked_and_changes_it_live() {
    if !std::process::Command::new("/usr/bin/python3").arg("--version").output().is_ok_and(|o| o.status.success()) {
        eprintln!("no /usr/bin/python3: the stub agent cannot run here");
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let work = home.path().join("api");
    std::fs::create_dir_all(&work).unwrap();
    let args = home.path().join("args.jsonl");
    let stub = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/stub-claude");
    // SAFETY: this test binary holds this one test, so nothing reads the environment meanwhile.
    unsafe {
        std::env::set_var("HOME", home.path());
        std::env::set_var("COCKPIT_CLAUDE", &stub);
        std::env::set_var("STUB_ARGS", &args);
    }
    cockpit_core::peer::trusted();
    let refused = session::start("claude", &work.to_string_lossy(), "default", &Picks { effort: "ultra".into(), ..Picks::default() });
    assert_eq!(refused.unwrap_err(), "Claude Code takes no effort level ultra");
    let picks = Picks { model: "opus".into(), effort: "high".into(), fast: Some(true), ..Picks::default() };
    let key = session::start("claude", &work.to_string_lossy(), "default", &picks).expect("the stub starts");

    let catalog = wait_for(&key, "its catalog", |s| s["models"].as_array().is_some_and(|m| !m.is_empty()));
    let first = &starts(&args)[0];
    assert!(has(first, ["--model", "opus"]) && has(first, ["--effort", "high"]) && has(first, ["--settings", "{\"fastMode\":true}"]), "{first:?}");
    let ids: Vec<&str> = catalog["models"].as_array().unwrap().iter().filter_map(|m| m["id"].as_str()).collect();
    assert_eq!(ids, ["default", "opus", "haiku"]);
    assert_eq!((catalog["models"][1]["resolved"].as_str(), catalog["models"][2]["fast"].as_bool()), (Some("claude-opus-5-5"), Some(false)));

    session::send(&key, "what resets the quota?").unwrap();
    let rested = wait_for(&key, "the turn to end", |s| s["working"] == false && s["fast_on"].is_boolean());
    assert_eq!((rested["effort"].as_str(), rested["fast_on"].as_bool()), (Some("high"), Some(true)), "as its init and result report them");

    // Live over its stream: an effort level, fast mode off, another model.
    assert_eq!(session::act("effort", &[json!({"key": key, "effort": "low"})]).unwrap(), Ok("set".into()));
    wait_for(&key, "the effort level", |s| s["effort"] == "low");
    assert_eq!(session::act("fast", &[json!({"key": key, "fast": false})]).unwrap(), Ok("set".into()));
    let off = wait_for(&key, "fast mode off", |s| s["fast"] == false);
    assert!(off["fast_on"].is_null(), "what it reported before waits for its next report: {off}");
    assert!(session::act("effort", &[json!({"key": key, "effort": "ultra"})]).unwrap().unwrap_err().contains("takes no effort level ultra"));
    assert!(session::act("model", &[json!({"key": key, "model": "gpt-acme-1"})]).unwrap().unwrap_err().contains("takes no model"));
    // Fast mode spends the plan's usage faster: the agent's own tools cannot turn it on, though they may turn it off.
    let stub_pid = session::keyed_pids()[0].1;
    cockpit_core::peer::set(Some(stub_pid));
    assert!(session::act("fast", &[json!({"key": key, "fast": true})]).unwrap().unwrap_err().contains("by a person"));
    assert!(session::act("fast", &[json!({"key": key, "fast": false})]).unwrap().is_ok(), "its own session");
    cockpit_core::peer::trusted();
    assert_eq!(session::act("model", &[json!({"key": key, "model": "haiku"})]).unwrap(), Ok("set".into()));
    wait_for(&key, "the model", |s| s["model"] == "claude-haiku-5-5");
    // Asked for on a model without it, fast mode does not serve, and the agent says why.
    session::act("fast", &[json!({"key": key, "fast": true})]).unwrap().unwrap();
    wait_for(&key, "fast mode set", |s| s["fast"] == true);
    session::send(&key, "and the banner?").unwrap();
    let after = wait_for(&key, "the next turn's report", |s| s["working"] == false && s["fast_on"] == false);
    assert_eq!(after["fast_why"], "model_not_allowed");
    assert_eq!(starts(&args).len(), 1, "every change went over the stream: one process");
    let log = std::fs::read_to_string(session::dir().join(format!("{key}.log"))).unwrap();
    assert!(log.contains("asked it for effort low") && log.contains("asked it for fast off"), "{log}");
    session::stop(&key);
}
