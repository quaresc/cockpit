//! The hub's own feed: GitHub, tickets, CI, deploys, the calendar, mail and the monitor read on their intervals (at once
//! on a refresh request, later after errors, a rate limit waited out), each source's freshness written beside its data.
//! COCKPIT_HUB_FEED=off keeps it still, and COCKPIT_GITHUB_API, COCKPIT_LINEAR_API and COCKPIT_CIRCLECI_API point it
//! at other hosts.

mod calendar;
mod circleci;
mod dashboard;
pub(crate) mod fresh;
mod github;
mod github_ci;
pub(crate) mod http;
mod ics;
mod linear;
pub(crate) mod mail;
pub(crate) mod mcp;
pub mod merge;
mod news;
mod requests;
mod schedule;
pub(crate) mod upgrade;

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
    /// The code host's server, when the PRs are not GitHub's.
    code_server: Option<cockpit_connectors::Server>,
    /// The MCP servers the hub calls for CI runs and meetings.
    ci_server: Option<cockpit_connectors::Server>,
    calendar_server: Option<cockpit_connectors::Server>,
    /// The platform's server, and the observability one (with its section) for the monitor space when it is pinned.
    platform_server: Option<cockpit_connectors::Server>,
    signals: Option<(cockpit_connectors::Server, Value)>,
    circleci: Option<String>,
    /// The calendar's secret iCal address.
    calendar: Option<String>,
}

impl Plan {
    /// The sources this plan reads, by the names their freshness goes by.
    fn sources(&self) -> Vec<&'static str> {
        let reads = [
            ("github", self.github || self.code_server.is_some()),
            ("tickets", self.tickets_server.is_some() || self.linear.is_some()),
            ("ci", self.github_ci || self.ci_server.is_some() || self.circleci.is_some()),
            ("platform", self.platform_server.is_some()),
            ("calendar", self.calendar_server.is_some() || self.calendar.is_some()),
            // Named, not asked whether it is logged in: that asks the keychain, and this runs every round.
            ("mail", self.config.0["mail"]["enabled"] != false && self.config.0["mail"]["server"].as_str().is_some_and(|s| !s.is_empty())),
            ("monitor", self.platform_server.is_some() && self.signals.is_some()),
        ];
        reads.iter().filter(|(_, on)| *on).map(|(source, _)| *source).collect()
    }
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
        code_server: mcp::server(&raw, c, "code"),
        ci_server: mcp::server(&raw, c, "ci").filter(|_| raw["ci"]["token"].as_str().is_none_or(|t| t.trim().is_empty())),
        calendar_server: mcp::server(&raw, c, "calendar").filter(|_| c["calendar"]["enabled"] == true && calendar::address(&raw["calendar"]["ics"]).is_none()),
        circleci: token("ci", "circleci").filter(|_| !ci_server.is_empty()),
        calendar: calendar::address(&raw["calendar"]["ics"]).filter(|_| c["calendar"]["enabled"] != false),
        platform_server: mcp::server(&raw, c, "platform").filter(|_| c["platform"]["enabled"] == true),
        signals: mcp::observability(&raw).filter(|_| config.space_names().iter().any(|s| s == "monitor")),
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
    mail: Schedule,
    circleci: Schedule,
    calendar: Schedule,
    circle: circleci::CircleCi,
    /// How fresh each source is, for the documents' `hub.fresh`.
    fresh: fresh::Sources,
    /// The latest error of each source, until it fetches again.
    errors: Vec<(&'static str, String)>,
    asked: u64,
}

