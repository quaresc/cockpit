//! My open tickets through a ticket connector's mapping, the hub's twin of the plugin's `myTickets` (tickets.ts).

use serde_json::{Value, json};

use crate::mapping::{list_at, pick, text};
use crate::tools::Tools;

/// Cockpit's status types are Linear's categories; the open ones, in the order the inbox lists them.
const ACTIVE: [&str; 4] = ["started", "unstarted", "triage", "backlog"];

/// The Linear connector's mapping, the default for a config that names no other (config.ts `LINEAR`).
pub fn linear() -> Value {
    json!({
        "tools": {
            "listMine": {"tool": "list_issues", "args": {"assignee": "me", "limit": 100, "fields": ["id", "title", "url", "status", "statusType", "parentId", "project"]}},
            "get": {"tool": "get_issue", "args": {"id": "{ticket}", "fields": ["id", "title", "url", "description", "attachments"]}},
            "comments": {"tool": "list_comments", "args": {"issueId": "{ticket}"}},
            "listStates": {"tool": "list_issue_statuses", "args": {"team": "{team}"}},
            "setState": {"tool": "save_issue", "args": {"id": "{ticket}", "state": "{state}"}}
        },
        "fields": {
            "tickets": "issues", "id": "id", "title": "title", "url": "url", "status": "status", "statusType": "statusType", "parentId": "parentId",
            "project": "project", "description": "description", "attachments": "attachments", "attachmentUrl": "url", "states": "", "stateId": "id",
            "stateName": "name", "stateType": "type"
        },
        "statusTypes": {}
    })
}

/// The config's tickets block over the Linear default, as the plugin's loadConfig merges it.
pub fn mapping(tickets: &Value) -> Value {
    let mut merged = linear();
    if tickets["tools"].as_object().is_some_and(|t| !t.is_empty()) {
        merged["tools"] = tickets["tools"].clone();
    }
    for key in ["fields", "statusTypes"] {
        if let (Some(base), Some(over)) = (merged[key].as_object_mut(), tickets[key].as_object()) {
            base.extend(over.clone());
        }
    }
    merged["urlTemplate"] = tickets["urlTemplate"].clone();
    merged
}

/// My open tickets, in progress first, in the inbox's `Ticket` shape.
pub async fn mine(tools: &impl Tools, mapping: &Value) -> Result<Vec<Value>, String> {
    let answer = tools.call("listMine", &json!({})).await?;
    let f = |name: &str| mapping["fields"][name].as_str().unwrap_or("").to_string();
    let status_type = |raw: Option<String>| {
        let raw = raw.unwrap_or_default();
        mapping["statusTypes"][&raw].as_str().map(str::to_string).unwrap_or(raw)
    };
    let url_template = mapping["urlTemplate"].as_str().unwrap_or("");
    let mut tickets: Vec<Value> = list_at(&answer, &f("tickets"))
        .into_iter()
        .map(|raw| {
            let id = text(pick(raw, &f("id"))).unwrap_or_default();
            let url = text(pick(raw, &f("url"))).unwrap_or_else(|| url_template.replace("{id}", &id));
            json!({
                "id": id,
                "title": text(pick(raw, &f("title"))).unwrap_or_default(),
                "url": url,
                "status": text(pick(raw, &f("status"))).unwrap_or_default(),
                "statusType": status_type(text(pick(raw, &f("statusType")))),
                "parentId": text(pick(raw, &f("parentId"))),
                "project": text(pick(raw, &f("project"))),
            })
        })
        .filter(|t| rank(t).is_some())
        .collect();
    tickets.sort_by_key(|t| rank(t).unwrap_or(usize::MAX));
    Ok(tickets)
}

