//! How fresh each source the feed reads is: when it was last read, when it is read next, and why the last read failed,
//! kept between rounds and written into the document the source fills as `hub.fresh.<source>`, with its name and the
//! command that signs it in again. The views draw it as the headers' chips.

use std::collections::BTreeMap;

use cockpit_connectors::Server;
use cockpit_core::fresh::{Freshness, Kind};
use serde_json::{Value, json};

use super::http::Failure;

/// The document each source fills.
pub fn doc_of(source: &str) -> &'static str {
    match source {
        "ci" => "ci",
        "calendar" => "meetings",
        "mail" => "mail",
        "monitor" => "monitor",
        _ => "inbox",
    }
}

/// The feed's sources and what it last learnt of each: its freshness, its name, and the server that failed when the
/// source reads through two (the monitor).
#[derive(Debug, Default)]
pub struct Sources(BTreeMap<String, (Freshness, String, Option<String>)>);

/// The documents the sources fill.
pub const DOCS: [&str; 5] = ["inbox", "ci", "meetings", "mail", "monitor"];

impl Sources {
    /// What the documents' records say, so a hub that starts again keeps when each source was read and since when it
    /// fails.
    pub fn seed(docs: &[Value]) -> Sources {
        let mut sources = Sources::default();
        for (id, value) in docs.iter().filter_map(|d| d["hub"]["fresh"].as_object()).flatten() {
            let mut fresh = Freshness::from_json(value);
            fresh.source = id.clone();
            let via = value["via"].as_str().map(str::to_string);
            sources.0.insert(id.clone(), (fresh, value["label"].as_str().unwrap_or(id).to_string(), via));
        }
        sources
    }

    /// Notes a read of `source` at `now` (ms) and when it is read next (seconds, as the schedule keeps it).
    pub fn record(&mut self, source: &str, label: &str, outcome: &Result<(), Failure>, now: u64, every_secs: u64, next_secs: u64) {
        let (fresh, name, via) = self.0.entry(source.to_string()).or_insert_with(|| (Freshness::new(source), String::new(), None));
        *name = label.to_string();
        fresh.every = Some(every_secs * 1000);
        fresh.next = Some(next_secs * 1000);
        match outcome {
            Ok(()) => {
                fresh.read(now);
                *via = None;
            }
            Err(failure) => fresh.failed(now, kind(failure), &message(failure)),
        }
    }

    /// Names the server of `source` whose read failed, for its sign-in.
    pub fn failed_through(&mut self, source: &str, server: &str) {
        if let Some((_, _, via)) = self.0.get_mut(source) {
            *via = Some(server.to_string());
        }
    }

    /// Forgets `source` (the monitor when the platform maps no deploys).
    pub fn forget(&mut self, source: &str) -> bool {
        self.0.remove(source).is_some()
    }

    /// Forgets the sources the config no longer reads, and says which they were.
    pub fn keep(&mut self, live: &[&str]) -> Vec<String> {
        let gone: Vec<String> = self.0.keys().filter(|s| !live.contains(&s.as_str())).cloned().collect();
        self.0.retain(|source, _| live.contains(&source.as_str()));
        gone
    }

    /// The `hub.fresh` record of a document: each of its sources' freshness, name and login.
    pub fn of_doc(&self, doc: &str, raw: &Value, config: &Value, now: u64) -> Value {
        let mut out = serde_json::Map::new();
        for (source, (fresh, label, via)) in self.0.iter().filter(|(s, _)| doc_of(s) == doc) {
            let mut entry = fresh.json(now);
            entry["label"] = json!(label);
            entry["login"] = json!(match login(source, raw, config, via.as_deref()) {
                Ok(argv) => shown(&argv),
                Err(say) => say,
            });
            entry["via"] = json!(via);
            out.insert(source.clone(), entry);
        }
        Value::Object(out)
    }
}

