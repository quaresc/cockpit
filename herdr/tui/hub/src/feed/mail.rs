//! My mail through the hub's own mail server (the config's `mail` section): the store's `mail` document, the new
//! unread messages worth a notification, and the threads read whole, kept a week in ~/.cache/cockpit/mail/.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use cockpit_connectors::mail;
use cockpit_connectors::tools::Mapped;
use serde_json::{Value, json};

use crate::value::list;

/// A thread read whole is kept this long.
const KEEP_THREADS: Duration = Duration::from_secs(7 * 86_400);

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
        return vec![(format!("✉️ {} new messages", new.len()), format!("{}\n…", lines.join("\n")), "space:mail".into())];
    }
    new.iter()
        .map(|m| {
            let subject = text(m, "subject");
            (format!("✉️ {}", text(m, "fromName")), if subject.is_empty() { text(m, "snippet") } else { subject }, format!("mail:{}", text(m, "id")))
        })
        .collect()
}

/// The folder of the threads read whole.
pub fn threads_dir() -> PathBuf {
    cockpit_core::files::cache().join("mail")
}

/// Where a message's thread is kept, as `{at, id, messages, error?}`.
pub fn thread_path(id: &str) -> PathBuf {
    threads_dir().join(format!("{}.json", mail::cache_name(id)))
}

/// Keeps a thread read whole, or why it could not be read.
pub fn keep_thread(id: &str, read: Result<Vec<Value>, String>) {
    let mut doc = json!({"at": super::now_ms(), "id": id, "messages": []});
    match read {
        Ok(messages) => doc["messages"] = Value::from(messages),
        Err(error) => doc["error"] = Value::from(error.chars().take(300).collect::<String>()),
    }
    let _ = cockpit_core::files::write(&thread_path(id), &doc);
}

/// The message as the `mail` document lists it, else its id alone (its thread then taken for its own).
pub fn listed(id: &str) -> Value {
    let doc = crate::store::global().get("mail");
    list(&doc["messages"]).iter().find(|m| m["id"] == id).cloned().unwrap_or_else(|| json!({"id": id, "threadId": id}))
}

/// Drops the threads, pages and marks older than a week.
pub fn prune_threads() {
    for entry in std::fs::read_dir(threads_dir()).into_iter().flatten().flatten() {
        let old = entry.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > KEEP_THREADS);
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
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
        assert_eq!(news(&first, &second), [("✉️ Ana".to_string(), "About b".to_string(), "mail:b".to_string())]);
        let many: Vec<Value> = ["d", "e", "f", "g"].iter().map(|id| message(id, true)).collect();
        let third = merge(&second, Ok(many), 3);
        let told = news(&second, &third);
        assert_eq!((told.len(), told[0].0.as_str(), told[0].2.as_str()), (1, "✉️ 4 new messages", "space:mail"));
        assert!(told[0].1.starts_with("Ana: About d\nAna: About e"));
        assert_eq!(news(&third, &merge(&third, Err("down".into()), 4)), []);
    }
}
