//! The monitor app drawn at 60, 90 and 120 columns on the fixture at a fixed time, both densities and both views, what
//! changed since you last looked, its sidebar line, and what each row's keys send to the hub.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use cockpit_core::files::Density;
use cockpit_core::testing::render;
use cockpit_monitor::app::{App, Sent};
use cockpit_monitor::data::{Clock, Doc};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use serde_json::{Value, json};

fn data() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/monitor.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// Wednesday 28 October 2026, 17:56:51 at UTC+1, when the fixture was written.
fn clock() -> Clock {
    let now = DateTime::parse_from_rfc3339("2026-10-28T17:56:51+01:00").unwrap();
    Clock { now: now.with_timezone(&Utc), zone: Some(*now.offset()) }
}

fn app(density: Density) -> App {
    let mut app = App::with(Some(clock()), density, false);
    app.set_doc(&data());
    app
}

fn press(app: &mut App, c: char) {
    app.event(Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)));
}

fn key(app: &mut App, code: KeyCode) {
    app.event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
}

/// The selection moved onto the row with this key.
fn select(app: &mut App, row: &str) {
    app.selected = Some(row.into());
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
fn comfortable_90_release() {
    let mut app = app(Density::Comfortable);
    select(&mut app, "api-production/release/622");
    insta::assert_snapshot!(render(&mut app, 90, 30));
}

#[test]
fn compact_90() {
    let mut app = app(Density::Compact);
    select(&mut app, "api-production/issue/deadbeef");
    insta::assert_snapshot!(render(&mut app, 90, 20));
}

#[test]
fn timeline_120() {
    let mut app = app(Density::Comfortable);
    press(&mut app, 'w');
    insta::assert_snapshot!(render(&mut app, 120, 24));
}

/// You last looked at 17:00 (16:00 UTC): a release, a verdict and an alert since, a release of an app not shown, and
/// an older event.
fn looked(app: &mut App) {
    let event = |at: u64, key: &str, title: &str| json!({"at": at, "doc": "monitor", "app": "monitor", "key": key, "title": title});
    app.feed_events(
        1_793_203_200_000,
        vec![
            event(1_793_206_200_000, "api-production/release/622", "⚠️ api v2026.10.28.1 moved: errors 0.13% → 1.9%"),
            event(1_793_205_900_000, "api-production", "🔔 [api] Sign-in availability alert"),
            event(1_793_205_600_000, "web-production/release/630", "📦 web v2026.10.28.3 released by sam"),
            event(1_793_205_000_000, "search-production/release/12", "📦 search v2026.10.28 released by maya"),
            event(1_793_202_600_000, "billing-production", "🐞 New error in billing: Ecto.NoResultsError"),
        ],
    );
}

#[test]
fn changes_since_you_last_looked() {
    let mut app = app(Density::Comfortable);
    looked(&mut app);
    insta::assert_snapshot!("changes_since_90", render(&mut app, 90, 30));
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Up);
    let screen = render(&mut app, 90, 30);
    assert!(screen.contains("3 changes since 17:00") && !screen.contains(" ▼ ● "), "selecting the api row clears its alert:\n{screen}");
    app.event(Event::FocusLost);
    let screen = render(&mut app, 90, 30);
    assert_eq!(screen, render(&mut self::app(Density::Comfortable), 90, 30), "leaving clears every marker and the head line");
}

#[test]
fn a_folded_app_shows_the_changes_it_hides() {
    let mut app = app(Density::Comfortable);
    looked(&mut app);
    select(&mut app, "web-production");
    press(&mut app, ' ');
    let screen = render(&mut app, 90, 30);
    let web = screen.lines().find(|l| l.contains("web ")).expect("the web row");
    assert!(web.contains("▶ ◆ "), "{screen}");
}

#[test]
fn waiting_for_the_mod() {
    insta::assert_snapshot!(render(&mut App::with(Some(clock()), Density::Comfortable, false), 60, 8));
}

#[test]
fn troubled_apps_come_first_and_unfold() {
    let app = app(Density::Comfortable);
    let keys: Vec<String> = app.rows().into_iter().map(|r| r.key).collect();
    assert_eq!(
        keys,
        [
            "api-production",
            "api-production/monitor/12345678",
            "api-production/issue/deadbeef",
            "api-production/release/622",
            "api-production/release/619",
            "web-production",
            "web-production/monitor/12345680",
            "web-production/release/630",
            "billing-production",
        ]
    );
}

