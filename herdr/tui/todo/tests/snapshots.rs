//! The to-do app drawn at 60 and 100 columns on a sandbox-like list, both densities, with its states.

use std::path::PathBuf;

use cockpit_core::files::Density;
use cockpit_core::testing::render;
use cockpit_todo::app::{App, Sel};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

/// The app on a copy of the fixture list, so an edit never touches it.
fn app(test: &str, density: Density) -> App {
    let dir = std::env::temp_dir().join(format!("cockpit-todo-{test}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("todo.md");
    std::fs::copy(fixture("todo.md"), &path).unwrap();
    let config = serde_json::from_str(&std::fs::read_to_string(fixture("config.json")).unwrap()).unwrap();
    App::with(path, config, serde_json::Value::Null, density, false)
}

#[test]
fn comfortable_100() {
    let mut app = app("c100", Density::Comfortable);
    app.sel = Sel::Item("review ENG-398 admin export https://github.com/acme/billing/pull/376".into());
    insta::assert_snapshot!(render(&mut app, 100, 22));
}

#[test]
fn comfortable_60() {
    let mut app = app("c60", Density::Comfortable);
    insta::assert_snapshot!(render(&mut app, 60, 22));
}

#[test]
fn compact_100() {
    let mut app = app("k100", Density::Compact);
    app.sel = Sel::Item("ADR Noor".into());
    insta::assert_snapshot!(render(&mut app, 100, 18));
}

#[test]
fn compact_60_done_unfolded() {
    let mut app = app("k60", Density::Compact);
    app.show_done = true;
    app.sel = Sel::Fold;
    insta::assert_snapshot!(render(&mut app, 60, 18));
}

#[test]
fn marked_and_filtered_100() {
    let mut app = app("m100", Density::Comfortable);
    app.filter = cockpit_core::input::Input::new("re");
    app.marks.insert("ADR Noor".into());
    app.marks.insert("release billing & billing-web (done 2026-10-27)".into());
    app.sel = Sel::Item("ADR Noor".into());
    insta::assert_snapshot!(render(&mut app, 100, 22));
}

#[test]
fn editing_in_place_60() {
    let mut app = app("e60", Density::Comfortable);
    app.start_edit(Some("ADR Noor".into()));
    insta::assert_snapshot!(render(&mut app, 60, 16));
}

#[test]
fn edits_round_trip_through_the_file() {
    let mut app = app("edits", Density::Comfortable);
    app.add("new thing OPS-1");
    app.shift("new thing OPS-1", 1);
    app.toggle(&["ADR Noor".into()]);
    app.delete(&["ask to get assigned on PR reviews, especially the front ones, by adding Maya's team".into()]);
    let text = std::fs::read_to_string(&app.path).unwrap();
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let expected = format!(
        "# To-do\n\n- [ ] Thursday follow up on archive/restore, especially the front part with Lin\n- [ ] new thing OPS-1\n\
         - [ ] review ENG-398 admin export https://github.com/acme/billing/pull/376\n\
         - [x] ADR Noor (done {today})\n- [x] remove admin access on superadmin for sam (done 2026-10-27)\n\
         - [x] release billing & billing-web (done 2026-10-27)\n- [x] Check eng-410 quotas in Metabase (done 2026-10-22)\n"
    );
    assert_eq!(text, expected);
    for _ in 0..4 {
        app.undo();
    }
    assert_eq!(std::fs::read_to_string(&app.path).unwrap(), std::fs::read_to_string(fixture("todo.md")).unwrap());
}

#[test]
fn keeps_what_another_process_added_while_typing() {
    let mut app = app("merge", Density::Comfortable);
    app.start_edit(None);
    let body = std::fs::read_to_string(&app.path).unwrap().replace("# To-do\n\n", "# To-do\n\n- [ ] from /todo\n");
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&app.path, body).unwrap();
    app.editing.as_mut().unwrap().input = cockpit_core::input::Input::new("typed here");
    app.commit_edit();
    let text = std::fs::read_to_string(&app.path).unwrap();
    assert!(text.starts_with("# To-do\n\n- [ ] from /todo\n- [ ] typed here\n"), "{text}");
}
