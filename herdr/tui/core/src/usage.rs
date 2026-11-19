//! Plan limits and token use of the coding agents, read from what they leave on disk: Claude Code's status line (the
//! plugin's limits.json), the rate limits a session cockpit drives reports, and Codex's rollouts. Every window names its
//! source and how fresh it is, a number is only ever one a source gave, and tokens and spend per day are estimates.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use chrono::{Local, TimeZone};
use crossterm::event::{Event, KeyCode, KeyEventKind, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::agent::{self, Transcripts};
use crate::fresh::{Freshness, State};
use crate::keys::{self, Move};
use crate::theme::{BG, DIM, FG, GREEN, ORANGE, PANEL, RED, bold, fg};
use crate::{files, term, transcripts};

pub const STATUSLINE: &str = "claude-statusline";
pub const STREAM: &str = "claude-stream";
pub const ROLLOUT: &str = "codex-rollout";
pub const COPILOT: &str = "copilot-gh";
/// A reading from the command the config names for an agent (`usage.commands`), for a provider cockpit has no source of.
pub const COMMAND: &str = "usage-command";

/// How often a source reports while its agent works, in ms: three of these without news make a reading stale.
pub const EVERY: u64 = 10 * 60_000;
/// Usage past this share of a window is worth a warning.
pub const WARN: f64 = 80.0;
const HOUR: u64 = 3_600_000;
const DAY: u64 = 24 * HOUR;
/// How many days of tokens the estimates cover, today included.
pub const DAYS: usize = 7;

/// Where a reading comes from, as a person would say it: the agents table's words for its source.
pub fn source_words(source: &str) -> &'static str {
    if source == COMMAND {
        return "your usage command";
    }
    agent::source_words(source).unwrap_or("its agent")
}

/// Whether an agent reads its limits from this source.
fn has_source(reports: Option<agent::Reports>, source: &str) -> bool {
    reports.is_some_and(|r| r.sources.iter().any(|(id, _)| *id == source))
}

fn agent_of(source: &str) -> String {
    agent::reporting(source).unwrap_or_default().to_string()
}

/// Whether the config asks the 7-day pace tick to count weekdays only, read again at most every 30 s.
fn weekday_pace() -> bool {
    static LAST: OnceLock<Mutex<Option<(Instant, bool)>>> = OnceLock::new();
    let mut last = LAST.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, on)) = *last
        && at.elapsed() < Duration::from_secs(30)
    {
        return on;
    }
    let on = files::config()["agents"]["pace"] == "weekdays";
    *last = Some((Instant::now(), on));
    on
}

/// The milliseconds between `from` and `to` that fall on a Monday to Friday in `tz`.
fn weekday_ms<Tz: chrono::TimeZone>(from: u64, to: u64, tz: &Tz) -> u64 {
    use chrono::{Datelike, Weekday};
    let mut total = 0;
    let mut t = from;
    while t < to {
        let Some(at) = tz.timestamp_millis_opt(t as i64).earliest() else { break };
        let midnight = at.date_naive().succ_opt().and_then(|d| d.and_hms_opt(0, 0, 0)).and_then(|d| tz.from_local_datetime(&d).earliest());
        let end = midnight.map_or(to, |m| (m.timestamp_millis() as u64).clamp(t + 1, to));
        if !matches!(at.weekday(), Weekday::Sat | Weekday::Sun) {
            total += end - t;
        }
        t = end;
    }
    total
}

// ---------------------------------------------------------------------------------------------------------------------
// A window

/// One limit of one agent as a source last reported it.
#[derive(Clone, Debug, PartialEq)]
pub struct Window {
    /// The agent it limits, by its id in the agents table.
    pub agent: String,
    /// "5h", "7d", "7d opus".
    pub name: String,
    /// Percent used, when the source says.
    pub percent: Option<f64>,
    /// The source's own verdict, when it gives one: allowed, allowed_warning or rejected.
    pub status: Option<String>,
    /// When the window starts over, in ms.
    pub resets_at: Option<u64>,
    /// How long the window lasts, in ms: where the pace tick goes.
    pub length: Option<u64>,
    /// When the source said so, in ms.
    pub as_of: u64,
    pub source: &'static str,
}

impl Window {
    pub fn freshness(&self) -> Freshness {
        Freshness { at: Some(self.as_of), every: Some(EVERY), ..Freshness::new(self.source) }
    }

    pub fn state(&self, now: u64) -> State {
        self.freshness().state(now)
    }

    /// How tight it is, for ordering: its percent, else what its verdict implies; a limit reached is full.
    pub fn tightness(&self) -> f64 {
        match self.status.as_deref() {
            Some("rejected") => 100.0,
            _ => self.percent.unwrap_or(if self.status.as_deref() == Some("allowed_warning") { WARN } else { 0.0 }),
        }
    }

    /// A stale percent is a floor until its window resets, since use only grows: true when it reads "≥".
    pub fn floor(&self, now: u64) -> bool {
        self.state(now) == State::Stale && self.resets_at.is_some_and(|r| r > now)
    }

    /// What the window reads as: "full" once its limit is reached, else its percent ("≥12%" for a floor), else its
    /// verdict in a word; never a number its source did not give.
    pub fn figure(&self, now: u64) -> String {
        match (self.status.as_deref(), self.percent) {
            (Some("rejected"), _) => "full".into(),
            (_, Some(p)) => format!("{}{}%", if self.floor(now) { "≥" } else { "" }, p.round()),
            (Some("allowed_warning"), None) => "near".into(),
            _ => "ok".into(),
        }
    }

    pub fn warn(&self) -> bool {
        self.percent.is_some_and(|p| p >= WARN) || matches!(self.status.as_deref(), Some("allowed_warning" | "rejected"))
    }

    /// How much of the window's time has gone, 0 to 1, when its length and reset are known.
    pub fn elapsed(&self, now: u64) -> Option<f64> {
        self.elapsed_with(now, weekday_pace())
    }

    /// How far through its window `now` is; with `weekdays`, a 7-day window moves a fifth per local weekday and holds
    /// still over the weekend (`agents.pace: "weekdays"`).
    pub fn elapsed_with(&self, now: u64, weekdays: bool) -> Option<f64> {
        let (length, resets) = (self.length.filter(|l| *l > 0)?, self.resets_at?);
        if weekdays && length == 7 * DAY {
            return Some((weekday_ms(resets.saturating_sub(length), now, &chrono::Local) as f64 / (5 * DAY) as f64).clamp(0.0, 1.0));
        }
        Some((1.0 - resets.saturating_sub(now) as f64 / length as f64).clamp(0.0, 1.0))
    }

    /// The shape every surface draws, the agents view's `quota` rows included.
    pub fn json(&self, now: u64) -> Value {
        let elapsed = self.elapsed(now);
        let percent = self.percent.map(f64::round);
        let ahead = elapsed.is_some_and(|e| percent.is_some_and(|p| p / 100.0 > e + 0.05));
        let short = agent::reports(&self.agent).map(|r| r.short).unwrap_or(&self.agent).to_string();
        let state = self.state(now);
        json!({
            "agent": self.agent, "provider": short, "icon": agent::icon_in(&[], &self.agent), "name": self.name, "label": format!("{short} {}", self.name),
            "percent": percent, "figure": self.figure(now), "floor": self.floor(now),
            "status": self.status, "status_words": self.status.as_deref().map(status_words),
            "resets_at": self.resets_at, "resets_words": self.resets_at.map(|r| resets_words(r, now)),
            // To the percent the bar draws, so a view moves with it rather than every second.
            "length": self.length, "elapsed": elapsed.map(|e| (e * 100.0).round() / 100.0), "ahead": ahead, "warn": self.warn(),
            "as_of": self.as_of, "as_of_words": format!("as of {}", clock(self.as_of, now)),
            "source": self.source, "source_words": source_words(self.source),
            "state": state.word(), "fresh": self.freshness().json(now),
            "tooltip": tooltip(self, state, now),
        })
    }
}

/// "resets in 2 h · as of 12:04 · from Claude Code's status line": what a bar's tooltip and the TUI line say.
fn tooltip(w: &Window, state: State, now: u64) -> String {
    let mut parts: Vec<String> = Vec::new();
    match (w.status.as_deref(), w.percent) {
        (Some("rejected"), _) => parts.push(status_words("rejected").to_string()),
        (_, Some(_)) => parts.push(format!("{} used", w.figure(now))),
        (Some(s), None) => parts.push(status_words(s).to_string()),
        _ => {}
    }
    if let Some(r) = w.resets_at {
        parts.push(resets_words(r, now));
    }
    let stale = if state == State::Stale { " (stale)" } else { "" };
    parts.push(format!("as of {}{stale}", clock(w.as_of, now)));
    parts.push(format!("from {}", source_words(w.source)));
    parts.join(" · ")
}

pub fn status_words(status: &str) -> &'static str {
    match status {
        "allowed_warning" => "near its limit",
        "rejected" => "limit reached",
        _ => "within its limit",
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// The sources

/// A time in ms from epoch seconds, epoch ms or RFC 3339.
fn ms_of(v: &Value) -> Option<u64> {
    match v {
        Value::Number(n) => n.as_f64().filter(|s| *s > 0.0).map(|s| if s < 10_000_000_000.0 { (s * 1000.0) as u64 } else { s as u64 }),
        Value::String(s) => chrono::DateTime::parse_from_rfc3339(s).ok().map(|d| d.timestamp_millis().max(0) as u64),
        _ => None,
    }
}

/// A window's name from the kind Claude Code reports: five_hour is "5h", seven_day_opus "7d opus".
fn kind_name(kind: &str) -> String {
    match kind {
        "five_hour" => "5h".into(),
        "seven_day" => "7d".into(),
        "seven_day_opus" => "7d opus".into(),
        "seven_day_sonnet" => "7d sonnet".into(),
        "seven_day_overage_included" => "7d extra".into(),
        "overage" => "extra".into(),
        other => other.replace('_', " "),
    }
}

/// A window's name from its length in minutes: 300 is "5h", 10080 "7d".
fn minutes_name(minutes: u64) -> String {
    match minutes {
        m if m > 0 && m % 1440 == 0 => format!("{}d", m / 1440),
        m if m > 0 && m % 60 == 0 => format!("{}h", m / 60),
        m => format!("{m}m"),
    }
}

/// How long a window named "5h" or "7d opus" lasts.
fn length_of(name: &str) -> Option<u64> {
    let head = name.split_whitespace().next()?;
    let (number, unit) = head.split_at(head.find(|c: char| !c.is_ascii_digit())?);
    let n: u64 = number.parse().ok()?;
    match unit {
        "m" => Some(n * 60_000),
        "h" => Some(n * HOUR),
        "d" => Some(n * DAY),
        _ => None,
    }
}

/// The plugin's limits.json, `{at, limits: [{name, percent, resetsAt}]}`, as Claude Code's status line gave them.
pub fn from_statusline(file: &Value) -> Vec<Window> {
    let Some(at) = file["at"].as_u64() else { return Vec::new() };
    file["limits"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|l| {
            let raw = l["name"].as_str().filter(|n| !n.is_empty())?;
            let name = if length_of(raw).is_some() { raw.to_string() } else { kind_name(raw) };
            Some(Window {
                agent: agent_of(STATUSLINE),
                length: length_of(&name),
                name,
                percent: l["percent"].as_f64(),
                status: None,
                resets_at: ms_of(&l["resetsAt"]),
                as_of: at,
                source: STATUSLINE,
            })
        })
        .collect()
}

/// A driven session's last `rate_limit_info`: a window per entry of `unifiedWindows` when it carries them, else its
/// one window, which may hold a verdict and no percent. Utilization is a fraction of the limit.
pub fn from_stream(info: &Value, as_of: u64) -> Vec<Window> {
    let window = |kind: &str, w: &Value| {
        let name = kind_name(kind);
        Window {
            agent: agent_of(STREAM),
            length: length_of(&name),
            name,
            percent: w["utilization"].as_f64().map(|u| u * 100.0),
            status: w["status"].as_str().map(str::to_string),
            resets_at: ms_of(&w["resetsAt"]),
            as_of,
            source: STREAM,
        }
    };
    if let Some(windows) = info["unifiedWindows"].as_object().filter(|w| !w.is_empty()) {
        return windows.iter().filter(|(_, w)| w.is_object()).map(|(kind, w)| window(kind, w)).collect();
    }
    if !info.is_object() || (info["status"].is_null() && info["utilization"].is_null()) {
        return Vec::new();
    }
    vec![window(info["rateLimitType"].as_str().unwrap_or("limit"), info)]
}

/// What a Codex rollout says of its limits: the windows, and the plan and credits beside them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Codex {
    pub windows: Vec<Window>,
    /// "plus", "pro"…, when the rollout says.
    pub plan: Option<String>,
    pub credits: Value,
}

/// The limit bucket a rollout's `rate_limits` belong to: "codex" (the plan's own) or another one Codex names.
fn bucket(limits: &Value) -> String {
    limits["limit_id"].as_str().filter(|id| !id.is_empty()).unwrap_or("codex").to_string()
}

/// A rollout's `token_count` record, when it carries `rate_limits`: its primary and secondary windows, named after
/// their bucket when it is not the plan's own.
pub fn from_rollout(line: &Value) -> Option<Codex> {
    let limits = &line["payload"]["rate_limits"];
    if !limits.is_object() {
        return None;
    }
    let as_of = ms_of(&line["timestamp"])?;
    let id = bucket(limits);
    let label = if id == "codex" { None } else { Some(limits["limit_name"].as_str().filter(|n| !n.is_empty()).unwrap_or(&id).to_string()) };
    let windows = ["primary", "secondary"]
        .iter()
        .map(|k| &limits[*k])
        .filter(|w| w.is_object())
        .map(|w| {
            let minutes = w["window_minutes"].as_u64();
            let name = minutes.map(minutes_name).unwrap_or_else(|| "limit".into());
            let resets_in = w["resets_in_seconds"].as_u64().map(|secs| as_of + secs * 1000);
            Window {
                agent: agent_of(ROLLOUT),
                name: label.as_ref().map_or_else(|| name.clone(), |l| format!("{name} {l}")),
                percent: w["used_percent"].as_f64(),
                status: None,
                resets_at: ms_of(&w["resets_at"]).or(resets_in),
                length: minutes.map(|m| m * 60_000),
                as_of,
                source: ROLLOUT,
            }
        })
        .collect();
    Some(Codex { windows, plan: limits["plan_type"].as_str().filter(|p| !p.is_empty()).map(str::to_string), credits: limits["credits"].clone() })
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// The last `token_count` with rate limits of each bucket at a rollout's end, looking further back when its last part
/// has none: one reading per bucket, so two buckets never take turns.
pub fn codex_reading(path: &Path) -> Option<Codex> {
    for bytes in [256 * 1024, 4 * 1024 * 1024] {
        let tail = transcripts::tail(path, bytes)?;
        let mut seen: Vec<String> = Vec::new();
        let mut out: Option<Codex> = None;
        for line in tail.split(|b| *b == b'\n').rev().filter(|l| contains(l, b"\"rate_limits\"")) {
            let Ok(v) = serde_json::from_slice::<Value>(line) else { continue };
            let Some(reading) = from_rollout(&v) else { continue };
            let id = bucket(&v["payload"]["rate_limits"]);
            if seen.contains(&id) {
                continue;
            }
            seen.push(id);
            match &mut out {
                Some(codex) => codex.windows.extend(reading.windows),
                None => out = Some(reading),
            }
        }
        if out.is_some() {
            return out;
        }
    }
    None
}

/// The newest readings, one per window: a reading past its reset says nothing any more, and a verdict newer than the
/// last percent goes beside that percent rather than replacing it.
pub fn merge(mut readings: Vec<Window>, now: u64) -> Vec<Window> {
    readings.retain(|w| match w.resets_at {
        Some(r) => r > now,
        None => now.saturating_sub(w.as_of) < w.length.unwrap_or(6 * HOUR),
    });
    readings.sort_by_key(|w| std::cmp::Reverse(w.as_of));
    let mut out: Vec<Window> = Vec::new();
    for w in &readings {
        if out.iter().any(|o| o.agent == w.agent && o.name == w.name) {
            continue;
        }
        let same: Vec<&Window> = readings.iter().filter(|o| o.agent == w.agent && o.name == w.name).collect();
        let mut best = same.iter().find(|o| o.percent.is_some()).copied().unwrap_or(w).clone();
        if let Some(status) = same.iter().filter(|o| o.as_of >= best.as_of).find_map(|o| o.status.clone()) {
            best.status = Some(status);
        }
        out.push(best);
    }
    out
}

/// Tightest first: the most used, then the soonest to reset.
pub fn tightest_first(windows: &mut [Window]) {
    windows.sort_by(|a, b| b.tightness().total_cmp(&a.tightness()).then(a.resets_at.unwrap_or(u64::MAX).cmp(&b.resets_at.unwrap_or(u64::MAX))));
}

// ---------------------------------------------------------------------------------------------------------------------
// What is read, and from where

/// Everything the providers are built from, read at once; tests and the demo world fill it themselves.
#[derive(Clone, Debug, Default)]
pub struct Inputs {
    /// limits.json as written, or null.
    pub statusline: Value,
    /// Why limits.json could not be read, when it is there.
    pub statusline_error: Option<String>,
    pub stream: Vec<Window>,
    pub codex: Option<Codex>,
    /// Copilot's windows and plan, from GitHub through `gh`.
    pub copilot: Option<Codex>,
    /// What each `usage.commands` command printed.
    pub commanded: Vec<Window>,
    /// Claude Code's `oauthAccount`: null when its file is missing, `false` when it holds no account.
    pub claude_account: Value,
    /// The agents installed here, by id.
    pub installed: Vec<String>,
}

static STREAM_FN: OnceLock<fn() -> Vec<Value>> = OnceLock::new();

/// The hub hands over how to list the sessions it drives (their snapshots, each with `key` and `rate_limit`).
pub fn stream_from(sessions: fn() -> Vec<Value>) {
    let _ = STREAM_FN.set(sessions);
}

/// A value kept between calls, made on first use.
type Kept<T> = OnceLock<Mutex<T>>;
/// A file's size and modification time: it changed when either did.
type Stamp = (u64, u128);

fn lock<T: Default>(m: &'static OnceLock<Mutex<T>>) -> MutexGuard<'static, T> {
    m.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner())
}

/// The driven sessions' limits, each as of when its session last reported it; a session that keeps no such time gives
/// the time cockpit first saw that reading.
fn stream_readings(now: u64) -> Vec<Window> {
    static SEEN: OnceLock<Mutex<HashMap<String, (String, u64)>>> = OnceLock::new();
    let Some(sessions) = STREAM_FN.get() else { return Vec::new() };
    let mut out = Vec::new();
    for s in sessions() {
        out.extend(stream_of(&s, now, &mut lock(&SEEN)));
    }
    out
}

/// One session's limits from its snapshot (`rate_limit`, `rate_limit_at`).
fn stream_of(session: &Value, now: u64, seen: &mut HashMap<String, (String, u64)>) -> Vec<Window> {
    let info = &session["rate_limit"];
    if !info.is_object() {
        return Vec::new();
    }
    let at = match session["rate_limit_at"].as_u64().filter(|at| *at > 0) {
        Some(at) => at,
        None => {
            let (key, text) = (session["key"].as_str().unwrap_or_default().to_string(), info.to_string());
            match seen.get(&key) {
                Some((was, at)) if *was == text => *at,
                _ => {
                    seen.insert(key, (text, now));
                    now
                }
            }
        }
    };
    from_stream(info, at)
}

fn stamp_of(meta: &std::fs::Metadata) -> Stamp {
    (meta.len(), meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos()).unwrap_or(0))
}

/// What limits.json says, read again only when it changed; a file caught half written keeps the last good value, and
/// only a file never read whole is an error.
fn statusline_file(path: &Path) -> (Value, Option<String>) {
    static LAST: Kept<Option<(Stamp, Value)>> = OnceLock::new();
    let Ok(meta) = std::fs::metadata(path) else { return (Value::Null, None) };
    let stamp = stamp_of(&meta);
    let mut last = lock(&LAST);
    if let Some((was, value)) = last.as_ref()
        && *was == stamp
    {
        return (value.clone(), None);
    }
    match std::fs::read(path).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()) {
        Some(value) => {
            *last = Some((stamp, value.clone()));
            (value, None)
        }
        None => match last.as_ref() {
            Some((_, value)) => (value.clone(), None),
            None => (Value::Null, Some("limits.json could not be read".to_string())),
        },
    }
}

