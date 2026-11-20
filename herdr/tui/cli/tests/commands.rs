//! Each subcommand against fake herdr, terminal-browser and hub: the calls it makes, what it prints and its exit code.

mod common;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{Sandbox, executable, stderr, stdout};
use serde_json::{Value, json};

const SPACES: &str = r#"{"result": {"workspaces": [{"workspace_id": "w1", "label": "[1] x  inbox"}, {"workspace_id": "w3", "label": "[3] y  actions"}]}}"#;

/// One test of this binary at a time: each runs the binary and fakes as processes, and run together under load (an
/// endpoint agent vetting every exec, say) their calls outlive the binary's timeouts.
fn serial() -> std::sync::MutexGuard<'static, ()> {
    static ONE: Mutex<()> = Mutex::new(());
    ONE.lock().unwrap_or_else(|e| e.into_inner())
}

// ---------------------------------------------------------------------------
// actions

#[test]
fn a_tab_in_the_actions_space_prints_its_pane_and_keeps_it_under_its_key() {
    let _one = serial();
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
    let _one = serial();
    let sandbox = Sandbox::new();
    sandbox.write("home/.config/cockpit/config.json", r#"{"spaces": ["inbox", {"name": "notes"}]}"#);
    sandbox.herdr(&[("workspace list", SPACES)]);
    let out = sandbox.cockpit(&["actions", "tab", "/repos", "runbook"], &[]);
    assert_eq!((out.status.code(), stdout(&out).as_str()), (Some(0), ""));
    assert!(sandbox.calls().is_empty());
}

#[test]
fn wrong_arguments_print_the_usage() {
    let _one = serial();
    let sandbox = Sandbox::new();
    for args in [&["actions"][..], &["actions", "tab", "/repos"], &["actions", "tab", "a", "b", "c", "d"], &[], &["dance"], &["hub", "dance"]] {
        let out = sandbox.cockpit(args, &[]);
        assert_eq!(out.status.code(), Some(1), "{args:?}");
        assert!(stderr(&out).contains("cockpit"), "{args:?}");
    }
}

#[test]
fn the_sidebar_lists_the_spaces_sessions() {
    let _one = serial();
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
    let _one = serial();
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
    sandbox.write(
        "home/.cache/cockpit/inbox.json",
        &format!(r#"{{"mine": [], "team": [{PR}], "tickets": [{{"id": "ENG-1", "url": "https://linear.app/acme/issue/ENG-1/title"}}]}}"#),
    );
    sandbox.write(
        "home/.cache/cockpit/ci.json",
        r#"{"runs": [{"repo": "api", "branch": "other", "workflowId": "x"}, {"repo": "api", "branch": "me/eng-4", "workflowId": "wf1"}]}"#,
    );
}

#[test]
fn act_review_asks_the_hub_and_prints_its_note() {
    let _one = serial();
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
    let _one = serial();
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
    let _one = serial();
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
    let _one = serial();
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
        assert_eq!(sandbox.calls().last().unwrap(), &format!("notify ⚠️ Cockpit {message} space:inbox"));
    }
    let out = sandbox.cockpit(&["act", "dance", "x"], &[]);
    assert_eq!(out.status.code(), Some(1));
    assert!(std::fs::read_to_string(sandbox.log()).unwrap().contains("notify ⚠️ Cockpit What a notification's button does"));
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn hub_act_hands_a_review_action_to_the_hub_and_prints_its_answer() {
    let _one = serial();
    let sandbox = Sandbox::new();
    let seen = fake_hub(&sandbox, Some(json!({"note": "review sent to the agent (1 comment)", "pending": {}, "refreshing": {}, "error": null})));
    let review = json!({"scope": "branch", "repo": "/src/acme/api", "comments": [{"path": "src/a.rs", "line": 12, "text": "use the constant"}]});
    let cases =
        [("send", json!(["t7", review])), ("walkthrough", json!(["/src/acme/api", "branch", "haiku"])), ("stop_walkthrough", json!(["/src/acme/api", "pr"]))];
    for (action, args) in &cases {
        let out = sandbox.cockpit(&["hub", "act", "review", action, &args.to_string()], &[]);
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        assert_eq!(serde_json::from_str::<Value>(&stdout(&out)).unwrap()["note"], "review sent to the agent (1 comment)");
    }
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), cases.len());
    for ((action, args), request) in cases.iter().zip(seen.iter()) {
        assert_eq!((&request["op"], &request["app"], &request["action"], &request["args"]), (&json!("act"), &json!("review"), &json!(action), args));
    }
}

