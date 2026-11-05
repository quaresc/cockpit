//! Mail acted on from the mail space: a message opened in the mail client, its thread read whole, archived or marked
//! read through the hub's own mail server (else by a Claude Code session), or answered by an agent that drafts the reply
//! for me to review.

use cockpit_connectors::mail;
use cockpit_connectors::tools::Mapped;
use cockpit_core::files;
use serde_json::Value;

use super::{Ctx, arg, inbox};
use crate::config;
use crate::feed::mail as kept;
use crate::value::{Res, text};

/// A thread asked of a Claude Code session is asked again after this long without an answer.
const ASK_AGAIN_MS: u64 = 3 * 60_000;

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
    let id = text(&message["id"]);
    // The list shows it done at once; the server or a session's connector does it meanwhile, a failure brings it back.
    change(&id, |_| None);
    match served() {
        Ok((server, mapping)) => {
            let done = crate::feed::mcp::with(&server, async |c| mail::archive(&Mapped { connection: c, section: &mapping }, &message).await);
            undo_on_failure(&id, &message, done)?;
        }
        Err(_) => by_session(ctx, "mail-archive", &message, "archiving")?,
    }
    ctx.note(format!("archived {}", text(&message["subject"])));
    Ok(())
}

/// Puts the message back as it was when its change failed, newest first again.
fn undo_on_failure(id: &str, message: &Value, done: Result<(), String>) -> Res {
    if done.is_err() {
        crate::store::global().update("mail", |doc| {
            let mut doc = doc.clone();
            if let Some(messages) = doc["messages"].as_array_mut() {
                messages.retain(|m| m["id"] != id);
                messages.push(message.clone());
                messages.sort_by(|a, b| text(&b["date"]).cmp(&text(&a["date"])));
            }
            doc
        });
    }
    done
}

/// `mail_read [message]`: marked read.
pub fn read(ctx: &mut Ctx, args: &[Value]) -> Res {
    let message = message(args)?;
    let id = text(&message["id"]);
    change(&id, |m| {
        let mut m = m.clone();
        m["unread"] = false.into();
        Some(m)
    });
    match served() {
        Ok((server, mapping)) => {
            let done = crate::feed::mcp::with(&server, async |c| mail::mark_read(&Mapped { connection: c, section: &mapping }, &message).await);
            undo_on_failure(&id, &message, done)?;
        }
        Err(_) => by_session(ctx, "mail-read", &message, "marking read")?,
    }
    ctx.note(format!("marked {} read", text(&message["subject"])));
    Ok(())
}

/// `mail_thread [message]`: the message's whole thread read into ~/.cache/cockpit/mail/<id>.json, through the hub's
/// mail server at once, else asked of a Claude Code session; the `mail-thread` view shows it once there.
pub fn thread(ctx: &mut Ctx, args: &[Value]) -> Res {
    let message = message(args)?;
    let id = text(&message["id"]);
    kept::prune_threads();
    let path = kept::thread_path(&id);
    if kept::kept_fresh(&message) {
        return Ok(());
    }
    let asked = path.with_extension("asked");
    let now = crate::feed::now_ms();
    match served() {
        Ok((server, mapping)) => {
            let read = crate::feed::mcp::with(&server, async |c| mail::thread(&Mapped { connection: c, section: &mapping }, &mapping, &message).await);
            let failed = read.as_ref().err().cloned();
            kept::keep_thread(&id, read);
            failed.map_or(Ok(()), Err)
        }
        Err(_) => {
            let at = std::fs::read_to_string(&asked).ok().and_then(|t| t.trim().parse::<u64>().ok()).unwrap_or(0);
            if now.saturating_sub(at) < ASK_AGAIN_MS && files::load(&path).is_none() {
                return Ok(());
            }
            let _ = std::fs::remove_file(&path);
            std::fs::create_dir_all(kept::threads_dir()).map_err(|e| e.to_string())?;
            std::fs::write(&asked, now.to_string()).map_err(|e| e.to_string())?;
            by_session(ctx, "mail-get", &message, "loading…")
        }
    }
}

/// A change a Claude Code session's mail connector makes, filed as a request, when the hub reaches no mail server.
fn by_session(ctx: &mut Ctx, kind: &str, message: &Value, working: &str) -> Res {
    let id = text(&message["id"]);
    let payload = serde_json::json!({"type": kind, "label": text(&message["subject"]), "message": id});
    inbox::request(ctx, payload, &format!("mail:{id}"), working)
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
    let kept = files::load(&kept::thread_path(&text(&read["id"])));
    let body = Some(thread_text(kept.as_ref()))
        .filter(|b| !b.is_empty())
        .or_else(|| Some(text(&read["body"])).filter(|b| !b.is_empty()))
        .unwrap_or_else(|| text(&read["snippet"]));
    let prompt = reply_prompt(&read, &body, &ctx.cfg.0["mail"]["name"]);
    let first = who(&read).split_whitespace().next().unwrap_or("mail").to_lowercase();
    let subject: String = text(&read["subject"]).chars().take(24).collect();
    let label = format!("reply {first} {subject}").trim().to_string();
    let repos = ctx.repos();
    inbox::start_action(ctx, &repos, &label, &prompt, &inbox::Launch::new());
    Ok(())
}

/// A kept thread as the prompt carries it: each message's sender, date and text, cut to the last 8000 characters.
fn thread_text(kept: Option<&Value>) -> String {
    let Some(messages) = kept.and_then(|k| k["messages"].as_array()) else { return String::new() };
    let all = messages
        .iter()
        .map(|m| format!("--- {} <{}>, {}\n{}", text(&m["fromName"]), text(&m["from"]), text(&m["date"]), text(&m["text"]).trim()))
        .collect::<Vec<_>>()
        .join("\n\n");
    let skip = all.chars().count().saturating_sub(8000);
    all.chars().skip(skip).collect()
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
         saved in {tool}, no archiving or labels, unless I ask. Read the whole thread with {tool} when this excerpt is not \
         enough.\n\nFrom: {from}\nSubject: {}\nDate: {}\n{}\n\n{body}",
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
        let kept = json!({"messages": [{"from": "maya@example.com", "fromName": "Maya", "date": "2026-11-04T10:00:00Z", "text": "Noon?\n"}]});
        assert_eq!(thread_text(Some(&kept)), "--- Maya <maya@example.com>, 2026-11-04T10:00:00Z\nNoon?");
        assert_eq!(thread_text(None), "");
    }
}
