//! Mail acted on from the inbox: a message opened in the mail client, archived or marked read through the hub's own
//! mail server, or answered by an agent that drafts the reply for me to review.

use cockpit_connectors::mail;
use cockpit_connectors::tools::Mapped;
use cockpit_core::files;
use serde_json::Value;

use super::{Ctx, arg, inbox};
use crate::config;
use crate::value::{Res, text};

/// The hub's mail server and the mail mapping.
fn served() -> Res<(cockpit_connectors::Server, Value)> {
    let raw = files::load(&files::config_dir().join("config.json")).unwrap_or(Value::Null);
    let server = crate::feed::mcp::server(&raw, &config::load().0, "mail").ok_or("no mail server cockpit reaches: map one with cockpit setup")?;
    Ok((server, mail::mapping(&raw["mail"])))
}

fn message(args: &[Value]) -> Res<Value> {
    let message = arg(args, 0)?;
    if message["id"].as_str().is_none_or(str::is_empty) {
        return Err("a message needs its id".into());
    }
    Ok(message.clone())
}

/// The store's `mail` document with that message changed by `f`, or left out when `f` says None.
fn change(id: &str, f: impl Fn(&Value) -> Option<Value>) {
    crate::store::global().update("mail", |doc| {
        let mut doc = doc.clone();
        if let Some(messages) = doc["messages"].as_array_mut() {
            *messages = messages.iter().filter_map(|m| if m["id"] == id { f(m) } else { Some(m.clone()) }).collect();
        }
        doc
    });
}

fn who(message: &Value) -> String {
    let name = text(&message["fromName"]);
    if name.is_empty() { text(&message["from"]) } else { name }
}

/// `mail_open [message]`: the message in the mail client's web page.
pub fn open(ctx: &mut Ctx, args: &[Value]) -> Res {
    let message = message(args)?;
    let url = text(&message["url"]);
    if url.is_empty() {
        return Err("the mail tool gives no link for this message (set mail.urlTemplate)".into());
    }
    config::open_url(&url);
    ctx.note(format!("opened {}", text(&message["subject"])));
    Ok(())
}

/// `mail_archive [message]`.
pub fn archive(ctx: &mut Ctx, args: &[Value]) -> Res {
    let message = message(args)?;
    let (server, mapping) = served()?;
    crate::feed::mcp::with(&server, async |c| mail::archive(&Mapped { connection: c, section: &mapping }, &message).await)?;
    change(&text(&message["id"]), |_| None);
    ctx.note(format!("archived {}", text(&message["subject"])));
    Ok(())
}

/// `mail_read [message]`: marked read.
pub fn read(ctx: &mut Ctx, args: &[Value]) -> Res {
    let message = message(args)?;
    let (server, mapping) = served()?;
    crate::feed::mcp::with(&server, async |c| mail::mark_read(&Mapped { connection: c, section: &mapping }, &message).await)?;
    change(&text(&message["id"]), |m| {
        let mut m = m.clone();
        m["unread"] = false.into();
        Some(m)
    });
    ctx.note(format!("marked {} read", text(&message["subject"])));
    Ok(())
}

/// `mail_reply [message]`: an agent session in the actions space drafts a reply from the whole message, and sends
/// nothing.
pub fn reply(ctx: &mut Ctx, args: &[Value]) -> Res {
    let listed = message(args)?;
    let read = match served() {
        Ok((server, mapping)) if mapping["tools"]["get"]["tool"].is_string() => {
            crate::feed::mcp::with(&server, async |c| mail::get(&Mapped { connection: c, section: &mapping }, &mapping, &listed).await).unwrap_or(listed)
        }
        _ => listed,
    };
    let body = Some(text(&read["body"])).filter(|b| !b.is_empty()).unwrap_or_else(|| text(&read["snippet"]));
    let prompt = reply_prompt(&read, &body, &ctx.cfg.0["mail"]["name"]);
    let first = who(&read).split_whitespace().next().unwrap_or("mail").to_lowercase();
    let subject: String = text(&read["subject"]).chars().take(24).collect();
    let label = format!("reply {first} {subject}").trim().to_string();
    let repos = ctx.repos();
    inbox::start_action(ctx, &repos, &label, &prompt, &inbox::Launch::new());
    Ok(())
}

fn reply_prompt(message: &Value, body: &str, tool: &Value) -> String {
    let from = match (text(&message["fromName"]), text(&message["from"])) {
        (name, address) if !name.is_empty() && name != address => format!("{name} <{address}>"),
        (_, address) => address,
    };
    let tool = Some(text(tool)).filter(|t| !t.is_empty()).unwrap_or_else(|| "my mail tool".into());
    format!(
        "Draft a reply to this email for me to review. Write the draft here, in my voice, short and to the point; ask me \
         first if it needs facts only I know. Do not send anything and change nothing in my mailbox: no sending, no draft \
         saved in {tool}, no archiving or labels, unless I ask.\n\nFrom: {from}\nSubject: {}\nDate: {}\n{}\n\n{body}",
        text(&message["subject"]),
        text(&message["date"]),
        text(&message["url"]),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_reply_prompt_carries_the_message_and_forbids_sending() {
        let message =
            json!({"id": "m1", "from": "ana@acme.com", "fromName": "Ana Lima", "subject": "Lunch?", "date": "2026-11-04T10:00:00Z", "url": "https://mail/m1"});
        let prompt = reply_prompt(&message, "Shall we meet at noon?", &json!("Gmail"));
        assert!(prompt.contains("Do not send anything"));
        assert!(prompt.contains("no draft saved in Gmail"));
        assert!(prompt.contains("From: Ana Lima <ana@acme.com>\nSubject: Lunch?"));
        assert!(prompt.ends_with("Shall we meet at noon?"));
        assert_eq!(who(&json!({"from": "bob@acme.com", "fromName": ""})), "bob@acme.com");
    }
}
