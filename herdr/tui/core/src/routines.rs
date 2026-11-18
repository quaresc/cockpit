//! Routines: an agent's prompt or a shell script in ~/.config/cockpit/routines/<slug>.md that the hub runs on its own,
//! on a schedule, every so often or when something happens. A file is a front matter of `key: value` lines, then the
//! prompt (or the script); `when` says when it runs, in words or as cron, on the machine's clock or in `tz`.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use chrono::{Datelike, Duration, FixedOffset, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Timelike};
use chrono_tz::Tz;
use serde_json::Value;

use crate::agent::Agent;
use crate::files;

pub fn dir() -> PathBuf {
    files::config_dir().join("routines")
}

/// What `on <trigger>` listens to: the trigger's word, the hub's event kinds it covers, and how it reads.
pub const TRIGGERS: [(&str, &[&str], &str); 16] = [
    ("review", &["review_requested", "team_review"], "a review requested"),
    ("approved", &["approved"], "a PR of mine approved"),
    ("changes", &["changes_requested"], "changes requested on a PR of mine"),
    ("opened", &["pr_opened"], "a PR of mine opened"),
    ("merged", &["merged"], "a PR of mine merged"),
    ("ci-failed", &["ci_failing", "ci_failed"], "CI failing"),
    ("ci-passed", &["ci_passing", "ci_passed"], "CI passing"),
    ("ticket", &["ticket_moved", "ticket_assigned", "ticket_left"], "a ticket moving"),
    ("assigned", &["ticket_assigned"], "a ticket assigned to me"),
    ("live", &["live"], "a PR of mine live"),
    ("release", &["released"], "a release"),
    ("release-ok", &["release_ok"], "a release that looks fine"),
    ("release-moved", &["release_moved"], "a release that moved"),
    ("release-failed", &["release_failed"], "a release that failed"),
    ("alert", &["alert"], "a monitor alerting"),
    ("error", &["new_error"], "a new error"),
];

const DAYS: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];

/// When a routine runs.
#[derive(Clone, Debug, PartialEq)]
pub enum When {
    /// Only when asked.
    Manual,
    /// Seconds after the last run.
    Every(u64),
    /// At the times the crons give, local time.
    Cron(Vec<Cron>),
    /// On each event of these kinds whose title holds every word.
    On { trigger: String, kinds: Vec<String>, words: Vec<String> },
}

/// A cron schedule: each field the set of values it allows, as bits.
#[derive(Clone, Debug, PartialEq)]
pub struct Cron {
    minutes: u64,
    hours: u32,
    days: u32,
    months: u16,
    weekdays: u8,
    any_day: bool,
    any_weekday: bool,
}

/// `a`, `a-b`, `*`, each with an optional `/step`, separated by commas: the bits of the values between `min` and `max`.
fn field(text: &str, min: u32, max: u32) -> Result<u64, String> {
    let mut bits = 0u64;
    for part in text.split(',') {
        let (range, step) = match part.split_once('/') {
            Some((range, step)) => (range, step.parse::<u32>().ok().filter(|s| *s > 0).ok_or_else(|| format!("bad step in {part:?}"))?),
            None => (part, 1),
        };
        let number = |s: &str| s.parse::<u32>().ok().filter(|n| (min..=max).contains(n)).ok_or_else(|| format!("{s:?} is not between {min} and {max}"));
        let (from, to) = match range {
            "*" => (min, max),
            _ => match range.split_once('-') {
                Some((a, b)) => (number(a)?, number(b)?),
                None if step > 1 => (number(range)?, max),
                None => (number(range)?, number(range)?),
            },
        };
        if from > to {
            return Err(format!("{part:?} runs backwards"));
        }
        for n in (from..=to).step_by(step as usize) {
            bits |= 1 << n;
        }
    }
    Ok(bits)
}

impl Cron {
    /// Five fields: minute, hour, day of month, month, day of week (0 or 7 is Sunday).
    pub fn parse(text: &str) -> Result<Cron, String> {
        let fields: Vec<&str> = text.split_whitespace().collect();
        let [minute, hour, day, month, weekday] = fields[..] else {
            return Err("cron takes five fields: minute hour day month weekday".into());
        };
        let mut weekdays = field(weekday, 0, 7)?;
        if weekdays & (1 << 7) != 0 {
            weekdays = (weekdays | 1) & 0x7f;
        }
        Ok(Cron {
            minutes: field(minute, 0, 59)?,
            hours: field(hour, 0, 23)? as u32,
            days: field(day, 1, 31)? as u32,
            months: field(month, 1, 12)? as u16,
            weekdays: weekdays as u8,
            any_day: day == "*",
            any_weekday: weekday == "*",
        })
    }

    /// At these minutes of the day, on these days of the week (Sunday first).
    fn at(times: &[(u32, u32)], weekdays: u8) -> Vec<Cron> {
        times
            .iter()
            .map(|(h, m)| Cron { minutes: 1 << m, hours: 1 << h, days: u32::MAX, months: u16::MAX, weekdays, any_day: true, any_weekday: weekdays == 0x7f })
            .collect()
    }

    /// Cron's rule: when both the day of month and of week are given, either one will do.
    fn on_day(&self, date: NaiveDate) -> bool {
        let day = self.days & (1 << date.day()) != 0;
        let weekday = self.weekdays & (1 << date.weekday().num_days_from_sunday()) != 0;
        let day_ok = match (self.any_day, self.any_weekday) {
            (true, true) => true,
            (true, false) => weekday,
            (false, true) => day,
            (false, false) => day || weekday,
        };
        day_ok && self.months & (1 << date.month()) != 0
    }

