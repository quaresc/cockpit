//! The hub's feed against a fake GitHub (and Linear) served on a local port: the inbox and CI caches it writes, what it
//! keeps of the mod's, its notifications, and a rate limit it waits out.

mod common;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{Hub, Sandbox};
use serde_json::{Value, json};

const TOKEN: &str = "ghp_feedtest";
const LINEAR_TOKEN: &str = "lin_api_feedtest";

/// A request the fake API received: method, path and Authorization header.
type Seen = Arc<Mutex<Vec<(String, String, String)>>>;
type Route = dyn Fn(&str, &str, &str) -> (u16, Vec<(String, String)>, String) + Send + Sync;

/// Serves each request with `route`, one per connection, and returns its base URL.
fn serve(route: Box<Route>) -> (String, Seen) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen: Seen = Arc::default();
    let log = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let mut parts = line.split_whitespace();
            let (method, path) = (parts.next().unwrap_or_default().to_string(), parts.next().unwrap_or_default().to_string());
            let (mut auth, mut length) = (String::new(), 0);
            loop {
                let mut header = String::new();
                reader.read_line(&mut header).unwrap();
                let header = header.trim_end();
                if header.is_empty() {
                    break;
                }
                let (name, value) = header.split_once(':').unwrap_or_default();
                match name.to_ascii_lowercase().as_str() {
                    "authorization" => auth = value.trim().to_string(),
                    "content-length" => length = value.trim().parse().unwrap_or(0),
                    _ => {}
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let (status, headers, answer) = route(&method, &path, &auth);
            log.lock().unwrap().push((method, path, auth));
            let mut stream = stream;
            let extra: String = headers.iter().map(|(k, v)| format!("{k}: {v}\r\n")).collect();
            let _ = write!(
                stream,
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n{answer}",
                answer.len()
            );
        }
    });
    (base, seen)
}

fn fixture(name: &str) -> String {
    std::fs::read_to_string(common::fixtures().join("feed").join(name)).unwrap()
}

fn read(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap_or_default()).unwrap_or(Value::Null)
}

