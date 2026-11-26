//! My mail through a mail connector's mapping: the unread or important messages of my inbox, one read whole with its
//! thread, archived or marked read, and which of them need me. No mail provider has a public MCP server cockpit could
//! map by default, so setup maps the person's.

use std::collections::HashSet;

use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::{Map, Value, json};

use crate::mapping::{list_at, pick, text};
use crate::tools::Tools;

/// The messages the inbox lists, newest first.
const MAX_MESSAGES: usize = 50;
const MAX_SNIPPET: usize = 200;
/// A message's body as an agent's prompt carries it.
const MAX_BODY: usize = 8000;
/// A thread message's HTML, past which it is cut.
const MAX_HTML: usize = 400_000;
/// How many undated threads one fetch peeks for a date, unread ones and the rest, each once ever.
pub const PEEK_UNREAD: usize = 15;
pub const PEEK_OTHERS: usize = 15;
/// How many threads' dates are kept.
const KEEP_DATES: usize = 300;

/// The field names a mapping that gives none reads: the messages' own names, then Gmail's in either case.
fn fields() -> Value {
    json!({
        "messages": "messages", "id": "id", "threadId": "threadId", "from": "from|sender", "fromName": "fromName", "subject": "subject",
        "snippet": "snippet", "url": "url|viewUrl|view_url", "date": "date", "unread": "unread", "labels": "labels|labelIds|label_ids",
        "body": "body|text|plaintextBody|plaintext_body", "html": "html|htmlBody|html_body", "thread": "messages"
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

/// My inbox, newest first, at most fifty, as `{id, threadId, from, fromName, subject, snippet, url, date, unread,
/// labels}`, with the dates kept by thread: a message is unread when `listUnread` lists its thread, and a listing with
/// no dates gets them from a `peek` of its newest threads, each once, the undated ones listed after the dated.
pub async fn inbox(tools: &impl Tools, mapping: &Value, dates: &Value) -> Result<(Vec<Value>, Value), String> {
    let listing = field(mapping, "messages");
    let answer = tools.call("listInbox", &json!({})).await?;
    let unread = if tools.has("listUnread") { Some(tools.call("listUnread", &json!({})).await?) } else { None };
    let unread_threads: Option<HashSet<String>> =
        unread.as_ref().map(|a| list_at(a, &listing).into_iter().filter_map(|raw| message(raw, mapping, None)).map(|m| thread_key(&m)).collect());
    let mut messages: Vec<Value> = Vec::new();
    let mut seen = HashSet::new();
    for raw in unread.iter().flat_map(|a| list_at(a, &listing)).chain(list_at(&answer, &listing)) {
        if let Some(m) = message(raw, mapping, unread_threads.as_ref())
            && seen.insert(m["id"].clone())
        {
            messages.push(m);
        }
    }
    let mut dates: Map<String, Value> = dates.as_object().cloned().unwrap_or_default();
    if tools.has("peek") {
        let (mut unread_left, mut others_left) = (PEEK_UNREAD, PEEK_OTHERS);
        for m in &messages {
            let key = thread_key(m);
            let left = if m["unread"] == true { &mut unread_left } else { &mut others_left };
            if m["date"] != "" || dates.contains_key(&key) || *left == 0 {
                continue;
            }
            *left -= 1;
            let date = match tools.call("peek", &values(m)).await {
                Ok(answer) => thread_of(&answer, mapping).last().and_then(|t| t["date"].as_str()).unwrap_or_default().to_string(),
                Err(_) => String::new(),
            };
            dates.insert(key, date.into());
        }
    }
    let keys: HashSet<String> = messages.iter().map(thread_key).collect();
    dates.retain(|key, _| keys.contains(key));
    for key in dates.keys().skip(KEEP_DATES).cloned().collect::<Vec<_>>() {
        dates.remove(&key);
    }
    for m in &mut messages {
        if m["date"] == ""
            && let Some(date) = dates.get(&thread_key(m)).and_then(Value::as_str).filter(|d| !d.is_empty())
        {
            m["date"] = date.into();
        }
    }
    messages.sort_by_key(|m| std::cmp::Reverse(stamp(&m["date"])));
    messages.truncate(MAX_MESSAGES);
    Ok((messages, Value::Object(dates)))
}

/// The key a message's thread goes by in the dates and the unread set: its thread id, else its own.
fn thread_key(message: &Value) -> String {
    message["threadId"].as_str().filter(|t| !t.is_empty()).or(message["id"].as_str()).unwrap_or_default().to_string()
}

/// One message read whole: the inbox's fields and its `body`.
pub async fn get(tools: &impl Tools, mapping: &Value, message: &Value) -> Result<Value, String> {
    let answer = tools.call("get", &values(message)).await?;
    let raw = list_at(&answer, &field(mapping, "messages")).into_iter().next().unwrap_or(&answer);
    let mut read = self::message(raw, mapping, None).unwrap_or_else(|| message.clone());
    let body = text(pick(raw, &field(mapping, "body"))).or_else(|| text(pick(raw, &field(mapping, "snippet")))).unwrap_or_default();
    read["body"] = body.chars().take(MAX_BODY).collect::<String>().into();
    Ok(read)
}

/// A message's whole thread, oldest first, as `{id, from, fromName, date, html, text}` (`id` each message's own); `get` answers with the thread's
/// messages at the `thread` field, else with the one message.
pub async fn thread(tools: &impl Tools, mapping: &Value, message: &Value) -> Result<Vec<Value>, String> {
    let answer = tools.call("get", &values(message)).await?;
    Ok(thread_of(&answer, mapping))
}

/// The thread in a `get` answer.
pub fn thread_of(answer: &Value, mapping: &Value) -> Vec<Value> {
    let listed = list_at(answer, &field(mapping, "thread"));
    let raws = if listed.is_empty() { vec![answer] } else { listed };
    let mut messages: Vec<Value> = raws
        .into_iter()
        .map(|raw| {
            let get = |name: &str| text(pick(raw, &field(mapping, name))).unwrap_or_default();
            let (address, named) = sender(&get("from"));
            let from_name = Some(get("fromName")).filter(|n| !n.is_empty()).or(named).unwrap_or_else(|| address.clone());
            let date = pick(raw, &field(mapping, "date")).map(|d| iso(d).unwrap_or_else(|| text(Some(d)).unwrap_or_default())).unwrap_or_default();
            let html: String = get("html").chars().take(MAX_HTML).collect();
            json!({"id": text(raw.get("id")).unwrap_or_default(), "from": address, "fromName": from_name, "date": date, "html": html, "text": get("body")})
        })
        .filter(|m| m["html"] != "" || m["text"] != "" || m["from"] != "")
        .collect();
    messages.sort_by_key(|m| stamp(&m["date"]));
    messages
}

/// The sidebar's line for the mail document: how many messages are unread; None before any fetch.
pub fn row_text(doc: &Value) -> Option<String> {
    let messages = doc["messages"].as_array()?;
    let unread = messages.iter().filter(|m| m["unread"] == true).count();
    Some(match (unread, messages.len()) {
        (0, 0) => "\u{f42f}  inbox zero".to_string(),
        (0, _) => "\u{f42f}  nothing unread".to_string(),
        (n, _) => format!("\u{f42f}  {n} unread"),
    })
}

/// The file name a message's cached thread goes by, safe whatever its id holds.
pub fn cache_name(id: &str) -> String {
    id.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '_' }).collect::<String>().trim_start_matches('.').to_string()
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

/// A listed message in the inbox's shape; None without an id. With an unread listing, a message is unread when it
/// holds the thread or a mapped unread field says so; without one, a mapping with no unread field lists unread mail only.
fn message(raw: &Value, mapping: &Value, unread_threads: Option<&HashSet<String>>) -> Option<Value> {
    let get = |name: &str| text(pick(raw, &field(mapping, name)));
    let id = get("id").filter(|id| !id.is_empty())?;
    let thread = get("threadId").unwrap_or_default();
    let (address, named) = sender(&get("from").unwrap_or_default());
    let from_name = get("fromName").filter(|n| !n.is_empty()).or(named).unwrap_or_else(|| address.clone());
    let template = mapping["urlTemplate"].as_str().unwrap_or_default();
    let url = get("url").filter(|u| !u.is_empty()).unwrap_or_else(|| template.replace("{id}", &id).replace("{threadId}", &thread));
    let snippet = unescape(&get("snippet").unwrap_or_default()).split_whitespace().collect::<Vec<_>>().join(" ");
    let unread_field = field(mapping, "unread");
    let flagged = !unread_field.is_empty() && pick(raw, &unread_field).is_some_and(is_unread);
    let key = if thread.is_empty() { id.clone() } else { thread.clone() };
    let unread = match unread_threads {
        Some(threads) => threads.contains(&key) || flagged,
        None => unread_field.is_empty() || flagged,
    };
    let date = pick(raw, &field(mapping, "date")).map(|d| iso(d).unwrap_or_else(|| text(Some(d)).unwrap_or_default())).unwrap_or_default();
    let labels: Vec<Value> = pick(raw, &field(mapping, "labels")).and_then(Value::as_array).into_iter().flatten().filter(|l| l.is_string()).cloned().collect();
    Some(json!({
        "id": id, "threadId": thread, "from": address, "fromName": from_name, "subject": unescape(&get("subject").unwrap_or_default()),
        "snippet": snippet.chars().take(MAX_SNIPPET).collect::<String>(), "url": url, "date": date, "unread": unread, "labels": labels,
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
/// A snippet or subject as the mail server escapes it for HTML (`haven&#39;t`, `R&amp;D`), as plain text.
pub fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let decoded = rest.find(';').filter(|end| *end <= 10).and_then(|end| {
            let name = &rest[1..end];
            let c = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                _ => name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")).map_or_else(
                    || name.strip_prefix('#').and_then(|n| n.parse().ok()).and_then(char::from_u32),
                    |hex| u32::from_str_radix(hex, 16).ok().and_then(char::from_u32),
                ),
            }?;
            Some((c, end + 1))
        });
        match decoded {
            Some((c, len)) => {
                out.push(c);
                rest = &rest[len..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

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
        let (messages, dates) = crate::block_on(inbox(&fake, &m, &Value::Null)).unwrap();
        assert_eq!(dates, json!({}), "no peek: no dates kept");
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

        let many: Vec<Value> = (0..60).map(|n| json!({"id": format!("m{n}"), "date": 1762000000 + n})).collect();
        let fake = Fake::default().with("listInbox", json!({"messages": many}));
        let (messages, _) = crate::block_on(inbox(&fake, &mapping(&json!({})), &Value::Null)).unwrap();
        assert_eq!(messages.len(), 50);
        assert_eq!(messages[0]["id"], "m59");
        assert_eq!(messages[0]["unread"], false, "an unread field that is mapped but absent");
        let unmapped = mapping(&json!({"fields": {"unread": ""}}));
        let fake = Fake::default().with("listInbox", json!({"messages": [{"id": "x"}]}));
        assert_eq!(crate::block_on(inbox(&fake, &unmapped, &Value::Null)).unwrap().0[0]["unread"], true, "no unread field: the listing is of unread messages");
    }

    #[test]
    fn a_summary_listing_is_unread_by_the_unread_listing_and_dated_by_a_peek() {
        // Gmail's search_threads in the summary view: id, sender, subject and threadId only, no labels nor dates.
        let summary = |thread: &str, sender: &str, subject: &str| json!({"id": thread, "messages": [{"id": format!("m-{thread}"), "threadId": thread, "sender": sender, "subject": subject}]});
        let inbox_answer = json!({"threads": [summary("t1", "Priya <priya@example.com>", "Roadmap"), summary("t2", "Maya <maya@example.com>", "Lunch?"), summary("t3", "Sam <sam@example.com>", "Retro")]});
        let unread_answer = json!({"threads": [summary("t2", "Maya <maya@example.com>", "Lunch?")]});
        let m = mapping(&json!({"fields": {"messages": "threads[].messages", "id": "threadId", "unread": "labelIds"}}));
        let peeked = json!({"id": "t1", "messages": [
            {"id": "a", "sender": "Priya <priya@example.com>", "date": "2026-11-03T09:00:00Z"},
            {"id": "b", "sender": "me@acme.example", "date": "2026-11-05T09:00:00Z"}
        ]});
        let fake = Fake::default()
            .with("listInbox", inbox_answer.clone())
            .with("listUnread", unread_answer.clone())
            .with("peek", json!({"id": "t2", "messages": []}))
            .with("peek", peeked);
        let (messages, dates) = crate::block_on(inbox(&fake, &m, &json!({"t3": "", "gone": "2026-11-01T09:00:00Z"}))).unwrap();
        let listed: Vec<(&str, bool)> = messages.iter().map(|m| (m["id"].as_str().unwrap(), m["unread"] == true)).collect();
        assert_eq!(listed, [("t1", false), ("t2", true), ("t3", false)], "dated first, then the undated unread first");
        assert_eq!(messages[0]["date"], "2026-11-05T09:00:00Z", "a peek dates a thread by its newest message");
        assert_eq!(dates, json!({"t1": "2026-11-05T09:00:00Z", "t2": "", "t3": ""}), "a thread peeked without a date is kept, a thread gone dropped");
        let peeks: Vec<Value> = fake.calls.borrow().iter().filter(|(v, _)| v == "peek").map(|(_, a)| a.clone()).collect();
        assert_eq!(
            peeks,
            [json!({"message": "t2", "thread": "t2"}), json!({"message": "t1", "thread": "t1"})],
            "the unread thread is peeked first, t3 never again"
        );

        let fake = Fake::default().with("listInbox", inbox_answer.clone()).with("listUnread", unread_answer).with("peek", json!({}));
        let (messages, again) = crate::block_on(inbox(&fake, &m, &dates)).unwrap();
        assert_eq!(fake.calls.borrow().iter().filter(|(v, _)| v == "peek").count(), 0, "every listed thread is dated or peeked already");
        assert_eq!((again, messages[0]["date"].as_str()), (dates, Some("2026-11-05T09:00:00Z")));

        let fake = Fake::default().with("listInbox", inbox_answer).with("listUnread", json!({"threads": []}));
        let (messages, _) = crate::block_on(inbox(&fake, &m, &Value::Null)).unwrap();
        assert!(messages.iter().all(|m| m["unread"] == false), "an empty unread listing: nothing unread, whatever the unread field");
        assert_eq!(messages.iter().map(|m| m["id"].as_str().unwrap()).collect::<Vec<_>>(), ["t1", "t2", "t3"], "no peek: the inbox's order");
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

    #[test]
    fn a_gmail_thread_maps_in_either_case_oldest_first() {
        // Gmail's get_thread with FULL_CONTENT, camelCase as the connector answers and snake_case as it documents.
        let answer = json!({"id": "t1", "messages": [
            {"id": "m2", "sender": "Maya <maya@example.com>", "date": "2026-11-04T10:00:00Z", "htmlBody": "<p>Yes</p>", "plaintextBody": "Yes"},
            {"id": "m1", "sender": "me@acme.example", "date": "2026-11-03T09:00:00Z", "plaintext_body": "Lunch?", "html_body": ""}
        ]});
        let fake = Fake::default().with("get", answer);
        let thread = crate::block_on(thread(&fake, &mapping(&json!({})), &json!({"id": "t1", "threadId": "t1"}))).unwrap();
        assert_eq!(fake.calls.borrow()[0].1, json!({"message": "t1", "thread": "t1"}));
        assert_eq!(thread.len(), 2);
        assert_eq!((&thread[0]["id"], &thread[0]["text"], &thread[0]["fromName"]), (&json!("m1"), &json!("Lunch?"), &json!("me@acme.example")));
        assert_eq!((&thread[1]["fromName"], &thread[1]["from"], &thread[1]["html"]), (&json!("Maya"), &json!("maya@example.com"), &json!("<p>Yes</p>")));
        let single = thread_of(&json!({"from": "Sam <sam@example.com>", "body": "Hi"}), &mapping(&json!({"fields": {"thread": "items"}})));
        assert_eq!((single.len(), &single[0]["text"]), (1, &json!("Hi")));
    }

    #[test]
    fn escaped_snippets_read_as_text() {
        assert_eq!(unescape("haven&#39;t &amp; R&amp;D &lt;3 &#x2019; &nbsp;5 & co &bogus;"), "haven't & R&D <3 \u{2019}  5 & co &bogus;");
    }

    #[test]
    fn the_sidebar_counts_the_unread_mail() {
        let doc = json!({"messages": [{"id": "a", "unread": true}, {"id": "b", "unread": false}]});
        assert_eq!(row_text(&doc).unwrap(), "\u{f42f}  1 unread");
        assert_eq!(row_text(&json!({"messages": []})).unwrap(), "\u{f42f}  inbox zero");
        assert_eq!(row_text(&Value::Null), None);
        assert_eq!(cache_name("../a/b:c"), "_a_b_c");
    }

    #[test]
    fn listed_messages_keep_their_labels() {
        let fake = Fake::default().with("listInbox", json!({"messages": [{"id": "a", "labelIds": ["INBOX", "IMPORTANT", 3]}]}));
        let (messages, _) = crate::block_on(inbox(&fake, &mapping(&json!({})), &Value::Null)).unwrap();
        assert_eq!(messages[0]["labels"], json!(["INBOX", "IMPORTANT"]));
    }
}
