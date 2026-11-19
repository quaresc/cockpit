//! The `routines` document the hub writes, read into the routines, what runs and waits, and each one's past runs; the
//! clock that says "in 2 h" and "12 min ago".

use std::collections::HashMap;

use chrono::{FixedOffset, NaiveDateTime, Utc};
use cockpit_core::routines::Zone;
use serde::Deserialize;
use serde_json::Value;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Routine {
    pub slug: String,
    pub name: String,
    pub path: String,
    pub when: String,
    pub when_text: String,
    /// manual, every, schedule, event or broken.
    pub trigger: String,
    pub error: Option<String>,
    /// agent or shell.
    pub run: String,
    /// The agent that runs it, its name, and the one its file names (empty for the default).
    pub agent: Option<String>,
    pub agent_name: Option<String>,
    pub agent_set: String,
    pub cwd: String,
    pub model: String,
    /// The models its agent takes, its default first (""): the hub's list, empty for a script.
    pub models: Vec<String>,
    pub tools: Vec<String>,
    pub mode: String,
    /// Dollars a week, and what its runs of the last seven days cost, as Claude Code estimates them.
    pub budget: Option<f64>,
    pub spent_week: f64,
    pub last_cost: Option<f64>,
    /// Its week's budget is spent: runs wait.
    pub over: bool,
    /// What its file lacks for a lightest run.
    pub problems: Vec<String>,
    pub paused: bool,
    pub prompt: String,
    pub next: Option<u64>,
    /// Its next three runs on its own.
    pub next_runs: Vec<u64>,
    pub notify: String,
    /// Each run in a worktree of its own.
    pub worktree: bool,
    /// Minutes a run missed while the hub was down may still start late.
    pub catch_up: Option<u64>,
    /// The shell command run first, which skips the run when it exits non-zero.
    pub precheck: String,
    /// `tz` as its file says it, and the time zone it names (empty for the machine's).
    pub tz: String,
    pub zone: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Running {
    pub id: String,
    pub started: u64,
    pub why: String,
    pub session: String,
    pub stopping: Option<u64>,
    /// "precheck" while its precheck runs, before the run itself.
    pub phase: String,
}

impl Running {
    pub fn is_checking(&self) -> bool {
        self.phase == "precheck"
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Run {
    pub id: String,
    pub started: u64,
    pub ended: u64,
    pub ok: bool,
    pub code: Option<i64>,
    pub why: String,
    pub session: Option<String>,
    pub cost: Option<f64>,
    pub summary: String,
    pub stopped: bool,
    pub timed_out: bool,
    /// Its time passed while the hub was down, too long ago to run late.
    pub missed: bool,
    /// It ran late, after the hub came back.
    pub late: bool,
    /// Seen in the run inbox, or opened.
    pub read: bool,
    pub worktree: Option<Worktree>,
    /// What skipped it ("precheck"): nothing was to do.
    pub skipped: Option<String>,
    /// How it went wrong, as a word, and in the plain sentence every surface shows.
    pub kind: Option<String>,
    pub reason: Option<String>,
    /// Skips in a row it stands for, the first one's start, and whether its precheck passed before it ran.
    pub times: Option<u64>,
    pub since: Option<u64>,
    pub checked: bool,
}

/// The git worktree a run worked in: kept while it holds commits or changes, removed otherwise.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Worktree {
    pub path: String,
    pub branch: String,
    pub commits: u64,
    pub dirty: bool,
    pub removed: bool,
}

impl Worktree {
    pub fn kept(&self) -> bool {
        !self.removed && !self.path.is_empty()
    }

    /// "auto/fix-lint-17… · 2 commits", "… · uncommitted changes".
    pub fn describe(&self) -> String {
        let mut what = Vec::new();
        if self.commits > 0 {
            what.push(if self.commits == 1 { "1 commit".to_string() } else { format!("{} commits", self.commits) });
        }
        if self.dirty {
            what.push("uncommitted changes".into());
        }
        if what.is_empty() { self.branch.clone() } else { format!("{} · {}", self.branch, what.join(", ")) }
    }
}

impl Run {
    pub fn session(&self) -> Option<&str> {
        self.session.as_deref().filter(|s| !s.is_empty())
    }

    /// How it ended, in a word: ok, skipped, failed, missed, stopped.
    pub fn word(&self) -> &'static str {
        if self.skipped.is_some() {
            "skipped"
        } else if self.ok {
            "ok"
        } else if self.missed {
            "missed"
        } else if self.stopped {
            "stopped"
        } else {
            "failed"
        }
    }

    pub fn took(&self) -> String {
        duration(self.ended.saturating_sub(self.started))
    }

    /// The sentence saying how it went wrong or why it skipped.
    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref().filter(|r| !r.is_empty())
    }

    /// "12 times since 08:00": the skips in a row it stands for.
    pub fn repeats(&self, clock: &Clock) -> Option<String> {
        let times = self.times.filter(|t| *t > 1)?;
        Some(format!("{times} times since {}", clock.when(self.since.unwrap_or(self.started))))
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct Queued {
    slug: String,
    /// Asked for by hand: it does not wait for the machine to settle.
    hand: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Raw {
    version: u64,
    at: Option<u64>,
    inbox_from: Option<u64>,
    routines: Vec<Routine>,
    running: HashMap<String, Running>,
    queued: Vec<Queued>,
    runs: HashMap<String, Vec<Run>>,
    agents: Agents,
    settling: bool,
}

/// The agents a routine can run on, as the hub has them: the default first, then the others installed.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Agents {
    /// The default agent's id; empty from a hub that predates agents, whose routines all ran Claude Code.
    pub default: String,
    /// {id, name, models} of each agent installed that runs headless, `models` its default first ("").
    pub installed: Vec<Value>,
}

impl Agents {
    /// The default agent's name and whether it is Claude Code.
    pub fn default_agent(&self) -> (String, bool) {
        if self.default.is_empty() {
            return ("Claude Code".into(), true);
        }
        let name = self.installed.iter().find(|a| a["id"] == self.default.as_str()).and_then(|a| a["name"].as_str()).unwrap_or(&self.default);
        (name.to_string(), self.default == "claude")
    }

    /// The models the default agent takes, its default first; none from a hub that lists none, as the app's form has.
    pub fn default_models(&self) -> Vec<String> {
        self.models_for("")
    }

    /// The selected agent's models, using the default when its id is empty.
    pub fn models_for(&self, id: &str) -> Vec<String> {
        let id = if id.is_empty() { self.default.as_str() } else { id };
        let listed = self.installed.iter().find(|a| a["id"] == id).and_then(|a| a["models"].as_array());
        match listed {
            Some(list) => list.iter().filter_map(|m| m.as_str().map(str::to_string)).collect(),
            None => Vec::new(),
        }
    }
}

/// How a routine stands, for its glyph and colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Running,
    Waiting,
    Broken,
    Over,
    Paused,
    Failed,
    /// Its last run was stopped by hand: nothing to look at.
    Stopped,
    Missed,
    Skipped,
    Ok,
    Never,
}

#[derive(Clone, Debug, Default)]
pub struct Doc {
    pub at: Option<u64>,
    /// Runs that ended after this and are not read show in the run inbox; none before the hub first set it.
    pub inbox_from: u64,
    pub routines: Vec<Routine>,
    pub running: HashMap<String, Running>,
    pub queued: Vec<String>,
    pub runs: HashMap<String, Vec<Run>>,
    pub agents: Agents,
    /// The machine just woke: the runs it queued on their own wait until it settles.
    pub settling: bool,
    /// The routines whose queued run waits for the machine to settle, not for a free slot.
    pub held: Vec<String>,
}

impl Doc {
    /// The document as the hub wrote it; None for another shape.
    pub fn parse(value: &Value) -> Option<Doc> {
        let raw: Raw = serde_json::from_value(value.clone()).ok()?;
        (raw.version == 1).then(|| Doc {
            at: raw.at,
            inbox_from: raw.inbox_from.unwrap_or(u64::MAX),
            routines: raw.routines,
            running: raw.running,
            held: raw.queued.iter().filter(|q| raw.settling && !q.hand).map(|q| q.slug.clone()).collect(),
            queued: raw.queued.into_iter().map(|q| q.slug).collect(),
            runs: raw.runs,
            agents: raw.agents,
            settling: raw.settling,
        })
    }

    pub fn routine(&self, slug: &str) -> Option<&Routine> {
        self.routines.iter().find(|r| r.slug == slug)
    }

    pub fn runs(&self, slug: &str) -> &[Run] {
        self.runs.get(slug).map(Vec::as_slice).unwrap_or_default()
    }

    pub fn last(&self, slug: &str) -> Option<&Run> {
        self.runs(slug).first()
    }

    pub fn run(&self, slug: &str, id: &str) -> Option<&Run> {
        self.runs(slug).iter().find(|r| r.id == id)
    }

    pub fn is_unread(&self, run: &Run) -> bool {
        !run.read && run.ended > self.inbox_from
    }

    /// The runs not read yet, newest first, with their routine's slug.
    pub fn unread(&self) -> Vec<(&str, &Run)> {
        let mut all: Vec<(&str, &Run)> =
            self.routines.iter().flat_map(|r| self.runs(&r.slug).iter().filter(|run| self.is_unread(run)).map(move |run| (r.slug.as_str(), run))).collect();
        all.sort_by_key(|(_, run)| std::cmp::Reverse(run.ended));
        all
    }

    pub fn state(&self, r: &Routine) -> State {
        if self.running.contains_key(&r.slug) {
            State::Running
        } else if self.queued.contains(&r.slug) {
            State::Waiting
        } else if r.error.is_some() {
            State::Broken
        } else if r.paused {
            State::Paused
        } else if r.over {
            State::Over
        } else {
            match self.last(&r.slug) {
                Some(run) if run.skipped.is_some() => State::Skipped,
                Some(run) if run.ok => State::Ok,
                Some(run) if run.missed => State::Missed,
                Some(run) if run.stopped => State::Stopped,
                Some(_) => State::Failed,
                None => State::Never,
            }
        }
    }
}

/// "38 s", "4 min", "1 h 05".
pub fn duration(ms: u64) -> String {
    let s = ms / 1000;
    match s {
        0..60 => format!("{s} s"),
        60..3600 => format!("{} min", s / 60),
        _ => format!("{} h {:02}", s / 3600, s / 60 % 60),
    }
}

/// "$0.12", "$1.40".
pub fn dollars(usd: f64) -> String {
    format!("${usd:.2}")
}

/// A cost Claude estimated, not billed on a Claude plan: "~$0.07", "~$0.004".
pub fn estimate(usd: f64) -> String {
    if usd > 0.0 && usd < 0.01 { format!("~${usd:.3}") } else { format!("~${usd:.2}") }
}

#[derive(Clone, Copy, Debug)]
pub struct Clock {
    pub now: u64,
    pub zone: Option<FixedOffset>,
}

impl Clock {
    pub fn system() -> Self {
        Self { now: Utc::now().timestamp_millis() as u64, zone: None }
    }

    /// The clock times show on: the fixed offset tests give, else the machine's.
    pub fn zone(&self) -> Zone {
        self.zone.map_or(Zone::Local, Zone::Fixed)
    }

    /// The clock a routine's `tz` reads its times on: the zone it names, else the one times show on.
    pub fn of(&self, tz: &str) -> Result<Zone, String> {
        Ok(match Zone::parse(tz)? {
            Zone::Local => self.zone(),
            zone => zone,
        })
    }

    pub fn local(&self, ms: u64) -> NaiveDateTime {
        self.zone().local(ms)
    }

    /// "09:00" today, "Thu 09:00" this week, "3 Nov 09:00" later.
    pub fn when(&self, ms: u64) -> String {
        let (at, now) = (self.local(ms), self.local(self.now));
        let days = (at.date() - now.date()).num_days();
        if days == 0 {
            at.format("%H:%M").to_string()
        } else if (-6..=6).contains(&days) {
            at.format("%a %H:%M").to_string()
        } else {
            at.format("%-d %b %H:%M").to_string()
        }
    }

    /// "now", "12 min ago", "2 h ago", "3 d ago".
    pub fn ago(&self, ms: u64) -> String {
        let mins = self.now.saturating_sub(ms) / 60_000;
        match mins {
            0 => "now".into(),
            1..60 => format!("{mins} min ago"),
            60..2880 => format!("{} h ago", mins / 60),
            _ => format!("{} d ago", mins / 1440),
        }
    }

    /// "in 4 min", "in 2 h", "09:00 tomorrow" as "Fri 09:00".
    pub fn until(&self, ms: u64) -> String {
        let mins = ms.saturating_sub(self.now).div_ceil(60_000);
        match mins {
            0 => "now".into(),
            1..60 => format!("in {mins} min"),
            _ => self.when(ms),
        }
    }

    pub fn second(&self) -> u64 {
        self.now / 1000
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::DateTime;
    use serde_json::json;

    #[test]
    fn the_document_reads_with_each_routines_state() {
        let doc = Doc::parse(&json!({"version": 1, "routines": [
            {"slug": "a", "name": "A"}, {"slug": "b", "name": "B"}, {"slug": "c", "name": "C", "paused": true},
            {"slug": "d", "name": "D", "error": "when?"}, {"slug": "e", "name": "E"}, {"slug": "f", "name": "F"}],
            "running": {"a": {"id": "1", "started": 5}}, "queued": [{"slug": "b"}],
            "runs": {"e": [{"id": "2", "ok": false}], "f": [{"id": "3", "ok": true, "started": 1000, "ended": 39_000}]}}))
        .unwrap();
        let states: Vec<State> = doc.routines.iter().map(|r| doc.state(r)).collect();
        assert_eq!(states, [State::Running, State::Waiting, State::Paused, State::Broken, State::Failed, State::Ok]);
        let skipped = Doc::parse(&json!({"version": 1, "routines": [{"slug": "s"}], "runs": {"s": [
            {"id": "4", "ok": true, "skipped": "precheck", "summary": "no PR waits", "reason": "Skipped: its precheck found nothing to do (exit 1).",
             "times": 3, "since": 1_000, "started": 9_000}]}}))
        .unwrap();
        let run = skipped.last("s").unwrap();
        assert_eq!(
            (skipped.state(&skipped.routines[0]), run.word(), run.reason()),
            (State::Skipped, "skipped", Some("Skipped: its precheck found nothing to do (exit 1)."))
        );
        assert_eq!(doc.last("f").unwrap().took(), "38 s");
        assert!(Doc::parse(&json!({"version": 2})).is_none());
    }

    #[test]
    fn an_agent_the_hub_lists_no_models_for_offers_none_as_in_the_app() {
        let none = Agents { default: "claude".into(), installed: vec![] };
        assert!(none.default_models().is_empty() && none.models_for("claude").is_empty());
        let listed = Agents { default: "claude".into(), installed: vec![serde_json::json!({"id": "claude", "models": ["", "opus"]})] };
        assert_eq!(listed.default_models(), ["", "opus"]);
    }

    #[test]
    fn times_read_short() {
        let now = DateTime::parse_from_rfc3339("2026-10-29T10:00:00+01:00").unwrap();
        let clock = Clock { now: now.timestamp_millis() as u64, zone: Some(*now.offset()) };
        let at = |s: &str| DateTime::parse_from_rfc3339(s).unwrap().timestamp_millis() as u64;
        assert_eq!(clock.until(at("2026-10-29T10:03:10+01:00")), "in 4 min");
        assert_eq!(clock.until(at("2026-10-29T17:30:00+01:00")), "17:30");
        assert_eq!(clock.until(at("2026-10-30T09:00:00+01:00")), "Fri 09:00");
        assert_eq!(clock.until(at("2026-11-12T09:00:00+01:00")), "12 Nov 09:00");
        assert_eq!(clock.ago(at("2026-10-29T08:00:00+01:00")), "2 h ago");
        assert_eq!((duration(4 * 60_000 + 5000), duration(3_900_000), dollars(0.123)), ("4 min".into(), "1 h 05".into(), "$0.12".into()));
    }
}
