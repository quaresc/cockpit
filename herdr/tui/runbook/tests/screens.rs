//! The app drawn on a test terminal over three sample runbooks: both densities at three widths, the filter, picked
//! steps, a done cascade stopped mid-way, editing and adding steps, and focus requests.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use cockpit_core::files::Density;
use cockpit_core::testing::render;
use cockpit_runbook::app::{App, Paths};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

/// A fresh copy of the fixtures for one test, the runbooks' mtimes set so the tabs read ENG-410, ENG-420, ENG-398.
fn sandbox(name: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("runbook-screens").join(name);
    let _ = fs::remove_dir_all(&root);
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    for dir in ["runbooks", "handoffs", "focus"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    let base = SystemTime::now() - Duration::from_secs(3600);
    for (i, slug) in ["eng-398", "eng-420", "eng-410"].iter().enumerate() {
        let to = root.join("runbooks").join(format!("{slug}.md"));
        fs::copy(fixtures.join("runbooks").join(format!("{slug}.md")), &to).unwrap();
        fs::File::options().write(true).open(&to).unwrap().set_modified(base + Duration::from_secs(60 * i as u64)).unwrap();
    }
    for entry in fs::read_dir(fixtures.join("handoffs")).unwrap().flatten() {
        fs::copy(entry.path(), root.join("handoffs").join(entry.file_name())).unwrap();
    }
    fs::copy(fixtures.join("inbox.json"), root.join("inbox.json")).unwrap();
    root
}

fn open(name: &str, density: Density) -> (App, PathBuf) {
    let root = sandbox(name);
    let paths = Paths { runbooks: root.join("runbooks"), handoffs: root.join("handoffs"), inbox: root.join("inbox.json"), focus: root.join("focus") };
    let mut app = App::open(paths, density, false);
    app.clock.set_today("28 Oct");
    (app, root)
}

/// The frame once every glide has reached its end, so the snapshot does not depend on the test's speed.
fn screen(app: &mut App, width: u16, height: u16) -> String {
    render(app, width, height);
    app.settle();
    render(app, width, height)
}

fn press(app: &mut App, code: KeyCode) {
    app.event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
}

fn ctrl(app: &mut App, c: char) {
    app.event(Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)));
}

fn typed(app: &mut App, text: &str) {
    for c in text.chars() {
        let mods = if c.is_uppercase() { KeyModifiers::SHIFT } else { KeyModifiers::NONE };
        app.event(Event::Key(KeyEvent::new(KeyCode::Char(c), mods)));
    }
}

fn runbook(root: &Path, slug: &str) -> String {
    fs::read_to_string(root.join("runbooks").join(format!("{slug}.md"))).unwrap()
}

/// The lines of `after` that differ from `before`, as (before, after) pairs; both must have as many lines.
fn changed_lines(before: &str, after: &str) -> Vec<(String, String)> {
    assert_eq!(before.lines().count(), after.lines().count());
    before.lines().zip(after.lines()).filter(|(a, b)| a != b).map(|(a, b)| (a.to_string(), b.to_string())).collect()
}

#[test]
fn comfortable_at_three_widths() {
    for width in [60, 90, 120] {
        let (mut app, _) = open(&format!("comfortable-{width}"), Density::Comfortable);
        insta::assert_snapshot!(format!("comfortable_{width}"), screen(&mut app, width, 40));
    }
}

#[test]
fn compact_at_three_widths() {
    for width in [60, 90, 120] {
        let (mut app, _) = open(&format!("compact-{width}"), Density::Compact);
        // On step 8, an open step its session runs: its buttons show at the row's end.
        typed(&mut app, "jjjjjjj");
        insta::assert_snapshot!(format!("compact_{width}"), screen(&mut app, width, 40));
    }
}

#[test]
fn filtered_as_typed_then_kept() {
    let (mut app, _) = open("filtered", Density::Comfortable);
    typed(&mut app, "/dry");
    insta::assert_snapshot!("filter_typing", screen(&mut app, 90, 30));
    press(&mut app, KeyCode::Enter);
    assert!(!app.filter.editing);
    // The kept filter moves the cursor among its matches only.
    press(&mut app, KeyCode::Char('j'));
    assert_eq!(app.book().unwrap().steps[app.selected].n, "15");
    insta::assert_snapshot!("filter_kept", screen(&mut app, 90, 30));
    press(&mut app, KeyCode::Esc);
    assert!(app.found.is_none());
    assert_eq!(app.shown().len(), 17);
}