/// Waits for a cache file to hold what `ready` looks for.
fn wait_for(path: &Path, ready: impl Fn(&Value) -> bool) -> Value {
    let start = Instant::now();
    loop {
        let value = read(path);
        if ready(&value) {
            return value;
        }
        assert!(start.elapsed() < Duration::from_secs(15), "{} never got ready: {value}", path.display());
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn edit(path: &Path, change: impl FnOnce(&mut Value)) {
    let mut value = read(path);
    change(&mut value);
    std::fs::write(path, value.to_string()).unwrap();
}

#[test]
fn github_and_linear_land_in_the_caches_beside_the_mods_fields() {
    let (base, seen) = serve(Box::new(|method, path, auth| match (method, path, auth) {
        ("POST", "/graphql", a) if a == format!("bearer {TOKEN}") => (200, vec![], fixture("github.json")),
        ("POST", "/graphql", LINEAR_TOKEN) => (200, vec![], fixture("linear.json")),
        ("GET", "/repos/acme/api/actions/jobs/111", _) => (200, vec![], json!({"steps": [{"name": "Run clippy", "conclusion": "failure"}]}).to_string()),
        _ => (404, vec![], json!({"message": "Not Found"}).to_string()),
    }));
    let sandbox = Sandbox::new(&json!({}), &json!({"ci": {"enabled": true, "name": "GitHub", "server": "", "imageJob": "docker_build"},
        "tickets": {"enabled": true, "name": "Linear", "server": "claude.ai Linear", "token": LINEAR_TOKEN}}));
    let cache = sandbox.home().join(".cache/cockpit");
    // The mod's last fetch: no direct review request yet, and the run of my PR still going.
    edit(&cache.join("inbox.json"), |inbox| inbox["direct"] = json!([]));
    std::fs::write(
        cache.join("ci.json"),
        json!({"version": 3, "at": 1, "runs": [{"repo": "api", "branch": "me/eng-401-login", "runId": "c0ffee4", "isRunning": true, "outcome": "running",
            "createdAt": "2026-10-27T15:50:00Z", "done": 1, "total": 4}]})
        .to_string(),
    )
    .unwrap();
    let hub = Hub::start(&sandbox, &[("COCKPIT_HUB_FEED", "on"), ("GH_TOKEN", TOKEN), ("COCKPIT_GITHUB_API", &base), ("COCKPIT_LINEAR_API", &base)]);
    hub.request(&json!({"op": "ping"}));

    let inbox = wait_for(&cache.join("inbox.json"), |v| v["hub"]["github"].is_number() && v["hub"]["tickets"].is_number());
    assert_eq!(inbox["version"], 1);
    assert_eq!(inbox["direct"][0]["url"], "https://github.com/acme/web/pull/9");
    assert_eq!(inbox["team"][0]["url"], "https://github.com/acme/docs/pull/3");
    assert_eq!(inbox["botRepos"], json!(["api"]));
    assert_eq!(inbox["mine"][0]["failedChecks"], json!(["lint", "ci/circleci: test"]));
    assert_eq!(inbox["merged"][0]["mergeSha"], "abc123");
    assert_eq!(inbox["tickets"].as_array().unwrap().len(), 4);
    assert_eq!(inbox["deploys"]["ENG-401"][0]["tag"], "v1.2", "the mod's fields stay");
    assert_eq!(inbox["errors"], json!([]));

    let ci = wait_for(&cache.join("ci.json"), |v| v["by"] == "hub");
    assert_eq!(ci["runs"][0]["failed"]["step"], "Run clippy");
    assert_eq!(ci["runs"][0]["dockerTag"], "succeeded");

    let calls = sandbox.calls();
    let notified: Vec<&str> = calls.iter().filter(|c| c["cmd"] == "notify").filter_map(|c| c["args"][1].as_str()).collect();
    assert!(notified.contains(&"👀 Review requested: web#9 Dark mode"), "{notified:?}");
    assert!(notified.contains(&"🔴 CI failed: api me/eng-401-login · lint"), "{notified:?}");
    assert!(notified.contains(&"🎫 Assigned: ENG-405 Deploy SSO"), "{notified:?}");
    assert!(!calls.iter().any(|c| c["cmd"] == "gh"), "a token in the env needs no gh");

    // Asked again, it fetches again at once, well before its minute.
    let before = seen.lock().unwrap().len();
    std::fs::write(cache.join("refresh-request"), "1").unwrap();
    let start = Instant::now();
    while seen.lock().unwrap().len() == before {
        assert!(start.elapsed() < Duration::from_secs(10), "no fetch after a refresh request");
        std::thread::sleep(Duration::from_millis(100));
    }
    for file in ["inbox.json", "ci.json", "config.json"] {
        let text = std::fs::read_to_string(cache.join(file)).unwrap_or_default();
        assert!(!text.contains(TOKEN) && !text.contains(LINEAR_TOKEN), "a token leaked into {file}");
    }
}

#[test]
fn a_rate_limit_is_said_and_waited_out_even_when_asked() {
    let reset = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() + 3600).to_string();
    let (base, seen) = serve(Box::new(move |_, _, _| {
        let headers = vec![("x-ratelimit-remaining".to_string(), "0".to_string()), ("x-ratelimit-reset".to_string(), reset.clone())];
        (403, headers, json!({"message": "API rate limit exceeded"}).to_string())
    }));
    let sandbox = Sandbox::new(&json!({}), &json!({}));
    let cache = sandbox.home().join(".cache/cockpit");
    let hub = Hub::start(&sandbox, &[("COCKPIT_HUB_FEED", "on"), ("GH_TOKEN", TOKEN), ("COCKPIT_GITHUB_API", &base)]);
    hub.request(&json!({"op": "ping"}));
    let inbox = wait_for(&cache.join("inbox.json"), |v| v["hub"]["errors"].as_array().is_some_and(|e| !e.is_empty()));
    let line = inbox["errors"][0].as_str().unwrap();
    assert!(line.starts_with("GitHub: rate limited, next try at "), "{line}");
    assert!(inbox["hub"].get("github").is_none());
    assert_eq!(inbox["mine"][0]["url"], "https://github.com/acme/api/pull/4", "the last data stays");
    std::fs::write(cache.join("refresh-request"), "1").unwrap();
    std::thread::sleep(Duration::from_secs(5));
    assert_eq!(seen.lock().unwrap().len(), 1, "no call before the reset");
}
