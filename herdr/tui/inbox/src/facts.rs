//! What inbox.py derives from a cache entry: ages, the CI and review glyphs, a ticket's status and links, a dashboard's
//! name; and the config values it reads.

use std::sync::{LazyLock, OnceLock};

use chrono::{DateTime, Local, TimeZone, Utc};
use cockpit_core::theme::{DIM, GREEN, ORANGE, RED, fg};
use ratatui::style::Style;
use regex::Regex;
use serde_json::Value;

use crate::icons::icon;

pub const OLD_REVIEW_DAYS: f64 = 60.0;
pub const STALE_PR_DAYS: f64 = 30.0;
pub const PREVIEW_WARN_HOURS: f64 = 3.0;
pub const RED_ALERT_HOURS: f64 = 2.0;

pub fn s<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}

pub fn flag(v: &Value, key: &str) -> bool {
    v[key].as_bool().unwrap_or(false)
}

pub fn list<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v[key].as_array().map(Vec::as_slice).unwrap_or(&[])
}

/// A field as text, whether the cache holds it as a string or a number.
pub fn text(v: &Value, key: &str) -> String {
    match &v[key] {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

pub fn reference(pr: &Value) -> String {
    format!("{}#{}", s(pr, "repo"), text(pr, "number"))
}

/// A fixed "now" for snapshot tests, in UTC, so ages and clocks read the same on every run.
static FROZEN: OnceLock<DateTime<Utc>> = OnceLock::new();

pub fn freeze(at: &str) {
    if let Ok(at) = DateTime::parse_from_rfc3339(at) {
        let _ = FROZEN.set(at.with_timezone(&Utc));
    }
}

fn now() -> DateTime<Utc> {
    FROZEN.get().copied().unwrap_or_else(Utc::now)
}

/// Days since an ISO date; None when it is missing or unreadable (a connector field left unmapped).
pub fn age_days(iso: &str) -> Option<f64> {
    let at = DateTime::parse_from_rfc3339(iso).ok()?;
    Some((now() - at.with_timezone(&Utc)).num_milliseconds() as f64 / 86_400_000.0)
}

pub fn ago(iso: &str) -> String {
    match age_days(iso) {
        None => String::new(),
        Some(days) if days < 1.0 / 24.0 => format!("{}m", (days * 1440.0) as i64),
        Some(days) if days < 2.0 => format!("{}h", (days * 24.0) as i64),
        Some(days) => format!("{}d", days as i64),
    }
}

/// A cache's millisecond timestamp as local HH:MM.
pub fn clock(ms: &Value) -> String {
    let ms = ms.as_i64().unwrap_or(0);
    let shown = if FROZEN.get().is_some() { Utc.timestamp_millis_opt(ms).single().map(|t| t.format("%H:%M").to_string()) } else { Local.timestamp_millis_opt(ms).single().map(|t| t.format("%H:%M").to_string()) };
    shown.unwrap_or_default()
}

pub fn today() -> String {
    match FROZEN.get() {
        Some(at) => at.format("%Y-%m-%d").to_string(),
        None => Local::now().format("%Y-%m-%d").to_string(),
    }
}

pub fn is_failing(pr: &Value) -> bool {
    matches!(s(pr, "ci"), "FAILURE" | "ERROR")
}

pub fn needs_me(pr: &Value) -> bool {
    is_failing(pr) || s(pr, "review") == "CHANGES_REQUESTED"
}

/// CI: green check circle, red cross circle, orange hourglass while running, dot when none.
pub fn ci_glyph(pr: &Value) -> (&'static str, Style) {
    if is_failing(pr) {
        (icon("fail"), fg(RED))
    } else if s(pr, "ci") == "SUCCESS" {
        (icon("ok"), fg(GREEN))
    } else if !s(pr, "ci").is_empty() {
        (icon("running"), fg(ORANGE))
    } else {
        ("·", fg(DIM))
    }
}

/// Review: check when approved, speech bubble when changes are requested, clock while waiting.
pub fn review_glyph(pr: &Value) -> (&'static str, Style) {
    match s(pr, "review") {
        "APPROVED" => (icon("approved"), fg(GREEN)),
        "CHANGES_REQUESTED" => (icon("changes"), fg(RED)),
        _ => (icon("waiting"), fg(DIM)),
    }
}

pub fn first_name(pr: &Value) -> String {
    s(pr, "authorName").split_whitespace().next().unwrap_or(s(pr, "author")).to_string()
}

pub fn short_status(ticket: &Value) -> String {
    static PARENS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s*\(.*\)").expect("a valid regex"));
    PARENS.replace_all(s(ticket, "status"), "").to_lowercase()
}

pub fn is_deploying(ticket: &Value) -> bool {
    s(ticket, "statusType") == "started" && s(ticket, "status").to_lowercase().contains("deploy")
}

/// Whether a branch names the ticket as a whole token: eng-4-login does, eng-42-login does not.
pub fn has_ticket(branch: &str, ticket: &str) -> bool {
    let (branch, key) = (branch.to_lowercase(), ticket.to_lowercase());
    !key.is_empty()
        && branch.match_indices(&key).any(|(at, _)| {
            let before = branch[..at].chars().next_back();
            let after = branch[at + key.len()..].chars().next();
            !before.is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit()) && !after.is_some_and(|c| c.is_ascii_digit())
        })
}

/// What a dashboard link is, from its host: ArgoCD, Vercel, Netlify, Render, Railway, Fly, else Dashboard.
pub fn dashboard_name(url: &str) -> &'static str {
    static ARGO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(^|\.)argo(cd)?[.-]").expect("a valid regex"));
    let host = url.split_once("://").map(|(_, rest)| rest.split('/').next().unwrap_or("")).unwrap_or("");
    if ARGO.is_match(host) {
        return "ArgoCD";
    }
    let names = [("vercel.com", "Vercel"), ("netlify.app", "Netlify"), ("netlify.com", "Netlify"), ("render.com", "Render"), ("railway.app", "Railway"), ("fly.io", "Fly")];
    names.iter().find(|(suffix, _)| host.ends_with(suffix)).map(|(_, name)| *name).unwrap_or("Dashboard")
}

