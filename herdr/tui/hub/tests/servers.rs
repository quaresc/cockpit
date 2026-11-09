//! The hub's own MCP servers, played by a fake one over stdio: each source's mapping reads tickets, pull requests, CI
//! runs, meetings, deploys and the monitor's numbers into the caches, and the apps' requests and ticket actions go out
//! through the same servers, whatever the tool and whatever the agent.

mod common;

use std::path::Path;
use std::time::{Duration, Instant};

use common::{Hub, Sandbox};
use serde_json::{Value, json};

fn read(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap_or_default()).unwrap_or(Value::Null)
}

fn wait_for(path: &Path, ready: impl Fn(&Value) -> bool) -> Value {
    let start = Instant::now();
    loop {
        let value = read(path);
        if ready(&value) {
            return value;
        }
        assert!(start.elapsed() < Duration::from_secs(30), "{} never got ready: {value}", path.display());
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// An ISO time this many minutes from now.
fn at(minutes: i64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    cockpit_iso(now + minutes * 60)
}

fn cockpit_iso(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rest = secs.rem_euclid(86_400);
    // Days since 1970-01-01 to a civil date.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rest / 3600, rest / 60 % 60, rest % 60)
}

/// A server entry running the fake with these answers, written beside the sandbox.
fn fake(sandbox: &Sandbox, name: &str, answers: Value) -> Value {
    let path = sandbox.home().join(format!("{name}.json"));
    std::fs::write(&path, answers.to_string()).unwrap();
    let script = common::fixtures().join("mcp/fake_mcp.py");
    json!({"command": "python3", "args": [script.to_string_lossy(), path.to_string_lossy()]})
}

fn calls(sandbox: &Sandbox, name: &str) -> Vec<Value> {
    std::fs::read_to_string(sandbox.home().join(format!("{name}.json.calls")))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

fn jira_section() -> Value {
    json!({
        "enabled": true, "name": "Jira", "server": "jira", "keys": ["ACME"],
        "tools": {
            "listMine": {"tool": "jira_search", "args": {"jql": "assignee = currentUser()"}},
            "details": {"tool": "jira_issue", "args": {"key": "{ticket}"}},
            "listStates": {"tool": "jira_transitions", "args": {"key": "{ticket}"}},
            "setState": {"tool": "jira_transition", "args": {"key": "{ticket}", "transition": "{state}"}},
            "assign": {"tool": "jira_assign", "args": {"key": "{ticket}", "user": "{assignee}"}},
            "create": {"tool": "jira_create", "args": {"project": "{team}", "summary": "{title}", "description": "{description}"}}
        },
        "fields": {
            "tickets": "issues", "id": "key", "title": "fields.summary", "url": "self", "status": "fields.status.name",
            "statusType": "fields.status.statusCategory.key", "description": "fields.description", "labels": "fields.labels",
            "priority": "fields.priority.name", "assignee": "fields.assignee.displayName", "states": "transitions", "stateId": "id",
            "stateName": "name", "stateType": "to.statusCategory.key", "createdId": "key", "createdUrl": "self"
        },
        "statusTypes": {"new": "unstarted", "indeterminate": "started", "done": "completed"}
    })
}

fn jira_answers() -> Value {
    json!({
        "jira_search": {"issues": [
            {"key": "ACME-7", "self": "https://acme/ACME-7", "fields": {"summary": "Quota banner", "status": {"name": "To deploy", "statusCategory": {"key": "indeterminate"}}}}
        ]},
        "jira_issue": {"key": "ACME-7", "self": "https://acme/ACME-7", "fields": {"summary": "Quota banner", "description": "Show it", "labels": ["front"],
            "priority": {"name": "High"}, "assignee": {"displayName": "Ana"}, "status": {"name": "To deploy", "statusCategory": {"key": "indeterminate"}}}},
        "jira_transitions": {"transitions": [{"id": "31", "name": "Done", "to": {"statusCategory": {"key": "done"}}},
            {"id": "21", "name": "In Review", "to": {"statusCategory": {"key": "indeterminate"}}}]},
        "jira_transition": {"ok": true}, "jira_assign": {"ok": true}, "jira_create": {"key": "ACME-99", "self": "https://acme/ACME-99"}
    })
}

#[test]
fn every_source_reads_through_its_server() {
    let sandbox = Sandbox::new(&json!({}), &json!({}));
    let code = fake(
        &sandbox,
        "gitlab",
        json!({
            "list_mine": {"_list": [{"iid": 4, "title": "Quota banner", "web_url": "https://gitlab.com/acme/api/-/merge_requests/4", "author": {"username": "me"},
                "updated_at": at(-30), "draft": false, "detailed_merge_status": "not_approved", "head_pipeline": {"status": "failed"},
                "source_branch": "acme-7-quota", "target_branch": "main", "project_path": "acme/api"}]},
            "list_reviews": {"_list": []},
            "list_merged": {"_list": [{"iid": 3, "web_url": "https://gitlab.com/acme/api/-/merge_requests/3", "project_path": "acme/api",
                "source_branch": "acme-7-quota", "merge_commit_sha": "abc", "merged_at": at(-120)}]}
        }),
    );
    let ci = fake(
        &sandbox,
        "circleci",
        json!({
            "list_runs": {"runs": [{"id": "r1", "attributes": {"created_at": at(-10), "phase": "ended", "current_outcome": "failed"},
                "references": {"event": {"attributes": {"vcs": {"origin_repository_url": "https://github.com/acme/api", "branch": "acme-7-quota"}}}}}]},
            "list_run_workflows": {"workflows": [{"id": "w1", "attributes": {"name": "build", "created_at": at(-10), "outcome": "failed"}}]},
            "list_workflow_jobs": {"jobs": [{"id": "j1", "attributes": {"name": "test", "phase": "ended", "outcome": "failed"}}]},
            "get_job": {"attributes": {"parallel_executions": [{"steps": [{"name": "mix test", "outcome": "failed", "exit_code": 1}]}]}},
            "list_job_tests": {"tests": [{"name": "quota test"}]},
            "rerun_workflow": {"ok": true}
        }),
    );
    let calendar = fake(
        &sandbox,
        "outlook",
        json!({
            "_versions": ["2025-11-25", "2025-06-18"],
            "list_events": {"value": [{"id": "e1", "subject": "Team daily", "start": at(3), "end": at(20), "webLink": "https://outlook/e1",
                "onlineMeeting": "https://teams/x", "responseStatus": "accepted"}]}
        }),
    );
    let platform = fake(
        &sandbox,
        "platform",
        json!({
            "deploy_status": {"apps": [{"id": "api", "name": "api", "repository": "https://github.com/acme/api", "tag": "v2", "kind": "web",
                "lastDeployedAt": at(-120), "service": "api"}]},
            "deploy_history": {"deployments": [{"workflow": "deploy-1", "namespace": "prod", "tag": "v2", "id": "d1", "phase": "Succeeded",
                "startedAt": at(-120), "finishedAt": at(-115)}]},
            "list_previews": {"sandboxes": [{"stage": "preview-7", "expiresAt": at(2880), "activated": true, "status": {"health": "Healthy"},
                "products": [{"name": "api", "enabled": true, "branch": "acme-7-quota"}]}]},
            "create_preview": {"sandbox": {"stage": "preview-8"}}
        }),
    );
    let grafana = fake(
        &sandbox,
        "grafana",
        json!({
            "query_prometheus": {"_range": true},
            "list_alert_rules": {"rules": [{"uid": "r1", "title": "API 5xx", "state": "firing", "url": "https://grafana/r1"}]},
            "search_issues": {"_list": [{"shortId": "API-1", "title": "KeyError", "count": "14", "firstSeen": at(-60), "permalink": "https://sentry/API-1"}]}
        }),
    );
    let jira = fake(&sandbox, "jira", jira_answers());
    let mail = fake(
        &sandbox,
        "fastmail",
        json!({
            "mail_search": {"_list": [{"id": "m1", "threadId": "t1", "from": "Ana <ana@acme.io>", "subject": "Quota numbers", "snippet": "Here they are",
                "date": at(-5), "unread": true}]},
            "get_thread": {"from": "Ana <ana@acme.io>", "date": at(-5), "html": "<p>Here they are</p>", "body": "Here they are"}
        }),
    );
    let config = json!({
        "github": {"enabled": false},
        "servers": {"gitlab": code, "circleci": ci, "outlook": calendar, "platform": platform, "grafana": grafana, "jira": jira, "fastmail": mail},
        "mail": {"enabled": true, "name": "Fastmail", "server": "fastmail",
            "tools": {"listInbox": {"tool": "mail_search"}, "get": {"tool": "get_thread", "args": {"threadId": "{thread}"}}},
            "fields": {"messages": "", "id": "id", "threadId": "threadId", "from": "from", "subject": "subject", "snippet": "snippet", "date": "date",
                "unread": "unread"}, "urlTemplate": "https://mail.acme.io/{id}"},
        "code": {"name": "GitLab", "server": "gitlab",
            "tools": {"listMine": {"tool": "list_mine"}, "listReviews": {"tool": "list_reviews"}, "listMerged": {"tool": "list_merged"}},
            "fields": {"prs": "", "repo": "project_path", "number": "iid", "title": "title", "url": "web_url", "author": "author.username",
                "updatedAt": "updated_at", "isDraft": "draft", "review": "detailed_merge_status", "ci": "head_pipeline.status",
                "branch": "source_branch", "base": "target_branch", "mergeSha": "merge_commit_sha", "mergedAt": "merged_at"},
            "reviews": {"not_approved": "REVIEW_REQUIRED"}, "checks": {"failed": "FAILURE"}},
        "tickets": jira_section(),
        "ci": {"enabled": true, "name": "CircleCI", "server": "circleci", "imageJob": ""},
        "calendar": {"enabled": true, "name": "Outlook", "server": "outlook", "tools": {"listEvents": {"tool": "list_events"}},
            "fields": {"events": "value", "id": "id", "title": "subject", "start": "start", "end": "end", "url": "webLink", "join": "onlineMeeting",
                "myResponse": "responseStatus"}, "skip": {}, "only": {}, "responses": {}},
        "platform": {"enabled": true, "server": "platform", "login": "me", "previewDashboardUrl": "https://dash/{preview}",
            "tools": {"deployStatus": {"tool": "deploy_status"}, "deployHistory": {"tool": "deploy_history", "args": {"app": "{appId}"}},
                "listPreviews": {"tool": "list_previews", "args": {"owner": "{login}"}}, "createPreview": {"tool": "create_preview", "args": {"app": "{app}", "branch": "{branch}"}}},
            "fields": {"apps": "apps", "appId": "id", "appRepo": "repository", "appTag": "tag", "appKind": "kind", "deployments": "deployments",
                "deployWorkflow": "workflow", "deployNamespace": "namespace", "deployTag": "tag", "deployId": "id", "deployPhase": "phase",
                "deployStarted": "startedAt", "deployFinished": "finishedAt", "previews": "sandboxes", "previewId": "stage", "previewExpires": "expiresAt",
                "previewActive": "activated", "previewHealth": "status.health", "previewApps": "products", "appName": "name", "appEnabled": "enabled",
                "appBranch": "branch", "createdId": "sandbox.stage"}},
        "observability": {"name": "Grafana", "server": "grafana",
            "tools": {"metric": {"tool": "query_prometheus", "args": {"expr": "{query}", "startTime": "{from}", "endTime": "{to}", "stepSeconds": "{step}"}},
                "monitors": {"tool": "list_alert_rules"}, "issues": {"tool": "search_issues"}},
            "fields": {"series": "data.result", "points": "values", "pointTime": "0", "pointValue": "1", "monitors": "rules", "monitorName": "title",
                "monitorStatus": "state", "monitorId": "uid", "monitorUrl": "url", "issues": "", "issueId": "shortId", "issueMessage": "title",
                "issueCount": "count", "issueFirstSeen": "firstSeen", "issueUrl": "permalink"},
            "statuses": {"firing": "alert"},
            "queries": {"errors": "sum(errors{service=\"{service}\"})", "hits": "sum(hits{service=\"{service}\"})", "p95": "histogram_quantile(0.95, x)"}},
        "spaces": ["inbox", "monitor"]
    });
    let path = sandbox.home().join(".config/cockpit/config.json");
    let mut current = read(&path);
    for (k, v) in config.as_object().unwrap() {
        current[k] = v.clone();
    }
    std::fs::write(&path, current.to_string()).unwrap();
    let cache = sandbox.home().join(".cache/cockpit");
    let hub = Hub::start(&sandbox, &[("COCKPIT_HUB_FEED", "on")]);
    hub.request(&json!({"op": "ping"}));

    let inbox = wait_for(&cache.join("inbox.json"), |v| v["hub"]["tickets"].is_number() && v["hub"]["github"].is_number() && v["hub"]["platform"].is_number());
    assert_eq!(inbox["tickets"][0]["id"], "ACME-7");
    assert_eq!(inbox["tickets"][0]["statusType"], "started");
    assert_eq!((&inbox["mine"][0]["number"], &inbox["mine"][0]["ci"], &inbox["mine"][0]["review"]), (&json!(4), &json!("FAILURE"), &json!("REVIEW_REQUIRED")));
    assert_eq!(inbox["merged"][0]["mergeSha"], "abc");
    assert_eq!(inbox["previews"][0]["id"], "preview-7");
    assert_eq!(inbox["errors"], json!([]));

    let ci = wait_for(&cache.join("ci.json"), |v| v["by"] == "hub");
    assert_eq!((&ci["runs"][0]["failed"]["job"], &ci["runs"][0]["failed"]["step"]), (&json!("test"), &json!("mix test")));

    let meetings = wait_for(&cache.join("meetings.json"), |v| v["by"] == "hub");
    assert_eq!((&meetings["meetings"][0]["title"], &meetings["meetings"][0]["join"]), (&json!("Team daily"), &json!("https://teams/x")));

    let mail = wait_for(&cache.join("mail.json"), |v| v["by"] == "hub");
    let message = &mail["messages"][0];
    assert_eq!((&message["fromName"], &message["subject"], &message["url"]), (&json!("Ana"), &json!("Quota numbers"), &json!("https://mail.acme.io/m1")));
    // Its thread is read ahead with the listing, so the mail space opens it at once.
    let ahead = wait_for(&cache.join("mail/m1.json"), |v| v["messages"].as_array().is_some_and(|m| !m.is_empty()));
    assert_eq!((&ahead["messages"][0]["fromName"], ahead["error"].is_null()), (&json!("Ana"), true));

    let monitor = wait_for(&cache.join("monitor.json"), |v| v["by"] == "hub");
    let app = &monitor["apps"][0];
    assert_eq!((&app["monitors"]["alert"], &app["issues"][0]["id"]), (&json!(1), &json!("API-1")));
    assert!(app["releases"][0]["verdict"]["state"].is_string(), "{app}");
}

#[test]
fn requests_and_ticket_actions_go_out_through_the_servers() {
    let sandbox = Sandbox::new(&json!({}), &json!({}));
    let jira = fake(&sandbox, "jira", jira_answers());
    let ci = fake(&sandbox, "circleci", json!({"rerun_workflow": {"ok": true}, "list_runs": {"runs": []}}));
    let mail = fake(
        &sandbox,
        "fastmail",
        json!({
            "mail_search": {"_list": []},
            "get_thread": {"messages": [{"id": "m1", "sender": "Ana <ana@acme.io>", "date": at(-5), "htmlBody": "<p>Here they are</p>", "plaintextBody": "Here they are"}]}
        }),
    );
    let mut config = read(&sandbox.home().join(".config/cockpit/config.json"));
    config["github"] = json!({"enabled": false});
    config["servers"] = json!({"jira": jira, "circleci": ci, "fastmail": mail});
    config["mail"] = json!({"enabled": true, "name": "Fastmail", "server": "fastmail",
        "tools": {"listInbox": {"tool": "mail_search"}, "get": {"tool": "get_thread", "args": {"threadId": "{thread}"}}}});
    config["tickets"] = jira_section();
    config["ci"] = json!({"enabled": true, "name": "CircleCI", "server": "circleci", "imageJob": ""});
    std::fs::write(sandbox.home().join(".config/cockpit/config.json"), config.to_string()).unwrap();
    let cache = sandbox.home().join(".cache/cockpit");
    let hub = Hub::start(&sandbox, &[("COCKPIT_HUB_FEED", "on")]);
    hub.request(&json!({"op": "ping"}));

    let requests = cache.join("requests");
    std::fs::create_dir_all(&requests).unwrap();
    std::fs::write(requests.join("1-rerun.req"), json!({"type": "rerun", "label": "api main", "workflowId": "w1"}).to_string()).unwrap();
    std::fs::write(requests.join("2-rerun.req"), json!({"type": "rerun", "label": "api main"}).to_string()).unwrap();
    let rerun = wait_for(&requests.join("results/1-rerun.req"), |v| v["state"].is_string());
    assert_eq!((&rerun["state"], &rerun["message"]), (&json!("done"), &json!("rerun started")));
    let refused = wait_for(&requests.join("results/2-rerun.req"), |v| v["state"].is_string());
    assert_eq!(refused["state"], "refused", "a rerun names its workflow");
    assert_eq!(calls(&sandbox, "circleci").iter().find(|c| c["tool"] == "rerun_workflow").unwrap()["args"]["workflow"], "w1");

    // A thread asked of a session is read by the hub when it reaches the mail server, and kept where the mail space looks.
    std::fs::write(requests.join("3-mail-get.req"), json!({"type": "mail-get", "label": "Quota numbers", "message": "t1"}).to_string()).unwrap();
    let got = wait_for(&requests.join("results/3-mail-get.req"), |v| v["state"].is_string());
    assert_eq!(got["state"], "done", "{got}");
    let kept = read(&cache.join("mail/t1.json"));
    assert_eq!((&kept["messages"][0]["fromName"], &kept["messages"][0]["html"]), (&json!("Ana"), &json!("<p>Here they are</p>")));
    assert_eq!(calls(&sandbox, "fastmail").iter().find(|c| c["tool"] == "get_thread").unwrap()["args"]["threadId"], "t1");
    // The mail space asks the hub itself, which reads it at once.
    std::fs::remove_file(cache.join("mail/t1.json")).unwrap();
    let asked = hub.request(&json!({"op": "act", "app": "mail", "action": "mail_thread", "args": [{"id": "t1", "threadId": "t1"}], "armed": null, "env": {}}));
    assert_eq!(asked["error"], Value::Null, "{asked}");
    assert_eq!(read(&cache.join("mail/t1.json"))["messages"][0]["text"], "Here they are");

    let act = |action: &str, args: Value| hub.request(&json!({"op": "act", "app": "inbox", "action": action, "args": args, "armed": null, "env": {}}));
    assert_eq!(act("assign", json!(["ACME-7", "Ana"]))["note"], "assigned ACME-7 to Ana");
    assert_eq!(act("file_issue", json!([{"team": "ACME", "title": "Export fails", "description": "500"}]))["note"], "filed ACME-99 https://acme/ACME-99");
    wait_for(&cache.join("inbox.json"), |v| v["tickets"][0]["id"] == "ACME-7");
    assert_eq!(act("set_state", json!(["ACME-7", "In Review"]))["note"], "moved ACME-7 to In Review");
    let inbox = read(&cache.join("inbox.json"));
    assert_eq!(inbox["tickets"][0]["status"], "In Review", "the inbox shows the move at once");
    let card = hub.request(&json!({"op": "fetch", "kind": "ticket_card", "args": {"id": "ACME-7"}}));
    let card = if card["result"].is_object() { card["result"].clone() } else { card };
    assert_eq!((&card["priority"], &card["assignee"], &card["labels"][0]["name"]), (&json!("High"), &json!("Ana"), &json!("front")), "{card}");
    let jira_calls = calls(&sandbox, "jira");
    let tool_args = |tool: &str| jira_calls.iter().find(|c| c["tool"] == tool).map(|c| c["args"].clone()).unwrap_or(Value::Null);
    assert_eq!(tool_args("jira_transition"), json!({"key": "ACME-7", "transition": "21"}));
    assert_eq!(tool_args("jira_create")["project"], "ACME");
}
