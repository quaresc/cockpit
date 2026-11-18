//! A model, an effort level or fast mode for an agent in a terminal, on the demo's made-up agents: each row says what its
//! agent offers and how, and a change is typed only as its own command, only while it rests, never one it changes by hand.

use cockpit_agents::gui;
use serde_json::{Value, json};

fn agent(terminal: &str, program: &str, status: &str) -> Value {
    json!({"terminal": terminal, "space": {"id": "sp-api", "name": "api"}, "tab": {"id": terminal, "name": terminal}, "title": format!("{program} {status}"),
           "program": program, "status": status, "since_min": 2, "cwd": "/Users/me/code/api", "screen": "› "})
}

#[test]
fn a_terminal_takes_its_own_command_only_at_rest_and_never_one_changed_by_hand() {
    let home = tempfile::tempdir().unwrap();
    let world = home.path().join("world.json");
    let agents =
        [agent("app:t46", "claude", "idle"), agent("app:t44", "claude", "working"), agent("app:t48", "codex", "idle"), agent("app:t50", "opencode", "idle")];
    std::fs::write(&world, json!({"agents": agents}).to_string()).unwrap();
    // SAFETY: this test binary holds this one test, so nothing reads the environment meanwhile.
    unsafe {
        std::env::set_var("HOME", home.path());
        std::env::set_var("COCKPIT_AGENTS_DEMO", &world);
    }
    cockpit_core::peer::trusted();
    let view = gui::view(&json!({}), &|_| Value::Null);
    let row = |t: &str| view["agents"].as_array().unwrap().iter().find(|r| r["terminal"] == t).cloned().unwrap();
    let claude = row("app:t46")["picks"].clone();
    assert_eq!(claude["models"].as_array().unwrap().iter().filter_map(|m| m["id"].as_str()).collect::<Vec<_>>(), ["opus", "sonnet", "haiku", "fable"]);
    assert_eq!((claude["typed"]["effort"]["command"].as_str(), claude["fast"].as_bool()), (Some("/effort {effort}"), Some(true)));
    assert_eq!(row("app:t48")["picks"]["typed"]["fast"]["byHand"], "/fast");
    let other = row("app:t50")["picks"].clone();
    assert!(
        other["efforts"].as_array().unwrap().is_empty() && other["fast"] == false && other["typed"]["model"].is_null(),
        "offers nothing it was not measured to take"
    );

    let pick =
        |t: &str, setting: &str, value: Value| gui::act("pick", &[json!({"terminal": t, "setting": setting, "value": value, "source": "test:click"})]).unwrap();
    assert_eq!(pick("app:t44", "effort", json!("low")), Err("Claude Code is working: change it once it rests".into()));
    assert_eq!(pick("app:t48", "fast", json!(true)), Err("change it in Codex's terminal: /fast".into()));
    assert_eq!(pick("app:t46", "effort", json!("ultra")), Err("Claude Code takes no effort level \"ultra\"".into()));
    assert_eq!(pick("app:t46", "model", json!("gpt-acme-1")), Err("Claude Code takes no model \"gpt-acme-1\"".into()));
    for typed_more in ["sonnet\n/exit", "sonnet\r", "acme-large; rm -rf ~", "-p", " sonnet", "sonnet\u{1b}[2J"] {
        assert!(pick("app:t46", "model", json!(typed_more)).unwrap_err().contains("takes no model"), "{typed_more:?} reaches /model");
    }
    assert_eq!(pick("app:t50", "model", json!("acme/large")), Err("OpenCode cannot change its model while it runs".into()));
    assert!(pick("app:t46", "speed", json!("fast")).unwrap_err().contains("which setting?"));
    // At rest, a value it takes goes on to delivery: here no window answers, so the words are refused there.
    let typed = pick("app:t46", "effort", json!("high")).unwrap_err();
    assert!(!typed.contains("rests") && !typed.contains("takes no"), "past every check: {typed}");
    let log = std::fs::read_to_string(home.path().join(".cache/cockpit/agents/answers.log")).unwrap();
    assert!(log.contains("app:t46 from test:click"), "{log}");
    // A connection that names no process turns nothing on.
    cockpit_core::peer::set(None);
    assert!(pick("app:t46", "fast", json!(true)).unwrap_err().contains("cannot tell"));
    cockpit_core::peer::trusted();
}
