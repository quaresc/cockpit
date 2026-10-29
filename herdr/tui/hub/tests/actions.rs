//! Every action against its expected outcome: the request runs through the hub on a sandbox, and the herdr, claude and
//! script calls, the reply and the files it changed must match `tests/fixtures/expected/<case>.json`.
//! `RECORD_EXPECTED=1 cargo test -p cockpit-hub --test actions` rewrites those files from the hub's outcomes.

mod common;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::LazyLock;

use common::{Hub, Sandbox, fixtures};
use serde_json::{Map, Value, json};

struct Case {
    name: &'static str,
    app: &'static str,
    action: &'static str,
    /// The arguments as JSON, @BOX@ standing for the sandbox's root.
    args: String,
    armed: Option<&'static str>,
    /// herdr's answers by command prefix, {"fail": message} for a failure.
    herdr: String,
    config: &'static str,
}

fn case(name: &'static str, app: &'static str, action: &'static str, args: impl Into<String>) -> Case {
    Case { name, app, action, args: args.into(), armed: None, herdr: "{}".into(), config: "{}" }
}

impl Case {
    fn armed(mut self, key: &'static str) -> Self {
        self.armed = Some(key);
        self
    }

    fn herdr(mut self, answers: impl Into<String>) -> Self {
        self.herdr = answers.into();
        self
    }

    fn config(mut self, patch: &'static str) -> Self {
        self.config = patch;
        self
    }
}

const PR4: &str = r#"{"repo": "api", "number": 4, "title": "Google sign-in", "url": "https://github.com/acme/api/pull/4", "review": "CHANGES_REQUESTED", "ci": "FAILURE", "branch": "me/eng-401-login", "base": "main", "failedChecks": ["lint", "test"], "isDraft": false}"#;
const RUN1: &str = r#"{"repo": "api", "branch": "me/eng-401-login", "workflowId": "wf1", "isRunning": false, "outcome": "failed", "failed": {"job": "test", "step": "mix test", "tests": ["LoginTest"], "logs": "circleci get_job_logs"}}"#;
const TICKET1: &str = r#"{"id": "ENG-401", "title": "Sign in with Google", "url": "https://linear.app/acme/issue/ENG-401/sign-in", "status": "In Progress", "statusType": "started"}"#;
const TICKET3: &str =
    r#"{"id": "ENG-403", "title": "Password reset", "url": "https://linear.app/acme/issue/ENG-403/reset", "status": "Backlog", "statusType": "backlog"}"#;
const STEP2: &str = r#"{"n": "2", "step": "Run the migration", "where": "prod", "status": "⬜ to do", "command": "mix ecto.migrate"}"#;
const NO_ACTIONS: &str = r#"{"workspace list": {"workspaces": [{"workspace_id": "w1", "label": "[1] x  inbox", "focused": false}]}}"#;
/// The api app as the monitor document has it, with its env: a release of mine that moved, an older one, a new error and
/// an alerting monitor.
const MON_API: &str = r#"{"id": "api-production", "name": "api", "service": "api", "repo": "api", "env": "production", "health": "Healthy", "tag": "v2026.10.28.1", "deployedAt": "2026-10-28T15:52:57Z", "argoUrl": "https://argocd.example.com/applications/api-production", "datadogUrl": "https://app.datadoghq.eu/apm/entity/service%3Aapi?env=production", "watched": true, "pinned": true, "monitors": {"ok": 12, "warn": 0, "alert": 1, "nodata": 3, "alerting": [{"id": 17255491, "name": "[api] Current user - API Availability", "status": "Alert", "url": "https://app.datadoghq.eu/monitors/17255491"}]}, "issues": [{"id": "1fcdadd8", "type": "Req.TransportError", "message": "socket closed", "count": 140, "firstSeen": "2026-10-28T15:55:59Z", "firstVersion": "main-v2026.10.28.1", "url": "https://app.datadoghq.eu/error-tracking/issue/1fcdadd8", "isNew": true}], "releases": [{"id": 622, "tag": "v2026.10.28.1", "actor": "me", "isMine": true, "phase": "Succeeded", "startedAt": "2026-10-28T15:50:47Z", "finishedAt": "2026-10-28T15:58:09Z", "tickets": [{"id": "ENG-401", "url": "https://linear.app/acme/issue/ENG-401/sign-in"}], "prs": [{"repo": "api", "number": 4, "url": "https://github.com/acme/api/pull/4"}], "verdict": {"state": "moved", "summary": "errors 0.13% → 1.9% · p95 33 → 161 ms · 1 new error", "before": {"errors": 2, "hits": 1512, "p95": 0.033}, "after": {"errors": 31, "hits": 1630, "p95": 0.161}, "newIssues": ["1fcdadd8"], "until": "2026-10-28T16:28:09Z"}}, {"id": 619, "tag": "v2026.10.28", "actor": "jonas", "isMine": false, "phase": "Succeeded", "startedAt": "2026-10-28T15:27:03Z", "finishedAt": "2026-10-28T15:34:29Z", "tickets": [], "prs": [], "verdict": {"state": "ok", "summary": "errors 0.12% → 0.13% · p95 31 → 33 ms", "newIssues": []}}]}"#;
const AGENT: &str = r#"{"pane get": {"pane": {"pane_id": "w9:p2", "tab_id": "w9:t2", "agent": "claude"}}}"#;

