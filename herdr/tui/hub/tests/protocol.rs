//! The socket protocol, the hub started on a sandbox HOME: one instance, a stale socket taken over, the idle exit, and
//! each op's answer.

mod common;

use std::time::{Duration, Instant};

use common::{Hub, Sandbox, request_on};
use serde_json::{Value, json};

fn sandbox() -> Sandbox {
    Sandbox::new(&json!({}), &json!({}))
}

fn calls(sandbox: &Sandbox) -> Vec<Value> {
    std::fs::read_to_string(sandbox.root.join("calls.log")).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}

#[test]
fn ping() {
    let sandbox = sandbox();
    let hub = Hub::start(&sandbox, &[]);
    assert_eq!(hub.request(&json!({"op": "ping"})), json!({"ok": true}));
}

#[test]
fn several_requests_on_one_connection_and_bad_ones() {
    use std::io::{BufRead, BufReader, Write};
    let sandbox = sandbox();
    let _hub = Hub::start(&sandbox, &[]);
    let mut stream = std::os::unix::net::UnixStream::connect(sandbox.socket()).unwrap();
    writeln!(stream, "{{\"op\": \"ping\"}}\nnot json\n{{\"op\": \"dance\"}}").unwrap();
    let answers: Vec<Value> = BufReader::new(stream).lines().take(3).map(|l| serde_json::from_str(&l.unwrap()).unwrap()).collect();
    assert_eq!(answers[0], json!({"ok": true}));
    assert!(answers[1]["error"].as_str().unwrap().starts_with("not JSON"));
    assert_eq!(answers[2], json!({"error": "unknown op \"dance\""}));
}

#[test]
fn one_hub_at_a_time() {
    let sandbox = sandbox();
    let hub = Hub::start(&sandbox, &[]);
    let mut second = std::process::Command::new(env!("CARGO_BIN_EXE_cockpit-hub")).env_clear().envs(sandbox.env()).spawn().unwrap();
    let status = second.wait().unwrap();
    assert!(status.success());
    assert_eq!(hub.request(&json!({"op": "ping"})), json!({"ok": true}));
}

#[test]
fn a_stale_socket_is_taken_over() {
    let sandbox = sandbox();
    let socket = sandbox.socket();
    std::fs::create_dir_all(socket.parent().unwrap()).unwrap();
    drop(std::os::unix::net::UnixListener::bind(&socket).unwrap());
    assert!(socket.exists());
    let hub = Hub::start(&sandbox, &[]);
    assert_eq!(hub.request(&json!({"op": "ping"})), json!({"ok": true}));
}

#[test]
fn exits_when_idle() {
    let sandbox = sandbox();
    let mut hub = Hub::start(&sandbox, &[("COCKPIT_HUB_IDLE_SECS", "2")]);
    let start = Instant::now();
    while hub.is_running() && start.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(!hub.is_running(), "still running after 10 s idle");
    assert!(!sandbox.socket().exists());
}

#[test]
fn search_across_apps() {
    let sandbox = sandbox();
    let hub = Hub::start(&sandbox, &[]);
    let found = hub.request(&json!({"op": "search", "query": "migration"}));
    let first = &found["entries"][0];
    assert_eq!((first["app"].as_str(), first["kind"].as_str()), (Some("runbook"), Some("step")));
    assert_eq!(first["target"], json!({"app": "runbook", "key": "eng-401#2"}));
    let found = hub.request(&json!({"op": "search", "query": "dark mode"}));
    assert_eq!(found["entries"][0]["target"], json!({"app": "inbox", "key": "https://github.com/acme/web/pull/9"}));
    let found = hub.request(&json!({"op": "search", "query": "bank"}));
    assert_eq!(found["entries"][0]["target"], json!({"app": "todo", "key": "call the bank about ENG-401"}));
    let found = hub.request(&json!({"op": "search", "query": "team daily"}));
    assert_eq!(found["entries"][0]["target"], json!({"app": "meetings", "key": "ev1_20261028T084500Z"}));
    let found = hub.request(&json!({"op": "search", "query": "blocked previews"}));
    assert_eq!(found["entries"][0]["target"], json!({"app": "show", "key": "demo#0"}));
}

