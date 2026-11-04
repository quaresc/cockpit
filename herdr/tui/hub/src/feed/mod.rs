//! The hub's own feed: GitHub every minute (the inbox's PR fields, and ci.json when the CI is GitHub's checks), and
//! my tickets through an MCP server of the hub's own (`feed::mcp`) or Linear's API given `tickets.token`, and CircleCI runs
//! given `ci.token`, so the inbox stays fresh with no Claude session; meetings.json too, every five minutes, given the calendar's secret iCal address in `calendar.ics`.
//! A touch of refresh-request fetches at once; errors back off, rate limits wait for their reset. COCKPIT_HUB_FEED=off
//! keeps it still (the hub's tests), and the APIs' base URLs follow COCKPIT_GITHUB_API, COCKPIT_LINEAR_API and
//! COCKPIT_CIRCLECI_API.

mod calendar;
mod circleci;
mod github;
mod github_ci;
pub(crate) mod http;
mod ics;
mod linear;
pub(crate) mod mcp;
pub mod merge;
mod news;
mod requests;
mod schedule;

use std::path::Path;
use std::sync::Once;
use std::time::Duration;

use cockpit_core::files;
use serde_json::Value;

use crate::config::{self, Config};
use http::{Failure, Fetched};
use merge::InboxPart;
use schedule::Schedule;

const TICK: Duration = Duration::from_secs(2);

pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub fn now_secs() -> u64 {
    now_ms() / 1000
}

/// Starts the feed once, on the hub's first request.
pub fn start() {
    static STARTED: Once = Once::new();
    if std::env::var("COCKPIT_HUB_FEED").is_ok_and(|v| v == "off") {
        return;
    }
    STARTED.call_once(|| {
        std::thread::spawn(run);
    });
}

/// What the config hands the feed: which sources it fetches, and their tokens from the config file itself (the copy
/// the mod shares leaves tokens out).
struct Plan {
    config: Config,
    /// The config file as written, tokens and servers included.
    raw: Value,
    github: bool,
    github_ci: bool,
    linear: Option<String>,
    /// The MCP server the hub calls for my tickets, when the config's `servers` block names one.
    tickets_server: Option<cockpit_connectors::Server>,
    /// The MCP servers the hub calls for CI runs and meetings.
    ci_server: Option<cockpit_connectors::Server>,
    calendar_server: Option<cockpit_connectors::Server>,
    /// The platform's server, and Datadog's for the monitor space when it is pinned.
    platform_server: Option<cockpit_connectors::Server>,
    datadog_server: Option<cockpit_connectors::Server>,
    circleci: Option<String>,
    /// The calendar's secret iCal address.
    calendar: Option<String>,
}

fn plan() -> Plan {
    let config = config::load();
    let raw = files::load(&files::config_dir().join("config.json")).unwrap_or(Value::Null);
    let c = &config.0;
    let token = |section: &str, pattern: &str| {
        let enabled = c[section]["enabled"] != false;
        let names = format!("{} {}", c[section]["name"].as_str().unwrap_or_default(), c[section]["server"].as_str().unwrap_or_default()).to_lowercase();
        raw[section]["token"].as_str().map(str::trim).filter(|t| enabled && !t.is_empty() && names.contains(pattern)).map(str::to_string)
    };
    let github = c["github"]["enabled"] != false;
    let ci_server = c["ci"]["server"].as_str().unwrap_or_default();
    Plan {
        github,
        github_ci: github && c["ci"]["enabled"] != false && ci_server.is_empty(),
        linear: token("tickets", "linear"),
        tickets_server: mcp::server(&raw, c, "tickets"),
        ci_server: mcp::server(&raw, c, "ci").filter(|_| raw["ci"]["token"].as_str().is_none_or(|t| t.trim().is_empty())),
        calendar_server: mcp::server(&raw, c, "calendar").filter(|_| c["calendar"]["enabled"] == true && calendar::address(&raw["calendar"]["ics"]).is_none()),
        circleci: token("ci", "circleci").filter(|_| !ci_server.is_empty()),
        calendar: calendar::address(&raw["calendar"]["ics"]).filter(|_| c["calendar"]["enabled"] != false),
        platform_server: mcp::server(&raw, c, "platform").filter(|_| c["platform"]["enabled"] == true),
        datadog_server: mcp::datadog(&raw, c).filter(|_| config.space_names().iter().any(|s| s == "monitor")),
        raw: raw.clone(),
        config,
    }
}

