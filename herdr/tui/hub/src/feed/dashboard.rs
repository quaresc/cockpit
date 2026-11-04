//! The counts on the herdr inbox space's sidebar row (reviews, CI, tickets, unread mail), reported by the hub where no
//! Claude Code session's plugin does.

use std::sync::Mutex;

use serde_json::Value;

use crate::config::Config;
use crate::value::list;

const OLD_REVIEW_DAYS: i64 = 60;

/// The tokens for the inbox and mail documents: name and text, none to clear it.
pub fn tokens(inbox: &Value, mail: &Value, muted: &[String], now_secs: i64) -> Vec<(&'static str, Option<String>)> {
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
    let unread = list(&mail["messages"]).iter().filter(|m| m["unread"] != false).count();
    vec![
        ("review", Some(if direct + team == 0 { "\u{f441}  nothing to review".into() } else { format!("\u{f441}  {direct} you · {team} team") })),
        ("ci_fail", (failing > 0).then(|| format!("\u{f52f}  {failing} failing"))),
        ("ci_ok", (failing == 0).then(|| "\u{f49e}  all green".to_string())),
        ("tickets", Some(format!("\u{f41b}  {working} active · {}", tickets.len()))),
        ("mail", mail["messages"].is_array().then(|| if unread == 0 { "\u{f42f}  inbox zero".to_string() } else { format!("\u{f42f}  {unread} unread") })),
    ]
}

/// Reports the counts on the inbox space's row when they changed, unless Claude Code's plugin does it.
pub fn report(config: &Config) {
    static LAST: Mutex<String> = Mutex::new(String::new());
    if cockpit_core::agent::get("claude").is_some_and(|a| a.installed()) || config.0["frontend"] == "app" {
        return;
    }
    let store = crate::store::global();
    let muted: Vec<String> = cockpit_core::files::load(&cockpit_core::files::config_dir().join("muted.json"))
        .map(|m| list(&m).iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    let now = chrono::Utc::now().timestamp();
    let tokens = tokens(&store.get("inbox"), &store.get("mail"), &muted, now);
    let Some(space) = crate::herdr::spaces().into_iter().find(|w| w["label"].as_str().is_some_and(|l| l.ends_with("  inbox"))) else { return };
    let id = space["workspace_id"].as_str().unwrap_or_default().to_string();
    let key = format!("{id}|{tokens:?}");
    if LAST.lock().is_ok_and(|last| *last == key) {
        return;
    }
    let mut args: Vec<String> = ["workspace", "report-metadata", &id, "--source", "cockpit"].iter().map(|s| s.to_string()).collect();
    for (name, value) in &tokens {
        match value {
            Some(text) => args.extend(["--token".into(), format!("{name}={text}")]),
            None => args.extend(["--clear-token".into(), name.to_string()]),
        }
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    if crate::herdr::run(&refs).ok()
        && let Ok(mut last) = LAST.lock()
    {
        *last = key;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_row_counts_reviews_ci_tickets_and_mail() {
        let inbox = json!({"direct": [{"updatedAt": "2026-11-01T00:00:00Z"}], "team": [{"repo": "noisy"}, {"repo": "api"}],
            "mine": [{"ci": "FAILURE"}], "tickets": [{"statusType": "started", "status": "In Progress"}, {"statusType": "started", "status": "To Deploy"}]});
        let mail = json!({"messages": [{"unread": true}, {"unread": false}]});
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-04T00:00:00Z").unwrap().timestamp();
        let tokens = tokens(&inbox, &mail, &["noisy".into()], now);
        let get = |name: &str| tokens.iter().find(|(n, _)| *n == name).and_then(|(_, v)| v.clone());
        assert_eq!(get("review").unwrap(), "\u{f441}  1 you · 1 team");
        assert_eq!((get("ci_fail").unwrap(), get("ci_ok")), ("\u{f52f}  1 failing".to_string(), None));
        assert_eq!(get("tickets").unwrap(), "\u{f41b}  1 active · 2");
        assert_eq!(get("mail").unwrap(), "\u{f42f}  1 unread");
        assert_eq!(super::tokens(&inbox, &Value::Null, &[], now).iter().find(|(n, _)| *n == "mail").unwrap().1, None);
    }
}