/// The states a ticket can move to, as `{id, name, type}` with Linear's categories as types.
pub async fn states(tools: &impl Tools, mapping: &Value, ticket: &str) -> Result<Vec<Value>, String> {
    let team = ticket.split('-').next().unwrap_or_default();
    let answer = tools.call("listStates", &json!({"ticket": ticket, "team": team})).await?;
    let f = |name: &str| mapping["fields"][name].as_str().unwrap_or("").to_string();
    Ok(list_at(&answer, &f("states"))
        .into_iter()
        .map(|s| {
            let raw = text(pick(s, &f("stateType"))).unwrap_or_default();
            let kind = mapping["statusTypes"][&raw].as_str().map(str::to_string).unwrap_or(raw);
            json!({"id": text(pick(s, &f("stateId"))).unwrap_or_default(), "name": text(pick(s, &f("stateName"))).unwrap_or_default(), "type": kind})
        })
        .collect())
}

/// Moves a ticket to the state whose name matches `name` (case aside, else the first that starts with it); its name.
pub async fn set_state(tools: &impl Tools, mapping: &Value, ticket: &str, name: &str) -> Result<String, String> {
    let states = states(tools, mapping, ticket).await?;
    let wanted = name.to_lowercase();
    let named = |s: &&Value| s["name"].as_str().unwrap_or_default().to_lowercase();
    let to = states
        .iter()
        .find(|s| named(s) == wanted)
        .or_else(|| states.iter().find(|s| named(s).starts_with(&wanted)))
        .ok_or_else(|| format!("{ticket} has no state like {name:?}"))?;
    let team = ticket.split('-').next().unwrap_or_default();
    tools.call("setState", &json!({"ticket": ticket, "team": team, "state": to["id"]})).await?;
    Ok(to["name"].as_str().unwrap_or(name).to_string())
}

/// Moves a ticket to its "Done" state, else the first completed one; its name.
pub async fn complete(tools: &impl Tools, mapping: &Value, ticket: &str) -> Result<String, String> {
    let states = states(tools, mapping, ticket).await?;
    let completed = |s: &&Value| s["type"] == "completed";
    let done = states
        .iter()
        .find(|s| completed(s) && s["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case("done")))
        .or_else(|| states.iter().find(completed))
        .ok_or_else(|| format!("no completed state for {ticket}"))?;
    let team = ticket.split('-').next().unwrap_or_default();
    tools.call("setState", &json!({"ticket": ticket, "team": team, "state": done["id"]})).await?;
    Ok(done["name"].as_str().unwrap_or("Done").to_string())
}

fn rank(ticket: &Value) -> Option<usize> {
    ACTIVE.iter().position(|s| ticket["statusType"] == *s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::Fake;

    #[test]
    fn open_tickets_in_progress_first() {
        let answer = json!({"issues": [
            {"id": "ENG-2", "title": "B", "url": "u2", "status": "Todo", "statusType": "unstarted"},
            {"id": "ENG-1", "title": "A", "url": "u1", "status": "In Progress", "statusType": "started", "project": "P"},
            {"id": "ENG-3", "title": "C", "url": "u3", "status": "Done", "statusType": "completed"}
        ]});
        let fake = Fake::default().with("listMine", answer);
        let tickets = crate::block_on(mine(&fake, &mapping(&json!({})))).unwrap();
        let ids: Vec<&str> = tickets.iter().map(|t| t["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["ENG-1", "ENG-2"]);
        assert_eq!(tickets[0]["project"], "P");
        assert_eq!(tickets[1]["parentId"], Value::Null);
    }

    #[test]
    fn a_ticket_moves_to_its_done_state() {
        let states = json!([{"id": "s1", "name": "In Progress", "type": "started"}, {"id": "s2", "name": "Shipped", "type": "completed"},
            {"id": "s3", "name": "Done", "type": "completed"}]);
        let fake = Fake::default().with("listStates", states.clone()).with("setState", json!({}));
        assert_eq!(crate::block_on(complete(&fake, &mapping(&json!({})), "ENG-1")).unwrap(), "Done");
        assert_eq!(fake.calls.borrow()[1].1["state"], "s3");
        let fake = Fake::default().with("listStates", states).with("setState", json!({}));
        assert_eq!(crate::block_on(set_state(&fake, &mapping(&json!({})), "ENG-1", "in prog")).unwrap(), "In Progress");
    }
}