#[test]
fn filtered_across_every_runbook() {
    let (mut app, _) = open("filtered-all", Density::Comfortable);
    typed(&mut app, "/");
    typed(&mut app, "/");
    typed(&mut app, "sweep");
    // ENG-410 has no match: the first runbook that has one comes forward.
    assert_eq!(app.book().unwrap().slug, "eng-420");
    insta::assert_snapshot!("filter_global", screen(&mut app, 90, 24));
    // ctrl+/ reaches the same from the list, as most terminals send it.
    press(&mut app, KeyCode::Esc);
    ctrl(&mut app, '7');
    assert!(app.filter.editing && app.filter.global);
}

#[test]
fn picked_steps() {
    let (mut app, _) = open("picked", Density::Comfortable);
    typed(&mut app, "jjjjvjvjjjjv");
    assert_eq!(app.picked.len(), 3);
    insta::assert_snapshot!("picked", screen(&mut app, 90, 40));
    // V picks every open step, and again puts them all back.
    typed(&mut app, "V");
    assert_eq!(app.picked.len(), 13);
    typed(&mut app, "V");
    assert!(app.picked.is_empty());
}

#[test]
fn batch_done_cascades() {
    let (mut app, root) = open("cascade", Density::Comfortable);
    let before = runbook(&root, "eng-410");
    typed(&mut app, "jjjjvjvjv");
    typed(&mut app, "x");
    insta::assert_snapshot!("cascade_confirm", screen(&mut app, 90, 40));
    let t0 = Instant::now();
    app.clock.freeze(t0);
    typed(&mut app, "y");
    let changed = changed_lines(&before, &runbook(&root, "eng-410"));
    assert_eq!(
        changed.iter().map(|(_, after)| after.as_str()).collect::<Vec<_>>(),
        [
            "| 5 | Merge and deploy api (consumer + limits) | api#2198 | ✅ done 28 Oct | |",
            "| 6 | Merge and deploy web (banner on settings page) | web#1836 | ✅ done 28 Oct | |",
            "| 7 | Merge and deploy billing PR 3 (store + broadcast) | billing#379 | ✅ done 28 Oct | |",
        ]
    );
    assert!(app.picked.is_empty());
    // Step 5 sparks, step 6 spins, step 7 has just started.
    app.clock.freeze(t0 + Duration::from_millis(650));
    insta::assert_snapshot!("cascade_mid", screen(&mut app, 90, 40));
    app.clock.freeze(t0 + Duration::from_secs(3));
    assert!(app.tick());
    assert!(app.fx.is_empty());
}

#[test]
fn a_command_and_a_status_edited_in_place() {
    let (mut app, root) = open("edit", Density::Comfortable);
    let before = runbook(&root, "eng-410");
    typed(&mut app, "jjjjjjj");
    typed(&mut app, "e");
    ctrl(&mut app, 'u');
    typed(&mut app, "./bin/backfill_daily_quota_usage --dry-run --limit 10 | tee out.log");
    insta::assert_snapshot!("edit_command", screen(&mut app, 90, 30));
    press(&mut app, KeyCode::Enter);
    assert!(app.editor.is_none());
    let after = runbook(&root, "eng-410");
    assert_eq!(
        changed_lines(&before, &after),
        [(
            "| 8 | Backfill dry run | prod | ⬜ to do | `./bin/backfill_daily_quota_usage --dry-run` |".to_string(),
            "| 8 | Backfill dry run | prod | ⬜ to do | `./bin/backfill_daily_quota_usage --dry-run --limit 10 \\| tee out.log` |".to_string()
        )]
    );
    assert_eq!(app.book().unwrap().steps[7].command, "./bin/backfill_daily_quota_usage --dry-run --limit 10 | tee out.log");
    typed(&mut app, "E");
    ctrl(&mut app, 'u');
    typed(&mut app, "🟡 running, 40%");
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.book().unwrap().steps[7].status, "🟡 running, 40%");
    assert_eq!(changed_lines(&before, &runbook(&root, "eng-410")).len(), 1);
    // Esc leaves the file alone.
    typed(&mut app, "e");
    typed(&mut app, "xyz");
    press(&mut app, KeyCode::Esc);
    assert!(!runbook(&root, "eng-410").contains("xyz"));
}