/// The document with its sources' record set; a document no one wrote yet gets the record alone, and an empty record
/// takes the old one away.
pub fn stamp(doc: &Value, record: Value) -> Value {
    if record.as_object().is_none_or(|r| r.is_empty()) {
        return without(doc, None);
    }
    let mut doc = if doc.is_object() { doc.clone() } else { json!({}) };
    if !doc["hub"].is_object() {
        doc["hub"] = json!({});
    }
    doc["hub"]["fresh"] = record;
    doc
}

/// The document without `source`'s record, or without any when `source` is None.
pub fn without(doc: &Value, source: Option<&str>) -> Value {
    let mut doc = doc.clone();
    if let Some(hub) = doc.get_mut("hub").and_then(Value::as_object_mut) {
        match source {
            Some(source) => {
                if let Some(fresh) = hub.get_mut("fresh").and_then(Value::as_object_mut) {
                    fresh.remove(source);
                }
            }
            None => {
                hub.remove("fresh");
            }
        }
    }
    doc
}

/// A document written whole by another writer (`store put`) keeps the hub's record of its sources: the hub's own,
/// never a copy the writer carried, which is dropped when the hub has none.
pub fn keep_record(before: &Value, written: Value) -> Value {
    if !written.is_object() {
        return written;
    }
    match &before["hub"]["fresh"] {
        Value::Object(_) => stamp(&written, before["hub"]["fresh"].clone()),
        _ => without(&written, None),
    }
}

/// What a failure's message says went wrong: a refused login, a rate limit, no network, nothing there, the service's
/// own error.
pub fn kind(failure: &Failure) -> Kind {
    let message = match failure {
        Failure::Limited(_) => return Kind::Rate,
        Failure::Other(message) => message,
    };
    let m = message.to_ascii_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| m.contains(w));
    if m.contains(super::http::UNREACHABLE) {
        return Kind::Network;
    }
    // GitHub's secondary limit is a 403 that says so in words.
    if has(&["rate limit", "rate-limit", "abuse", "too many requests"]) {
        return Kind::Rate;
    }
    if let Some(status) = status(&m) {
        return match status {
            401 | 403 => Kind::Auth,
            404 | 410 => Kind::NotFound,
            429 => Kind::Rate,
            500..=599 => Kind::Server,
            _ if m.contains("authenticat") => Kind::Auth,
            _ => Kind::Other,
        };
    }
    // A refused or lapsed MCP login, as cockpit's connector and the MCP client word it.
    let auth = [
        "connector login",
        "not logged in",
        "login unusable",
        "no token",
        "authenticat",
        "bad credentials",
        "printed no token",
        "auth required",
        "authorization required",
        "oauth",
        "invalid_token",
        "insufficient scope",
        "authoriz",
    ];
    // The connection's own failure comes first: "OAuth discovery failed: connection refused" never reached the login.
    if has(&["host not found", "dns", "error sending request", "connection refused", "connection reset", "connection failed", "network is unreachable"]) {
        return Kind::Network;
    }
    if has(&auth) {
        return Kind::Auth;
    }
    Kind::guess(message)
}

/// The status of an "HTTP 401: …" message.
fn status(message: &str) -> Option<u16> {
    let at = message.find("http ")?;
    message[at + 5..].get(..3).and_then(|code| code.parse().ok())
}

/// The message a failure is shown with.
fn message(failure: &Failure) -> String {
    match failure {
        Failure::Limited(_) => "rate limited: read again once the limit resets".into(),
        Failure::Other(message) => message.clone(),
    }
}

/// This binary, which the commands it hands out run.
fn exe() -> String {
    std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| "cockpit".into())
}

fn join(words: &[String]) -> String {
    crate::value::join(words)
}

/// The section of the config a source reads through.
fn section(source: &str) -> &str {
    match source {
        "github" => "code",
        "monitor" => "observability",
        other => other,
    }
}

