//! My mail through a mail connector's mapping: the unread or important messages of my inbox, one read whole, archived
//! or marked read. No mail provider has a public MCP server cockpit could map by default, so setup maps the person's.

use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::{Value, json};

use crate::mapping::{list_at, pick, text};
use crate::tools::Tools;

/// The messages the inbox lists, newest first.
const MAX_MESSAGES: usize = 30;
const MAX_SNIPPET: usize = 200;
/// A message's body as an agent's prompt carries it.
const MAX_BODY: usize = 8000;

/// The field names a mapping that gives none reads, the messages' own names.
fn fields() -> Value {
    json!({
        "messages": "messages", "id": "id", "threadId": "threadId", "from": "from", "fromName": "fromName", "subject": "subject",
        "snippet": "snippet", "url": "url", "date": "date", "unread": "unread", "body": "body"
    })
}

/// The config's mail block over the default field names; `urlTemplate` builds a link from `{id}` and `{threadId}` when
/// the answer has none.
pub fn mapping(section: &Value) -> Value {
    let mut merged = json!({"tools": {}, "fields": fields(), "urlTemplate": ""});
    if let Some(tools) = section["tools"].as_object() {
        merged["tools"] = Value::Object(tools.clone());
    }
    if let (Some(base), Some(over)) = (merged["fields"].as_object_mut(), section["fields"].as_object()) {
        base.extend(over.clone());
    }
    if section["urlTemplate"].is_string() {
        merged["urlTemplate"] = section["urlTemplate"].clone();
    }
    merged
}

/// My unread or important messages, newest first, at most thirty, as `{id, threadId, from, fromName, subject, snippet,
/// url, date, unread}`.
pub async fn inbox(tools: &impl Tools, mapping: &Value) -> Result<Vec<Value>, String> {
    let answer = tools.call("listInbox", &json!({})).await?;
    let mut messages: Vec<Value> = list_at(&answer, &field(mapping, "messages")).into_iter().filter_map(|raw| message(raw, mapping)).collect();
    messages.sort_by_key(|m| std::cmp::Reverse(stamp(&m["date"])));
    messages.truncate(MAX_MESSAGES);
    Ok(messages)
}

/// One message read whole: the inbox's fields and its `body`.
pub async fn get(tools: &impl Tools, mapping: &Value, message: &Value) -> Result<Value, String> {
    let answer = tools.call("get", &values(message)).await?;
    let raw = list_at(&answer, &field(mapping, "messages")).into_iter().next().unwrap_or(&answer);
    let mut read = self::message(raw, mapping).unwrap_or_else(|| message.clone());
    let body = text(pick(raw, &field(mapping, "body"))).or_else(|| text(pick(raw, &field(mapping, "snippet")))).unwrap_or_default();
    read["body"] = body.chars().take(MAX_BODY).collect::<String>().into();
    Ok(read)
}

/// Archives a message, out of the inbox.
pub async fn archive(tools: &impl Tools, message: &Value) -> Result<(), String> {
    tools.call("archive", &values(message)).await.map(|_| ())
}

pub async fn mark_read(tools: &impl Tools, message: &Value) -> Result<(), String> {
    tools.call("markRead", &values(message)).await.map(|_| ())
}

fn field(mapping: &Value, name: &str) -> String {
    mapping["fields"][name].as_str().unwrap_or_default().to_string()
}

/// A tool's placeholders for a message: `{message}` its id, `{thread}` its thread's.
fn values(message: &Value) -> Value {
    let id = message["id"].as_str().or(message.as_str()).unwrap_or_default();
    let mut values = json!({"message": id});
    if let Some(thread) = message["threadId"].as_str().filter(|t| !t.is_empty()) {
        values["thread"] = thread.into();
    }
    values
}