    /// The first time it fires strictly after `after`, within the next eight years.
    pub fn next_after(&self, after: NaiveDateTime) -> Option<NaiveDateTime> {
        let start = after.with_second(0)?.with_nanosecond(0)? + Duration::minutes(1);
        let mut date = start.date();
        for _ in 0..(8 * 366) {
            if self.on_day(date) {
                let first_hour = if date == start.date() { start.hour() } else { 0 };
                for hour in (first_hour..24).filter(|h| self.hours & (1 << h) != 0) {
                    let first_minute = if date == start.date() && hour == start.hour() { start.minute() } else { 0 };
                    if let Some(minute) = (first_minute..60).find(|m| self.minutes & (1 << m) != 0) {
                        return Some(date.and_time(NaiveTime::from_hms_opt(hour, minute, 0)?));
                    }
                }
            }
            date = date.succ_opt()?;
        }
        None
    }
}

/// "09:00", "9:30", "17h": hours and minutes.
fn clock(text: &str) -> Option<(u32, u32)> {
    let (h, m) = text.split_once(':').or_else(|| text.strip_suffix('h').map(|h| (h, "0")))?;
    let (h, m) = (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?);
    (h < 24 && m < 60).then_some((h, m))
}

/// "mon", "Monday", "mon-fri", "mon,thu": the days as bits, Sunday first.
fn weekdays(text: &str) -> Option<u8> {
    let day = |name: &str| {
        let name = name.to_lowercase();
        DAYS.iter().position(|d| name.len() >= 3 && (d.starts_with(&name) || name.starts_with(d)))
    };
    let mut bits = 0u8;
    for part in text.split(',').filter(|p| !p.is_empty()) {
        match part.split_once('-') {
            Some((a, b)) => {
                let (a, b) = (day(a)?, day(b)?);
                let mut d = a;
                loop {
                    bits |= 1 << d;
                    if d == b {
                        break;
                    }
                    d = (d + 1) % 7;
                }
            }
            None => bits |= 1 << day(part)?,
        }
    }
    (bits != 0).then_some(bits)
}

/// "30m", "30 min", "2h", "1 day": seconds.
fn interval(text: &str) -> Option<u64> {
    let text = text.replace(' ', "");
    let split = text.find(|c: char| !c.is_ascii_digit()).unwrap_or(text.len());
    let (n, unit) = text.split_at(split);
    let n: u64 = if n.is_empty() { 1 } else { n.parse().ok()? };
    let unit = match unit {
        "m" | "min" | "mins" | "minute" | "minutes" => 60,
        "h" | "hour" | "hours" => 3600,
        "d" | "day" | "days" => 86_400,
        _ => return None,
    };
    Some(n * unit).filter(|s| *s >= 60)
}

impl When {
    /// `manual`, `every 30m`, `hourly`, `daily 09:00`, `weekdays 9:00,17:30`, `mon,thu 14:00`, `cron 0 9 * * 1-5`, or
    /// `on <trigger> [words]`; empty is manual.
    pub fn parse(text: &str) -> Result<When, String> {
        let text = text.trim();
        let lower = text.to_lowercase();
        let (head, rest) = lower.split_once(char::is_whitespace).map_or((lower.as_str(), ""), |(h, r)| (h, r.trim()));
        let times = |rest: &str| -> Result<Vec<(u32, u32)>, String> {
            let times: Option<Vec<_>> = rest.split([',', ' ']).filter(|t| !t.is_empty() && *t != "at").map(clock).collect();
            times.filter(|t| !t.is_empty()).ok_or_else(|| format!("{rest:?} is not a time like 09:00"))
        };
        match head {
            "" | "manual" | "never" => Ok(When::Manual),
            "hourly" => Ok(When::Every(3600)),
            "every" => interval(rest).map(When::Every).ok_or_else(|| format!("{rest:?} is not a duration like 30m, 2h or 1d (a minute at least)")),
            "daily" | "at" => Ok(When::Cron(Cron::at(&times(rest)?, 0x7f))),
            "weekdays" => Ok(When::Cron(Cron::at(&times(rest)?, 0b0111110))),
            "weekends" => Ok(When::Cron(Cron::at(&times(rest)?, 0b1000001))),
            "cron" => Ok(When::Cron(vec![Cron::parse(&text[4..])?])),
            "on" => {
                let mut words = rest.split_whitespace();
                let trigger = words.next().unwrap_or_default();
                let found = if trigger == "any" {
                    Some(TRIGGERS.iter().flat_map(|(_, kinds, _)| kinds.iter().map(|k| k.to_string())).collect())
                } else {
                    TRIGGERS.iter().find(|(w, _, _)| *w == trigger).map(|(_, kinds, _)| kinds.iter().map(|k| k.to_string()).collect())
                };
                let Some(kinds) = found else {
                    let known: Vec<&str> = TRIGGERS.iter().map(|(w, _, _)| *w).collect();
                    return Err(format!("on what? one of any, {}", known.join(", ")));
                };
                Ok(When::On { trigger: trigger.into(), kinds, words: words.map(str::to_string).collect() })
            }
            days => match weekdays(days) {
                Some(bits) => Ok(When::Cron(Cron::at(&times(rest)?, bits))),
                None => Err("when? manual, every 30m, daily 09:00, weekdays 09:00, mon,thu 14:00, cron 0 9 * * 1-5 or on ci-failed".into()),
            },
        }
    }