fn run() {
    // What the documents say of each source, so a restart keeps when each was read and since when one fails.
    let docs: Vec<Value> = fresh::DOCS.iter().map(|d| crate::store::global().get(d)).collect();
    let mut feed = Feed {
        calendar: Schedule::every(calendar::EVERY),
        monitor: Schedule::every(120),
        mail: Schedule::every(120),
        fresh: fresh::Sources::seed(&docs),
        ..Feed::default()
    };
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
    /// Each source's interval in seconds from the config's `refresh` block, at least 15.
    fn pace(&mut self, plan: &Plan) {
        let every = |name: &str, default: u64| plan.config.0["refresh"][name].as_u64().unwrap_or(default).max(15);
        let calendar = if plan.calendar_server.is_some() { 120 } else { calendar::EVERY };
        self.github.set_every(every("github", 30));
        self.linear.set_every(every("tickets", 60));
        self.mcp_tickets.set_every(every("tickets", 60));
        self.mcp_ci.set_every(every("ci", 30));
        self.circleci.set_every(every("ci", 30));
        self.mail.set_every(every("mail", 60));
        self.calendar.set_every(every("calendar", calendar));
        self.mcp_platform.set_every(every("platform", 60));
        self.monitor.set_every(every("monitor", 120));
    }

    fn round(&mut self, cache: &Path, plan: &Plan) {
        self.pace(plan);
        // A source the config no longer reads leaves its chip, its document's error and its inbox error line with it.
        let gone = self.fresh.keep(&plan.sources());
        if !gone.is_empty() {
            self.errors.retain(|(name, _)| !gone.iter().any(|g| note_source(name) == g));
            let lines: Vec<Value> = self.errors.iter().map(|(_, e)| Value::from(e.as_str())).collect();
            crate::store::global().update("inbox", |doc| {
                let mut doc = gone.iter().filter(|g| fresh::doc_of(g) == "inbox").fold(doc.clone(), |doc, g| fresh::without(&doc, Some(g)));
                if doc["hub"]["errors"].is_array() {
                    // The hub's lines are written again from what it still reads; the mod's own stay.
                    let theirs: Vec<Value> = doc["errors"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|e| !doc["hub"]["errors"].as_array().is_some_and(|h| h.contains(e)))
                        .cloned()
                        .collect();
                    doc["errors"] = Value::from(theirs.into_iter().chain(lines.iter().cloned()).collect::<Vec<_>>());
                    doc["hub"]["errors"] = Value::from(lines);
                }
                doc
            });
            for source in gone.iter().filter(|g| fresh::doc_of(g) != "inbox") {
                crate::store::global().update(fresh::doc_of(source), |doc| {
                    let mut doc = fresh::without(doc, Some(source));
                    if let Some(fields) = doc.as_object_mut() {
                        fields.remove("error");
                    }
                    doc
                });
            }
        }
        let asked = asked_at(cache);
        let is_asked = asked > self.asked;
        self.asked = asked;
        let now = now_secs();
        let mut part: Option<InboxPart> = None;
        let mut runs: Option<Result<Vec<Value>, String>> = None;
        if let Some(server) = &plan.code_server
            && self.github.is_due(now, is_asked)
        {
            let name = plan.raw["code"]["name"].as_str().unwrap_or("Code").to_string();
            let fetched = mcp::code(server, &plan.raw["code"]);
            let outcome = fetched.as_ref().map(|_| ()).map_err(Clone::clone);
            let error = self.github.after(&outcome, now);
            self.fresh.record("github", &name, &outcome, now_ms(), self.github.interval(), self.github.next_at());
            self.note("GitHub", error.map(|e| format!("{name} {e}")));
            part = Some(match fetched {
                Ok(fields) => InboxPart { github: Some(fields), ..InboxPart::default() },
                Err(_) => InboxPart::default(),
            });
        } else if plan.github && self.github.is_due(now, is_asked) {
            let fetched = github::token().ok_or_else(|| Failure::Other("no token: run gh auth login, or set GH_TOKEN".into())).and_then(|token| {
                let org = plan.config.0["github"]["org"].as_str().unwrap_or_default();
                let data = github::fetch(&token, org)?;
                Ok((token, data))
            });
            let outcome = fetched.as_ref().map(|_| ()).map_err(Clone::clone);
            let error = self.github.after(&outcome, now);
            self.fresh.record("github", "GitHub", &outcome, now_ms(), self.github.interval(), self.github.next_at());
            if plan.github_ci {
                self.fresh.record("ci", "CI", &outcome, now_ms(), self.github.interval(), self.github.next_at());
            }
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
            let outcome = fetched.as_ref().map(|_| ()).map_err(Clone::clone);
            let error = self.mcp_tickets.after(&outcome, now);
            self.fresh.record("tickets", &name, &outcome, now_ms(), self.mcp_tickets.interval(), self.mcp_tickets.next_at());
            self.note("Tickets", error.map(|e| format!("{name} {e}")));
            part = Some(InboxPart { tickets: fetched.ok(), ..part.unwrap_or_default() });
        } else if let Some(token) = &plan.linear
            && self.linear.is_due(now, is_asked)
        {
            let fetched = linear::fetch(token);
            let outcome = fetched.as_ref().map(|_| ()).map_err(Clone::clone);
            let error = self.linear.after(&outcome, now);
            self.fresh.record("tickets", "Linear", &outcome, now_ms(), self.linear.interval(), self.linear.next_at());
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
            let outcome = fetched.as_ref().map(|_| ()).map_err(Clone::clone);
            let error = self.mcp_ci.after(&outcome, now);
            self.fresh.record("ci", &name, &outcome, now_ms(), self.mcp_ci.interval(), self.mcp_ci.next_at());
            self.note("CI", error.clone().map(|e| format!("{name} {e}")));
            runs = Some(fetched.map_err(|_| format!("{name} {}", error.unwrap_or_default())));
        } else if let Some(token) = &plan.circleci
            && self.circleci.is_due(now, is_asked)
        {
            let fetched = self.circleci_runs(token, &plan.config);
            let outcome = fetched.as_ref().map(|_| ()).map_err(Clone::clone);
            let error = self.circleci.after(&outcome, now);
            let label = plan.config.0["ci"]["name"].as_str().filter(|n| !n.is_empty()).unwrap_or("CircleCI").to_string();
            self.fresh.record("ci", &label, &outcome, now_ms(), self.circleci.interval(), self.circleci.next_at());
            self.note("CircleCI", error.clone());
            runs = Some(fetched.map_err(|_| format!("CircleCI {}", error.unwrap_or_default())));
        }
        if let Some(server) = &plan.calendar_server
            && self.calendar.is_due(now, is_asked)
        {
            let fetched = mcp::meetings(server, &plan.raw["calendar"], &plan.config.0);
            let outcome = fetched.as_ref().map(|_| ()).map_err(Clone::clone);
            let error = self.calendar.after(&outcome, now);
            self.fresh.record("calendar", "Calendar", &outcome, now_ms(), self.calendar.interval(), self.calendar.next_at());
            let fetched = fetched.map_err(|_| format!("Calendar: {}", error.unwrap_or_default()));
            let record = self.fresh.of_doc("meetings", &plan.raw, &plan.config.0, now_ms());
            crate::store::global().update("meetings", |doc| fresh::stamp(&calendar::merge(doc, fetched, now_ms()), record));
        } else if let Some(url) = &plan.calendar
            && self.calendar.is_due(now, is_asked)
        {
            let fetched = calendar::fetch(url, &plan.config);
            let outcome = fetched.as_ref().map(|_| ()).map_err(Clone::clone);
            let error = self.calendar.after(&outcome, now);
            self.fresh.record("calendar", "Calendar", &outcome, now_ms(), self.calendar.interval(), self.calendar.next_at());
            let fetched = fetched.map_err(|_| format!("Calendar: {}", error.unwrap_or_default()));
            let record = self.fresh.of_doc("meetings", &plan.raw, &plan.config.0, now_ms());
            crate::store::global().update("meetings", |doc| fresh::stamp(&calendar::merge(doc, fetched, now_ms()), record));
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
            let outcome = fetched.as_ref().map(|_| ()).map_err(Clone::clone);
            let error = self.mcp_platform.after(&outcome, now);
            self.fresh.record("platform", "Deploys", &outcome, now_ms(), self.mcp_platform.interval(), self.mcp_platform.next_at());
            self.note("Platform", error.map(|e| format!("Deploys and previews: {e}")));
            part = Some(InboxPart { platform: fetched.ok(), ..part.unwrap_or_default() });
        }
        if let (Some(server), Some(signals)) = (&plan.platform_server, &plan.signals)
            && self.monitor.is_due(now, is_asked)
        {
            let before = crate::store::global().get("monitor");
            let inbox = crate::store::global().get("inbox");
            let fetched = mcp::monitor_doc(server, signals, &plan.raw, &plan.config.0, &before, &inbox, now_ms() as i64);
            let outcome = fetched.as_ref().map(|_| ()).map_err(|e| Failure::Other(e.clone()));
            let _ = self.monitor.after(&outcome, now);
            let label = signals.1["name"].as_str().filter(|n| !n.is_empty()).unwrap_or("Monitor").to_string();
            self.fresh.record("monitor", &label, &outcome, now_ms(), self.monitor.interval(), self.monitor.next_at());
            if let Err(message) = &fetched {
                // It reads through two servers: the sign-in goes to the one that failed.
                let platform = plan.raw["platform"]["server"].as_str().unwrap_or_default();
                let failed =
                    if !platform.is_empty() && message.contains(&server.describe()) { platform } else { signals.1["server"].as_str().unwrap_or_default() };
                self.fresh.failed_through("monitor", failed);
            }
            if matches!(fetched, Ok(None)) && self.fresh.forget("monitor") {
                // No deploys mapped, no monitor: nothing to say of it.
                crate::store::global().update("monitor", |doc| fresh::without(doc, Some("monitor")));
            }
            let record = self.fresh.of_doc("monitor", &plan.raw, &plan.config.0, now_ms());
            if fetched.is_err() {
                // The numbers stay as they were; the record says they could not be read again, even before any came.
                crate::store::global().update("monitor", |doc| fresh::stamp(doc, record.clone()));
            }
            if let Ok(Some(mut doc)) = fetched {
                doc["by"] = "hub".into();
                let doc = fresh::stamp(&doc, record);
                crate::store::global().update("monitor", |_| doc.clone());
                let notify = plan.config.0["monitor"]["notify"].as_str().unwrap_or("mine");
                for n in cockpit_connectors::monitor::changes(&before, &doc, notify) {
                    news::notify(&plan.config, &n.title, &n.body, &n.action, &[]);
                }
            }
        }
        if self.mail.is_due(now, is_asked)
            && let Some(fetched) = mail::fetch(&plan.raw, &plan.config.0)
        {
            let outcome = fetched.as_ref().map(|_| ()).map_err(|e| Failure::Other(e.clone()));
            let error = self.mail.after(&outcome, now);
            let label = plan.config.0["mail"]["name"].as_str().filter(|n| !n.is_empty()).unwrap_or("Mail").to_string();
            self.fresh.record("mail", &label, &outcome, now_ms(), self.mail.interval(), self.mail.next_at());
            let record = self.fresh.of_doc("mail", &plan.raw, &plan.config.0, now_ms());
            let fetched = fetched.map_err(|_| format!("Mail: {}", error.unwrap_or_default()));
            let (before, after) = crate::store::global().update("mail", |doc| fresh::stamp(&mail::merge(doc, fetched, now_ms()), record));
            for (title, body, action) in mail::news(&before, &after) {
                news::notify(&plan.config, &title, &body, &action, &[]);
            }
            dashboard::report(&plan.config);
        }
        warn_meetings(&plan.config, now_ms());
        upgrade::check(&plan.config, now_ms());
        requests::take(cache, &plan.config, &plan.raw);
        if let Some(mut part) = part {
            part.errors = self.errors.iter().map(|(_, e)| e.clone()).collect();
            part.fresh = self.fresh.of_doc("inbox", &plan.raw, &plan.config.0, now_ms());
            write_inbox(&plan.config, &part);
            dashboard::report(&plan.config);
        }
        if let Some(runs) = runs {
            write_ci(&plan.config, runs, self.fresh.of_doc("ci", &plan.raw, &plan.config.0, now_ms()));
        }
    }

    fn note(&mut self, source: &'static str, error: Option<String>) {
        debug_assert!(!note_source(source).is_empty());
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

/// The source an error line's name stands for (`note`'s names).
fn note_source(name: &str) -> &'static str {
    match name {
        "GitHub" => "github",
        "Tickets" | "Linear" => "tickets",
        "CI" | "CircleCI" => "ci",
        "Platform" => "platform",
        _ => "",
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

fn write_ci(config: &Config, runs: Result<Vec<Value>, String>, record: Value) {
    let (before, after) = crate::store::global().update("ci", |doc| fresh::stamp(&merge::merge_ci(doc, runs, now_ms()), record));
    if before["version"].as_i64() == Some(merge::CI_VERSION) && after["error"].is_null() {
        let inbox = crate::store::global().get("inbox");
        news::announce_ci(config, &news::ci_changes(&before, &after, &inbox));
    }
}
