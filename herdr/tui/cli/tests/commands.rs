//! Each subcommand against fake herdr, terminal-browser and hub: the calls it makes, what it prints and its exit code.

mod common;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{Sandbox, stderr, stdout};
use serde_json::{Value, json};

const SPACES: &str = r#"{"result": {"workspaces": [{"workspace_id": "w1", "label": "[1] x  inbox"}, {"workspace_id": "w3", "label": "[3] y  actions"}]}}"#;

// ---------------------------------------------------------------------------
// actions

#[test]
fn a_tab_in_the_actions_space_prints_its_pane_and_keeps_it_under_its_key() {
    let sandbox = Sandbox::new();
    sandbox.write("home/.cache/cockpit/actions.json", r#"{"review api#1": "w3:p1", "review api#2": "w3:p2"}"#);
    sandbox.herdr(&[
        ("workspace list", SPACES),
        ("tab create", r#"{"result": {"root_pane": {"pane_id": "w3:p9"}}}"#),
        ("pane get w3:p1", r#"{"result": {"pane": {"pane_id": "w3:p1"}}}"#),
        ("pane get w3:p2", r#"{"error": "no such pane"}"#),
    ]);
    let out = sandbox.cockpit(&["actions", "tab", "/repos", "runbook", "talk eng-4"], &[]);
    assert_eq!((out.status.code(), stdout(&out).as_str()), (Some(0), "w3:p9\n"));
    assert!(sandbox.calls().contains(&"herdr tab create --workspace w3 --cwd /repos --label runbook --focus".to_string()));
    let kept: Value = serde_json::from_str(&std::fs::read_to_string(sandbox.cache().join("actions.json")).unwrap()).unwrap();
    assert_eq!(kept, json!({"review api#1": "w3:p1", "talk eng-4": "w3:p9"}));
}

#[test]
fn no_tab_without_the_actions_space() {
    let sandbox = Sandbox::new();
    sandbox.write("home/.config/cockpit/config.json", r#"{"spaces": ["inbox", {"name": "notes"}]}"#);
    sandbox.herdr(&[("workspace list", SPACES)]);
    let out = sandbox.cockpit(&["actions", "tab", "/repos", "runbook"], &[]);
    assert_eq!((out.status.code(), stdout(&out).as_str()), (Some(0), ""));
    assert!(sandbox.calls().is_empty());
}

#[test]
fn wrong_arguments_print_the_usage() {
    let sandbox = Sandbox::new();
    for args in [&["actions"][..], &["actions", "tab", "/repos"], &["actions", "tab", "a", "b", "c", "d"], &[], &["dance"], &["hub"]] {
        let out = sandbox.cockpit(args, &[]);
        assert_eq!(out.status.code(), Some(1), "{args:?}");
        assert!(stderr(&out).contains("cockpit"), "{args:?}");
    }
}

#[test]
fn the_sidebar_lists_the_spaces_sessions() {
    let sandbox = Sandbox::new();
    sandbox.write("home/.config/herdr/config.toml", "[ui]\nsidebar_width = 20\n");
    let tabs = json!({"result": {"tabs": [
        {"workspace_id": "w3", "label": "review api#4", "agent_status": "working"},
        {"workspace_id": "w1", "label": "elsewhere", "agent_status": "working"},
        {"workspace_id": "w3", "label": "a shell", "agent_status": "unknown"},
        {"workspace_id": "w3", "label": "plan eng-4 with a long label", "agent_status": "blocked"},
    ]}});
    sandbox.herdr(&[("workspace list", SPACES), ("tab list", &tabs.to_string())]);
    let out = sandbox.cockpit(&["actions", "sidebar"], &[]);
    assert_eq!(out.status.code(), Some(0));
    let report = sandbox.calls().into_iter().find(|c| c.starts_with("herdr workspace report-metadata")).unwrap();
    assert!(report.starts_with("herdr workspace report-metadata w3 --source cockpit-actions --token act_1=◐ review api#4 --clear-token act_1_wait"));
    assert!(report.contains("--clear-token act_2 --token act_2_wait=⏸ plan eng-4 wi…"), "{report}");
    assert!(report.ends_with("--clear-token act_6 --clear-token act_6_wait"));
}

#[test]
fn the_sidebar_does_nothing_without_the_space() {
    let sandbox = Sandbox::new();
    sandbox.herdr(&[("workspace list", r#"{"result": {"workspaces": []}}"#)]);
    let out = sandbox.cockpit(&["actions", "sidebar"], &[]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(sandbox.calls(), ["herdr workspace list"]);
}

// ---------------------------------------------------------------------------
// act and hub ping, against a fake hub

/// Where the binary looks for the hub with this HOME (see cockpit_core::hub::socket).
fn socket(sandbox: &Sandbox) -> PathBuf {
    let cache = PathBuf::from(sandbox.home().to_string_lossy().into_owned()).join(".cache/cockpit");
    let path = cache.join("hub.sock");
    if path.as_os_str().len() < 100 {
        return path;
    }
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    cache.hash(&mut hasher);
    std::env::temp_dir().join(format!("cockpit-hub-{:016x}.sock", hasher.finish()))
}

/// A hub that answers every request with `answer` and keeps the requests; None as answer reads them and says nothing.
fn fake_hub(sandbox: &Sandbox, answer: Option<Value>) -> Arc<Mutex<Vec<Value>>> {
    let path = socket(sandbox);
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let kept = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut line = String::new();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            kept.lock().unwrap().push(serde_json::from_str(&line).unwrap_or(Value::Null));
            match &answer {
                Some(answer) => {
                    let _ = writeln!(&stream, "{answer}");
                }
                None => std::thread::sleep(Duration::from_secs(5)),
            }
        }
    });
    seen
}

const PR: &str = r#"{"repo": "api", "number": 4, "url": "https://github.com/acme/api/pull/4", "branch": "me/eng-4"}"#;

fn with_inbox(sandbox: &Sandbox) {
    sandbox.write("home/.cache/cockpit/inbox.json", &format!(r#"{{"mine": [], "team": [{PR}], "tickets": [{{"id": "ENG-1", "url": "https://linear.app/acme/issue/ENG-1/title"}}]}}"#));
    sandbox.write(
        "home/.cache/cockpit/ci.json",
        r#"{"runs": [{"repo": "api", "branch": "other", "workflowId": "x"}, {"repo": "api", "branch": "me/eng-4", "workflowId": "wf1"}]}"#,
    );
}

#[test]
fn act_review_asks_the_hub_and_prints_its_note() {
    let sandbox = Sandbox::new();
    with_inbox(&sandbox);
    let seen = fake_hub(&sandbox, Some(json!({"note": "started \"review api#4\" in the actions space", "pending": {}, "refreshing": {}, "error": null})));
    let out = sandbox.cockpit(&["act", "review", "https://github.com/acme/api/pull/4"], &[("HERDR_PANE_ID", "w1:p1")]);
    assert_eq!((out.status.code(), stdout(&out).as_str()), (Some(0), "started \"review api#4\" in the actions space\n"));
    let request = seen.lock().unwrap()[0].clone();
    let pr: Value = serde_json::from_str(PR).unwrap();
    assert_eq!(request, json!({"op": "act", "app": "inbox", "action": "review", "args": [pr], "armed": null, "env": {"HERDR_PANE_ID": "w1:p1"}}));
}

#[test]
fn act_ci_and_rerun_take_the_branchs_run() {
    let sandbox = Sandbox::new();
    with_inbox(&sandbox);
    let seen = fake_hub(&sandbox, Some(json!({"note": "", "pending": {}, "refreshing": {}, "error": null})));
    let url = "https://github.com/acme/api/pull/4";
    assert_eq!(sandbox.cockpit(&["act", "ci", url], &[]).status.code(), Some(0));
    let out = sandbox.cockpit(&["act", "rerun", url], &[]);
    assert_eq!((out.status.code(), stdout(&out).as_str()), (Some(0), ""));
    let seen = seen.lock().unwrap();
    assert_eq!((seen[0]["action"].as_str(), &seen[0]["args"][1]["workflowId"], &seen[0]["args"][2]), (Some("fix_ci"), &json!("wf1"), &json!("here")));
    assert_eq!((seen[1]["action"].as_str(), seen[1]["armed"].as_str()), (Some("rerun"), Some("rerun-wf1")));
}

#[test]
fn act_plan_finds_the_tickets_url() {
    let sandbox = Sandbox::new();
    let seen = fake_hub(&sandbox, Some(json!({"note": "ok", "pending": {}, "refreshing": {}, "error": null})));
    with_inbox(&sandbox);
    assert_eq!(sandbox.cockpit(&["act", "plan", "eng-7"], &[]).status.code(), Some(0));
    sandbox.write("home/.config/cockpit/config.json", r#"{"tickets": {"urlTemplate": "https://t.test/{id}"}}"#);
    assert_eq!(sandbox.cockpit(&["act", "plan", "eng-7"], &[]).status.code(), Some(0));
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0]["args"], json!([{"id": "ENG-7", "url": "https://linear.app/acme/issue/ENG-7"}]));
    assert_eq!(seen[1]["args"], json!([{"id": "ENG-7", "url": "https://t.test/ENG-7"}]));
}

#[test]
fn act_failures_come_back_as_a_notification() {
    let sandbox = Sandbox::new();
    with_inbox(&sandbox);
    sandbox.write("home/.cache/cockpit/ci.json", r#"{"runs": []}"#);
    let seen = fake_hub(&sandbox, Some(json!({"note": "", "pending": {}, "refreshing": {}, "error": "review: 'repo'"})));
    let cases = [
        (vec!["act", "review", "https://x.test/9"], "https://x.test/9 is not in the inbox cache"),
        (vec!["act", "rerun", "https://github.com/acme/api/pull/4"], "no CI run to rerun for api#4"),
        (vec!["act", "review", "https://github.com/acme/api/pull/4"], "review: 'repo'"),
    ];
    for (args, message) in cases {
        let out = sandbox.cockpit(&args, &[]);
        assert_eq!((out.status.code(), stderr(&out).trim()), (Some(1), message));
        assert_eq!(sandbox.calls().last().unwrap(), &format!("notify.sh ⚠️ Cockpit {message} space:inbox"));
    }
    let out = sandbox.cockpit(&["act", "dance", "x"], &[]);
    assert_eq!(out.status.code(), Some(1));
    assert!(std::fs::read_to_string(sandbox.log()).unwrap().contains("notify.sh ⚠️ Cockpit What a notification's button does"));
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn hub_ping_answers_by_its_exit_code() {
    let sandbox = Sandbox::new();
    assert_eq!(sandbox.cockpit(&["hub", "ping"], &[]).status.code(), Some(1));
    assert!(!socket(&sandbox).exists(), "a ping started a hub");
    let seen = fake_hub(&sandbox, Some(json!({"ok": true})));
    assert_eq!(sandbox.cockpit(&["hub", "ping"], &[]).status.code(), Some(0));
    assert_eq!(seen.lock().unwrap()[0], json!({"op": "ping"}));
}

#[test]
fn hub_ping_gives_up_after_two_seconds() {
    let sandbox = Sandbox::new();
    fake_hub(&sandbox, None);
    let start = Instant::now();
    assert_eq!(sandbox.cockpit(&["hub", "ping"], &[]).status.code(), Some(1));
    assert!(start.elapsed() < Duration::from_secs(4));
}

// ---------------------------------------------------------------------------
// browse and shots

const PANES: &str = r#"{"result": {"panes": [{"pane_id": "w2:p1", "tab_id": "w2:t1", "agent_session": {"value": "s-1"}}, {"pane_id": "w5:p1", "tab_id": "w5:t1"}]}}"#;

fn browsers(json: &str) -> String {
    common::says(json)
}

#[test]
fn browse_brings_a_page_already_open_to_the_front() {
    let sandbox = Sandbox::new();
    sandbox.herdr(&[("pane list", PANES)]);
    let ls = browsers(r#"{"browsers": [{"key": "b1", "tabs": [{"id": 3, "url": "https://app.test/a/", "active": true}]}]}"#);
    sandbox.terminal_browser(&[("ls", &ls)]);
    assert_eq!(sandbox.cockpit(&["browse", "https://app.test/a", "s-1"], &[]).status.code(), Some(0));
    assert_eq!(sandbox.calls(), ["herdr pane list", "tb [w2:p1] ls --json", "tb [] action --browser b1 --tab 3 --follow -- get url"]);
    std::fs::write(sandbox.log(), "").unwrap();
    assert_eq!(sandbox.cockpit(&["browse", "--size", "0.4", "--reload", "https://app.test/a"], &[("HERDR_PANE_ID", "w5:p1")]).status.code(), Some(0));
    assert_eq!(sandbox.calls(), ["herdr pane list", "tb [w5:p1] ls --json", "tb [w5:p1] action --browser b1 --tab 3 --follow -- reload"]);
}

#[test]
fn browse_opens_a_new_tab_in_the_browser_beside() {
    let sandbox = Sandbox::new();
    sandbox.herdr(&[("pane list", PANES)]);
    sandbox.terminal_browser(&[("ls", &browsers(r#"{"browsers": [{"key": "b1", "tabs": [{"id": 3, "url": "https://other.test"}]}]}"#))]);
    sandbox.cockpit(&["browse", "https://app.test/b", "s-1"], &[]);
    assert_eq!(sandbox.calls().last().unwrap(), "tb [] new-tab --browser b1 https://app.test/b");
}

#[test]
fn browse_splits_the_pane_when_no_browser_is_beside() {
    let sandbox = Sandbox::new();
    sandbox.herdr(&[("pane list", PANES), ("pane split", r#"{"result": {"pane": {"pane_id": "w2:p7"}}}"#)]);
    sandbox.terminal_browser(&[("ls", &browsers(r#"{"browsers": []}"#))]);
    let out = sandbox.cockpit(&["browse", "--size", "0.3", "https://app.test/?a=1&b", "s-1"], &[]);
    assert_eq!(out.status.code(), Some(0));
    let tb = sandbox.root.join("bin/terminal-browser");
    assert_eq!(
        sandbox.calls()[2..],
        [
            "herdr pane process-info --pane w2:p1".to_string(),
            "herdr pane split --pane w2:p1 --direction right --ratio 0.7 --no-focus".to_string(),
            format!("herdr pane run w2:p7 exec {} open 'https://app.test/?a=1&b'", tb.display()),
        ]
    );
}

#[test]
fn browse_falls_back_to_the_default_browser() {
    let sandbox = Sandbox::new();
    sandbox.herdr(&[("pane list", PANES)]);
    sandbox.cockpit(&["browse", "/tmp/report.html", "s-1"], &[]);
    sandbox.cockpit(&["browse", "https://app.test"], &[("COCKPIT_TERMINAL_BROWSER", "/nowhere/terminal-browser")]);
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    assert_eq!(sandbox.calls(), [format!("{opener} file:///tmp/report.html"), format!("{opener} https://app.test")]);
    let out = sandbox.cockpit(&["browse"], &[]);
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn browse_beside_and_external() {
    let sandbox = Sandbox::new();
    sandbox.herdr(&[("pane list", PANES)]);
    sandbox.terminal_browser(&[(
        "ls",
        &browsers(r#"{"browsers": [{"key": "b1", "tabs": [{"id": 2, "url": "https://x.test", "title": "X"}, {"id": 3, "url": "https://app.test/a", "title": "Café", "active": true}]}]}"#),
    )]);
    let out = sandbox.cockpit(&["browse", "--beside", "s-1"], &[]);
    assert_eq!(stdout(&out).trim(), r#"{"key": "b1", "tab": 3, "url": "https://app.test/a", "title": "Caf\u00e9"}"#);
    assert_eq!(sandbox.cockpit(&["browse", "--external", "s-1"], &[]).status.code(), Some(0));
    assert!(sandbox.calls().last().unwrap().ends_with("open https://app.test/a"));
    let out = sandbox.cockpit(&["browse", "--beside", "s-9"], &[]);
    assert_eq!(stdout(&out), "{}\n");
    let out = sandbox.cockpit(&["browse", "--external", "s-9"], &[]);
    assert_eq!((out.status.code(), stderr(&out).trim()), (Some(1), "no terminal-browser beside this pane"));
}

/// A terminal-browser beside the pane that opens tabs and fails `fail` when given.
fn shooting_browser(sandbox: &Sandbox, fail: Option<&str>) {
    let mut answers = vec![
        ("ls".to_string(), browsers(r#"{"browsers": [{"key": "b1", "tabs": []}]}"#)),
        ("new-tab".to_string(), browsers(r#"{"openedTab": 7}"#)),
    ];
    if let Some(command) = fail {
        answers.push((format!("action --browser b1 --tab 7 -- {command}"), "    echo 'it broke' >&2; exit 1".to_string()));
    }
    let refs: Vec<(&str, &str)> = answers.iter().map(|(p, a)| (p.as_str(), a.as_str())).collect();
    sandbox.terminal_browser(&refs);
}

#[test]
fn shots_at_each_size_in_tabs_of_their_own() {
    let sandbox = Sandbox::new();
    sandbox.herdr(&[("pane list", PANES)]);
    shooting_browser(&sandbox, None);
    let out = sandbox.cockpit(&["shots", "https://app.test", "shots", "--sizes", "tablet,watch,mobile", "--full", "--session", "s-1"], &[]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let dir = sandbox.root.join("shots");
    assert!(dir.is_dir());
    assert_eq!(stdout(&out).trim(), format!(r#"{{"tablet": "{0}/tablet.png", "mobile": "{0}/mobile.png"}}"#, dir.display()));
    let tab: Vec<String> = sandbox.calls().into_iter().filter_map(|c| c.strip_prefix("tb [] ").map(str::to_string)).collect();
    let action = |rest: &str| format!("action --browser b1 --tab 7 -- {rest}");
    assert_eq!(
        tab,
        [
            "new-tab --browser b1 https://app.test".to_string(),
            action("get url"),
            action("set device iPad"),
            action("reload"),
            action("wait --load networkidle"),
            action(&format!("screenshot {}/tablet.png --full", dir.display())),
            action("tab close"),
            "new-tab --browser b1 https://app.test".to_string(),
            action("get url"),
            action("set device iPhone 16"),
            action("reload"),
            action("wait --load networkidle"),
            action(&format!("screenshot {}/mobile.png --full", dir.display())),
            action("tab close"),
            "action --browser b1 done".to_string(),
        ]
    );
}

#[test]
fn shots_say_what_failed_and_close_the_tab() {
    let sandbox = Sandbox::new();
    sandbox.herdr(&[("pane list", PANES)]);
    shooting_browser(&sandbox, Some("reload"));
    let out = sandbox.cockpit(&["shots", "https://app.test", "out", "--sizes", "desktop", "--session", "s-1"], &[]);
    assert_eq!((out.status.code(), stderr(&out).trim()), (Some(1), "reload: it broke"));
    assert_eq!(sandbox.calls().last().unwrap(), "tb [] action --browser b1 --tab 7 -- tab close");
}

#[test]
fn shots_exit_2_without_a_browser_beside_a_pane() {
    let sandbox = Sandbox::new();
    sandbox.herdr(&[("pane list", PANES)]);
    shooting_browser(&sandbox, None);
    let out = sandbox.cockpit(&["shots", "https://app.test", "out", "--session", "s-9"], &[]);
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("take these screenshots in Chrome instead"));
    let out = sandbox.cockpit(&["shots", "https://app.test"], &[]);
    assert_eq!(out.status.code(), Some(1));
}
