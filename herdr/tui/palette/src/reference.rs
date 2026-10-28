//! What a found item reads as inside a Claude session: enough for the session to act on it without asking which one.

use cockpit_core::hub::Found;
use serde_json::Value;

/// A PR's URL, a ticket's id and URL, a release with its environment and verdict, an event with its time; anything
/// else its title, with its help when that says more.
pub fn reference(found: &Found, inbox: &Value, config: &Value) -> String {
    let key = found.target["key"].as_str().unwrap_or_default();
    match (found.app.as_str(), found.kind.as_str()) {
        (_, "changed") => format!("{} ({})", found.title, when(&found.target["at"]).unwrap_or_else(|| found.help.clone())),
        ("inbox", "pr" | "review") if !key.is_empty() => key.to_string(),
        ("inbox", "ticket") => ticket(key, inbox, config),
        ("monitor", "release" | "app") => monitored(found, key),
        ("session", _) => format!("the past Claude session \"{}\" ({key})", found.title),
        ("todo", _) => format!("the to-do \"{}\"", found.title),
        ("runbook", "runbook") => format!("the runbook \"{}\"", found.title),
        ("runbook", "step") => format!("the runbook step \"{}\" ({})", found.title, found.help),
        ("meetings", _) => format!("the meeting \"{}\" ({})", found.title, found.help),
        _ if found.help.is_empty() => found.title.clone(),
        _ => format!("{} ({})", found.title, found.help),
    }
}

/// An event's time in epoch milliseconds, as "Wed 28 Oct 14:05" in local time.
fn when(at: &Value) -> Option<String> {
    let at = chrono::DateTime::from_timestamp_millis(at.as_i64()?)?;
    Some(at.with_timezone(&chrono::Local).format("%a %d %b %H:%M").to_string())
}

/// "ENG-401 (https://…/ENG-401)": the URL the inbox knows, else the config's template's.
fn ticket(id: &str, inbox: &Value, config: &Value) -> String {
    let known = inbox["tickets"].as_array().into_iter().flatten().find(|t| t["id"].as_str().is_some_and(|i| i.eq_ignore_ascii_case(id)));
    let url = known.and_then(|t| t["url"].as_str()).map(str::to_string).or_else(|| {
        let template = config["tickets"]["urlTemplate"].as_str().filter(|t| t.contains("{id}"))?;
        Some(template.replace("{id}", &id.to_uppercase()))
    });
    match url.filter(|u| !u.is_empty()) {
        Some(url) => format!("{id} ({url})"),
        None => id.to_string(),
    }
}

/// "api v2 (production, moved: errors 0.1% → 2%)" for a release, "api (production, v2: Healthy)" for an app: the
/// environment is what the app's id adds to its name.
fn monitored(found: &Found, key: &str) -> String {
    let name = found.title.split(' ').next().unwrap_or_default();
    let app = key.split("/release/").next().unwrap_or_default();
    let env = app.strip_prefix(&format!("{name}-")).unwrap_or(app);
    let verdict = match found.help.split_once(" · ") {
        Some((state, summary)) => format!("{state}: {summary}"),
        None => found.help.clone(),
    };
    let about: Vec<&str> = [env, verdict.as_str()].into_iter().filter(|s| !s.is_empty()).collect();
    if about.is_empty() { found.title.clone() } else { format!("{} ({})", found.title, about.join(", ")) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn found(app: &str, kind: &str, title: &str, help: &str, target: Value) -> Found {
        Found { app: app.into(), kind: kind.into(), title: title.into(), help: help.into(), target }
    }

    #[test]
    fn a_pr_is_its_url_and_a_ticket_its_id_with_its_url() {
        let inbox = json!({"tickets": [{"id": "ENG-401", "url": "https://linear.app/acme/issue/ENG-401/sign-in"}]});
        let pr = found("inbox", "review", "web#9 Dark mode", "review for you · Maya", json!({"app": "inbox", "key": "https://github.com/acme/web/pull/9"}));
        assert_eq!(reference(&pr, &inbox, &Value::Null), "https://github.com/acme/web/pull/9");
        let assigned = found("inbox", "ticket", "ENG-401 Sign in with Google", "In Progress", json!({"app": "inbox", "key": "ENG-401"}));
        assert_eq!(reference(&assigned, &inbox, &Value::Null), "ENG-401 (https://linear.app/acme/issue/ENG-401/sign-in)");
        let other = found("inbox", "ticket", "ENG-7 Billing", "", json!({"app": "inbox", "key": "eng-7"}));
        let config = json!({"tickets": {"urlTemplate": "https://linear.app/acme/issue/{id}"}});
        assert_eq!(reference(&other, &inbox, &config), "eng-7 (https://linear.app/acme/issue/ENG-7)");
        assert_eq!(reference(&other, &inbox, &Value::Null), "eng-7");
    }

    #[test]
    fn a_release_says_its_environment_and_verdict() {
        let target = json!({"app": "monitor", "key": "api-production/release/2"});
        let moved = found("monitor", "release", "api v2", "moved · errors 0.1% → 2%", target.clone());
        assert_eq!(reference(&moved, &Value::Null, &Value::Null), "api v2 (production, moved: errors 0.1% → 2%)");
        assert_eq!(reference(&found("monitor", "release", "api v2", "", target), &Value::Null, &Value::Null), "api v2 (production)");
        let app = found("monitor", "app", "api", "v2 · Healthy", json!({"app": "monitor", "key": "api-production"}));
        assert_eq!(reference(&app, &Value::Null, &Value::Null), "api (production, v2: Healthy)");
    }

    #[test]
    fn an_event_says_when_it_happened() {
        let at = 1_761_660_000_000_i64;
        let local = chrono::DateTime::from_timestamp_millis(at).unwrap().with_timezone(&chrono::Local).format("%a %d %b %H:%M").to_string();
        let event = found("inbox", "changed", "🎫 ENG-412: To review → Done", "3 h ago", json!({"app": "inbox", "key": "ENG-412", "at": at}));
        assert_eq!(reference(&event, &Value::Null, &Value::Null), format!("🎫 ENG-412: To review → Done ({local})"));
        let undated = found("inbox", "changed", "🟢 CI passing again: api#4", "5 min ago", json!({"app": "inbox", "key": "x"}));
        assert_eq!(reference(&undated, &Value::Null, &Value::Null), "🟢 CI passing again: api#4 (5 min ago)");
    }

    #[test]
    fn the_rest_by_title() {
        let none = Value::Null;
        let session = found("session", "session", "Quota limits", "2 d ago · api", json!({"app": "session", "key": "4f1c-77"}));
        assert_eq!(reference(&session, &none, &none), "the past Claude session \"Quota limits\" (4f1c-77)");
        let meeting = found("meetings", "meeting", "Team daily", "Wed 28 Oct 09:45", json!({"app": "meetings", "key": "m1"}));
        assert_eq!(reference(&meeting, &none, &none), "the meeting \"Team daily\" (Wed 28 Oct 09:45)");
        let view = found("show", "view", "Quotas demo", "", json!({"app": "show", "key": "demo"}));
        assert_eq!(reference(&view, &none, &none), "Quotas demo");
    }
}
