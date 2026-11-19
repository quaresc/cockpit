//! The routines app drawn at 60, 90 and 120 columns on the fixture at a fixed time, both densities, a routine unfolded
//! onto its runs, a skipped and a failed run, the form, its sidebar line, and what each row's keys send to the hub.

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
    key(&mut app, KeyCode::Enter);
    typed(&mut app, "Review digest");
    key(&mut app, KeyCode::Tab);
    for _ in 0.."weekdays 09:00".len() {
        key(&mut app, KeyCode::Backspace);
    }
    typed(&mut app, "mon,thu 14:30");
    key(&mut app, KeyCode::Tab);
    typed(&mut app, "America/New_York");
    key(&mut app, KeyCode::Tab);
    typed(&mut app, "List the PRs waiting for my review, oldest first.");
    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::Tab);
    typed(&mut app, "gh pr list --search review-requested:@me | grep -q .");
    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::Right);
    insta::assert_snapshot!(render(&mut app, 90, 32));
    key(&mut app, KeyCode::Enter);
    assert!(app.form.is_none());
    let sent = last(&app);
    let args: Vec<&str> = sent.args.iter().map(|a| a.as_str().unwrap()).collect();
    assert_eq!(sent.action, "add");
    assert_eq!(
        args,
        [
            "Review digest",
            "mon,thu 14:30",
            "List the PRs waiting for my review, oldest first.",
            "",
            "",
            "lightest",
            "",
            "",
            "gh pr list --search review-requested:@me | grep -q .",
            "America/New_York",
            "",
            ""
        ]
    );
}

#[test]
fn a_new_routine_starts_from_a_template_picked_with_the_keyboard() {
    let mut app = app(Density::Comfortable);
    press(&mut app, 'a');
    assert!(app.picker.is_some() && app.form.is_none(), "the templates come first");
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Down);
    insta::assert_snapshot!("the_template_picker", render(&mut app, 100, 24));
    key(&mut app, KeyCode::Esc);
    assert!(app.picker.is_none() && app.form.is_none() && app.sent.is_empty(), "esc closes it, nothing is saved");
    press(&mut app, 'a');
    press(&mut app, '2');
    let form = app.form.as_ref().expect("1–5 picks at once");
    assert_eq!((form.text(cockpit_routines::form::NAME), form.text(cockpit_routines::form::WHEN)), ("Morning sweep", "weekdays 09:00"));
    insta::assert_snapshot!("a_form_filled_from_a_template", render(&mut app, 100, 36));
    assert!(app.sent.is_empty(), "a template fills the form: nothing is saved yet");
    for _ in 0..4 {
        key(&mut app, KeyCode::Tab);
    }
    key(&mut app, KeyCode::Enter);
    assert!(app.sent.is_empty() && app.form.as_ref().unwrap().at == cockpit_routines::form::PRECHECK, "enter from the folder goes on to the precheck");
    key(&mut app, KeyCode::Enter);
    let sent = last(&app);
    let args: Vec<&str> = sent.args.iter().map(|a| a.as_str().unwrap()).collect();
    let t = cockpit_core::routines::TEMPLATES[0];
    assert_eq!(sent.action, "add");
    assert_eq!(args, [t.name, t.when, t.body, "", "", "sonnet", "", "always", t.precheck, "", t.tools, ""]);
    press(&mut app, 'a');
    press(&mut app, '4');
    assert!(app.form.as_ref().unwrap().shell, "the cleanup is a script");
}

#[test]
fn the_form_reads_a_precheck_and_a_time_zone_and_says_what_each_does() {
    let mut app = app(Density::Comfortable);
    press(&mut app, 'a');
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::Tab);
    typed(&mut app, "Paris");
    let form = app.form.as_ref().unwrap();
    assert_eq!(form.problem().as_deref(), Some("\"Paris\" is not a time zone: a name like Europe/Paris, or local"));
    assert_eq!(form.when_hint(&app.clock), ("\"Paris\" is not a time zone: a name like Europe/Paris, or local".to_string(), false));
    for _ in 0.."Paris".len() {
        key(&mut app, KeyCode::Backspace);
    }
    typed(&mut app, "Europe/London");
    // 10:00 at UTC+1 is 09:00 in London: its next 09:00 is tomorrow's.
    let (hint, ok) = app.form.as_ref().unwrap().when_hint(&app.clock);
    assert_eq!((hint.as_str(), ok), ("weekdays at 09:00 in Europe/London · next Fri 10:00, Mon 10:00, Tue 10:00", true));
    for _ in 0..3 {
        key(&mut app, KeyCode::Tab);
    }
    insta::assert_snapshot!(render(&mut app, 90, 32));
}

#[test]
fn a_skipped_run_shows_what_its_precheck_said() {
    let mut app = app(Density::Comfortable);
    app.selected = Some("review-digest".into());
    press(&mut app, ' ');
    key(&mut app, KeyCode::Down);
    insta::assert_snapshot!(render(&mut app, 110, 30));
}

#[test]
fn a_failed_run_says_how_in_one_sentence() {
    let mut app = app(Density::Comfortable);
    app.selected = Some("fix-ci/run/1793257920000".into());
    app.opened.insert("fix-ci".into());
    insta::assert_snapshot!(render(&mut app, 110, 30));
}

#[test]
fn a_stretch_of_skips_shows_its_last_words_not_the_first_ones() {
    let runs = std::env::temp_dir().join(format!("cockpit-routines-test-{}", std::process::id()));
    std::fs::create_dir_all(runs.join("digest")).unwrap();
    let mut app = App::with(Some(clock()), Density::Comfortable, false, runs.clone());
    std::fs::write(runs.join("digest/7.txt"), "nothing waits\n").unwrap();
    assert_eq!(app.output("digest", "7", 100), ["nothing waits"]);
    std::fs::write(runs.join("digest/7.txt"), "").unwrap();
    assert!(app.output("digest", "7", 200).is_empty(), "the stretch's next skip, same id, said nothing");
    let _ = std::fs::remove_dir_all(&runs);
}