#[test]
fn space_folds_and_unfolds() {
    let mut app = app(Density::Comfortable);
    select(&mut app, "api-production/release/619");
    press(&mut app, ' ');
    assert_eq!(app.selected.as_deref(), Some("api-production"));
    assert_eq!(app.rows().len(), 5);
    press(&mut app, ' ');
    assert_eq!(app.rows().len(), 9);
}

#[test]
fn the_sidebar_line() {
    let doc = Doc::parse(&data()).unwrap();
    assert_eq!(cockpit_monitor::pinned::sidebar_text(&doc), ("⚠️ 1 release moved · 1 alert".to_string(), true));
    let calm = Doc::parse(&json!({"apps": [{"id": "billing-production", "releases": [{"verdict": {"state": "ok"}}]}]})).unwrap();
    assert_eq!(cockpit_monitor::pinned::sidebar_text(&calm), ("\u{f0e4}  all good".to_string(), false));
    let watching = Doc::parse(&json!({"apps": [{"id": "web-production", "releases": [{"verdict": {"state": "watching"}}]}]})).unwrap();
    assert_eq!(cockpit_monitor::pinned::sidebar_text(&watching), ("⏳ 1 release watching".to_string(), false));
}

#[test]
fn a_focus_request_unfolds_the_app() {
    let mut app = app(Density::Comfortable);
    press(&mut app, 'w');
    app.focus("billing");
    assert!(!app.timeline);
    assert_eq!(app.selected.as_deref(), Some("billing-production"));
}

// ---------------------------------------------------------------------------------------------------------------------
// What the keys send

#[test]
fn app_row_keys() {
    let mut app = app(Density::Comfortable);
    select(&mut app, "api-production");
    key(&mut app, KeyCode::Enter);
    let sent = last(&app);
    assert_eq!((sent.action.as_str(), &sent.args[1]), ("investigate", &json!({})), "a troubled app's main action investigates");
    assert_eq!((sent.args[0]["id"].as_str(), sent.args[0]["env"].as_str()), (Some("api-production"), Some("production")));
    press(&mut app, 'o');
    assert_eq!(last(&app).args, [json!("https://app.datadoghq.com/apm/entity/service%3Aapi?env=production")]);
    press(&mut app, 'A');
    assert_eq!(last(&app).args, [json!("https://argocd.example.com/applications/api-production")]);
    press(&mut app, 'p');
    assert_eq!(last(&app), Sent { action: "pin".into(), args: vec![json!("api-production"), json!(false)], armed: None });
    press(&mut app, 'R');
    assert_eq!(last(&app).action, "request_refresh");
    select(&mut app, "billing-production");
    key(&mut app, KeyCode::Enter);
    assert_eq!(last(&app).action, "open_url", "a calm app's main action opens Datadog");
    press(&mut app, 'p');
    assert_eq!(last(&app).args, [json!("billing-production"), json!(true)]);
}

#[test]
fn release_row_keys() {
    let mut app = app(Density::Comfortable);
    select(&mut app, "api-production/release/622");
    key(&mut app, KeyCode::Enter);
    assert_eq!((last(&app).action.as_str(), &last(&app).args[1]), ("investigate", &json!({"release": 622})));
    press(&mut app, 'o');
    let url = last(&app).args[0].as_str().unwrap().to_string();
    assert!(url.starts_with("https://app.datadoghq.com/apm/entity/service%3Aapi?env=production&start=") && url.ends_with("&paused=true"), "{url}");
    press(&mut app, 't');
    assert_eq!((last(&app).action.as_str(), &last(&app).args[1]), ("talk", &json!("622")));
    press(&mut app, 'b');
    assert_eq!((last(&app).action.as_str(), &last(&app).args[1]), ("bug", &json!({"release": 622})));
    let before = app.sent.len();
    press(&mut app, 'u');
    assert!(app.dialog.is_some() && app.sent.len() == before, "a roll back asks first");
    press(&mut app, 'n');
    assert!(app.dialog.is_none() && app.sent.len() == before);
    press(&mut app, 'u');
    press(&mut app, 'y');
    assert_eq!(
        last(&app),
        Sent {
            action: "roll_back".into(),
            args: vec![last(&app).args[0].clone(), json!("622"), json!("v2026.10.28")],
            armed: Some("rollback-api-production-v2026.10.28.1".into())
        }
    );
    // The older release has no roll back, and jonas's has no talk.
    select(&mut app, "api-production/release/619");
    let before = app.sent.len();
    press(&mut app, 'u');
    press(&mut app, 't');
    assert!(app.dialog.is_none() && app.sent.len() == before);
    key(&mut app, KeyCode::Enter);
    assert_eq!(last(&app).action, "open_url", "a calm release's main action opens Datadog");
}