/// The command that signs a source in again, as it is read (`via`: the server that failed): a connector login, `gh auth
/// login`, else `cockpit setup`. Err says what to do instead when no command does it, like a refused GH_TOKEN.
pub fn login(source: &str, raw: &Value, config: &Value, via: Option<&str>) -> Result<Vec<String>, String> {
    let cockpit = |args: &[&str]| std::iter::once(exe()).chain(args.iter().map(|a| a.to_string())).collect::<Vec<_>>();
    // A server counts only when the config's `servers` block holds it: the source is read through it, not a token.
    let name = |section: &str| {
        let name = raw[section]["server"].as_str().or(config[section]["server"].as_str()).filter(|s| !s.is_empty())?;
        raw["servers"][name].is_object().then(|| name.to_string())
    };
    let has_token = |section: &str| raw[section]["token"].as_str().is_some_and(|t| !t.trim().is_empty());
    let gh = || gh(token_var());
    let server = match source {
        _ if via.is_some_and(|v| raw["servers"][v].is_object()) => via.map(str::to_string),
        "monitor" => name("observability").or_else(|| name("datadog")).or_else(|| name("platform")),
        "github" if name("code").is_none() => return gh(),
        "ci" if has_token("ci") => None,
        "ci" if name("ci").is_none() => return gh(),
        other => name(section(other)),
    };
    let Some(server) = server else { return Ok(cockpit(&["setup"])) };
    match Server::from_entry(&raw["servers"][&server]) {
        Some(Server::Http(_) | Server::App { .. }) => Ok(cockpit(&["connector", "login", &server])),
        Some(Server::Keyed { bearer_command, .. }) if bearer_command.first().is_some_and(|p| p == "gh" || p.ends_with("/gh")) => gh(),
        _ => Ok(cockpit(&["setup"])),
    }
}

/// The variable the hub reads GitHub's token from, when one is set: gh's own login is not what it uses then.
fn token_var() -> Option<&'static str> {
    ["GH_TOKEN", "GITHUB_TOKEN"].into_iter().find(|v| std::env::var(v).is_ok_and(|t| !t.trim().is_empty()))
}

/// GitHub's sign-in: `gh auth login`, or what to do when the token comes from `var`.
fn gh(var: Option<&str>) -> Result<Vec<String>, String> {
    match var {
        Some(var) => Err(format!("cockpit reads GitHub with {var}: replace that token where it is set, then refresh")),
        None => Ok(vec!["gh".into(), "auth".into(), "login".into()]),
    }
}

