//! The inbox drawn on the anonymised sandbox fixtures: three widths, both groupings, filtered, with picked rows, a PR's
//! details, the compact layout, what changed since you last looked, and how fresh each source is.

mod common;

use cockpit_core::term::App as _;
use cockpit_core::testing::render;
use common::{app, focus, typed};
use crossterm::event::Event;
use serde_json::{Value, json};

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
fn f_goes_round_the_review_requests_yours_team_bots_and_all() {
    let mut app = app(false);
    assert!(render(&mut app, 120, HEIGHT).contains("f me/team/bots"), "the legend names the key");
    for word in ["yours", "team", "bots", "all"] {
        typed(&mut app, "f");
        let seen = render(&mut app, 120, HEIGHT);
        assert!(seen.contains(&format!("Review requests: {word}")), "f once more says {word}: {seen}");
    }
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
fn todays_routine_runs_that_need_a_look() {
    let mut inputs = common::inputs();
    inputs.routines = Some(json!({"version": 1, "inboxFrom": 0,
        "routines": [{"slug": "sweep", "name": "Morning sweep"}, {"slug": "digest", "name": "Review digest"}, {"slug": "notes", "name": "Pull my notes"}],
        "runs": {
            "sweep": [{"id": "1793170800000", "ok": false, "missed": true, "ended": 1793170800000u64, "summary": "Missed Wed 07:00: cockpit was not running then."}],
            "digest": [{"id": "2", "ok": true, "skipped": "precheck", "ended": 1793175000000u64, "summary": "nothing waits"}],
            "notes": [{"id": "1793174400000", "ok": false, "ended": 1793174402000u64, "summary": "fatal: unable to access github.com", "reason": "Ended with exit 128."}]}}));
    let mut app =
        cockpit_inbox::app::App::fixture(inputs, json!({}), false, cockpit_inbox::sessions::Local::fixed(Default::default(), Vec::new()), common::series());
    insta::assert_snapshot!(render(&mut app, 100, 16));
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

#[test]
fn each_source_says_how_fresh_it_is() {
    // The fixtures' 10:00 UTC: GitHub read two minutes ago, Linear refusing its token since 09:35, CI offline since 09:04.
    let now: u64 = 1_793_181_600_000;
    cockpit_core::header::freeze(now);
    let mut inputs = common::inputs();
    let data = inputs.data.as_mut().unwrap();
    data["errors"] = json!(["Previews: the preview connector is not connected", "Linear: HTTP 401: token refused"]);
    data["hub"] = json!({"errors": ["Linear: HTTP 401: token refused"], "fresh": {
        "github": {"source": "github", "label": "GitHub", "at": now - 120_000, "every": 60_000, "error": null},
        "tickets": {"source": "tickets", "label": "Linear", "at": now - 1_800_000, "every": 60_000, "login": "cockpit connector login Linear",
                    "error": {"kind": "auth", "message": "HTTP 401: token refused", "since": now - 1_500_000}}}});
    inputs.ci["error"] = json!("CI unreachable: io: Connection refused");
    inputs.ci["hub"] = json!({"fresh": {"ci": {"source": "ci", "label": "CI", "at": now - 3_600_000, "every": 30_000,
        "error": {"kind": "network", "message": "unreachable: io: Connection refused", "since": now - 3_360_000}}}});
    let mut app =
        cockpit_inbox::app::App::fixture(inputs, json!({}), false, cockpit_inbox::sessions::Local::fixed(Default::default(), Vec::new()), common::series());
    for width in [160, 90, 50] {
        let screen = render(&mut app, width, 6);
        insta::assert_snapshot!(format!("sources_{width}"), screen);
    }
    let screen = render(&mut app, 90, 6);
    assert!(screen.contains("Previews: the preview connector") && !screen.contains("HTTP 401"), "the hub's errors are told by the chips:\n{screen}");
}

#[test]
fn worktrees_to_clean_up_say_what_keeps_them_and_a_batch_says_what_goes() {
    let mut inputs = common::inputs();
    let tree = |branch: &str, blockers: Value, merged: bool| json!({"path": format!("/w/accounts-web/{branch}"), "repo": "accounts-web", "branch": branch, "changes": 0, "merged": merged, "blockers": blockers});
    inputs.worktrees = vec![
        // Merged on GitHub at the commit it has: the scan took the code host's word, nothing keeps it.
        tree("dev/eng-399-bump-ui", json!([]), false),
        tree(
            "dev/eng-399-remove-icon-workaround",
            json!([{"id": "uncommitted", "words": "2 uncommitted changes"}, {"id": "running", "words": "Claude Code still runs in it"}]),
            true,
        ),
    ];
    let mut app =
        cockpit_inbox::app::App::fixture(inputs, json!({}), false, cockpit_inbox::sessions::Local::fixed(Default::default(), Vec::new()), common::series());
    focus(&mut app, "worktree:/w/accounts-web/dev/eng-399-bump-ui");
    // Tall enough for the whole tree, so the section is on screen wherever the view scrolled.
    let screen = render(&mut app, 120, 120);
    let section = screen.lines().skip_while(|l| !l.contains("To clean up")).take(3).collect::<Vec<_>>().join("\n");
    insta::assert_snapshot!("cleanup_rows", section);
    assert!(screen.contains("Nothing keeps it: removing it deletes its folder; its branch too once git counts it merged."), "{screen}");
    typed(&mut app, "v");
    focus(&mut app, "worktree:/w/accounts-web/dev/eng-399-remove-icon-workaround");
    typed(&mut app, "v");
    typed(&mut app, "D");
    let asked = render(&mut app, 120, 120);
    let lines: Vec<Vec<char>> = asked.lines().map(|l| l.chars().collect()).collect();
    let top = lines.iter().position(|l| l.contains(&'╭')).expect("a dialog");
    let (left, right) = (lines[top].iter().position(|&c| c == '╭').unwrap_or(0), lines[top].iter().position(|&c| c == '╮').unwrap_or(0));
    let question = lines[top..].iter().take_while(|l| !l.contains(&'╰')).map(|l| l[left..=right].iter().collect::<String>()).collect::<Vec<_>>().join("\n");
    insta::assert_snapshot!("cleanup_batch_question", question);
}

#[test]
fn only_a_removal_leaves_the_kept_worktrees_out_and_a_long_list_stays_short() {
    use cockpit_inbox::select;
    let mut inputs = common::inputs();
    let tree = |branch: &str, kept: bool| {
        let blockers = if kept { json!([{"id": "uncommitted", "words": "1 uncommitted change"}]) } else { json!([]) };
        json!({"path": format!("/w/api/{branch}"), "repo": "api", "branch": branch, "changes": u8::from(kept), "merged": true, "blockers": blockers})
    };
    inputs.worktrees = (1..=9).map(|n| tree(&format!("dev/eng-4{n:02}"), n == 9)).collect();
    let app =
        cockpit_inbox::app::App::fixture(inputs, json!({}), false, cockpit_inbox::sessions::Local::fixed(Default::default(), Vec::new()), common::series());
    let tree = app.tree();
    let rows: Vec<usize> = (0..tree.nodes.len()).filter(|&n| tree.nodes[n].row.key.starts_with("worktree:")).collect();
    assert_eq!(rows.len(), 9);
    assert_eq!(select::takes(tree, &rows, select::REMOVE).len(), 8, "the kept one stays out of a removal");
    assert_eq!(select::takes(tree, &rows, "go").len(), 9, "and of nothing else");
    assert_eq!(
        select::ending("session on merged api me/x"),
        "remove worktree: session on merged api me/x? Ends the session in it, then deletes its folder; its branch too once git counts it merged."
    );
    let question = select::question(tree, &rows, select::REMOVE);
    assert!(question.starts_with("remove worktree: 8 worktrees? Their folders are deleted; their branches too once git counts them merged."), "{question}");
    assert!(question.contains("and 3 more") && question.contains("Stays: api dev/eng-409 (1 uncommitted change)."), "{question}");
}

#[test]
fn a_row_a_focus_request_selected_takes_no_key_that_acts_until_a_move() {
    let mut app = app(false);
    app.focus(&json!({"app": "inbox", "key": "https://github.com/acme/api/pull/2219"}));
    typed(&mut app, "\nmvn");
    let screen = render(&mut app, 90, HEIGHT);
    assert!(screen.contains("Move to it first"), "{screen}");
    assert!(!screen.contains("picked") && !screen.contains("Start work"), "nothing picked, merged or started: {screen}");
    assert!(app.pointed.is_on(), "looking around leaves it pointed at");
    typed(&mut app, "jkv");
    assert!(!app.pointed.is_on(), "a move makes it the person's");
    assert!(render(&mut app, 90, HEIGHT).contains("picked"), "and its keys act again");
}
