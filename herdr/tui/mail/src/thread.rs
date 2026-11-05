//! A message's thread read whole, as the hub or a Claude Code session keeps it in ~/.cache/cockpit/mail/<id>.json
//! (`{at, id, messages, error?}`, with an `<id>.asked` mark while a session is asked for it), and the page the side
//! browser shows of it.

use std::path::PathBuf;

use cockpit_connectors::mail::cache_name;
use cockpit_core::files;
use serde_json::Value;

use crate::html;

/// Where the thread of a message stands.
#[derive(Clone, Debug, PartialEq)]
pub enum Thread {
    /// Nobody asked for it yet.
    Unread,
    /// Asked of a Claude Code session at this time (epoch milliseconds), not there yet.
    Loading(u64),
    Ready(Vec<Read>),
    Failed(String),
}

/// One message of a thread.
#[derive(Clone, Debug, PartialEq)]
pub struct Read {
    pub from: String,
    pub name: String,
    pub date: String,
    /// The message's HTML, sanitized; empty for a plain text message.
    pub html: String,
    /// Its HTML for a browser page, sanitized with its layout kept.
    pub page_html: String,
    /// Its plain text as the mail gives it; empty when it has only HTML.
    pub text: String,
}

pub fn dir() -> PathBuf {
    files::cache().join("mail")
}

pub fn path(id: &str) -> PathBuf {
    dir().join(format!("{}.json", cache_name(id)))
}

/// The page the side browser shows.
pub fn page_path(id: &str) -> PathBuf {
    path(id).with_extension("html")
}

/// The thread as kept on disk now.
pub fn read(id: &str) -> Thread {
    let path = path(id);
    match files::load(&path) {
        Some(doc) => parse(&doc),
        None => match std::fs::read_to_string(path.with_extension("asked")).ok().and_then(|t| t.trim().parse().ok()) {
            Some(at) => Thread::Loading(at),
            None => Thread::Unread,
        },
    }
}

pub fn parse(doc: &Value) -> Thread {
    if let Some(error) = doc["error"].as_str() {
        return Thread::Failed(error.to_string());
    }
    let s = |m: &Value, key: &str| m[key].as_str().unwrap_or_default().to_string();
    let messages = doc["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|m| {
            let raw_html = s(m, "html");
            Read {
                from: s(m, "from"),
                name: Some(s(m, "fromName")).filter(|n| !n.is_empty()).unwrap_or_else(|| s(m, "from")),
                date: s(m, "date"),
                html: if raw_html.trim().is_empty() { String::new() } else { html::sanitize(&raw_html) },
                page_html: if raw_html.trim().is_empty() { String::new() } else { html::sanitize_page(&raw_html) },
                text: s(m, "text"),
            }
        })
        .collect();
    Thread::Ready(messages)
}

impl Read {
    /// The message as text, its HTML's when it has no plain text.
    pub fn body(&self, width: usize) -> String {
        html::text_of(&self.text, &self.html, width)
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The thread as one HTML page: the subject, then each message under its sender and date.
pub fn page(subject: &str, messages: &[Read]) -> String {
    let mut out = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{0}</title><style>body{{font:15px/1.5 -apple-system,system-ui,sans-serif;\
         max-width:820px;margin:24px auto;padding:0 16px}}header{{color:#666;border-top:1px solid #ddd;padding-top:12px;margin-top:24px}}\
         pre{{white-space:pre-wrap;font:inherit}}</style></head><body><h2>{0}</h2>",
        escape(subject)
    );
    for m in messages {
        out.push_str(&format!("<header><b>{}</b> &lt;{}&gt; · {}</header>", escape(&m.name), escape(&m.from), escape(&m.date)));
        if m.html.is_empty() {
            out.push_str(&format!("<pre>{}</pre>", escape(&m.text)));
        } else {
            out.push_str(&format!("<div>{}</div>", m.page_html));
        }
    }
    out.push_str("</body></html>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_kept_thread_reads_sanitized_and_as_a_page() {
        let doc = json!({"at": 1, "id": "t1", "messages": [
            {"from": "maya@example.com", "fromName": "Maya", "date": "2026-11-04T10:00:00Z", "html": "<p onclick=\"x()\">Noon <script>bad()</script>works</p>", "text": ""},
            {"from": "me@acme.example", "fromName": "", "date": "2026-11-04T11:00:00Z", "html": "", "text": "See you <there>"}
        ]});
        let Thread::Ready(messages) = parse(&doc) else { panic!("not ready") };
        assert_eq!(messages[0].html, "<p>Noon works</p>");
        assert_eq!(messages[0].body(80), "Noon works");
        assert_eq!((messages[1].name.as_str(), messages[1].body(80).as_str()), ("me@acme.example", "See you <there>"));
        let page = page("Lunch <today>", &messages);
        assert!(page.contains("<title>Lunch &lt;today&gt;</title>") && page.contains("<p>Noon works</p>") && page.contains("<pre>See you &lt;there&gt;</pre>"));
        assert_eq!(parse(&json!({"error": "Gmail: no thread"})), Thread::Failed("Gmail: no thread".into()));
    }
}
