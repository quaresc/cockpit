//! Terminal answers against the demo world: two answers to one question at once, and only one of them is taken.

use std::sync::{Arc, Barrier};

use cockpit_agents::{gui, session::CHANGED};
use serde_json::{Value, json};

#[test]
fn two_answers_to_one_question_at_once_take_turns_and_the_second_is_refused() {
    let home = tempfile::tempdir().unwrap();
    let world = home.path().join("world.json");
    let screen = "⏺ Old accounts have no quota row yet.\n\nWhich limit should existing accounts start with?\n\n❯ 1. The plan's default\n  2. Their last 30 days' peak\n\nEnter to select · Esc to cancel";
    let agent = json!({"terminal": "app:t41", "space": {"id": "sp-api", "name": "api"}, "tab": {"id": "tb-412", "name": "quotas"}, "title": "Enforce daily quotas",
                       "program": "claude", "status": "blocked", "since_min": 6, "cwd": "/Users/me/code/api", "screen": screen});
    std::fs::write(&world, json!({"agents": [agent]}).to_string()).unwrap();
    // SAFETY: this test binary holds this one test, so nothing reads the environment meanwhile.
    unsafe {
        std::env::set_var("HOME", home.path());
        std::env::set_var("COCKPIT_AGENTS_DEMO", &world);
    }
    // These answers are the person's: the test says so, as only cockpit's own code may.
    cockpit_core::peer::trusted();
    let view = gui::view(&json!({}), &|_| Value::Null);
    let asked = view["agents"][0]["question"]["asked"].as_str().expect("the question carries its fingerprint").to_string();
    let start = Arc::new(Barrier::new(2));
    let answers: Vec<_> = (0..2)
        .map(|i| {
            let (start, asked) = (Arc::clone(&start), asked.clone());
            std::thread::spawn(move || {
                cockpit_core::peer::trusted();
                start.wait();
                gui::act("answer", &[json!({"terminal": "app:t41", "choice": 1, "asked": asked, "source": format!("test:click{i}")})]).unwrap()
            })
        })
        .collect();
    let mut done: Vec<Result<String, String>> = answers.into_iter().map(|t| t.join().unwrap()).collect();
    done.sort_by_key(|d| d.is_err());
    assert!(done[0].as_ref().is_ok_and(|n| n.contains("The plan's default")), "{done:?}");
    assert_eq!(done[1], Err(CHANGED.to_string()), "the second looks again after the first and finds another screen");
    let log = std::fs::read_to_string(home.path().join(".cache/cockpit/agents/answers.log")).unwrap();
    assert_eq!(log.lines().count(), 2, "{log}");
}
