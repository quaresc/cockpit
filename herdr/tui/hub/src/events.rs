//! What happened, read from two versions of a shared document: a review requested, CI turning, a ticket moving on, a
//! release judged. The store keeps a week of them; the palette searches them and the apps mark the rows they touch.

use std::collections::{HashMap, HashSet};

use serde_json::{Value, json};

use crate::value::list;

fn s<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or_default()
}

/// One event: the app and row it belongs to (the row's key there), and its line.
fn event(app: &str, key: &str, title: String) -> Value {
    json!({"app": app, "key": key, "title": title})
}

/// An id as the apps write it in a row key: a string as is, a number in digits.
fn id_text(id: &Value) -> String {
    id.as_str().map_or_else(|| id.to_string(), str::to_string)
}

fn pr_name(pr: &Value) -> String {
    format!("{}#{}", s(pr, "repo"), pr["number"])
}

/// What changed between two versions of a document, the inbox telling which PR a CI run belongs to; nothing for a
/// first version or another shape.
pub fn of(doc: &str, before: &Value, after: &Value, inbox_doc: &Value) -> Vec<Value> {
    if before.is_null() || before["version"] != after["version"] {
        return Vec::new();
    }
    match doc {
        "inbox" => inbox(before, after),
        "ci" => ci(before, after, inbox_doc),
        "monitor" => monitor(before, after),
        _ => Vec::new(),
    }
}

fn by<'a>(items: &'a [Value], key: &str) -> HashMap<&'a str, &'a Value> {
    items.iter().map(|i| (s(i, key), i)).collect()
}

fn ci_state(pr: &Value) -> &str {
    match s(pr, "ci") {
        "FAILURE" | "ERROR" => "failing",
        "SUCCESS" => "passing",
        _ => "",
    }
}

fn inbox(before: &Value, after: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    for (field, what) in [("direct", "👀 Review requested"), ("team", "👥 Team review")] {
        let known = by(list(&before[field]), "url");
        for pr in list(&after[field]).iter().filter(|p| !known.contains_key(s(p, "url"))) {
            out.push(event("inbox", s(pr, "url"), format!("{what}: {} {}", pr_name(pr), s(pr, "title"))));
        }
    }
    let was_mine = by(list(&before["mine"]), "url");
    for pr in list(&after["mine"]) {
        let (url, name) = (s(pr, "url"), pr_name(pr));
        let Some(was) = was_mine.get(url) else {
            out.push(event("inbox", url, format!("📝 Opened: {name} {}", s(pr, "title"))));
            continue;
        };
        match (ci_state(was), ci_state(pr)) {
            (a, "failing") if a != "failing" => out.push(event("inbox", url, format!("🔴 CI failing: {name}"))),
            ("failing", "passing") => out.push(event("inbox", url, format!("🟢 CI passing again: {name}"))),
            _ => {}
        }
        if pr["review"] != was["review"] {
            match s(pr, "review") {
                "APPROVED" => out.push(event("inbox", url, format!("✅ Approved: {name}"))),
                "CHANGES_REQUESTED" => out.push(event("inbox", url, format!("❌ Changes requested: {name}"))),
                _ => {}
            }
        }
    }
    let merged_before: HashSet<&str> = list(&before["merged"]).iter().map(|p| s(p, "url")).collect();
    for pr in list(&after["merged"]).iter().filter(|p| !merged_before.contains(s(p, "url"))) {
        out.push(event("inbox", s(pr, "url"), format!("🟣 Merged: {}", pr_name(pr))));
    }
    let tickets_before = by(list(&before["tickets"]), "id");
    let tickets_after = by(list(&after["tickets"]), "id");
    for ticket in list(&after["tickets"]) {
        let id = s(ticket, "id");
        match tickets_before.get(id) {
            None if !tickets_before.is_empty() => out.push(event("inbox", id, format!("🎫 Assigned: {id} {}", s(ticket, "title")))),
            Some(was) if was["status"] != ticket["status"] => {
                out.push(event("inbox", id, format!("🎫 {id}: {} → {}", s(was, "status"), s(ticket, "status"))));
            }
            _ => {}
        }
    }
    for ticket in list(&before["tickets"]).iter().filter(|t| !tickets_after.contains_key(s(t, "id"))) {
        out.push(event("inbox", s(ticket, "id"), format!("🎫 {} left the inbox ({})", s(ticket, "id"), s(ticket, "title"))));
    }
    if let Some(deploys) = after["deploys"].as_object() {
        for (id, prs) in deploys {
            let was = &before["deploys"][id];
            for pr in list(prs).iter().filter(|p| p["isLive"] == true) {
                let was_live = list(was).iter().any(|w| w["repo"] == pr["repo"] && w["number"] == pr["number"] && w["isLive"] == true);
                if !was_live {
                    out.push(event("inbox", id, format!("🚀 Live: {id} {}#{} in {}", s(pr, "repo"), pr["number"], s(pr, "tag"))));
                }
            }
        }
    }
    out
}

