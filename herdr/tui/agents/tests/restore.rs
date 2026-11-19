//! A driven session across a restart, against `tests/fixtures/stub-claude`: the registry names it, a process the old hub
//! left running is ended, the session comes back paused with its history, and the next prompt resumes it under its id.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use cockpit_agents::session;
use serde_json::{Value, json};

fn wait_for(key: &str, what: &str, done: impl Fn(&Value) -> bool) -> Value {
    let started = Instant::now();
    loop {
        let now = session::snapshot(key).expect("the session lives");
        if done(&now) {
            return now;
        }
        assert!(started.elapsed() < Duration::from_secs(15), "waited too long for {what}: {now:#}");
        std::thread::sleep(Duration::from_millis(30));
    }
}

fn said(s: &Value) -> Vec<String> {
    s["messages"].as_array().into_iter().flatten().filter_map(|m| m["text"].as_str().filter(|t| !t.is_empty()).map(str::to_string)).collect()
}

/// The processes running the stub for session `id`.
fn agents_of(id: &str) -> usize {
    let out = Command::new("ps").args(["-axo", "command="]).output().unwrap();
    String::from_utf8_lossy(&out.stdout).lines().filter(|l| l.contains("stub-claude") && l.contains(id)).count()
}

#[test]
fn a_session_comes_back_paused_after_a_restart_and_the_next_prompt_resumes_it() {
    if !Command::new("/usr/bin/python3").arg("--version").output().is_ok_and(|o| o.status.success()) {
        eprintln!("no /usr/bin/python3: the stub agent cannot run here");
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let work = home.path().join("api");
    std::fs::create_dir_all(&work).unwrap();
    let stub = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/stub-claude");
    // SAFETY: this test binary holds this one test, so nothing reads the environment meanwhile.
    unsafe {
        std::env::set_var("HOME", home.path());
        std::env::set_var("COCKPIT_CLAUDE", &stub);
    }
    // Only a hub keeps the registry: this one is told of every move, as the hub is, and acts for the person.
    session::on_change(|| {}, || {});
    cockpit_core::peer::trusted();

    let key = session::start("claude", &work.to_string_lossy(), "default", &session::Picks::default()).expect("the stub starts");
    session::send(&key, "What resets the quota?").unwrap();
    let first = wait_for(&key, "the first turn", |s| s["working"] == false && said(s).len() == 2);
    let id = first["session"].as_str().unwrap().to_string();
    let registry = cockpit_core::files::load(&session::registry()).expect("the registry is written");
    let entry = registry["sessions"].as_array().unwrap().iter().find(|e| e["key"] == key.as_str()).cloned().expect("it names the session");
    assert_eq!(entry["session_id"], id.as_str(), "the id it was started under, which its agent took");
    let real = work.canonicalize().unwrap();
    assert!(entry["pid"].is_u64() && entry["cwd"] == real.to_string_lossy().as_ref() && entry["mode"] == "default", "{entry}");

    // The hub goes away: its registry stays behind, naming a process still running that session.
    session::stop(&key);
    let stream = cockpit_core::agent::get("claude").and_then(|a| a.stream).unwrap();
    let mut orphan = Command::new("/usr/bin/python3")
        .arg(&stub)
        .args(&stream)
        .args(["--resume", &id])
        .current_dir(&work)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let mut left = entry.clone();
    left["pid"] = json!(orphan.id());
    // Another process that took a pid the registry names is never ended.
    let mut stranger = Command::new("/bin/sleep").arg("30").spawn().unwrap();
    let mut other = left.clone();
    (other["key"], other["session_id"], other["pid"]) = (json!("claude:9-0"), json!("00000000-0000-4000-8000-000000000000"), json!(stranger.id()));
    cockpit_core::files::write(&session::registry(), &json!({"sessions": [left, other]})).unwrap();

    session::restore();
    assert!(orphan.try_wait().unwrap().is_some(), "the process left behind is ended");
    assert!(stranger.try_wait().unwrap().is_none(), "a process that is not the session's agent is left alone");
    let _ = stranger.kill();
    assert!(session::snapshot("claude:9-0").is_none(), "a session with no transcript does not come back");
    let back = session::snapshot(&key).expect("it comes back");
    assert_eq!((back["paused"].clone(), back["working"].clone(), back["session"].clone()), (json!(true), json!(false), json!(id)));
    assert_eq!(said(&back), said(&first), "its history, read from its transcript");
    assert!(session::summary(&key).unwrap()["messages"].is_null(), "a summary carries no messages");

    // Paused, it takes words the person sends, never words that would go on their own.
    assert!(cockpit_agents::gui::ready(&key, 0, true).is_ok());
    assert!(cockpit_agents::gui::ready(&key, 0, false).unwrap_err().contains("not running"));

    // The next prompt starts its agent again on the same session: one process, however many send at once.
    let other = std::thread::spawn({
        let key = key.clone();
        move || {
            cockpit_core::peer::trusted();
            session::send(&key, "And the week after?")
        }
    });
    session::send(&key, "And after midnight?").unwrap();
    other.join().unwrap().unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(agents_of(&id), 1, "two sends at once start one agent");
    let resumed = wait_for(&key, "the resumed turns", |s| s["working"] == false && said(s).len() == 6);
    assert_eq!((resumed["paused"].clone(), resumed["session"].clone()), (json!(false), json!(id)));
    assert!(said(&resumed).iter().any(|t| t == "And after midnight?") && said(&resumed).iter().any(|t| t == "And the week after?"));
    assert_eq!(agents_of(&id), 1, "one agent runs the session");
    let entry = cockpit_core::files::load(&session::registry()).unwrap()["sessions"][0].clone();
    assert!(entry["pid"].is_u64() && entry["pid"] != json!(orphan.id()), "the registry names the new process");

    // End: refused while it works unless confirmed; once ended, nothing is left of it.
    session::send(&key, "ask to run the tests").unwrap();
    wait_for(&key, "the ask", |s| s["pending"].as_array().is_some_and(|p| !p.is_empty()));
    let refused = session::act("stop", &[json!({"key": key})]).unwrap();
    assert!(refused.unwrap_err().contains("confirm"), "a working session is ended only once confirmed");
    session::act("stop", &[json!({"key": key, "confirm": true})]).unwrap().unwrap();
    assert!(session::snapshot(&key).is_none());
    let started = Instant::now();
    while agents_of(&id) > 0 && started.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(agents_of(&id), 0, "no process is left");
    let registry = cockpit_core::files::load(&session::registry()).unwrap();
    assert!(!registry.to_string().contains(&id), "and the registry forgets it");
}
