//! The monitor document as the mod writes it: every app of the environment, most recent deploy first, and for the
//! watched ones their monitors, error and latency series, Error Tracking issues and releases with a verdict.

use chrono::{DateTime, FixedOffset, Local, NaiveDateTime, Utc};
use serde::Deserialize;
use serde_json::Value;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Doc {
    /// When the mod wrote it, in milliseconds.
    pub at: Option<i64>,
    pub env: String,
    pub errors: Vec<Value>,
    pub apps: Vec<App>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct App {
    pub id: String,
    pub name: String,
    pub service: String,
    pub repo: String,
    pub health: String,
    pub tag: String,
    #[serde(deserialize_with = "time")]
    pub deployed_at: Option<DateTime<Utc>>,
    pub behind: i64,
    pub argo_url: String,
    pub datadog_url: String,
    pub watched: bool,
    pub pinned: bool,
    pub monitors: Option<Monitors>,
    pub series: Option<Series>,
    pub issues: Vec<Issue>,
    pub releases: Vec<Release>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Monitors {
    pub ok: u32,
    pub warn: u32,
    pub alert: u32,
    pub nodata: u32,
    pub alerting: Vec<Monitor>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Monitor {
    pub id: Value,
    pub name: String,
    pub status: String,
    pub url: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Series {
    pub errors: Vec<f64>,
    /// Seconds.
    pub p95: Vec<f64>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Issue {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub message: String,
    pub count: u64,
    #[serde(deserialize_with = "time")]
    pub first_seen: Option<DateTime<Utc>>,
    pub first_version: String,
    pub url: String,
    pub is_new: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Release {
    pub id: Value,
    pub tag: String,
    pub actor: String,
    pub is_mine: bool,
    pub phase: String,
    #[serde(deserialize_with = "time")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(deserialize_with = "time")]
    pub finished_at: Option<DateTime<Utc>>,
    pub tickets: Vec<Link>,
    pub prs: Vec<Pr>,
    pub verdict: Verdict,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Link {
    pub id: String,
    pub url: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Pr {
    pub repo: String,
    pub number: u64,
    pub url: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Verdict {
    pub state: State,
    pub summary: String,
    pub before: Option<Window>,
    pub after: Option<Window>,
    pub new_issues: Vec<String>,
    #[serde(deserialize_with = "time")]
    pub until: Option<DateTime<Utc>>,
}

/// Errors, requests and p95 (seconds) over the window before or after a release.
#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Window {
    pub errors: f64,
    pub hits: f64,
    pub p95: f64,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Watching,
    Ok,
    Moved,
    Failed,
    #[default]
    #[serde(other)]
    Unknown,
}

impl State {
    pub fn icon(self) -> &'static str {
        match self {
            State::Ok => "✅",
            State::Moved => "⚠️",
            State::Watching => "⏳",
            State::Failed => "❌",
            State::Unknown => "❔",
        }
    }

    pub fn is_bad(self) -> bool {
        matches!(self, State::Moved | State::Failed)
    }
}

impl Doc {
    /// The document, or None when it does not read as one.
    pub fn parse(value: &Value) -> Option<Doc> {
        serde_json::from_value(value.clone()).ok()
    }

    pub fn app(&self, id: &str) -> Option<&App> {
        self.apps.iter().find(|a| a.id == id)
    }

    /// The error lines the mod left, each as one text.
    pub fn error_lines(&self) -> Vec<String> {
        self.errors.iter().map(|e| e.as_str().map(str::to_string).unwrap_or_else(|| e.to_string())).collect()
    }
}

impl App {
    pub fn alerting(&self) -> &[Monitor] {
        self.monitors.as_ref().map(|m| m.alerting.as_slice()).unwrap_or_default()
    }

    pub fn new_issues(&self) -> impl Iterator<Item = &Issue> {
        self.issues.iter().filter(|i| i.is_new)
    }

    /// A release that moved or failed, or a monitor not OK: the app shows first, unfolded.
    pub fn is_troubled(&self) -> bool {
        self.releases.iter().any(|r| r.verdict.state.is_bad()) || !self.alerting().is_empty()
    }

    pub fn release(&self, id: &str) -> Option<&Release> {
        self.releases.iter().find(|r| id_text(&r.id) == id)
    }

    /// The tag a roll back of this release goes to: the release before it that succeeded, else the one before it;
    /// None unless it is the app's newest release.
    pub fn previous_tag(&self, release: &str) -> Option<String> {
        let newest = self.releases.first().filter(|r| id_text(&r.id) == release)?;
        let older = &self.releases[1..];
        let back = older.iter().find(|r| r.phase == "Succeeded" && r.tag != newest.tag).or(older.first())?;
        Some(back.tag.clone())
    }

    /// The monitor counts as a row shows them: "12 ok · 1 alert".
    pub fn counts(&self) -> Option<String> {
        let m = self.monitors.as_ref()?;
        let mut parts = vec![format!("{} ok", m.ok)];
        if m.alert > 0 {
            parts.push(format!("{} alert", m.alert));
        }
        if m.warn > 0 {
            parts.push(format!("{} warn", m.warn));
        }
        Some(parts.join(" · "))
    }
}

impl Release {
    /// When it reached production, else when it started.
    pub fn at(&self) -> Option<DateTime<Utc>> {
        self.finished_at.or(self.started_at)
    }

    /// Who shipped it: "you" for Clement's own.
    pub fn who(&self) -> String {
        if self.actor == "me" || (self.is_mine && self.actor.is_empty()) { "you".into() } else { self.actor.clone() }
    }
}

/// An RFC 3339 time, None when missing, null or unreadable.
fn time<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<DateTime<Utc>>, D::Error> {
    let text = Option::<String>::deserialize(d)?;
    Ok(text.and_then(|t| DateTime::parse_from_rfc3339(&t).ok()).map(|t| t.with_timezone(&Utc)))
}

/// An id as text, whether the document gives a number or a string.
pub fn id_text(id: &Value) -> String {
    id.as_str().map(str::to_string).unwrap_or_else(|| id.to_string())
}

/// The time it is, and the zone it is shown in: fixed in tests, the system's otherwise.
#[derive(Clone, Copy, Debug)]
pub struct Clock {
    pub now: DateTime<Utc>,
    pub zone: Option<FixedOffset>,
}

impl Clock {
    pub fn system() -> Self {
        Self { now: Utc::now(), zone: None }
    }

    pub fn local(&self, at: DateTime<Utc>) -> NaiveDateTime {
        match self.zone {
            Some(zone) => at.with_timezone(&zone).naive_local(),
            None => at.with_timezone(&Local).naive_local(),
        }
    }

    pub fn hm(&self, at: DateTime<Utc>) -> String {
        self.local(at).format("%H:%M").to_string()
    }

    pub fn is_today(&self, at: DateTime<Utc>) -> bool {
        self.local(at).date() == self.local(self.now).date()
    }

    /// How long ago, in the largest unit: "now", "12 min ago", "2 h ago", "8 d ago".
    pub fn ago(&self, at: DateTime<Utc>) -> String {
        let mins = (self.now - at).num_minutes();
        match mins {
            m if m < 1 => "now".into(),
            m if m < 60 => format!("{m} min ago"),
            m if m < 48 * 60 => format!("{} h ago", m / 60),
            m => format!("{} d ago", m / (24 * 60)),
        }
    }

    pub fn minute(&self) -> i64 {
        self.now.timestamp() / 60
    }
}

/// A p95 in seconds as a short text: "161 ms", "<1 ms", "1.2 s".
pub fn duration(seconds: f64) -> String {
    let ms = seconds * 1000.0;
    if ms < 1.0 {
        "<1 ms".into()
    } else if ms < 1000.0 {
        format!("{} ms", ms.round())
    } else {
        format!("{:.1} s", seconds)
    }
}

/// An error count as a short text: "14", "1.2k".
pub fn count(n: f64) -> String {
    if n >= 1000.0 { format!("{:.1}k", n / 1000.0) } else { format!("{}", n.round()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_and_counts() {
        assert_eq!(duration(0.161), "161 ms");
        assert_eq!(duration(0.0003), "<1 ms");
        assert_eq!(duration(1.24), "1.2 s");
        assert_eq!(count(14.0), "14");
        assert_eq!(count(1234.0), "1.2k");
    }

    #[test]
    fn an_unknown_state_reads_as_unknown() {
        let verdict: Verdict = serde_json::from_value(serde_json::json!({"state": "weird"})).unwrap();
        assert_eq!(verdict.state, State::Unknown);
    }
}