#[test]
fn search_follows_file_changes() {
    let sandbox = sandbox();
    let hub = Hub::start(&sandbox, &[]);
    assert!(hub.request(&json!({"op": "search", "query": "quarterly taxes"}))["entries"].as_array().unwrap().is_empty());
    std::fs::write(sandbox.home().join(".config/cockpit/todo.md"), "# To-do\n\n- [ ] pay the quarterly taxes\n").unwrap();
    let start = Instant::now();
    loop {
        let found = hub.request(&json!({"op": "search", "query": "quarterly taxes"}));
        if found["entries"].as_array().is_some_and(|e| !e.is_empty()) {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(5), "the index never saw the new item");
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn go_focuses_the_space_and_asks_the_app() {
    let sandbox = sandbox();
    let hub = Hub::start(&sandbox, &[]);
    let target = json!({"app": "runbook", "key": "eng-401#2"});
    assert_eq!(hub.request(&json!({"op": "go", "target": target})), json!({"ok": true}));
    let asked: Value = serde_json::from_str(&std::fs::read_to_string(sandbox.home().join(".cache/cockpit/focus/runbook.json")).unwrap()).unwrap();
    assert_eq!(asked, target);
    let herdr: Vec<Value> = calls(&sandbox).into_iter().map(|c| c["args"].clone()).collect();
    assert_eq!(herdr, [json!(["workspace", "list"]), json!(["workspace", "focus", "w5"])]);
}

#[test]
fn go_to_a_show_view_has_no_space() {
    let sandbox = sandbox();
    let hub = Hub::start(&sandbox, &[]);
    hub.request(&json!({"op": "go", "target": {"app": "show", "key": "demo#0"}}));
    assert!(calls(&sandbox).is_empty());
    assert!(sandbox.home().join(".cache/cockpit/focus/show.json").exists());
}

#[test]
fn fetch_failed_tests_from_ci_json() {
    let sandbox = sandbox();
    let hub = Hub::start(&sandbox, &[]);
    let tests = hub.request(&json!({"op": "fetch", "kind": "failed_tests", "args": {"repo": "api", "branch": "me/eng-401-login"}}));
    assert_eq!(tests["tests"], json!(["LoginTest", "GoogleTest"]));
    assert_eq!(tests["job"], "test");
}

#[test]
fn fetch_ci_log_of_another_ci_needs_a_session() {
    let sandbox = sandbox();
    let hub = Hub::start(&sandbox, &[]);
    let log = hub.request(&json!({"op": "fetch", "kind": "ci_log", "args": {"repo": "api", "branch": "me/eng-401-login"}}));
    assert_eq!(log["message"], "The CircleCI logs need a Claude session with its connector: fix ci opens one.");
}

/// gh answering canned JSON, and logging its calls like the other fakes.
fn fake_gh(sandbox: &Sandbox) {
    use std::os::unix::fs::PermissionsExt;
    let script = r#"#!/bin/sh
echo "$*" >> "$COCKPIT_TEST_DIR/gh.log"
case "$1 $2" in
  "api graphql") echo '{"data":{"repository":{"pullRequest":{"reviewThreads":{"nodes":[
     {"isResolved":false,"isOutdated":false,"path":"lib/a.ex","line":12,"originalLine":10,"comments":{"nodes":[{"author":{"login":"maya"},"body":"Why?"},{"author":{"login":"me"},"body":"Because."}]}},
     {"isResolved":true,"path":"lib/b.ex","line":1,"comments":{"nodes":[]}}]}}}}}' ;;
  "pr view") echo '{"files":[{"path":"lib/a.ex","additions":3,"deletions":1}]}' ;;
  "run list") echo '[{"databaseId":42}]' ;;
  "run view") seq 1 250 | sed 's/^/line /' ;;
