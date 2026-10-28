//! The hub's own feed: GitHub every minute (the inbox's PR fields, and ci.json when the CI is GitHub's checks), and
//! Linear tickets or CircleCI runs when the config gives `tickets.token` or `ci.token`, so the inbox stays fresh with no
//! Claude session. A touch of refresh-request fetches at once; errors back off, rate limits wait for their reset.
//! COCKPIT_HUB_FEED=off keeps it still (the hub's tests), and the APIs' base URLs follow COCKPIT_GITHUB_API,
//! COCKPIT_LINEAR_API and COCKPIT_CIRCLECI_API.

mod circleci;
mod github;
mod github_ci;
mod http;
mod linear;
mod news;
mod schedule;
mod store;

use std::path::Path;
use std::sync::Once;
use std::time::Duration;

use cockpit_core::files;
use serde_json::Value;

use crate::config::{self, Config};
use http::{Failure, Fetched};
use schedule::Schedule;
use store::InboxPart;

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
    github: bool,
    github_ci: bool,
    linear: Option<String>,
    circleci: Option<String>,
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
        circleci: token("ci", "circleci").filter(|_| !ci_server.is_empty()),
        config,
    }
}

#[derive(Default)]
struct Feed {
    github: Schedule,
    linear: Schedule,
    circleci: Schedule,
    circle: circleci::CircleCi,
    /// The latest error of each source, until it fetches again.
    errors: Vec<(&'static str, String)>,
    /// Parts not written yet because the mod held the lock.
    inbox: Option<InboxPart>,
    ci: Option<Result<Vec<Value>, String>>,
    asked: u64,
}

fn run() {
    let mut feed = Feed::default();
    loop {
        feed.round(&files::cache(), &plan());
        std::thread::sleep(TICK);
    }
}

/// When the inbox app (or the mod after a request) last asked for fresh data.
fn asked_at(cache: &Path) -> u64 {
    std::fs::metadata(cache.join("refresh-request")).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64).unwrap_or(0)
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
            let prev_ci = files::load(&cache.join("ci.json")).unwrap_or(Value::Null);
            let image_job = plan.config.0["ci"]["imageJob"].as_str().unwrap_or_default().to_string();
            match fetched {
                Ok((token, data)) => {
                    if plan.github_ci {
                        runs = Some(Ok(github_ci::runs(&data.mine_nodes, &prev_ci, &image_job, &mut |slug, job| github::failed_step(&token, slug, job))));
                    }
                    part = Some(InboxPart { github: Some(store::github_fields(&data)), ..InboxPart::default() });
                }
                Err(_) => {
                    if plan.github_ci {
                        runs = Some(Err(format!("GitHub {}", error.unwrap_or_default())));
                    }
                    part = Some(InboxPart::default());
                }
            }
        }
        if let Some(token) = &plan.linear
            && self.linear.is_due(now, is_asked)
        {
            let fetched = linear::fetch(token);
            let error = self.linear.after(&fetched.as_ref().map(|_| ()).map_err(Clone::clone), now);
            self.note("Linear", error);
            part = Some(InboxPart { tickets: fetched.ok(), ..part.unwrap_or_default() });
        }
        if let Some(token) = &plan.circleci
            && self.circleci.is_due(now, is_asked)
        {
            let fetched = self.circleci_runs(token, &plan.config);
            let error = self.circleci.after(&fetched.as_ref().map(|_| ()).map_err(Clone::clone), now);
            self.note("CircleCI", error.clone());
            runs = Some(fetched.map_err(|_| format!("CircleCI {}", error.unwrap_or_default())));
        }
        if let Some(mut part) = part {
            part.errors = self.errors.iter().map(|(_, e)| e.clone()).collect();
            self.inbox = Some(match self.inbox.take() {
                Some(pending) => pending.then(part),
                None => part,
            });
        }
        if runs.is_some() {
            self.ci = runs;
        }
        self.write(cache, &plan.config);
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

    /// Writes what is pending under each file's lock, then announces what changed; a held lock leaves it for next round.
    fn write(&mut self, cache: &Path, config: &Config) {
        if let Some(part) = self.inbox.take() {
            match store::update(cache, "inbox", |file| store::merge_inbox(file, &part, now_ms())) {
                Some((before, after)) => {
                    let mut changes = Vec::new();
                    if part.github.is_some() && store::has_github(&before) {
                        changes.extend(news::pr_changes(&before, &after));
                    }
                    if part.tickets.is_some() && before["version"].as_i64() == Some(store::INBOX_VERSION) {
                        changes.extend(news::ticket_changes(&before, &after));
                    }
                    news::announce_inbox(config, &changes);
                }
                None => self.inbox = Some(part),
            }
        }
        if let Some(runs) = self.ci.take() {
            match store::update(cache, "ci", |file| store::merge_ci(file, runs.clone(), now_ms())) {
                Some((before, after)) => {
                    if before["version"].as_i64() == Some(store::CI_VERSION) && after["error"].is_null() {
                        let inbox = files::load(&cache.join("inbox.json")).unwrap_or(Value::Null);
                        news::announce_ci(config, &news::ci_changes(&before, &after, &inbox));
                    }
                }
                None => self.ci = Some(runs),
            }
        }
    }
}