fn ci(before: &Value, after: &Value, inbox: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    for run in list(&after["runs"]).iter().filter(|r| r["isRunning"] != true) {
        let was = list(&before["runs"]).iter().find(|r| r["repo"] == run["repo"] && r["branch"] == run["branch"]);
        let ended = was.is_some_and(|w| w["runId"] != run["runId"] || w["isRunning"] == true);
        let pr = list(&inbox["mine"]).iter().find(|p| p["repo"] == run["repo"] && p["branch"] == run["branch"]);
        let key = pr.map(|p| s(p, "url").to_string()).unwrap_or_else(|| format!("{}/{}", s(run, "repo"), s(run, "branch")));
        match (ended, s(run, "outcome")) {
            (true, "succeeded") => out.push(event("inbox", &key, format!("🟢 CI passed: {} {}", s(run, "repo"), s(run, "branch")))),
            (true, "failed") => {
                let job = run["failed"]["job"].as_str().map(|j| format!(" · {j}")).unwrap_or_default();
                out.push(event("inbox", &key, format!("🔴 CI failed: {} {}{job}", s(run, "repo"), s(run, "branch"))));
            }
            _ => {}
        }
    }
    out
}

fn monitor(before: &Value, after: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    let apps_before = by(list(&before["apps"]), "id");
    for app in list(&after["apps"]) {
        let (id, name) = (s(app, "id"), s(app, "name"));
        let was = apps_before.get(id).copied().unwrap_or(&Value::Null);
        let releases_before: HashMap<String, &Value> = list(&was["releases"]).iter().map(|r| (id_text(&r["id"]), r)).collect();
        for release in list(&app["releases"]) {
            let rid = id_text(&release["id"]);
            let key = format!("{id}/release/{rid}");
            let tag = s(release, "tag");
            let state = s(&release["verdict"], "state");
            match releases_before.get(&rid) {
                None => out.push(event("monitor", &key, format!("📦 {name} {tag} released by {}", s(release, "actor")))),
                Some(was) if s(&was["verdict"], "state") != state => {
                    let line = match state {
                        "ok" => format!("✅ {name} {tag} looks fine"),
                        "moved" => format!("⚠️ {name} {tag} moved: {}", s(&release["verdict"], "summary")),
                        "failed" => format!("❌ {name} {tag} failed to deploy"),
                        _ => continue,
                    };
                    out.push(event("monitor", &key, line));
                }
                _ => {}
            }
        }
        let alerting_before: HashSet<String> = list(&was["monitors"]["alerting"]).iter().map(|m| m["id"].to_string()).collect();
        for m in list(&app["monitors"]["alerting"]).iter().filter(|m| !alerting_before.contains(&m["id"].to_string())) {
            out.push(event("monitor", id, format!("🔔 {} {}", s(m, "name"), s(m, "status").to_lowercase())));
        }
        let issues_before: HashSet<&str> = list(&was["issues"]).iter().map(|i| s(i, "id")).collect();
        for issue in list(&app["issues"]).iter().filter(|i| i["isNew"] == true && !issues_before.contains(s(i, "id"))) {
            out.push(event("monitor", id, format!("🐞 New error in {name}: {} {}", s(issue, "type"), s(issue, "message"))));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titles(events: &[Value]) -> Vec<&str> {
        events.iter().map(|e| e["title"].as_str().unwrap()).collect()
    }

    #[test]
    fn the_inbox_says_what_moved() {
        let pr = |n: u32, ci: &str, review: &str| json!({"repo": "api", "number": n, "title": "t", "url": format!("u{n}"), "ci": ci, "review": review});
        let before = json!({"version": 1, "direct": [], "mine": [pr(1, "PENDING", "REVIEW_REQUIRED"), pr(2, "FAILURE", "")], "merged": [],
            "tickets": [{"id": "ENG-1", "status": "Todo", "title": "a"}, {"id": "ENG-2", "status": "To review", "title": "b"}],
            "deploys": {"ENG-2": [{"repo": "api", "number": 2, "tag": null, "isLive": false}]}});
        let after = json!({"version": 1, "direct": [{"repo": "web", "number": 9, "title": "Dark mode", "url": "w9"}],
            "mine": [pr(1, "FAILURE", "APPROVED"), pr(2, "SUCCESS", ""), pr(3, "", "")], "merged": [{"repo": "api", "number": 2, "url": "u2"}],
            "tickets": [{"id": "ENG-1", "status": "In progress", "title": "a"}, {"id": "ENG-3", "status": "Todo", "title": "c"}],
            "deploys": {"ENG-2": [{"repo": "api", "number": 2, "tag": "v2", "isLive": true}]}});
        let events = of("inbox", &before, &after, &Value::Null);
        assert_eq!(
            titles(&events),
            [
                "👀 Review requested: web#9 Dark mode",
                "🔴 CI failing: api#1",
                "✅ Approved: api#1",
                "🟢 CI passing again: api#2",
                "📝 Opened: api#3 t",
                "🟣 Merged: api#2",
                "🎫 ENG-1: Todo → In progress",
                "🎫 Assigned: ENG-3 c",
                "🎫 ENG-2 left the inbox (b)",
                "🚀 Live: ENG-2 api#2 in v2",
            ]
        );
        assert_eq!((&events[1]["app"], &events[1]["key"]), (&json!("inbox"), &json!("u1")));
        assert!(of("inbox", &Value::Null, &after, &Value::Null).is_empty(), "a first version says nothing");
        assert!(of("inbox", &json!({"version": 0}), &after, &Value::Null).is_empty(), "nor another shape");
    }

    #[test]
    fn the_monitor_says_releases_verdicts_alerts_and_new_errors() {
        let before = json!({"version": 1, "apps": [{"id": "api-production", "name": "api",
            "releases": [{"id": 1, "tag": "v1", "actor": "jonas", "verdict": {"state": "watching"}}],
            "monitors": {"alerting": []}, "issues": []}]});
        let after = json!({"version": 1, "apps": [{"id": "api-production", "name": "api",
            "releases": [{"id": 2, "tag": "v2", "actor": "me", "verdict": {"state": "watching"}},
                         {"id": 1, "tag": "v1", "actor": "jonas", "verdict": {"state": "moved", "summary": "errors 0.1% → 2%"}}],
            "monitors": {"alerting": [{"id": 7, "name": "[api] Availability", "status": "Alert"}]},
            "issues": [{"id": "e1", "type": "Req.TransportError", "message": "socket closed", "isNew": true}, {"id": "e0", "type": "x", "message": "y", "isNew": false}]}]});
        let events = of("monitor", &before, &after, &Value::Null);
        assert_eq!(
            titles(&events),
            [
                "📦 api v2 released by me",
                "⚠️ api v1 moved: errors 0.1% → 2%",
                "🔔 [api] Availability alert",
                "🐞 New error in api: Req.TransportError socket closed"
            ]
        );
        assert_eq!(events[1]["key"], "api-production/release/1");
    }

    #[test]
    fn a_ci_run_that_ends_says_how() {
        let before = json!({"version": 3, "runs": [{"repo": "api", "branch": "b", "runId": "1", "isRunning": true}]});
        let after = json!({"version": 3, "runs": [{"repo": "api", "branch": "b", "runId": "1", "isRunning": false, "outcome": "failed", "failed": {"job": "lint"}}]});
        let inbox = json!({"mine": [{"repo": "api", "branch": "b", "url": "u4"}]});
        let events = of("ci", &before, &after, &inbox);
        assert_eq!(titles(&events), ["🔴 CI failed: api b · lint"]);
        assert_eq!(events[0]["key"], "u4");
        assert!(of("ci", &after, &after, &inbox).is_empty());
    }
}