/// Claude Code's own settings file, which names the account it is signed in with.
fn claude_json() -> PathBuf {
    match std::env::var_os("CLAUDE_CONFIG_DIR").filter(|d| !d.is_empty()) {
        Some(dir) => PathBuf::from(dir).join(".claude.json"),
        None => files::home().join(".claude.json"),
    }
}

/// Claude Code's settings file as far as cockpit keeps it: the account's address and organization, everything else
/// passed over by the parser and dropped.
#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct ClaudeFile {
    oauth_account: Option<OauthAccount>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct OauthAccount {
    email_address: Option<String>,
    organization_name: Option<String>,
}

/// Only the account's address and organization: nothing else of that file is kept, and it is read again only when it
/// changed. Null without the file, false when it holds no account.
fn claude_account() -> Value {
    static LAST: Kept<Option<(Stamp, Value)>> = OnceLock::new();
    let Ok(meta) = std::fs::metadata(claude_json()) else { return Value::Null };
    let stamp = stamp_of(&meta);
    let mut last = lock(&LAST);
    if let Some((was, value)) = last.as_ref()
        && *was == stamp
    {
        return value.clone();
    }
    let Some(value) = std::fs::read(claude_json()).ok().and_then(|b| account_of(&b)) else {
        return last.as_ref().map(|(_, v)| v.clone()).unwrap_or(Value::Null);
    };
    *last = Some((stamp, value.clone()));
    value
}

/// `{emailAddress, organizationName}` from the file's text, false when it names no account, None when it is no JSON.
fn account_of(bytes: &[u8]) -> Option<Value> {
    let file = serde_json::from_slice::<ClaudeFile>(bytes).ok()?;
    Some(match file.oauth_account {
        Some(OauthAccount { email_address: Some(email), organization_name }) => json!({"emailAddress": email, "organizationName": organization_name}),
        _ => Value::Bool(false),
    })
}

/// The rollouts of the last 8 days' folders (`sessions/YYYY/MM/DD`), newest first: only those folders are listed.
fn rollouts(now: u64) -> Vec<(PathBuf, Stamp)> {
    let Some(root) = transcripts::root(Transcripts::Codex) else { return Vec::new() };
    let mut out: Vec<(PathBuf, Stamp, u128)> = Vec::new();
    for back in 0..=8u64 {
        let Some(day) = local(now.saturating_sub(back * DAY)) else { continue };
        let Ok(entries) = std::fs::read_dir(root.join(day.format("%Y/%m/%d").to_string())) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !(name.starts_with("rollout-") && name.ends_with(".jsonl")) {
                continue;
            }
            if let Ok(meta) = entry.metadata() {
                let stamp = stamp_of(&meta);
                out.push((entry.path(), stamp, stamp.1));
            }
        }
    }
    out.sort_by_key(|(_, _, modified)| std::cmp::Reverse(*modified));
    out.into_iter().map(|(path, stamp, _)| (path, stamp)).collect()
}

/// Every file under `dir` whose name `wanted` takes, `depth` folders deep at most, with its modification time in ms;
/// only those files are looked at, and tool-results folders are skipped.
fn walk(dir: &Path, depth: usize, wanted: &dyn Fn(&str) -> bool, found: &mut dyn FnMut(&Path, u64)) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else { continue };
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if kind.is_dir() {
            if depth > 0 && name != "tool-results" {
                walk(&entry.path(), depth - 1, wanted, found);
            }
        } else if kind.is_file() && wanted(&name) {
            let modified = entry.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok());
            found(&entry.path(), modified.map(|d| d.as_millis() as u64).unwrap_or(0));
        }
    }
}

/// The newest rollouts' limits: each window's newest reading among the latest few rollouts (a bucket may have been read
/// in an older one), the plan and credits of the newest; a file is read again only when it changed.
fn codex_now(now: u64) -> Option<Codex> {
    static READ: Kept<HashMap<PathBuf, (Stamp, Option<Codex>)>> = OnceLock::new();
    let files = rollouts(now);
    let mut read = lock(&READ);
    read.retain(|path, _| files.iter().any(|(f, _)| f == path));
    let mut out: Option<Codex> = None;
    for (path, stamp) in files.iter().take(6) {
        let reading = match read.get(path) {
            Some((was, reading)) if was == stamp => reading.clone(),
            _ => {
                let reading = codex_reading(path);
                read.insert(path.clone(), (*stamp, reading.clone()));
                reading
            }
        };
        let Some(reading) = reading else { continue };
        let Some(codex) = &mut out else {
            out = Some(reading);
            continue;
        };
        for w in reading.windows {
            match codex.windows.iter_mut().find(|o| o.name == w.name) {
                Some(older) if older.as_of < w.as_of => *older = w,
                Some(_) => {}
                None => codex.windows.push(w),
            }
        }
    }
    out
}

/// Copilot's quota as GitHub keeps it (`gh api /copilot_internal/user`, the sign-in `gh` already has), asked at most once
/// per `EVERY`; None when `gh` is missing, signed out, or the account has no Copilot plan.
fn copilot_now(now: u64) -> Option<Codex> {
    static LAST: Kept<Option<(u64, Option<Codex>)>> = OnceLock::new();
    let mut last = lock(&LAST);
    if let Some((at, reading)) = last.as_ref()
        && now.saturating_sub(*at) < EVERY
    {
        return reading.clone();
    }
    let out =
        crate::run::output(std::process::Command::new("gh").args(["api", "/copilot_internal/user"]), Duration::from_secs(10), COMMAND_CAP, &Default::default());
    let reading = out.ok().filter(|o| o.status.success()).and_then(|o| serde_json::from_slice::<Value>(&o.stdout).ok()).and_then(|v| from_copilot(&v, now));
    *last = Some((now, reading.clone()));
    reading
}

/// Copilot's limited quotas from GitHub's answer: each one used up to its percent, starting over on the reset date.
pub fn from_copilot(user: &Value, now: u64) -> Option<Codex> {
    let resets_at =
        user["quota_reset_date_utc"].as_str().and_then(|d| chrono::DateTime::parse_from_rfc3339(d).ok()).map(|d| d.timestamp_millis().max(0) as u64).or_else(
            || {
                let day = chrono::NaiveDate::parse_from_str(user["quota_reset_date"].as_str()?, "%Y-%m-%d").ok()?;
                Some(day.and_hms_opt(0, 0, 0)?.and_utc().timestamp_millis().max(0) as u64)
            },
        );
    let names = [("premium_interactions", "premium"), ("chat", "chat"), ("completions", "completions")];
    let windows: Vec<Window> = names
        .iter()
        .filter_map(|(key, name)| {
            let q = &user["quota_snapshots"][key];
            let limited = q["unlimited"] == false && q["entitlement"].as_f64().is_some_and(|e| e > 0.0);
            limited.then(|| Window {
                agent: "copilot".into(),
                name: (*name).into(),
                percent: q["percent_remaining"].as_f64().map(|left| (100.0 - left).clamp(0.0, 100.0)),
                status: None,
                resets_at,
                length: Some(30 * DAY),
                as_of: now,
                source: COPILOT,
            })
        })
        .collect();
    // GitHub's words, shown in the window: cleaned of escape and unseen characters, and short.
    let plan = user["copilot_plan"].as_str().map(|p| crate::text::clean(p, 32)).filter(|p| !p.is_empty());
    (!windows.is_empty() || plan.is_some()).then_some(Codex { windows, plan, credits: Value::Null })
}

/// The most of a command's output read: a usage answer is a few lines.
const COMMAND_CAP: usize = 64 * 1024;

/// Each agent's windows as the command `usage.commands.<agent id>` prints them, run at most once per `EVERY`: a JSON
/// list (or `{windows: [...]}`) of `{name, percent, resets_at}`, `resets_at` in seconds, ms or RFC 3339. The commands
/// run side by side, each stopped past 10 s, so a slow one holds the usage thread no longer than that.
fn commanded_now(now: u64) -> Vec<Window> {
    static LAST: Kept<Option<(u64, Vec<Window>)>> = OnceLock::new();
    let mut last = lock(&LAST);
    if let Some((at, windows)) = last.as_ref()
        && now.saturating_sub(*at) < EVERY
    {
        return windows.clone();
    }
    let config = files::config();
    let commands = config["usage"]["commands"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(agent, command)| Some((agent.clone(), command.as_str().filter(|c| !c.trim().is_empty())?.to_string())))
        .collect();
    let windows = run_commands(commands, now);
    *last = Some((now, windows.clone()));
    windows
}

/// Runs each agent's usage command side by side and reads the windows each printed.
fn run_commands(commands: Vec<(String, String)>, now: u64) -> Vec<Window> {
    let runs: Vec<_> = commands
        .into_iter()
        .map(|(agent, command)| {
            std::thread::spawn(move || {
                let out =
                    crate::run::output(std::process::Command::new("sh").args(["-c", &command]), Duration::from_secs(10), COMMAND_CAP, &Default::default());
                (agent, out.ok().filter(|o| o.status.success()).and_then(|o| serde_json::from_slice::<Value>(&o.stdout).ok()))
            })
        })
        .collect();
    let mut windows = Vec::new();
    for run in runs {
        if let Ok((agent, Some(printed))) = run.join() {
            windows.extend(from_command(&agent, &printed, now));
        }
    }
    windows
}

/// The windows a usage command printed for `agent`, those with a name and a percent from 0 to 100.
pub fn from_command(agent: &str, printed: &Value, now: u64) -> Vec<Window> {
    let list = printed.as_array().or_else(|| printed["windows"].as_array()).cloned().unwrap_or_default();
    list.iter()
        .filter_map(|w| {
            let name: String = w["name"].as_str()?.chars().filter(|c| !c.is_control()).take(16).collect();
            let percent = w["percent"].as_f64().filter(|p| (0.0..=100.0).contains(p))?;
            (!name.trim().is_empty()).then(|| Window {
                agent: agent.to_string(),
                length: length_of(&name),
                name,
                percent: Some(percent),
                status: None,
                resets_at: ms_of(&w["resets_at"]),
                as_of: now,
                source: COMMAND,
            })
        })
        .collect()
}

/// The installed agents, looked for again every few minutes.
fn installed(now: u64) -> Vec<String> {
    static LAST: Kept<Option<(u64, Vec<String>)>> = OnceLock::new();
    let mut last = lock(&LAST);
    if let Some((at, ids)) = last.as_ref()
        && now.saturating_sub(*at) < 5 * 60_000
    {
        return ids.clone();
    }
    let ids: Vec<String> = agent::all().into_iter().filter(|a| a.installed()).map(|a| a.id).collect();
    *last = Some((now, ids.clone()));
    ids
}

/// The demo world's `usage` block (COCKPIT_AGENTS_DEMO), ages and resets given in minutes from now.
fn demo(now: u64) -> Option<Inputs> {
    let path = std::env::var_os("COCKPIT_AGENTS_DEMO").filter(|p| !p.is_empty())?;
    let world = files::load(Path::new(&path))?;
    let u = world.get("usage").filter(|u| u.is_object())?;
    let ago = |v: &Value| now.saturating_sub((v.as_f64().unwrap_or(0.0) * 60_000.0) as u64);
    let ahead = |v: &Value| now + (v.as_f64().unwrap_or(0.0) * 60_000.0) as u64;
    let statusline = match u["statusline"].as_object() {
        Some(s) => {
            let limits: Vec<Value> = s
                .get("limits")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|l| json!({"name": l["name"], "percent": l["percent"], "resetsAt": ahead(&l["resets_in_min"])}))
                .collect();
            json!({"at": ago(&s.get("at_min").cloned().unwrap_or_default()), "limits": limits})
        }
        None => Value::Null,
    };
    let codex = u["codex"].as_object().and_then(|c| {
        let window = |w: Option<&Value>| {
            w.filter(|w| w.is_object())
                .map(|w| json!({"used_percent": w["used_percent"], "window_minutes": w["window_minutes"], "resets_at": ahead(&w["resets_in_min"]) / 1000}))
        };
        let at = ago(&c.get("at_min").cloned().unwrap_or_default());
        let stamp = Local.timestamp_millis_opt(at as i64).single()?.to_rfc3339();
        let limits =
            json!({"primary": window(c.get("primary")), "secondary": window(c.get("secondary")), "plan_type": c.get("plan_type"), "credits": c.get("credits")});
        from_rollout(&json!({"timestamp": stamp, "payload": {"type": "token_count", "rate_limits": limits}}))
    });
    // A driven session's `rate_limit_info`, its reset and its time given in minutes from now.
    let stream = match u["stream"].as_object() {
        Some(s) => {
            let mut info = Value::Object(s.clone());
            info["resetsAt"] = json!(ahead(&s.get("resets_in_min").cloned().unwrap_or_default()) / 1000);
            from_stream(&info, ago(&s.get("at_min").cloned().unwrap_or_default()))
        }
        None => Vec::new(),
    };
    let installed = u["installed"].as_array().into_iter().flatten().filter_map(|a| a.as_str().map(str::to_string)).collect();
    Some(Inputs { statusline, statusline_error: None, stream, codex, claude_account: u["claude_account"].clone(), installed, ..Inputs::default() })
}

/// What the sources say: the demo world's when one is set, else the files, each read again only when it changed.
fn gather(now: u64) -> Inputs {
    if let Some(inputs) = demo(now) {
        return inputs;
    }
    let (statusline, statusline_error) = statusline_file(&files::cache().join("limits.json"));
    let installed = installed(now);
    let copilot = if installed.iter().any(|a| a == "copilot") { copilot_now(now) } else { None };
    Inputs {
        statusline,
        statusline_error,
        stream: Vec::new(),
        codex: codex_now(now),
        copilot,
        commanded: commanded_now(now),
        claude_account: claude_account(),
        installed,
    }
}

fn kept() -> MutexGuard<'static, Option<Inputs>> {
    static INPUTS: Kept<Option<Inputs>> = OnceLock::new();
    lock(&INPUTS)
}

/// Reads the sources again and keeps what they say: the usage thread's job, so a view build only reads memory.
pub fn refresh(now: u64) {
    let inputs = gather(now);
    *kept() = Some(inputs);
}

/// What the sources said at the last refresh, with the driven sessions' limits as they are now; the sources are read
/// once when no refresh ran yet.
pub fn read(now: u64) -> Inputs {
    let known = kept().clone();
    let mut inputs = match known {
        Some(inputs) => inputs,
        None => {
            refresh(now);
            kept().clone().unwrap_or_default()
        }
    };
    inputs.stream.extend(stream_readings(now));
    inputs
}

// ---------------------------------------------------------------------------------------------------------------------
// Providers

/// One agent as the usage surfaces list it: its windows and what is known of its account.
#[derive(Clone, Debug, PartialEq)]
pub struct Provider {
    pub agent: String,
    /// "Claude", or the agent's own name when it reports nothing.
    pub name: String,
    pub letter: String,
    /// Whether the agent has a usage source this machine can read.
    pub reports: bool,
    /// Its source read an account whose plan limits nothing it reports.
    pub unlimited: bool,
    /// In the order the bars draw them: shortest window first.
    pub windows: Vec<Window>,
    /// `{signed_in, account, org, plan, words}`, or null when nothing says.
    pub identity: Value,
    pub credits: Value,
    /// Why its source could not be read.
    pub error: Option<String>,
}

impl Provider {
    /// fresh, stale, none (no recent session), unreported (no source this machine can read), unlimited (its plan limits
    /// nothing) or unavailable (it failed).
    pub fn state(&self, now: u64) -> &'static str {
        if !self.reports {
            "unreported"
        } else if self.windows.is_empty() && self.unlimited {
            "unlimited"
        } else if self.windows.is_empty() && self.error.is_some() {
            "unavailable"
        } else if self.windows.is_empty() {
            "none"
        } else if self.windows.iter().any(|w| w.state(now) == State::Fresh) {
            "fresh"
        } else {
            "stale"
        }
    }

    pub fn words(&self, now: u64) -> String {
        let newest = self.windows.iter().map(|w| w.as_of).max().unwrap_or(0);
        match self.state(now) {
            "unreported" => "Not reported by this agent".into(),
            "unlimited" => "Unlimited on this plan".into(),
            "unavailable" => format!("Usage unavailable: {}", self.error.as_deref().unwrap_or("its source could not be read")),
            "none" => "No recent reading".into(),
            "stale" => format!("as of {} · stale", clock(newest, now)),
            _ => format!("as of {}", clock(newest, now)),
        }
    }

    pub fn tightest(&self) -> Option<&Window> {
        let mut sorted: Vec<&Window> = self.windows.iter().collect();
        sorted.sort_by(|a, b| b.tightness().total_cmp(&a.tightness()).then(a.resets_at.unwrap_or(u64::MAX).cmp(&b.resets_at.unwrap_or(u64::MAX))));
        sorted.first().copied()
    }

    pub fn json(&self, now: u64) -> Value {
        json!({
            "agent": self.agent, "name": self.name, "icon": agent::icon_in(&[], &self.agent), "letter": self.letter, "reports": self.reports,
            "state": self.state(now), "words": self.words(now), "identity": self.identity, "credits": self.credits,
            "error": self.error, "windows": self.windows.iter().map(|w| w.json(now)).collect::<Vec<_>>(),
            "tightest": self.tightest().map(|w| w.json(now)),
        })
    }
}

/// The account of the agent on this machine as its own files tell it, in the agents table's words. It is shown beside
/// the windows, never as theirs: a reading can come from a session signed in elsewhere.
fn identity_of(agent: &str, inputs: &Inputs) -> Value {
    let Some(reports) = agent::reports(agent) else { return Value::Null };
    if has_source(Some(reports), STATUSLINE) {
        return match &inputs.claude_account {
            Value::Object(o) => {
                let account = o.get("emailAddress").and_then(Value::as_str).unwrap_or_default();
                let org = o.get("organizationName").and_then(Value::as_str).filter(|s| !s.is_empty());
                let who = org.map(|org| format!("{account} · {org}")).unwrap_or_else(|| account.to_string());
                json!({"signed_in": true, "account": account, "org": org, "plan": null, "words": reports.account.replace("{account}", &who)})
            }
            Value::Bool(false) => json!({"signed_in": false, "account": null, "org": null, "plan": null, "words": reports.signed_out}),
            _ => Value::Null,
        };
    }
    let reading = if has_source(Some(reports), ROLLOUT) {
        inputs.codex.as_ref()
    } else if has_source(Some(reports), COPILOT) {
        inputs.copilot.as_ref()
    } else {
        None
    };
    if let Some(plan) = reading.and_then(|c| c.plan.as_deref()) {
        let mut name: Vec<char> = plan.chars().collect();
        if let Some(first) = name.first_mut() {
            *first = first.to_ascii_uppercase();
        }
        let plan: String = name.into_iter().collect();
        return json!({"signed_in": true, "account": null, "org": null, "plan": plan, "words": reports.account.replace("{plan}", &plan)});
    }
    Value::Null
}

/// Every agent worth listing, in the agents table's order: the ones with a reading or a source and installed, then the
/// installed ones that report nothing.
pub fn providers(inputs: &Inputs, now: u64) -> Vec<Provider> {
    let mut readings = from_statusline(&inputs.statusline);
    readings.extend(inputs.stream.iter().cloned());
    for reading in [&inputs.codex, &inputs.copilot].into_iter().flatten() {
        readings.extend(reading.windows.iter().cloned());
    }
    readings.extend(inputs.commanded.iter().cloned());
    let windows = merge(readings, now);
    let mut out: Vec<Provider> = Vec::new();
    for a in agent::all() {
        let reports = agent::reports(&a.id);
        let mut mine: Vec<Window> = windows.iter().filter(|w| w.agent == a.id).cloned().collect();
        if mine.is_empty() && !inputs.installed.contains(&a.id) {
            continue;
        }
        mine.sort_by(|x, y| x.length.unwrap_or(u64::MAX).cmp(&y.length.unwrap_or(u64::MAX)).then(x.name.cmp(&y.name)));
        let error = if has_source(reports, STATUSLINE) { inputs.statusline_error.clone() } else { None };
        let credits = if has_source(reports, ROLLOUT) { inputs.codex.as_ref().map(|c| c.credits.clone()).unwrap_or_default() } else { Value::Null };
        // GitHub answers through `gh`: missing or signed out, Copilot reports nothing here.
        let copilot = has_source(reports, COPILOT);
        out.push(Provider {
            name: reports.map(|r| r.short.to_string()).unwrap_or_else(|| a.name.clone()),
            letter: reports.map(|r| r.letter.to_string()).unwrap_or_else(|| a.name.chars().take(1).collect()),
            reports: reports.is_some() && (!copilot || !mine.is_empty() || inputs.copilot.is_some()),
            unlimited: copilot && inputs.copilot.as_ref().is_some_and(|c| c.windows.is_empty() && c.plan.is_some()),
            identity: identity_of(&a.id, inputs),
            agent: a.id,
            windows: mine,
            credits,
            error,
        });
    }
    // Those with a reading first, then those that could report, then the rest; the table's order within each.
    out.sort_by_key(|p| {
        if !p.windows.is_empty() {
            0
        } else if p.reports {
            1
        } else {
            2
        }
    });
    out
}

