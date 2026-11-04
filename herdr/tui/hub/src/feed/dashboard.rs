//! The counts on the herdr inbox space's sidebar row (reviews, CI, tickets) and the mail space's (the messages that need
//! me), reported by the hub where no Claude Code session's plugin does.

use std::sync::Mutex;

use serde_json::Value;

use crate::config::Config;
use crate::value::list;

const OLD_REVIEW_DAYS: i64 = 60;

/// The inbox space's tokens: name and text, none to clear it; the mail count lives on the mail space's row.
pub fn tokens(inbox: &Value, muted: &[String], now_secs: i64) -> Vec<(&'static str, Option<String>)> {
    let fresh = |pr: &&Value| {
        pr["updatedAt"]
            .as_str()
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
            .is_none_or(|t| (now_secs - t.timestamp()) / 86_400 <= OLD_REVIEW_DAYS)
    };
    let direct = list(&inbox["direct"]).iter().filter(fresh).count();
    let team = list(&inbox["team"]).iter().filter(|pr| !muted.iter().any(|m| pr["repo"] == m.as_str())).filter(fresh).count();
    let failing = list(&inbox["mine"]).iter().filter(|pr| pr["ci"] == "FAILURE").count();
    let tickets = list(&inbox["tickets"]);
    let deploying = |t: &Value| t["status"].as_str().is_some_and(|s| s.to_lowercase().contains("deploy"));
    let working = tickets.iter().filter(|t| t["statusType"] == "started" && !deploying(t)).count();
    vec![
        ("review", Some(if direct + team == 0 { "\u{f441}  nothing to review".into() } else { format!("\u{f441}  {direct} you · {team} team") })),
        ("ci_fail", (failing > 0).then(|| format!("\u{f52f}  {failing} failing"))),
        ("ci_ok", (failing == 0).then(|| "\u{f49e}  all green".to_string())),
        ("tickets", Some(format!("\u{f41b}  {working} active · {}", tickets.len()))),
        ("mail", None),
    ]
}

/// Reports the counts on the inbox and mail spaces' rows when they changed, unless Claude Code's plugin does it.
pub fn report(config: &Config) {
    if cockpit_core::agent::get("claude").is_some_and(|a| a.installed()) || config.0["frontend"] == "app" {
        return;
    }
    static INBOX: Mutex<String> = Mutex::new(String::new());
    static MAIL: Mutex<String> = Mutex::new(String::new());
    let store = crate::store::global();
    let muted: Vec<String> = cockpit_core::files::load(&cockpit_core::files::config_dir().join("muted.json"))
        .map(|m| list(&m).iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    let now = chrono::Utc::now().timestamp();
    let spaces = crate::herdr::spaces();
    send(&spaces, "inbox", &tokens(&store.get("inbox"), &muted, now), &INBOX);
    send(&spaces, "mail", &[("mail", cockpit_connectors::mail::row_text(&store.get("mail")))], &MAIL);
}

/// The tokens on the row of the pinned space with that name, unless they are what was sent last.
fn send(spaces: &[Value], name: &str, tokens: &[(&str, Option<String>)], last: &Mutex<String>) {
    let suffix = format!("  {name}");
    let Some(space) = spaces.iter().find(|w| w["label"].as_str().is_some_and(|l| l.ends_with(&suffix))) else { return };
    let id = space["workspace_id"].as_str().unwrap_or_default().to_string();
    let key = format!("{id}|{tokens:?}");
    if last.lock().is_ok_and(|last| *last == key) {
        return;
    }
    let mut args: Vec<String> = ["workspace", "report-metadata", &id, "--source", "cockpit"].iter().map(|s| s.to_string()).collect();
    for (name, value) in tokens {
        match value {
            Some(text) => args.extend(["--token".into(), format!("{name}={text}")]),
            None => args.extend(["--clear-token".into(), name.to_string()]),
        }
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    if crate::herdr::run(&refs).ok()
        && let Ok(mut last) = last.lock()
    {
        *last = key;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_row_counts_reviews_ci_and_tickets() {
        let inbox = json!({"direct": [{"updatedAt": "2026-11-01T00:00:00Z"}], "team": [{"repo": "noisy"}, {"repo": "api"}],
            "mine": [{"ci": "FAILURE"}], "tickets": [{"statusType": "started", "status": "In Progress"}, {"statusType": "started", "status": "To Deploy"}]});
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-04T00:00:00Z").unwrap().timestamp();
        let tokens = tokens(&inbox, &["noisy".into()], now);
        let get = |name: &str| tokens.iter().find(|(n, _)| *n == name).and_then(|(_, v)| v.clone());
        assert_eq!(get("review").unwrap(), "\u{f441}  1 you · 1 team");
        assert_eq!((get("ci_fail").unwrap(), get("ci_ok")), ("\u{f52f}  1 failing".to_string(), None));
        assert_eq!(get("tickets").unwrap(), "\u{f41b}  1 active · 2");
        assert_eq!(get("mail"), None, "cleared: the mail space's row has it");
    }
}
