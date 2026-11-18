//! A setting its agent takes only at a start, against `tests/fixtures/stub-claude` with the stream's live settings turned
//! off in the config: kept for the next message, which resumes the same conversation with the new switches.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use cockpit_agents::session::{self, NEXT, Picks};
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

fn starts(file: &Path) -> Vec<Vec<String>> {
    std::fs::read_to_string(file).unwrap_or_default().lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

#[test]
fn a_setting_taken_only_at_a_start_waits_for_the_next_message() {
    if !std::process::Command::new("/usr/bin/python3").arg("--version").output().is_ok_and(|o| o.status.success()) {
        eprintln!("no /usr/bin/python3: the stub agent cannot run here");
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let work = home.path().join("api");
    std::fs::create_dir_all(&work).unwrap();
    let config = home.path().join(".config/cockpit");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(config.join("config.json"), json!({"agents": {"commands": {"claude": {"live": []}}}}).to_string()).unwrap();
    let args = home.path().join("args.jsonl");
    let stub = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/stub-claude");
    // SAFETY: this test binary holds this one test, so nothing reads the environment meanwhile.
    unsafe {
        std::env::set_var("HOME", home.path());
        std::env::set_var("COCKPIT_CLAUDE", &stub);
        std::env::set_var("STUB_ARGS", &args);
    }
    cockpit_core::peer::trusted();
    let key = session::start("claude", &work.to_string_lossy(), "default", &Picks::default()).expect("the stub starts");
    session::send(&key, "what resets the quota?").unwrap();
    let rested = wait_for(&key, "the turn to end", |s| s["working"] == false && s["count"].as_u64() >= Some(2));
    let id = rested["session"].as_str().unwrap().to_string();

    assert_eq!(session::act("effort", &[json!({"key": key, "effort": "low"})]).unwrap(), Ok(NEXT.into()));
    let waiting = session::summary(&key).unwrap();
    assert_eq!((waiting["restart"].as_bool(), waiting["effort"].as_str()), (Some(true), Some("low")), "kept, and said to wait");
    assert_eq!(starts(&args).len(), 1, "nothing restarted yet");

    session::send(&key, "and the banner?").unwrap();
    let again = wait_for(&key, "the resumed turn", |s| s["working"] == false && s["count"].as_u64() >= Some(4));
    let all = starts(&args);
    assert_eq!(all.len(), 2, "one more start, for the message");
    assert!(all[1].windows(2).any(|w| w[0] == "--effort" && w[1] == "low"), "{:?}", all[1]);
    assert!(all[1].windows(2).any(|w| w[0] == "--resume" && w[1] == id), "the same conversation: {:?}", all[1]);
    assert_eq!((again["restart"].as_bool(), again["ended"].is_null(), again["failed"].is_null()), (Some(false), true, true), "no failure told");
    let messages = session::snapshot(&key).unwrap()["messages"].clone();
    assert!(messages.to_string().contains("and the banner?"), "the message went to the resumed process");
    session::stop(&key);
}
