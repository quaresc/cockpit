//! What changed in the data the feed writes, worth a notification, with where its click leads; the same rules as the
//! mod's, so whichever of the two fetched a field announces its changes, once.

use std::collections::HashSet;

use serde_json::Value;

use crate::config::Config;
use crate::value::list;

#[derive(Clone, Debug, PartialEq)]
pub struct News {
    pub text: String,
    pub action: String,
    pub buttons: Vec<(String, String)>,
}

fn news(text: String, action: String, buttons: &[(&str, String)]) -> News {
    News { text, action, buttons: buttons.iter().map(|(label, target)| (label.to_string(), target.clone())).collect() }
}

fn s<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or_default()
}

fn is_failing(pr: &Value) -> bool {
    matches!(pr["ci"].as_str(), Some("FAILURE" | "ERROR"))
}

/// A new direct review request, a PR of mine newly failing, approved or sent back.
pub fn pr_changes(prev: &Value, next: &Value) -> Vec<News> {
    let mut out = Vec::new();
    let known: HashSet<&str> = list(&prev["direct"]).iter().map(|p| s(p, "url")).collect();
    for pr in list(&next["direct"]).iter().filter(|p| !known.contains(s(p, "url"))) {
        let url = s(pr, "url").to_string();
        out.push(news(
            format!("👀 Review requested: {}#{} {}", s(pr, "repo"), pr["number"], s(pr, "title")),
            url.clone(),
            &[("Review", format!("act:review:{url}"))],
        ));
    }
    for pr in list(&next["mine"]) {
        let Some(was) = list(&prev["mine"]).iter().find(|p| p["url"] == pr["url"]) else {
            continue;
        };
        let (url, name) = (s(pr, "url"), format!("{}#{}", s(pr, "repo"), pr["number"]));
        if is_failing(pr) && !is_failing(was) {
            out.push(news(
                format!("🔴 CI failing: {name}"),
                format!("{url}/checks"),
                &[("Why?", format!("act:ci:{url}")), ("Rerun", format!("act:rerun:{url}"))],
            ));
        }
        if pr["review"] != was["review"] && pr["review"] == "APPROVED" {
            out.push(news(format!("✅ Approved: {name}"), url.into(), &[]));
        }
        if pr["review"] != was["review"] && pr["review"] == "CHANGES_REQUESTED" {
            out.push(news(format!("❌ Changes requested: {name}"), format!("{url}/files"), &[]));
        }
    }
    out
}

/// Tickets newly assigned to me, once there was a list to compare with.
pub fn ticket_changes(prev: &Value, next: &Value) -> Vec<News> {
    let before = list(&prev["tickets"]);
    if before.is_empty() {
        return Vec::new();
    }
    let ids: HashSet<&str> = before.iter().map(|t| s(t, "id")).collect();
    list(&next["tickets"])
        .iter()
        .filter(|t| !ids.contains(s(t, "id")))
        .map(|t| news(format!("🎫 Assigned: {} {}", s(t, "id"), s(t, "title")), s(t, "url").into(), &[("Plan", format!("act:plan:{}", s(t, "id")))]))
        .collect()
}

/// A run's notification when it ends, its click opening the checks of the branch's PR.
pub fn ci_changes(prev: &Value, next: &Value, inbox: &Value) -> Vec<News> {
    let mut out = Vec::new();
    let prev_at = prev["at"].as_f64().unwrap_or(0.0);
    for run in list(&next["runs"]).iter().filter(|r| r["isRunning"] != true) {
        let was = list(&prev["runs"]).iter().find(|r| r["repo"] == run["repo"] && r["branch"] == run["branch"]);
        let is_newly_ended = match was {
            Some(was) => was["runId"] != run["runId"] || was["isRunning"] == true,
            None => chrono::DateTime::parse_from_rfc3339(s(run, "createdAt")).is_ok_and(|at| at.timestamp_millis() as f64 > prev_at),
        };
        if !is_newly_ended {
            continue;
        }
        let pr = list(&inbox["mine"]).iter().find(|p| p["repo"] == run["repo"] && p["branch"] == run["branch"]);
        let action = pr.map(|p| format!("{}/checks", s(p, "url"))).unwrap_or_else(|| "space:inbox".into());
        let (repo, branch) = (s(run, "repo"), s(run, "branch"));
        match s(run, "outcome") {
            "succeeded" => out.push(news(format!("🟢 CI passed: {repo} {branch}"), action, &[])),
            "failed" => {
                let job = run["failed"]["job"].as_str().map(|j| format!(" · {j}")).unwrap_or_default();
                let mut buttons = Vec::new();
                if let Some(pr) = pr {
                    let url = s(pr, "url");
                    buttons.push(("Why?", format!("act:ci:{url}")));
                    if run["workflowId"].is_string() {
                        buttons.push(("Rerun", format!("act:rerun:{url}")));
                    }
                }
                out.push(news(format!("🔴 CI failed: {repo} {branch}{job}"), action, &buttons));
            }
            _ => {}
        }
    }
    out
}

