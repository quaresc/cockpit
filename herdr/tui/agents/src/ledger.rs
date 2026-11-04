//! The time agents spend working, per ticket: each stretch an agent works is a line of ledger.jsonl {start, end,
//! ticket, terminal}, written when it stops, and every minute while it goes on so a hub that stops loses little. Two
//! agents on one ticket at once count once.

use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{Datelike, Duration, Local, NaiveTime, TimeZone};
use cockpit_core::files;
use serde_json::{Value, json};

const CHECKPOINT_MS: u64 = 60_000;
/// Lines older than this go when the file is rewritten.
const KEEP_MS: u64 = 35 * 86_400_000;

#[derive(Clone, Debug, PartialEq)]
pub struct Stretch {
    pub start: u64,
    pub end: u64,
    pub ticket: String,
    pub terminal: String,
}

#[derive(Default)]
pub struct Ledger {
    /// Working agents: terminal -> (since, ticket).
    open: HashMap<String, (u64, String)>,
}

pub fn path() -> PathBuf {
    files::cache().join("agents/ledger.jsonl")
}

fn append(path: &Path, stretches: &[Stretch]) {
    if stretches.is_empty() {
        return;
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) else { return };
    for s in stretches.iter().filter(|s| s.end > s.start) {
        let _ = writeln!(f, "{}", json!({"start": s.start, "end": s.end, "ticket": s.ticket, "terminal": s.terminal}));
    }
}

impl Ledger {
    /// One look at who works on what: opens, closes and checkpoints stretches; the open ones as they stand.
    pub fn tick(&mut self, path: &Path, working: &[(String, String)], now: u64) {
        let mut done = Vec::new();
        self.open.retain(|terminal, (start, ticket)| {
            let still = working.iter().any(|(t, k)| t == terminal && k == ticket);
            if !still {
                done.push(Stretch { start: *start, end: now, ticket: ticket.clone(), terminal: terminal.clone() });
            }
            still
        });
        for (terminal, ticket) in working {
            let entry = self.open.entry(terminal.clone()).or_insert((now, ticket.clone()));
            if now.saturating_sub(entry.0) >= CHECKPOINT_MS {
                done.push(Stretch { start: entry.0, end: now, ticket: ticket.clone(), terminal: terminal.clone() });
                entry.0 = now;
            }
        }
        append(path, &done);
    }

    pub fn open_stretches(&self, now: u64) -> Vec<Stretch> {
        self.open.iter().map(|(t, (s, k))| Stretch { start: *s, end: now, ticket: k.clone(), terminal: t.clone() }).collect()
    }
}

/// The stretches written since `since`.
pub fn read(path: &Path, since: u64) -> Vec<Stretch> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    text.lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|v| v["end"].as_u64().unwrap_or(0) > since)
        .map(|v| Stretch {
            start: v["start"].as_u64().unwrap_or(0),
            end: v["end"].as_u64().unwrap_or(0),
            ticket: v["ticket"].as_str().unwrap_or_default().to_string(),
            terminal: v["terminal"].as_str().unwrap_or_default().to_string(),
        })
        .collect()
}

/// Rewrites the file without what is older than five weeks.
pub fn trim(path: &Path, now: u64) {
    let kept = read(path, now.saturating_sub(KEEP_MS));
    let tmp = path.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp);
    append(&tmp, &kept);
    let _ = std::fs::rename(&tmp, path);
}

/// The time intervals cover inside [from, to), overlaps counted once, in seconds.
pub fn covered(intervals: &[(u64, u64)], from: u64, to: u64) -> u64 {
    let mut clipped: Vec<(u64, u64)> = intervals.iter().map(|&(s, e)| (s.max(from), e.min(to))).filter(|(s, e)| e > s).collect();
    clipped.sort();
    let mut total = 0;
    let mut cur: Option<(u64, u64)> = None;
    for (s, e) in clipped {
        cur = match cur {
            Some((cs, ce)) if s <= ce => Some((cs, ce.max(e))),
            Some((cs, ce)) => {
                total += ce - cs;
                Some((s, e))
            }
            None => Some((s, e)),
        };
    }
    if let Some((cs, ce)) = cur {
        total += ce - cs;
    }
    total / 1000
}