#[test]
fn issue_row_keys() {
    let mut app = app(Density::Comfortable);
    select(&mut app, "api-production/issue/deadbeef");
    key(&mut app, KeyCode::Enter);
    assert_eq!((last(&app).action.as_str(), &last(&app).args[1]), ("fix", &json!("deadbeef")));
    press(&mut app, 'o');
    assert_eq!(last(&app).args, [json!("https://app.datadoghq.com/error-tracking/issue/deadbeef")]);
    press(&mut app, 'b');
    assert_eq!((last(&app).action.as_str(), &last(&app).args[1]), ("bug", &json!({"issue": "deadbeef"})));
    press(&mut app, 'i');
    assert_eq!((last(&app).action.as_str(), &last(&app).args[1]), ("investigate", &json!({"issue": "deadbeef"})));
}

#[test]
fn monitor_row_keys() {
    let mut app = app(Density::Comfortable);
    select(&mut app, "api-production/monitor/12345678");
    key(&mut app, KeyCode::Enter);
    assert_eq!(last(&app).args, [json!("https://app.datadoghq.com/monitors/12345678")]);
    press(&mut app, 'i');
    assert_eq!((last(&app).action.as_str(), &last(&app).args[1]), ("investigate", &json!({"monitor": "12345678"})));
}

#[test]
fn w_switches_to_the_releases_newest_first() {
    let mut app = app(Density::Comfortable);
    press(&mut app, 'w');
    let keys: Vec<String> = app.rows().into_iter().map(|r| r.key).collect();
    assert_eq!(keys, ["web-production/release/630", "api-production/release/622", "api-production/release/619"]);
    press(&mut app, 'w');
    assert_eq!(app.rows().len(), 9);
}

#[test]
fn a_row_a_focus_request_selected_runs_nothing_until_a_move() {
    let mut app = app(Density::Comfortable);
    app.focus("api");
    key(&mut app, KeyCode::Enter);
    for c in ['o', 'A', 'p', 'r', 'u'] {
        press(&mut app, c);
    }
    assert!(app.sent.is_empty(), "{:?}", app.sent);
    assert!(app.pointed.is_on());
    press(&mut app, 'j');
    press(&mut app, 'k');
    key(&mut app, KeyCode::Enter);
    assert_eq!(last(&app).action, "investigate", "moved to, its main action runs");
}

#[test]
fn pin_and_mute_answer_before_the_refresh() {
    let mut app = app(Density::Comfortable);
    let pin = "\u{f08d}";
    let markers = |app: &mut App| render(app, 90, 30).matches(pin).count();
    select(&mut app, "billing-production");
    assert_eq!(markers(&mut app), 1, "only api is pinned at first");
    press(&mut app, 'p');
    let after = render(&mut app, 90, 30);
    assert_eq!(after.matches(pin).count(), 2, "billing shows its marker at once, beside api's");
    assert!(after.contains("unpin"), "its button reads unpin now");
    assert!(after.contains("pinned: its deploys and Datadog numbers load with the next refresh"), "{after}");
    press(&mut app, 'p');
    assert_eq!(markers(&mut app), 1, "a second press takes the pin off again");
    press(&mut app, 'm');
    let muted = render(&mut app, 90, 30);
    assert!(muted.contains("unmute"), "{muted}");
    press(&mut app, 'p');
    let back = render(&mut app, 90, 30);
    assert!(!back.contains("unmute"), "pinning unmutes it");
    assert_eq!(back.matches(pin).count(), 2);
    // A document from before the press does not undo it; one that agrees clears it.
    let mut before = data();
    before["apps"][2]["pinned"] = json!(false);
    app.set_doc(&before);
    assert_eq!(markers(&mut app), 2, "the pin stays on show until the document has it");
    let mut agrees = data();
    agrees["apps"][2]["pinned"] = json!(true);
    app.set_doc(&agrees);
    assert_eq!(markers(&mut app), 2);
    app.set_doc(&before);
    assert_eq!(markers(&mut app), 1, "once the document agreed, what it says goes");
}