/// The command as the person would type it: `cockpit`, not this binary's path.
pub fn shown(argv: &[String]) -> String {
    let words: Vec<String> = argv.iter().enumerate().map(|(i, w)| if i == 0 && *w == exe() { "cockpit".to_string() } else { w.clone() }).collect();
    join(&words)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: u64 = 60_000;

    fn other(message: &str) -> Result<(), Failure> {
        Err(Failure::Other(message.into()))
    }

    #[test]
    fn each_failure_is_read_as_its_kind() {
        let kinds: Vec<Kind> = [
            Failure::Other("HTTP 401: token refused".into()),
            Failure::Other("Linear HTTP 403: Resource protected".into()),
            Failure::Limited(9),
            Failure::Other("unreachable: io: Connection refused (os error 61)".into()),
            Failure::Other("https://mcp.example.com/mcp: error sending request for url".into()),
            Failure::Other("HTTP 404: Not Found".into()),
            Failure::Other("HTTP 502: bad gateway".into()),
            Failure::Other("cockpit is not logged in to https://mcp.example.com → cockpit connector login https://mcp.example.com".into()),
            Failure::Other("no token: run gh auth login, or set GH_TOKEN".into()),
            Failure::Other("HTTP 400: Authentication required, not authenticated".into()),
            Failure::Other("the calendar address answered with no calendar".into()),
            Failure::Other("https://mcp.example.com/mcp: Auth required".into()),
            Failure::Other("OAuth token refresh failed: invalid_grant".into()),
            Failure::Other("Transport error: invalid_token".into()),
            Failure::Other("HTTP 403: You have exceeded a secondary rate limit".into()),
            Failure::Other("GitHub HTTP 403: abuse detection mechanism".into()),
            Failure::Other("OAuth discovery failed: connection refused".into()),
        ]
        .iter()
        .map(kind)
        .collect();
        use Kind::*;
        assert_eq!(kinds, [Auth, Auth, Rate, Network, Network, NotFound, Server, Auth, Auth, Auth, Other, Auth, Auth, Auth, Rate, Rate, Network]);
    }

    #[test]
    fn a_source_is_fresh_then_failing_then_fresh_again() {
        let mut sources = Sources::default();
        sources.record("github", "GitHub", &Ok(()), 10 * MIN, 30, 630);
        let doc = |s: &Sources, now| s.of_doc("inbox", &Value::Null, &Value::Null, now);
        assert_eq!((&doc(&sources, 10 * MIN)["github"]["state"], &doc(&sources, 10 * MIN)["github"]["every"]), (&json!("fresh"), &json!(30_000)));
        sources.record("github", "GitHub", &other("HTTP 401: token refused"), 11 * MIN, 30, 690);
        sources.record("github", "GitHub", &other("HTTP 401: token refused"), 12 * MIN, 30, 750);
        let failing = doc(&sources, 12 * MIN);
        assert_eq!(failing["github"]["state"], "failing");
        assert_eq!(failing["github"]["error"], json!({"kind": "auth", "message": "HTTP 401: token refused", "since": 11 * MIN}));
        assert_eq!((&failing["github"]["at"], &failing["github"]["next"]), (&json!(10 * MIN), &json!(750_000)));
        assert_eq!(failing["github"]["login"], json!(gh(token_var()).map(|argv| shown(&argv)).unwrap_or_else(|say| say)));
        sources.record("github", "GitHub", &Ok(()), 13 * MIN, 30, 810);
        let back = doc(&sources, 13 * MIN);
        assert_eq!((&back["github"]["state"], &back["github"]["error"], &back["github"]["at"]), (&json!("fresh"), &Value::Null, &json!(13 * MIN)));
    }

    #[test]
    fn data_well_past_its_next_read_is_stale() {
        let mut sources = Sources::default();
        sources.record("calendar", "Calendar", &Ok(()), 0, 300, 300);
        let at = |now| sources.of_doc("meetings", &Value::Null, &Value::Null, now)["calendar"]["state"].clone();
        // Due at 5 min, then two intervals.
        assert_eq!(at(15 * MIN), "fresh");
        assert_eq!(at(15 * MIN + 1), "stale");
        assert!(sources.of_doc("inbox", &Value::Null, &Value::Null, 0).as_object().unwrap().is_empty(), "each record in its own document");
    }

    #[test]
    fn offline_a_rate_limit_and_a_missing_page_say_so_and_recover() {
        let mut sources = Sources::default();
        sources.record("ci", "CI", &other("unreachable: io: Connection refused"), 5 * MIN, 30, 330);
        sources.record("mail", "Mail", &Err(Failure::Limited(1800)), 5 * MIN, 60, 1800);
        sources.record("tickets", "Linear", &other("HTTP 404: Not Found"), 5 * MIN, 60, 360);
        sources.record("monitor", "Monitor", &other("HTTP 503: unavailable"), 5 * MIN, 120, 420);
        let kind = |doc: &str, source: &str| sources.of_doc(doc, &Value::Null, &Value::Null, 5 * MIN)[source]["error"]["kind"].clone();
        assert_eq!([kind("ci", "ci"), kind("mail", "mail"), kind("inbox", "tickets"), kind("monitor", "monitor")], ["network", "rate", "not_found", "server"]);
        assert_eq!(sources.of_doc("mail", &Value::Null, &Value::Null, 5 * MIN)["mail"]["next"], 1_800_000, "a rate limit says when it lifts");
        sources.record("ci", "CI", &Ok(()), 6 * MIN, 30, 390);
        assert_eq!(sources.of_doc("ci", &Value::Null, &Value::Null, 6 * MIN)["ci"]["error"], Value::Null);
        assert_eq!(sources.keep(&["ci"]), ["mail", "monitor", "tickets"], "the sources the config dropped, to take off their documents");
        assert!(sources.of_doc("mail", &Value::Null, &Value::Null, 6 * MIN).as_object().unwrap().is_empty(), "a source the config dropped goes");
        assert!(sources.keep(&["ci"]).is_empty());
    }

    #[test]
    fn a_hub_that_starts_again_keeps_when_each_source_was_read_and_since_when_it_fails() {
        let mut before = Sources::default();
        before.record("github", "GitHub", &Ok(()), MIN, 30, 90);
        before.record("tickets", "Linear", &other("HTTP 401: token refused"), 2 * MIN, 60, 180);
        let inbox = stamp(&json!({"version": 1}), before.of_doc("inbox", &Value::Null, &Value::Null, 2 * MIN));
        let mut after = Sources::seed(&[inbox, Value::Null]);
        after.record("tickets", "Linear", &other("HTTP 401: token refused"), 9 * MIN, 60, 600);
        let doc = after.of_doc("inbox", &Value::Null, &Value::Null, 9 * MIN);
        assert_eq!((&doc["github"]["at"], &doc["github"]["label"]), (&json!(MIN), &json!("GitHub")));
        assert_eq!(doc["tickets"]["error"]["since"], 2 * MIN, "the failure began before the restart");
    }

    #[test]
    fn a_dropped_source_leaves_its_document_and_a_whole_write_keeps_the_record() {
        let ci = json!({"at": 5, "hub": {"fresh": {"ci": {"source": "ci"}, "x": {}}}});
        assert_eq!(without(&ci, Some("ci"))["hub"]["fresh"], json!({"x": {}}));
        assert_eq!(stamp(&ci, json!({})), json!({"at": 5, "hub": {}}), "an empty record takes the old one away");
        let written = keep_record(&ci, json!({"at": 9, "runs": []}));
        assert_eq!((&written["at"], &written["hub"]["fresh"]["ci"]), (&json!(9), &json!({"source": "ci"})));
        assert_eq!(keep_record(&Value::Null, json!({"at": 9})), json!({"at": 9}));
        let forged = json!({"at": 9, "hub": {"fresh": {"ci": {"source": "ci", "state": "fresh", "at": 9}}}});
        assert_eq!(keep_record(&ci, forged.clone())["hub"]["fresh"]["ci"], json!({"source": "ci"}), "the hub's own record, not the writer's copy");
        assert_eq!(keep_record(&json!({"at": 5}), forged)["hub"]["fresh"], Value::Null, "a copy the hub never wrote is dropped");
    }

    #[test]
    fn signing_in_goes_through_the_sources_own_way() {
        let servers =
            json!({"Linear": {"url": "https://mcp.example.com/linear"}, "mail": {"command": "mail-mcp"}, "Datadog": {"url": "https://mcp.example.com/dd"}});
        let raw = json!({"tickets": {"server": "Linear"}, "mail": {"server": "mail"}, "observability": {"server": "Datadog"}, "ci": {"token": "t"}, "servers": servers});
        let line = |source: &str, raw: &Value, via: Option<&str>| login(source, raw, &Value::Null, via).map(|argv| shown(&argv)).unwrap_or_else(|say| say);
        assert_eq!(line("tickets", &raw, None), "cockpit connector login Linear");
        assert_eq!(line("mail", &raw, None), "cockpit setup", "a server run as a command has no browser login");
        assert_eq!(line("monitor", &raw, None), "cockpit connector login Datadog");
        assert_eq!(line("ci", &raw, None), "cockpit setup", "a CircleCI token is asked again");
        assert_eq!(line("calendar", &raw, None), "cockpit setup");
        let gh_way = gh(token_var()).map(|argv| shown(&argv)).unwrap_or_else(|say| say);
        assert_eq!(line("github", &raw, None), gh_way);
        assert_eq!(line("ci", &Value::Null, None), gh_way, "GitHub's checks sign in as GitHub does");
        let code = json!({"code": {"server": "GitLab"}, "servers": {"GitLab": {"url": "https://mcp.example.com/gitlab"}}});
        assert_eq!(line("tickets", &json!({"tickets": {"server": "claude.ai Linear", "token": "t"}}), None), "cockpit setup", "a token, no server");
        assert_eq!(line("github", &code, None), "cockpit connector login GitLab");
        let keyed = json!({"code": {"server": "gh"}, "servers": {"gh": {"url": "https://api.example.com/mcp", "bearerCommand": ["gh", "auth", "token"]}}});
        assert_eq!(line("github", &keyed, None), gh_way, "a token gh prints is renewed by gh");
        let both = json!({"servers": {"platform": {"url": "https://mcp.example.com/platform"}, "Datadog": {"command": "dd-mcp"}},
                          "platform": {"server": "platform"}, "observability": {"server": "Datadog"}});
        assert_eq!(line("monitor", &both, Some("platform")), "cockpit connector login platform", "the server that failed");
        assert_eq!(line("monitor", &both, None), "cockpit setup");
        assert_eq!(gh(Some("GH_TOKEN")), Err("cockpit reads GitHub with GH_TOKEN: replace that token where it is set, then refresh".into()));
    }

    #[test]
    fn a_monitor_failing_before_any_numbers_is_stamped_with_the_server_that_failed() {
        let raw = json!({"servers": {"platform": {"url": "https://mcp.example.com/platform"}, "Datadog": {"url": "https://mcp.example.com/dd"}},
                         "platform": {"server": "platform"}, "observability": {"server": "Datadog"}});
        let mut sources = Sources::default();
        sources.record("monitor", "Monitor", &other("platform: connection refused"), 3 * MIN, 120, 300);
        sources.failed_through("monitor", "platform");
        let doc = stamp(&Value::Null, sources.of_doc("monitor", &raw, &Value::Null, 3 * MIN));
        let monitor = &doc["hub"]["fresh"]["monitor"];
        assert_eq!(
            (&monitor["state"], &monitor["via"], &monitor["login"]),
            (&json!("failing"), &json!("platform"), &json!("cockpit connector login platform"))
        );
        assert_eq!((doc.get("at"), &monitor["error"]["since"]), (None, &json!(3 * MIN)), "no numbers yet, only the record of the first failure");
        sources.record("monitor", "Monitor", &other("platform: connection refused"), 5 * MIN, 120, 420);
        assert_eq!(sources.of_doc("monitor", &raw, &Value::Null, 5 * MIN)["monitor"]["error"]["since"], 3 * MIN, "the streak keeps its start");
        sources.failed_through("monitor", "Datadog");
        assert_eq!(sources.of_doc("monitor", &raw, &Value::Null, 5 * MIN)["monitor"]["login"], "cockpit connector login Datadog");
    }

    #[test]
    fn a_record_lands_beside_the_documents_fields() {
        let record = json!({"ci": {"source": "ci"}});
        assert_eq!(stamp(&json!({"at": 5, "runs": []}), record.clone()), json!({"at": 5, "runs": [], "hub": {"fresh": {"ci": {"source": "ci"}}}}));
        assert_eq!(stamp(&json!({"hub": {"at": 3}}), record.clone())["hub"], json!({"at": 3, "fresh": {"ci": {"source": "ci"}}}));
        assert_eq!(stamp(&json!({"at": 5}), json!({})), json!({"at": 5}), "nothing to say, nothing written");
    }
}
