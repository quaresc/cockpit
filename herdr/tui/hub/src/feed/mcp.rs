//! Sources the hub reaches through an MCP server of its own, named in the config's `servers` block under the name the
//! source's `server` gives: `{"servers": {"claude.ai Linear": {"url": "https://mcp.linear.app/mcp"}}}`, or a local
//! `{"command": "npx", "args": [...]}`. The source's mapping is the one the Claude plugin reads.

use cockpit_connectors::observability::{self, Signals};
use cockpit_connectors::tools::Mapped;
use cockpit_connectors::{Connection, Server, block_on, calendar, ci, connect, monitor, platform, tickets};
use serde_json::Value;

use super::http::{Failure, Fetched};

/// The server the hub calls for a section, from the raw config, once it is ready to call.
pub fn server(raw: &Value, config: &Value, section: &str) -> Option<Server> {
    if config[section]["enabled"] == false {
        return None;
    }
    let name = config[section]["server"].as_str().filter(|s| !s.is_empty())?;
    Server::from_entry(&raw["servers"][name]).filter(Server::is_ready)
}

/// Whether the hub carries out a request of this type itself, through the server of the section it touches.
pub fn serves(raw: &Value, config: &Value, kind: &str) -> bool {
    server(raw, config, super::requests::section_of(kind)).is_some()
}

/// The server the hub reads the monitor's numbers from, and its section (see `observability::server`).
pub fn observability(raw: &Value) -> Option<(Server, Value)> {
    cockpit_connectors::observability::server(raw)
}

/// One connection to the server for the work `f` does with it, closed after.
pub fn with<T>(server: &Server, f: impl AsyncFnOnce(&Connection) -> Result<T, String>) -> Result<T, String> {
    block_on(async {
        let connection = connect(server).await?;
        let out = f(&connection).await;
        connection.close().await;
        out
    })
}

/// My open tickets through the server, with the tickets section's mapping.
pub fn tickets(server: &Server, section: &Value) -> Fetched<Vec<Value>> {
    let mapping = tickets::mapping(section);
    with(server, async |c| tickets::mine(&Mapped { connection: c, section: &mapping }, &mapping).await).map_err(Failure::Other)
}

/// The inbox's PR fields (direct, team, botRepos, mine, merged) from a code host's server.
pub fn code(server: &Server, section: &Value) -> Fetched<[Value; 5]> {
    let mapping = cockpit_connectors::code::mapping(section);
    with(server, async |c| cockpit_connectors::code::inbox(&Mapped { connection: c, section: &mapping }, &mapping).await).map_err(Failure::Other)
}

/// My latest CI runs through the server, the ended ones taken from ci.json as it was.
pub fn ci_runs(server: &Server, section: &Value, prev: &Value) -> Fetched<Vec<Value>> {
    let mapping = ci::mapping(section);
    with(server, async |c| ci::runs(&Mapped { connection: c, section: &mapping }, &mapping, prev).await).map_err(Failure::Other)
}

