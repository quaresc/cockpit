//! The meetings app drawn at 60 and 100 columns on a sandbox-like week at a fixed time, both densities, and the week
//! view's picture as the SVG it rasterizes.

use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use cockpit_core::files::Density;
use cockpit_core::testing::render;
use cockpit_meetings::app::App;
use cockpit_meetings::timeline::{self, Layout};
use cockpit_meetings::week::{Clock, Week};
use ratatui_image::picker::Picker;
use serde_json::Value;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

fn data() -> Value {
    serde_json::from_str(&std::fs::read_to_string(fixture("meetings.json")).unwrap()).unwrap()
}

/// Wednesday 28 October 2026, 10:06:30 at UTC+1: a meeting running, one starting in four minutes, one an hour ago.
fn clock() -> Clock {
    let now = DateTime::parse_from_rfc3339("2026-10-28T10:06:30+01:00").unwrap();
    Clock { now: now.with_timezone(&Utc), zone: Some(*now.offset()) }
}

fn app(density: Density) -> App {
    let config = serde_json::json!({"meetingBrief": {"enabled": true}});
    let mut app = App::with(Some(clock()), &config, density, Picker::halfblocks(), false);
    let brief: Vec<String> = std::fs::read_to_string(fixture("brief.md")).unwrap().lines().map(str::to_string).collect();
    app.set_week(&data(), HashMap::from([("evt01_20261028".to_string(), brief)]));
    app
}

#[test]
fn comfortable_100() {
    let mut app = app(Density::Comfortable);
    insta::assert_snapshot!(render(&mut app, 100, 36));
}

#[test]
fn comfortable_60() {
    let mut app = app(Density::Comfortable);
    insta::assert_snapshot!(render(&mut app, 60, 36));
}

#[test]
fn compact_100_brief_open() {
    let mut app = app(Density::Compact);
    app.selected = Some("evt01_20261028".into());
    app.open.insert("evt01_20261028".into());
    app.preparing.insert("evt02_20261028".into());
    insta::assert_snapshot!(render(&mut app, 100, 30));
}

#[test]
fn compact_60() {
    let mut app = app(Density::Compact);
    insta::assert_snapshot!(render(&mut app, 60, 24));
}

#[test]
fn waiting_for_the_calendar() {
    let config = serde_json::json!({});
    let mut app = App::with(Some(clock()), &config, Density::Comfortable, Picker::halfblocks(), false);
    insta::assert_snapshot!(render(&mut app, 60, 8));
}

#[test]
fn week_svg() {
    let week = Week::parse(&data());
    insta::assert_snapshot!(timeline::svg(&week, &clock(), Some("evt01_20261028"), 800.0, 480.0, (8.0, 16.0)));
}

#[test]
fn overlapping_meetings_share_their_column() {
    let week = Week::parse(&data());
    let layout = Layout::new(&week, &clock(), 800.0, 480.0, (8.0, 16.0));
    let block = |id: &str| layout.blocks.iter().find(|b| b.id == id).unwrap().clone();
    let (now, soon) = (block("evt_now"), block("evt_soon"));
    assert!(soon.x > now.x && (soon.w - now.w).abs() < 0.01, "{now:?} {soon:?}");
    assert_eq!(layout.at(now.x + 1.0, now.y + 1.0), Some("evt_now"));
    // Saturday and Sunday have nothing: five days from Wednesday.
    assert_eq!(layout.days.len(), 5);
}

#[test]
fn reports_the_next_meeting() {
    let week = Week::parse(&data());
    assert_eq!(cockpit_meetings::pinned::next_text(&week, &clock()), "\u{f455}  now · Pairing on quotas");
    let later = Clock { now: clock().now + chrono::TimeDelta::minutes(30), ..clock() };
    assert_eq!(cockpit_meetings::pinned::next_text(&week, &later), "\u{f455}  in 38 min · Ledger Move Daily");
}

#[test]
fn week_view_draws_its_picture_in_the_background() {
    let mut app = app(Density::Comfortable);
    app.week_view = true;
    let first = render(&mut app, 80, 30);
    assert!(first.contains("drawing the week…"), "{first}");
    let start = std::time::Instant::now();
    while !app.picture.take() {
        assert!(start.elapsed().as_secs() < 20, "the picture never came");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(app.picture.error.is_none(), "{:?}", app.picture.error);
    let drawn = render(&mut app, 80, 30);
    assert!(drawn.matches('▀').count() > 500, "{drawn}");
}