    /// How it reads in the app: "every 30 min", "weekdays at 09:00", "on CI failing · api".
    pub fn describe(&self, text: &str) -> String {
        match self {
            When::Manual => "by hand".into(),
            When::Every(secs) => match secs {
                s if s % 86_400 == 0 => plural(s / 86_400, "day"),
                s if s % 3600 == 0 => {
                    if *s == 3600 {
                        "every hour".into()
                    } else {
                        format!("every {} h", s / 3600)
                    }
                }
                s => format!("every {} min", s / 60),
            },
            When::Cron(_) => {
                let lower = text.trim().to_lowercase();
                let (head, rest) = lower.split_once(char::is_whitespace).map_or((lower.as_str(), ""), |(h, r)| (h, r.trim()));
                let times = rest.split([',', ' ']).filter(|t| !t.is_empty() && *t != "at").filter_map(clock).map(|(h, m)| format!("{h:02}:{m:02}"));
                let times = times.collect::<Vec<_>>().join(", ");
                match head {
                    "cron" => lower,
                    "daily" | "at" => format!("daily at {times}"),
                    "weekdays" | "weekends" => format!("{head} at {times}"),
                    days => format!("{} at {times}", days.split(',').map(capital).collect::<Vec<_>>().join(", ")),
                }
            }
            When::On { trigger, words, .. } => {
                let what = TRIGGERS.iter().find(|(w, _, _)| w == trigger).map_or("anything happening", |(_, _, what)| what);
                if words.is_empty() { format!("on {what}") } else { format!("on {what} · {}", words.join(" ")) }
            }
        }
    }

    /// Whether one of the hub's events, {kind, title}, sets it off.
    pub fn is_set_off_by(&self, event: &Value) -> bool {
        let When::On { kinds, words, .. } = self else { return false };
        let kind = event["kind"].as_str().unwrap_or_default();
        let title = event["title"].as_str().unwrap_or_default().to_lowercase();
        kinds.iter().any(|k| k == kind) && words.iter().all(|w| title.contains(w.as_str()))
    }

    /// The first time it runs on its own after `after`, local time: None for one run by hand or by an event, and for an
    /// interval, which counts from its last run.
    pub fn next_after(&self, after: NaiveDateTime) -> Option<NaiveDateTime> {
        let When::Cron(crons) = self else { return None };
        crons.iter().filter_map(|c| c.next_after(after)).min()
    }

    /// The last time it was due in (after, now], when it was.
    pub fn last_due(&self, after: NaiveDateTime, now: NaiveDateTime) -> Option<NaiveDateTime> {
        let mut last = None;
        let mut at = after;
        // A minute-by-minute cron over a week asleep is ten thousand steps: past that, the hub would rather skip it.
        for _ in 0..20_000 {
            match self.next_after(at) {
                Some(next) if next <= now => {
                    last = Some(next);
                    at = next;
                }
                _ => break,
            }
        }
        last
    }
}

fn capital(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

fn plural(n: u64, unit: &str) -> String {
    if n == 1 { format!("every {unit}") } else { format!("every {n} {unit}s") }
}

/// The clock a routine's times are read on: the machine's, a named time zone, or a fixed offset (tests).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Zone {
    Local,
    Named(Tz),
    Fixed(FixedOffset),
}

impl Zone {
    /// `Europe/Paris`, `UTC`; empty or `local` is the machine's own.
    pub fn parse(text: &str) -> Result<Zone, String> {
        match text.trim() {
            "" => Ok(Zone::Local),
            t if t.eq_ignore_ascii_case("local") => Ok(Zone::Local),
            t => Tz::from_str(t).map(Zone::Named).map_err(|_| format!("{t:?} is not a time zone: a name like Europe/Paris, or local")),
        }
    }

    /// Its name as `tz` takes it, empty for the machine's own.
    pub fn name(&self) -> String {
        match self {
            Zone::Named(tz) => tz.name().to_string(),
            Zone::Local | Zone::Fixed(_) => String::new(),
        }
    }

    /// What the clock reads at this instant.
    pub fn local(&self, ms: u64) -> NaiveDateTime {
        let at = chrono::DateTime::from_timestamp_millis(ms as i64).unwrap_or_default();
        match self {
            Zone::Local => at.with_timezone(&Local).naive_local(),
            Zone::Named(tz) => at.with_timezone(tz).naive_local(),
            Zone::Fixed(offset) => at.with_timezone(offset).naive_local(),
        }
    }

    /// The instant the clock reads this: the first one when it reads it twice (the hour summer time ends), and the
    /// end of the gap when it never does (the hour it starts).
    pub fn epoch_ms(&self, at: NaiveDateTime) -> Option<u64> {
        let instant = |at: NaiveDateTime| match self {
            Zone::Local => Local.from_local_datetime(&at).earliest().map(|t| t.timestamp_millis()),
            Zone::Named(tz) => tz.from_local_datetime(&at).earliest().map(|t| t.timestamp_millis()),
            Zone::Fixed(offset) => offset.from_local_datetime(&at).earliest().map(|t| t.timestamp_millis()),
        };
        (0..=180).find_map(|m| instant(at + Duration::minutes(m))).map(|ms| ms as u64)
    }
}

/// What a routine runs: a prompt for a coding agent, or a script.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Run {
    Agent,
    Shell,
}

/// When a run's end shows as a notification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Notify {
    Always,
    Failed,
    Never,
}

