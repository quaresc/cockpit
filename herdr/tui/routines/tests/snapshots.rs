//! The routines app drawn at 60, 90 and 120 columns on the fixture at a fixed time, both densities, a routine unfolded
//! onto its runs, the form, its sidebar line, and what each row's keys send to the hub.

use std::path::PathBuf;

use chrono::DateTime;
use cockpit_core::files::Density;
use cockpit_core::testing::render;
use cockpit_routines::app::{App, Sent};
use cockpit_routines::data::{Clock, Doc};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use serde_json::{Value, json};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn data() -> Value {
    serde_json::from_str(&std::fs::read_to_string(fixtures().join("routines.json")).unwrap()).unwrap()
}

/// Thursday 29 October 2026, 10:00 at UTC+1.
fn clock() -> Clock {
    let now = DateTime::parse_from_rfc3339("2026-10-29T10:00:00+01:00").unwrap();
    Clock { now: now.timestamp_millis() as u64, zone: Some(*now.offset()) }
}

fn app(density: Density) -> App {
    let mut app = App::with(Some(clock()), density, false, fixtures().join("runs"));
    app.set_doc(&data());
    app
}

fn press(app: &mut App, c: char) {
    key(app, KeyCode::Char(c));
}

fn key(app: &mut App, code: KeyCode) {
    app.event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
}

fn typed(app: &mut App, text: &str) {
    for c in text.chars() {
        press(app, c);
    }
}

fn last(app: &App) -> Sent {
    app.sent.last().cloned().expect("a call was sent")
}

#[test]
fn comfortable_60() {
    insta::assert_snapshot!(render(&mut app(Density::Comfortable), 60, 30));
}

#[test]
fn comfortable_90() {
    insta::assert_snapshot!(render(&mut app(Density::Comfortable), 90, 30));
}

#[test]
fn comfortable_120() {
    insta::assert_snapshot!(render(&mut app(Density::Comfortable), 120, 30));
}

#[test]
fn compact_90() {
    insta::assert_snapshot!(render(&mut app(Density::Compact), 90, 16));
}

#[test]
fn the_last_runs_output_in_the_panel() {
    let mut app = app(Density::Comfortable);
    app.selected = Some("morning-sweep".into());
    insta::assert_snapshot!(render(&mut app, 100, 34));
}

#[test]
fn a_routine_unfolded_onto_its_runs() {
    let mut app = app(Density::Comfortable);
    app.selected = Some("morning-sweep".into());
    press(&mut app, ' ');
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Down);
    insta::assert_snapshot!(render(&mut app, 110, 30));
}

#[test]
fn the_form_reads_when_back_and_adds_the_routine() {
    let mut app = app(Density::Comfortable);
    press(&mut app, 'a');
    typed(&mut app, "Review digest");
    key(&mut app, KeyCode::Tab);
    for _ in 0.."weekdays 09:00".len() {
        key(&mut app, KeyCode::Backspace);
    }
    typed(&mut app, "mon,thu 14:30");
    key(&mut app, KeyCode::Tab);
    typed(&mut app, "List the PRs waiting for my review, oldest first.");
    insta::assert_snapshot!(render(&mut app, 90, 30));
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Enter);
    assert!(app.form.is_none());
    let sent = last(&app);
    let args: Vec<&str> = sent.args.iter().map(|a| a.as_str().unwrap()).collect();
    assert_eq!(sent.action, "add");
    assert_eq!(args, ["Review digest", "mon,thu 14:30", "List the PRs waiting for my review, oldest first.", "", "claude"]);
}

#[test]
fn a_form_with_a_bad_when_says_why_and_adds_nothing() {
    let mut app = app(Density::Comfortable);
    press(&mut app, 'a');
    key(&mut app, KeyCode::Tab);
    typed(&mut app, "ish");
    key(&mut app, KeyCode::Tab);
    typed(&mut app, "x");
    for _ in 0..4 {
        key(&mut app, KeyCode::Enter);
    }
    assert!(app.form.is_some() && app.sent.is_empty());
    key(&mut app, KeyCode::Esc);
    assert!(app.form.is_none());
}

#[test]
fn each_rows_keys_send_their_action() {
    let mut app = app(Density::Comfortable);
    app.selected = Some("morning-sweep".into());
    press(&mut app, 'r');
    assert_eq!(last(&app), Sent { action: "run".into(), args: vec![json!("morning-sweep")], armed: None });
    press(&mut app, 't');
    assert_eq!(last(&app), Sent { action: "talk".into(), args: vec![json!("morning-sweep"), Value::Null], armed: None });
    press(&mut app, 'o');
    assert_eq!(last(&app).action, "output");
    press(&mut app, 'p');
    assert_eq!(last(&app), Sent { action: "pause".into(), args: vec![json!("morning-sweep"), json!(true)], armed: None });
    press(&mut app, 'e');
    assert_eq!(last(&app).action, "edit");
    press(&mut app, 'D');
    assert_eq!(app.sent.len(), 5, "a delete asks first");
    press(&mut app, 'y');
    assert_eq!(last(&app), Sent { action: "delete".into(), args: vec![json!("morning-sweep")], armed: Some("delete-routine-morning-sweep".into()) });
    app.selected = Some("notes-pull".into());
    key(&mut app, KeyCode::Enter);
    assert_eq!(last(&app).action, "stop", "a running routine's main action stops it");
    app.selected = Some("weekly-digest".into());
    press(&mut app, 'p');
    assert_eq!(last(&app).args, [json!("weekly-digest"), json!(false)], "a paused one resumes");
    app.selected = Some("standup".into());
    let before = app.sent.len();
    press(&mut app, 'r');
    assert_eq!(app.sent.len(), before, "a broken routine does not run");
}

#[test]
fn the_list_puts_running_and_failed_first() {
    let app = app(Density::Comfortable);
    let keys: Vec<String> = app.rows().into_iter().map(|r| r.key).collect();
    assert_eq!(keys, ["notes-pull", "fix-ci", "standup", "morning-sweep", "weekly-digest"]);
}

#[test]
fn the_sidebar_says_what_runs_and_what_failed() {
    let doc = Doc::parse(&data()).unwrap();
    assert_eq!(cockpit_routines::pinned::sidebar_text(&doc, &clock()), ("\u{f04b}  1 running · 1 failed".into(), true));
    let calm = Doc::parse(&json!({"version": 1, "routines": [{"slug": "a", "next": clock().now + 240_000}, {"slug": "b", "paused": true, "next": 1}]})).unwrap();
    assert_eq!(cockpit_routines::pinned::sidebar_text(&calm, &clock()).0, "\u{f017}  next in 4 min");
}

#[test]
fn waiting_for_the_hub() {
    let mut app = App::with(Some(clock()), Density::Comfortable, false, fixtures().join("runs"));
    insta::assert_snapshot!(render(&mut app, 90, 8));
}