/// The config values the inbox reads, from the config the mod resolved.
pub struct Config {
    raw: Value,
}

impl Config {
    pub fn load() -> Self {
        Self::from(cockpit_core::files::config())
    }

    pub fn from(raw: Value) -> Self {
        crate::icons::set_ci(s(&raw["ci"], "name"));
        Self { raw }
    }

    /// The preview's dashboard, from the configured URL template; None when there is none.
    pub fn dashboard_url(&self, preview: &str) -> Option<String> {
        static PLACEHOLDER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{(\w+)\}").expect("a valid regex"));
        let template = s(&self.raw["platform"], "previewDashboardUrl");
        let value = |c: &regex::Captures| if &c[1] == "preview" { quote(preview) } else { String::new() };
        (!template.is_empty()).then(|| PLACEHOLDER.replace_all(template, value).into_owned())
    }

    /// Whether a green image with no preview can get one in a press.
    pub fn can_preview(&self) -> bool {
        let platform = &self.raw["platform"];
        let tools = &platform["tools"];
        flag(platform, "enabled") && (tools.get("createPreview").is_some() || tools.as_array().is_some_and(|t| t.iter().any(|t| t == "createPreview")))
    }

    /// The CI connector's name when GitHub checks only mirror its jobs.
    pub fn mirror(&self) -> Option<String> {
        let ci = &self.raw["ci"];
        (!s(ci, "server").is_empty()).then(|| s(ci, "name").to_lowercase())
    }

    /// Whether a herdr space label, "[N] <icon>  <name>", is one of the pinned spaces.
    pub fn is_pinned(&self, label: &str) -> bool {
        list(&self.raw, "spaces").iter().any(|space| {
            let name = space.as_str().unwrap_or_else(|| s(space, "name"));
            label.ends_with(&format!("  {name}"))
        })
    }
}

/// A URL component escaped as Python's quote(safe="") does.
fn quote(text: &str) -> String {
    text.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"_.-~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}
