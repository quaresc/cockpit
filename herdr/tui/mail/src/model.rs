//! The mail document as the space lists it: the messages that need me, then the updates, each group newest first.

use chrono::{DateTime, Utc};
use cockpit_connectors::mail::needs_you;
use serde_json::Value;

/// What the space shows of the hub's `mail` document.
#[derive(Clone, Debug, Default)]
pub struct Mail {
    /// The messages that need me first, then the updates, each newest first.
    pub messages: Vec<Message>,
    pub error: Option<String>,
    /// When the list was last fetched, in epoch milliseconds; None before any fetch.
    pub at: Option<i64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    pub id: String,
    pub from: String,
    pub name: String,
    pub subject: String,
    pub snippet: String,
    pub url: String,
    pub date: Option<DateTime<Utc>>,
    pub unread: bool,
    pub needs: bool,
    /// The message as the document holds it, what the hub's actions take.
    pub raw: Value,
}

fn s(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or_default().to_string()
}

impl Mail {
    pub fn parse(doc: Option<&Value>) -> Self {
        let Some(doc) = doc.filter(|d| d.is_object()) else { return Self::default() };
        let mut messages: Vec<Message> = doc["messages"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|m| m["id"].as_str().is_some_and(|id| !id.is_empty()))
            .map(|m| Message {
                id: s(m, "id"),
                from: s(m, "from"),
                name: Some(s(m, "fromName")).filter(|n| !n.is_empty()).unwrap_or_else(|| s(m, "from")),
                subject: s(m, "subject"),
                snippet: s(m, "snippet"),
                url: s(m, "url"),
                date: m["date"].as_str().and_then(|d| DateTime::parse_from_rfc3339(d).ok()).map(|d| d.with_timezone(&Utc)),
                unread: m["unread"] == true,
                needs: needs_you(m),
                raw: m.clone(),
            })
            .collect();
        messages.sort_by_key(|m| (!m.needs, std::cmp::Reverse(m.date)));
        Self { messages, error: doc["error"].as_str().map(str::to_string), at: doc["at"].as_i64().filter(|at| *at > 0) }
    }

    pub fn needs(&self) -> usize {
        self.messages.iter().filter(|m| m.needs).count()
    }

    pub fn message(&self, id: &str) -> Option<&Message> {
        self.messages.iter().find(|m| m.id == id)
    }
}

impl Message {
    /// The subject, or a word saying it has none.
    pub fn title(&self) -> &str {
        if self.subject.is_empty() { "(no subject)" } else { &self.subject }
    }
}

/// How long ago, in one short word: "now", "12m", "3h", "2d", "5w".
pub fn age(date: Option<DateTime<Utc>>, now: DateTime<Utc>) -> String {
    let Some(date) = date else { return String::new() };
    let mins = (now - date).num_minutes().max(0);
    match mins {
        0 => "now".into(),
        m if m < 60 => format!("{m}m"),
        m if m < 48 * 60 => format!("{}h", m / 60),
        m if m < 14 * 24 * 60 => format!("{}d", m / (24 * 60)),
        m => format!("{}w", m / (7 * 24 * 60)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn people_come_first_then_updates_each_newest_first() {
        let doc = json!({"at": 5, "messages": [
            {"id": "a", "from": "noreply@ci.example.com", "fromName": "CI", "subject": "Build passed", "date": "2026-11-04T10:00:00Z", "unread": true},
            {"id": "b", "from": "maya@example.com", "fromName": "Maya", "subject": "Lunch?", "date": "2026-11-04T08:00:00Z", "unread": true},
            {"id": "c", "from": "sam@example.com", "fromName": "", "subject": "", "date": "2026-11-04T09:00:00Z", "labels": ["CATEGORY_UPDATES"]},
            {"id": "d", "from": "leo@example.com", "fromName": "Leo", "subject": "Review", "date": "2026-11-04T09:30:00Z", "labels": ["IMPORTANT"]},
            {"id": "", "from": "x@example.com"}
        ], "error": "Mail: HTTP 502"});
        let mail = Mail::parse(Some(&doc));
        let ids: Vec<&str> = mail.messages.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["d", "b", "a", "c"]);
        assert_eq!(mail.needs(), 2);
        assert_eq!((mail.messages[3].name.as_str(), mail.messages[3].title()), ("sam@example.com", "(no subject)"));
        assert_eq!((mail.error.as_deref(), mail.at), (Some("Mail: HTTP 502"), Some(5)));
        assert!(Mail::parse(None).messages.is_empty());
        let now = DateTime::parse_from_rfc3339("2026-11-04T11:00:00Z").unwrap().with_timezone(&Utc);
        assert_eq!(age(mail.messages[1].date, now), "3h");
        assert_eq!(age(mail.messages[0].date, now), "1h");
        assert_eq!(age(None, now), "");
    }
}
