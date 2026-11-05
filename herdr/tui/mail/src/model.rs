//! The mail document as the space lists it, as a mail client does: the latest unread messages, then the latest read
//! ones, each group newest first.

use chrono::{DateTime, Utc};
use cockpit_connectors::mail::unescape;
use serde_json::Value;

/// What the space shows of the hub's `mail` document.
#[derive(Clone, Debug, Default)]
pub struct Mail {
    /// The latest unread messages, then the latest read ones, each newest first.
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
                subject: unescape(&s(m, "subject")),
                snippet: unescape(&s(m, "snippet")),
                url: s(m, "url"),
                date: m["date"].as_str().and_then(|d| DateTime::parse_from_rfc3339(d).ok()).map(|d| d.with_timezone(&Utc)),
                unread: m["unread"] == true,
                raw: m.clone(),
            })
            .collect();
        messages.sort_by_key(|m| (m.group(), std::cmp::Reverse(m.date)));
        let mut shown = [0usize; 2];
        messages.retain(|m| {
            let n = &mut shown[m.group() as usize];
            *n += 1;
            *n <= PER_GROUP
        });
        Self { messages, error: doc["error"].as_str().map(str::to_string), at: doc["at"].as_i64().filter(|at| *at > 0) }
    }

    pub fn unread(&self) -> usize {
        self.messages.iter().filter(|m| m.unread).count()
    }

    pub fn message(&self, id: &str) -> Option<&Message> {
        self.messages.iter().find(|m| m.id == id)
    }
}

/// How many messages each group lists, the latest.
pub const PER_GROUP: usize = 15;

/// The list's groups in their order: key, label.
pub const GROUPS: [(Group, &str, &str); 2] = [(Group::Unread, "unread", "Unread"), (Group::Read, "read", "Read")];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    Unread,
    Read,
}

impl Message {
    /// Where the list puts it.
    pub fn group(&self) -> Group {
        if self.unread { Group::Unread } else { Group::Read }
    }

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
    fn the_latest_unread_come_first_then_the_latest_read() {
        let doc = json!({"at": 5, "messages": [
            {"id": "a", "from": "noreply@ci.example.com", "fromName": "CI", "subject": "Build passed", "date": "2026-11-04T10:00:00Z", "unread": true},
            {"id": "b", "from": "maya@example.com", "fromName": "Maya", "subject": "Lunch?", "date": "2026-11-04T08:00:00Z", "unread": true},
            {"id": "c", "from": "sam@example.com", "fromName": "", "subject": "", "date": "2026-11-04T09:00:00Z", "unread": false},
            {"id": "d", "from": "leo@example.com", "fromName": "Leo", "subject": "Review", "date": "2026-11-04T09:30:00Z", "unread": true},
            {"id": "", "from": "x@example.com"}
        ], "error": "Mail: HTTP 502"});
        let mail = Mail::parse(Some(&doc));
        let ids: Vec<&str> = mail.messages.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["a", "d", "b", "c"]);
        assert_eq!(mail.unread(), 3);
        assert_eq!((mail.messages[3].name.as_str(), mail.messages[3].title()), ("sam@example.com", "(no subject)"));
        let many: Vec<Value> = (0..20).map(|i| json!({"id": format!("r{i}"), "date": format!("2026-11-04T10:{i:02}:00Z"), "unread": false})).collect();
        let read = Mail::parse(Some(&json!({"messages": many})));
        assert_eq!((read.messages.len(), read.messages[0].id.as_str()), (PER_GROUP, "r19"), "the latest of each group only");
        assert_eq!((mail.error.as_deref(), mail.at), (Some("Mail: HTTP 502"), Some(5)));
        assert!(Mail::parse(None).messages.is_empty());
        let now = DateTime::parse_from_rfc3339("2026-11-04T11:00:00Z").unwrap().with_timezone(&Utc);
        assert_eq!(age(mail.messages[2].date, now), "3h");
        assert_eq!(age(mail.messages[0].date, now), "1h");
        assert_eq!(age(None, now), "");
    }
}