/// What an agent's row says of its limits: `reports` (bars, or a reading to come), `unreported` (no source this machine
/// can read) or `unlimited` (its plan limits nothing it reports).
pub fn limits_of(providers: &[Provider], agent: &str, now: u64) -> &'static str {
    match providers.iter().find(|p| p.agent == agent).map(|p| p.state(now)) {
        Some(state @ ("unreported" | "unlimited")) => state,
        Some(_) => "reports",
        None if agent::reports(agent).is_some() => "reports",
        None => "unreported",
    }
}

/// The room the `$limits` text has on herdr's sidebar (34 columns), after its icon.
const TOKEN_ROOM: usize = 28;

/// The `$limits` text on the herdr sidebar, grouped by agent: its letter, then each window as its figure reads,
/// `C 64%·5h 38%·7d  X ≥12%·7d` (`C full·5h` once a limit is reached); each agent's tightest window alone when that runs
/// past the sidebar's room; none when no agent reports a window.
pub fn token(providers: &[Provider], now: u64) -> Option<String> {
    let line = |every: bool| {
        let parts: Vec<String> = providers
            .iter()
            .filter(|p| !p.windows.is_empty())
            .map(|p| {
                let shown: Vec<&Window> = if every { p.windows.iter().collect() } else { p.tightest().into_iter().collect() };
                let figures: Vec<String> =
                    shown.iter().map(|w| format!("{}·{}", w.figure(now), w.name.split_whitespace().collect::<Vec<_>>().join("-"))).collect();
                format!("{} {}", p.letter, figures.join(" "))
            })
            .collect();
        parts.join("  ")
    };
    let grouped = line(true);
    let text = if grouped.chars().count() <= TOKEN_ROOM { grouped } else { line(false) };
    (!text.is_empty()).then(|| format!("\u{f029a}  {text}"))
}

/// The agents view's `quota`: every window as a bar, agent by agent, shortest window first.
pub fn quota(providers: &[Provider], now: u64) -> Value {
    Value::Array(providers.iter().flat_map(|p| p.windows.iter().map(|w| w.json(now))).collect())
}

/// The `usage` view: the agents with their windows and accounts, every window tightest first, the sidebar token, and
/// the last days' tokens and estimated spend (null while the first count runs).
pub fn view_of(inputs: &Inputs, days: Option<&Days>, now: u64) -> Value {
    let list = providers(inputs, now);
    let mut all: Vec<Window> = list.iter().flat_map(|p| p.windows.iter().cloned()).collect();
    tightest_first(&mut all);
    json!({
        "at": now,
        "providers": list.iter().map(|p| p.json(now)).collect::<Vec<_>>(),
        "windows": all.iter().map(|w| w.json(now)).collect::<Vec<_>>(),
        "tightest": all.first().map(|w| w.json(now)),
        "token": token(&list, now),
        "sources": crate::header::json(&sources(&list, now), now),
        "loading": days.is_none(),
        "tokens": days.map(|d| d.json(now)),
    })
}

/// Each agent's windows as one header source chip, grouped as the bars are ("Claude 5h 7d · 2 min ago", "Codex 7d ·
/// stale, 3 h ago"), as fresh as its oldest reading; an agent whose source could not be read as a failing one. Their
/// ids start with `usage-`, which `sources refresh` reads.
pub fn sources(providers: &[Provider], now: u64) -> Vec<crate::header::Source> {
    let mut out = Vec::new();
    for p in providers {
        if let Some(oldest) = p.windows.iter().min_by_key(|w| w.as_of) {
            let names: Vec<&str> = p.windows.iter().map(|w| w.name.as_str()).collect();
            let fresh = Freshness { source: format!("usage-{}", p.agent), ..oldest.freshness() };
            out.push(crate::header::Source { fresh, label: format!("{} {}", p.name, names.join(" ")), login: None, elsewhere: None });
        }
        if let (true, Some(error)) = (p.windows.is_empty(), &p.error) {
            let mut fresh = Freshness { every: Some(EVERY), ..Freshness::new(&format!("usage-{}", p.agent)) };
            fresh.failed(now, crate::fresh::Kind::Other, error);
            out.push(crate::header::Source { fresh, label: p.name.clone(), login: None, elsewhere: None });
        }
    }
    out
}

/// The sources a view carries, back as the header draws them.
fn sources_of(view: &Value) -> Vec<crate::header::Source> {
    view["sources"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|s| crate::header::Source {
            fresh: Freshness::from_json(s),
            label: s["label"].as_str().unwrap_or_default().to_string(),
            login: s["login"].as_str().map(str::to_string),
            elsewhere: s["elsewhere"].as_u64(),
        })
        .collect()
}

/// The view from what the sources say now and the last count of tokens.
pub fn view(now: u64) -> Value {
    let days = result().clone();
    view_of(&read(now), days.as_ref(), now)
}

// ---------------------------------------------------------------------------------------------------------------------
// Words for times

fn local(ms: u64) -> Option<chrono::DateTime<Local>> {
    Local.timestamp_millis_opt(ms as i64).single()
}

/// "11:52" today, "Mon 11:52" this week, "3 Nov" before.
pub fn clock(ms: u64, now: u64) -> String {
    let (Some(t), Some(n)) = (local(ms), local(now)) else { return String::new() };
    let days = n.date_naive().signed_duration_since(t.date_naive()).num_days();
    match days {
        0 => t.format("%H:%M").to_string(),
        1..=6 => t.format("%a %H:%M").to_string(),
        _ => t.format("%-d %b").to_string(),
    }
}

/// "resets in 45 min", "resets in 2 h 10 min", "resets in 3 d", "reset" once past.
pub fn resets_words(at: u64, now: u64) -> String {
    if at <= now {
        return "reset".into();
    }
    let mins = (at - now).div_ceil(60_000);
    match mins {
        0..=59 => format!("resets in {mins} min"),
        60..=599 if !mins.is_multiple_of(60) => format!("resets in {} h {} min", mins / 60, mins % 60),
        60..=2879 => format!("resets in {} h", mins / 60),
        _ => format!("resets in {} d", (mins + 720) / 1440),
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Tokens and spend per day, estimated

/// USD per million tokens (input, output, cache read), from the Claude API's price list; a cache write costs 1.25 times
/// the input for five minutes and twice it for an hour. A model missing here is counted in tokens, never priced.
const PRICES: &[(&str, f64, f64, f64)] = &[
    ("claude-fable-5-1", 10.0, 50.0, 0.25),
    ("claude-mythos-5-1", 10.0, 50.0, 0.25),
    ("claude-fable-5", 10.0, 50.0, 1.0),
    ("claude-mythos-5", 10.0, 50.0, 1.0),
    ("claude-opus-5-5", 4.0, 20.0, 0.20),
    ("claude-opus-5", 5.0, 25.0, 0.50),
    ("claude-opus-4-8", 5.0, 25.0, 0.50),
    ("claude-opus-4-7", 5.0, 25.0, 0.50),
    ("claude-opus-4-6", 5.0, 25.0, 0.50),
    ("claude-sonnet-5-5", 2.0, 10.0, 0.20),
    ("claude-sonnet-5", 2.0, 10.0, 0.20),
    ("claude-sonnet-4-6", 3.0, 15.0, 0.30),
    ("claude-haiku-5-5", 0.10, 0.50, 0.01),
    ("claude-haiku-4-5", 1.0, 5.0, 0.10),
];

/// The models whose fast mode has a known price, twice their standard one; fast mode elsewhere is not priced.
const FAST: &[&str] = &["claude-opus-5-5", "claude-opus-5"];

/// The table's name for a model: without a `[1m]`-style suffix or a date.
fn base_model(model: &str) -> &str {
    let model = model.split('[').next().unwrap_or(model);
    match model.rsplit_once('-') {
        Some((head, date)) if date.len() == 8 && date.chars().all(|c| c.is_ascii_digit()) => head,
        _ => model,
    }
}

/// Tokens of one agent on one day, and what the priced part of them would cost.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tokens {
    /// Input not read from the cache.
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    /// USD for the tokens of priced models.
    pub cost: f64,
    /// Tokens of models with no known price.
    pub unpriced: u64,
}

impl Tokens {
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_write
    }

    fn add(&mut self, o: &Tokens) {
        self.input += o.input;
        self.output += o.output;
        self.cache_read += o.cache_read;
        self.cache_write += o.cache_write;
        self.cost += o.cost;
        self.unpriced += o.unpriced;
    }

    pub fn json(&self) -> Value {
        json!({"input": self.input, "output": self.output, "cache_read": self.cache_read, "cache_write": self.cache_write,
               "total": self.total(), "cost": (self.cost * 100.0).round() / 100.0, "unpriced": self.unpriced})
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct CacheWrites {
    ephemeral_5m_input_tokens: u64,
    ephemeral_1h_input_tokens: u64,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ClaudeUsage {
    input_tokens: u64,
    output_tokens: u64,
    cache_read_input_tokens: u64,
    cache_creation_input_tokens: u64,
    cache_creation: Option<CacheWrites>,
    speed: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ClaudeMessage {
    id: Option<String>,
    model: Option<String>,
    usage: Option<ClaudeUsage>,
}

/// An assistant line of a Claude transcript, its content skipped unread.
#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct ClaudeLine {
    #[serde(rename = "type")]
    kind: String,
    timestamp: Option<String>,
    request_id: Option<String>,
    message: Option<ClaudeMessage>,
}

/// A Claude call's tokens, priced when the model is in the table.
fn claude_tokens(model: &str, u: &ClaudeUsage) -> Tokens {
    let (w5, w1) = match &u.cache_creation {
        Some(c) if c.ephemeral_5m_input_tokens + c.ephemeral_1h_input_tokens > 0 => (c.ephemeral_5m_input_tokens, c.ephemeral_1h_input_tokens),
        _ => (u.cache_creation_input_tokens, 0),
    };
    let mut t = Tokens { input: u.input_tokens, output: u.output_tokens, cache_read: u.cache_read_input_tokens, cache_write: w5 + w1, ..Tokens::default() };
    let fast = u.speed.as_deref() == Some("fast");
    match PRICES.iter().find(|(m, ..)| *m == base_model(model)).filter(|(m, ..)| !fast || FAST.contains(m)) {
        Some((m, input, output, read)) => {
            let prompt = t.input + t.cache_read + t.cache_write;
            // Haiku 5.5 costs five times more on a prompt past 100K tokens.
            let tier = if *m == "claude-haiku-5-5" && prompt > 100_000 { 5.0 } else { 1.0 };
            let speed = if fast { 2.0 } else { 1.0 };
            let per = |n: u64, usd: f64| n as f64 * usd / 1_000_000.0;
            t.cost = (per(t.input, *input) + per(t.output, *output) + per(t.cache_read, *read) + per(w5, input * 1.25) + per(w1, input * 2.0)) * tier * speed;
        }
        None => t.unpriced = t.total(),
    }
    t
}

/// The local date of an RFC 3339 time, "2026-11-11".
fn date_of(stamp: &str) -> Option<String> {
    let t = chrono::DateTime::parse_from_rfc3339(stamp).ok()?;
    Some(t.with_timezone(&Local).format("%Y-%m-%d").to_string())
}

/// The days counted, oldest first, today last.
pub fn dates(now: u64) -> Vec<String> {
    let Some(today) = local(now).map(|t| t.date_naive()) else { return Vec::new() };
    (0..DAYS as i64).rev().map(|d| (today - chrono::Duration::days(d)).format("%Y-%m-%d").to_string()).collect()
}

/// What one transcript added: read up to `offset`, per (agent, day).
#[derive(Default)]
struct Scanned {
    agent: String,
    /// The session it is the transcript of: its file's name.
    session: String,
    /// When it started, its first line's time: the older session owns the messages a fork repeats.
    started: String,
    len: u64,
    /// Its modification time when read, in ns: a file rewritten whole is read again.
    modified: u128,
    offset: u64,
    cells: HashMap<(String, String), Tokens>,
    /// (day, model) of the tokens it holds whose price is not known.
    unpriced: BTreeSet<(String, String)>,
    /// The records it counted and no other file may count again: Claude messages, Codex token counts.
    ids: Vec<u64>,
    /// USD of the priced messages it counted first, whatever their day: its session's estimate.
    usd: f64,
    /// Codex: the running totals so far (input, cached, output, cache writes) and the model in use.
    totals: [u64; 4],
    model: Option<String>,
    /// Gemini's latest token snapshot per message, retained across incremental scans.
    gemini_calls: HashMap<String, Value>,
}

#[derive(Default)]
struct Scanner {
    files: HashMap<PathBuf, Scanned>,
    /// Every record counted, hashed: a fork or a resume repeats its parent's messages and token counts.
    counted: HashSet<u64>,
    /// The session each message was charged to at an earlier count, from estimates.json: no other session takes it,
    /// whether its transcript is still there or not.
    owners: HashMap<u64, String>,
}

/// A record's hash, 53 bits so JSON keeps it exact.
fn hashed(parts: &[&[u8]]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    parts.hash(&mut h);
    h.finish() & ((1 << 53) - 1)
}

impl Scanned {
    fn json(&self, path: &Path) -> Value {
        let cells: Vec<Value> =
            self.cells.iter().map(|((agent, date), t)| json!([agent, date, t.input, t.output, t.cache_read, t.cache_write, t.cost, t.unpriced])).collect();
        json!({"path": path, "agent": self.agent, "session": self.session, "started": self.started, "len": self.len,
               "modified": self.modified.to_string(), "offset": self.offset, "cells": cells, "unpriced": self.unpriced, "ids": self.ids,
               "usd": self.usd, "totals": self.totals, "model": self.model, "gemini_calls": self.gemini_calls})
    }

    fn of(v: &Value) -> Option<(PathBuf, Scanned)> {
        if v["agent"] == "gemini" && !v["gemini_calls"].is_object() {
            return None;
        }
        let n = |v: &Value| v.as_u64().unwrap_or(0);
        let cells = v["cells"]
            .as_array()?
            .iter()
            .filter_map(|c| {
                let t = Tokens { input: n(&c[2]), output: n(&c[3]), cache_read: n(&c[4]), cache_write: n(&c[5]), cost: c[6].as_f64()?, unpriced: n(&c[7]) };
                Some(((c[0].as_str()?.to_string(), c[1].as_str()?.to_string()), t))
            })
            .collect();
        let unpriced = v["unpriced"].as_array()?.iter().filter_map(|u| Some((u[0].as_str()?.to_string(), u[1].as_str()?.to_string()))).collect();
        let totals: Vec<u64> = v["totals"].as_array()?.iter().map(n).collect();
        let f = Scanned {
            agent: v["agent"].as_str()?.to_string(),
            session: v["session"].as_str()?.to_string(),
            started: v["started"].as_str().unwrap_or_default().to_string(),
            len: n(&v["len"]),
            modified: v["modified"].as_str()?.parse().ok()?,
            offset: n(&v["offset"]),
            cells,
            unpriced,
            ids: v["ids"].as_array()?.iter().filter_map(Value::as_u64).collect(),
            usd: v["usd"].as_f64()?,
            totals: totals.try_into().ok()?,
            model: v["model"].as_str().map(str::to_string),
            gemini_calls: v["gemini_calls"].as_object().map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect()).unwrap_or_default(),
        };
        Some((PathBuf::from(v["path"].as_str()?), f))
    }
}

fn scan_path() -> PathBuf {
    files::cache().join("usage-scan.json")
}

/// The last hub's count, so a new hub reads only what the transcripts gained since, not every one again.
fn load_scan(path: &Path) -> Scanner {
    let mut s = Scanner::default();
    for (p, f) in files::load(path).and_then(|v| v["files"].as_array().cloned()).unwrap_or_default().iter().filter_map(Scanned::of) {
        s.counted.extend(f.ids.iter().copied());
        s.files.insert(p, f);
    }
    s
}

fn save_scan(s: &Scanner, path: &Path) {
    let files: Vec<Value> = s.files.iter().map(|(p, f)| f.json(p)).collect();
    let _ = files::write(path, &json!({"version": 1, "files": files}));
}

/// The tokens of the last days, per agent and day.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Days {
    pub at: u64,
    pub dates: Vec<String>,
    pub cells: BTreeMap<(String, String), Tokens>,
    /// Models seen on those days whose price is not known.
    pub unpriced: BTreeSet<String>,
}

impl Days {
    pub fn day(&self, agent: &str, date: &str) -> Tokens {
        self.cells.get(&(agent.to_string(), date.to_string())).cloned().unwrap_or_default()
    }

    fn agents(&self) -> Vec<String> {
        let mut seen: Vec<String> = Vec::new();
        for (agent, _) in self.cells.keys() {
            if !seen.contains(agent) {
                seen.push(agent.clone());
            }
        }
        seen
    }

    fn week_of(&self, agent: &str) -> Tokens {
        let mut t = Tokens::default();
        for date in &self.dates {
            t.add(&self.day(agent, date));
        }
        t
    }

    /// What the week's estimate covers when it is not everything: "Claude only" when another agent's tokens have no
    /// price, "Claude, some models without a price" when some of its own do; None when every token has a price.
    fn covers(&self) -> Option<String> {
        let weeks: Vec<(String, Tokens)> = self.agents().iter().map(|a| (short_name(a), self.week_of(a))).filter(|(_, w)| w.total() > 0).collect();
        let priced: Vec<&(String, Tokens)> = weeks.iter().filter(|(_, w)| w.total() > w.unpriced).collect();
        if priced.is_empty() || weeks.iter().all(|(_, w)| w.unpriced == 0) {
            return None;
        }
        if priced.iter().any(|(_, w)| w.unpriced > 0) {
            let names: Vec<String> =
                priced.iter().map(|(name, w)| if w.unpriced > 0 { format!("{name}, some models without a price") } else { name.clone() }).collect();
            return Some(names.join("; "));
        }
        Some(format!("{} only", priced.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>().join(", ")))
    }

    /// `{days: [{date, label, agents: {id: tokens}, total}], week: {agents, total}, today, unpriced, covers, estimated}`.
    pub fn json(&self, now: u64) -> Value {
        let agents = self.agents();
        let sum = |pick: &dyn Fn(&str) -> Tokens| {
            let mut t = Tokens::default();
            for a in &agents {
                t.add(&pick(a));
            }
            t
        };
        let days: Vec<Value> = self
            .dates
            .iter()
            .map(|date| {
                let label = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").map(|d| d.format("%a %-d").to_string()).unwrap_or_default();
                let per: serde_json::Map<String, Value> = agents.iter().map(|a| (a.clone(), self.day(a, date).json())).collect();
                json!({"date": date, "label": label, "agents": per, "total": sum(&|a| self.day(a, date)).json()})
            })
            .collect();
        let week: serde_json::Map<String, Value> = agents.iter().map(|a| (a.clone(), self.week_of(a).json())).collect();
        let today = self.dates.last().map(|d| sum(&|a| self.day(a, d))).unwrap_or_default();
        let names: serde_json::Map<String, Value> = agents.iter().map(|a| (a.clone(), json!(short_name(a)))).collect();
        json!({
            "at": self.at, "as_of_words": format!("as of {}", clock(self.at, now)), "estimated": true, "names": names,
            "days": days, "week": {"agents": week, "total": sum(&|a| self.week_of(a)).json()}, "today": today.json(),
            "unpriced": self.unpriced.iter().collect::<Vec<_>>(), "covers": self.covers(),
        })
    }

    /// Home's spend tile: `{today, week, estimated, unpriced, covers}` in USD; null when none of the week's tokens has
    /// a price, since a $0 would be made up.
    pub fn spend(&self) -> Value {
        let mut week = Tokens::default();
        let mut today = Tokens::default();
        for ((_, date), t) in &self.cells {
            if self.dates.contains(date) {
                week.add(t);
            }
            if self.dates.last() == Some(date) {
                today.add(t);
            }
        }
        if week.total() <= week.unpriced {
            return Value::Null;
        }
        let round = |usd: f64| (usd * 100.0).round() / 100.0;
        json!({"today": round(today.cost), "week": round(week.cost), "estimated": true, "unpriced": self.unpriced.iter().collect::<Vec<_>>(), "covers": self.covers()})
    }
}

fn scanner() -> MutexGuard<'static, Scanner> {
    static SCANNER: OnceLock<Mutex<Scanner>> = OnceLock::new();
    SCANNER.get_or_init(|| Mutex::new(load_scan(&scan_path()))).lock().unwrap_or_else(|e| e.into_inner())
}

