//! A driven session on another Claude Code account than the hub's, against `tests/fixtures/stub-claude`: it runs with
//! that account's config folder, keeps it, and an account this machine has not is refused.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use cockpit_agents::session::{self, Picks};
use serde_json::Value;

#[test]
fn a_driven_session_runs_on_the_account_picked() {
    if !std::process::Command::new("/usr/bin/python3").arg("--version").output().is_ok_and(|o| o.status.success()) {
        eprintln!("no /usr/bin/python3: the stub agent cannot run here");
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let work = home.path().join("api");
    let account = home.path().join(".claude-work");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(&account).unwrap();
    std::fs::write(account.join(".claude.json"), r#"{"oauthAccount": {"emailAddress": "me@acme.example", "organizationName": "Acme"}}"#).unwrap();
    let env = home.path().join("env.json");
    let stub = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/stub-claude");
    // SAFETY: this test binary holds this one test, so nothing reads the environment meanwhile.
    unsafe {
        std::env::set_var("HOME", home.path());
        std::env::remove_var("CLAUDE_CONFIG_DIR");
        std::env::set_var("COCKPIT_CLAUDE", &stub);
        std::env::set_var("STUB_ENV", &env);
    }
    cockpit_core::peer::trusted();
    let cwd = work.to_string_lossy();
    let gone = Picks { account: home.path().join(".claude-gone").to_string_lossy().into_owned(), ..Picks::default() };
    assert!(session::start("claude", &cwd, "default", &gone).unwrap_err().starts_with("no Claude Code account in "));

    let dir = account.to_string_lossy().into_owned();
    let key = session::start("claude", &cwd, "default", &Picks { account: format!("{dir}/"), ..Picks::default() }).expect("the stub starts");
    let started = Instant::now();
    let seen: Value = loop {
        if let Some(v) = std::fs::read_to_string(&env).ok().and_then(|t| serde_json::from_str(&t).ok()) {
            break v;
        }
        assert!(started.elapsed() < Duration::from_secs(15), "the stub never started");
        std::thread::sleep(Duration::from_millis(30));
    };
    assert_eq!(seen["CLAUDE_CONFIG_DIR"].as_str(), Some(dir.as_str()), "the account's folder, in the variable its row names");
    assert_eq!(session::summary(&key).unwrap()["account"].as_str(), Some(dir.as_str()), "the session keeps it");
    session::act("stop", &[serde_json::json!({"key": key, "confirm": true})]);
}