/// One routine as its file says.
#[derive(Clone, Debug, PartialEq)]
pub struct Routine {
    pub slug: String,
    pub path: PathBuf,
    pub name: String,
    /// `when` as written, and what it means or why it does not parse.
    pub when_text: String,
    pub when: Result<When, String>,
    pub run: Run,
    /// The agent's id (`claude`, `codex`), empty for the configured default.
    pub agent: String,
    /// The prompt, or the script.
    pub body: String,
    /// Where it runs, `~` expanded; empty for the configured reposDir.
    pub cwd: String,
    pub model: String,
    pub tools: Vec<String>,
    pub mode: String,
    /// Dollars a Claude Code routine may spend over the last seven days, as it estimates its runs.
    pub budget: Option<f64>,
    /// Minutes a run may take before it is stopped.
    pub timeout: u64,
    pub notify: Notify,
    pub paused: bool,
    /// Each run in a git worktree of its own, on a branch `auto/<slug>-<id>`, so its changes stay apart.
    pub worktree: bool,
    /// Minutes a scheduled run missed while the hub was down may still start late; 0 runs only on time.
    pub catch_up: u64,
    /// A shell command run first: a non-zero exit skips the run, so it spends nothing when there is nothing to do.
    pub precheck: String,
    /// `tz` as written, and the clock its times are read on or why it is no time zone.
    pub tz: String,
    pub zone: Result<Zone, String>,
}

/// The `key: value` lines between two `---` lines at the top, and what follows.
pub fn front_matter(text: &str) -> (Vec<(String, String)>, String) {
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("---") {
        return (Vec::new(), text.to_string());
    }
    let mut fields = Vec::new();
    let mut body = Vec::new();
    let mut in_head = true;
    for line in lines {
        if in_head {
            if line.trim() == "---" {
                in_head = false;
            } else if let Some((key, value)) = line.split_once(':') {
                fields.push((key.trim().to_lowercase(), value.trim().to_string()));
            }
        } else {
            body.push(line);
        }
    }
    if in_head {
        return (Vec::new(), text.to_string());
    }
    (fields, body.join("\n").trim().to_string())
}

/// The file's text with the field set (added at the end of the front matter when missing), the rest as it was.
pub fn set_field(text: &str, key: &str, value: &str) -> String {
    let (fields, body) = front_matter(text);
    let mut out = vec!["---".to_string()];
    let mut set = false;
    for (k, v) in fields {
        if k == key {
            out.push(format!("{key}: {value}"));
            set = true;
        } else {
            out.push(format!("{k}: {v}"));
        }
    }
    if !set {
        out.push(format!("{key}: {value}"));
    }
    out.push("---".into());
    out.push(String::new());
    out.push(body);
    let mut text = out.join("\n");
    text.push('\n');
    text
}

/// "2", "$2", "$1.50/week", "2 a week": the leading amount.
fn dollars(text: &str) -> Option<f64> {
    let text = text.trim().trim_start_matches('$');
    let end = text.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(text.len());
    text[..end].parse().ok().filter(|b: &f64| *b > 0.0)
}

/// Minutes a late run may still start: `2h`, `90m`, `30`; `never`, `no` or `-1` for only on time; 12 hours by default.
fn catch_up(text: &str) -> u64 {
    let text = text.trim().to_lowercase();
    match text.as_str() {
        "" => 12 * 60,
        "never" | "no" | "off" | "-1" | "0" => 0,
        _ => interval(&text).or_else(|| interval(&format!("{text}m"))).map_or(12 * 60, |s| s / 60),
    }
}

fn expand(path: &str) -> String {
    match path.strip_prefix('~') {
        Some(rest) => format!("{}{rest}", files::home().to_string_lossy()),
        None => path.to_string(),
    }
}

impl Routine {
    pub fn parse(slug: &str, path: &Path, text: &str) -> Routine {
        let (fields, body) = front_matter(text);
        let get = |key: &str| fields.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v.clone()).unwrap_or_default();
        let when_text = get("when");
        let tz = get("tz");
        let truthy = |key: &str| ["true", "yes", "on", "1"].contains(&get(key).to_lowercase().as_str());
        Routine {
            slug: slug.into(),
            path: path.to_path_buf(),
            name: Some(get("name")).filter(|n| !n.is_empty()).unwrap_or_else(|| slug.replace('-', " ")),
            when: When::parse(&when_text),
            when_text,
            run: if get("run").eq_ignore_ascii_case("shell") { Run::Shell } else { Run::Agent },
            agent: get("agent").to_lowercase(),
            body,
            cwd: expand(&get("cwd")),
            model: get("model"),
            tools: get("tools").split(',').map(str::trim).filter(|t| !t.is_empty()).map(str::to_string).collect(),
            mode: get("mode"),
            budget: dollars(&get("budget")),
            timeout: get("timeout").trim_end_matches("m").trim().parse().ok().filter(|t| *t > 0).unwrap_or(30),
            notify: match get("notify").to_lowercase().as_str() {
                "never" | "no" | "off" => Notify::Never,
                "failed" | "failures" | "errors" => Notify::Failed,
                _ => Notify::Always,
            },
            paused: truthy("paused"),
            worktree: truthy("worktree"),
            catch_up: catch_up(&get("catch-up")),
            precheck: get("precheck"),
            zone: Zone::parse(&tz),
            tz,
        }
    }

    /// Haiku with nothing but its own tools.
    pub fn is_lightest(&self) -> bool {
        matches!(self.model.to_lowercase().as_str(), "lightest" | "light")
    }

    /// How its `when` reads, with the time zone its times are in when it names one, or why it does not parse.
    pub fn when_shown(&self) -> String {
        match (&self.when, &self.zone) {
            (Ok(when @ When::Cron(_)), Ok(zone @ Zone::Named(_))) => format!("{} in {}", when.describe(&self.when_text), zone.name()),
            (Ok(when), _) => when.describe(&self.when_text),
            (Err(err), _) => err.clone(),
        }
    }

    /// Why it cannot run on its own: its `when` does not parse, or its `tz` is no time zone.
    pub fn broken(&self) -> Option<String> {
        match (&self.when, &self.zone) {
            (Err(err), _) => Some(err.clone()),
            (_, Err(err)) => Some(format!("tz: {err}")),
            _ => None,
        }
    }

    /// The clock its times are read on; the machine's when its `tz` is broken, which keeps it from running anyway.
    pub fn clock(&self) -> Zone {
        self.zone.clone().unwrap_or(Zone::Local)
    }
}

