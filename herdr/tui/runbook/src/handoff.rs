//! What the other parts of cockpit say about a step: the session a step was handed to, and the inbox's view of its PR.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use cockpit_core::theme::{GREEN, ORANGE, RED, bold, fg};
use ratatui::style::{Modifier, Style};
use serde_json::Value;

/// A handoff no session took after SENT_WAIT seconds says so; a success shows under its step for DONE_SHOWN seconds.
const SENT_WAIT: f64 = 900.0;
const DONE_SHOWN: f64 = 1800.0;
pub const MERGED: &str = "\u{f419}";

pub fn dir() -> PathBuf {
    cockpit_core::files::cache().join("handoffs")
}

pub fn inbox_path() -> PathBuf {
    cockpit_core::files::cache().join("inbox.json")
}

fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or_default()
}

fn ms(handoff: &Value, key: &str) -> f64 {
    handoff[key].as_f64().unwrap_or_default() / 1000.0
}

pub fn read() -> HashMap<String, Value> {
    let Ok(entries) = std::fs::read_dir(dir()) else { return HashMap::new() };
    entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| cockpit_core::files::load(&e.path()))
        .filter_map(|h| Some((h.get("id")?.as_str()?.to_string(), h)))
        .collect()
}

/// Whether a session still has the step: a handoff nobody took for SENT_WAIT seconds no longer counts.
pub fn in_flight(handoff: &Value) -> bool {
    match handoff["state"].as_str().unwrap_or_default() {
        "sent" => now() - ms(handoff, "sentAt") <= SENT_WAIT,
        state => state == "running" || state == "waiting",
    }
}

/// What the step's session made of it, shown under the step, or None once a success is old news.
pub fn line(handoff: &Value) -> Option<(String, Style)> {
    let message = handoff["message"].as_str().unwrap_or_default();
    Some(match handoff["state"].as_str().unwrap_or_default() {
        "sent" if now() - ms(handoff, "sentAt") > SENT_WAIT => ("↳ no Claude session took it: is the cockpit plugin loaded there?".into(), fg(RED)),
        "sent" => ("↳ sent to its session…".into(), fg(ORANGE)),
        "running" => ("↳ running in its session…".into(), fg(ORANGE)),
        "waiting" => (format!("↳ ⏸ waits for your go · {message}"), bold(ORANGE)),
        "unreported" => ("↳ its session stopped without saying how it went".into(), fg(RED)),
        "done" if now() - ms(handoff, "reportedAt") < DONE_SHOWN => (format!("↳ {message}"), fg(GREEN).add_modifier(Modifier::DIM)),
        "done" => return None,
        state => (format!("↳ {state}: {message}"), fg(RED)),
    })
}

/// What the inbox knows about a PR right now: open with its CI, or merged.
pub fn live_badge(repo: &str, number: u64, inbox: &Value) -> Option<(String, Style)> {
    let is = |pr: &&Value| pr["repo"].as_str() == Some(repo) && pr["number"].as_u64() == Some(number);
    let list = |key: &str| inbox[key].as_array().cloned().unwrap_or_default();
    if let Some(pr) = list("mine").iter().find(is) {
        if matches!(pr["ci"].as_str(), Some("FAILURE" | "ERROR")) {
            return Some(("ci✗".into(), fg(RED)));
        }
        return Some(if pr["review"].as_str() != Some("APPROVED") { ("open".into(), fg(ORANGE)) } else { ("approved".into(), fg(GREEN)) });
    }
    list("merged").iter().find(is).map(|_| (format!("{MERGED} merged"), fg(GREEN)))
}