/// A listed message in the inbox's shape; None without an id.
fn message(raw: &Value, mapping: &Value) -> Option<Value> {
    let get = |name: &str| text(pick(raw, &field(mapping, name)));
    let id = get("id").filter(|id| !id.is_empty())?;
    let thread = get("threadId").unwrap_or_default();
    let (address, named) = sender(&get("from").unwrap_or_default());
    let from_name = get("fromName").filter(|n| !n.is_empty()).or(named).unwrap_or_else(|| address.clone());
    let template = mapping["urlTemplate"].as_str().unwrap_or_default();
    let url = get("url").filter(|u| !u.is_empty()).unwrap_or_else(|| template.replace("{id}", &id).replace("{threadId}", &thread));
    let snippet = get("snippet").unwrap_or_default().split_whitespace().collect::<Vec<_>>().join(" ");
    let unread_field = field(mapping, "unread");
    let unread = unread_field.is_empty() || pick(raw, &unread_field).is_some_and(is_unread);
    let date = pick(raw, &field(mapping, "date")).map(|d| iso(d).unwrap_or_else(|| text(Some(d)).unwrap_or_default())).unwrap_or_default();
    Some(json!({
        "id": id, "threadId": thread, "from": address, "fromName": from_name, "subject": get("subject").unwrap_or_default(),
        "snippet": snippet.chars().take(MAX_SNIPPET).collect::<String>(), "url": url, "date": date, "unread": unread,
    }))
}

/// "Ana Lima <ana@acme.com>" as its address and name; a bare address has no name.
fn sender(from: &str) -> (String, Option<String>) {
    match (from.rfind('<'), from.rfind('>')) {
        (Some(open), Some(close)) if open < close => {
            let name = from[..open].trim().trim_matches('"').trim();
            (from[open + 1..close].trim().to_string(), (!name.is_empty()).then(|| name.to_string()))
        }
        _ => (from.trim().to_string(), None),
    }
}

/// Whether a message is unread: true, "unread", 1, or a label list holding UNREAD (Gmail's `labelIds`).
fn is_unread(value: &Value) -> bool {
    match value {
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|n| n != 0.0),
        Value::String(s) => matches!(s.to_lowercase().as_str(), "true" | "unread" | "yes" | "1"),
        Value::Array(labels) => labels.iter().any(|l| l.as_str().is_some_and(|l| l.eq_ignore_ascii_case("unread"))),
        _ => false,
    }
}

/// A date as RFC 3339 in UTC: from RFC 3339, RFC 2822 (a mail's Date header), or epoch seconds or milliseconds.
fn iso(date: &Value) -> Option<String> {
    let at: DateTime<Utc> = match date {
        Value::Number(n) => epoch(n.as_f64()?)?,
        Value::String(s) if s.chars().all(|c| c.is_ascii_digit()) && !s.is_empty() => epoch(s.parse().ok()?)?,
        Value::String(s) => DateTime::parse_from_rfc3339(s).or_else(|_| DateTime::parse_from_rfc2822(s)).ok()?.with_timezone(&Utc),
        _ => return None,
    };
    Some(at.to_rfc3339_opts(SecondsFormat::Secs, true))
}

fn epoch(n: f64) -> Option<DateTime<Utc>> {
    let ms = if n > 1e11 { n } else { n * 1000.0 };
    DateTime::from_timestamp_millis(ms as i64)
}