/// Every routine in the folder, by slug.
pub fn read_all(dir: &Path) -> Vec<Routine> {
    let mut found: Vec<Routine> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let slug = path.file_name()?.to_str()?.strip_suffix(".md")?.to_string();
            (!slug.starts_with('.')).then(|| Routine::parse(&slug, &path, &std::fs::read_to_string(&path).unwrap_or_default()))
        })
        .collect();
    found.sort_by(|a, b| a.slug.cmp(&b.slug));
    found
}

/// A file name of the name's words, lowercased, unique in the folder.
pub fn slug_for(dir: &Path, name: &str) -> String {
    let words: Vec<String> = name.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).take(6).map(|w| w.to_lowercase()).collect();
    let base = if words.is_empty() { "routine".to_string() } else { words.join("-") };
    let mut slug = base.clone();
    let mut n = 2;
    while dir.join(format!("{slug}.md")).exists() {
        slug = format!("{base}-{n}");
        n += 1;
    }
    slug
}

/// A new routine's file text.
pub fn new_file(name: &str, when: &str, cwd: &str, body: &str) -> String {
    let mut head = vec!["---".to_string(), format!("name: {}", name.trim()), format!("when: {}", when.trim())];
    if !cwd.trim().is_empty() {
        head.push(format!("cwd: {}", cwd.trim()));
    }
    head.push("---".into());
    format!("{}\n\n{}\n", head.join("\n"), body.trim())
}

/// A routine to start from: what fills the form, and nothing is saved until the form is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Template {
    pub id: &'static str,
    pub name: &'static str,
    /// What it does, in a line.
    pub about: &'static str,
    pub when: &'static str,
    /// A script run with `sh -c` rather than a prompt for the agent.
    pub shell: bool,
    pub model: &'static str,
    pub tools: &'static str,
    pub mode: &'static str,
    pub precheck: &'static str,
    pub worktree: bool,
    pub notify: &'static str,
    pub body: &'static str,
}

impl Template {
    /// The template as this agent takes it: a model only when the agent's table names it its own, tools and a permission
    /// mode only on Claude Code, the one agent a routine passes them to.
    pub fn on(&self, agent: &Agent) -> Template {
        let own = agent.models.iter().any(|m| self.model == m || (m.ends_with(['-', '/']) && self.model.starts_with(m.as_str())));
        let claude = agent.id == "claude";
        Template { model: if own { self.model } else { "" }, tools: if claude { self.tools } else { "" }, mode: if claude { self.mode } else { "" }, ..*self }
    }

    /// The template as the built-in agent of this id takes it; one the table lacks gets no model, tools or mode.
    pub fn for_agent(&self, id: &str) -> Template {
        match crate::agent::builtin().iter().find(|a| a.id == id) {
            Some(agent) => self.on(agent),
            None => Template { model: "", tools: "", mode: "", ..*self },
        }
    }
}

/// The routines most people start with; their prechecks skip a run when there is nothing for it to do.
pub const TEMPLATES: [Template; 4] = [
    Template {
        id: "morning-sweep",
        name: "Morning sweep",
        about: "Your open PRs each weekday morning: what waits for review, fails CI or can merge.",
        when: "weekdays 09:00",
        shell: false,
        model: "sonnet",
        tools: "Bash(gh search prs:*), Bash(gh pr view:*), Bash(gh pr checks:*)",
        mode: "",
        precheck: "gh search prs --author=@me --state=open --json number --jq length | grep -qv '^0$'",
        worktree: false,
        notify: "always",
        body: "Sum up my open PRs: which wait for review, which fail CI, which can merge. Start with one line for the whole \
               picture, then one line per PR.",
    },
    Template {
        id: "ci-watcher",
        name: "CI watcher",
        about: "When CI fails on a PR of yours and is still red, what broke and the likely fix, in a few lines.",
        when: "on ci-failed",
        shell: false,
        model: "sonnet",
        tools: "Bash(gh run view:*), Bash(gh pr view:*), Bash(gh pr checks:*)",
        mode: "",
        precheck: "case \"$COCKPIT_EVENT_KEY\" in https://*/pull/*) gh pr checks \"$COCKPIT_EVENT_KEY\" --json bucket --jq \
                   '[.[] | select(.bucket == \"fail\")] | length' | grep -qv '^0$' ;; *) true ;; esac",
        worktree: false,
        notify: "always",
        body: "CI failed on a PR of mine; what set it off is below. Read the failing job's log with gh run view \
               --log-failed and tell me in a few lines what broke and the likely fix. Change nothing.",
    },
    Template {
        id: "worktree-cleanup",
        name: "Weekly worktree cleanup",
        about: "Every Friday, the worktrees of your repos whose branch merged, idle for 3 days and holding no work, removed.",
        when: "fri 17:00",
        shell: true,
        model: "",
        tools: "",
        mode: "",
        precheck: "",
        worktree: false,
        notify: "failed",
        body: include_str!("routine-cleanup.sh"),
    },
    Template {
        id: "release-check",
        name: "Release check",
        about: "Each weekday afternoon, which of your PRs merged since the last weekday are released and which still wait.",
        when: "weekdays 16:00",
        shell: false,
        model: "haiku",
        tools: "Bash(gh search prs:*), Bash(gh release list:*), Bash(gh release view:*)",
        mode: "",
        precheck: "days=1; [ \"$(date +%u)\" = 1 ] && days=3; since=$(date -d \"$days days ago\" +%F 2>/dev/null || date -v-\"$days\"d +%F); \
                   gh search prs --author=@me --merged --merged-at=\">=$since\" --json number --jq length | grep -qv '^0$'",
        worktree: false,
        notify: "always",
        body: "List my PRs merged since the last weekday with gh search prs, and for each one's repo whether its latest \
               release (gh release list --limit 1) came after the merge. One line per PR: released, or still waiting.",
    },
];