#[test]
fn a_form_with_a_bad_when_says_why_and_adds_nothing() {
    let mut app = app(Density::Comfortable);
    press(&mut app, 'a');
    key(&mut app, KeyCode::Enter);
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
    press(&mut app, 'm');
    assert_eq!(last(&app), Sent { action: "model".into(), args: vec![json!("morning-sweep"), json!("haiku")], armed: None }, "sonnet's next");
    press(&mut app, 'D');
    assert_eq!(app.sent.len(), 6, "a delete asks first");
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
    assert_eq!(keys, ["notes-pull", "fix-ci", "standup", "morning-sweep", "ping-team", "review-digest", "weekly-digest"]);
}

#[test]
fn the_sidebar_says_what_runs_and_what_failed() {
    let doc = Doc::parse(&data()).unwrap();
    assert_eq!(cockpit_routines::pinned::sidebar_text(&doc, &clock()), ("\u{f04b}  1 running · 1 failed".into(), true));
    let calm =
        Doc::parse(&json!({"version": 1, "routines": [{"slug": "a", "next": clock().now + 240_000}, {"slug": "b", "paused": true, "next": 1}]})).unwrap();
    assert_eq!(cockpit_routines::pinned::sidebar_text(&calm, &clock()).0, "\u{f017}  next in 4 min");
}

#[test]
fn waiting_for_the_hub() {
    let mut app = App::with(Some(clock()), Density::Comfortable, false, fixtures().join("runs"));
    insta::assert_snapshot!(render(&mut app, 90, 8));
}

#[test]
fn a_routine_a_focus_request_selected_runs_nothing_until_a_move() {
    let mut app = app(Density::Comfortable);
    app.focus("morning-sweep");
    key(&mut app, KeyCode::Enter);
    for c in ['r', 't', 'p', 'D', 'x'] {
        press(&mut app, c);
    }
    assert!(app.sent.is_empty(), "{:?}", app.sent);
    assert!(app.pointed.is_on());
    press(&mut app, 'j');
    assert!(!app.pointed.is_on(), "a move makes the row the person's");
}

/// The app on the fixture with another default agent, which lists these models.
fn app_on(agent: &str, models: &[&str]) -> App {
    let mut doc = data();
    doc["agents"] = json!({"default": agent, "installed": [{"id": agent, "name": "OpenCode", "models": models}]});
    let mut app = App::with(Some(clock()), Density::Comfortable, false, fixtures().join("runs"));
    app.set_doc(&doc);
    app
}

/// A blank form typed as far as its model switch.
fn to_model(app: &mut App) {
    press(app, 'a');
    key(app, KeyCode::Enter);
    typed(app, "Tidy");
    for _ in 0..3 {
        key(app, KeyCode::Tab);
    }
    typed(app, "Tidy the logs");
    for _ in 0..4 {
        key(app, KeyCode::Tab);
    }
}

#[test]
fn another_agent_s_form_offers_the_models_it_lists() {
    let mut app = app_on("opencode", &["", "acme/large", "acme/small"]);
    to_model(&mut app);
    key(&mut app, KeyCode::Right);
    let screen = render(&mut app, 90, 34);
    assert!(screen.contains("model      default   acme/large   acme/small"), "{screen}");
    key(&mut app, KeyCode::Enter);
    assert_eq!((last(&app).action.as_str(), &last(&app).args[5]), ("add", &json!("acme/large")));

    let many: Vec<String> = (0..20).map(|n| format!("acme/model-{n}")).collect();
    let mut app = app_on("opencode", &many.iter().map(String::as_str).collect::<Vec<_>>());
    to_model(&mut app);
    key(&mut app, KeyCode::Left);
    let screen = render(&mut app, 90, 34);
    assert!(screen.contains("‹ acme/model-19 ›  21 of 21"), "a long list one at a time: {screen}");

    let mut app = app_on("opencode", &[""]);
    to_model(&mut app);
    assert!(!render(&mut app, 90, 34).contains("model    "), "an agent that lists none shows no model switch");
}

#[test]
fn the_form_picks_an_installed_agent_and_refreshes_models_without_losing_text() {
    let mut doc = data();
    doc["agents"] = json!({"default": "claude", "installed": [
        {"id": "claude", "name": "Claude Code", "models": ["", "haiku"]},
        {"id": "opencode", "name": "OpenCode", "models": [""]}
    ]});
    let mut app = App::with(Some(clock()), Density::Comfortable, false, fixtures().join("runs"));
    app.set_doc(&doc);
    to_model(&mut app);
    app.form.as_mut().unwrap().at = cockpit_routines::form::KIND;
    key(&mut app, KeyCode::Right);
    assert_eq!(app.form.as_ref().unwrap().agent_set, "opencode");
    assert_eq!(app.form.as_ref().unwrap().text(cockpit_routines::form::PROMPT), "Tidy the logs");
    doc["agents"]["installed"][1]["models"] = json!(["", "Acme/Large@stable"]);
    app.set_doc(&doc);
    let form = app.form.as_mut().unwrap();
    form.at = cockpit_routines::form::MODEL;
    key(&mut app, KeyCode::Right);
    app.set_doc(&doc);
    assert_eq!(app.form.as_ref().unwrap().model, "Acme/Large@stable", "a refresh preserves the selection");
    key(&mut app, KeyCode::Enter);
    assert_eq!((&last(&app).args[4], &last(&app).args[5]), (&json!("opencode"), &json!("Acme/Large@stable")));
}
