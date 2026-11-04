//! My mail through the hub's own mail server (the config's `mail` section): the store's `mail` document, and the new
//! unread messages worth a notification.

use std::collections::HashSet;

use cockpit_connectors::mail;
use cockpit_connectors::tools::Mapped;
use serde_json::{Value, json};

use crate::value::list;

/// The ids `notified` keeps, so a message that leaves the list and comes back is not announced again.
const KEEP_NOTIFIED: usize = 300;

/// My inbox through the mail server; None when no server serves the `mail` section.
pub fn fetch(raw: &Value, config: &Value) -> Option<Result<Vec<Value>, String>> {
    let server = crate::feed::mcp::server(raw, config, "mail")?;
    let mapping = mail::mapping(&raw["mail"]);
    Some(crate::feed::mcp::with(&server, async |c| mail::inbox(&Mapped { connection: c, section: &mapping }, &mapping).await))
}

/// The `mail` document after a fetch: its messages, and every unread one in `notified`; a failed fetch keeps the
/// messages and says why.
pub fn merge(doc: &Value, fetched: Result<Vec<Value>, String>, now: u64) -> Value {
    match fetched {
        Ok(messages) => {
            let unread: Vec<Value> = messages.iter().filter(|m| m["unread"] == true).map(|m| m["id"].clone()).collect();
            let fresh: HashSet<&Value> = unread.iter().collect();
            let kept = list(&doc["notified"]).iter().filter(|id| !fresh.contains(id)).cloned();
            let notified: Vec<Value> = unread.iter().cloned().chain(kept).take(KEEP_NOTIFIED).collect();
            json!({"at": now, "by": "hub", "messages": messages, "notified": notified})
        }
        Err(error) => {
            let mut doc = if doc.is_object() { doc.clone() } else { json!({"at": 0, "by": "hub", "messages": [], "notified": []}) };
            doc["error"] = json!(error);
            doc
        }
    }
}

/// The unread messages new since the last fetch, as notifications (title, body, click); none on the first fetch, and
/// one summing them up past three.
pub fn news(before: &Value, after: &Value) -> Vec<(String, String, String)> {
    if !before.is_object() || before["at"].as_u64().unwrap_or(0) == 0 || after["error"].is_string() {
        return Vec::new();
    }
    let known: HashSet<&Value> = list(&before["notified"]).iter().collect();
    let new: Vec<&Value> = list(&after["messages"]).iter().filter(|m| m["unread"] == true && !known.contains(&m["id"])).collect();
    let text = |m: &Value, key: &str| m[key].as_str().unwrap_or_default().to_string();
    if new.len() > 3 {
        let lines: Vec<String> = new.iter().take(3).map(|m| format!("{}: {}", text(m, "fromName"), text(m, "subject"))).collect();
        return vec![(format!("✉️ {} new messages", new.len()), format!("{}\n…", lines.join("\n")), "space:inbox".into())];
    }
    new.iter()
        .map(|m| {
            let url = text(m, "url");
            let click = if url.is_empty() { "space:inbox".to_string() } else { url };
            let subject = text(m, "subject");
            (format!("✉️ {}", text(m, "fromName")), if subject.is_empty() { text(m, "snippet") } else { subject }, click)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(id: &str, unread: bool) -> Value {
        json!({"id": id, "fromName": "Ana", "subject": format!("About {id}"), "url": format!("https://mail/{id}"), "unread": unread})
    }

    #[test]
    fn a_fetch_keeps_the_unread_ids_and_a_failure_keeps_the_messages() {
        let before = json!({"at": 5, "by": "hub", "messages": [message("a", true)], "notified": ["a", "gone"], "error": "Mail: old"});
        let after = merge(&before, Ok(vec![message("b", true), message("c", false)]), 9);
        assert_eq!(after, json!({"at": 9, "by": "hub", "messages": [message("b", true), message("c", false)], "notified": ["b", "a", "gone"]}));
        let failed = merge(&after, Err("Mail: HTTP 502".into()), 12);
        assert_eq!((&failed["messages"], &failed["at"], &failed["error"]), (&after["messages"], &json!(9), &json!("Mail: HTTP 502")));
        assert_eq!(merge(&Value::Null, Err("x".into()), 1)["messages"], json!([]));
    }

    #[test]
    fn news_is_the_unread_messages_not_announced_yet() {
        let first = merge(&Value::Null, Ok(vec![message("a", true)]), 1);
        assert_eq!(news(&Value::Null, &first), [], "the first fetch announces nothing");
        let second = merge(&first, Ok(vec![message("b", true), message("c", false), message("a", true)]), 2);
        assert_eq!(news(&first, &second), [("✉️ Ana".to_string(), "About b".to_string(), "https://mail/b".to_string())]);
        let many: Vec<Value> = ["d", "e", "f", "g"].iter().map(|id| message(id, true)).collect();
        let third = merge(&second, Ok(many), 3);
        let told = news(&second, &third);
        assert_eq!((told.len(), told[0].0.as_str(), told[0].2.as_str()), (1, "✉️ 4 new messages", "space:inbox"));
        assert!(told[0].1.starts_with("Ana: About d\nAna: About e"));
        assert_eq!(news(&third, &merge(&third, Err("down".into()), 4)), []);
    }
}
