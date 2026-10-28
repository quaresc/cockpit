//! The fixtures under tests/fixtures as the app's inputs, the clock frozen at the time they were taken, and keys typed.
#![allow(dead_code)]

use std::collections::HashMap;

use cockpit_core::term::App as _;
use cockpit_inbox::app::App;
use cockpit_inbox::facts;
use cockpit_inbox::inputs::Inputs;
use cockpit_inbox::sessions::Local;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use serde_json::{Value, json};

pub fn fixture(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name);
    serde_json::from_str(&std::fs::read_to_string(path).expect("a fixture")).expect("fixture JSON")
}

pub fn inputs() -> Inputs {
    let costs = fixture("costs.json").as_object().expect("costs by ticket").iter().map(|(k, v)| (k.clone(), v.as_f64().unwrap_or(0.0))).collect();
    let muted = fixture("muted.json").as_array().expect("muted repos").iter().filter_map(|r| r.as_str().map(str::to_string)).collect();
    Inputs { data: Some(fixture("inbox.json")), ci: fixture("ci.json"), brief: Some(fixture("brief.json")), muted, costs }
}

pub fn series() -> HashMap<String, Vec<f64>> {
    fixture("history.json").as_object().expect("series by ticket").iter().map(|(k, v)| (k.clone(), v.as_array().expect("values").iter().filter_map(Value::as_f64).collect())).collect()
}

pub fn app(compact: bool) -> App {
    facts::freeze("2026-10-28T10:00:00Z");
    App::fixture(inputs(), json!({}), compact, Local::fixed(HashMap::new(), Vec::new()), series())
}

/// Types each character as a key press; `\n` is enter, `\x1b` esc.
pub fn typed(app: &mut App, keys: &str) {
    for c in keys.chars() {
        let code = match c {
            '\n' => KeyCode::Enter,
            '\x1b' => KeyCode::Esc,
            c => KeyCode::Char(c),
        };
        app.event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
    }
}

pub fn focus(app: &mut App, key: &str) {
    app.focus(&json!({"app": "inbox", "key": key}));
}
