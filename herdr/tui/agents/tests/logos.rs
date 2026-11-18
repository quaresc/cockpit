//! Agent rows carry their display identity on watched terminals and driven sessions.

use cockpit_agents::gui;
use serde_json::{Value, json};

#[test]
fn watched_and_driven_rows_name_their_agent_and_herdr_icon() {
    let home = tempfile::tempdir().unwrap();
    let world = home.path().join("world.json");
    let agents: Vec<Value> = ["claude", "agy", "pig"]
        .iter()
        .enumerate()
        .map(|(i, program)| {
            json!({
                "terminal": format!("app:t{}", i + 1), "space": {"id": "acme", "name": "api"},
                "tab": {"id": format!("tab-{i}"), "name": "Quota checks"}, "title": program,
                "program": program, "status": "idle", "cwd": "/acme/api", "screen": "Ready"
            })
        })
        .collect();
    std::fs::write(&world, json!({"agents": agents}).to_string()).unwrap();
    // SAFETY: this test binary has one test and owns its environment.
    unsafe {
        std::env::set_var("HOME", home.path());
        std::env::set_var("COCKPIT_AGENTS_DEMO", &world);
    }
    let rows = gui::view(&json!({}), &|_| Value::Null);
    // Rows are found by terminal: their title follows the naming chain, here the tab's own label.
    for (terminal, icon, name, reports) in
        [("app:t1", "claude", "Claude Code", true), ("app:t2", "antigravity", "Antigravity", false), ("app:t3", "pig", "PiG", false)]
    {
        let row = rows["agents"].as_array().unwrap().iter().find(|r| r["terminal"] == terminal).unwrap();
        assert_eq!((&row["icon"], &row["agent_name"], &row["reports_limits"]), (&json!(icon), &json!(name), &json!(reports)));
    }
    let summary = json!({"key": "claude:acme", "agent": "claude", "cwd": "/acme/api", "session": "", "pending": [], "started": 1});
    let row = gui::driven_row(&summary, None, &Value::Null, &Value::Null, None, &Value::Null);
    assert_eq!((row["icon"].as_str(), row["agent_name"].as_str(), row["reports_limits"].as_bool()), (Some("claude"), Some("Claude Code"), Some(true)));
}