/// What needs a look among the hub's `routines` document, newest first: each routine whose latest run failed or was
/// missed and is not read yet (a skip after it changes nothing), and each routine whose file is broken.
pub fn needing_look(doc: &Value) -> Vec<Value> {
    if doc["version"] != 1 {
        return Vec::new();
    }
    let inbox_from = doc["inboxFrom"].as_u64().unwrap_or(u64::MAX);
    let mut found = Vec::new();
    for r in doc["routines"].as_array().into_iter().flatten() {
        let slug = r["slug"].as_str().unwrap_or_default();
        let name = r["name"].as_str().unwrap_or(slug);
        if let Some(error) = r["error"].as_str().filter(|e| !e.is_empty()) {
            found.push(serde_json::json!({"key": slug, "slug": slug, "id": null, "name": name, "word": "broken", "reason": error, "summary": error, "at": 0}));
            continue;
        }
        let runs = doc["runs"][slug].as_array().map(Vec::as_slice).unwrap_or_default();
        let Some(run) = runs.iter().find(|run| run["skipped"].is_null()) else { continue };
        let word = if run["missed"] == true {
            "missed"
        } else if run["ok"] == false && run["stopped"] != true {
            "failed"
        } else {
            continue;
        };
        let ended = run["ended"].as_u64().unwrap_or(0);
        if run["read"] == true || ended <= inbox_from {
            continue;
        }
        let id = run["id"].as_str().unwrap_or_default();
        let summary = run["summary"].as_str().unwrap_or_default();
        let reason = run["reason"].as_str().filter(|r| !r.is_empty()).unwrap_or(summary);
        found.push(serde_json::json!({
            "key": format!("{slug}/run/{id}"), "slug": slug, "id": id, "name": name, "word": word, "reason": reason, "summary": summary, "at": ended,
        }));
    }
    found.sort_by_key(|item| std::cmp::Reverse(item["at"].as_u64().unwrap_or(0)));
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M").unwrap()
    }

    fn next(when: &str, after: &str) -> String {
        When::parse(when).unwrap().next_after(at(after)).map(|t| t.format("%a %Y-%m-%d %H:%M").to_string()).unwrap_or_default()
    }

    #[test]
    fn words_and_cron_give_the_next_run() {
        // 2026-10-29 is a Thursday.
        assert_eq!(next("daily 09:00", "2026-10-29 08:59"), "Thu 2026-10-29 09:00");
        assert_eq!(next("daily 09:00", "2026-10-29 09:00"), "Fri 2026-10-30 09:00");
        assert_eq!(next("weekdays 9:00, 17:30", "2026-10-30 18:00"), "Mon 2026-11-02 09:00");
        assert_eq!(next("weekends 10:00", "2026-10-29 12:00"), "Sat 2026-10-31 10:00");
        assert_eq!(next("mon,thu 14:00", "2026-10-29 14:00"), "Mon 2026-11-02 14:00");
        assert_eq!(next("tue-thu 08:15", "2026-10-29 09:00"), "Tue 2026-11-03 08:15");
        assert_eq!(next("at 7h", "2026-10-29 09:00"), "Fri 2026-10-30 07:00");
        assert_eq!(next("cron */15 9-17 * * 1-5", "2026-10-29 17:50"), "Fri 2026-10-30 09:00");
        assert_eq!(next("cron 0 0 1 * *", "2026-10-29 12:00"), "Sun 2026-11-01 00:00");
        assert_eq!(next("cron 30 6 13 * 5", "2026-10-29 12:00"), "Fri 2026-10-30 06:30", "day of month or of week");
        assert_eq!(next("cron 0 12 * * 7", "2026-10-29 12:00"), "Sun 2026-11-01 12:00", "7 is Sunday too");
        assert_eq!(next("every 30m", "2026-10-29 12:00"), "", "an interval counts from the last run");
    }

    #[test]
    fn the_last_time_due_catches_up_once() {
        let when = When::parse("daily 09:00").unwrap();
        assert_eq!(when.last_due(at("2026-10-27 10:00"), at("2026-10-29 12:00")), Some(at("2026-10-29 09:00")));
        assert_eq!(when.last_due(at("2026-10-29 09:00"), at("2026-10-29 12:00")), None);
    }

    #[test]
    fn whens_read_in_words() {
        let shown = |text: &str| When::parse(text).map(|w| w.describe(text)).unwrap_or_else(|e| format!("error: {e}"));
        assert_eq!(shown(""), "by hand");
        assert_eq!(shown("every 30 min"), "every 30 min");
        assert_eq!(shown("every 2h"), "every 2 h");
        assert_eq!(shown("hourly"), "every hour");
        assert_eq!(shown("every day"), "every day");
        assert_eq!(shown("weekdays 9:00,17:30"), "weekdays at 09:00, 17:30");
        assert_eq!(shown("Mon,thu 14:00"), "Mon, Thu at 14:00");
        assert_eq!(shown("cron 0 9 * * 1-5"), "cron 0 9 * * 1-5");
        assert_eq!(shown("on ci-failed api"), "on CI failing · api");
        assert!(shown("every 10s").starts_with("error: "));
        assert!(shown("daily 25:00").starts_with("error: "));
        assert!(shown("cron 0 9 * *").starts_with("error: cron takes five fields"));
        assert!(shown("on nothing").starts_with("error: on what? one of any, review"));
        assert!(shown("sometimes").starts_with("error: when?"));
    }

    #[test]
    fn an_event_sets_off_the_routines_listening_to_its_kind_and_words() {
        let when = When::parse("on ci-failed checkout").unwrap();
        assert!(when.is_set_off_by(&serde_json::json!({"kind": "ci_failed", "title": "🔴 CI failed: Checkout me/x"})));
        assert!(when.is_set_off_by(&serde_json::json!({"kind": "ci_failing", "title": "🔴 CI failing: checkout#12"})));
        assert!(!when.is_set_off_by(&serde_json::json!({"kind": "ci_failed", "title": "🔴 CI failed: api main"})));
        assert!(!when.is_set_off_by(&serde_json::json!({"kind": "merged", "title": "🟣 Merged: checkout#12"})));
        assert!(When::parse("on any").unwrap().is_set_off_by(&serde_json::json!({"kind": "merged", "title": "x"})));
    }

    #[test]
    fn a_file_reads_its_front_matter_and_body() {
        let text = "---\nname: Morning sweep\nwhen: weekdays 09:00\ncwd: ~/work\ntools: Read, Bash(gh pr list:*)\nmodel: sonnet\nbudget: $0.50/week\nnotify: failed\n---\n\nSum up my PRs.\n";
        let r = Routine::parse("morning-sweep", Path::new("/r/morning-sweep.md"), text);
        assert_eq!((r.name.as_str(), r.body.as_str(), r.model.as_str()), ("Morning sweep", "Sum up my PRs.", "sonnet"));
        assert_eq!(r.tools, ["Read", "Bash(gh pr list:*)"]);
        assert_eq!((r.budget, r.timeout, r.notify, r.paused, r.run), (Some(0.5), 30, Notify::Failed, false, Run::Agent));
        assert_eq!(r.agent, "", "the default agent");
        let codex = Routine::parse("c", Path::new("/r/c.md"), "---\nagent: Codex\n---\nGo.");
        assert_eq!((codex.run, codex.agent.as_str()), (Run::Agent, "codex"));
        assert!(r.cwd.ends_with("/work") && !r.cwd.starts_with('~'));
        assert_eq!(r.when_shown(), "weekdays at 09:00");
        assert_eq!([dollars("2"), dollars("$1.5 a week"), dollars("0"), dollars("lots")], [Some(2.0), Some(1.5), None, None]);
        let bare = Routine::parse("fetch-all", Path::new("/r/fetch-all.md"), "git fetch --all");
        assert_eq!((bare.name.as_str(), bare.when.clone(), bare.body.as_str()), ("fetch all", Ok(When::Manual), "git fetch --all"));
        assert_eq!((bare.worktree, bare.catch_up), (false, 12 * 60));
        let isolated = Routine::parse("x", Path::new("/r/x.md"), "---\nworktree: yes\ncatch-up: 2h\n---\nfix it");
        assert_eq!((isolated.worktree, isolated.catch_up), (true, 120));
        assert_eq!([catch_up("never"), catch_up("90"), catch_up("1d")], [0, 90, 1440]);
    }

    #[test]
    fn a_precheck_and_a_time_zone_read_from_the_file() {
        let text = "---\nname: Reviews\nwhen: weekdays 09:00\ntz: Europe/Paris\nprecheck: gh pr list --search review-requested:@me | grep -q .\n---\nSum up.";
        let r = Routine::parse("reviews", Path::new("/r/reviews.md"), text);
        assert_eq!(r.precheck, "gh pr list --search review-requested:@me | grep -q .");
        assert_eq!((r.tz.as_str(), r.zone.clone().map(|z| z.name())), ("Europe/Paris", Ok("Europe/Paris".to_string())));
        assert_eq!((r.when_shown(), r.broken()), ("weekdays at 09:00 in Europe/Paris".to_string(), None));
        let plain = Routine::parse("p", Path::new("/r/p.md"), "---\nwhen: every 30m\ntz: Asia/Tokyo\n---\nGo.");
        assert_eq!(plain.when_shown(), "every 30 min", "an interval reads the same in every zone");
        let bad = Routine::parse("b", Path::new("/r/b.md"), "---\nwhen: daily 09:00\ntz: Paris\n---\nGo.");
        assert_eq!(bad.broken(), Some("tz: \"Paris\" is not a time zone: a name like Europe/Paris, or local".to_string()));
        assert_eq!((bad.clock(), Zone::parse("local"), Zone::parse("")), (Zone::Local, Ok(Zone::Local), Ok(Zone::Local)));
    }

    #[test]
    fn a_time_skipped_or_read_twice_by_summer_time_lands_once() {
        let paris = Zone::parse("Europe/Paris").unwrap();
        let utc = |text: &str| chrono::DateTime::parse_from_rfc3339(text).unwrap().timestamp_millis() as u64;
        // 2026-03-29 02:00 jumps to 03:00 in Paris: 02:30 never shows on its clocks, so it lands at 03:00.
        assert_eq!(paris.epoch_ms(at("2026-03-29 02:30")), Some(utc("2026-03-29T01:00:00Z")));
        assert_eq!(paris.epoch_ms(at("2026-03-29 09:00")), Some(utc("2026-03-29T07:00:00Z")));
        assert_eq!(paris.epoch_ms(at("2026-03-28 09:00")), Some(utc("2026-03-28T08:00:00Z")));
        // 2026-10-25 03:00 goes back to 02:00: 02:30 shows twice, the first one counts.
        assert_eq!(paris.epoch_ms(at("2026-10-25 02:30")), Some(utc("2026-10-25T00:30:00Z")));
        assert_eq!(paris.local(utc("2026-10-25T01:30:00Z")), at("2026-10-25 02:30"), "the second time it reads 02:30");
        let fixed = Zone::Fixed(FixedOffset::east_opt(3600).unwrap());
        assert_eq!(fixed.epoch_ms(at("2026-03-29 02:30")), Some(utc("2026-03-29T01:30:00Z")));
    }

    #[test]
    fn a_field_set_keeps_the_rest_of_the_file() {
        let text = "---\nname: x\nwhen: hourly\n---\n\nDo it.\n";
        let paused = set_field(text, "paused", "true");
        assert_eq!(paused, "---\nname: x\nwhen: hourly\npaused: true\n---\n\nDo it.\n");
        assert_eq!(set_field(&paused, "paused", "false"), "---\nname: x\nwhen: hourly\npaused: false\n---\n\nDo it.\n");
        assert_eq!(set_field("Do it.", "when", "hourly"), "---\nwhen: hourly\n---\n\nDo it.\n");
    }

    #[test]
    fn a_new_routine_gets_a_free_slug_and_reads_back() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(slug_for(dir.path(), "Sum up my PRs!"), "sum-up-my-prs");
        std::fs::write(dir.path().join("sum-up-my-prs.md"), new_file("Sum up my PRs!", "daily 09:00", "", "List them.")).unwrap();
        assert_eq!(slug_for(dir.path(), "sum up my PRs"), "sum-up-my-prs-2");
        let all = read_all(dir.path());
        assert_eq!((all.len(), all[0].name.as_str(), all[0].body.as_str()), (1, "Sum up my PRs!", "List them."));
    }
    #[test]
    fn templates_read_as_routines_that_need_no_more_than_their_tools() {
        let claude = crate::agent::builtin().into_iter().find(|a| a.id == "claude").unwrap();
        let models = crate::models::for_routine(&claude, || {});
        for t in TEMPLATES {
            assert!(When::parse(t.when).is_ok(), "{}", t.id);
            assert!(!t.body.is_empty() && !t.about.is_empty() && t.about.ends_with('.'), "{}", t.id);
            assert!(t.shell || !t.tools.is_empty(), "an agent template lists its tools: {}", t.id);
            assert!(models.iter().any(|m| m == t.model), "{}", t.id);
            for script in [t.precheck].into_iter().chain(t.shell.then_some(t.body)).filter(|s| !s.is_empty()) {
                let read = std::process::Command::new("sh").args(["-n", "-c", script]).output().unwrap();
                assert!(read.status.success(), "{}: {}", t.id, String::from_utf8_lossy(&read.stderr));
            }
        }
        let ids: Vec<&str> = TEMPLATES.iter().map(|t| t.id).collect();
        assert_eq!(ids, ["morning-sweep", "ci-watcher", "worktree-cleanup", "release-check"]);
        let agents = crate::agent::builtin();
        let on = |id: &str| TEMPLATES[0].on(agents.iter().find(|a| a.id == id).unwrap());
        assert_eq!((on("claude").model, on("claude").tools.is_empty()), ("sonnet", false));
        assert_eq!((on("codex").model, on("codex").tools, on("codex").precheck), ("", "", TEMPLATES[0].precheck), "Codex takes neither");
    }

    #[test]
    fn what_needs_a_look_is_each_routines_unread_failure_or_miss_and_each_broken_file() {
        let doc = serde_json::json!({"version": 1, "inboxFrom": 100, "routines": [
            {"slug": "a", "name": "A"}, {"slug": "b", "name": "B"}, {"slug": "c", "name": "C"}, {"slug": "d", "name": "D", "error": "when?"},
            {"slug": "e", "name": "E"}, {"slug": "f", "name": "F"}, {"slug": "g", "name": "G"}],
            "runs": {
                "a": [{"id": "3", "ok": true, "skipped": "precheck", "ended": 900}, {"id": "2", "ok": false, "ended": 800, "summary": "exit 3", "reason": "Ended with exit 3."}],
                "b": [{"id": "5", "ok": true, "ended": 700}, {"id": "4", "ok": false, "ended": 600}],
                "c": [{"id": "6", "ok": false, "missed": true, "ended": 500, "summary": "Missed Sun 09:00: cockpit was not running then."}],
                "e": [{"id": "7", "ok": false, "ended": 400, "read": true}],
                "f": [{"id": "8", "ok": false, "stopped": true, "ended": 300}],
                "g": [{"id": "9", "ok": false, "ended": 50}]}});
        let look: Vec<(String, String, String)> = needing_look(&doc)
            .iter()
            .map(|x| (x["key"].as_str().unwrap().into(), x["word"].as_str().unwrap().into(), x["reason"].as_str().unwrap().into()))
            .collect();
        assert_eq!(
            look,
            [
                ("a/run/2".into(), "failed".into(), "Ended with exit 3.".into()),
                ("c/run/6".into(), "missed".into(), "Missed Sun 09:00: cockpit was not running then.".into()),
                ("d".into(), "broken".into(), "when?".into()),
            ],
            "b ran fine since, e is read, f was stopped by hand, g ended before the inbox began"
        );
        assert!(needing_look(&serde_json::json!({"version": 2})).is_empty());
    }
}