esac
"#;
    let path = sandbox.root.join("bin/gh");
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn fetch_review_threads_and_files_through_gh_kept_a_while() {
    let sandbox = sandbox();
    fake_gh(&sandbox);
    let hub = Hub::start(&sandbox, &[]);
    let args = json!({"repo": "api", "number": 4});
    let threads = hub.request(&json!({"op": "fetch", "kind": "review_threads", "args": args}));
    assert_eq!(threads["threads"].as_array().unwrap().len(), 1);
    let thread = &threads["threads"][0];
    assert_eq!(
        (thread["path"].as_str(), thread["line"].as_i64(), thread["author"].as_str(), thread["body"].as_str()),
        (Some("lib/a.ex"), Some(12), Some("maya"), Some("Why?"))
    );
    assert_eq!(thread["comments"].as_array().unwrap().len(), 2);
    hub.request(&json!({"op": "fetch", "kind": "review_threads", "args": args}));
    let files = hub.request(&json!({"op": "fetch", "kind": "pr_files", "args": args}));
    assert_eq!(files["files"][0]["path"], "lib/a.ex");
    let log = std::fs::read_to_string(sandbox.root.join("gh.log")).unwrap();
    assert_eq!(log.lines().filter(|l| l.starts_with("api graphql")).count(), 1, "the second ask came from the cache");
    assert!(log.contains("owner=acme") && log.contains("name=api") && log.contains("number=4"));
    assert!(log.contains("pr view 4 --repo acme/api --json files"));
}

#[test]
fn fetch_ci_log_from_github_actions() {
    let sandbox = Sandbox::new(&json!({}), &json!({"ci": {"enabled": true, "name": "GitHub", "server": ""}}));
    fake_gh(&sandbox);
    let hub = Hub::start(&sandbox, &[]);
    let log = hub.request(&json!({"op": "fetch", "kind": "ci_log", "args": {"repo": "api", "branch": "me/eng-401-login"}}));
    let lines = log["lines"].as_array().unwrap();
    assert_eq!((lines.len(), lines[0].as_str(), lines[199].as_str()), (200, Some("line 51"), Some("line 250")));
    assert_eq!(log["run"], 42);
}

#[test]
fn fetch_errors_reach_the_app() {
    let sandbox = Sandbox::new(&json!({}), &json!({"github": {"enabled": true, "org": ""}}));
    let hub = Hub::start(&sandbox, &[]);
    let answer = hub.request(&json!({"op": "fetch", "kind": "pr_files", "args": {"repo": "api", "number": 4}}));
    assert_eq!(answer, json!({"error": "set github.org in the cockpit config"}));
}

#[test]
fn history_of_ticket_costs() {
    let sandbox = sandbox();
    let hub = Hub::start(&sandbox, &[]);
    let values = |hub: &Hub| hub.request(&json!({"op": "history", "kind": "ticket_cost", "key": "ENG-401"}))["values"].clone();
    let start = Instant::now();
    while values(&hub) != json!([8.75]) {
        assert!(start.elapsed() < Duration::from_secs(5), "no first point: {}", values(&hub));
        std::thread::sleep(Duration::from_millis(100));
    }
    std::fs::write(sandbox.home().join(".cache/cockpit/costs/s-new.json"), r#"{"ENG-401": 1.25}"#).unwrap();
    while values(&hub) != json!([8.75, 10.0]) {
        assert!(start.elapsed() < Duration::from_secs(10), "no second point: {}", values(&hub));
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn act_through_the_socket_like_core_bridge() {
    let sandbox = sandbox();
    let hub = Hub::start(&sandbox, &[]);
    let reply = request_on(&sandbox.socket(), &json!({"op": "act", "app": "inbox", "action": "set_muted", "args": ["docs", true], "armed": null}));
    assert_eq!(reply, json!({"note": "muted docs", "pending": {}, "refreshing": {}, "error": null}));
    drop(hub);
}

#[test]
fn meetings_join_and_brief_toggle() {
    let sandbox = sandbox();
    let hub = Hub::start(&sandbox, &[]);
    let meeting = json!({"id": "ev1_20261028T084500Z", "title": "Team daily", "join": "https://meet/abc"});
    let joined = hub.request(&json!({"op": "act", "app": "meetings", "action": "join", "args": [meeting], "armed": null}));
    assert_eq!(joined["note"], "opened https://meet/abc");
    let toggled = hub.request(&json!({"op": "act", "app": "meetings", "action": "toggle_brief", "args": [meeting], "armed": null}));
    assert_eq!(toggled["note"], "preparing the brief of Team daily, about a minute");
    let pending = json!({"id": "ev2/odd id", "title": "Planning"});
    let toggled = hub.request(&json!({"op": "act", "app": "meetings", "action": "toggle_brief", "args": [pending], "armed": null}));
    assert_eq!(toggled["note"], "");
}
