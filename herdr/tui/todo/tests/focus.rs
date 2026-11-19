//! What `cockpit focus todo:<text>` writes reaches the running to-do app: that item selected, pointed at.

use std::path::PathBuf;

use cockpit_core::files::Density;
use cockpit_todo::app::{App, Sel};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use serde_json::json;

#[test]
fn a_focus_request_for_an_item_selects_it_pointed_at() {
    let home = tempfile::tempdir().unwrap();
    // SAFETY: this test binary holds this one test, so nothing reads the environment meanwhile.
    unsafe { std::env::set_var("HOME", home.path()) };
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let path = home.path().join("todo.md");
    std::fs::copy(fixtures.join("todo.md"), &path).unwrap();
    let config = serde_json::from_str(&std::fs::read_to_string(fixtures.join("config.json")).unwrap()).unwrap();
    let mut app = App::with(path, config, serde_json::Value::Null, Density::Comfortable, false);
    assert!(!app.pointed.is_on());
    std::thread::sleep(std::time::Duration::from_millis(20));
    cockpit_core::focus::request("todo", &json!({"app": "todo", "key": "ADR Noor"}));
    assert!(app.tick(), "the request is taken on the next tick");
    assert_eq!(app.sel, Sel::Item("ADR Noor".into()));
    assert!(app.pointed.is_on(), "pointed at from outside: its keys wait for a move");
    app.event(Event::Key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)));
    assert!(!app.pointed.is_on(), "a move makes it the person's");
    app.tick();
    assert!(!app.pointed.is_on(), "a request is taken once");
}