#[derive(Default)]
struct Feed {
    github: Schedule,
    linear: Schedule,
    mcp_tickets: Schedule,
    mcp_ci: Schedule,
    mcp_platform: Schedule,
    monitor: Schedule,
    circleci: Schedule,
    calendar: Schedule,
    circle: circleci::CircleCi,
    /// The latest error of each source, until it fetches again.
    errors: Vec<(&'static str, String)>,
    asked: u64,
}

fn run() {
    let mut feed = Feed { calendar: Schedule::every(calendar::EVERY), monitor: Schedule::every(120), ..Feed::default() };
    loop {
        feed.round(&files::cache(), &plan());
        std::thread::sleep(TICK);
    }
}

/// When the inbox app (or the mod after a request) last asked for fresh data.
fn asked_at(cache: &Path) -> u64 {
    std::fs::metadata(cache.join("refresh-request"))
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl Feed {
    fn round(&mut self, cache: &Path, plan: &Plan) {
        let asked = asked_at(cache);
        let is_asked = asked > self.asked;
        self.asked = asked;
        let now = now_secs();
        let mut part: Option<InboxPart> = None;
        let mut runs: Option<Result<Vec<Value>, String>> = None;
        if plan.github && self.github.is_due(now, is_asked) {
            let fetched = github::token().ok_or_else(|| Failure::Other("no token: run gh auth login, or set GH_TOKEN".into())).and_then(|token| {
                let org = plan.config.0["github"]["org"].as_str().unwrap_or_default();
                let data = github::fetch(&token, org)?;
                Ok((token, data))
            });
            let error = self.github.after(&fetched.as_ref().map(|_| ()).map_err(Clone::clone), now);
            self.note("GitHub", error.clone());
            let prev_ci = crate::store::global().get("ci");
            let image_job = plan.config.0["ci"]["imageJob"].as_str().unwrap_or_default().to_string();
            match fetched {
                Ok((token, data)) => {
                    if plan.github_ci {
                        runs = Some(Ok(github_ci::runs(&data.mine_nodes, &prev_ci, &image_job, &mut |slug, job| github::failed_step(&token, slug, job))));
                    }
                    part = Some(InboxPart { github: Some(merge::github_fields(&data)), ..InboxPart::default() });
                }
                Err(_) => {
                    if plan.github_ci {
                        runs = Some(Err(format!("GitHub {}", error.unwrap_or_default())));
                    }
                    part = Some(InboxPart::default());
                }
            }
        }
        if let Some(server) = &plan.tickets_server
            && self.mcp_tickets.is_due(now, is_asked)
        {
            let name = plan.config.0["tickets"]["name"].as_str().unwrap_or("Tickets").to_string();
            let fetched = mcp::tickets(server, &plan.raw["tickets"]);
            let error = self.mcp_tickets.after(&fetched.as_ref().map(|_| ()).map_err(Clone::clone), now);
            self.note("Tickets", error.map(|e| format!("{name} {e}")));
            part = Some(InboxPart { tickets: fetched.ok(), ..part.unwrap_or_default() });
        } else if let Some(token) = &plan.linear
            && self.linear.is_due(now, is_asked)
        {
            let fetched = linear::fetch(token);
            let error = self.linear.after(&fetched.as_ref().map(|_| ()).map_err(Clone::clone), now);
            self.note("Linear", error);
            part = Some(InboxPart { tickets: fetched.ok(), ..part.unwrap_or_default() });
        }
        if let Some(server) = &plan.ci_server
            && self.mcp_ci.is_due(now, is_asked)
        {
            let name = plan.config.0["ci"]["name"].as_str().unwrap_or("CI").to_string();
            let prev = crate::store::global().get("ci");
            let prev = if prev["version"].as_i64() == Some(merge::CI_VERSION) { prev } else { Value::Null };
            let fetched = mcp::ci_runs(server, &plan.raw["ci"], &prev);
            let error = self.mcp_ci.after(&fetched.as_ref().map(|_| ()).map_err(Clone::clone), now);
            self.note("CI", error.clone().map(|e| format!("{name} {e}")));
            runs = Some(fetched.map_err(|_| format!("{name} {}", error.unwrap_or_default())));
        } else if let Some(token) = &plan.circleci
            && self.circleci.is_due(now, is_asked)
        {
            let fetched = self.circleci_runs(token, &plan.config);
            let error = self.circleci.after(&fetched.as_ref().map(|_| ()).map_err(Clone::clone), now);
            self.note("CircleCI", error.clone());
            runs = Some(fetched.map_err(|_| format!("CircleCI {}", error.unwrap_or_default())));
        }
        if let Some(server) = &plan.calendar_server
            && self.calendar.is_due(now, is_asked)
        {
            let fetched = mcp::meetings(server, &plan.raw["calendar"], &plan.config.0);
            let error = self.calendar.after(&fetched.as_ref().map(|_| ()).map_err(Clone::clone), now);
            let fetched = fetched.map_err(|_| format!("Calendar: {}", error.unwrap_or_default()));
            crate::store::global().update("meetings", |doc| calendar::merge(doc, fetched, now_ms()));
        } else if let Some(url) = &plan.calendar
            && self.calendar.is_due(now, is_asked)
        {
            let fetched = calendar::fetch(url, &plan.config);
            let error = self.calendar.after(&fetched.as_ref().map(|_| ()).map_err(Clone::clone), now);
            let fetched = fetched.map_err(|_| format!("Calendar: {}", error.unwrap_or_default()));
            crate::store::global().update("meetings", |doc| calendar::merge(doc, fetched, now_ms()));
        }
        if let Some(server) = &plan.platform_server
            && self.mcp_platform.is_due(now, is_asked)
        {
            // This round's tickets and merged PRs, else the inbox's.
            let mut inbox = crate::store::global().get("inbox");
            if let Some(tickets) = part.as_ref().and_then(|p| p.tickets.clone()) {
                inbox["tickets"] = tickets.into();
            }
            if let Some(github) = part.as_ref().and_then(|p| p.github.clone()) {
                inbox["merged"] = github[4].clone();
            }
            let fetched = mcp::platform_part(server, &plan.raw["platform"], &inbox);
            let error = self.mcp_platform.after(&fetched.as_ref().map(|_| ()).map_err(Clone::clone), now);
            self.note("Platform", error.map(|e| format!("Deploys and previews: {e}")));
            part = Some(InboxPart { platform: fetched.ok(), ..part.unwrap_or_default() });
        }
        if let (Some(server), Some(datadog)) = (&plan.platform_server, &plan.datadog_server)
            && self.monitor.is_due(now, is_asked)
        {
            let before = crate::store::global().get("monitor");
            let inbox = crate::store::global().get("inbox");
            let fetched = mcp::monitor_doc(server, datadog, &plan.raw, &plan.config.0, &before, &inbox, now_ms() as i64);
            let _ = self.monitor.after(&fetched.as_ref().map(|_| ()).map_err(|e| Failure::Other(e.clone())), now);
            if let Ok(Some(mut doc)) = fetched {
                doc["by"] = "hub".into();
                crate::store::global().update("monitor", |_| doc.clone());
                let notify = plan.config.0["monitor"]["notify"].as_str().unwrap_or("mine");
                for n in cockpit_connectors::monitor::changes(&before, &doc, notify) {
                    news::notify(&plan.config, &n.title, &n.body, &n.action, &[]);
                }
            }
        }
        warn_meetings(&plan.config, now_ms());
        requests::take(cache, &plan.config, &plan.raw);
        if let Some(mut part) = part {
            part.errors = self.errors.iter().map(|(_, e)| e.clone()).collect();
            write_inbox(&plan.config, &part);
        }
        if let Some(runs) = runs {
            write_ci(&plan.config, runs);
        }
    }

    fn note(&mut self, source: &'static str, error: Option<String>) {
        self.errors.retain(|(s, _)| *s != source);
        if let Some(error) = error {
            self.errors.push((source, format!("{source}: {error}")));
        }
    }

    fn circleci_runs(&mut self, token: &str, config: &Config) -> Fetched<Vec<Value>> {
        let api = http::base("COCKPIT_CIRCLECI_API", "https://circleci.com");
        let mut get = |path: &str| http::get(&format!("{api}{path}"), &[("Circle-Token", token), ("Accept", "application/json")]);
        let ci = &config.0["ci"];
        let name = ci["name"].as_str().unwrap_or("CircleCI").to_string();
        let mapped: Vec<String> = ["getJob", "listTests", "logs"].iter().filter_map(|c| ci["tools"][c]["tool"].as_str().map(str::to_string)).collect();
        let tools = if mapped.is_empty() { ["get_job", "list_job_tests", "get_job_logs"].map(str::to_string).to_vec() } else { mapped };
        let logs = move |job: &str| circleci::logs_hint(&name, &tools, job);
        let org = config.0["github"]["org"].as_str().unwrap_or_default();
        self.circle.runs(&mut get, org, ci["imageJob"].as_str().unwrap_or_default(), &logs)
    }
}

/// Warns once of each meeting starting within five minutes, where no Claude session's plugin does (no Claude Code).
fn warn_meetings(config: &Config, now: u64) {
    if config.0["calendar"]["enabled"] != true || cockpit_core::agent::get("claude").is_some_and(|a| a.installed()) {
        return;
    }
    let doc = crate::store::global().get("meetings");
    let soon = calendar::soon(&doc, now);
    if soon.is_empty() {
        return;
    }
    for m in &soon {
        let minutes = (calendar::start_ms(m).saturating_sub(now) + 30_000) / 60_000;
        let action = m["join"].as_str().or(m["url"].as_str()).unwrap_or("space:meetings");
        news::notify(config, "📅 Meeting", &format!("{} in {minutes} min", m["title"].as_str().unwrap_or("A meeting")), action, &[]);
    }
    let ids: Vec<Value> = soon.iter().map(|m| m["id"].clone()).collect();
    crate::store::global().update("meetings", |doc| {
        let mut doc = doc.clone();
        if let Some(notified) = doc["notified"].as_array_mut() {
            notified.extend(ids.iter().cloned());
        }
        doc
    });
}

/// The feed's part of the inbox into the store, then what changed announced.
fn write_inbox(config: &Config, part: &InboxPart) {
    let (before, after) = crate::store::global().update("inbox", |doc| merge::merge_inbox(doc, part, now_ms()));
    let mut changes = Vec::new();
    if part.github.is_some() && merge::has_github(&before) {
        changes.extend(news::pr_changes(&before, &after));
    }
    if part.tickets.is_some() && before["version"].as_i64() == Some(merge::INBOX_VERSION) {
        changes.extend(news::ticket_changes(&before, &after));
    }
    if part.platform.is_some() && before["hub"]["platform"].is_number() {
        changes.extend(news::deploy_changes(&before, &after));
    }
    news::announce_inbox(config, &changes);
}

fn write_ci(config: &Config, runs: Result<Vec<Value>, String>) {
    let (before, after) = crate::store::global().update("ci", |doc| merge::merge_ci(doc, runs, now_ms()));
    if before["version"].as_i64() == Some(merge::CI_VERSION) && after["error"].is_null() {
        let inbox = crate::store::global().get("inbox");
        news::announce_ci(config, &news::ci_changes(&before, &after, &inbox));
    }
}
