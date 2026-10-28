//! Past Claude sessions through the socket, from the fixture box's transcripts: indexed in the background once the hub
//! starts, found after the apps' own entries, and a fork of one opened in a tab of the actions space.

mod common;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use common::{Hub, Sandbox};
use serde_json::{Value, json};

fn sandbox() -> Sandbox {
    Sandbox::new(&json!({}), &json!({}))
}

fn herdr_calls(sandbox: &Sandbox) -> Vec<Vec<String>> {
    sandbox
        .calls()
        .iter()
        .filter(|c| c["cmd"] == "herdr")
        .map(|c| c["args"].as_array().unwrap().iter().map(|a| a.as_str().unwrap().to_string()).collect())
        .collect()
}

fn real(path: &str) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path))
}

/// The search's entries once sessions are among them: the index builds in the background after the hub starts.
fn search_with_sessions(hub: &Hub, query: &str) -> Vec<Value> {
    let start = Instant::now();
    loop {
        let found = hub.request(&json!({"op": "search", "query": query}));
        let entries = found["entries"].as_array().unwrap().clone();
        if entries.iter().any(|e| e["app"] == "session") {
            return entries;
        }
        assert!(start.elapsed() < Duration::from_secs(10), "no session found for {query:?}: {entries:?}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn sessions_come_after_the_apps_entries() {
    let sandbox = sandbox();
    let hub = Hub::start(&sandbox, &[]);
    let entries = search_with_sessions(&hub, "dark mode");
    assert_eq!(entries[0]["app"], "inbox");
    let last = entries.last().unwrap();
    assert_eq!((last["kind"].as_str(), last["title"].as_str()), (Some("session"), Some("Add dark mode tokens to the sign-in page")));
    assert_eq!(last["target"]["key"], "s-dark");
    let sessions: Vec<String> =
        search_with_sessions(&hub, "sign-in").iter().filter(|e| e["app"] == "session").map(|e| e["target"]["key"].as_str().unwrap().to_string()).collect();
    assert_eq!(sessions, ["s-login", "s-dark"]);
    assert!(sandbox.home().join(".cache/cockpit/sessions-index/v2/manifest.json").exists());
}

#[test]
fn a_new_transcript_is_found_after_the_watch_sees_it() {
    let sandbox = sandbox();
    let hub = Hub::start(&sandbox, &[]);
    search_with_sessions(&hub, "dark mode");
    let line = json!({"type": "user", "cwd": sandbox.root.join("repos/api"), "timestamp": "2026-10-28T08:00:00Z", "message": {"role": "user", "content": "Rotate the narwhal keys"}});
    std::fs::write(sandbox.home().join(".claude/projects/-box-repos-api/s-keys.jsonl"), format!("{line}\n")).unwrap();
    let start = Instant::now();
    // The watch waits for quiet and spaces its updates half a minute apart; the test only checks it comes.
    while !hub.request(&json!({"op": "search", "query": "narwhal"}))["entries"].as_array().unwrap().iter().any(|e| e["target"]["key"] == "s-keys") {
        assert!(start.elapsed() < Duration::from_secs(60), "the new transcript was never indexed");
        std::thread::sleep(Duration::from_millis(500));
    }
}

#[test]
fn going_to_a_session_forks_it_in_a_tab_of_the_actions_space() {
    let sandbox = sandbox();
    let hub = Hub::start(&sandbox, &[]);
    let target = search_with_sessions(&hub, "callback").iter().find(|e| e["app"] == "session").unwrap()["target"].clone();
    let reply = hub.request(&json!({"op": "go", "target": target}));
    assert_eq!(reply["note"], "opened a fork of \"Google sign-in for ENG-401\"");
    let calls = herdr_calls(&sandbox);
    let tab = calls.iter().find(|c| c.starts_with(&["tab".into(), "create".into()])).unwrap_or_else(|| panic!("no tab create in {calls:?}"));
    let after = |flag: &str| tab[tab.iter().position(|a| a == flag).unwrap() + 1].clone();
    assert_eq!((after("--workspace").as_str(), after("--label").as_str()), ("w3", "↩ Google sign-in for ENG-401"));
    assert_eq!(real(&after("--cwd")), real(&sandbox.root.join("repos/api").to_string_lossy()));
    let run = calls.iter().find(|c| c.starts_with(&["pane".into(), "run".into()])).unwrap();
    assert!(run[3].starts_with("claude --resume s-login --fork-session 'This is a fork of a past session"), "{run:?}");
    assert!(!sandbox.home().join(".cache/cockpit/focus/session.json").exists());
    let kept: Value = serde_json::from_str(&std::fs::read_to_string(sandbox.home().join(".cache/cockpit/actions.json")).unwrap()).unwrap();
    assert_eq!(kept["session s-login"], "wT:p1");
}

#[test]
fn a_session_already_open_comes_back_into_view() {
    let sandbox = Sandbox::new(&json!({"pane get": {"pane": {"pane_id": "w3:p7", "tab_id": "w3:t7", "agent": "claude"}}}), &json!({}));
    std::fs::write(sandbox.home().join(".cache/cockpit/actions.json"), r#"{"session s-login": "w3:p7"}"#).unwrap();
    let hub = Hub::start(&sandbox, &[]);
    let target = json!({"app": "session", "key": "s-login", "cwd": sandbox.root.join("repos/api"), "title": "Google sign-in"});
    let reply = hub.request(&json!({"op": "go", "target": target}));
    assert_eq!(reply["note"], "the session is already open: focused its tab");
    let calls = herdr_calls(&sandbox);
    assert!(calls.contains(&vec!["tab".into(), "focus".into(), "w3:t7".into()]), "{calls:?}");
    assert!(!calls.iter().any(|c| c.starts_with(&["tab".into(), "create".into()])), "{calls:?}");
}

#[test]
fn a_session_whose_folder_is_gone_is_not_opened() {
    let sandbox = sandbox();
    let hub = Hub::start(&sandbox, &[]);
    let reply = hub.request(&json!({"op": "go", "target": {"app": "session", "key": "s-old", "cwd": "/nowhere/at/all"}}));
    assert_eq!(reply["error"], "the folder of session s-old is gone");
    let reply = hub.request(&json!({"op": "go", "target": {"app": "session", "key": "s;rm -rf ~", "cwd": "/tmp"}}));
    assert!(reply["error"].as_str().unwrap().starts_with("not a session id"));
    assert!(herdr_calls(&sandbox).is_empty());
}
