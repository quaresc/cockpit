//! Routines: a Claude prompt or a shell script in ~/.config/cockpit/routines/<slug>.md that the hub runs on its own, on
//! a schedule, every so often or when something happens. A file is a front matter of `key: value` lines, then the
//! prompt (or the script); `when` says when it runs, in words or as cron.

use std::path::{Path, PathBuf};

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use serde_json::Value;

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
            .map(|(h, m)| Cron {
                minutes: 1 << m,
                hours: 1 << h,
                days: u32::MAX,
                months: u16::MAX,
                weekdays,
                any_day: true,
                any_weekday: weekdays == 0x7f,
            })
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
                s if s % 3600 == 0 => if *s == 3600 { "every hour".into() } else { format!("every {} h", s / 3600) },
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

/// What a routine runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Run {
    Claude,
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
    /// The prompt, or the script.
    pub body: String,
    /// Where it runs, `~` expanded; empty for the configured reposDir.
    pub cwd: String,
    pub model: String,
    pub tools: Vec<String>,
    pub mode: String,
    /// Dollars a Claude routine may spend over the last seven days, as Claude estimates its runs.
    pub budget: Option<f64>,
    /// Minutes a run may take before it is stopped.
    pub timeout: u64,
    pub notify: Notify,
    pub paused: bool,
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
        let truthy = |key: &str| ["true", "yes", "on", "1"].contains(&get(key).to_lowercase().as_str());
        Routine {
            slug: slug.into(),
            path: path.to_path_buf(),
            name: Some(get("name")).filter(|n| !n.is_empty()).unwrap_or_else(|| slug.replace('-', " ")),
            when: When::parse(&when_text),
            when_text,
            run: if get("run").eq_ignore_ascii_case("shell") { Run::Shell } else { Run::Claude },
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
        }
    }

    /// How its `when` reads, or why it does not parse.
    pub fn when_shown(&self) -> String {
        match &self.when {
            Ok(when) => when.describe(&self.when_text),
            Err(err) => err.clone(),
        }
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
    let words: Vec<String> =
        name.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).take(6).map(|w| w.to_lowercase()).collect();
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
        let when = When::parse("on ci-failed sourcing").unwrap();
        assert!(when.is_set_off_by(&serde_json::json!({"kind": "ci_failed", "title": "🔴 CI failed: Sourcing cq/x"})));
        assert!(when.is_set_off_by(&serde_json::json!({"kind": "ci_failing", "title": "🔴 CI failing: sourcing#12"})));
        assert!(!when.is_set_off_by(&serde_json::json!({"kind": "ci_failed", "title": "🔴 CI failed: api main"})));
        assert!(!when.is_set_off_by(&serde_json::json!({"kind": "merged", "title": "🟣 Merged: sourcing#12"})));
        assert!(When::parse("on any").unwrap().is_set_off_by(&serde_json::json!({"kind": "merged", "title": "x"})));
    }

    #[test]
    fn a_file_reads_its_front_matter_and_body() {
        let text = "---\nname: Morning sweep\nwhen: weekdays 09:00\ncwd: ~/work\ntools: Read, Bash(gh pr list:*)\nmodel: sonnet\nbudget: $0.50/week\nnotify: failed\n---\n\nSum up my PRs.\n";
        let r = Routine::parse("morning-sweep", Path::new("/r/morning-sweep.md"), text);
        assert_eq!((r.name.as_str(), r.body.as_str(), r.model.as_str()), ("Morning sweep", "Sum up my PRs.", "sonnet"));
        assert_eq!(r.tools, ["Read", "Bash(gh pr list:*)"]);
        assert_eq!((r.budget, r.timeout, r.notify, r.paused, r.run), (Some(0.5), 30, Notify::Failed, false, Run::Claude));
        assert!(r.cwd.ends_with("/work") && !r.cwd.starts_with('~'));
        assert_eq!(r.when_shown(), "weekdays at 09:00");
        assert_eq!([dollars("2"), dollars("$1.5 a week"), dollars("0"), dollars("lots")], [Some(2.0), Some(1.5), None, None]);
        let bare = Routine::parse("fetch-all", Path::new("/r/fetch-all.md"), "git fetch --all");
        assert_eq!((bare.name.as_str(), bare.when.clone(), bare.body.as_str()), ("fetch all", Ok(When::Manual), "git fetch --all"));
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
}