#[test]
fn a_step_added_under_the_cursor() {
    let (mut app, root) = open("add", Density::Comfortable);
    typed(&mut app, "jjj");
    typed(&mut app, "a");
    typed(&mut app, "Check consumer lag");
    press(&mut app, KeyCode::Tab);
    typed(&mut app, "Datadog");
    press(&mut app, KeyCode::Enter);
    typed(&mut app, "kubectl get pods");
    insta::assert_snapshot!("add_form", screen(&mut app, 90, 30));
    press(&mut app, KeyCode::Enter);
    let text = runbook(&root, "eng-410");
    let rows: Vec<&str> = text.lines().filter(|l| l.starts_with("| ") && !l.starts_with("| #")).collect();
    assert_eq!(rows.len(), 18);
    assert_eq!(rows[4], "| 5 | Check consumer lag | Datadog | ⬜ to do | `kubectl get pods` |");
    assert_eq!(rows[5], "| 6 | Merge and deploy api (consumer + limits) | api#2198 | ⬜ to do, approved green | |");
    assert_eq!(rows[17], "| 18 | Close ENG-410 and ENG-409 | Linear | ⏳ waiting, In Review | |");
    let book = app.book().unwrap();
    assert_eq!(book.steps[app.selected].n, "5");
    assert_eq!(book.steps.iter().map(|s| s.n.as_str()).collect::<Vec<_>>(), (1..=18).map(|n| n.to_string()).collect::<Vec<_>>());
}

#[test]
fn focus_requests_bring_a_step_into_view() {
    let (mut app, _) = open("focus", Density::Comfortable);
    typed(&mut app, "/zzz");
    press(&mut app, KeyCode::Enter);
    app.focus("eng-420#4d");
    let book = app.book().unwrap();
    assert_eq!((book.slug.as_str(), book.steps[app.selected].n.as_str()), ("eng-420", "4d"));
    // The filter hid the step: it gives way.
    assert!(app.found.is_none());
    assert!(app.is_animating());
    insta::assert_snapshot!("focus", screen(&mut app, 90, 30));
    app.focus("eng-398");
    assert_eq!(app.book().unwrap().slug, "eng-398");
    app.focus("nothing#1");
    assert_eq!(app.book().unwrap().slug, "eng-398");
}

#[test]
fn a_failed_handoff_flashes_and_says_why() {
    let (mut app, root) = open("failed", Density::Comfortable);
    render(&mut app, 90, 30);
    app.settle();
    assert!(!app.is_animating());
    let handoff = root.join("handoffs/eng-410-8.json");
    let blocked = fs::read_to_string(&handoff).unwrap().replace("\"running\"", "\"blocked\", \"message\": \"needs the prod console\"");
    fs::write(&handoff, blocked).unwrap();
    app.reread();
    assert!(app.is_animating());
    let screen = screen(&mut app, 90, 30);
    assert!(screen.contains("ENG-410 step 8 blocked: needs the prod"), "{screen}");
}

#[test]
fn an_idle_tick_draws_nothing() {
    let (mut app, _) = open("idle", Density::Comfortable);
    render(&mut app, 90, 30);
    app.settle();
    assert!(!app.tick());
}

#[test]
fn done_by_hand_closes_the_running_handoff() {
    let (mut app, root) = open("done-handoff", Density::Comfortable);
    typed(&mut app, "jjjjjjj");
    typed(&mut app, "x");
    assert!(runbook(&root, "eng-410").contains("| 8 | Backfill dry run | prod | ✅ done 28 Oct |"));
    let handoff: serde_json::Value = serde_json::from_str(&fs::read_to_string(root.join("handoffs/eng-410-8.json")).unwrap()).unwrap();
    assert_eq!((handoff["state"].as_str(), handoff["message"].as_str()), (Some("done"), Some("marked done by hand")));
}