/// Local midnight of the day holding `now`, and the Monday that starts its week, in ms.
pub fn day_and_week_start(now: u64) -> (u64, u64) {
    let local = Local.timestamp_millis_opt(now as i64).single().unwrap_or_else(Local::now);
    let midnight = local.with_time(NaiveTime::MIN).single().map(|d| d.timestamp_millis() as u64).unwrap_or(now);
    let monday = (local - Duration::days(local.weekday().num_days_from_monday() as i64))
        .with_time(NaiveTime::MIN)
        .single()
        .map(|d| d.timestamp_millis() as u64)
        .unwrap_or(midnight);
    (midnight, monday)
}

pub struct Totals {
    /// ticket -> (today, this week), seconds.
    pub tickets: BTreeMap<String, (u64, u64)>,
    /// terminal -> today, seconds.
    pub agents: HashMap<String, u64>,
    /// The week's days (YYYY-MM-DD) and per ticket its seconds on each.
    pub days: Vec<String>,
    pub week: BTreeMap<String, Vec<u64>>,
}

pub fn totals(stretches: &[Stretch], now: u64) -> Totals {
    let (today, monday) = day_and_week_start(now);
    let mut by_ticket: BTreeMap<String, Vec<(u64, u64)>> = BTreeMap::new();
    let mut agents: HashMap<String, u64> = HashMap::new();
    for s in stretches {
        if !s.ticket.is_empty() {
            by_ticket.entry(s.ticket.clone()).or_default().push((s.start, s.end));
        }
        *agents.entry(s.terminal.clone()).or_default() += covered(&[(s.start, s.end)], today, now + 1);
    }
    let starts: Vec<u64> = (0..8)
        .map(|d| {
            let day = Local.timestamp_millis_opt(monday as i64).single().map(|m| m + Duration::days(d));
            day.and_then(|d| d.with_time(NaiveTime::MIN).single()).map(|d| d.timestamp_millis() as u64).unwrap_or(monday + d as u64 * 86_400_000)
        })
        .collect();
    let days =
        starts[..7].iter().map(|s| Local.timestamp_millis_opt(*s as i64).single().map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_default()).collect();
    let mut tickets = BTreeMap::new();
    let mut week = BTreeMap::new();
    for (k, iv) in by_ticket {
        let w = covered(&iv, monday, now + 1);
        if w == 0 {
            continue;
        }
        tickets.insert(k.clone(), (covered(&iv, today, now + 1), w));
        week.insert(k, (0..7).map(|d| covered(&iv, starts[d], starts[d + 1])).collect());
    }
    Totals { tickets, agents, days, week }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlapping_agents_on_one_ticket_count_once() {
        assert_eq!(covered(&[(0, 10_000), (5_000, 15_000), (20_000, 21_000)], 0, 100_000), 16);
        assert_eq!(covered(&[(0, 10_000)], 4_000, 6_000), 2);
    }

    #[test]
    fn stretches_open_checkpoint_and_close_into_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("ledger.jsonl");
        let mut l = Ledger::default();
        let w = vec![("app:t1".to_string(), "ENG-1".to_string())];
        l.tick(&p, &w, 1_000);
        l.tick(&p, &w, 61_000);
        assert_eq!(read(&p, 0).len(), 1);
        l.tick(&p, &[], 70_000);
        let all = read(&p, 0);
        assert_eq!(all.iter().map(|s| (s.start, s.end)).collect::<Vec<_>>(), [(1_000, 61_000), (61_000, 70_000)]);
        assert!(l.open_stretches(80_000).is_empty());
    }

    #[test]
    fn totals_split_today_the_week_and_each_agent() {
        let now = Local.with_ymd_and_hms(2026, 11, 4, 12, 0, 0).unwrap().timestamp_millis() as u64;
        let h = 3_600_000;
        let s = |start, end, t: &str, a: &str| Stretch { start, end, ticket: t.into(), terminal: a.into() };
        let t = totals(&[s(now - 2 * h, now - h, "ENG-1", "a"), s(now - 2 * h, now - h, "ENG-1", "b"), s(now - 30 * h, now - 29 * h, "ENG-1", "a")], now);
        assert_eq!(t.tickets["ENG-1"], (3600, 7200));
        assert_eq!(t.agents["a"], 3600);
        assert_eq!(t.days[0], "2026-11-02");
        assert_eq!(t.week["ENG-1"][1..3], [3600, 3600]);
    }
}
