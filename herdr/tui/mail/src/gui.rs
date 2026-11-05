//! The mail for the Herdr app's cockpit mod: the `mail` view (the messages grouped "Unread" above "Read", each with
//! its age and actions) and the `mail-thread` view (one message's thread, its HTML sanitized, for the reading panel).

use std::path::PathBuf;

use chrono::{DateTime, Local, Utc};
use cockpit_core::files;
use serde_json::{Value, json};

use crate::model::{GROUPS, Group, Mail, Message, age};
use crate::thread::{self, Thread};

/// The folder of the kept threads: one arriving rebuilds `mail-thread`.
pub fn dirs() -> Vec<PathBuf> {
    vec![thread::dir()]
}

pub fn view(_args: &Value, doc: &dyn Fn(&str) -> Value) -> Value {
    let data = doc("mail");
    let data = if data.is_null() { files::load(&files::cache().join("mail.json")) } else { Some(data) };
    model(data.as_ref(), &files::config(), Utc::now())
}

/// Whether the mail mapping has a tool for this verb (`archive`, `markRead`).
fn maps(config: &Value, verb: &str) -> bool {
    let tools = &config["mail"]["tools"];
    tools[verb]["tool"].as_str().is_some_and(|t| !t.is_empty()) || tools.as_array().is_some_and(|t| t.iter().any(|t| t == verb))
}

/// A message's actions in order, the first the main one: see, reply, archive, mark read, open in the mail client.
pub fn acts(m: &Message, config: &Value) -> Vec<Value> {
    let mut acts =
        vec![json!({"id": "see", "label": "Open", "key": "enter", "main": true}), json!({"id": "reply", "label": "Reply", "key": "c", "main": false})];
    if maps(config, "archive") {
        acts.push(json!({"id": "archive", "label": "Archive", "key": "b", "main": false}));
    }
    if m.unread && maps(config, "markRead") {
        acts.push(json!({"id": "read", "label": "Mark read", "key": "m", "main": false}));
    }
    acts.push(json!({"id": "page", "label": "Web page", "key": "h", "main": false}));
    if !m.url.is_empty() {
        acts.push(json!({"id": "open", "label": "Open in mail", "key": "o", "main": false}));
    }
    acts
}

pub fn model(data: Option<&Value>, config: &Value, now: DateTime<Utc>) -> Value {
    let enabled = config["mail"]["enabled"] == true;
    let mail = Mail::parse(data);
    let updated = mail.at.and_then(DateTime::from_timestamp_millis).map(|at| at.with_timezone(&Local).format("%H:%M").to_string());
    let ids = |group: Group| mail.messages.iter().filter(|m| m.group() == group).map(|m| m.id.clone()).collect::<Vec<_>>();
    let messages: Vec<Value> = mail
        .messages
        .iter()
        .map(|m| {
            json!({
                "id": m.id, "from": m.from, "fromName": m.name, "subject": m.title(), "snippet": m.snippet, "url": m.url,
                "date": m.raw["date"], "age": age(m.date, now), "unread": m.unread, "raw": m.raw, "acts": acts(m, config),
            })
        })
        .collect();
    json!({
        "waiting": data.is_none_or(|d| !d.is_object()),
        "enabled": enabled,
        "updated": updated,
        "error": mail.error,
        "counts": {"unread": mail.unread(), "read": ids(Group::Read).len(), "total": mail.messages.len()},
        "groups": GROUPS.iter().map(|(group, key, label)| json!({"key": key, "label": label, "ids": ids(*group)})).collect::<Vec<_>>(),
        "messages": messages,
    })
}

/// `mail-thread {id}`: that message's thread as the reading panel draws it.
pub fn thread_view(args: &Value, _doc: &dyn Fn(&str) -> Value) -> Value {
    thread_json(&thread::read(args["id"].as_str().unwrap_or_default()))
}

pub fn thread_json(thread: &Thread) -> Value {
    match thread {
        Thread::Unread => json!({"state": "unread", "messages": []}),
        Thread::Loading(at) => json!({"state": "loading", "since": at, "messages": []}),
        Thread::Failed(error) => json!({"state": "failed", "error": error, "messages": []}),
        Thread::Ready(messages) => {
            let messages: Vec<Value> = messages
                .iter()
                .map(|m| {
                    let when =
                        DateTime::parse_from_rfc3339(&m.date).map(|d| d.with_timezone(&Local).format("%a %-d %b %H:%M").to_string()).unwrap_or(m.date.clone());
                    json!({"from": m.from, "fromName": m.name, "date": m.date, "when": when, "html": crate::html::for_view(&m.html), "text": m.body(100)})
                })
                .collect();
            json!({"state": "ready", "messages": messages})
        }
    }
}

/// The mod's own actions: none, the hub runs `mail_thread`, `mail_open`, `mail_archive`, `mail_read` and `mail_reply`.
/// `mail_page [message]`: the thread as a web page in the default browser, its layout and images as the sender made
/// them.
pub fn act(action: &str, args: &[Value]) -> Option<Result<String, String>> {
    (action == "mail_page").then(|| web_page(args.first().unwrap_or(&Value::Null)))
}

fn web_page(message: &Value) -> Result<String, String> {
    let id = message["id"].as_str().filter(|id| !id.is_empty()).ok_or("a message needs its id")?;
    let Thread::Ready(messages) = thread::read(id) else { return Err("the mail is still loading".into()) };
    let subject = message["subject"].as_str().unwrap_or_default();
    let path = thread::page_path(id);
    std::fs::write(&path, thread::page(subject, &messages)).map_err(|e| e.to_string())?;
    files::open_url(&format!("file://{}", path.display()));
    Ok(format!("opened {subject} in the browser"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_as_the_mod_draws_it() {
        let data = json!({"at": 1_762_250_400_000_i64, "messages": [
            {"id": "a", "from": "noreply@ci.example.com", "fromName": "CI", "subject": "Build passed", "date": "2026-11-04T10:00:00Z", "unread": true, "url": "https://mail.example.com/a"},
            {"id": "b", "from": "maya@example.com", "fromName": "Maya", "subject": "Lunch?", "date": "2026-11-04T08:00:00Z", "unread": true}
        ]});
        let config = json!({"mail": {"enabled": true, "tools": {"archive": {"tool": "archive_thread"}, "markRead": {"tool": "read_thread"}}}});
        let now = DateTime::parse_from_rfc3339("2026-11-04T11:00:00Z").unwrap().with_timezone(&Utc);
        let view = model(Some(&data), &config, now);
        assert_eq!(view["groups"][0]["ids"], json!(["a", "b"]));
        assert_eq!(view["groups"][1]["ids"], json!([]));
        assert_eq!(view["counts"], json!({"unread": 2, "read": 0, "total": 2}));
        let ids = |m: &Value| m["acts"].as_array().unwrap().iter().map(|a| a["id"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        assert_eq!(ids(&view["messages"][0]), ["see", "reply", "archive", "read", "page", "open"]);
        assert_eq!(ids(&view["messages"][1]), ["see", "reply", "archive", "read", "page"]);
        assert_eq!((&view["messages"][0]["age"], &view["messages"][1]["age"]), (&json!("1h"), &json!("3h")));
        let bare = model(None, &json!({}), now);
        assert_eq!((&bare["waiting"], &bare["enabled"]), (&json!(true), &json!(false)));
        assert_eq!(thread_json(&Thread::Loading(5))["state"], "loading");
    }
}
