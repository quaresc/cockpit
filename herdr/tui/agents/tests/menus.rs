//! Words never go into a menu cockpit could not read (one scrolled past its first choice, one whose question reads as
//! words), nor into an agent that asks nothing cockpit can read.

use cockpit_agents::gui;
use serde_json::{Value, json};

#[test]
fn words_for_a_menu_cockpit_cannot_read_are_refused() {
    let home = tempfile::tempdir().unwrap();
    let world = home.path().join("world.json");
    let screen = "⏺ Running the export again.\n\n❯ 2. Yes, and don't ask again for this command\n  3. No, and tell the agent what to do differently\n";
    let agent = json!({"terminal": "app:t48", "space": {"id": "sp-api", "name": "api"}, "tab": {"id": "tb-417", "name": "4"}, "title": "Retry failed CSV exports",
                       "program": "codex", "status": "blocked", "since_min": 2, "cwd": "/Users/me/code/api", "screen": screen});
    // Its question reads as words, and its menu's lines are none cockpit can take for choices.
    let worded = "⏺ Which export should I retry first?\n\n  the nightly one   the weekly one\n\nEnter to select · Esc to cancel\n";
    let words = json!({"terminal": "app:t49", "space": {"id": "sp-api", "name": "api"}, "tab": {"id": "tb-418", "name": "5"}, "title": "Pick the export",
                       "program": "claude", "status": "blocked", "since_min": 3, "cwd": "/Users/me/code/api", "screen": worded});
    // Stopped at its limit: nothing to answer and nothing to type into.
    let limit = "⏺ Bash(mix test)\n  ⎿  You've hit your limit · resets 8am\n\n  7d 100%\n  ⏵⏵ auto mode on (shift+tab to cycle)\n";
    let at_limit = json!({"terminal": "app:t52", "space": {"id": "sp-api", "name": "api"}, "tab": {"id": "tb-433", "name": "6"}, "title": "Quoted keywords",
                          "program": "claude", "status": "blocked", "since_min": 5, "cwd": "/Users/me/code/api", "screen": limit});
    std::fs::write(&world, json!({"agents": [agent, words, at_limit]}).to_string()).unwrap();
    // SAFETY: this test binary holds this one test, so nothing reads the environment meanwhile.
    unsafe {
        std::env::set_var("HOME", home.path());
        std::env::set_var("COCKPIT_AGENTS_DEMO", &world);
    }
    // These answers are the person's: the test says so, as only cockpit's own code may.
    cockpit_core::peer::trusted();
    let view = gui::view(&json!({}), &|_| Value::Null);
    let row = |t: &str| view["agents"].as_array().unwrap().iter().find(|r| r["terminal"] == t).cloned().unwrap();
    let row = &row("app:t48");
    assert_eq!(row["question"], Value::Null, "its menu does not read as one, and its last choices are no question");
    assert_eq!((row["stage"].as_str(), row["waits"]["text"].as_str()), (Some("needs_you"), Some("Waiting at its terminal")));
    let answer = gui::act("answer", &[json!({"terminal": "app:t48", "text": "go ahead", "source": "test:click"})]).unwrap();
    assert_eq!(answer, Err("Retry failed CSV exports shows a menu cockpit cannot read: answer it in its terminal".into()));
    let mut sent = false;
    let typed = gui::waiting("app:t48", &mut || {
        sent = true;
        true
    });
    assert_eq!(typed, Err("Retry failed CSV exports is waiting on a question: answer it first".into()), "named as the views title it");
    assert!(!sent, "nothing was typed");

    // A question that reads as words over a menu it cannot read: its answer, naming that question, types nothing.
    let words = view["agents"].as_array().unwrap().iter().find(|r| r["terminal"] == "app:t49").cloned().unwrap();
    assert!(words["question"]["text"].as_str().unwrap().starts_with("⏺ Which export should I retry first?"), "{}", words["question"]);
    let asked = words["question"]["asked"].as_str().expect("its fingerprint").to_string();
    let answer = gui::act("answer", &[json!({"terminal": "app:t49", "text": "the nightly one", "asked": asked, "source": "test:click"})]).unwrap();
    assert_eq!(answer, Err("Pick the export shows a menu cockpit cannot read: answer it in its terminal".into()));
    // At its limit: an answer is refused for asking nothing, and words are refused for waiting at its terminal.
    let answer = gui::act("answer", &[json!({"terminal": "app:t52", "text": "go on", "source": "test:click"})]).unwrap();
    assert_eq!(answer, Err("Quoted keywords asks nothing cockpit can read: go to its terminal".into()));
    let mut sent = false;
    let typed = gui::waiting("app:t52", &mut || {
        sent = true;
        true
    });
    assert_eq!(typed, Err("Quoted keywords waits at its terminal: nothing goes to it from here".into()));
    assert!(!sent, "nothing was typed");
}
