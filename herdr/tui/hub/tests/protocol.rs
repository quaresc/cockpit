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
    writeln!(writes.get_mut(), "{}", json!({"op": "store.put", "name": "meetings", "value": {"items": ["one"]}, "token": info["token"]})).unwrap();
    assert_eq!(read(&mut writes)["value"]["items"][0], "one");
    // The queue's items become prompts: no client writes them, on any side.
    writeln!(writes.get_mut(), "{}", json!({"op": "store.put", "name": "queue", "value": {"items": ["rm -rf"]}, "token": info["token"]})).unwrap();
    assert!(read(&mut writes)["error"].as_str().unwrap().contains("hub alone"));
}

#[test]
fn an_idle_subscription_hears_from_the_hub_before_a_herdr_read_gives_up() {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpStream;

    let sandbox = sandbox();
    let hub = Hub::start(&sandbox, &[]);
    hub.request(&json!({"op": "ping"}));
    let started = Instant::now();
    let info: Value = loop {
        if let Ok(text) = std::fs::read_to_string(sandbox.home().join(".cache/cockpit/hub.json")) {
            break serde_json::from_str(&text).unwrap();
        }
        assert!(started.elapsed() < Duration::from_secs(5), "no TCP discovery file");
        std::thread::sleep(Duration::from_millis(20));
    };
    let stream = TcpStream::connect(("127.0.0.1", info["port"].as_u64().unwrap() as u16)).unwrap();
    // Herdr's socket reads give up after 30 s with nothing to read: a subscription that waits half of that hears a line.
    stream.set_read_timeout(Some(Duration::from_secs(15))).unwrap();
    let mut stream = BufReader::new(stream);
    writeln!(stream.get_mut(), "{}", json!({"op": "subscribe", "docs": ["inbox"], "token": info["token"]})).unwrap();
    let mut line = String::new();
    stream.read_line(&mut line).unwrap();
    assert_eq!(serde_json::from_str::<Value>(&line).unwrap()["doc"], "inbox");
    let waited = Instant::now();
    line.clear();
    stream.read_line(&mut line).expect("an empty line before the read gives up");
    assert_eq!(line, "\n", "nothing changed: the heartbeat");
    assert!(waited.elapsed() < Duration::from_secs(12), "{:?}", waited.elapsed());
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

/// Codex and OpenCode installed as stand-ins that log their calls, each with models kept from a listing of its own.
fn listed_agents(sandbox: &Sandbox) {
    for agent in ["codex", "opencode"] {
        std::fs::copy(sandbox.root.join("bin/claude"), sandbox.root.join("bin").join(agent)).unwrap();
    }
    let kept = json!({
        "codex": {"at": 1, "attempted": 1, "models": [{"id": "gpt-acme-1", "label": "GPT Acme 1", "efforts": ["low", "high"], "fast": true}, {"id": "gpt-acme-mini", "label": "GPT Acme Mini"}]},
        "opencode": {"at": 1, "attempted": 1, "models": ["acme/large", "acme/small"]},
    });
    std::fs::write(sandbox.home().join(".cache/cockpit/models.json"), kept.to_string()).unwrap();
    // The hub writes the routines' document once there is a routine: one run by hand only.
    let routines = sandbox.home().join(".config/cockpit/routines");
    std::fs::create_dir_all(&routines).unwrap();
    std::fs::write(routines.join("standup.md"), "---\nname: Standup notes\nwhen: manual\n---\n\nSum up ENG-412.\n").unwrap();
}

#[test]
fn start_work_and_routines_offer_each_agent_the_same_models_and_ask_no_agent_in_a_test() {
    let sandbox = sandbox();
    listed_agents(&sandbox);
    let hub = Hub::start(&sandbox, &[]);
    let plan = hub.request(&json!({"op": "fetch", "kind": "start_work", "args": {"request": {}}}));
    assert!(plan["choices"]["agents"].is_array(), "the plan gives a form its choices: {plan}");
    let started = Instant::now();
    let routines = loop {
        let doc = hub.request(&json!({"op": "store.get", "name": "routines"}))["value"].clone();
        if doc["agents"]["installed"].as_array().is_some_and(|a| !a.is_empty()) {
            break doc;
        }
        assert!(started.elapsed() < Duration::from_secs(10), "the routines were never read: {doc}");
        std::thread::sleep(Duration::from_millis(50));
    };
    let ids = |list: &Value, key: Option<&str>| -> Vec<String> {
        list.as_array().into_iter().flatten().filter_map(|m| key.map_or(m.as_str(), |k| m[k].as_str())).map(str::to_string).collect()
    };
    let of = |list: &Value, id: &str| {
        list.as_array().and_then(|a| a.iter().find(|a| a["id"] == id)).cloned().unwrap_or_else(|| panic!("{id} is not offered: {list}"))
    };
    for (agent, models, routine_only) in [
        ("codex", vec!["gpt-acme-1", "gpt-acme-mini"], vec![]),
        ("opencode", vec!["acme/large", "acme/small"], vec![]),
        ("claude", vec!["opus", "sonnet", "haiku", "fable"], vec!["lightest"]),
    ] {
        let picked = ids(&of(&plan["choices"]["agents"], agent)["picks"]["models"], Some("id"));
        assert_eq!(picked, models, "{agent}: what Start work and the chat's chips offer");
        let expected: Vec<String> = [""].into_iter().chain(routine_only).chain(models).map(str::to_string).collect();
        assert_eq!(ids(&of(&routines["agents"]["installed"], agent)["models"], None), expected, "{agent}: what a routine offers");
    }
    let codex = of(&plan["choices"]["agents"], "codex")["picks"].clone();
    assert_eq!(
        (codex["models"][0]["efforts"].clone(), codex["models"][0]["fast"].clone(), codex["loading"].clone()),
        (json!(["low", "high"]), json!(true), json!(false))
    );
    // The kept lists are a day old and more: a hub outside a test would ask each agent again. Here none is run.
    std::thread::sleep(Duration::from_millis(700));
    let log = std::fs::read_to_string(sandbox.root.join("calls.log")).unwrap_or_default();
    let asked: Vec<&str> = log.lines().filter(|l| l.contains(r#""cmd":"codex""#) || l.contains(r#""cmd":"opencode""#)).collect();
    assert!(asked.is_empty(), "an agent was run from PATH: {asked:?}");
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
  "run list") echo '[{"databaseId":42,"conclusion":"failure","workflowName":"CI","headSha":"c0ffee"},{"databaseId":41,"conclusion":"success","workflowName":"Docs","headSha":"c0ffee"},{"databaseId":40,"conclusion":"failure","workflowName":"CI","headSha":"0ld"}]' ;;
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
    assert_eq!((&log["run"], &log["runs"], &log["sha"]), (&json!(42), &json!([42]), &json!("c0ffee")), "the newest commit's failed runs only");
    let at = hub.request(&json!({"op": "fetch", "kind": "ci_log", "args": {"repo": "api", "branch": "me/eng-401-login", "sha": "c0ffee"}}));
    assert_eq!(at["runs"], json!([42]));
    let gh = std::fs::read_to_string(sandbox.root.join("gh.log")).unwrap();
    assert!(gh.contains("run list --repo acme/api --limit 30 --json databaseId,conclusion,workflowName,headSha --commit c0ffee"), "{gh}");
}

#[test]
fn fetch_errors_reach_the_app() {
    let sandbox = Sandbox::new(&json!({}), &json!({"github": {"enabled": true, "org": ""}}));
    let hub = Hub::start(&sandbox, &[]);
    let answer = hub.request(&json!({"op": "fetch", "kind": "pr_files", "args": {"repo": "api", "number": 4}}));
    assert_eq!(answer, json!({"error": "set github.org in the cockpit config"}));
}

#[test]
fn history_of_ticket_estimates() {
    let sandbox = sandbox();
    let hub = Hub::start(&sandbox, &[]);
    let values = |hub: &Hub| hub.request(&json!({"op": "history", "kind": "ticket_estimate", "key": "ENG-401"}))["values"].clone();
    // Waits on the hub's first count, so the budget is wide: the whole file's hubs share this machine.
    let start = Instant::now();
    while values(&hub) != json!([8.75]) {
        assert!(start.elapsed() < Duration::from_secs(30), "no first point: {}", values(&hub));
        std::thread::sleep(Duration::from_millis(100));
    }
    // A new session's estimate, $1.25 at $4 per million input tokens, all of it ENG-401's by its book.
    let call = r#"{"type": "assistant", "timestamp": "2026-10-28T11:00:00Z", "requestId": "r-new", "message": {"id": "m-new", "model": "claude-opus-5-5", "usage": {"input_tokens": 312500, "output_tokens": 0}}}"#;
    std::fs::write(sandbox.home().join(".claude/projects/-box-repos/s-new.jsonl"), format!("{call}\n")).unwrap();
    std::fs::write(sandbox.home().join(".cache/cockpit/costs/s-new.json"), r#"{"ENG-401": 1}"#).unwrap();
    // A usage chip's refresh counts again at once.
    hub.request(&json!({"op": "act", "app": "sources", "action": "refresh", "args": ["usage-all"], "armed": null}));
    while values(&hub) != json!([8.75, 10.0]) {
        assert!(start.elapsed() < Duration::from_secs(60), "no second point: {}", values(&hub));
        std::thread::sleep(Duration::from_millis(100));
    }
    let estimates: Value = serde_json::from_str(&std::fs::read_to_string(sandbox.home().join(".cache/cockpit/estimates.json")).unwrap()).unwrap();
    assert_eq!((estimates["sessions"]["s-new"].as_f64(), estimates["tickets"]["ENG-401"].as_f64()), (Some(1.25), Some(10.0)));
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
fn meetings_join_and_brief() {
    let sandbox = sandbox();
    let hub = Hub::start(&sandbox, &[]);
    let meeting = json!({"id": "ev1_20261028T084500Z", "title": "Team daily", "join": "https://meet/abc"});
    let joined = hub.request(&json!({"op": "act", "app": "meetings", "action": "join", "args": [meeting], "armed": null}));
    assert_eq!(joined["note"], "opened https://meet/abc");
    let prepared = hub.request(&json!({"op": "act", "app": "meetings", "action": "prepare", "args": [meeting], "armed": null}));
    assert_eq!(prepared["note"], "preparing the brief of Team daily, about a minute");
}

#[test]
fn a_red_run_a_client_writes_is_marked_as_the_client_s() {
    // An agent's tools write the inbox: green, then red. The event says a client made it, so no CI news comes of it.
    let sandbox = sandbox();
    let hub = Hub::start(&sandbox, &[(common::SENT_BY, "app:t3")]);
    let pr = |ci: &str| json!({"version": 1, "mine": [{"repo": "api", "number": 4, "url": "https://github.com/acme/api/pull/4", "branch": "me/eng-401-login", "ci": ci}]});
    for ci in ["SUCCESS", "FAILURE"] {
        assert!(hub.request(&json!({"op": "store.put", "name": "inbox", "value": pr(ci)}))["value"].is_object());
    }
    let events = hub.request(&json!({"op": "events", "since": 0}))["events"].clone();
    let red = events.as_array().unwrap().iter().find(|e| e["kind"] == "ci_failing").unwrap_or_else(|| panic!("no red event in {events}"));
    assert_eq!(red["by"], "client", "{red}");
}

/// The notifications a window following `hub` gets while `send` runs: the {notify} lines' notifications.
fn notified(sandbox: &Sandbox, send: impl FnOnce()) -> Vec<Value> {
    use std::io::{BufRead, BufReader, Write};
    let mut stream = std::os::unix::net::UnixStream::connect(sandbox.socket()).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    writeln!(stream, "{}", json!({"op": "subscribe", "notify": true})).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    send();
    let lines = BufReader::new(stream).lines().map_while(Result::ok).filter(|l| !l.is_empty());
    lines.take(1).map(|l| serde_json::from_str::<Value>(&l).unwrap()["notify"].clone()).collect()
}

#[test]
fn a_client_s_notification_only_navigates_and_says_who_sent_it() {
    let pr = "https://github.com/acme/api/pull/4";
    let forged = json!({"title": "✅ Tests passed", "body": "Ready to merge\u{1b}[2J", "action": format!("act:merge:{pr}"), "row": "update",
                        "buttons": [format!("Merge=act:merge:{pr}"), "Update=update", "Docs=https://docs.acme.dev", "Resume=herdr-app:resume"]});
    for (sent_by, who) in [(Some("app:t3"), "an agent's tools (app:t3)"), (None, "a terminal")] {
        let sandbox = sandbox();
        let env: Vec<(&str, &str)> = sent_by.map(|h| (common::SENT_BY, h)).into_iter().collect();
        let hub = Hub::start(&sandbox, &env);
        let got = notified(&sandbox, || assert_eq!(hub.request(&json!({"op": "notify", "notification": forged})), json!({"delivered": true})));
        let n = &got[0];
        assert_eq!(n["title"], format!("From {who}: ✅ Tests passed"), "never cockpit's own words");
        assert_eq!((n["body"].as_str(), n["from"].as_str()), (Some("Ready to merge"), Some(who)));
        assert_eq!(
            (n["action"].as_str(), n["row"].as_str()),
            (Some(format!("inbox:{pr}").as_str()), Some(format!("inbox:{pr}").as_str())),
            "an act becomes its row"
        );
        assert_eq!(n["buttons"], json!([format!("Merge=inbox:{pr}"), "Docs=https://docs.acme.dev"]), "no act, update or resume is kept");
    }
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
