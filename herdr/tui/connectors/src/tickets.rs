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
            "setState": {"tool": "save_issue", "args": {"id": "{ticket}", "state": "{state}"}},
            "assign": {"tool": "save_issue", "args": {"id": "{ticket}", "assignee": "{assignee}"}},
            "create": {"tool": "save_issue", "args": {"team": "{team}", "title": "{title}", "description": "{description}", "assignee": "me"}},
            "details": {"tool": "get_issue", "args": {"id": "{ticket}", "fields": ["id", "title", "url", "description", "labels", "priority", "assignee", "status", "statusType", "gitBranchName"]}}
        },
        "fields": {
            "tickets": "issues", "id": "id", "title": "title", "url": "url", "status": "status", "statusType": "statusType", "parentId": "parentId",
            "project": "project", "description": "description", "attachments": "attachments", "attachmentUrl": "url", "states": "", "stateId": "id",
            "stateName": "name", "stateType": "type", "labels": "labels", "labelName": "name", "priority": "priority.name|priority",
            "assignee": "assignee.name|assignee", "branch": "gitBranchName", "createdId": "id|identifier", "createdUrl": "url"
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

/// Moves a ticket to the state whose name matches `name` (case aside, else the first that starts with it); its name and
/// category.
pub async fn set_state(tools: &impl Tools, mapping: &Value, ticket: &str, name: &str) -> Result<(String, String), String> {
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
    Ok((to["name"].as_str().unwrap_or(name).to_string(), to["type"].as_str().unwrap_or_default().to_string()))
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

/// Assigns a ticket to `who`: "me", or a person the tool knows by name or email.
pub async fn assign(tools: &impl Tools, ticket: &str, who: &str) -> Result<(), String> {
    tools.call("assign", &json!({"ticket": ticket, "assignee": who, "team": team_of(ticket)})).await.map(|_| ())
}

/// Files a ticket in a team, assigned to me as the mapping says; its id and URL.
pub async fn create(tools: &impl Tools, mapping: &Value, team: &str, title: &str, description: &str) -> Result<(String, String), String> {
    let answer = tools.call("create", &json!({"team": team, "title": title, "description": description})).await?;
    let f = |name: &str| mapping["fields"][name].as_str().unwrap_or("").to_string();
    let (id, url) = (text(pick(&answer, &f("createdId"))).unwrap_or_default(), text(pick(&answer, &f("createdUrl"))).unwrap_or_default());
    if id.is_empty() && url.is_empty() {
        return Err(format!("the tool answered no ticket: {}", answer.to_string().chars().take(160).collect::<String>()));
    }
    Ok((id, url))
}

/// A ticket's card: description, labels, priority, assignee, state, branch, and the states it can move to in their
/// categories' order.
pub async fn details(tools: &impl Tools, mapping: &Value, ticket: &str) -> Result<Value, String> {
    let verb = if tools.has("details") { "details" } else { "get" };
    let raw = tools.call(verb, &json!({"ticket": ticket, "team": team_of(ticket)})).await?;
    let f = |name: &str| mapping["fields"][name].as_str().unwrap_or("").to_string();
    let get = |name: &str| text(pick(&raw, &f(name)));
    let labels: Vec<Value> = list_at(&raw, &f("labels"))
        .into_iter()
        .filter_map(|l| if l.is_string() { text(Some(l)) } else { text(pick(l, &f("labelName"))) })
        .map(|name| json!({"name": name}))
        .collect();
    let mut states = if tools.has("listStates") { states(tools, mapping, ticket).await.unwrap_or_default() } else { Vec::new() };
    let order = |s: &Value| ["triage", "backlog", "unstarted", "started", "completed", "canceled"].iter().position(|t| s["type"] == *t).unwrap_or(9);
    states.sort_by_key(order);
    let raw_type = get("statusType").unwrap_or_default();
    let kind = mapping["statusTypes"][&raw_type].as_str().map(str::to_string).unwrap_or(raw_type);
    Ok(json!({
        "id": get("id").unwrap_or_else(|| ticket.to_string()),
        "title": get("title").unwrap_or_default(),
        "url": get("url").unwrap_or_default(),
        "description": get("description").unwrap_or_default().chars().take(1200).collect::<String>(),
        "labels": labels,
        "priority": get("priority"),
        "assignee": get("assignee"),
        "state": {"name": get("status").unwrap_or_default(), "type": kind},
        "branch": get("branch"),
        "states": states.iter().map(|s| json!({"name": s["name"], "type": s["type"]})).collect::<Vec<_>>(),
    }))
}

fn team_of(ticket: &str) -> &str {
    ticket.split('-').next().unwrap_or_default()
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
    fn a_ticket_is_assigned_filed_and_detailed() {
        let m = mapping(&json!({}));
        let fake = Fake::default().with("assign", json!({})).with("create", json!({"id": "SOU-9", "url": "https://linear.app/x/SOU-9"}));
        crate::block_on(assign(&fake, "SOU-1", "me")).unwrap();
        assert_eq!(crate::block_on(create(&fake, &m, "SOU", "T", "D")).unwrap(), ("SOU-9".into(), "https://linear.app/x/SOU-9".into()));
        assert_eq!(fake.calls.borrow()[0].1["assignee"], "me");
        assert_eq!(fake.calls.borrow()[1].1, json!({"team": "SOU", "title": "T", "description": "D"}));
        let issue = json!({"id": "SOU-1", "title": "T", "url": "u", "description": "d", "labels": ["Back-end"], "priority": {"name": "Medium", "value": 3},
            "assignee": "Ana", "status": "To Review", "statusType": "started", "gitBranchName": "sou-1-t"});
        let states = json!([{"id": "2", "name": "Done", "type": "completed"}, {"id": "1", "name": "Todo", "type": "unstarted"}]);
        let fake = Fake::default().with("details", issue).with("listStates", states);
        let card = crate::block_on(details(&fake, &m, "SOU-1")).unwrap();
        assert_eq!(card["labels"], json!([{"name": "Back-end"}]));
        assert_eq!((&card["priority"], &card["assignee"], &card["branch"]), (&json!("Medium"), &json!("Ana"), &json!("sou-1-t")));
        assert_eq!(card["state"], json!({"name": "To Review", "type": "started"}));
        assert_eq!(card["states"][0]["name"], "Todo");
        assert!(crate::block_on(create(&Fake::default().with("create", json!({})), &m, "SOU", "T", "")).is_err());
    }

    #[test]
    fn a_ticket_moves_to_its_done_state() {
        let states = json!([{"id": "s1", "name": "In Progress", "type": "started"}, {"id": "s2", "name": "Shipped", "type": "completed"},
            {"id": "s3", "name": "Done", "type": "completed"}]);
        let fake = Fake::default().with("listStates", states.clone()).with("setState", json!({}));
        assert_eq!(crate::block_on(complete(&fake, &mapping(&json!({})), "ENG-1")).unwrap(), "Done");
        assert_eq!(fake.calls.borrow()[1].1["state"], "s3");
        let fake = Fake::default().with("listStates", states).with("setState", json!({}));
        assert_eq!(crate::block_on(set_state(&fake, &mapping(&json!({})), "ENG-1", "in prog")).unwrap(), ("In Progress".to_string(), "started".to_string()));
    }
}
