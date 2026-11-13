//! The socket protocol, the hub started on a sandbox HOME: one instance, a stale socket taken over, the idle exit, each
//! op's answer, and the shared documents written, pushed and leased.

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
fn tcp_on_an_explicit_interface_authenticates_and_streams_inbox_updates() {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpStream;

    let sandbox = sandbox();
    let hub = Hub::start(&sandbox, &[("COCKPIT_HUB_LISTEN", "::1")]);
    let started = Instant::now();
    let info: Value = loop {
        if let Ok(text) = std::fs::read_to_string(sandbox.home().join(".cache/cockpit/hub.json")) {
            break serde_json::from_str(&text).unwrap();
        }
        assert!(started.elapsed() < Duration::from_secs(5), "no TCP discovery file");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(info["host"], "::1");
    let connect = || {
        let stream = TcpStream::connect(("::1", info["port"].as_u64().unwrap() as u16)).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        BufReader::new(stream)
    };
    let read = |stream: &mut BufReader<TcpStream>| {
        let mut line = String::new();
        stream.read_line(&mut line).unwrap();
        serde_json::from_str::<Value>(&line).unwrap()
    };
    let mut denied = connect();
    writeln!(denied.get_mut(), "{}", json!({"op": "ping", "token": "wrong"})).unwrap();
    assert!(read(&mut denied)["error"].is_string());
    let mut stream = connect();
    writeln!(stream.get_mut(), "{}", json!({"op": "ping", "token": info["token"]})).unwrap();
    assert_eq!(read(&mut stream), json!({"ok": true}));
    writeln!(stream.get_mut(), "{}", json!({"op": "subscribe", "docs": ["inbox"], "token": info["token"]})).unwrap();
    assert_eq!(read(&mut stream)["doc"], "inbox");
    hub.request(&json!({"op": "store.put", "name": "inbox", "value": {"tickets": [{"id": "ENG-401"}]}}));
    assert_eq!(read(&mut stream)["value"]["tickets"][0]["id"], "ENG-401");
    // ::1 is this machine, so the connection still writes; only a hub listening further out reads only.
    let mut writes = connect();
    writeln!(writes.get_mut(), "{}", json!({"op": "store.put", "name": "queue", "value": {"items": ["one"]}, "token": info["token"]})).unwrap();
    assert_eq!(read(&mut writes)["value"]["items"][0], "one");
}

#[test]
fn a_listen_address_that_cannot_be_bound_leaves_no_hub_behind() {
    let sandbox = sandbox();
    // TEST-NET-1: routable nowhere and assigned to no interface.
    let mut child = common::hub_command(&sandbox).envs([("COCKPIT_HUB_LISTEN", "192.0.2.1")]).spawn().unwrap();
    assert!(child.wait().unwrap().success(), "the hub says why on stderr rather than failing");
    assert!(!sandbox.socket().exists(), "a hub that cannot listen must not take the socket a working one holds");
    assert!(!sandbox.home().join(".cache/cockpit/hub.json").exists());
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
    let mut second = common::hub_command(&sandbox).spawn().unwrap();
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
        assert!(start.elapsed() < Duration::from_secs(10), "the index never saw the new item");
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
    assert_eq!(log["message"], "The CircleCI logs need an agent session with its connector: fix ci opens one.");
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
    // Waits on a background scan, so the budget is wide: the whole file's hubs share this machine.
    let start = Instant::now();
    while values(&hub) != json!([8.75]) {
        assert!(start.elapsed() < Duration::from_secs(30), "no first point: {}", values(&hub));
        std::thread::sleep(Duration::from_millis(100));
    }
    std::fs::write(sandbox.home().join(".cache/cockpit/costs/s-new.json"), r#"{"ENG-401": 1.25}"#).unwrap();
    while values(&hub) != json!([8.75, 10.0]) {
        assert!(start.elapsed() < Duration::from_secs(60), "no second point: {}", values(&hub));
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

#[test]
fn the_documents_are_written_mirrored_and_pushed() {
    use std::io::{BufRead, BufReader, Write};
    let sandbox = sandbox();
    let hub = Hub::start(&sandbox, &[]);
    let cache = sandbox.home().join(".cache/cockpit");
    let mut stream = std::os::unix::net::UnixStream::connect(sandbox.socket()).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    writeln!(stream, "{}", json!({"op": "subscribe", "docs": ["ci", "nope"]})).unwrap();
    let mut pushed = BufReader::new(stream).lines().map(|l| serde_json::from_str::<Value>(&l.unwrap()).unwrap());
    let boxed: Value = serde_json::from_str(&std::fs::read_to_string(cache.join("ci.json")).unwrap()).unwrap();
    assert_eq!(pushed.next().unwrap(), json!({"doc": "ci", "value": boxed}), "the document at once, an unknown one left out");

    let put = hub.request(&json!({"op": "store.put", "name": "ci", "value": {"at": 1, "runs": []}}));
    assert_eq!(put, json!({"value": {"at": 1, "runs": []}}));
    assert_eq!(pushed.next().unwrap(), json!({"doc": "ci", "value": {"at": 1, "runs": []}}));
    assert_eq!(serde_json::from_str::<Value>(&std::fs::read_to_string(cache.join("ci.json")).unwrap()).unwrap(), put["value"]);

    // The mod's part of the inbox keeps the fields the box's inbox.json holds, and puts its errors before the hub's.
    let patched = hub.request(&json!({"op": "store.patch", "name": "inbox", "fields": {"version": 1, "previews": [], "errors": ["Previews failed"]}}));
    assert_eq!(patched["value"]["previews"], json!([]));
    assert!(patched["value"]["tickets"].as_array().is_some_and(|t| !t.is_empty()), "{patched}");
    assert_eq!(patched["value"]["errors"], json!(["Previews failed"]));
    assert_eq!(hub.request(&json!({"op": "store.get", "name": "inbox"})), patched);
    assert_eq!(hub.request(&json!({"op": "store.past", "name": "inbox", "since": 0}))["versions"].as_array().unwrap().len(), 1);
    assert!(hub.request(&json!({"op": "store.put", "name": "../config", "value": {}}))["error"].as_str().unwrap().starts_with("no document"));
    assert!(!cache.join("../config.json").exists());

    let lease =
        |holder: &str, renew: bool| hub.request(&json!({"op": "store.lease", "name": "inbox", "holder": holder, "ttl": 60, "renew": renew}))["granted"].clone();
    assert_eq!((lease("a", false), lease("b", false), lease("a", true)), (json!(true), json!(false), json!(true)));
    hub.request(&json!({"op": "store.release", "name": "inbox", "holder": "a"}));
    assert_eq!((lease("a", true), lease("b", false)), (json!(false), json!(true)));
}

/// A subscription to the ci document, its first line read: the hub has it among its subscribers.
fn subscribed(sandbox: &Sandbox) -> std::os::unix::net::UnixStream {
    use std::io::{BufRead, Write};
    let mut stream = std::os::unix::net::UnixStream::connect(sandbox.socket()).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    writeln!(stream, "{}", json!({"op": "subscribe", "docs": ["ci"]})).unwrap();
    let mut first = String::new();
    std::io::BufReader::new(&stream).read_line(&mut first).unwrap();
    assert!(first.starts_with(r#"{"doc":"ci""#), "{first}");
    stream
}

/// Reads what the hub sent until it closes the connection; false when it is still open after `quiet` with nothing new.
fn closed_by_the_hub(mut stream: std::os::unix::net::UnixStream, quiet: Duration) -> bool {
    use std::io::Read;
    stream.set_read_timeout(Some(quiet)).unwrap();
    let mut buf = vec![0; 1 << 16];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => return true,
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => return true,
            Err(_) => return false,
        }
    }
}

#[test]
fn a_subscriber_that_falls_far_behind_is_cut_off() {
    let sandbox = sandbox();
    let hub = Hub::start(&sandbox, &[]);
    let stream = subscribed(&sandbox);
    let blob = "x".repeat(1 << 20);
    for n in 0..24 {
        hub.request(&json!({"op": "store.put", "name": "ci", "value": {"n": n, "blob": blob}}));
    }
    assert!(closed_by_the_hub(stream, Duration::from_secs(5)), "24 MiB of versions waited for a subscriber that read none of them");
}

#[test]
fn a_subscriber_that_stops_reading_is_let_go() {
    let sandbox = sandbox();
    let hub = Hub::start(&sandbox, &[("COCKPIT_HUB_WRITE_SECS", "1")]);
    let stream = subscribed(&sandbox);
    let blob = "x".repeat(1 << 20);
    for n in 0..2 {
        hub.request(&json!({"op": "store.put", "name": "ci", "value": {"n": n, "blob": blob}}));
    }
    std::thread::sleep(Duration::from_secs(3));
    assert!(closed_by_the_hub(stream, Duration::from_secs(5)), "a write blocked past COCKPIT_HUB_WRITE_SECS keeps the connection");
}

/// The hub on ::1 with its TCP side up (and these settings), and a subscription to the ci document there, its first
/// line read.
fn tcp_subscribed(sandbox: &Sandbox, extra: &[(&str, &str)]) -> (Hub, std::net::TcpStream) {
    use std::io::{BufRead, Write};
    let hub = Hub::start(sandbox, &[&[("COCKPIT_HUB_LISTEN", "::1")], extra].concat());
    let started = Instant::now();
    let info: Value = loop {
        if let Ok(info) = std::fs::read_to_string(sandbox.home().join(".cache/cockpit/hub.json")).map(|t| serde_json::from_str::<Value>(&t).unwrap()) {
            break info;
        }
        assert!(started.elapsed() < Duration::from_secs(5), "no TCP discovery file");
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut stream = std::net::TcpStream::connect(("::1", info["port"].as_u64().unwrap() as u16)).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    writeln!(stream, "{}", json!({"op": "subscribe", "docs": ["ci"], "token": info["token"]})).unwrap();
    let mut first = String::new();
    std::io::BufReader::new(&stream).read_line(&mut first).unwrap();
    assert!(first.starts_with(r#"{"doc":"ci""#), "{first}");
    (hub, stream)
}

#[test]
fn a_tcp_subscriber_that_stopped_reading_is_reset_rather_than_left_waiting_for_a_fin() {
    use std::io::Read;
    let sandbox = sandbox();
    let (hub, mut stream) = tcp_subscribed(&sandbox, &[("COCKPIT_HUB_WRITE_SECS", "1")]);
    // Loopback buffers take megabytes before a write blocks.
    let blob = "x".repeat(1 << 20);
    for n in 0..30 {
        hub.request(&json!({"op": "store.put", "name": "ci", "value": {"n": n, "blob": blob}}));
    }
    std::thread::sleep(Duration::from_secs(3));
    let mut buf = vec![0; 1 << 16];
    let end = loop {
        match stream.read(&mut buf) {
            Ok(0) => break "a FIN",
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => break "a reset",
            Err(_) => break "nothing",
        }
    };
    assert_eq!(end, "a reset", "a FIN waits behind the lines a peer with its window shut never takes, so the hub resets");
}

#[test]
fn a_subscriber_that_promised_keepalives_and_went_quiet_is_let_go() {
    use std::io::{BufRead, Write};
    let sandbox = sandbox();
    let _hub = Hub::start(&sandbox, &[]);
    let mut stream = std::os::unix::net::UnixStream::connect(sandbox.socket()).unwrap();
    writeln!(stream, "{}", json!({"op": "subscribe", "docs": ["ci"], "keepalive": 1})).unwrap();
    let mut first = String::new();
    std::io::BufReader::new(&stream).read_line(&mut first).unwrap();
    assert!(closed_by_the_hub(stream, Duration::from_secs(6)), "no line for three keepalives keeps the subscription");
}

#[test]
fn a_subscriber_that_keeps_its_keepalives_stays() {
    use std::io::{BufRead, Write};
    let sandbox = sandbox();
    let hub = Hub::start(&sandbox, &[]);
    let mut stream = std::os::unix::net::UnixStream::connect(sandbox.socket()).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    writeln!(stream, "{}", json!({"op": "subscribe", "docs": ["ci"], "keepalive": 1})).unwrap();
    let mut pushed = std::io::BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    pushed.read_line(&mut line).unwrap();
    for _ in 0..10 {
        std::thread::sleep(Duration::from_millis(500));
        writeln!(stream).unwrap();
    }
    hub.request(&json!({"op": "store.put", "name": "ci", "value": {"runs": ["kept"]}}));
    line.clear();
    pushed.read_line(&mut line).unwrap();
    assert_eq!(serde_json::from_str::<Value>(&line).unwrap()["value"]["runs"][0], "kept");
}