fn cases() -> Vec<Case> {
    vec![
        case("open_url", "inbox", "open_url", r#"["https://example.com/a b"]"#),
        case("details", "inbox", "details", format!("[{PR4}]")),
        case("details falls back to the browser", "inbox", "details", format!("[{PR4}]")).herdr(r#"{"plugin pane open": {"fail": "no such plugin"}}"#),
        case("mute", "inbox", "set_muted", r#"["docs", true]"#),
        case("unmute", "inbox", "set_muted", r#"["web", false]"#),
        case("review in the actions space", "inbox", "review", format!("[{PR4}]")),
        case("review already running", "inbox", "review", r#"[{"repo": "web", "number": 9, "url": "https://github.com/acme/web/pull/9"}]"#).herdr(AGENT),
        case("review without the actions space", "inbox", "review", r#"[{"repo": "docs", "number": 3, "url": "https://github.com/acme/docs/pull/3"}]"#)
            .herdr(NO_ACTIONS),
        case("review whose space runs", "inbox", "review", r#"[{"repo": "docs", "number": 3, "url": "https://github.com/acme/docs/pull/3"}]"#)
            .herdr(r#"{"workspace list": {"workspaces": [{"workspace_id": "w8", "label": "[8] review docs#3"}]}}"#),
        case("review with herdr failing", "inbox", "review", format!("[{PR4}]"))
            .herdr(r#"{"workspace": {"fail": "herdr is down"}}"#)
            .config(r#"{"spaces": ["inbox"]}"#),
        case("fix ci here", "inbox", "fix_ci", format!(r#"[{PR4}, {RUN1}, "here"]"#)),
        case("fix ci here, space open", "inbox", "fix_ci", format!(r#"[{PR4}, {RUN1}, "here"]"#)).herdr(
            r#"{"workspace list": {"workspaces": [{"workspace_id": "w7", "label": "[7] api login", "worktree": {"checkout_path": "@BOX@/repos/api"}}]}}"#,
        ),
        case("fix ci on a new PR", "inbox", "fix_ci", format!(r#"[{PR4}, {RUN1}, "new"]"#)),
        case(
            "fix ci with no run",
            "inbox",
            "fix_ci",
            r#"[{"repo": "web", "number": 12, "url": "https://github.com/acme/web/pull/12", "branch": "me/eng-401-form", "failedChecks": ["e2e"]}, null, "here"]"#,
        ),
        case(
            "fix ci in a new worktree",
            "inbox",
            "fix_ci",
            r#"[{"repo": "api", "number": 7, "url": "https://github.com/acme/api/pull/7", "branch": "me/other"}, null, "here"]"#,
        )
        .config(r#"{"worktrees": true}"#),
        case("rerun asks first", "inbox", "rerun", format!("[{PR4}, {RUN1}]")),
        case("rerun", "inbox", "rerun", format!("[{PR4}, {RUN1}]")).armed("rerun-wf1"),
        case("redeploy", "inbox", "redeploy", r#"[{"id": "pv-eng-401", "app": "api"}, "https://github.com/acme/api/pull/4"]"#)
            .armed("redeploy-pv-eng-401-api"),
        case("create a preview", "inbox", "create_preview", r#"[{"repo": "web", "branch": "me/eng-401-form", "url": "https://github.com/acme/web/pull/12"}]"#)
            .armed("preview-web-me/eng-401-form"),
        case("stop a preview", "inbox", "stop_preview", r#"["pv-eng-401"]"#).armed("stop-pv-eng-401"),
        case("extend a preview", "inbox", "extend", r#"["pv-eng-401"]"#).armed("extend-pv-eng-401"),
        case("extend a preview 3 days", "inbox", "extend", r#"["pv-eng-401", 3]"#).armed("extend-pv-eng-401"),
        case("release a ticket", "inbox", "mark_done", r#"[{"id": "ENG-402"}]"#).armed("done-ENG-402"),
        case("release asks first", "inbox", "mark_done", r#"[{"id": "ENG-402"}]"#).armed("done-ENG-401"),
        case("plan", "inbox", "plan", format!("[{TICKET3}]")),
        case("plan without url", "inbox", "plan", r#"[{"id": "ENG-409"}]"#),
        case("continue on the PR's branch", "inbox", "continue_ticket", format!("[{TICKET1}, [{PR4}]]")),
        case("continue moves the plan tab", "inbox", "continue_ticket", format!("[{TICKET1}, []]")).herdr(moved()),
        case("continue without PR", "inbox", "continue_ticket", format!("[{TICKET3}, []]")),
        case("talk forks the busiest session", "inbox", "talk", format!(r#"[{TICKET1}, [{PR4}], {{"$inbox": true}}]"#)),
        case("talk fresh", "inbox", "talk", format!(r#"[{TICKET3}, [], {{"$inbox": true}}]"#)),
        case("talk forks the owner", "inbox", "talk", format!(r#"[{TICKET3}, [], {{}}, "s-main"]"#)),
        case("close a tab", "inbox", "close_space", r#"["tab:w3:t2", "api me/x", false]"#).armed("close-tab:w3:t2"),
        case("remove a worktree", "inbox", "close_space", r#"["w7", "api me/x", true]"#).armed("close-w7"),
        case("worktree kept", "inbox", "close_space", r#"["w7", "api me/x", true]"#)
            .armed("close-w7")
            .herdr(r#"{"worktree remove": {"fail": "uncommitted changes in it"}}"#),
        case("close a space", "inbox", "close_space", r#"["w7", "api me/x", false]"#).armed("close-w7"),
        case("close asks first", "inbox", "close_space", r#"["w7", "api me/x", false]"#),
        case("dismiss the brief", "inbox", "dismiss_brief", "[]"),
        case("request a refresh", "inbox", "request_refresh", "[]"),
        case("run step asks first", "runbook", "run_step", format!(r#"[{{"$book": "eng-401"}}, {STEP2}]"#)),
        case("run step", "runbook", "run_step", format!(r#"[{{"$book": "eng-401"}}, {STEP2}]"#)).armed("run-eng-401-2"),
        case("run step of a book passed whole", "runbook", "run_step", BOOK_WHOLE).armed("run-notes-1"),
        case("mark step done", "runbook", "mark_done", format!(r#"[{{"$book": "eng-401"}}, {STEP2}]"#)),
        case("mark a missing step done", "runbook", "mark_done", r#"[{"$book": "eng-401"}, {"n": "9"}]"#),
        case("open a known PR", "runbook", "open_pr", r#"["api", 4]"#),
        case("open a PR from the org", "runbook", "open_pr", r#"["zzz", 5]"#),
        case("open a PR with no org", "runbook", "open_pr", r#"["zzz", 5]"#).config(r#"{"github": {"enabled": true, "org": ""}}"#),
        case("session not picked up", "runbook", "focus_session", r#"[{"$handoff": "eng-401-1"}]"#),
        case("focus the session", "runbook", "focus_session", r#"[{"$handoff": "eng-401-2"}]"#),
        case("session gone", "runbook", "focus_session", r#"[{"$handoff": "eng-401-2"}]"#).herdr(r#"{"pane get": {"fail": "no such pane"}}"#),
        case("refresh a runbook", "runbook", "update", r#"[{"$book": "eng-401"}]"#),
        case("refresh a runbook with no owner", "runbook", "update", r#"[{"$book": "notes"}]"#)
            .config(r#"{"platform": {"enabled": false}, "runbookTools": []}"#),
        case("talk about a runbook", "runbook", "talk", r#"[{"$book": "eng-401"}]"#),
        case("talk about a runbook with no ticket", "runbook", "talk", r#"[{"$book": "notes"}]"#),
        case("delete asks first", "runbook", "delete", r#"[{"$book": "notes"}]"#),
        case("delete", "runbook", "delete", r#"[{"$book": "notes"}]"#).armed("delete-@BOX@/home/.config/cockpit/runbooks/notes.md"),
        case("announce a refresh", "runbook", "announce", r#"["eng-401"]"#),
        case("announce while in view", "runbook", "announce", r#"["eng-401"]"#)
            .herdr(r#"{"workspace list": {"workspaces": [{"workspace_id": "w5", "label": "[5] x  runbook", "focused": true}]}}"#),
        case("copy a command", "runbook", "copy", r#"["mix ecto.migrate"]"#),
        case("open reviewr", "show", "open_reviewr", r#"["@BOX@/repos/api"]"#),
        case("reviewr fails", "show", "open_reviewr", r#"["@BOX@/repos/api"]"#).herdr(r#"{"plugin pane open": {"fail": "reviewr is not installed"}}"#),
        case("prepare a brief", "meetings", "prepare", r#"[{"id": "ev1_20261028T084500Z", "title": "Team daily"}]"#),
        case(
            "prepare from the meetings app",
            "meetings",
            "prepare",
            r#"[{"id": "ev1_20261028T084500Z", "title": "Team daily", "start": "2026-10-28T09:45:00+01:00", "end": "2026-10-28T10:00:00+01:00", "url": "https://calendar/ev1", "join": "https://meet/abc", "response": "accepted", "attendees": 9, "briefable": true}]"#,
        ),
        case("brief already on its way", "meetings", "prepare", r#"[{"id": "ev2/odd id", "title": "Planning"}]"#),
        case("open a meeting", "meetings", "open_url", r#"["https://calendar/ev1"]"#),
        case("open a ticket from the to-do", "todo", "open_link", r#"["eng-401"]"#),
        case("open a URL from the to-do", "todo", "open_link", r#"["https://x.y/z"]"#),
        case("investigate an app", "monitor", "investigate", format!("[{MON_API}, {{}}]")),
        case("investigate a release", "monitor", "investigate", format!(r#"[{MON_API}, {{"release": 622}}]"#)),
        case("investigate an error", "monitor", "investigate", format!(r#"[{MON_API}, {{"issue": "1fcdadd8"}}]"#)),
        case("investigate a monitor", "monitor", "investigate", format!(r#"[{MON_API}, {{"monitor": "17255491"}}]"#)),
        case("fix an error", "monitor", "fix", format!(r#"[{MON_API}, "1fcdadd8"]"#)),
        case("bug for an error", "monitor", "bug", format!(r#"[{MON_API}, {{"issue": "1fcdadd8"}}]"#)),
        case("bug for a release", "monitor", "bug", format!(r#"[{MON_API}, {{"release": 622}}]"#)),
        case("roll back asks first", "monitor", "roll_back", format!(r#"[{MON_API}, "622", "v2026.10.28"]"#)),
        case("roll back", "monitor", "roll_back", format!(r#"[{MON_API}, "622", "v2026.10.28"]"#)).armed("rollback-api-production-v2026.10.28.1"),
        case("talk about my release", "monitor", "talk", format!(r#"[{MON_API}, "622"]"#)),
        case("talk about a release with no ticket", "monitor", "talk", format!(r#"[{MON_API}, "619"]"#)),
        case("pin an app", "monitor", "pin", r#"["web-production", true]"#),
        case("unpin an app", "monitor", "pin", r#"["api-production", false]"#),
        case("refresh the monitor", "monitor", "request_refresh", "[]"),
        case("open from the monitor", "monitor", "open_url", r#"["https://app.datadoghq.eu/monitors/17255491"]"#),
    ]
}

fn moved() -> String {
    let mut answers: Value = serde_json::from_str(AGENT).unwrap();
    answers["pane move"] = json!({"workspace_id": "wN"});
    answers.to_string()
}

const BOOK_WHOLE: &str = r#"[{"path": "@BOX@/home/.config/cockpit/runbooks/notes.md", "slug": "notes", "title": "Notes for later", "updated": "", "steps": [], "notes": []}, {"n": "1", "step": "Look", "where": "api#4", "status": "⬜ to do", "command": ""}]"#;

/// Text with the sandbox's root and the clock taken out: today's date too, as a step marked done says it.
fn norm_text(text: &str, root: &Path) -> String {
    let root = root.to_string_lossy();
    let today = chrono::Local::now().format("done %-d %b").to_string();
    let text = text.replace(&format!("/private{root}"), "<BOX>").replace(&*root, "<BOX>").replace(&today, "done <TODAY>");
    static MS: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"\b\d{13}-").unwrap());
    static MINUTE: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"\d{4}-\d{2}-\d{2} \d{2}:\d{2}").unwrap());
    MINUTE.replace_all(&MS.replace_all(&text, "<MS>-"), "<MINUTE>").into_owned()
}

fn norm(value: &Value, root: &Path) -> Value {
    match value {
        Value::String(s) => Value::String(norm_text(s, root)),
        Value::Array(items) => Value::Array(items.iter().map(|v| norm(v, root)).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| {
                    let v = if ["at", "sentAt", "reportedAt"].contains(&k.as_str()) && v.is_number() { json!(0) } else { norm(v, root) };
                    (norm_text(k, root), v)
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

/// The sandbox's files after the action, by path: JSON parsed, the rest as text, clock and root taken out.
fn files(root: &Path) -> BTreeMap<String, Value> {
    fn walk(dir: &Path, root: &Path, out: &mut BTreeMap<String, Value>) {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if [".git", "hub.sock", "store.redb", "history", "sessions-index", "herdr", "bin", "calls.log", "herdr.json"].contains(&name.as_str()) {
                continue;
            }
            if path.is_dir() {
                walk(&path, root, out);
                continue;
            }
            let rel = norm_text(&path.strip_prefix(root).unwrap().to_string_lossy(), root);
            let content = std::fs::read_to_string(&path).unwrap_or_default();
            let value = if name == "refresh-request" {
                json!("<T>")
            } else {
                serde_json::from_str::<Value>(&content).map(|v| norm(&v, root)).unwrap_or_else(|_| json!(norm_text(&content, root)))
            };
            out.insert(rel, value);
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

/// The files an action wrote or removed (null), against the sandbox's files before it.
fn changes(before: &BTreeMap<String, Value>, after: BTreeMap<String, Value>) -> Value {
    let removed = before.keys().filter(|path| !after.contains_key(*path)).map(|path| (path.clone(), Value::Null)).collect::<Vec<_>>();
    let written = after.into_iter().filter(|(path, value)| before.get(path) != Some(value));
    Value::Object(written.chain(removed).collect::<Map<_, _>>())
}

/// What the action did, with the sandbox's root and the clock taken out.
fn outcome(sandbox: &Sandbox, reply: &Value, before: &BTreeMap<String, Value>) -> Value {
    let root = &sandbox.root;
    // A background refresh logs its claude call whenever it gets to it: its exit file says it has.
    for slug in reply["refreshing"].as_object().into_iter().flatten().map(|(slug, _)| slug) {
        let exit = sandbox.home().join(format!(".cache/cockpit/runbook-refresh-{slug}.exit"));
        let start = std::time::Instant::now();
        while !exit.exists() && start.elapsed() < std::time::Duration::from_secs(10) {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
    // Linux opens a URL with xdg-open where macOS uses open: the recorded calls say open.
    let opener = |cmd: &Value| {
        if cmd == "xdg-open" { json!("open") } else { cmd.clone() }
    };
    let calls: Vec<Value> =
        sandbox.calls().iter().map(|c| norm(&json!({"cmd": opener(&c["cmd"]), "args": c["args"], "stdin": c.get("stdin")}), root)).collect();
    let mut refreshing: Vec<&String> = reply["refreshing"].as_object().map(|m| m.keys().collect()).unwrap_or_default();
    refreshing.sort();
    json!({
        "note": norm(&reply["note"], root),
        "error": norm(&reply["error"], root),
        "pending": norm(&reply["pending"], root),
        "refreshing": refreshing,
        "calls": calls,
        "files": changes(before, files(root)),
    })
}

/// The case's expected outcome file: its name in lower case, words joined by dashes.
fn expected_path(case: &Case) -> std::path::PathBuf {
    let slug: String = case.name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    fixtures().join("expected").join(format!("{slug}.json"))
}

fn run(case: &Case) -> Option<String> {
    let sandbox = Sandbox::new(&json!({}), &serde_json::from_str(case.config).unwrap());
    std::fs::write(sandbox.root.join("herdr.json"), sandbox.fill(&case.herdr).to_string()).unwrap();
    let before = files(&sandbox.root);
    let armed = case.armed.map(|a| a.replace("@BOX@", &sandbox.root.to_string_lossy()));
    let hub = Hub::start(&sandbox, &[]);
    let reply = hub.request(&json!({"op": "act", "app": case.app, "action": case.action, "args": sandbox.fill(&case.args), "armed": armed}));
    let got = outcome(&sandbox, &reply, &before);
    drop(hub);
    let path = expected_path(case);
    if std::env::var_os("RECORD_EXPECTED").is_some() {
        std::fs::write(&path, serde_json::to_string_pretty(&got).unwrap() + "\n").unwrap();
        return None;
    }
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Some(format!("{}: no {}, RECORD_EXPECTED=1 records it", case.name, path.display()));
    };
    let want: Value = serde_json::from_str(&text).unwrap();
    (want != got).then(|| format!("{}:\n  expected: {}\n  hub:      {}", case.name, want, got))
}

#[test]
fn every_action_has_its_expected_outcome() {
    let cases = cases();
    let failures: Vec<String> = std::thread::scope(|scope| {
        let handles: Vec<_> =
            cases.chunks(cases.len().div_ceil(8)).map(|chunk| scope.spawn(move || chunk.iter().filter_map(run).collect::<Vec<_>>())).collect();
        handles.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });
    assert!(failures.is_empty(), "{} of {} actions differ from their expected outcome:\n{}", failures.len(), cases.len(), failures.join("\n\n"));
}

#[test]
fn an_unknown_action_answers_unknown() {
    let sandbox = Sandbox::new(&json!({}), &json!({}));
    let hub = Hub::start(&sandbox, &[]);
    let reply = hub.request(&json!({"op": "act", "app": "inbox", "action": "no_such_thing", "args": [], "armed": null}));
    assert_eq!(reply, json!({"unknown": true}));
}