/// The inbox's news as notifications: each on its own, or more than three as one that opens the inbox.
pub fn announce_inbox(config: &Config, news: &[News]) {
    if news.len() > 3 {
        let texts: Vec<&str> = news.iter().take(3).map(|n| n.text.as_str()).collect();
        notify(config, &format!("📥 {} updates", news.len()), &format!("{}\n…", texts.join("\n")), "space:inbox", &[]);
    } else {
        for n in news {
            notify(config, "📥 Inbox", &n.text, &n.action, &n.buttons);
        }
    }
}

pub fn announce_ci(config: &Config, news: &[News]) {
    let name = config.0["ci"]["name"].as_str().unwrap_or("CI");
    for n in news {
        notify(config, &format!("🔁 {name}"), &n.text, &n.action, &n.buttons);
    }
}

/// One system notification, unless notifications are off.
fn notify(config: &Config, title: &str, body: &str, action: &str, buttons: &[(String, String)]) {
    if config.0["notifications"] == false {
        return;
    }
    let buttons: Vec<String> = buttons.iter().map(|(label, target)| format!("{label}={target}")).collect();
    cockpit_core::notify::post(title, body, action, &buttons);
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn pr(url: &str, ci: &str, review: Value) -> Value {
        json!({"repo": "api", "number": 4, "title": "Sign-in", "url": url, "ci": ci, "review": review, "branch": "me/eng-401-login"})
    }

    #[test]
    fn a_new_review_request_a_newly_failing_pr_and_a_review_verdict() {
        let prev = json!({"direct": [], "mine": [pr("https://github.com/acme/api/pull/4", "SUCCESS", Value::Null)]});
        let next = json!({
            "direct": [{"repo": "web", "number": 9, "title": "Dark mode", "url": "https://github.com/acme/web/pull/9"}],
            "mine": [pr("https://github.com/acme/api/pull/4", "FAILURE", json!("APPROVED")), pr("https://github.com/acme/api/pull/5", "FAILURE", Value::Null)],
        });
        let texts: Vec<String> = pr_changes(&prev, &next).into_iter().map(|n| n.text).collect();
        assert_eq!(texts, ["👀 Review requested: web#9 Dark mode", "🔴 CI failing: api#4", "✅ Approved: api#4"]);
        assert_eq!(pr_changes(&next, &next), []);
        assert_eq!(pr_changes(&prev, &next)[1].buttons[1], ("Rerun".into(), "act:rerun:https://github.com/acme/api/pull/4".into()));
    }

    #[test]
    fn a_ticket_is_announced_only_against_an_earlier_list() {
        let next = json!({"tickets": [{"id": "ENG-1", "title": "a", "url": "u1"}, {"id": "ENG-2", "title": "b", "url": "u2"}]});
        assert_eq!(ticket_changes(&json!({"tickets": []}), &next), []);
        let news = ticket_changes(&json!({"tickets": [{"id": "ENG-1"}]}), &next);
        assert_eq!(news, [News { text: "🎫 Assigned: ENG-2 b".into(), action: "u2".into(), buttons: vec![("Plan".into(), "act:plan:ENG-2".into())] }]);
    }

    #[test]
    fn a_run_is_announced_once_when_it_ends() {
        let run = json!({"repo": "api", "branch": "me/eng-401-login", "runId": "a", "createdAt": "2026-10-27T10:00:00Z", "done": 3, "total": 3});
        let with = |over: Value| {
            let mut r = run.clone();
            r.as_object_mut().unwrap().extend(over.as_object().unwrap().clone());
            r
        };
        let prev = json!({"at": 0, "runs": [with(json!({"isRunning": true, "outcome": "running"}))]});
        let next = json!({"at": 1, "runs": [with(json!({"isRunning": false, "outcome": "failed", "workflowId": "w", "failed": {"job": "test"}}))]});
        let inbox = json!({"mine": [{"repo": "api", "branch": "me/eng-401-login", "url": "https://github.com/acme/api/pull/4"}]});
        let news = ci_changes(&prev, &next, &inbox);
        assert_eq!(news.len(), 1);
        assert_eq!(news[0].text, "🔴 CI failed: api me/eng-401-login · test");
        assert_eq!(news[0].action, "https://github.com/acme/api/pull/4/checks");
        assert_eq!(news[0].buttons.len(), 2);
        assert_eq!(ci_changes(&next, &next, &inbox), []);
    }
}