fn result() -> MutexGuard<'static, Option<Days>> {
    static RESULT: OnceLock<Mutex<Option<Days>>> = OnceLock::new();
    lock(&RESULT)
}

/// The last count, None until the first one ends.
pub fn days() -> Option<Days> {
    result().clone()
}

/// Home's spend tile from the last count, null while there is none.
pub fn spend() -> Value {
    result().as_ref().map(Days::spend).unwrap_or(Value::Null)
}

/// Counts the tokens again, reading only what the transcripts gained since the last time, and keeps the result; the
/// sessions' and tickets' estimates go to estimates.json when they moved, one process at a time and never over a newer.
pub fn rescan(now: u64) -> Days {
    let roots: Vec<(String, Transcripts, PathBuf)> = agent::all()
        .into_iter()
        .filter(|a| counts_tokens(a.transcripts))
        .filter_map(|a| Some((a.id.clone(), a.transcripts, transcripts::root(a.transcripts)?)))
        .collect();
    let priced: Vec<String> = roots.iter().filter(|(_, kind, _)| *kind == Transcripts::Claude).map(|(id, _, _)| id.clone()).collect();
    let books = files::cache().join("costs");
    let mut s = scanner();
    // Another process (a second hub, `cockpit usage` without one) counts under the same lock.
    let held = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(files::cache().join("estimates.lock"));
    let held = held.ok().filter(|f| f.lock().is_ok());
    let was = files::load(&estimates_path()).unwrap_or(Value::Null);
    s.owners = owners_of(&was);
    let read = scan_into(&mut s, &roots, now, &booked(&books));
    let next = estimates_after(&s, &priced, &books, &was, now);
    if read {
        save_scan(&s, &scan_path());
    }
    let days = days_of(&s, now);
    drop(s);
    let newer = was["at"].as_u64().is_some_and(|at| at > now);
    let moved = ["sessions", "owned", "gone", "tickets", "archived"].iter().any(|k| next[*k] != was[*k]);
    if moved && !newer {
        let mut written = next.clone();
        written["at"] = json!(now);
        written.as_object_mut().map(|o| o.remove("retired"));
        if files::write(&estimates_path(), &written).is_ok() {
            for id in next["retired"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                let _ = std::fs::remove_file(books.join(format!("{id}.json")));
            }
        }
    }
    prune_books(&books, &next["sessions"], now);
    drop(held);
    *result() = Some(days.clone());
    days
}

/// Each message's owner as estimates.json keeps them.
fn owners_of(estimates: &Value) -> HashMap<u64, String> {
    let mut owners = HashMap::new();
    for (session, ids) in estimates["owned"].as_object().into_iter().flatten() {
        for id in ids.as_array().into_iter().flatten().filter_map(Value::as_u64) {
            owners.insert(id, session.clone());
        }
    }
    owners
}