/// The coming meetings through the server, briefable ones marked.
pub fn meetings(server: &Server, section: &Value, config: &Value) -> Fetched<Vec<Value>> {
    let mapping = calendar::mapping(section);
    let mut meetings = with(server, async |c| calendar::meetings(&Mapped { connection: c, section: &mapping }, &mapping).await).map_err(Failure::Other)?;
    let skip: Vec<String> = config["meetingBrief"]["skip"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
    super::calendar::mark_briefable(&mut meetings, &skip);
    Ok(meetings)
}

/// Whether a commit is in a ref yet, by GitHub's compare; answers are kept, as a merged commit never leaves.
fn is_contained(token: &str, slug: &str, sha: &str, reference: &str) -> bool {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static SEEN: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();
    let key = format!("{slug}:{sha}:{reference}");
    if let Some(hit) = SEEN.get_or_init(Default::default).lock().ok().and_then(|m| m.get(&key).copied()) {
        return hit;
    }
    let api = super::http::base("COCKPIT_GITHUB_API", "https://api.github.com");
    let url = format!("{api}/repos/{slug}/compare/{sha}...{reference}");
    let Ok(answer) = super::http::get(&url, &[("Authorization", &format!("Bearer {token}")), ("Accept", "application/vnd.github+json")]) else {
        return false;
    };
    let live = matches!(answer["status"].as_str(), Some("identical" | "ahead"));
    if let Ok(mut seen) = SEEN.get_or_init(Default::default).lock() {
        seen.insert(key, live);
    }
    live
}

/// My GitHub login, asked once.
pub fn login(token: &str) -> String {
    use std::sync::OnceLock;
    static LOGIN: OnceLock<String> = OnceLock::new();
    if let Some(login) = LOGIN.get() {
        return login.clone();
    }
    let api = super::http::base("COCKPIT_GITHUB_API", "https://api.github.com");
    let login =
        super::http::get(&format!("{api}/user"), &[("Authorization", &format!("Bearer {token}"))]).ok().and_then(|u| u["login"].as_str().map(str::to_string));
    login.map(|l| LOGIN.get_or_init(|| l).clone()).unwrap_or_default()
}

/// The inbox's deploys (each ticket waiting for production, its merged PRs and whether they are live) and my previews.
pub fn platform_part(server: &Server, section: &Value, inbox: &Value) -> Fetched<(Value, Value)> {
    let mapping = platform::mapping(section);
    let token = super::github::token().unwrap_or_default();
    // The platform's own name for me, else my GitHub login.
    let login = match section["login"].as_str().filter(|l| !l.is_empty()) {
        Some(login) => login.to_string(),
        None if token.is_empty() => String::new(),
        None => login(&token),
    };
    let tickets = inbox["tickets"].as_array().cloned().unwrap_or_default();
    let merged = inbox["merged"].as_array().cloned().unwrap_or_default();
    with(server, async |c| {
        let tools = Mapped { connection: c, section: &mapping };
        let deploys = platform::deploys_for(&tools, &mapping, &tickets, &merged, |slug, sha, reference| is_contained(&token, slug, sha, reference)).await?;
        let previews = if login.is_empty() { Vec::new() } else { platform::previews_for(&tools, &mapping, &login).await? };
        Ok((Value::Object(deploys), Value::Array(previews)))
    })
    .map_err(Failure::Other)
}

/// The monitor space's document, through the platform's server and Datadog's; None when the platform maps no deploys.
pub fn monitor_doc(
    platform_server: &Server,
    signals: &(Server, Value),
    raw: &Value,
    config: &Value,
    previous: &Value,
    inbox: &Value,
    now: i64,
) -> Result<Option<Value>, String> {
    let mapping = platform::mapping(&raw["platform"]);
    let settings = monitor::settings(&raw["monitor"]);
    let token = super::github::token().unwrap_or_default();
    let login = if token.is_empty() { String::new() } else { login(&token) };
    let ids = |file: &str| -> Vec<String> {
        cockpit_core::files::load(&cockpit_core::files::config_dir().join(file))
            .and_then(|v| v.as_array().map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()))
            .unwrap_or_default()
    };
    let (pinned, muted) = (ids("monitor-pinned.json"), ids("monitor-muted.json"));
    let offset = chrono::Local::now().offset().local_minus_utc() / 60;
    let ctx = monitor::Context {
        platform: &mapping,
        monitor: &settings,
        after_minutes: config["deployWatch"]["afterMinutes"].as_i64().unwrap_or(30),
        login: &login,
        org: config["github"]["org"].as_str().unwrap_or_default(),
        inbox: Some(inbox),
        pinned: &pinned,
        muted: &muted,
        now,
        utc_offset_minutes: offset,
    };
    let previous = previous.is_object().then_some(previous);
    block_on(async {
        let p = connect(platform_server).await?;
        let (server, section) = signals;
        let d = connect(server).await?;
        let tools = Mapped { connection: &d, section };
        let signals = if observability::is_datadog(section, section["server"].as_str().unwrap_or("")) {
            Signals::Datadog(&tools)
        } else {
            Signals::Mapped { tools: &tools, section }
        };
        let doc = monitor::refresh(&Mapped { connection: &p, section: &mapping }, &signals, &ctx, previous).await;
        p.close().await;
        d.close().await;
        doc
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_section_is_served_by_the_entry_its_server_names() {
        let config = json!({"tickets": {"server": "linear"}, "ci": {"server": "circleci", "enabled": false}});
        let raw = json!({"servers": {"linear": {"command": "linear-mcp", "args": ["--stdio"]}, "circleci": {"command": "c"}}});
        assert!(matches!(server(&raw, &config, "tickets"), Some(Server::Command { argv, .. }) if argv == ["linear-mcp", "--stdio"]));
        assert!(server(&raw, &config, "ci").is_none(), "a section that is off");
        assert!(server(&json!({}), &config, "tickets").is_none(), "no servers block");
    }
}