/// A message's date in milliseconds, for the order; an unreadable one sorts last.
fn stamp(date: &Value) -> i64 {
    date.as_str().and_then(|d| DateTime::parse_from_rfc3339(d).ok()).map_or(i64::MIN, |d| d.timestamp_millis())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::Fake;

    #[test]
    fn the_inbox_maps_sorts_and_cuts() {
        let section = json!({
            "tools": {"listInbox": {"tool": "search_threads", "args": {"query": "is:unread"}}},
            "fields": {"messages": "threads", "id": "messageId", "from": "sender", "subject": "headers.subject", "date": "internalDate", "unread": "labelIds"},
            "urlTemplate": "https://mail.example.com/#all/{threadId}"
        });
        let m = mapping(&section);
        let long = "word ".repeat(80);
        let answer = json!({"threads": [
            {"messageId": "a", "threadId": "t1", "sender": "Ana Lima <ana@acme.com>", "headers": {"subject": "Old"}, "internalDate": "1762000000000", "labelIds": ["INBOX"], "snippet": long},
            {"messageId": "b", "threadId": "t2", "sender": "bob@acme.com", "headers": {"subject": "New"}, "internalDate": 1762100000, "labelIds": ["UNREAD", "INBOX"]},
            {"threadId": "t3", "sender": "no id", "internalDate": 1762200000},
            {"messageId": "c", "sender": "\"Cy\" <cy@acme.com>", "internalDate": "Wed, 05 Nov 2025 10:00:00 +0100", "url": "https://x/c"}
        ]});
        let fake = Fake::default().with("listInbox", answer);
        let messages = crate::block_on(inbox(&fake, &m)).unwrap();
        let ids: Vec<&str> = messages.iter().map(|m| m["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["c", "b", "a"]);
        assert_eq!((&messages[2]["from"], &messages[2]["fromName"]), (&json!("ana@acme.com"), &json!("Ana Lima")));
        assert_eq!((&messages[1]["fromName"], &messages[1]["unread"]), (&json!("bob@acme.com"), &json!(true)));
        assert_eq!(messages[2]["unread"], false);
        assert_eq!(messages[1]["url"], "https://mail.example.com/#all/t2");
        assert_eq!(messages[0]["url"], "https://x/c");
        assert_eq!(messages[0]["fromName"], "Cy");
        assert_eq!(messages[1]["date"], "2025-11-02T16:13:20Z");
        assert_eq!(messages[0]["date"], "2025-11-05T09:00:00Z");
        assert_eq!(messages[2]["snippet"].as_str().unwrap().chars().count(), 200);

        let many: Vec<Value> = (0..40).map(|n| json!({"id": format!("m{n}"), "date": 1762000000 + n})).collect();
        let fake = Fake::default().with("listInbox", json!({"messages": many}));
        let messages = crate::block_on(inbox(&fake, &mapping(&json!({})))).unwrap();
        assert_eq!(messages.len(), 30);
        assert_eq!(messages[0]["id"], "m39");
        assert_eq!(messages[0]["unread"], false, "an unread field that is mapped but absent");
        let unmapped = mapping(&json!({"fields": {"unread": ""}}));
        let fake = Fake::default().with("listInbox", json!({"messages": [{"id": "x"}]}));
        assert_eq!(crate::block_on(inbox(&fake, &unmapped)).unwrap()[0]["unread"], true, "no unread field: the listing is of unread messages");
    }

    #[test]
    fn a_message_is_archived_marked_read_and_read_whole() {
        let fake = Fake::default().with("archive", json!({})).with("markRead", json!({}));
        crate::block_on(archive(&fake, &json!({"id": "m1", "threadId": "t1"}))).unwrap();
        crate::block_on(mark_read(&fake, &json!({"id": "m2"}))).unwrap();
        assert_eq!(fake.calls.borrow()[0], ("archive".to_string(), json!({"message": "m1", "thread": "t1"})));
        assert_eq!(fake.calls.borrow()[1], ("markRead".to_string(), json!({"message": "m2"})));

        let fake = Fake::default().with("get", json!({"id": "m1", "from": "Ana <ana@acme.com>", "subject": "Lunch?", "body": "Shall we?"}));
        let read = crate::block_on(get(&fake, &mapping(&json!({})), &json!({"id": "m1"}))).unwrap();
        assert_eq!((&read["fromName"], &read["subject"], &read["body"]), (&json!("Ana"), &json!("Lunch?"), &json!("Shall we?")));
        assert!(crate::block_on(archive(&Fake::default(), &json!({"id": "m1"}))).is_err(), "no tool mapped");
    }
}