/// The first line's time of a transcript, from its first 64 KB: when its session started.
fn started(path: &Path) -> String {
    use std::io::Read;
    let mut head = Vec::new();
    let _ = std::fs::File::open(path).map(|f| f.take(64 * 1024).read_to_end(&mut head));
    head.split(|b| *b == b'\n')
        .filter_map(|l| serde_json::from_slice::<Value>(l).ok())
        .find_map(|v| v["timestamp"].as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Whether a format's transcripts record each call's tokens, so a count reads them.
pub fn counts_tokens(kind: Transcripts) -> bool {
    matches!(
        kind,
        Transcripts::Claude | Transcripts::Codex | Transcripts::Pi | Transcripts::Omp | Transcripts::Qwen | Transcripts::Copilot | Transcripts::Gemini
    )
}

/// How deep under its root a format keeps its session files, and which file names are sessions.
fn layout(kind: Transcripts) -> (usize, fn(&str) -> bool) {
    match kind {
        Transcripts::Codex => (4, |name| name.ends_with(".jsonl")),
        Transcripts::Copilot => (1, |name| name == "events.jsonl"),
        _ => (3, |name| name.ends_with(".jsonl")),
    }
}

/// The session a transcript holds, as its format names it: a cost book and the estimates go by it.
fn session_of(kind: Transcripts, path: &Path) -> String {
    transcripts::id_of(kind, path).or_else(|| path.file_stem().map(|n| n.to_string_lossy().into_owned())).unwrap_or_default()
}

/// An agent's name in the counts: its usage name, else its name in the table, else its id.
pub fn short_name(agent: &str) -> String {
    agent::reports(agent).map(|r| r.short.to_string()).or_else(|| agent::get(agent).map(|a| a.name)).unwrap_or_else(|| agent.to_string())
}

/// A transcript to count: its path, agent and kind.
type Wanted = (PathBuf, String, Transcripts);
/// When a session started, as it orders a count: its first line's time, when its file was made, that time as written.
type Start = (i64, u128, String);

/// Counts what the transcripts of the last days gained, and those of the sessions in `keep` (ones a cost book names)
/// whatever their age, the sessions that started first first, so a message a fork repeats stays its parent's; true when
/// a file was read or left.
fn scan_into(s: &mut Scanner, roots: &[(String, Transcripts, PathBuf)], now: u64, keep: &HashSet<String>) -> bool {
    let since = now.saturating_sub((DAYS as u64 + 1) * DAY);
    let mut wanted: Vec<Wanted> = Vec::new();
    for (agent, kind, root) in roots {
        if wanted.iter().any(|(_, _, k)| k == kind) {
            continue;
        }
        let (depth, session_file) = layout(*kind);
        walk(root, depth, &session_file, &mut |path, modified| {
            let booked = keep.contains(&session_of(*kind, path));
            if modified >= since || booked {
                wanted.push((path.to_path_buf(), agent.clone(), *kind));
            }
        });
    }
    // By when each session started, then when its file was made, never when it was last written.
    let order = |p: &PathBuf| {
        let start = s.files.get(p).map(|f| f.started.clone()).filter(|t| !t.is_empty()).unwrap_or_else(|| started(p));
        let parsed = chrono::DateTime::parse_from_rfc3339(&start).map(|t| t.timestamp_nanos_opt().unwrap_or(0)).unwrap_or(i64::MAX);
        let made = std::fs::metadata(p).and_then(|m| m.created()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos());
        (parsed, made, start)
    };
    let mut keyed: Vec<(Start, Wanted)> = wanted.into_iter().map(|w| (order(&w.0), w)).collect();
    keyed.sort_by(|a, b| a.0.0.cmp(&b.0.0).then(a.0.1.cmp(&b.0.1)).then(a.1.0.cmp(&b.1.0)));
    // Files gone or too old to count leave with what they added.
    let kept: HashSet<&PathBuf> = keyed.iter().map(|(_, (p, _, _))| p).collect();
    let gone: Vec<PathBuf> = s.files.keys().filter(|p| !kept.contains(p)).cloned().collect();
    let mut read = !gone.is_empty();
    for path in gone {
        forget(s, &path);
    }
    for ((_, _, start), (path, agent, kind)) in &keyed {
        read |= scan_file(s, path, agent, *kind, start);
    }
    read
}

/// The tokens of the last days in what the scanner holds.
fn days_of(s: &Scanner, now: u64) -> Days {
    let dates = dates(now);
    let mut days = Days { at: now, dates: dates.clone(), ..Days::default() };
    for f in s.files.values() {
        for ((agent, date), t) in &f.cells {
            if dates.contains(date) {
                days.cells.entry((agent.clone(), date.clone())).or_default().add(t);
            }
        }
        days.unpriced.extend(f.unpriced.iter().filter(|(date, _)| dates.contains(date)).map(|(_, model)| model.clone()));
    }
    days
}

fn forget(s: &mut Scanner, path: &Path) {
    if let Some(f) = s.files.remove(path) {
        for id in f.ids {
            s.counted.remove(&id);
        }
    }
}

/// Reads what the file gained since the last scan; a file that shrank, or was rewritten whole, is read again from its
/// start. True when it read anything.
fn scan_file(s: &mut Scanner, path: &Path, agent: &str, kind: Transcripts, started: &str) -> bool {
    let Ok(meta) = std::fs::metadata(path) else { return false };
    let modified = stamp_of(&meta).1;
    if s.files.get(path).is_some_and(|f| f.len > meta.len() || (f.len == meta.len() && f.modified != modified)) {
        forget(s, path);
    }
    let session = session_of(kind, path);
    let mut f = s.files.remove(path).unwrap_or_else(|| Scanned { agent: agent.to_string(), session, started: started.to_string(), ..Scanned::default() });
    let read = f.offset < meta.len() || f.modified != modified;
    f.len = meta.len();
    f.modified = modified;
    if f.offset < f.len
        && let Ok(mut file) = std::fs::File::open(path)
        && file.seek(SeekFrom::Start(f.offset)).is_ok()
    {
        let mut reader = BufReader::with_capacity(64 * 1024, file);
        let mut line = Vec::new();
        loop {
            line.clear();
            let Ok(n) = reader.read_until(b'\n', &mut line) else { break };
            // A line still being written is read next time, whole.
            if n == 0 || line.last() != Some(&b'\n') {
                break;
            }
            f.offset += n as u64;
            match kind {
                Transcripts::Claude => claude_line(&mut f, &mut s.counted, &s.owners, &line),
                Transcripts::Codex => codex_line(&mut f, &mut s.counted, &line),
                Transcripts::Pi | Transcripts::Omp => pi_line(&mut f, &mut s.counted, &line),
                Transcripts::Qwen => qwen_line(&mut f, &mut s.counted, &line),
                Transcripts::Copilot => copilot_line(&mut f, &mut s.counted, &line),
                Transcripts::Gemini => gemini_line(&mut f, &line),
                _ => {}
            }
        }
    }
    if read && kind == Transcripts::Gemini {
        gemini_totals(&mut f, &mut s.counted);
    }
    s.files.insert(path.to_path_buf(), f);
    read
}

/// A Claude call's tokens and day, once per message: Claude Code writes a message once per content block, the same
/// usage on each, and a fork repeats its parent's, which stays the session's `owners` names.
fn claude_call(line: &[u8], counted: &mut HashSet<u64>, owners: &HashMap<u64, String>, session: &str, ids: &mut Vec<u64>) -> Option<(String, String, Tokens)> {
    if !contains(line, b"\"usage\"") || !contains(line, b"\"assistant\"") {
        return None;
    }
    let l = serde_json::from_slice::<ClaudeLine>(line).ok()?;
    let (m, stamp) = (l.message?, l.timestamp?);
    let (u, model) = (m.usage?, m.model?);
    if l.kind != "assistant" || model.starts_with('<') {
        return None;
    }
    if let Some(id) = m.id {
        let key = hashed(&[id.as_bytes(), l.request_id.unwrap_or_default().as_bytes()]);
        if owners.get(&key).is_some_and(|owner| owner != session) || !counted.insert(key) {
            return None;
        }
        ids.push(key);
    }
    Some((date_of(&stamp)?, model.clone(), claude_tokens(&model, &u)))
}

fn claude_line(f: &mut Scanned, counted: &mut HashSet<u64>, owners: &HashMap<u64, String>, line: &[u8]) {
    let Some((date, model, t)) = claude_call(line, counted, owners, &f.session, &mut f.ids) else { return };
    f.usd += t.cost;
    if t.unpriced > 0 {
        f.unpriced.insert((date.clone(), base_model(&model).to_string()));
    }
    f.cells.entry((f.agent.clone(), date)).or_default().add(&t);
}

/// A rollout's `token_count` records hold running totals: each adds what grew since the one before, on its own day.
/// A record a resumed or forked rollout copied from its parent moves the totals on without being counted again.
fn codex_line(f: &mut Scanned, counted: &mut HashSet<u64>, line: &[u8]) {
    if contains(line, b"\"turn_context\"") {
        if let Ok(v) = serde_json::from_slice::<Value>(line)
            && let Some(model) = v["payload"]["model"].as_str()
        {
            f.model = Some(model.to_string());
        }
        return;
    }
    if !contains(line, b"\"token_count\"") || !contains(line, b"total_token_usage") {
        return;
    }
    let Ok(v) = serde_json::from_slice::<Value>(line) else { return };
    let total = &v["payload"]["info"]["total_token_usage"];
    if !total.is_object() {
        return;
    }
    let n = |k: &str| total[k].as_u64().unwrap_or(0);
    let now = [n("input_tokens"), n("cached_input_tokens"), n("output_tokens"), n("cache_write_input_tokens")];
    let grew: Vec<u64> = now.iter().zip(f.totals).map(|(a, b)| a.saturating_sub(b)).collect();
    f.totals = now;
    let stamp = v["timestamp"].as_str().unwrap_or_default();
    let key = hashed(&[stamp.as_bytes(), total.to_string().as_bytes()]);
    if !counted.insert(key) {
        return;
    }
    f.ids.push(key);
    let Some(date) = date_of(stamp) else { return };
    let t = Tokens { input: grew[0].saturating_sub(grew[1]), cache_read: grew[1], output: grew[2], cache_write: grew[3], cost: 0.0, unpriced: 0 };
    let t = Tokens { unpriced: t.total(), ..t };
    if t.total() == 0 {
        return;
    }
    let model = match &f.model {
        Some(model) => model.clone(),
        None => format!("{} (model unknown)", short_name(&f.agent)),
    };
    f.unpriced.insert((date.clone(), model));
    f.cells.entry((f.agent.clone(), date)).or_default().add(&t);
}

/// Whether a record is counted for the first time, kept as this file's when it is: a fork or a resume that copies its
/// parent's records adds nothing.
fn first_time(f: &mut Scanned, counted: &mut HashSet<u64>, parts: &[&[u8]]) -> bool {
    let key = hashed(parts);
    if !counted.insert(key) {
        return false;
    }
    f.ids.push(key);
    true
}

/// One call's tokens on its day: priced when its model is in the price table, else at the price its agent wrote beside
/// them, else counted in tokens only.
fn add_call(f: &mut Scanned, date: String, model: Option<&str>, t: Tokens, own_price: Option<f64>) {
    let model =
        model.filter(|m| !m.is_empty()).map(str::to_string).or_else(|| f.model.clone()).unwrap_or_else(|| format!("{} (model unknown)", short_name(&f.agent)));
    let u = ClaudeUsage {
        input_tokens: t.input,
        output_tokens: t.output,
        cache_read_input_tokens: t.cache_read,
        cache_creation_input_tokens: t.cache_write,
        ..ClaudeUsage::default()
    };
    let mut t = claude_tokens(&model, &u);
    if t.unpriced > 0
        && let Some(usd) = own_price.filter(|usd| *usd > 0.0)
    {
        (t.cost, t.unpriced) = (usd, 0);
    }
    if t.total() == 0 {
        return;
    }
    f.usd += t.cost;
    if t.unpriced > 0 {
        f.unpriced.insert((date.clone(), base_model(&model).to_string()));
    }
    f.cells.entry((f.agent.clone(), date)).or_default().add(&t);
}

/// A pi or oh-my-pi assistant message's `usage` (badlogic/pi-mono `session-manager.ts`), its own `cost.total` the price
/// of a model the table lacks; the model is the message's, else the last `model_change`.
fn pi_line(f: &mut Scanned, counted: &mut HashSet<u64>, line: &[u8]) {
    if contains(line, b"\"model_change\"") {
        if let Ok(v) = serde_json::from_slice::<Value>(line)
            && let Some(model) = v["modelId"].as_str()
        {
            f.model = Some(model.to_string());
        }
        return;
    }
    if !contains(line, b"\"usage\"") || !contains(line, b"\"assistant\"") {
        return;
    }
    let Ok(v) = serde_json::from_slice::<Value>(line) else { return };
    let (m, u) = (&v["message"], &v["message"]["usage"]);
    if v["type"] != "message" || m["role"] != "assistant" || !u.is_object() {
        return;
    }
    let stamp = v["timestamp"].as_str().unwrap_or_default();
    if !first_time(f, counted, &[b"pi", v["id"].as_str().unwrap_or_default().as_bytes(), stamp.as_bytes()]) {
        return;
    }
    let Some(date) = date_of(stamp) else { return };
    let n = |k: &str| u[k].as_u64().unwrap_or(0);
    let t = Tokens { input: n("input"), output: n("output"), cache_read: n("cacheRead"), cache_write: n("cacheWrite"), ..Tokens::default() };
    add_call(f, date, m["model"].as_str(), t, u["cost"]["total"].as_f64());
}

/// A Qwen Code assistant record's `usageMetadata` (QwenLM/qwen-code `chatRecordingService.ts`): the prompt less what
/// came from the cache, and the answer with its thoughts.
fn qwen_line(f: &mut Scanned, counted: &mut HashSet<u64>, line: &[u8]) {
    if !contains(line, b"usageMetadata") {
        return;
    }
    let Ok(v) = serde_json::from_slice::<Value>(line) else { return };
    let u = &v["usageMetadata"];
    if v["type"] != "assistant" || !u.is_object() {
        return;
    }
    let stamp = v["timestamp"].as_str().unwrap_or_default();
    if !first_time(f, counted, &[b"qwen", v["uuid"].as_str().unwrap_or_default().as_bytes(), stamp.as_bytes()]) {
        return;
    }
    let Some(date) = date_of(stamp) else { return };
    let n = |k: &str| u[k].as_u64().unwrap_or(0);
    let cached = n("cachedContentTokenCount");
    let t = Tokens {
        input: n("promptTokenCount").saturating_sub(cached),
        cache_read: cached,
        output: n("candidatesTokenCount") + n("thoughtsTokenCount"),
        ..Tokens::default()
    };
    add_call(f, date, v["model"].as_str(), t, None);
}

/// Retains the latest token snapshot of a Gemini answer as it grows.
fn gemini_line(f: &mut Scanned, line: &[u8]) {
    if !contains(line, b"\"tokens\"") || !contains(line, b"\"gemini\"") {
        return;
    }
    let Ok(v) = serde_json::from_slice::<Value>(line) else { return };
    gemini_snapshot(f, &v);
    for message in v["messages"].as_array().into_iter().flatten().chain(v["$set"]["messages"].as_array().into_iter().flatten()) {
        gemini_snapshot(f, message);
    }
}

fn gemini_snapshot(f: &mut Scanned, v: &Value) {
    if v["type"] == "gemini"
        && v["tokens"].is_object()
        && let Some(id) = v["id"].as_str().filter(|id| !id.is_empty())
    {
        let n = |k: &str| v["tokens"][k].as_u64().unwrap_or(0);
        f.gemini_calls.insert(
            id.to_string(),
            json!({
                "timestamp": v["timestamp"].as_str(), "model": v["model"].as_str(),
                "tokens": {"input": n("input"), "cached": n("cached"), "output": n("output"), "thoughts": n("thoughts")},
            }),
        );
    }
}

/// Replaces each Gemini message's contribution with its latest totals, including snapshots read after a restart.
fn gemini_totals(f: &mut Scanned, counted: &mut HashSet<u64>) {
    for id in f.ids.drain(..) {
        counted.remove(&id);
    }
    f.cells.clear();
    f.unpriced.clear();
    f.usd = 0.0;
    let calls = std::mem::take(&mut f.gemini_calls);
    for (id, v) in &calls {
        let session = f.session.clone();
        let Some(date) = date_of(v["timestamp"].as_str().unwrap_or_default()) else { continue };
        if !first_time(f, counted, &[b"gemini", session.as_bytes(), id.as_bytes()]) {
            continue;
        }
        let n = |k: &str| v["tokens"][k].as_u64().unwrap_or(0);
        let t = Tokens { input: n("input").saturating_sub(n("cached")), cache_read: n("cached"), output: n("output") + n("thoughts"), ..Tokens::default() };
        add_call(f, date, v["model"].as_str(), t, None);
    }
    f.gemini_calls = calls;
}

/// A Copilot CLI `assistant.usage` event (github/copilot-sdk `session-events`); its model is the event's, else the
/// session's selected one.
fn copilot_line(f: &mut Scanned, counted: &mut HashSet<u64>, line: &[u8]) {
    let usage = contains(line, b"\"assistant.usage\"");
    if !usage && !contains(line, b"\"session.") {
        return;
    }
    let Ok(v) = serde_json::from_slice::<Value>(line) else { return };
    let d = &v["data"];
    match v["type"].as_str().unwrap_or_default() {
        "session.start" | "session.resume" => f.model = d["selectedModel"].as_str().map(str::to_string).or(f.model.take()),
        "session.model_change" => f.model = d["newModel"].as_str().map(str::to_string).or(f.model.take()),
        "assistant.usage" => {
            let stamp = v["timestamp"].as_str().unwrap_or_default();
            if !first_time(f, counted, &[b"copilot", v["id"].as_str().unwrap_or_default().as_bytes(), stamp.as_bytes()]) {
                return;
            }
            let Some(date) = date_of(stamp) else { return };
            let n = |k: &str| d[k].as_u64().unwrap_or(0);
            let t = Tokens {
                input: n("inputTokens"),
                output: n("outputTokens"),
                cache_read: n("cacheReadTokens"),
                cache_write: n("cacheWriteTokens"),
                ..Tokens::default()
            };
            add_call(f, date, d["model"].as_str(), t, None);
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Costs per session and per ticket, estimated

/// The cost book key the plugin books spend made on no ticket under: it weighs in a session's split, never a ticket.
pub const NO_TICKET: &str = "(no ticket)";
/// A cost book whose session never got an estimate goes after this long, in ms: it would add nothing.
const BOOK_KEPT: u64 = 30 * DAY;

/// A booked session whose transcript has been gone this long, in ms, is folded into its tickets' archived sums.
const GONE_KEPT: u64 = 30 * DAY;

fn estimates_path() -> PathBuf {
    files::cache().join("estimates.json")
}

/// `f` of the hub's estimates as last written, `{at, sessions, owned, gone, tickets, archived}`, read again only when
/// the file changed, never copied; null before the first count.
fn with_estimates<T>(f: impl FnOnce(&Value) -> T) -> T {
    static LAST: Kept<Option<(Stamp, Value)>> = OnceLock::new();
    let mut last = lock(&LAST);
    let stamp = std::fs::metadata(estimates_path()).ok().map(|m| stamp_of(&m));
    if last.as_ref().map(|(was, _)| *was) != stamp {
        *last = stamp.map(|st| (st, files::load(&estimates_path()).unwrap_or(Value::Null)));
    }
    f(last.as_ref().map_or(&Value::Null, |(_, v)| v))
}

/// Whether any count wrote its estimates yet.
pub fn estimated() -> bool {
    with_estimates(|v| !v.is_null())
}

/// What a session cost so far, estimated at the last count; it stays once Claude Code deletes the transcript.
pub fn session_estimate(session: &str) -> Option<f64> {
    with_estimates(|v| v["sessions"][session].as_f64())
}

/// What each ticket cost so far, estimated at the last count, archived sessions included.
pub fn ticket_estimates() -> HashMap<String, f64> {
    with_estimates(|v| v["tickets"].as_object().map(|o| o.iter().filter_map(|(k, v)| Some((k.clone(), v.as_f64()?))).collect()).unwrap_or_default())
}

/// What each ticket cost, estimated: each session's estimate (`cost`), split over the tickets its cost book in `books`
/// names in the book's proportions, spend made on no ticket weighing in but never shown.
pub fn ticket_costs(books: &Path, cost: &dyn Fn(&str) -> Option<f64>) -> HashMap<String, f64> {
    let mut total: HashMap<String, f64> = HashMap::new();
    for entry in std::fs::read_dir(books).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(session) = name.strip_suffix(".json") else { continue };
        let Some(usd) = cost(session).filter(|u| *u > 0.0) else { continue };
        for (ticket, usd) in split(&entry.path(), usd) {
            *total.entry(ticket).or_insert(0.0) += usd;
        }
    }
    total
}

/// A session's estimate split over the tickets of its cost book, in the book's proportions, `NO_TICKET` left out.
fn split(book: &Path, usd: f64) -> Vec<(String, f64)> {
    let Some(Value::Object(book)) = files::load(book) else { return Vec::new() };
    let weight: f64 = book.values().filter_map(Value::as_f64).filter(|w| *w > 0.0).sum();
    book.iter()
        .filter(|(t, _)| t.as_str() != NO_TICKET)
        .map(|(ticket, w)| {
            let share = if weight > 0.0 { w.as_f64().unwrap_or(0.0).max(0.0) / weight } else { 1.0 / book.len() as f64 };
            (ticket.clone(), usd * share)
        })
        .collect()
}

/// The sessions the cost books name.
fn booked(books: &Path) -> HashSet<String> {
    std::fs::read_dir(books).into_iter().flatten().flatten().filter_map(|e| e.file_name().to_string_lossy().strip_suffix(".json").map(str::to_string)).collect()
}

fn cents(usd: f64) -> f64 {
    (usd * 10_000.0).round() / 10_000.0
}

/// The estimates after a count: each counted session's own (`usd` of its transcript, the messages it owns and was charged
/// for), tickets split from them, and a booked session whose transcript is gone keeps its last ones for 30 days, then its
/// tickets' share goes to `archived` and it is `retired`, its book deleted.
fn estimates_after(s: &Scanner, priced: &[String], books: &Path, was: &Value, now: u64) -> Value {
    let booked = booked(books);
    let (mut sessions, mut owned, mut gone) = (BTreeMap::<String, f64>::new(), BTreeMap::<String, Value>::new(), BTreeMap::<String, u64>::new());
    // Claude Code's sessions always, another agent's once a call of it had a price: never a made-up $0.
    for f in s.files.values().filter(|f| priced.contains(&f.agent) || f.usd > 0.0) {
        sessions.insert(f.session.clone(), cents(f.usd));
        owned.insert(f.session.clone(), json!(f.ids));
    }
    let mut archived: BTreeMap<String, f64> = was["archived"].as_object().into_iter().flatten().filter_map(|(t, u)| Some((t.clone(), u.as_f64()?))).collect();
    let mut retired = Vec::new();
    for (id, usd) in was["sessions"].as_object().into_iter().flatten() {
        if sessions.contains_key(id) || !booked.contains(id) {
            continue;
        }
        let since = was["gone"][id].as_u64().unwrap_or(now);
        if now.saturating_sub(since) > GONE_KEPT {
            for (ticket, share) in split(&books.join(format!("{id}.json")), usd.as_f64().unwrap_or(0.0)) {
                *archived.entry(ticket).or_insert(0.0) += share;
            }
            retired.push(id.clone());
            continue;
        }
        sessions.insert(id.clone(), usd.as_f64().unwrap_or(0.0));
        owned.insert(id.clone(), was["owned"][id].clone());
        gone.insert(id.clone(), since);
    }
    let mut tickets: BTreeMap<String, f64> = ticket_costs(books, &|id| sessions.get(id).copied()).into_iter().collect();
    for (ticket, usd) in &archived {
        *tickets.entry(ticket.clone()).or_insert(0.0) += usd;
    }
    let round = |m: BTreeMap<String, f64>| m.into_iter().map(|(k, v)| (k, cents(v))).collect::<BTreeMap<_, _>>();
    json!({"sessions": sessions, "owned": owned, "gone": gone, "tickets": round(tickets), "archived": round(archived), "retired": retired})
}

/// Cost books of sessions that never got an estimate, past `BOOK_KEPT`: they would add nothing.
fn prune_books(books: &Path, sessions: &Value, now: u64) {
    for entry in std::fs::read_dir(books).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(session) = name.strip_suffix(".json") else { continue };
        let modified =
            entry.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64);
        if sessions[session].is_null() && modified.is_some_and(|m| now.saturating_sub(m) > BOOK_KEPT) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// As text, for the TUI

/// How a line of the text page is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Head,
    Plain,
    Good,
    Warn,
    Bad,
    Dim,
}

/// A bar of `cells` cells, a `│` where the window's time stands: `████████░░│░`.
pub fn bar(percent: f64, elapsed: Option<f64>, cells: usize) -> String {
    let filled = ((percent.clamp(0.0, 100.0) / 100.0) * cells as f64).round() as usize;
    let tick = elapsed.map(|e| ((e * cells as f64).round() as usize).min(cells.saturating_sub(1)));
    (0..cells)
        .map(|i| {
            if Some(i) == tick {
                '│'
            } else if i < filled {
                '█'
            } else {
                '░'
            }
        })
        .collect()
}

/// "1.2M", "34K", "800": a token count for a narrow column.
pub fn count(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => format!("{:.0}K", n as f64 / 1e3),
        1_000_000..=9_999_999 => format!("{:.1}M", n as f64 / 1e6),
        _ => format!("{:.0}M", n as f64 / 1e6),
    }
}

/// "~$1.84", "~$0.004": a cost cockpit estimated, never billed as such.
pub fn estimate(usd: f64) -> String {
    if usd > 0.0 && usd < 0.01 { format!("~${usd:.3}") } else { format!("~${usd:.2}") }
}

/// The view as lines of text: each agent with its account and windows, then the last days' tokens and estimate.
pub fn text(view: &Value) -> Vec<(Tone, String)> {
    let mut out: Vec<(Tone, String)> = Vec::new();
    for p in view["providers"].as_array().into_iter().flatten() {
        let name = p["name"].as_str().unwrap_or_default();
        out.push((Tone::Head, format!("{name}  ·  {}", p["words"].as_str().unwrap_or_default())));
        if let Some(who) = p["identity"]["words"].as_str() {
            out.push((Tone::Dim, format!("  {who}")));
        }
        for w in p["windows"].as_array().into_iter().flatten() {
            let stale = w["state"] == "stale";
            let said = w["figure"].as_str().unwrap_or_default();
            let figure = match (w["status"].as_str(), w["percent"].as_f64()) {
                (Some("rejected"), _) => format!("{} {:>4}  {}", bar(100.0, w["elapsed"].as_f64(), 12), said, status_words("rejected")),
                (_, Some(pc)) => format!("{} {said:>4}", bar(pc, w["elapsed"].as_f64(), 12)),
                (Some(status), None) => format!("{:<12} {}", "", status_words(status)),
                _ => format!("{:<12} no figure", ""),
            };
            let resets = w["resets_words"].as_str().map(|r| format!("  {r}")).unwrap_or_default();
            let tone = if w["status"] == "rejected" {
                Tone::Bad
            } else if stale {
                Tone::Dim
            } else if w["warn"] == true {
                Tone::Warn
            } else {
                Tone::Plain
            };
            out.push((tone, format!("  {:<9} {figure}{resets}", w["name"].as_str().unwrap_or_default())));
            let ahead = if w["ahead"] == true { " · ahead of pace" } else { "" };
            let mark = if stale { " · stale" } else { "" };
            out.push((
                Tone::Dim,
                format!("            {}{mark} · {}{ahead}", w["as_of_words"].as_str().unwrap_or_default(), w["source_words"].as_str().unwrap_or_default()),
            ));
        }
        if p["credits"]["unlimited"] == true {
            out.push((Tone::Dim, "  credits: unlimited".into()));
        } else if p["credits"]["has_credits"] == true {
            out.push((Tone::Dim, format!("  credits: {}", p["credits"]["balance"].as_str().unwrap_or("some"))));
        }
        out.push((Tone::Plain, String::new()));
    }
    if view["providers"].as_array().is_none_or(Vec::is_empty) {
        out.push((Tone::Dim, "No agent here reports its limits.".into()));
        out.push((Tone::Plain, String::new()));
    }
    let t = &view["tokens"];
    if view["loading"] == true {
        out.push((Tone::Head, "Tokens, last 7 days".into()));
        out.push((Tone::Dim, "  Loading usage…".into()));
        return out;
    }
    out.push((Tone::Head, format!("Tokens and estimated spend, last 7 days  ·  {}", t["as_of_words"].as_str().unwrap_or_default())));
    let agents: Vec<String> = t["week"]["agents"].as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();
    if agents.is_empty() {
        out.push((Tone::Dim, "  No usage data in the transcripts of these days.".into()));
        return out;
    }
    let short = |a: &str| t["names"][a].as_str().map(str::to_string).unwrap_or_else(|| short_name(a));
    let money = |t: &Value| match (t["cost"].as_f64().unwrap_or(0.0), t["unpriced"].as_u64().unwrap_or(0), t["total"].as_u64().unwrap_or(0)) {
        (_, _, 0) => "·".to_string(),
        (c, u, total) if u == total && c == 0.0 => "no price".to_string(),
        (c, _, _) => estimate(c),
    };
    let head: String = agents.iter().map(|a| format!("{:>14}", short(a))).collect();
    out.push((Tone::Dim, format!("  {:<8}{head}", "")));
    for d in t["days"].as_array().into_iter().flatten() {
        let cells: String = agents
            .iter()
            .map(|a| {
                let c = &d["agents"][a];
                let total = c["total"].as_u64().unwrap_or(0);
                format!("{:>14}", if total == 0 { "·".to_string() } else { format!("{} {}", count(total), money(c)) })
            })
            .collect();
        out.push((Tone::Plain, format!("  {:<8}{cells}", d["label"].as_str().unwrap_or_default())));
    }
    let week: String = agents
        .iter()
        .map(|a| {
            let c = &t["week"]["agents"][a];
            format!("{:>14}", format!("{} {}", count(c["total"].as_u64().unwrap_or(0)), money(c)))
        })
        .collect();
    out.push((Tone::Head, format!("  {:<8}{week}", "7 days")));
    let unpriced: Vec<&str> = t["unpriced"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    out.push((Tone::Dim, "  Estimated at API prices from the transcripts; a plan is not billed this way.".into()));
    if let Some(covers) = t["covers"].as_str() {
        out.push((Tone::Dim, format!("  The estimate covers {covers}.")));
    }
    if !unpriced.is_empty() {
        out.push((Tone::Dim, format!("  No price known for {}: counted in tokens only.", unpriced.join(", "))));
    }
    out
}

// ---------------------------------------------------------------------------------------------------------------------
// The popup

/// How often the popup asks for the view again.
const REFRESH: Duration = Duration::from_secs(15);

/// The hub's view; when no hub answers, one read here (without the sessions only the hub drives): its windows at once,
/// its tokens once a count here ends, which `wait` waits for.
pub fn fetch(wait: bool) -> Value {
    static COUNTING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if let Ok(answer) = crate::hub::request(&json!({"op": "view", "name": "usage"}), Duration::from_secs(5))
        && answer["value"].is_object()
    {
        return answer["value"].clone();
    }
    let now = crate::seen::now_ms();
    refresh(now);
    if days().is_none() {
        if wait {
            rescan(now);
        } else if !COUNTING.swap(true, std::sync::atomic::Ordering::SeqCst) {
            std::thread::spawn(move || {
                rescan(now);
                term::redraw();
            });
        }
    }
    view(now)
}

fn color(tone: Tone) -> Style {
    match tone {
        Tone::Head => bold(FG),
        Tone::Plain => fg(FG),
        Tone::Good => fg(GREEN),
        Tone::Warn => fg(ORANGE),
        Tone::Bad => fg(RED),
        Tone::Dim => fg(DIM),
    }
}

/// The popup: a header with when the view was read, the page scrolled with the usual keys, and a footer of keys.
pub struct Popup {
    view: Value,
    read: Instant,
    offset: usize,
    shown: usize,
    asking: Option<Receiver<Value>>,
    /// Its keys are shown over the page.
    keys: bool,
}

/// The popup's keys, as `?` shows them: it has no palette, so the spaces' overlay would promise one.
const POPUP_KEYS: [(&str, &str); 5] = [
    ("R", "refresh: the agents' files read again"),
    ("j k  ↑ ↓", "scroll"),
    ("g G", "top, bottom"),
    ("q  esc", "close"),
    ("?", "these keys; any key closes them"),
];

impl Popup {
    pub fn new(view: Value) -> Popup {
        Popup { view, read: Instant::now(), offset: 0, shown: 0, asking: None, keys: false }
    }

    /// The view asked for again; `read_again` (the R key) first has the hub read the files and count the tokens anew.
    fn refresh(&mut self, read_again: bool) {
        if self.asking.is_some() {
            return;
        }
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            if read_again {
                let _ = crate::hub::request(&json!({"op": "act", "app": "sources", "action": "refresh", "args": ["usage-all"]}), Duration::from_secs(60));
            }
            let _ = tx.send(fetch(false));
            term::redraw();
        });
        self.asking = Some(rx);
        self.read = Instant::now();
    }

    fn lines(&self) -> Vec<Line<'static>> {
        text(&self.view).into_iter().map(|(tone, text)| Line::from(Span::styled(format!(" {text}"), color(tone)))).collect()
    }

    fn scroll(&mut self, movement: Move) {
        let last = self.lines().len().saturating_sub(self.shown.max(1));
        self.offset = match movement {
            Move::By(n) | Move::Group(n) => self.offset.saturating_add_signed(n),
            Move::Pages(n) => self.offset.saturating_add_signed(n * self.shown.max(1) as isize),
            Move::Top => 0,
            Move::Bottom => last,
        }
        .min(last);
    }

    pub fn draw(&mut self, frame: &mut Frame) {
        let at = self.view["at"].as_u64().map(|at| clock(at, at)).unwrap_or_default();
        let loading = if self.asking.is_some() { "  reading…" } else { "" };
        let title = Line::from(vec![Span::styled(" \u{f029a}  Usage", bold(ORANGE)), Span::styled(format!("   updated {at}{loading}"), fg(DIM))]);
        // Each window's age as a source chip, the way every space's header says how fresh its data is.
        let mut header = crate::header::lines(title, &sources_of(&self.view), frame.area().width.saturating_sub(1) as usize);
        header.push(Line::raw(""));
        let [head, body, foot] = Layout::vertical([Constraint::Length(header.len() as u16), Constraint::Min(1), Constraint::Length(1)]).areas(frame.area());
        frame.render_widget(Paragraph::new(header).style(Style::new().bg(BG)), head);
        let lines = self.lines();
        self.shown = body.height as usize;
        self.offset = self.offset.min(lines.len().saturating_sub(self.shown));
        let total = lines.len();
        frame.render_widget(Paragraph::new(lines).scroll((self.offset as u16, 0)).style(Style::new().bg(BG)), body);
        crate::widgets::scrollbar(frame, body, self.offset, total, self.shown);
        let keys = [("q", "quit"), ("R", "refresh"), ("j/k", "scroll"), ("?", "keys")];
        let mut spans = vec![Span::raw(" ")];
        for (key, what) in keys {
            spans.extend([Span::styled(format!("{key} "), bold(ORANGE)), Span::styled(format!("{what}  "), fg(FG))]);
        }
        frame.render_widget(Paragraph::new(Line::from(spans)).style(Style::new().bg(PANEL)), foot);
        if self.keys {
            let lines: Vec<Line> =
                POPUP_KEYS.iter().map(|(k, what)| Line::from(vec![Span::styled(format!("  {k:<10}"), bold(ORANGE)), Span::styled(*what, fg(FG))])).collect();
            let area = frame.area();
            let (w, h) = (52.min(area.width), (lines.len() as u16 + 2).min(area.height));
            let at = ratatui::layout::Rect { x: area.x + area.width.saturating_sub(w) / 2, y: area.y + area.height.saturating_sub(h) / 2, width: w, height: h };
            frame.render_widget(ratatui::widgets::Clear, at);
            let block =
                ratatui::widgets::Block::bordered().border_style(fg(ORANGE)).title(Span::styled(" Keys in usage ", bold(ORANGE))).style(Style::new().bg(PANEL));
            frame.render_widget(Paragraph::new(lines).block(block), at);
        }
    }
}

impl term::App for Popup {
    fn draw(&mut self, frame: &mut Frame) {
        Popup::draw(self, frame);
    }