#[test]
fn hub_act_fails_with_the_hubs_error() {
    let _one = serial();
    let sandbox = Sandbox::new();
    fake_hub(&sandbox, Some(json!({"note": "", "pending": {}, "refreshing": {}, "error": "walkthrough: no changes to walk through"})));
    let out = sandbox.cockpit(&["hub", "act", "review", "walkthrough", r#"["/src/acme/api", "uncommitted", "haiku"]"#], &[]);
    assert_eq!((out.status.code(), stderr(&out).trim()), (Some(1), "walkthrough: no changes to walk through"));
}

#[test]
fn hub_act_refuses_every_other_action_and_malformed_arguments() {
    let _one = serial();
    let sandbox = Sandbox::new();
    let seen = fake_hub(&sandbox, Some(json!({"note": "done", "pending": {}, "refreshing": {}, "error": null})));
    let cases: [&[&str]; 22] = [
        &["hub", "act", "review", "send", r#"["other/t7", {"comments": []}]"#],
        &["hub", "act", "review", "send", r#"["app:t7", {"comments": []}]"#],
        &["hub", "act", "review", "send", r#"["w1-p2", {"comments": []}]"#],
        &["hub", "act", "inbox", "review", "[]"],
        &["hub", "act", "review", "open_reviewr", r#"["/src/acme/api"]"#],
        &["hub", "act", "review", "send"],
        &["hub", "act", "review", "walkthrough", "not json"],
        &["hub", "act", "review", "walkthrough", r#"["acme/api", "branch", "haiku"]"#],
        &["hub", "act", "review", "walkthrough", r#"["/src/acme/api", "everything", "haiku"]"#],
        &["hub", "act", "review", "stop_walkthrough", r#"["/src/acme/api", "branch", "extra"]"#],
        &["hub", "act", "review", "send", r#"["", {"comments": []}]"#],
        &["hub", "act", "review", "send", r#"["t7", "use the constant"]"#],
        &["hub", "act", "runbook", "delete", r#"[{"$book": "eng-412"}]"#],
        &["hub", "act", "runbook", "focus_session", r#"[{"$handoff": "eng-412-2"}]"#],
        &["hub", "act", "runbook", "done", r#"[["eng-412", "2"]]"#],
        &["hub", "act", "runbook", "done", r#"[[["eng-412"]]]"#],
        &["hub", "act", "runbook", "set", r#"["eng-412", "2", "command"]"#],
        &["hub", "act", "runbook", "undo", r#"["x"]"#],
        &["hub", "act", "runbook", "run_step", r#"[{"$book": "eng-412"}, {"n": "2", "step": "Deploy"}]"#],
        &["hub", "act", "runbook", "run_step", r#"[{"$book": "eng-412"}, {"n": "2", "step": "s", "where": "w", "status": "t", "command": "c"}]"#, "delete-x"],
        &["hub", "act", "runbook", "run_step", r#"[{"slug": "eng-412"}, {"n": "2", "step": "s", "where": "w", "status": "t", "command": "c"}]"#],
        &["hub", "act", "runbook", "add", r#"["eng-412", 3, "s", "w", "c"]"#],
    ];
    for args in cases {
        let out = sandbox.cockpit(args, &[]);
        assert_eq!(out.status.code(), Some(1), "{args:?} was taken");
        assert!(!stderr(&out).trim().is_empty());
    }
    assert!(seen.lock().unwrap().is_empty(), "a refused action reached the hub");
}

#[test]
fn hub_act_hands_a_runbook_action_to_the_hub_with_its_armed_key() {
    let sandbox = Sandbox::new();
    let seen = fake_hub(&sandbox, Some(json!({"note": "ok", "pending": {}, "refreshing": {}, "error": null})));
    let step = json!({"n": "2", "step": "Deploy", "where": "prod", "status": "open", "command": "make deploy"});
    let cases = [
        ("done", json!([[["eng-412", "2"]]]), None),
        ("set", json!(["eng-412", "2", "command", "make ship"]), None),
        ("add", json!(["eng-412", null, "Smoke", "prod", "make smoke"]), None),
        ("undo", json!([]), None),
        ("run_step", json!([{"$book": "eng-412"}, step]), Some("run-eng-412-2")),
    ];
    for (action, args, armed) in &cases {
        let list = args.to_string();
        let mut command = vec!["hub", "act", "runbook", action, list.as_str()];
        command.extend(armed);
        let out = sandbox.cockpit(&command, &[]);
        assert_eq!(out.status.code(), Some(0), "{action}: {}", stderr(&out));
    }
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), cases.len());
    for ((action, args, armed), request) in cases.iter().zip(seen.iter()) {
        assert_eq!((&request["app"], &request["action"], &request["args"], &request["armed"]), (&json!("runbook"), &json!(action), args, &json!(armed)));
    }
}

#[test]
fn hub_ping_answers_by_its_exit_code() {
    let _one = serial();
    let sandbox = Sandbox::new();
    assert_eq!(sandbox.cockpit(&["hub", "ping"], &[]).status.code(), Some(1));
    assert!(!socket(&sandbox).exists(), "a ping started a hub");
    let seen = fake_hub(&sandbox, Some(json!({"ok": true})));
    assert_eq!(sandbox.cockpit(&["hub", "ping"], &[]).status.code(), Some(0));
    assert_eq!(seen.lock().unwrap()[0], json!({"op": "ping"}));
}

#[test]
fn hub_ping_gives_up_after_two_seconds() {
    let _one = serial();
    let sandbox = Sandbox::new();
    fake_hub(&sandbox, None);
    let start = Instant::now();
    assert_eq!(sandbox.cockpit(&["hub", "ping"], &[]).status.code(), Some(1));
    assert!(start.elapsed() < Duration::from_secs(4));
}

#[test]
fn the_binary_says_its_version_and_serves_as_the_hub() {
    let _one = serial();
    let sandbox = Sandbox::new();
    let version = format!("cockpit {}\n", env!("CARGO_PKG_VERSION"));
    assert_eq!(stdout(&sandbox.cockpit(&["--version"], &[])), version);
    assert_eq!(stdout(&sandbox.cockpit(&["inbox", "--version"], &[])), version);
    let mut hub = std::process::Command::new(env!("CARGO_BIN_EXE_cockpit")).arg("hub").env_clear().envs(sandbox.env()).spawn().unwrap();
    let start = Instant::now();
    while !socket(&sandbox).exists() && start.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(20));
    }
    let answered = sandbox.cockpit(&["hub", "ping"], &[]).status.code();
    let _ = hub.kill();
    let _ = hub.wait();
    assert_eq!(answered, Some(0));
}

// ---------------------------------------------------------------------------
// browse and shots

const PANES: &str =
    r#"{"result": {"panes": [{"pane_id": "w2:p1", "tab_id": "w2:t1", "agent_session": {"value": "s-1"}}, {"pane_id": "w5:p1", "tab_id": "w5:t1"}]}}"#;

fn browsers(json: &str) -> String {
    common::says(json)
}

#[test]
fn browse_brings_a_page_already_open_to_the_front() {
    let _one = serial();
    let sandbox = Sandbox::new();
    sandbox.herdr(&[("pane list", PANES)]);
    let ls = browsers(r#"{"browsers": [{"key": "b1", "tabs": [{"id": 3, "url": "https://app.test/a/", "active": true}]}]}"#);
    sandbox.terminal_browser(&[("ls", &ls)]);
    let out = sandbox.cockpit(&["browse", "https://app.test/a", "s-1"], &[]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(sandbox.calls(), ["herdr pane list", "tb [w2:p1] ls --json", "tb [] action --browser b1 --tab 3 --follow -- get url"]);
    std::fs::write(sandbox.log(), "").unwrap();
    assert_eq!(sandbox.cockpit(&["browse", "--size", "0.4", "--reload", "https://app.test/a"], &[("HERDR_PANE_ID", "w5:p1")]).status.code(), Some(0));
    assert_eq!(sandbox.calls(), ["herdr pane list", "tb [w5:p1] ls --json", "tb [w5:p1] action --browser b1 --tab 3 --follow -- reload"]);
}

#[test]
fn browse_opens_a_new_tab_in_the_browser_beside() {
    let _one = serial();
    let sandbox = Sandbox::new();
    sandbox.herdr(&[("pane list", PANES)]);
    sandbox.terminal_browser(&[("ls", &browsers(r#"{"browsers": [{"key": "b1", "tabs": [{"id": 3, "url": "https://other.test"}]}]}"#))]);
    sandbox.cockpit(&["browse", "https://app.test/b", "s-1"], &[]);
    assert_eq!(sandbox.calls().last().unwrap(), "tb [] new-tab --browser b1 https://app.test/b");
}

#[test]
fn browse_splits_the_pane_when_no_browser_is_beside() {
    let _one = serial();
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

/// A listing that fails is no answer: browse says so and opens nothing, beside the pane or in the default browser.
#[test]
fn browse_fails_when_the_browser_beside_cannot_be_listed() {
    let _one = serial();
    let sandbox = Sandbox::new();
    sandbox.herdr(&[("pane list", PANES)]);
    sandbox.terminal_browser(&[("ls", "    echo 'no server' >&2; exit 3")]);
    let out = sandbox.cockpit(&["browse", "https://app.test/a", "s-1"], &[]);
    assert_eq!((out.status.code(), stderr(&out).trim()), (Some(1), "browse: terminal-browser ls: no server"));
    assert_eq!(sandbox.calls(), ["herdr pane list", "tb [w2:p1] ls --json"], "nothing split, nothing opened elsewhere");
    let out = sandbox.cockpit(&["browse", "--beside", "s-1"], &[]);
    assert_eq!((out.status.code(), stdout(&out).as_str()), (Some(1), ""), "never {{}} as if nothing were beside it");
}

#[test]
fn browse_falls_back_to_the_default_browser() {
    let _one = serial();
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
    let _one = serial();
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

// The Herdr app: a session's terminal t2 shows in pane p62d2, in a tab with t3, a terminal-browser on ttys013.

const APP_SPACES: &str = r#"{"spaces": [{"id": "sp1", "tabs": [{"id": "tb1", "terminals": [{"terminal": "t2", "pane": "p62d2"}, {"terminal": "t3", "pane": "pe496"}]}, {"id": "tb2", "terminals": [{"terminal": "t7", "pane": "pa11"}]}]}]}"#;

/// The Herdr app's herdr-app and hc, and a ps that knows the browser's tty; terminal-browser fails as it does in an app
/// terminal (Ghostty's variables, no herdr pane) unless it is told the pane.
fn app_sandbox(listed: &str) -> Sandbox {
    let sandbox = Sandbox::new();
    let says = common::says;
    let split = format!("    : > '{}'\n{}", sandbox.root.join("split").display(), says(r#"{"pane": "p9", "terminal": "t9"}"#));
    let app = [("ls", says(APP_SPACES)), ("do pane.split", split), ("*", says("{}"))];
    let refs: Vec<(&str, &str)> = app.iter().map(|(p, a)| (*p, a.as_str())).collect();
    executable(&sandbox.root.join("bin/herdr-app"), &common::fake("herdr-app", &sandbox.log(), &refs));
    let hc = [
        ("get t3", says(r#"{"pid": 700, "foreground": {"name": "terminal-browser", "pid": 4242}}"#)),
        ("get t7", says(r#"{"pid": 900, "foreground": {"name": "terminal-browser", "pid": 4343}}"#)),
        ("*", says(r#"{"pid": 500, "foreground": {"name": "claude", "pid": 501}}"#)),
    ];
    let refs: Vec<(&str, &str)> = hc.iter().map(|(p, a)| (*p, a.as_str())).collect();
    executable(&sandbox.root.join("bin/hc"), &common::fake("hc", &sandbox.log(), &refs));
    executable(
        &sandbox.root.join("bin/ps"),
        "#!/bin/sh\ncase \"$*\" in\n  '-o tty= -p 4242') echo ttys013 ;;\n  '-o tty= -p 4343') echo ttys020 ;;\n  *) echo 1 ;;\nesac\n",
    );
    let ghostty = "    [ -n \"$HERDR_PANE_ID\" ] || { echo 'terminal-browser: Ghostty is not running, so there is no window to script' >&2; exit 1; }\n";
    // Once split, the new browser lists.
    let started =
        browsers(r#"{"browsers": [{"key": "b9", "tty": "/dev/ttys013", "tabs": [{"id": 1, "url": "https://app.test/c", "title": "C", "active": true}]}]}"#);
    let ls = format!("{ghostty}    if [ -e '{}' ]; then\n{started}\n    else\n{}\n    fi", sandbox.root.join("split").display(), browsers(listed));
    sandbox.terminal_browser(&[("ls", &ls), ("*", ghostty)]);
    sandbox
}

/// A session in the app terminal t2 with none of herdr's variables, as the Herdr app starts its terminals.
fn in_app(sandbox: &Sandbox) -> Vec<(String, String)> {
    let bin = |name: &str| sandbox.root.join("bin").join(name).to_string_lossy().into_owned();
    vec![
        ("HC_TERMINAL_ID".into(), "t2".into()),
        ("TERM_PROGRAM".into(), "ghostty".into()),
        ("HERDR_APP_HOME".into(), sandbox.root.join("app-home").to_string_lossy().into_owned()),
        ("HERDR_APP_BIN".into(), bin("herdr-app")),
        ("HC_BIN".into(), bin("hc")),
    ]
}

fn in_app_refs(env: &[(String, String)]) -> Vec<(&str, &str)> {
    env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect()
}

fn tb_calls(sandbox: &Sandbox) -> Vec<String> {
    sandbox.calls().into_iter().filter(|c| c.starts_with("tb ")).collect()
}

#[test]
fn in_the_app_terminal_browser_is_told_the_sessions_pane_and_the_apps_socket() {
    let _one = serial();
    let sandbox = app_sandbox(
        r#"{"browsers": [{"key": "b1", "tty": "/dev/ttys013", "tabs": [{"id": 3, "url": "https://app.test/a", "title": "A", "active": true}]}, {"key": "b2", "tty": "/dev/ttys020", "tabs": []}]}"#,
    );
    let env = in_app(&sandbox);
    let socket = sandbox.root.join("app-home/herdr-app.sock");
    let out = sandbox.cockpit(&["browse", "--beside"], &in_app_refs(&env));
    assert_eq!(stdout(&out).trim(), r#"{"key": "b1", "tab": 3, "url": "https://app.test/a", "title": "A"}"#, "{}", stderr(&out));
    std::fs::write(sandbox.log(), "").unwrap();
    let out = sandbox.cockpit(&["browse", "https://app.test/b"], &in_app_refs(&env));
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let told = format!("tb [p62d2 {}]", socket.display());
    assert_eq!(tb_calls(&sandbox), [format!("{told} ls --all --json"), format!("{told} new-tab --browser b1 https://app.test/b")], "b2 is another tab's");
    let out = sandbox.cockpit(&["browse", "--env"], &in_app_refs(&env));
    assert_eq!(stdout(&out), format!("export HERDR_PANE_ID=p62d2\nexport HERDR_SOCKET_PATH={}\n", socket.display()));
}

#[test]
fn in_the_app_a_browser_splits_right_of_the_sessions_pane() {
    let _one = serial();
    let sandbox = app_sandbox(r#"{"browsers": []}"#);
    let env = in_app(&sandbox);
    let out = sandbox.cockpit(&["browse", "https://app.test/c"], &in_app_refs(&env));
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let tb = sandbox.root.join("bin/terminal-browser");
    let app: Vec<String> = sandbox.calls().into_iter().filter(|c| c.starts_with("herdr-app do")).collect();
    assert_eq!(
        app,
        [
            format!(r#"herdr-app do pane.split {{"argv":["{}","open","https://app.test/c"],"direction":"right","pane":"p62d2"}}"#, tb.display()),
            r#"herdr-app do pane.focus {"terminal":"t2"}"#.to_string(),
        ]
    );
    let listings = tb_calls(&sandbox).iter().filter(|c| c.ends_with("ls --all --json")).count();
    assert_eq!(listings, 2, "browse returns once the new browser lists, so --beside finds it");
    let out = sandbox.cockpit(&["browse", "--beside"], &in_app_refs(&env));
    assert_eq!(stdout(&out).trim(), r#"{"key": "b9", "tab": 1, "url": "https://app.test/c", "title": "C"}"#);
}

#[test]
fn in_the_app_a_session_shown_nowhere_says_why_and_opens_the_default_browser() {
    let _one = serial();
    let sandbox = app_sandbox(r#"{"browsers": []}"#);
    let mut env = in_app(&sandbox);
    env[0].1 = "t5".into();
    let out = sandbox.cockpit(&["browse", "https://app.test/d"], &in_app_refs(&env));
    assert_eq!(
        (out.status.code(), stderr(&out).trim()),
        (Some(0), "browse: the Herdr window shows terminal t5 in no pane: opening the page in the default browser")
    );
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    assert_eq!(sandbox.calls().last().unwrap(), &format!("{opener} https://app.test/d"));
    assert!(tb_calls(&sandbox).is_empty() && !sandbox.calls().iter().any(|c| c.contains("pane.split")), "nothing placed beside another pane");
}

/// A terminal-browser beside the pane that opens tabs and fails `fail` when given.
fn shooting_browser(sandbox: &Sandbox, fail: Option<&str>) {
    let mut answers =
        vec![("ls".to_string(), browsers(r#"{"browsers": [{"key": "b1", "tabs": []}]}"#)), ("new-tab".to_string(), browsers(r#"{"openedTab": 7}"#))];
    if let Some(command) = fail {
        answers.push((format!("action --browser b1 --tab 7 -- {command}"), "    echo 'it broke' >&2; exit 1".to_string()));
    }
    let refs: Vec<(&str, &str)> = answers.iter().map(|(p, a)| (p.as_str(), a.as_str())).collect();
    sandbox.terminal_browser(&refs);
}

#[test]
fn shots_at_each_size_in_tabs_of_their_own() {
    let _one = serial();
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
    let _one = serial();
    let sandbox = Sandbox::new();
    sandbox.herdr(&[("pane list", PANES)]);
    shooting_browser(&sandbox, Some("reload"));
    let out = sandbox.cockpit(&["shots", "https://app.test", "out", "--sizes", "desktop", "--session", "s-1"], &[]);
    assert_eq!((out.status.code(), stderr(&out).trim()), (Some(1), "reload: it broke"));
    assert_eq!(sandbox.calls().last().unwrap(), "tb [] action --browser b1 --tab 7 -- tab close");
}

#[test]
fn shots_exit_2_without_a_browser_beside_a_pane() {
    let _one = serial();
    let sandbox = Sandbox::new();
    sandbox.herdr(&[("pane list", PANES)]);
    shooting_browser(&sandbox, None);
    let out = sandbox.cockpit(&["shots", "https://app.test", "out", "--session", "s-9"], &[]);
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("take these screenshots in Chrome instead"));
    let out = sandbox.cockpit(&["shots", "https://app.test"], &[]);
    assert_eq!(out.status.code(), Some(1));
}

// store, against a fake hub

#[test]
fn store_hands_the_mods_file_to_the_hub_and_prints_the_document() {
    let _one = serial();
    let sandbox = Sandbox::new();
    let seen = fake_hub(&sandbox, Some(json!({"value": {"version": 1, "previews": []}})));
    sandbox.write("home/.cache/cockpit/store-inbox-x.json", r#"{"version": 1, "previews": []}"#);
    let file = sandbox.cache().join("store-inbox-x.json");
    let out = sandbox.cockpit(&["store", "patch", "inbox", &file.to_string_lossy()], &[]);
    assert_eq!(stdout(&out), "{\"previews\":[],\"version\":1}\n");
    assert!(!file.exists(), "the file goes once read");
    assert_eq!(seen.lock().unwrap().last().unwrap(), &json!({"op": "store.patch", "name": "inbox", "fields": {"version": 1, "previews": []}}));
}

#[test]
fn store_leases_print_granted_or_held() {
    let _one = serial();
    let sandbox = Sandbox::new();
    let seen = fake_hub(&sandbox, Some(json!({"granted": false})));
    assert_eq!(stdout(&sandbox.cockpit(&["store", "renew", "inbox", "s1-abc", "90"], &[])), "held\n");
    assert_eq!(seen.lock().unwrap().last().unwrap(), &json!({"op": "store.lease", "name": "inbox", "holder": "s1-abc", "ttl": 90, "renew": true}));
    assert_eq!(stdout(&sandbox.cockpit(&["store", "release", "inbox", "s1-abc"], &[])), "released\n");
    assert_eq!(sandbox.cockpit(&["store", "lease", "inbox"], &[]).status.code(), Some(1));
}

// ---------------------------------------------------------------------------
// search and events, against a fake hub

#[test]
fn search_prints_the_hubs_answer() {
    let _one = serial();
    let sandbox = Sandbox::new();
    let answer = json!({"entries": [{"app": "inbox", "kind": "ticket", "title": "ENG-401 Sign in with Google", "help": "In Progress", "target": {"app": "inbox", "key": "ENG-401"}}]});
    let seen = fake_hub(&sandbox, Some(answer.clone()));
    let out = sandbox.cockpit(&["search", "sign", "in"], &[]);
    assert_eq!(serde_json::from_str::<Value>(&stdout(&out)).unwrap(), answer);
    assert_eq!(seen.lock().unwrap()[0], json!({"op": "search", "query": "sign in"}));
    assert_eq!(sandbox.cockpit(&["search"], &[]).status.code(), Some(1));
}

#[test]
fn events_ask_since_when_and_say_each_ones_local_time() {
    let _one = serial();
    let sandbox = Sandbox::new();
    let event = json!({"at": 1_761_660_000_000_u64, "doc": "inbox", "app": "inbox", "key": "ENG-412", "title": "🎫 ENG-412: To review → Done"});
    let seen = fake_hub(&sandbox, Some(json!({"events": [event]})));
    let out = sandbox.cockpit(&["events", "--since", "1761000000000", "--app", "inbox"], &[]);
    let printed: Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(printed["events"][0]["key"], "ENG-412");
    assert!(regex::Regex::new(r"^2025-10-2\d \d\d:\d\d$").unwrap().is_match(printed["events"][0]["when"].as_str().unwrap()));
    assert_eq!(seen.lock().unwrap()[0], json!({"op": "events", "since": 1_761_000_000_000_u64, "app": "inbox", "limit": 500}));
    let out = sandbox.cockpit(&["events", "--app", "todo"], &[]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("--since 30m|2h|1d"));
}

// ---------------------------------------------------------------------------
// notify and focus

#[test]
fn notify_hands_the_notification_over_and_keeps_the_terminal() {
    let _one = serial();
    let sandbox = Sandbox::new();
    let pr = "https://github.com/acme/web/pull/9";
    let (review, docs) = (format!("Review=act:review:{pr}"), "Docs=https://x.test/9".to_string());
    let args = ["notify", "📥 Inbox", "web#9 Dark mode", "https://x.test/9", &review, &docs];
    let out = sandbox.cockpit(&args, &[("__CFBundleIdentifier", "com.mitchellh.ghostty")]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    // Any process may post one: a button's act becomes the row it is about, where the person acts themselves.
    assert_eq!(sandbox.calls(), [format!("notify 📥 Inbox web#9 Dark mode https://x.test/9 Review=inbox:{pr} {docs}")]);
    let terminal = sandbox.cache().join("terminal-app");
    assert_eq!(std::fs::read_to_string(&terminal).unwrap(), "com.mitchellh.ghostty\n");
    // A click handled by the notifier app itself does not make it the terminal.
    sandbox.cockpit(&["notify", "a", "b"], &[("__CFBundleIdentifier", "dev.cockpit.notifier")]);
    assert_eq!(std::fs::read_to_string(&terminal).unwrap(), "com.mitchellh.ghostty\n");
    assert_eq!(sandbox.cockpit(&["notify", "only a title"], &[]).status.code(), Some(1));
}

#[test]
fn setup_run_runs_the_command_under_itself_and_exits_as_it_did() {
    let _one = serial();
    let sandbox = Sandbox::new();
    let out = sandbox.cockpit(&["setup-run", "/bin/sh", "-c", "echo ran; exit 3"], &[]);
    assert_eq!((out.status.code(), String::from_utf8_lossy(&out.stdout).into_owned()), (Some(3), "ran\n".to_string()));
    let killed = sandbox.cockpit(&["setup-run", "/bin/sh", "-c", "kill -TERM $$"], &[]);
    assert_eq!(killed.status.code(), Some(128 + 15), "a signal reads as the shell reports it");
    assert_eq!(sandbox.cockpit(&["setup-run"], &[]).status.code(), Some(1));
}

#[test]
fn notify_is_silent_when_notifications_are_off() {
    let _one = serial();
    let sandbox = Sandbox::new();
    sandbox.write("home/.config/cockpit/config.json", r#"{"notifications": false}"#);
    assert_eq!(sandbox.cockpit(&["notify", "a", "b", "space:inbox"], &[]).status.code(), Some(0));
    assert!(sandbox.calls().is_empty());
}

#[test]
fn focus_brings_each_target_into_view() {
    let _one = serial();
    let sandbox = Sandbox::new();
    let spaces = r#"{"result": {"workspaces": [{"workspace_id": "w1", "label": "[1] x  inbox"}, {"workspace_id": "w5", "label": "[5] y  runbook"}, {"workspace_id": "w6", "label": "[6] z  meetings"}]}}"#;
    sandbox.herdr(&[("workspace list", spaces)]);
    for target in ["space:inbox", "workspace:w9", "tab:t4", "runbook:eng-4:a,b", "meeting:ev1", "space:nowhere", "dance"] {
        let out = sandbox.cockpit(&["focus", target], &[]);
        assert_eq!(out.status.code(), Some(0), "{target}: {}", stderr(&out));
    }
    let focused: Vec<String> = sandbox.calls().into_iter().filter(|c| !c.ends_with("workspace list")).collect();
    assert_eq!(focused, ["herdr workspace focus w1", "herdr workspace focus w9", "herdr tab focus t4", "herdr workspace focus w5", "herdr workspace focus w6"]);
    let runbook: Value = serde_json::from_str(&std::fs::read_to_string(sandbox.cache().join("runbook-focus.json")).unwrap()).unwrap();
    assert_eq!((&runbook["slug"], &runbook["steps"]), (&json!("eng-4"), &json!(["a", "b"])));
    assert!(runbook["at"].as_f64().unwrap() > 1e9);
    let meeting: Value = serde_json::from_str(&std::fs::read_to_string(sandbox.cache().join("meetings-focus.json")).unwrap()).unwrap();
    assert_eq!(meeting["id"], "ev1");
    sandbox.cockpit(&["focus", "runbook:eng-5"], &[]);
    let runbook: Value = serde_json::from_str(&std::fs::read_to_string(sandbox.cache().join("runbook-focus.json")).unwrap()).unwrap();
    assert_eq!((&runbook["slug"], &runbook["steps"]), (&json!("eng-5"), &json!([])));
}

#[test]
fn focus_todo_brings_the_to_do_space_with_that_item_selected() {
    let _one = serial();
    let sandbox = Sandbox::new();
    let spaces = r#"{"result": {"workspaces": [{"workspace_id": "w1", "label": "[1] x  inbox"}, {"workspace_id": "w4", "label": "[4] t  to-do"}]}}"#;
    sandbox.herdr(&[("workspace list", spaces)]);
    let out = sandbox.cockpit(&["focus", "todo:review ENG-398 admin export"], &[]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let todo: Value = serde_json::from_str(&std::fs::read_to_string(sandbox.cache().join("focus/todo.json")).unwrap()).unwrap();
    assert_eq!(todo, json!({"app": "todo", "key": "review ENG-398 admin export"}));
    let focused: Vec<String> = sandbox.calls().into_iter().filter(|c| !c.ends_with("workspace list")).collect();
    assert_eq!(focused, ["herdr workspace focus w4"]);
    let usage = sandbox.cockpit(&["focus"], &[]);
    assert!(stderr(&usage).contains("cockpit focus todo:<text>"), "the help names it: {}", stderr(&usage));
}

#[test]
fn focus_opens_a_url_and_brings_the_terminal_forward() {
    let _one = serial();
    let sandbox = Sandbox::new();
    sandbox.write("home/.cache/cockpit/terminal-app", "com.mitchellh.ghostty\n");
    sandbox.cockpit(&["focus", "https://x.test/a"], &[]);
    sandbox.cockpit(&["focus", "tab:t4"], &[]);
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    let mut expected = vec![format!("{opener} https://x.test/a"), "herdr tab focus t4".to_string()];
    if cfg!(target_os = "macos") {
        expected.push(r#"osascript -e tell application "Ghostty" to focus (first terminal whose tty is "/dev/ttys007")"#.into());
    }
    assert_eq!(sandbox.calls(), expected);
}

#[test]
fn focus_brings_the_app_forward_when_ghostty_cannot_say_where_herdr_runs() {
    let _one = serial();
    let sandbox = Sandbox::new();
    executable(&sandbox.root.join("bin/osascript"), "#!/bin/sh\nexit 1\n");
    sandbox.write("home/.cache/cockpit/terminal-app", "com.mitchellh.ghostty\n");
    sandbox.cockpit(&["focus", "tab:t4"], &[]);
    let mut expected = vec!["herdr tab focus t4".to_string()];
    if cfg!(target_os = "macos") {
        expected.push("open -b com.mitchellh.ghostty".into());
    }
    assert_eq!(sandbox.calls(), expected);
}

#[test]
fn a_notification_button_brings_its_row_and_does_nothing_else() {
    let _one = serial();
    let sandbox = Sandbox::new();
    with_inbox(&sandbox);
    let seen = fake_hub(&sandbox, Some(json!({"note": "merged", "pending": {}, "refreshing": {}, "error": null})));
    let spaces = r#"{"result": {"workspaces": [{"workspace_id": "w1", "label": "[1] x  inbox"}, {"workspace_id": "w2", "label": "[2] y  routines"}]}}"#;
    sandbox.herdr(&[("workspace list", spaces)]);
    let pr = "https://github.com/acme/api/pull/4";
    for target in [format!("act:merge:{pr}"), format!("act:review:{pr}/files"), "act:plan:eng-1".into(), "act:routine:sweep".into()] {
        let target = target.as_str();
        let out = sandbox.cockpit(&["focus", target], &[]);
        assert_eq!(out.status.code(), Some(0), "{target}: {}", stderr(&out));
    }
    assert!(seen.lock().unwrap().is_empty(), "the hub is asked to do nothing");
    assert!(!sandbox.cache().join("act.log").exists());
    let inbox: Value = serde_json::from_str(&std::fs::read_to_string(sandbox.cache().join("focus/inbox.json")).unwrap()).unwrap();
    assert_eq!(inbox, json!({"app": "inbox", "key": "ENG-1"}));
    let routine: Value = serde_json::from_str(&std::fs::read_to_string(sandbox.cache().join("focus/routines.json")).unwrap()).unwrap();
    assert_eq!(routine, json!({"app": "routines", "key": "sweep"}));
    let focused: Vec<String> = sandbox.calls().into_iter().filter(|c| !c.ends_with("workspace list")).collect();
    assert_eq!(focused, ["herdr workspace focus w1", "herdr workspace focus w1", "herdr workspace focus w1", "herdr workspace focus w2"]);
    assert_eq!(sandbox.cockpit(&["focus", "act:deploy:prod"], &[]).status.code(), Some(1));
}

#[test]
fn focus_update_runs_update_sh_in_the_background() {
    let _one = serial();
    let sandbox = Sandbox::new();
    sandbox.write("herdr/update.sh", "echo updating \"$@\"\n");
    assert_eq!(sandbox.cockpit(&["focus", "update"], &[]).status.code(), Some(0));
    let log = sandbox.cache().join("update.log");
    let start = Instant::now();
    while std::fs::read_to_string(&log).unwrap_or_default() != "updating\n" {
        assert!(start.elapsed() < Duration::from_secs(5), "update.sh did not run");
        std::thread::sleep(Duration::from_millis(50));
    }
}

// ---------------------------------------------------------------------------
// options

#[test]
fn options_say_what_an_update_brought_once_and_set_nothing_without_a_terminal() {
    let _one = serial();
    let sandbox = Sandbox::new();
    sandbox.write("home/.config/cockpit/config.json", "{\n  \"reposDir\": \"~/code\",\n  \"queue\": {\"drain\": true}\n}\n");
    sandbox.write("home/.cache/cockpit/options.json", r#"{"ran": "0.1.0"}"#);
    sandbox.write(
        "home/.cache/cockpit/update.json",
        r#"{"current": "0.1.0", "notes": [{"version": "0.2.0", "date": "2026-11-20", "changes": ["Quotas grouped by provider"]}]}"#,
    );
    assert_eq!(sandbox.cockpit(&["options", "pending"], &[]).status.code(), Some(0), "an update from 0.1.0 waits to be shown");
    let said = stdout(&sandbox.cockpit(&["options", "new"], &[]));
    assert!(said.contains("(you ran 0.1.0)") && said.contains("  - Quotas grouped by provider\n"), "{said}");
    assert!(said.contains("    cockpit options set chat.nextSteps on\n"), "{said}");
    assert_eq!(sandbox.cockpit(&["options", "pending"], &[]).status.code(), Some(1), "shown once");
    // No terminal on the input (a script, an agent's tools): nothing is set, and the file stays as written.
    let out = sandbox.cockpit(&["options", "set", "queue.drain", "off"], &[]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("an option is set by a person"), "{}", String::from_utf8_lossy(&out.stderr));
    let written = std::fs::read_to_string(sandbox.home().join(".config/cockpit/config.json")).unwrap();
    assert_eq!(written, "{\n  \"reposDir\": \"~/code\",\n  \"queue\": {\"drain\": true}\n}\n");
    let view: Value = serde_json::from_str(&stdout(&sandbox.cockpit(&["options", "--json"], &[]))).unwrap();
    let drain = view["options"].as_array().unwrap().iter().find(|o| o["key"] == "queue.drain").unwrap().clone();
    assert_eq!((drain["value"].as_bool(), drain["new"].as_bool()), (Some(true), Some(true)));
    assert_eq!(view["update"]["shown"], true);
    for bad in [&["options", "set", "queue.drain", "maybe"][..], &["options", "set", "made.up", "on"], &["options", "what"]] {
        assert_eq!(sandbox.cockpit(bad, &[]).status.code(), Some(1), "{bad:?}");
    }
    // A field or a number is a person's to set too, and a value its option does not take says why in its own words.
    let out = sandbox.cockpit(&["options", "set", "refresh.github", "45"], &[]);
    assert!(String::from_utf8_lossy(&out.stderr).contains("an option is set by a person"), "{}", String::from_utf8_lossy(&out.stderr));
    let out = sandbox.cockpit(&["options", "set", "refresh.github", "5"], &[]);
    assert!(String::from_utf8_lossy(&out.stderr).starts_with("GitHub takes 15 to 86400 s"), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(std::fs::read_to_string(sandbox.home().join(".config/cockpit/config.json")).unwrap(), written);
    // The list names every section, then what only the file holds.
    let listed = stdout(&sandbox.cockpit(&["options"], &[]));
    assert!(listed.contains("\nQueue and CI\n\n  Send queued prompts on their own: on (default on, since "), "{listed}");
    assert!(listed.contains("\nRefresh\n\n  GitHub: 30 s (default 30 s)\n") && listed.contains("\nIn config.json only\n\n  spaces: "), "{listed}");
}

#[test]
fn options_ask_changes_only_the_picked_options_and_tries_the_layout_on_yes() {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let _one = serial();
    let sandbox = Sandbox::new();
    sandbox.write("home/.config/cockpit/config.json", "{\"repo\":\"acme/api\"}\n");
    sandbox.write("home/.cache/cockpit/options.json", r#"{"ran":"0.1.0"}"#);
    sandbox.write("home/.config/herdr-app/layouts/cockpit-chat.json", "{}");
    for tool in ["herdr-app", "open", "xdg-open", "gh", "glab"] {
        executable(&sandbox.root.join("bin").join(tool), &common::fake(tool, &sandbox.log(), &[("*", "    printf '{}\\n'")]));
    }
    let mut child = Command::new(env!("CARGO_BIN_EXE_cockpit"))
        .args(["options", "ask"])
        .env_clear()
        .envs(sandbox.env())
        .env("HERDR_APP_HOME", sandbox.home().join(".config/herdr-app"))
        .env("HERDR_APP_BIN", sandbox.root.join("bin/herdr-app"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"n\n\ny\ny\n").unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "{}", common::stderr(&out));
    let said = stdout(&out);
    assert!(said.contains("Turn it on? (y/n)") && said.contains("Try Cockpit Chat in the Herdr desktop app?"), "{said}");
    assert!(said.contains("Opened Cockpit Chat"), "{said}");
    let config: Value = serde_json::from_str(&std::fs::read_to_string(sandbox.home().join(".config/cockpit/config.json")).unwrap()).unwrap();
    assert_eq!(config, serde_json::json!({"repo": "acme/api", "queue": {"drain": false}, "chat": {"nextSteps": true}}));
    assert_eq!(sandbox.calls(), [r#"herdr-app do layout.switch {"name":"cockpit-chat"}"#]);
}

// ---------------------------------------------------------------------------
// space

/// The sandbox's herdr/bin/cockpit: run as `space <app>` it is the real supervisor, any other way it logs its call.
fn space_binary(sandbox: &Sandbox) {
    let script = format!(
        "#!/bin/sh\n[ \"$1\" = space ] && exec '{}' \"$@\"\nprintf '%s\\n' \"$0 $* [$COCKPIT_SPACE]\" >> '{}'\n",
        env!("CARGO_BIN_EXE_cockpit"),
        sandbox.log().display()
    );
    common::executable(&sandbox.root.join("herdr/bin/cockpit"), &script);
}

fn supervisor(sandbox: &Sandbox, app: &str) -> std::process::Child {
    std::process::Command::new(sandbox.root.join("herdr/bin/cockpit"))
        .args(["space", app])
        .env_clear()
        .envs(sandbox.env())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap()
}

fn wait_for_calls(sandbox: &Sandbox, count: usize) -> Vec<String> {
    let start = Instant::now();
    loop {
        let calls = sandbox.calls();
        if calls.len() >= count {
            return calls;
        }
        assert!(start.elapsed() < Duration::from_secs(15), "{calls:?}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn space_starts_the_app_again_then_runs_from_a_new_binary() {
    let _one = serial();
    let sandbox = Sandbox::new();
    space_binary(&sandbox);
    let mut child = supervisor(&sandbox, "todo");
    let app = format!("{} todo cockpit-space [1]", sandbox.root.join("herdr/bin/cockpit").display());
    // The app ends at once: started again after a second, then two.
    assert_eq!(wait_for_calls(&sandbox, 2), [app.clone(), app.clone()]);
    // An update puts a new binary in: the supervisor runs from it.
    std::fs::remove_file(sandbox.root.join("herdr/bin/cockpit")).unwrap();
    common::executable(&sandbox.root.join("herdr/bin/cockpit"), &format!("#!/bin/sh\nprintf 'new %s\\n' \"$*\" >> '{}'\n", sandbox.log().display()));
    assert_eq!(wait_for_calls(&sandbox, 3).last().unwrap(), "new space todo");
    let status = child.wait().unwrap();
    assert!(status.success());
    let mut printed = String::new();
    std::io::Read::read_to_string(child.stdout.as_mut().unwrap(), &mut printed).unwrap();
    assert!(printed.starts_with("\x1b[?1003l\x1b[?1006l\x1b[?1000l"), "{printed:?}");
}

#[test]
fn space_leaves_on_a_ctrl_c_between_runs() {
    let _one = serial();
    let sandbox = Sandbox::new();
    space_binary(&sandbox);
    let mut child = supervisor(&sandbox, "inbox");
    let calls = wait_for_calls(&sandbox, 1);
    assert_eq!(calls, [format!("{} inbox cockpit-space []", sandbox.root.join("herdr/bin/cockpit").display())]);
    std::thread::sleep(Duration::from_millis(300));
    // SAFETY: kill only sends SIGINT to the child this test started.
    unsafe { libc::kill(child.id() as i32, libc::SIGINT) };
    let start = Instant::now();
    while child.try_wait().unwrap().is_none() {
        assert!(start.elapsed() < Duration::from_secs(5), "the supervisor kept going");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(child.wait().unwrap().success());
    assert_eq!(sandbox.calls().len(), 1);
}

#[test]
fn space_wants_one_of_the_space_apps() {
    let _one = serial();
    let sandbox = Sandbox::new();
    for args in [&["space"][..], &["space", "show"], &["space", "inbox", "x"]] {
        assert_eq!(sandbox.cockpit(args, &[]).status.code(), Some(1), "{args:?}");
    }
}

// ---------------------------------------------------------------------------
// doctor

#[test]
fn doctor_prints_a_line_per_check() {
    let _one = serial();
    let sandbox = Sandbox::new();
    let spaces = r#"{"result": {"workspaces": [{"workspace_id": "w1", "label": "[1] x  inbox"}, {"workspace_id": "w2", "label": "[2] eng-4"}]}}"#;
    let panes = r#"{"result": {"panes": [{"pane_id": "p1", "workspace_id": "w2", "agent": "claude"}]}}"#;
    sandbox.herdr(&[("--version", "herdr 0.8.1"), ("workspace list", spaces), ("pane list", panes)]);
    sandbox.write(
        "home/.config/cockpit/config.json",
        r#"{"spaces": ["inbox", {"name": "notes"}], "tickets": {"enabled": true, "tools": {"listMine": {"tool": "x"}}}, "ci": {"enabled": false},
            "platform": {"enabled": true, "server": "platform", "tools": {"stop": {"tool": "s"}, "deploy": {"tool": "d"}, "preview": {"tool": ""}}}, "metabase": {"server": "metabase"}}"#,
    );
    sandbox.write("home/.config/herdr/config.toml", "[ui.toast]\ndelivery = \"herdr\"\n");
    sandbox.write("home/.claude/settings.json", r#"{"permissions": {"allow": ["mcp__platform", "mcp__metabase"]}}"#);
    sandbox.write("home/.cache/cockpit/inbox.json", r#"{"at": 1000, "errors": ["GitHub: 401"]}"#);
    sandbox.write("home/.cache/cockpit/sessions/a.json", r#"{"pane": "p1", "version": "0.1.0", "at": 5}"#);
    let out = sandbox.cockpit(&["doctor", "--all"], &[]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let printed = stdout(&out);
    let link = sandbox.home().join(".config/cockpit/herdr");
    let lines: Vec<&str> = printed.lines().collect();
    for line in [
        "fail  herdr 0.8.1: cockpit needs 0.9 or later → herdr update".to_string(),
        "ok    config.json".into(),
        "fail  herdr plugin not installed → herdr plugin install quaresc/cockpit/herdr --yes".into(),
        "ok    session \"eng-4\" on cockpit 0.1.0".into(),
        "fail  inbox: GitHub: 401".into(),
        "warn  space inbox is hidden in the sidebar → add { contains = \"  \" } before the hiding rule of ui.sidebar.spaces in ~/.config/herdr/config.toml"
            .into(),
        format!("fail  space notes missing → sh {}/ensure-spaces.sh", link.display()),
        "ok    herdr toasts in-app".into(),
        "ok    platform tools mapped: stop, deploy".into(),
        "warn  ticket tools not mapped: get, listStates, setState; cockpit skips what they do → /cockpit:setup or cockpit connector map maps them".into(),
        "ok    CI from GitHub checks".into(),
        "ok    allow rule for platform".into(),
        "fail  no allow rule for claude.ai Linear: background refreshes are refused → add \"mcp__claude_ai_Linear\" to permissions.allow".into(),
        "ok    allow rule for mcp__metabase__execute_query".into(),
    ] {
        assert!(lines.contains(&line.as_str()), "{line}\n{printed}");
    }
    assert!(!printed.contains("CI:"), "{printed}");
    assert!(!printed.contains("Datadog"), "no Datadog rule asked of a config without Datadog: {printed}");
    // Whether a supervisor runs is this machine's business; the line is there either way.
    let no_app = format!("fail  space inbox has no app running → sh {}/ensure-spaces.sh", link.display());
    assert!(lines.iter().any(|l| *l == no_app || *l == "ok    space inbox, app running"), "{printed}");
    // Each line is one check, in its order.
    let at = |prefix: &str| lines.iter().position(|l| l.contains(prefix)).unwrap();
    assert!(at("herdr 0.8.1") < at("config.json") && at("config.json") < at("session") && at("session") < at("inbox:") && at("inbox:") < at("platform tools"));
}

/// The Claude plugin is looked up with the Claude Code the agents config starts, not whichever `claude` is on PATH.
#[test]
fn doctor_asks_the_configured_claude_for_its_plugins() {
    let _one = serial();
    let sandbox = Sandbox::new();
    sandbox.herdr(&[("--version", "herdr 0.9.1")]);
    let listing = "    printf '%s\\n' 'Installed plugins:' '' '  ❯ cockpit@cockpit' '    Version: 1.0.1' '    Scope: user'";
    // The `claude` on PATH reads another profile, without the plugin; the configured wrapper reads the one with it.
    executable(&sandbox.root.join("bin/claude"), &common::fake("claude", &sandbox.log(), &[("plugin list", "    printf 'Installed plugins:\\n'")]));
    executable(&sandbox.root.join("bin/claude-work"), &common::fake("claude-work", &sandbox.log(), &[("plugin list", listing)]));
    let wrapper = sandbox.root.join("bin/claude-work");
    sandbox.write(
        "home/.config/cockpit/config.json",
        &format!(r#"{{"agents": {{"default": "claude", "commands": {{"claude": {{"bin": "{}"}}}}}}}}"#, wrapper.display()),
    );
    let report: Value = serde_json::from_str(&stdout(&sandbox.cockpit(&["doctor", "--json"], &[]))).unwrap();
    let check = report["checks"].as_array().unwrap().iter().find(|c| c["id"] == "claude.plugin").cloned().unwrap_or_default();
    assert_eq!(check["status"], "pass", "{report}");
    assert_eq!(check["message"], "Claude plugin cockpit 1.0.1", "{report}");
    assert!(sandbox.calls().iter().any(|c| c == "claude-work plugin list"), "{:?}", sandbox.calls());
    assert!(!sandbox.calls().iter().any(|c| c == "claude plugin list"), "{:?}", sandbox.calls());
}

/// Every check of one report has an id of its own: a fix names its check by it.
#[test]
fn doctor_json_gives_no_two_checks_one_id() {
    let _one = serial();
    let sandbox = Sandbox::new();
    let spaces = r#"{"result": {"workspaces": [{"workspace_id": "w1", "label": "[1] x  inbox"}, {"workspace_id": "w2", "label": "[2] eng-4"}]}}"#;
    let panes =
        r#"{"result": {"panes": [{"pane_id": "p1", "workspace_id": "w2", "agent": "claude"}, {"pane_id": "p2", "workspace_id": "w2", "agent": "claude"}]}}"#;
    sandbox.herdr(&[("--version", "herdr 0.9.1"), ("workspace list", spaces), ("pane list", panes)]);
    sandbox.write(
        "home/.config/cockpit/config.json",
        r#"{"spaces": ["inbox", "notes"], "tickets": {"enabled": true, "server": "Linear", "tools": {"listMine": {"tool": "x"}}},
            "ci": {"enabled": true, "server": "Linear"}, "platform": {"enabled": true, "server": "Linear"}, "calendar": {"enabled": true, "server": "Linear"},
            "mail": {"enabled": true, "server": "Linear", "tools": {"listInbox": {"tool": "l"}, "get": {"tool": "g"}, "archive": {"tool": "l"}}}}"#,
    );
    sandbox.write("home/.claude/settings.json", r#"{"permissions": {"allow": []}}"#);
    sandbox.write("home/.cache/cockpit/sessions/a.json", r#"{"pane": "p1", "version": "0.1.0", "at": 5}"#);
    let report: Value = serde_json::from_str(&stdout(&sandbox.cockpit(&["doctor", "--json"], &[]))).unwrap();
    let mut ids: Vec<&str> = report["checks"].as_array().unwrap().iter().map(|c| c["id"].as_str().unwrap()).collect();
    let all = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), all, "{report}");
}

/// A mapping with no tool for the states a ticket moves to lists them itself (GitHub Issues').
#[test]
fn doctor_takes_a_mappings_own_states_for_the_tool_listing_them() {
    let _one = serial();
    let sandbox = Sandbox::new();
    sandbox.write(
        "home/.config/cockpit/config.json",
        r#"{"tickets": {"enabled": true, "states": [{"id": "open", "name": "Open", "type": "unstarted"}],
            "tools": {"listMine": {"tool": "a"}, "get": {"tool": "b"}, "setState": {"tool": "c"}}}}"#,
    );
    let printed = stdout(&sandbox.cockpit(&["doctor", "--all"], &[]));
    assert!(printed.contains("ok    ticket tools mapped"), "{printed}");
}

/// The `cockpit` on PATH left on an install folder that an update pruned.
#[test]
fn doctor_catches_a_cockpit_on_path_that_leads_nowhere() {
    let _one = serial();
    let sandbox = Sandbox::new();
    sandbox.write("home/.config/cockpit/config.json", "{}");
    let bin = sandbox.home().join(".local/bin/cockpit");
    std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(sandbox.home().join("gone/bin/cockpit"), &bin).unwrap();
    let printed = stdout(&sandbox.cockpit(&["doctor", "--all"], &[]));
    assert!(printed.contains("fail  ~/.local/bin/cockpit points at a folder that is gone"), "{printed}");
}

/// Without --all, only what needs doing is printed, and the last line says how much passed.
#[test]
fn doctor_holds_back_the_checks_that_passed() {
    let _one = serial();
    let sandbox = Sandbox::new();
    sandbox.write("home/.config/cockpit/config.json", "{}");
    let short = stdout(&sandbox.cockpit(&["doctor"], &[]));
    let all = stdout(&sandbox.cockpit(&["doctor", "--all"], &[]));
    assert!(!short.contains("\nok    "), "no passing check is listed:\n{short}");
    assert!(all.contains("ok    "), "--all lists them:\n{all}");
    assert!(short.lines().count() < all.lines().count(), "the short report is the shorter one");
    let last = short.trim_end().lines().next_back().unwrap_or_default();
    assert!(last.contains("failure") && last.contains("warning") && last.contains("--all"), "a verdict closes it: {last}");
}

/// `--json`: every check with its id, category and fix, the steps of getting started they make, the agents, and the
/// report kept for `--cached`.
#[test]
fn doctor_json_gives_each_check_an_id_its_fix_and_the_steps_of_getting_started() {
    let _one = serial();
    let sandbox = Sandbox::new();
    sandbox.write("home/.config/cockpit/config.json", r#"{"agents": {"default": "nope"}, "tickets": {"enabled": true}, "spaces": ["inbox"]}"#);
    let out = sandbox.cockpit(&["doctor", "--json"], &[]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let report: Value = serde_json::from_str(&stdout(&out)).expect("one JSON document and nothing else");
    assert_eq!((&report["version"], &report["cockpit"]), (&json!(1), &json!(env!("CARGO_PKG_VERSION"))));
    let checks = report["checks"].as_array().unwrap();
    let check = |id: &str| checks.iter().find(|c| c["id"] == id).cloned().unwrap_or_else(|| panic!("no check {id}: {report}"));
    assert_eq!(check("tools.config")["status"], "pass");
    let gh = check("tools.gh");
    assert_eq!((&gh["category"], &gh["status"], &gh["message"]), (&json!("tools"), &json!("fail"), &json!("gh not signed in")));
    assert_eq!(gh["fix"], json!({"label": "gh auth login", "command": "gh auth login"}));
    assert_eq!(check("spaces.inbox")["fix"]["command"], format!("sh {}/ensure-spaces.sh", sandbox.home().join(".config/cockpit/herdr").display()));
    assert!(checks.iter().all(|c| ["pass", "warn", "fail"].contains(&c["status"].as_str().unwrap())), "{report}");
    let counted: usize = ["pass", "warn", "fail"].iter().map(|s| report["summary"][s].as_u64().unwrap() as usize).sum();
    assert_eq!(counted, checks.len());
    let steps: Vec<(&str, bool)> = report["getting_started"].as_array().unwrap().iter().map(|s| (s["id"].as_str().unwrap(), s["done"] == true)).collect();
    assert_eq!(steps[..2], [("setup", true), ("agent", false)], "{steps:?}");
    assert!(steps.contains(&("github", false)) && steps.contains(&("tickets", false)), "{steps:?}");
    let codex = report["agents"].as_array().unwrap().iter().find(|a| a["id"] == "codex").cloned().unwrap();
    assert_eq!((&codex["can"]["resume"], &codex["can"]["driven"], &codex["tools"]), (&json!(true), &json!(false), &json!("mcp")));
    let agent = |id: &str| report["agents"].as_array().unwrap().iter().find(|a| a["id"] == id).cloned().unwrap();
    assert_eq!((&codex["limits"], &agent("claude")["can"]["driven"]), (&json!("Codex's session log"), &json!(true)), "from the table, not the plugin");
    assert_eq!(agent("devin")["limits"], Value::Null, "an agent with no source reports none");
    assert!(!stdout(&out).contains("fail  "), "no text line beside the JSON");
    // The report is kept, and --cached answers with it instead of checking again.
    let kept: Value = serde_json::from_str(&std::fs::read_to_string(sandbox.cache().join("doctor.json")).unwrap()).unwrap();
    assert_eq!(kept["at"], report["at"]);
    let again: Value = serde_json::from_str(&stdout(&sandbox.cockpit(&["doctor", "--json", "--cached"], &[]))).unwrap();
    assert_eq!(again["at"], report["at"]);
}

#[test]
fn doctor_tells_the_hub_fetches_the_calendar_without_printing_its_address() {
    let _one = serial();
    let sandbox = Sandbox::new();
    let config = |enabled: bool| json!({"calendar": {"enabled": enabled, "ics": "https://calendar.acme.example/ical/private-s3cret/basic.ics"}}).to_string();
    sandbox.write("home/.config/cockpit/config.json", &config(true));
    sandbox.write("home/.cache/cockpit/meetings.json", r#"{"at": 1000, "meetings": [], "notified": [], "by": "hub"}"#);
    let printed = stdout(&sandbox.cockpit(&["doctor", "--all"], &[]));
    let lines: Vec<&str> = printed.lines().collect();
    assert!(lines.contains(&"ok    calendar fetched by the hub from its secret iCal address"), "{printed}");
    let stale = |l: &&str| l.starts_with("warn  meetings: last refresh") && l.contains("→ neither a cockpit app (its hub) nor");
    assert!(lines.iter().any(stale), "{printed}");
    assert!(!printed.contains("s3cret"), "{printed}");
    sandbox.write("home/.config/cockpit/config.json", &config(false));
    let printed = stdout(&sandbox.cockpit(&["doctor", "--all"], &[]));
    assert!(printed.contains("warn  calendar.ics is set but the calendar is off"), "{printed}");
}

#[test]
fn doctor_leaves_the_pinned_spaces_alone_while_herdr_is_not_running() {
    // herdr answers on stderr and exits 1: its own error when no server runs, a wire error on the Herdr app's socket.
    let down = r#"{"id":"cli:workspace:list","error":{"code":"server_not_running","message":"no herdr server is running"}}"#;
    let wire = "Error: Custom { kind: Other, error: Json(Error(\"data did not match any variant of untagged enum WireResponse\")) }";
    for answer in [down, wire] {
        let sandbox = Sandbox::new();
        let failing = format!("{} >&2\n    exit 1", common::says(answer));
        let version = common::says("herdr 0.9.5");
        executable(&sandbox.root.join("bin/herdr"), &common::fake("herdr", &sandbox.log(), &[("--version", &version), ("*", &failing)]));
        sandbox.write("home/.config/cockpit/config.json", r#"{"spaces": ["inbox"]}"#);
        let printed = stdout(&sandbox.cockpit(&["doctor", "--all"], &[]));
        assert!(printed.lines().any(|l| l == "ok    herdr is not running: its pinned spaces are checked while it runs"), "{printed}");
        assert!(!printed.contains("space inbox"), "{printed}");
    }
}
