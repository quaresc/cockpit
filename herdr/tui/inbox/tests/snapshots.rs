//! The inbox drawn on the anonymised sandbox fixtures: three widths, both groupings, filtered, with picked rows, a PR's
//! details, the compact layout, and what changed since you last looked.

mod common;

use cockpit_core::term::App as _;
use cockpit_core::testing::render;
use common::{app, focus, typed};
use crossterm::event::Event;
use serde_json::json;

const HEIGHT: u16 = 64;

#[test]
fn by_feature_at_three_widths() {
    for width in [60, 90, 120] {
        insta::assert_snapshot!(format!("feature_{width}"), render(&mut app(false), width, HEIGHT));
    }
}

#[test]
fn by_status() {
    let mut app = app(false);
    typed(&mut app, "w");
    insta::assert_snapshot!(render(&mut app, 90, HEIGHT));
}

#[test]
fn filtered_while_typing_and_kept() {
    let mut app = app(false);
    typed(&mut app, "/quota");
    insta::assert_snapshot!("filter_typing", render(&mut app, 90, 30));
    typed(&mut app, "\n");
    insta::assert_snapshot!("filter_kept", render(&mut app, 90, 30));
    typed(&mut app, "\x1b");
    assert!(render(&mut app, 90, HEIGHT).contains("Other PRs"), "esc brings every row back");
}

#[test]
fn picked_rows_share_their_actions() {
    let mut app = app(false);
    focus(&mut app, "https://github.com/acme/api/pull/2219");
    typed(&mut app, "v");
    focus(&mut app, "https://github.com/acme/api/pull/2207");
    typed(&mut app, "v");
    insta::assert_snapshot!(render(&mut app, 90, HEIGHT));
}

#[test]
fn all_alike_picks_every_visible_review_request() {
    let mut app = app(false);
    focus(&mut app, "https://github.com/acme/billing/pull/381");
    typed(&mut app, "V");
    let screen = render(&mut app, 90, HEIGHT);
    assert!(screen.contains("9 review requests picked"), "{screen}");
    typed(&mut app, "\x1b");
    assert!(!render(&mut app, 90, HEIGHT).contains("picked"));
}

#[test]
fn a_failing_prs_details() {
    let mut app = app(false);
    let url = "https://github.com/acme/web/pull/1866";
    focus(&mut app, url);
    let threads = json!({"threads": [{"path": "src/QuotaBanner.tsx", "line": 42, "author": "jonas", "body": "Should this link open in a new tab?"}]});
    let tests = json!({"tests": ["QuotaBanner links to the help center", "QuotaBanner hides when dismissed"]});
    let log = json!({"log": "FAIL src/QuotaBanner.test.tsx\n  ✕ links to the help center (35 ms)\nTests: 2 failed, 41 passed\nexit code 1"});
    app.show_details(url, &[("review_threads", threads), ("failed_tests", tests), ("ci_log", log)]);
    insta::assert_snapshot!(render(&mut app, 90, HEIGHT));
}

#[test]
fn compact() {
    let mut app = app(true);
    focus(&mut app, "https://github.com/acme/web/pull/1866");
    insta::assert_snapshot!("compact_90", render(&mut app, 90, HEIGHT));
    insta::assert_snapshot!("compact_60", render(&mut app, 60, HEIGHT));
    typed(&mut app, "v");
    focus(&mut app, "https://github.com/acme/api/pull/2224");
    typed(&mut app, "v");
    insta::assert_snapshot!("compact_picked", render(&mut app, 90, 20));
}

#[test]
fn changes_since_you_last_looked() {
    let mut app = app(false);
    // You last looked at 09:30; an event before then is old, one whose key matches no row counts in the head only.
    let event = |at: u64, key: &str, title: &str| json!({"at": at, "doc": "inbox", "app": "inbox", "key": key, "title": title});
    app.feed_events(
        1_793_179_800_000,
        vec![
            event(1_793_181_300_000, "https://github.com/acme/web/pull/1866", "🔴 CI failing: web#1866"),
            event(1_793_181_000_000, "https://github.com/acme/billing/pull/381", "👀 Review requested: billing#381 chore(ci): add an SBOM step"),
            event(1_793_180_700_000, "ENG-415", "🎫 ENG-415: In progress → To review"),
            event(1_793_180_520_000, "api/main", "🟢 CI passed: api main"),
            event(1_793_180_400_000, "https://github.com/acme/site/pull/1291", "✅ Approved: site#1291"),
            event(1_793_179_200_000, "ENG-405", "🎫 ENG-405: Todo → In progress"),
        ],
    );
    insta::assert_snapshot!("changes_since_90", render(&mut app, 90, HEIGHT + 1));
    focus(&mut app, "https://github.com/acme/billing/pull/381");
    typed(&mut app, "j");
    typed(&mut app, "k");
    let screen = render(&mut app, 90, HEIGHT + 1);
    assert!(screen.contains("4 changes since 09:30: 🔴  CI failing: web#1866 · 🎫  ENG-415"), "selecting a row clears its change:\n{screen}");
    app.event(Event::FocusLost);
    let mut unchanged = common::app(false);
    focus(&mut unchanged, "https://github.com/acme/billing/pull/381");
    typed(&mut unchanged, "jk");
    assert_eq!(render(&mut app, 90, HEIGHT + 1), render(&mut unchanged, 90, HEIGHT + 1), "leaving clears every marker and the head line");
}