    fn event(&mut self, event: Event) -> bool {
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                if self.keys {
                    self.keys = false;
                    return false;
                }
                if keys::is_quit(&key) || key.code == KeyCode::Esc {
                    return true;
                }
                if key.code == KeyCode::Char('?') {
                    self.keys = true;
                } else if key.code == KeyCode::Char('R') {
                    self.refresh(true);
                } else if let Some(movement) = keys::movement(&key) {
                    self.scroll(movement);
                }
            }
            Event::Mouse(m) if m.kind == MouseEventKind::ScrollDown => self.scroll(Move::By(3)),
            Event::Mouse(m) if m.kind == MouseEventKind::ScrollUp => self.scroll(Move::By(-3)),
            _ => {}
        }
        false
    }

    fn tick(&mut self) -> bool {
        if let Some(rx) = &self.asking
            && let Ok(view) = rx.try_recv()
        {
            self.view = view;
            self.asking = None;
            return true;
        }
        // While the tokens are being counted, the view is asked for again soon.
        let due = if self.view["loading"] == true { Duration::from_secs(2) } else { REFRESH };
        if self.asking.is_none() && self.read.elapsed() >= due {
            self.refresh(false);
            return true;
        }
        false
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000_000;

    fn inputs() -> Inputs {
        Inputs { installed: vec!["claude".into(), "codex".into()], ..Inputs::default() }
    }

    #[test]
    fn the_status_line_file_gives_named_windows_with_their_reset() {
        let file = json!({"at": NOW - 60_000, "limits": [
            {"name": "5h", "percent": 64.4, "resetsAt": "2027-01-15T10:00:00Z"},
            {"name": "seven_day_opus", "percent": 12, "resetsAt": (NOW + DAY) / 1000}]});
        let w = from_statusline(&file);
        assert_eq!((w[0].agent.as_str(), w[0].name.as_str(), w[0].source), ("claude", "5h", STATUSLINE));
        assert_eq!(w[0].length, Some(5 * HOUR));
        assert_eq!(w[0].resets_at, Some(chrono::DateTime::parse_from_rfc3339("2027-01-15T10:00:00Z").unwrap().timestamp_millis() as u64));
        assert_eq!(w[1].name, "7d opus");
        assert_eq!((w[1].resets_at, w[1].length), (Some(NOW + DAY), Some(7 * DAY)));
        assert!(from_statusline(&json!({"limits": [{"name": "5h"}]})).is_empty(), "no time, no reading");
    }

    #[test]
    fn a_driven_session_reports_windows_or_a_bare_verdict() {
        let windows = json!({"status": "allowed", "unifiedWindows": {
            "five_hour": {"utilization": 0.42, "resetsAt": (NOW + HOUR) / 1000, "status": "allowed"},
            "seven_day": {"utilization": 0.9, "resetsAt": (NOW + 3 * DAY) / 1000, "status": "allowed_warning"}}});
        let w = from_stream(&windows, NOW);
        let five = w.iter().find(|w| w.name == "5h").unwrap();
        assert!((five.percent.unwrap() - 42.0).abs() < 1e-9);
        assert_eq!((five.source, five.agent.as_str()), (STREAM, "claude"));
        assert!(w.iter().find(|w| w.name == "7d").unwrap().warn());
        let bare = from_stream(&json!({"status": "allowed_warning", "rateLimitType": "five_hour", "resetsAt": (NOW + HOUR) / 1000}), NOW);
        assert_eq!(bare.len(), 1);
        assert_eq!((bare[0].percent, bare[0].status.as_deref()), (None, Some("allowed_warning")), "never a made-up number");
        assert!(from_stream(&Value::Null, NOW).is_empty());
        assert!(from_stream(&json!({"rateLimitType": "five_hour"}), NOW).is_empty(), "nothing said, nothing shown");
    }

    fn rollout_line(at: u64, percent: f64) -> Value {
        json!({"timestamp": Local.timestamp_millis_opt(at as i64).unwrap().to_rfc3339(), "type": "event_msg", "payload": {"type": "token_count", "info": null,
            "rate_limits": {"limit_id": "codex", "primary": {"used_percent": percent, "window_minutes": 10080, "resets_at": (NOW + 2 * DAY) / 1000},
                "secondary": null, "credits": {"has_credits": false, "unlimited": false, "balance": "0"}, "plan_type": "plus"}}})
    }

    #[test]
    fn a_codex_rollout_gives_its_weekly_window_plan_and_credits() {
        let c = from_rollout(&rollout_line(NOW - 60_000, 12.0)).unwrap();
        assert_eq!(c.windows.len(), 1, "secondary is null");
        let w = &c.windows[0];
        assert_eq!((w.agent.as_str(), w.name.as_str(), w.source, w.percent), ("codex", "7d", ROLLOUT, Some(12.0)));
        assert_eq!((w.length, w.resets_at, w.as_of), (Some(7 * DAY), Some(NOW + 2 * DAY), NOW - 60_000));
        assert_eq!(c.plan.as_deref(), Some("plus"));
        assert_eq!(c.credits["has_credits"], false);
        assert!(from_rollout(&json!({"timestamp": "2026-11-04T13:45:23Z", "payload": {"rate_limits": null}})).is_none());
    }

    #[test]
    fn the_last_rate_limits_of_a_rollout_file_are_found_in_its_tail() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout-x.jsonl");
        let lines = [
            rollout_line(NOW - 120_000, 3.0).to_string(),
            rollout_line(NOW - 60_000, 4.0).to_string(),
            r#"{"type":"event_msg","payload":{"type":"task_complete"}}"#.into(),
        ];
        std::fs::write(&path, lines.join("\n") + "\n").unwrap();
        let c = codex_reading(&path).unwrap();
        assert_eq!(c.windows[0].percent, Some(4.0));
    }

    #[test]
    fn readings_merge_newest_first_and_drop_what_has_reset() {
        let w = |name: &str, percent: Option<f64>, status: Option<&str>, as_of: u64, resets: u64| Window {
            agent: "claude".into(),
            name: name.into(),
            percent,
            status: status.map(str::to_string),
            resets_at: Some(resets),
            length: length_of(name),
            as_of,
            source: STATUSLINE,
        };
        let merged = merge(
            vec![
                w("5h", Some(50.0), None, NOW - 600_000, NOW + HOUR),
                w("5h", Some(60.0), None, NOW - 60_000, NOW + HOUR),
                w("5h", None, Some("allowed_warning"), NOW - 1_000, NOW + HOUR),
                w("7d", Some(90.0), None, NOW - DAY, NOW - 1),
            ],
            NOW,
        );
        assert_eq!(merged.len(), 1, "the 7d reading's window has reset");
        assert_eq!((merged[0].percent, merged[0].status.as_deref(), merged[0].as_of), (Some(60.0), Some("allowed_warning"), NOW - 60_000));
        let older = merge(vec![w("5h", None, Some("rejected"), NOW - 600_000, NOW + HOUR), w("5h", Some(60.0), None, NOW - 60_000, NOW + HOUR)], NOW);
        assert_eq!((older[0].percent, older[0].status.as_deref()), (Some(60.0), None), "a verdict older than the percent is not its verdict");
    }

    #[test]
    fn a_newer_rejected_verdict_reads_full_on_every_surface() {
        let i = Inputs {
            statusline: json!({"at": NOW - 60_000, "limits": [{"name": "5h", "percent": 64, "resetsAt": (NOW + HOUR) / 1000}]}),
            stream: from_stream(&json!({"status": "rejected", "rateLimitType": "five_hour", "resetsAt": (NOW + HOUR) / 1000}), NOW - 1_000),
            ..inputs()
        };
        let list = providers(&i, NOW);
        let w = list[0].tightest().unwrap();
        assert_eq!((w.percent, w.status.as_deref(), w.figure(NOW), w.tightness()), (Some(64.0), Some("rejected"), "full".to_string(), 100.0));
        assert_eq!(token(&list, NOW).unwrap(), "\u{f029a}  C full·5h");
        let v = view_of(&i, None, NOW);
        assert_eq!(v["windows"][0]["figure"], "full");
        assert!(v["windows"][0]["tooltip"].as_str().unwrap().starts_with("limit reached · resets in "), "{}", v["windows"][0]["tooltip"]);
        let page: Vec<(Tone, String)> = text(&v);
        let line = page.iter().find(|(_, l)| l.trim_start().starts_with("5h ")).unwrap();
        assert_eq!(line.0, Tone::Bad);
        assert!(line.1.contains("██████████│█ full  limit reached"), "{}", line.1);
    }

    #[test]
    fn a_stale_percent_is_a_floor_only_until_its_known_reset() {
        let stale = Window {
            agent: "codex".into(),
            name: "7d".into(),
            percent: Some(12.0),
            status: None,
            resets_at: Some(NOW + DAY),
            length: Some(7 * DAY),
            as_of: NOW - 3 * HOUR,
            source: ROLLOUT,
        };
        assert_eq!((stale.floor(NOW), stale.figure(NOW)), (true, "≥12%".to_string()));
        let unknown = Window { resets_at: None, ..stale.clone() };
        assert_eq!((unknown.floor(NOW), unknown.figure(NOW)), (false, "12%".to_string()), "no reset known, no floor");
        let fresh = Window { as_of: NOW, ..stale };
        assert_eq!(fresh.figure(NOW), "12%");
    }

    #[test]
    fn a_driven_verdict_is_as_of_when_its_session_last_reported_it() {
        let info = json!({"status": "allowed_warning", "rateLimitType": "five_hour", "resetsAt": (NOW + HOUR) / 1000});
        let mut seen = HashMap::new();
        let session = |at: u64| json!({"key": "s1", "rate_limit": info, "rate_limit_at": at});
        assert_eq!(stream_of(&session(NOW - 5_000), NOW, &mut seen)[0].as_of, NOW - 5_000);
        assert_eq!(stream_of(&session(NOW - 1_000), NOW, &mut seen)[0].as_of, NOW - 1_000, "the same verdict confirmed again is fresh again");
        let bare = json!({"key": "s2", "rate_limit": info});
        assert_eq!(stream_of(&bare, NOW, &mut seen)[0].as_of, NOW);
        assert_eq!(stream_of(&bare, NOW + 60_000, &mut seen)[0].as_of, NOW, "without its time, as of when cockpit first saw it");
        assert!(stream_of(&json!({"key": "s3"}), NOW, &mut seen).is_empty());
    }

    #[test]
    fn each_codex_bucket_is_its_own_window_and_a_reset_may_come_as_seconds() {
        let line = |at: u64, id: &str, name: Option<&str>, percent: f64| {
            json!({"timestamp": Local.timestamp_millis_opt(at as i64).unwrap().to_rfc3339(), "type": "event_msg", "payload": {"type": "token_count",
                "rate_limits": {"limit_id": id, "limit_name": name, "primary": {"used_percent": percent, "window_minutes": 300, "resets_in_seconds": 600}}}})
        };
        let other = from_rollout(&line(NOW, "codex_bengalfox", Some("GPT-5.3 Spark"), 7.0)).unwrap();
        assert_eq!((other.windows[0].name.as_str(), other.windows[0].resets_at), ("5h GPT-5.3 Spark", Some(NOW + 600_000)));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout-b.jsonl");
        let lines =
            [line(NOW - 3_000, "codex", None, 20.0), line(NOW - 2_000, "codex_bengalfox", Some("GPT-5.3 Spark"), 7.0), line(NOW - 1_000, "codex", None, 21.0)];
        std::fs::write(&path, lines.iter().map(Value::to_string).collect::<Vec<_>>().join("\n") + "\n").unwrap();
        let reading = codex_reading(&path).unwrap();
        let names: Vec<(&str, Option<f64>)> = reading.windows.iter().map(|w| (w.name.as_str(), w.percent)).collect();
        assert_eq!(names, [("5h", Some(21.0)), ("5h GPT-5.3 Spark", Some(7.0))], "the newest of each bucket, never one for the other");
    }

    #[test]
    fn an_installed_agent_with_no_source_reads_not_reported_never_a_figure() {
        let i = Inputs { installed: vec!["gemini".into(), "pi".into(), "amp".into()], ..Inputs::default() };
        for p in providers(&i, NOW) {
            assert_eq!((p.state(NOW), p.words(NOW).as_str(), p.windows.len()), ("unreported", "Not reported by this agent", 0), "{}", p.agent);
        }
        assert_eq!(providers(&i, NOW).iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Pi", "Gemini CLI", "Amp"], "the table's names and order");
    }

    #[test]
    fn only_the_account_address_and_organization_are_kept() {
        let file = br#"{"numStartups": 9, "userID": "u-1", "oauthAccount": {"accountUuid": "a-1", "emailAddress": "me@acme.example",
            "organizationUuid": "o-1", "organizationName": "Acme", "organizationRole": "admin"}, "projects": {"/code/api": {}}}"#;
        assert_eq!(account_of(file), Some(json!({"emailAddress": "me@acme.example", "organizationName": "Acme"})));
        assert_eq!(account_of(br#"{"numStartups": 9}"#), Some(Value::Bool(false)));
        assert_eq!(account_of(b"{\"oauthAcc"), None, "a file caught half written says nothing");
    }

    #[test]
    fn limits_json_caught_half_written_keeps_the_last_good_reading() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("limits.json");
        assert_eq!(statusline_file(&path), (Value::Null, None), "no file, no error");
        std::fs::write(&path, "{\"at\": 1").unwrap();
        assert_eq!(statusline_file(&path).1.as_deref(), Some("limits.json could not be read"), "never read whole");
        std::fs::write(&path, r#"{"at": 1, "limits": [{"name": "5h", "percent": 64}]}"#).unwrap();
        let good = statusline_file(&path).0;
        assert_eq!(good["limits"][0]["percent"], 64);
        std::fs::write(&path, r#"{"at": 2, "limits": [{"na"#).unwrap();
        assert_eq!(statusline_file(&path), (good, None));
    }

    #[test]
    fn each_state_reads_as_it_should() {
        let mut i = inputs();
        i.installed.push("opencode".into());
        let none = providers(&i, NOW);
        let state = |list: &[Provider], id: &str| list.iter().find(|p| p.agent == id).map(|p| (p.state(NOW), p.words(NOW)));
        assert_eq!(state(&none, "claude").unwrap(), ("none", "No recent reading".to_string()));
        assert_eq!(state(&none, "opencode").unwrap(), ("unreported", "Not reported by this agent".to_string()));
        assert!(state(&none, "gemini").is_none(), "an agent not installed is not listed");

        i.statusline = json!({"at": NOW - 60_000, "limits": [{"name": "5h", "percent": 64, "resetsAt": (NOW + HOUR) / 1000}]});
        i.codex = from_rollout(&rollout_line(NOW - 3 * HOUR, 12.0));
        let list = providers(&i, NOW);
        assert_eq!(state(&list, "claude").unwrap().0, "fresh");
        let (stale, words) = state(&list, "codex").unwrap();
        assert_eq!(stale, "stale");
        assert!(words.starts_with("as of ") && words.ends_with(" · stale"), "{words}");
        assert_eq!(list.iter().map(|p| p.agent.as_str()).collect::<Vec<_>>()[..2], ["claude", "codex"], "readings first");
        assert_eq!(list.last().unwrap().agent, "opencode");

        let failing = Inputs { statusline_error: Some("limits.json could not be read".into()), ..inputs() };
        let p = providers(&failing, NOW);
        assert_eq!(state(&p, "claude").unwrap(), ("unavailable", "Usage unavailable: limits.json could not be read".to_string()));
    }

    #[test]
    fn identity_is_the_address_and_organization_or_the_plan() {
        let i = Inputs {
            claude_account: json!({"emailAddress": "me@acme.example", "organizationName": "Acme"}),
            codex: from_rollout(&rollout_line(NOW, 1.0)),
            ..inputs()
        };
        let list = providers(&i, NOW);
        let of = |id: &str| list.iter().find(|p| p.agent == id).unwrap().identity["words"].clone();
        assert_eq!((of("claude"), of("codex")), (json!("This machine's Claude Code: me@acme.example · Acme"), json!("ChatGPT Plus plan")));
        let out = Inputs { claude_account: Value::Bool(false), ..inputs() };
        let signed_out = &providers(&out, NOW)[0].identity;
        assert_eq!(
            (&signed_out["signed_in"], &signed_out["words"]),
            (&json!(false), &json!("This machine's Claude Code is not signed in to a Claude account"))
        );
        assert_eq!(providers(&inputs(), NOW)[0].identity, Value::Null, "nothing says, nothing shown");
    }

    #[test]
    fn the_quota_rows_keep_the_bars_shape_and_add_where_they_come_from() {
        let i = Inputs {
            statusline: json!({"at": NOW, "limits": [{"name": "7d", "percent": 38, "resetsAt": (NOW + DAY) / 1000}, {"name": "5h", "percent": 90, "resetsAt": (NOW + 4 * HOUR) / 1000}]}),
            ..inputs()
        };
        let q = quota(&providers(&i, NOW), NOW);
        assert_eq!(q[0]["name"], "5h", "shortest window first");
        assert!((q[0]["elapsed"].as_f64().unwrap() - 0.2).abs() < 1e-9);
        assert_eq!(quota(&providers(&i, NOW + 1_000), NOW + 1_000)[0]["elapsed"], q[0]["elapsed"], "to the percent the bar draws, not each second");
        assert_eq!((q[0]["ahead"].as_bool(), q[0]["warn"].as_bool()), (Some(true), Some(true)));
        assert_eq!((q[0]["label"].as_str(), q[0]["source"].as_str(), q[0]["state"].as_str()), (Some("Claude 5h"), Some(STATUSLINE), Some("fresh")));
        assert_eq!(q[0]["icon"], "claude", "Herdr's logo for the agent it limits");
        assert_eq!(q[0]["fresh"]["state"], "fresh");
        let tip = q[0]["tooltip"].as_str().unwrap();
        assert!(tip.starts_with("90% used · resets in 4 h · as of ") && tip.ends_with(" · from Claude Code's status line"), "{tip}");
        assert_eq!(quota(&providers(&inputs(), NOW), NOW), json!([]), "no bar for an agent that reports nothing");
    }

    #[test]
    fn the_sidebar_token_groups_each_agent_s_windows_under_its_letter() {
        let i = Inputs {
            statusline: json!({"at": NOW, "limits": [{"name": "5h", "percent": 64, "resetsAt": (NOW + HOUR) / 1000}, {"name": "7d", "percent": 38, "resetsAt": (NOW + DAY) / 1000}]}),
            codex: from_rollout(&rollout_line(NOW - 3 * HOUR, 12.0)),
            ..inputs()
        };
        assert_eq!(token(&providers(&i, NOW), NOW).unwrap(), "\u{f029a}  C 64%·5h 38%·7d  X ≥12%·7d", "each agent's windows under its letter");
        let verdict =
            Inputs { stream: from_stream(&json!({"status": "rejected", "rateLimitType": "five_hour", "resetsAt": (NOW + HOUR) / 1000}), NOW), ..inputs() };
        assert_eq!(token(&providers(&verdict, NOW), NOW).unwrap(), "\u{f029a}  C full·5h");
        assert_eq!(token(&providers(&inputs(), NOW), NOW), None);
        let crowded = Inputs {
            statusline: json!({"at": NOW, "limits": [
                {"name": "5h", "percent": 64, "resetsAt": (NOW + HOUR) / 1000}, {"name": "7d", "percent": 38, "resetsAt": (NOW + DAY) / 1000},
                {"name": "7d opus", "percent": 91, "resetsAt": (NOW + DAY) / 1000}]}),
            codex: from_rollout(&rollout_line(NOW - 3 * HOUR, 12.0)),
            ..inputs()
        };
        assert_eq!(token(&providers(&crowded, NOW), NOW).unwrap(), "\u{f029a}  C 91%·7d-opus  X ≥12%·7d", "past the sidebar's room, each agent's tightest");
    }

    #[test]
    fn the_view_lists_windows_tightest_first_and_waits_for_the_count() {
        let i = Inputs {
            statusline: json!({"at": NOW, "limits": [{"name": "5h", "percent": 20, "resetsAt": (NOW + HOUR) / 1000}]}),
            codex: from_rollout(&rollout_line(NOW, 55.0)),
            ..inputs()
        };
        let v = view_of(&i, None, NOW);
        assert_eq!(v["windows"][0]["label"], "Codex 7d");
        assert_eq!(v["windows"][0]["icon"], "codex");
        assert!(v["providers"].as_array().unwrap().iter().all(|p| p["icon"] == p["agent"]), "each agent with its own icon: {}", v["providers"]);
        assert_eq!(v["tightest"]["percent"], 55.0);
        assert_eq!((v["loading"].as_bool(), v["tokens"].is_null()), (Some(true), true));
    }

    #[test]
    fn each_agent_s_windows_are_one_source_chip_and_a_failed_read_a_failing_one() {
        let i = Inputs {
            statusline: json!({"at": NOW - 60_000, "limits": [
                {"name": "5h", "percent": 64, "resetsAt": (NOW + HOUR) / 1000}, {"name": "7d", "percent": 38, "resetsAt": (NOW + DAY) / 1000}]}),
            codex: from_rollout(&rollout_line(NOW - 3 * HOUR, 12.0)),
            ..inputs()
        };
        let v = view_of(&i, None, NOW);
        let chips: Vec<(String, String, String)> = v["sources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| (s["source"].as_str().unwrap().into(), s["label"].as_str().unwrap().into(), s["state"].as_str().unwrap().into()))
            .collect();
        assert_eq!(chips, [("usage-claude".into(), "Claude 5h 7d".into(), "fresh".into()), ("usage-codex".into(), "Codex 7d".into(), "stale".into())]);
        assert_eq!(v["sources"][0]["every"], EVERY);
        let failing = Inputs { statusline_error: Some("limits.json could not be read".into()), ..inputs() };
        let s = &view_of(&failing, None, NOW)["sources"][0];
        assert_eq!(
            (s["source"].as_str(), s["state"].as_str(), s["error"]["message"].as_str()),
            (Some("usage-claude"), Some("failing"), Some("limits.json could not be read"))
        );
    }

    #[test]
    fn times_read_as_a_person_says_them() {
        assert_eq!(resets_words(NOW + 45 * 60_000, NOW), "resets in 45 min");
        assert_eq!(resets_words(NOW + 130 * 60_000, NOW), "resets in 2 h 10 min");
        assert_eq!(resets_words(NOW + 2 * HOUR, NOW), "resets in 2 h");
        assert_eq!(resets_words(NOW + 3 * DAY, NOW), "resets in 3 d");
        assert_eq!(resets_words(NOW - 1, NOW), "reset");
        assert_eq!(clock(NOW, NOW).len(), 5);
        assert_eq!(bar(50.0, Some(0.75), 8), "████░░│░");
        assert_eq!((count(950), count(34_200), count(1_240_000)), ("950".into(), "34K".into(), "1.2M".into()));
        assert_eq!((estimate(1.844), estimate(0.004)), ("~$1.84".into(), "~$0.004".into()));
    }

    #[test]
    fn claude_calls_are_priced_by_their_model_and_unknown_models_never_are() {
        let u = ClaudeUsage { input_tokens: 1_000_000, output_tokens: 100_000, cache_read_input_tokens: 1_000_000, ..ClaudeUsage::default() };
        let t = claude_tokens("claude-opus-5-5", &u);
        assert!((t.cost - (4.0 + 2.0 + 0.2)).abs() < 1e-9, "{}", t.cost);
        assert!((claude_tokens("claude-haiku-4-5-20251001", &u).cost - (1.0 + 0.5 + 0.1)).abs() < 1e-9, "the date is dropped");
        let writes = ClaudeUsage {
            cache_creation_input_tokens: 2_000_000,
            cache_creation: Some(CacheWrites { ephemeral_5m_input_tokens: 1_000_000, ephemeral_1h_input_tokens: 1_000_000 }),
            ..ClaudeUsage::default()
        };
        assert!((claude_tokens("claude-sonnet-5-5", &writes).cost - (2.5 + 4.0)).abs() < 1e-9);
        let unknown = claude_tokens("sonnet", &u);
        assert_eq!((unknown.cost, unknown.unpriced), (0.0, 2_100_000));
    }

    /// A time this many minutes before `now`, as the transcripts write it.
    fn at(now: u64, minutes_ago: u64) -> String {
        Local.timestamp_millis_opt((now - minutes_ago * 60_000) as i64).unwrap().to_rfc3339()
    }

    fn jsonl(path: &Path, lines: &[Value]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, lines.iter().map(|l| l.to_string() + "\n").collect::<String>()).unwrap();
    }

    #[test]
    fn pi_qwen_copilot_and_gemini_calls_count_like_claude_s_each_once() {
        let now = crate::seen::now_ms();
        let dir = tempfile::tempdir().unwrap();
        let used = |input: u64, read: u64, cost: f64| json!({"input": input, "output": 50, "cacheRead": read, "cacheWrite": 0, "cost": {"total": cost}});
        let pi = |id: &str, model: &str, cost: f64| json!({"type": "message", "id": id, "timestamp": at(now, 30), "message": {"role": "assistant", "model": model, "usage": used(1000, 9000, cost), "content": []}});
        let header = json!({"type": "session", "id": "p-1", "timestamp": at(now, 31), "cwd": "/code/api"});
        jsonl(
            &dir.path().join("pi/--code-api--/2026-11-03T10-00-00-000Z_p-1.jsonl"),
            &[header.clone(), pi("a1", "claude-sonnet-5-5", 0.5), pi("a2", "acme-large", 0.004)],
        );
        // A fork copies its parent's entries before its own.
        let fork = json!({"type": "session", "id": "p-2", "timestamp": at(now, 20), "cwd": "/code/api"});
        jsonl(&dir.path().join("pi/--code-api--/2026-11-03T10-10-00-000Z_p-2.jsonl"), &[fork, pi("a1", "claude-sonnet-5-5", 0.5), pi("a3", "acme-free", 0.0)]);
        jsonl(
            &dir.path().join("qwen/-code-api/chats/q-1.jsonl"),
            &[json!({"uuid": "u2", "sessionId": "q-1", "timestamp": at(now, 25), "type": "assistant", "model": "acme-coder",
                "usageMetadata": {"promptTokenCount": 9000, "candidatesTokenCount": 30, "thoughtsTokenCount": 20, "cachedContentTokenCount": 8000}})],
        );
        let session = "5c1e0000-0000-4000-8000-000000000412";
        jsonl(
            &dir.path().join(format!("copilot/{session}/events.jsonl")),
            &[
                json!({"id": "e1", "timestamp": at(now, 26), "type": "session.start", "data": {"sessionId": session, "selectedModel": "acme-mini"}}),
                json!({"id": "e4", "timestamp": at(now, 24), "type": "assistant.usage", "data": {"inputTokens": 8000, "outputTokens": 120, "cacheReadTokens": 6000}}),
            ],
        );
        std::fs::write(dir.path().join(format!("copilot/{session}/workspace.jsonl")), "{}\n").unwrap();
        let answer =
            |tokens: Value| json!({"id": "g2", "timestamp": at(now, 22), "type": "gemini", "content": "Done.", "model": "gemini-acme-pro", "tokens": tokens});
        let used = json!({"input": 9000, "output": 40, "cached": 6000, "thoughts": 20, "total": 9060});
        jsonl(
            &dir.path().join("gemini/api/chats/session-2026-11-03T10-00-9d2e0000.jsonl"),
            &[json!({"sessionId": "9d2e0000-0000-4000-8000-000000000412", "startTime": at(now, 23)}), answer(Value::Null), answer(used.clone()), answer(used)],
        );
        let roots = vec![
            ("pi".to_string(), Transcripts::Pi, dir.path().join("pi")),
            ("qwen".to_string(), Transcripts::Qwen, dir.path().join("qwen")),
            ("copilot".to_string(), Transcripts::Copilot, dir.path().join("copilot")),
            ("gemini".to_string(), Transcripts::Gemini, dir.path().join("gemini")),
        ];
        let mut s = Scanner::default();
        scan_into(&mut s, &roots, now, &HashSet::new());
        let days = days_of(&s, now);
        let p = days.week_of("pi");
        assert_eq!((p.input, p.cache_read, p.output), (3000, 27_000, 150), "a1 once, a2, a3: the fork's copy of a1 is its parent's");
        let sonnet = 1000.0 * 2.0 / 1e6 + 9000.0 * 0.2 / 1e6 + 50.0 * 10.0 / 1e6;
        assert!((p.cost - (sonnet + 0.004)).abs() < 1e-9, "the table's price, else pi's own: {}", p.cost);
        assert_eq!(p.unpriced, 10_050, "a model with neither counts in tokens only");
        let q = days.week_of("qwen");
        assert_eq!((q.input, q.cache_read, q.output, q.unpriced), (1000, 8000, 50, 9050));
        let c = days.week_of("copilot");
        assert_eq!((c.input, c.cache_read, c.output), (8000, 6000, 120), "events.jsonl only");
        let g = days.week_of("gemini");
        assert_eq!((g.input, g.cache_read, g.output), (3000, 6000, 60), "a message written again whole counts once");
        assert_eq!(
            days.unpriced.iter().collect::<Vec<_>>(),
            ["acme-coder", "acme-free", "acme-mini", "gemini-acme-pro"],
            "the session's model when the call names none"
        );
        let sessions: Vec<&str> = s.files.values().map(|f| f.session.as_str()).collect();
        assert!(sessions.contains(&session) && sessions.contains(&"p-2"), "named as each format names its sessions: {sessions:?}");
        let est = estimates_after(&s, &[], &dir.path().join("costs"), &Value::Null, now);
        assert!(est["sessions"]["p-1"].as_f64().is_some_and(|usd| usd > 0.0), "a priced session gets its estimate");
        assert!(est["sessions"]["q-1"].is_null() && est["sessions"][session].is_null(), "an unpriced one none, never $0");
        assert_eq!(days.json(now)["names"]["copilot"], "Copilot", "its usage name, as Claude and Codex have");
    }

    #[test]
    fn gemini_usage_replaces_partial_totals_after_an_incremental_scan_and_restart() {
        let now = crate::seen::now_ms();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gemini/api/chats/session-acme.jsonl");
        let answer = |id, output| json!({"id": id, "type": "gemini", "content": "acme-content-do-not-copy", "toolCalls": [{"result": "acme-tool-output-do-not-copy"}], "timestamp": at(now, 1), "model": "gemini-acme", "tokens": {"input": 1000, "cached": 500, "output": output}});
        jsonl(&path, &[json!({"sessionId": "acme-session", "messages": [answer("a1", 10)]})]);
        let roots = [("gemini".into(), Transcripts::Gemini, dir.path().join("gemini"))];
        let mut s = Scanner::default();
        scan_into(&mut s, &roots, now, &HashSet::new());
        assert_eq!(days_of(&s, now).week_of("gemini").output, 10);
        let cache = dir.path().join("scan.json");
        save_scan(&s, &cache);
        assert!(!std::fs::read_to_string(&cache).unwrap().contains("do-not-copy"), "the usage cache holds counters, not message or tool content");
        let mut s = load_scan(&cache);
        use std::io::Write;
        writeln!(std::fs::OpenOptions::new().append(true).open(&path).unwrap(), "{}", answer("a1", 40)).unwrap();
        scan_into(&mut s, &roots, now, &HashSet::new());
        let t = days_of(&s, now).week_of("gemini");
        assert_eq!((t.input, t.cache_read, t.output), (500, 500, 40), "the final snapshot replaces the partial one");
        scan_into(&mut s, &roots, now, &HashSet::new());
        assert_eq!(days_of(&s, now).week_of("gemini").output, 40);
        writeln!(std::fs::OpenOptions::new().append(true).open(&path).unwrap(), "{}", json!({"$set": {"messages": [answer("a2", 25)]}})).unwrap();
        save_scan(&s, &cache);
        let mut s = load_scan(&cache);
        scan_into(&mut s, &roots, now, &HashSet::new());
        assert_eq!(days_of(&s, now).week_of("gemini").output, 65, "checkpoint-only answers count and omitted historical calls stay billed");
        writeln!(std::fs::OpenOptions::new().append(true).open(&path).unwrap(), "{}", json!({"$rewindTo": "a2"})).unwrap();
        scan_into(&mut s, &roots, now, &HashSet::new());
        assert_eq!(days_of(&s, now).week_of("gemini").output, 65, "rewinding the conversation does not refund tokens");
    }

    #[test]
    fn transcripts_count_per_day_once_each_and_only_what_they_gained() {
        // Files are counted by their modification time, so this one runs on the real clock.
        let now = crate::seen::now_ms();
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().join("claude/projects/-code-api");
        let codex = dir.path().join("codex/sessions/2027/01/15");
        std::fs::create_dir_all(&claude).unwrap();
        std::fs::create_dir_all(&codex).unwrap();
        let call = |id: &str, ago: u64| {
            json!({"type": "assistant", "timestamp": at(now, ago), "requestId": "r", "message": {"id": id, "model": "claude-opus-5-5",
                "usage": {"input_tokens": 1000, "output_tokens": 500, "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0}}})
            .to_string()
        };
        // The same message twice (a block per line), and a fork that repeats it.
        std::fs::write(claude.join("s1.jsonl"), [call("m1", 10), call("m1", 10), call("m2", 5)].join("\n") + "\n").unwrap();
        std::fs::write(claude.join("s2.jsonl"), call("m1", 10) + "\n").unwrap();
        let count = |total: (u64, u64, u64), ago: u64| {
            json!({"timestamp": at(now, ago), "type": "event_msg", "payload": {"type": "token_count",
                "info": {"total_token_usage": {"input_tokens": total.0, "cached_input_tokens": total.1, "output_tokens": total.2}}}})
            .to_string()
        };
        let context = json!({"type": "turn_context", "payload": {"model": "gpt-5-codex"}}).to_string();
        let rollout = codex.join("rollout-a.jsonl");
        std::fs::write(&rollout, [context, count((1000, 400, 100), 8), count((3000, 1400, 300), 4)].join("\n") + "\n").unwrap();
        let roots = vec![
            ("claude".to_string(), Transcripts::Claude, dir.path().join("claude/projects")),
            ("codex".to_string(), Transcripts::Codex, dir.path().join("codex/sessions")),
        ];
        let mut s = Scanner::default();
        scan_into(&mut s, &roots, now, &HashSet::new());
        let days = days_of(&s, now);
        // Summed over the counted days: the calls are minutes old, which may be yesterday just after midnight.
        let c = days.week_of("claude");
        assert_eq!((c.input, c.output), (2000, 1000), "m1 once, m2 once");
        assert!((c.cost - (0.008 + 0.02)).abs() < 1e-9);
        let x = days.week_of("codex");
        assert_eq!((x.input, x.cache_read, x.output, x.unpriced), (1600, 1400, 300, 3300));
        assert!(days.unpriced.contains("gpt-5-codex"));
        // A line appended later is the only one read again.
        let mut file = std::fs::OpenOptions::new().append(true).open(claude.join("s1.jsonl")).unwrap();
        std::io::Write::write_all(&mut file, (call("m3", 1) + "\n").as_bytes()).unwrap();
        scan_into(&mut s, &roots, now, &HashSet::new());
        let again = days_of(&s, now);
        assert_eq!(again.week_of("claude").input, 3000);
        let spend = again.spend();
        assert_eq!((spend["estimated"].as_bool(), spend["week"].as_f64()), (Some(true), Some(0.04)));
        let j = again.json(now);
        assert_eq!(j["days"].as_array().unwrap().len(), DAYS);
        assert_eq!(j["week"]["agents"]["codex"]["unpriced"], 3300);
        assert_eq!(
            (j["covers"].clone(), spend["covers"].clone(), spend["unpriced"].clone()),
            (json!("Claude only"), json!("Claude only"), json!(["gpt-5-codex"]))
        );
        let mut partial = again.clone();
        let today = again.dates.last().unwrap().clone();
        partial.cells.entry(("claude".into(), today)).or_default().unpriced += 10;
        assert_eq!(partial.json(now)["covers"], "Claude, some models without a price", "never \"Claude only\" while some of Claude is unpriced");
        let mut whole = again.clone();
        whole.cells.retain(|(agent, _), _| agent == "claude");
        assert_eq!(whole.json(now)["covers"], Value::Null, "all of it priced: nothing to say");
    }

    #[test]
    fn a_fork_counts_only_what_it_added_and_a_rollout_without_its_model_says_so() {
        let now = crate::seen::now_ms();
        let dir = tempfile::tempdir().unwrap();
        let codex = dir.path().join("sessions/2027/01/15");
        std::fs::create_dir_all(&codex).unwrap();
        let count = |total: (u64, u64, u64), ago: u64| {
            json!({"timestamp": at(now, ago), "type": "event_msg", "payload": {"type": "token_count",
                "info": {"total_token_usage": {"input_tokens": total.0, "cached_input_tokens": total.1, "output_tokens": total.2}}}})
            .to_string()
        };
        // No turn_context: the model is not known. The fork repeats the parent's two records, then adds its own.
        let parent = [count((1000, 0, 100), 9), count((2000, 0, 200), 8)];
        std::fs::write(codex.join("rollout-parent.jsonl"), parent.join("\n") + "\n").unwrap();
        std::fs::write(codex.join("rollout-fork.jsonl"), [parent[0].clone(), parent[1].clone(), count((2500, 0, 260), 2)].join("\n") + "\n").unwrap();
        // A Claude call of a model with no price, too old to count this week.
        let claude = dir.path().join("projects/-code-api");
        std::fs::create_dir_all(&claude).unwrap();
        let old = json!({"type": "assistant", "timestamp": at(now, 9 * 24 * 60), "requestId": "r", "message": {"id": "m-old", "model": "claude-mystery-1",
            "usage": {"input_tokens": 10, "output_tokens": 5}}});
        std::fs::write(claude.join("s-old.jsonl"), old.to_string() + "\n").unwrap();
        let roots = vec![
            ("codex".to_string(), Transcripts::Codex, dir.path().join("sessions")),
            ("claude".to_string(), Transcripts::Claude, dir.path().join("projects")),
        ];
        let mut s = Scanner::default();
        scan_into(&mut s, &roots, now, &HashSet::new());
        let days = days_of(&s, now);
        let x = days.week_of("codex");
        assert_eq!((x.input, x.output), (2500, 260), "the parent's 2000 once, the fork's own 500");
        assert_eq!(days.unpriced.iter().collect::<Vec<_>>(), ["Codex (model unknown)"], "an older week's model is not this week's");
        assert_eq!(days.spend(), Value::Null, "nothing priced: no spend rather than $0");
        assert_eq!(days.json(now)["covers"], Value::Null);
    }

    #[test]
    fn ticket_costs_split_each_sessions_estimate_by_its_book() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("s-a.json"), r#"{"ENG-1": 3, "ENG-2": 1}"#).unwrap();
        std::fs::write(dir.path().join("s-b.json"), r#"{"ENG-2": 0, "ENG-3": 0}"#).unwrap();
        std::fs::write(dir.path().join("s-gone.json"), r#"{"ENG-1": 1}"#).unwrap();
        // Three quarters of s-c's spend was made on no ticket: ENG-4 gets its quarter, never the whole.
        std::fs::write(dir.path().join("s-c.json"), format!(r#"{{"ENG-4": 1, "{NO_TICKET}": 3}}"#)).unwrap();
        let cost = |session: &str| match session {
            "s-a" => Some(4.0),
            "s-b" => Some(2.0),
            "s-c" => Some(4.0),
            _ => None,
        };
        let mut costs: Vec<(String, f64)> = ticket_costs(dir.path(), &cost).into_iter().collect();
        costs.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(costs, [("ENG-1".to_string(), 3.0), ("ENG-2".to_string(), 2.0), ("ENG-3".to_string(), 1.0), ("ENG-4".to_string(), 1.0)]);
    }

    /// Whether this file system keeps when a file was made: a fork repeats its parent's first lines, so only that orders
    /// the two (Linux file systems without birth times keep the order of their names instead).
    fn birth_times(dir: &Path) -> bool {
        let probe = dir.join("birth");
        std::fs::write(&probe, "").is_ok() && std::fs::metadata(&probe).and_then(|m| m.created()).is_ok()
    }

    #[test]
    fn a_fork_s_repeated_messages_cost_its_parent_once_and_a_deleted_transcript_keeps_its_estimate() {
        let now = crate::seen::now_ms();
        let dir = tempfile::tempdir().unwrap();
        if !birth_times(dir.path()) {
            return;
        }
        let (project, books) = (dir.path().join("projects/-code-api"), dir.path().join("costs"));
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&books).unwrap();
        let call = |id: &str| {
            json!({"type": "assistant", "timestamp": at(now, 5), "requestId": "r", "message": {"id": id, "model": "claude-opus-5-5",
                "usage": {"input_tokens": 1_000_000, "output_tokens": 0}}})
            .to_string()
        };
        let (parent, fork) = (project.join("s-parent.jsonl"), project.join("s-fork.jsonl"));
        std::fs::write(&parent, [call("m1"), call("m2")].join("\n") + "\n").unwrap();
        // Linux keeps creation times on a coarse clock: files made in the same few milliseconds share one.
        std::thread::sleep(std::time::Duration::from_millis(50));
        std::fs::write(&fork, [call("m1"), call("m2"), call("m3")].join("\n") + "\n").unwrap();
        let earlier = std::time::SystemTime::now() - Duration::from_secs(600);
        std::fs::File::options().write(true).open(&parent).unwrap().set_modified(earlier).unwrap();
        std::fs::write(books.join("s-parent.json"), r#"{"ENG-1": 1}"#).unwrap();
        std::fs::write(books.join("s-fork.json"), r#"{"ENG-2": 1}"#).unwrap();
        let roots = vec![("claude".to_string(), Transcripts::Claude, dir.path().join("projects"))];
        let mut s = Scanner::default();
        scan_into(&mut s, &roots, now, &booked(&books));
        let first = estimates_after(&s, &["claude".to_string()], &books, &Value::Null, now);
        assert_eq!((first["sessions"]["s-parent"].as_f64(), first["sessions"]["s-fork"].as_f64()), (Some(8.0), Some(4.0)), "m1 and m2 are the parent's");
        assert_eq!((first["tickets"]["ENG-1"].as_f64(), first["tickets"]["ENG-2"].as_f64()), (Some(8.0), Some(4.0)));
        std::fs::remove_file(&parent).unwrap();
        scan_into(&mut s, &roots, now, &booked(&books));
        let after = estimates_after(&s, &["claude".to_string()], &books, &first, now);
        assert_eq!(after["tickets"]["ENG-1"].as_f64(), Some(8.0), "a deleted transcript keeps its last estimate");
        assert_eq!(after["sessions"]["s-fork"].as_f64(), Some(4.0), "the fork does not take its parent's messages over");
    }

    /// Two transcripts as a fork leaves them: the fork repeats m1 and m2, each goes on with a message of its own, and
    /// every call costs $4.
    fn forked(dir: &Path, now: u64, parent_own: &str, fork_own: &str) -> (PathBuf, PathBuf, PathBuf) {
        let (project, books) = (dir.join("projects/-code-api"), dir.join("costs"));
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&books).unwrap();
        let call = |id: &str| {
            json!({"type": "assistant", "timestamp": at(now, 5), "requestId": "r", "message": {"id": id, "model": "claude-opus-5-5",
                "usage": {"input_tokens": 1_000_000, "output_tokens": 0}}})
            .to_string()
        };
        let (parent, fork) = (project.join("s-parent.jsonl"), project.join("s-fork.jsonl"));
        std::fs::write(&parent, [call("m1"), call("m2"), call(parent_own)].join("\n") + "\n").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
        std::fs::write(&fork, [call("m1"), call("m2"), call(fork_own)].join("\n") + "\n").unwrap();
        std::fs::write(books.join("s-parent.json"), r#"{"ENG-1": 1}"#).unwrap();
        std::fs::write(books.join("s-fork.json"), r#"{"ENG-2": 1}"#).unwrap();
        (parent, fork, books)
    }

    #[test]
    fn a_new_hub_after_the_parent_s_transcript_went_charges_no_message_twice() {
        let now = crate::seen::now_ms();
        let dir = tempfile::tempdir().unwrap();
        if !birth_times(dir.path()) {
            return;
        }
        let (parent, _, books) = forked(dir.path(), now, "m4", "m3");
        let roots = vec![("claude".to_string(), Transcripts::Claude, dir.path().join("projects"))];
        let mut s = Scanner::default();
        scan_into(&mut s, &roots, now, &booked(&books));
        let first = estimates_after(&s, &["claude".to_string()], &books, &Value::Null, now);
        assert_eq!((first["sessions"]["s-parent"].as_f64(), first["sessions"]["s-fork"].as_f64()), (Some(12.0), Some(4.0)));
        std::fs::remove_file(&parent).unwrap();
        // A new hub: a fresh count, with only estimates.json to go on.
        let mut fresh = Scanner { owners: owners_of(&first), ..Scanner::default() };
        scan_into(&mut fresh, &roots, now, &booked(&books));
        let after = estimates_after(&fresh, &["claude".to_string()], &books, &first, now);
        let total: f64 = after["tickets"].as_object().unwrap().values().filter_map(Value::as_f64).sum();
        assert_eq!(total, 16.0, "four messages at $4, each once: {after}");
        assert_eq!((after["sessions"]["s-parent"].as_f64(), after["sessions"]["s-fork"].as_f64()), (Some(12.0), Some(4.0)));
        assert_eq!(after["owned"]["s-parent"], first["owned"]["s-parent"], "the parent keeps what it was charged for");
    }

    #[test]
    fn the_session_that_started_first_owns_what_a_fork_repeats_whatever_was_written_last() {
        let now = crate::seen::now_ms();
        let dir = tempfile::tempdir().unwrap();
        if !birth_times(dir.path()) {
            return;
        }
        let (parent, _, books) = forked(dir.path(), now, "m4", "m3");
        // The parent goes on after the fork, so its file is the one written last.
        std::thread::sleep(Duration::from_millis(20));
        let more = json!({"type": "user", "timestamp": at(now, 1), "message": {"content": "go on"}});
        std::io::Write::write_all(&mut std::fs::OpenOptions::new().append(true).open(&parent).unwrap(), format!("{more}\n").as_bytes()).unwrap();
        let roots = vec![("claude".to_string(), Transcripts::Claude, dir.path().join("projects"))];
        let mut fresh = Scanner::default();
        scan_into(&mut fresh, &roots, now, &booked(&books));
        let est = estimates_after(&fresh, &["claude".to_string()], &books, &Value::Null, now);
        assert_eq!((est["sessions"]["s-parent"].as_f64(), est["sessions"]["s-fork"].as_f64()), (Some(12.0), Some(4.0)), "{est}");
        // The fork's own first line starts later: it never goes first, its file made first or not.
        let late = dir.path().join("late");
        let (parent, fork, books) = forked(&late, now, "m6", "m5");
        let line = |id: &str, ago: u64| {
            json!({"type": "assistant", "timestamp": at(now, ago), "requestId": "r", "message": {"id": id, "model": "claude-opus-5-5",
                "usage": {"input_tokens": 1_000_000, "output_tokens": 0}}})
            .to_string()
        };
        std::fs::write(&fork, [json!({"type": "summary", "timestamp": at(now, 2)}).to_string(), line("m1", 9), line("m5", 1)].join("\n") + "\n").unwrap();
        std::fs::write(&parent, [line("m1", 9), line("m6", 3)].join("\n") + "\n").unwrap();
        let roots = vec![("claude".to_string(), Transcripts::Claude, late.join("projects"))];
        let mut fresh = Scanner::default();
        scan_into(&mut fresh, &roots, now, &booked(&books));
        let est = estimates_after(&fresh, &["claude".to_string()], &books, &Value::Null, now);
        assert_eq!((est["sessions"]["s-parent"].as_f64(), est["sessions"]["s-fork"].as_f64()), (Some(8.0), Some(4.0)), "{est}");
    }

    #[test]
    fn a_new_hub_reads_no_transcript_that_did_not_change() {
        let now = crate::seen::now_ms();
        let dir = tempfile::tempdir().unwrap();
        let (_, _, books) = forked(dir.path(), now, "m4", "m3");
        let roots = vec![("claude".to_string(), Transcripts::Claude, dir.path().join("projects"))];
        let mut s = Scanner::default();
        assert!(scan_into(&mut s, &roots, now, &booked(&books)), "a first count reads them");
        let kept = dir.path().join("usage-scan.json");
        save_scan(&s, &kept);
        let mut again = load_scan(&kept);
        assert!(!scan_into(&mut again, &roots, now, &booked(&books)), "nothing changed, nothing read");
        let (a, b) = (
            estimates_after(&s, &["claude".to_string()], &books, &Value::Null, now),
            estimates_after(&again, &["claude".to_string()], &books, &Value::Null, now),
        );
        assert_eq!((a["sessions"].clone(), a["tickets"].clone()), (b["sessions"].clone(), b["tickets"].clone()));
        assert_eq!(days_of(&s, now), days_of(&again, now));
    }

    #[test]
    fn a_session_gone_for_a_month_folds_into_its_tickets_and_its_book_retires() {
        let now = crate::seen::now_ms();
        let dir = tempfile::tempdir().unwrap();
        let books = dir.path().join("costs");
        std::fs::create_dir_all(&books).unwrap();
        std::fs::write(books.join("s-old.json"), format!(r#"{{"ENG-1": 3, "{NO_TICKET}": 1}}"#)).unwrap();
        let was = json!({"sessions": {"s-old": 8.0}, "owned": {"s-old": [1, 2]}, "gone": {"s-old": now - 31 * DAY}, "archived": {"ENG-1": 1.0}});
        let after = estimates_after(&Scanner::default(), &["claude".to_string()], &books, &was, now);
        assert_eq!(after["tickets"]["ENG-1"].as_f64(), Some(7.0), "its $6 share kept, archived beside the $1 already there: {after}");
        assert_eq!((after["sessions"].get("s-old"), after["owned"].get("s-old"), &after["retired"]), (None, None, &json!(["s-old"])));
        let recent = json!({"sessions": {"s-old": 8.0}, "owned": {"s-old": [1, 2]}, "gone": {"s-old": now - DAY}});
        let kept = estimates_after(&Scanner::default(), &["claude".to_string()], &books, &recent, now);
        assert_eq!(
            (kept["sessions"]["s-old"].as_f64(), kept["owned"]["s-old"].clone(), kept["tickets"]["ENG-1"].as_f64()),
            (Some(8.0), json!([1, 2]), Some(6.0))
        );
        let first = estimates_after(&Scanner::default(), &["claude".to_string()], &books, &json!({"sessions": {"s-old": 8.0}}), now);
        assert_eq!(first["gone"]["s-old"].as_u64(), Some(now), "gone from now on");
    }

    #[test]
    fn the_text_page_names_states_and_estimates() {
        let i = Inputs {
            statusline: json!({"at": NOW, "limits": [{"name": "5h", "percent": 64, "resetsAt": (NOW + HOUR) / 1000}]}),
            codex: from_rollout(&rollout_line(NOW - 3 * HOUR, 12.0)),
            claude_account: json!({"emailAddress": "me@acme.example", "organizationName": "Acme"}),
            installed: vec!["claude".into(), "codex".into(), "opencode".into()],
            ..Inputs::default()
        };
        let mut days = Days { at: NOW, dates: dates(NOW), ..Days::default() };
        let today = days.dates.last().unwrap().clone();
        days.cells.insert(("claude".into(), today), Tokens { input: 1_200_000, cost: 4.8, ..Tokens::default() });
        let lines: Vec<String> = text(&view_of(&i, Some(&days), NOW)).into_iter().map(|(_, l)| l).collect();
        let page = lines.join("\n");
        assert!(page.contains("Claude  ·  as of "), "{page}");
        assert!(page.contains("me@acme.example · Acme"));
        assert!(page.contains("5h        ████████░░│░  64%  resets in 1 h"), "{page}");
        assert!(page.contains("Codex  ·  as of ") && page.contains(" · stale"));
        assert!(page.contains("OpenCode  ·  Not reported by this agent"), "{page}");
        assert!(page.contains("1.2M ~$4.80"));
        assert!(page.contains("Estimated at API prices"));
        let loading: Vec<String> = text(&view_of(&i, None, NOW)).into_iter().map(|(_, l)| l).collect();
        assert!(loading.iter().any(|l| l.contains("Loading usage…")));
    }

    #[test]
    fn the_popup_shows_each_agent_its_windows_and_the_keys() {
        let inputs = Inputs {
            statusline: json!({"at": NOW, "limits": [{"name": "5h", "percent": 64, "resetsAt": (NOW + 95 * 60_000) / 1000}]}),
            claude_account: json!({"emailAddress": "me@acme.example", "organizationName": "Acme"}),
            installed: vec!["claude".into(), "codex".into()],
            ..Inputs::default()
        };
        let mut popup = Popup::new(view_of(&inputs, None, NOW));
        let screen = crate::testing::render(&mut popup, 80, 16);
        assert!(screen.contains("Usage   updated "), "{screen}");
        assert!(screen.contains("Claude  ·  as of "), "{screen}");
        assert!(screen.contains("me@acme.example · Acme"));
        assert!(screen.contains("5h        ████████│░░░  64%  resets in 1 h 35 min"), "{screen}");
        assert!(screen.contains("Codex  ·  No recent reading"), "{screen}");
        assert!(screen.contains("Loading usage…"));
        assert!(screen.lines().last().unwrap().starts_with(" q quit  R refresh  j/k scroll  ? keys"));
        popup.keys = true;
        let keys = crate::testing::render(&mut popup, 80, 16);
        assert!(keys.contains("Keys in usage") && keys.contains("refresh: the agents' files read again"), "{keys}");
        assert!(!keys.contains("^p"), "the popup lists only its own keys: {keys}");
    }

    #[test]
    fn copilot_s_limited_quotas_read_as_windows_and_its_plan_as_the_account() {
        let user = json!({"copilot_plan": "individual", "quota_reset_date": "2027-02-01", "quota_snapshots": {
            "chat": {"entitlement": 200, "remaining": 150, "percent_remaining": 75.0, "unlimited": false},
            "completions": {"entitlement": 0, "remaining": 0, "percent_remaining": 0.0, "unlimited": false},
            "premium_interactions": {"entitlement": 300, "remaining": 30, "percent_remaining": 10.0, "unlimited": false}}});
        let reading = from_copilot(&user, NOW).unwrap();
        let named: Vec<(&str, Option<f64>)> = reading.windows.iter().map(|w| (w.name.as_str(), w.percent)).collect();
        assert_eq!(named, [("premium", Some(90.0)), ("chat", Some(25.0))], "a quota of zero or unlimited is no limit");
        assert_eq!(
            reading.windows[0].resets_at,
            Some(chrono::NaiveDate::from_ymd_opt(2027, 2, 1).unwrap().and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp_millis() as u64)
        );
        let i = Inputs { copilot: Some(reading), installed: vec!["copilot".into()], ..Inputs::default() };
        let copilot = providers(&i, NOW).into_iter().find(|p| p.agent == "copilot").unwrap();
        assert_eq!((copilot.name.as_str(), copilot.windows.len()), ("Copilot", 2));
        assert_eq!(copilot.identity["words"], "GitHub Copilot Individual plan");
        assert!(from_copilot(&json!({"message": "Not Found"}), NOW).is_none(), "no plan, no reading");
    }

    #[test]
    fn copilot_without_gh_is_not_reported_and_a_plan_with_no_limit_is_unlimited() {
        let none = Inputs { installed: vec!["copilot".into()], ..Inputs::default() };
        let copilot = |i: &Inputs| providers(i, NOW).into_iter().find(|p| p.agent == "copilot").unwrap();
        assert_eq!((copilot(&none).state(NOW), copilot(&none).words(NOW).as_str()), ("unreported", "Not reported by this agent"), "gh missing or signed out");
        assert_eq!(limits_of(&providers(&none, NOW), "copilot", NOW), "unreported");
        let free = from_copilot(&json!({"copilot_plan": "business\u{1b}[31m\u{202e}", "quota_snapshots": {"chat": {"unlimited": true}}}), NOW).unwrap();
        assert_eq!(free.plan.as_deref(), Some("business"), "GitHub's words cleaned");
        let i = Inputs { copilot: Some(free), ..none.clone() };
        assert_eq!((copilot(&i).state(NOW), copilot(&i).words(NOW).as_str()), ("unlimited", "Unlimited on this plan"));
        assert_eq!(copilot(&i).identity["words"], "GitHub Copilot Business plan");
        assert_eq!(limits_of(&providers(&i, NOW), "copilot", NOW), "unlimited");
        let long = from_copilot(&json!({"copilot_plan": "x".repeat(200)}), NOW).unwrap();
        assert_eq!(long.plan.unwrap().chars().count(), 32, "and short");
        assert_eq!((limits_of(&[], "codex", NOW), limits_of(&[], "kiro", NOW)), ("reports", "unreported"));
        // Codex's plan is its own: Copilot's never stands for another agent's account.
        let both = Inputs {
            copilot: Some(from_copilot(&json!({"copilot_plan": "pro"}), NOW).unwrap()),
            installed: vec!["codex".into(), "copilot".into()],
            ..Inputs::default()
        };
        assert_eq!(identity_of("codex", &both), Value::Null);
    }

    #[test]
    fn usage_commands_run_side_by_side_and_read_a_capped_answer() {
        let started = std::time::Instant::now();
        let slow = |name: &str| (name.to_string(), format!("sleep 1; printf '[{{\"name\": \"{name}\", \"percent\": 5}}]'"));
        let windows = run_commands(vec![slow("gemini"), slow("kiro"), ("pi".into(), "head -c 200000 /dev/zero".into())], NOW);
        assert!(started.elapsed() < Duration::from_millis(1900), "side by side: {:?}", started.elapsed());
        assert_eq!(windows.iter().map(|w| w.agent.as_str()).collect::<Vec<_>>(), ["gemini", "kiro"], "an oversized answer is cut and read as nothing");
    }

    #[test]
    fn a_usage_command_gives_any_agent_its_windows_and_bad_lines_are_left_out() {
        let printed = json!([{"name": "daily", "percent": 42, "resets_at": "2027-01-16T00:00:00Z"}, {"name": "weekly", "percent": 140}, {"percent": 3}, {"name": "5h", "percent": 7}]);
        let windows = from_command("gemini", &printed, NOW);
        let named: Vec<(&str, Option<f64>, &str)> = windows.iter().map(|w| (w.name.as_str(), w.percent, w.source)).collect();
        assert_eq!(named, [("daily", Some(42.0), COMMAND), ("5h", Some(7.0), COMMAND)]);
        assert_eq!(windows[1].length, Some(5 * HOUR), "a known length places the pace tick");
        assert_eq!(from_command("amp", &json!({"windows": [{"name": "credits", "percent": 12.5}]}), NOW).len(), 1);
        let i = Inputs { commanded: windows, ..Inputs::default() };
        let gemini = providers(&i, NOW).into_iter().find(|p| p.agent == "gemini").unwrap();
        assert_eq!(gemini.windows.len(), 2, "listed though not installed: it has a reading");
        assert_eq!(source_words(COMMAND), "your usage command");
    }

    #[test]
    fn a_weekday_pace_skips_the_weekend() {
        let noon = |d: u32| chrono::NaiveDate::from_ymd_opt(2026, 11, d).unwrap().and_hms_opt(12, 0, 0).unwrap().and_utc().timestamp_millis() as u64;
        let week = 7 * DAY;
        let seven = |start: u64| Window {
            agent: "claude".into(),
            name: "7d".into(),
            percent: Some(30.0),
            status: None,
            resets_at: Some(start + week),
            length: Some(week),
            as_of: start,
            source: STATUSLINE,
        };
        let (wed, thu, fri, mon) = (noon(11), noon(12), noon(13), noon(16));
        assert!((seven(wed).elapsed_with(thu, true).unwrap() - 0.2).abs() < 1e-9);
        assert!((seven(fri).elapsed_with(mon, false).unwrap() - 3.0 / 7.0).abs() < 1e-9);
        assert!((seven(fri).elapsed_with(fri + week, true).unwrap() - 1.0).abs() < 1e-9);
        for hours in [0i64, 13, -10] {
            let tz = chrono::FixedOffset::east_opt(hours as i32 * 3600).unwrap();
            let local_noon = |utc: u64| (utc as i64 - hours * 3_600_000) as u64;
            assert_eq!(weekday_ms(local_noon(fri), local_noon(mon), &tz), DAY, "Friday noon to Monday noon at UTC{hours:+}");
        }
        let five = Window { name: "5h".into(), length: Some(5 * HOUR), resets_at: Some(mon + HOUR), ..seven(mon) };
        assert!((five.elapsed_with(mon, true).unwrap() - 0.8).abs() < 1e-9, "only the 7-day window counts weekdays");
    }
}
