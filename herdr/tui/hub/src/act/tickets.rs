//! Tickets changed from the inbox: a state, an assignee, a new ticket, and a ticket's card, through Linear's GraphQL API
//! when the config gives `tickets.token`, else through the hub's own ticket server; otherwise an agent session in the
//! actions space makes the change with the ticket connector.

use cockpit_core::files;
use serde_json::{Value, json};

use super::{Ctx, arg, inbox, opt};
use crate::feed::http;
use crate::value::{Res, list, text};

/// Linear's token, when tickets are Linear's and the config holds one.
pub fn token() -> Option<String> {
    let raw = files::load(&files::config_dir().join("config.json")).unwrap_or(Value::Null);
    let tickets = &raw["tickets"];
    let names = format!("{} {}", tickets["name"].as_str().unwrap_or("Linear"), tickets["server"].as_str().unwrap_or_default()).to_lowercase();
    tickets["token"].as_str().map(str::trim).filter(|t| !t.is_empty() && tickets["enabled"] != false && names.contains("linear")).map(str::to_string)
}

fn gql(token: &str, query: &str, variables: Value) -> Res<Value> {
    let api = http::base("COCKPIT_LINEAR_API", "https://api.linear.app");
    let body = http::post(&format!("{api}/graphql"), &[("Authorization", token)], &json!({"query": query, "variables": variables})).map_err(|e| match e {
        http::Failure::Limited(_) => "Linear's rate limit: try again in a minute".to_string(),
        http::Failure::Other(m) => m,
    })?;
    if let Some(error) = body["errors"][0]["message"].as_str() {
        return Err(error.to_string());
    }
    Ok(body["data"].clone())
}

const ISSUE: &str = "query($id: String!) { issue(id: $id) { id identifier title url description priorityLabel branchName
  state { id name type } assignee { id displayName isMe } labels { nodes { name color } }
  team { id key states { nodes { id name type position } } } } }";

/// The issue as Linear has it now; None without a token.
pub fn issue(id: &str) -> Res<Option<Value>> {
    let Some(token) = token() else { return Ok(None) };
    let data = gql(&token, ISSUE, json!({"id": id}))?;
    if data["issue"].is_null() {
        return Err(format!("Linear has no issue {id}"));
    }
    Ok(Some(data["issue"].clone()))
}

fn viewer(token: &str) -> Res<String> {
    let data = gql(token, "query { viewer { id } }", json!({}))?;
    data["viewer"]["id"].as_str().map(str::to_string).ok_or_else(|| "Linear does not say who you are".into())
}

/// The team's state for a name: the same name, else the first of the category the name means.
pub fn state_for<'a>(states: &'a [Value], name: &str) -> Option<&'a Value> {
    let lower = name.to_lowercase();
    if let Some(found) = states.iter().find(|s| s["name"].as_str().is_some_and(|n| n.to_lowercase() == lower)) {
        return Some(found);
    }
    let kind = match lower.as_str() {
        "done" | "completed" => "completed",
        "todo" | "to do" => "unstarted",
        "backlog" => "backlog",
        "canceled" | "cancelled" => "canceled",
        _ => "started",
    };
    let mut fitting: Vec<&Value> = states.iter().filter(|s| s["type"] == kind).collect();
    fitting.sort_by(|a, b| a["position"].as_f64().unwrap_or(0.0).total_cmp(&b["position"].as_f64().unwrap_or(0.0)));
    if kind == "started" && lower.contains("review") {
        return fitting.iter().copied().find(|s| s["name"].as_str().is_some_and(|n| n.to_lowercase().contains("review")));
    }
    fitting.first().copied()
}

fn update(token: &str, issue_id: &str, input: Value) -> Res {
    let data = gql(
        token,
        "mutation($id: String!, $input: IssueUpdateInput!) { issueUpdate(id: $id, input: $input) { success } }",
        json!({"id": issue_id, "input": input}),
    )?;
    if data["issueUpdate"]["success"] == true { Ok(()) } else { Err("Linear refused the change".into()) }
}

fn refresh() {
    let _ = std::fs::write(files::cache().join("refresh-request"), crate::value::now().to_string());
}

/// Moves an issue Linear returned to In Progress and assigns it to you, as far as each is needed; what changed.
pub fn take(id: &str, issue: &serde_json::Map<String, Value>) -> Res<String> {
    let Some(token) = token() else { return Ok(String::new()) };
    let mut input = serde_json::Map::new();
    let mut done = Vec::new();
    let state = issue.get("state").cloned().unwrap_or(Value::Null);
    if !matches!(state["type"].as_str(), Some("started" | "completed" | "canceled"))
        && let Some(to) = state_for(list(&issue["team"]["states"]["nodes"]), "In Progress")
    {
        input.insert("stateId".into(), to["id"].clone());
        done.push(format!("moved to {}", text(&to["name"])));
    }
    if issue.get("assignee").is_none_or(Value::is_null) {
        input.insert("assigneeId".into(), json!(viewer(&token)?));
        done.push("assigned to you".into());
    }
    if input.is_empty() {
        return Ok(String::new());
    }
    update(&token, issue.get("id").and_then(Value::as_str).unwrap_or(id), Value::Object(input))?;
    refresh();
    Ok(format!("{id} {}", done.join(", ")))
}

fn ticket_id(v: &Value) -> String {
    v["id"].as_str().or(v.as_str()).unwrap_or_default().to_uppercase()
}

/// The hub's own ticket server and the tickets mapping, when there is no Linear token.
fn served() -> Option<(cockpit_connectors::Server, Value)> {
    if token().is_some() {
        return None;
    }
    let raw = files::load(&files::config_dir().join("config.json")).unwrap_or(Value::Null);
    let config = crate::config::load();
    let server = crate::feed::mcp::server(&raw, &config.0, "tickets")?;
    Some((server, cockpit_connectors::tickets::mapping(&raw["tickets"])))
}

/// A change the ticket connector makes, in an agent session of the actions space.
fn by_session(ctx: &mut Ctx, id: &str, what: &str) {
    let name = ctx.cfg.0["tickets"]["name"].as_str().unwrap_or("the ticket tool").to_string();
    let prompt = format!("{what} in {name}, then say it is done in one line. Change nothing else.");
    let repos = ctx.cfg.repos();
    inbox::start_action(ctx, &repos, &format!("ticket {}", id.to_lowercase()), &prompt, &inbox::Launch::new());
}

/// `set_state [ticket, state name]`.
pub fn set_state(ctx: &mut Ctx, args: &[Value]) -> Res {
    let id = ticket_id(arg(args, 0)?);
    let name = text(arg(args, 1)?);
    if let Some((server, mapping)) = served() {
        use cockpit_connectors::{tickets, tools::Mapped};
        let moved = crate::feed::mcp::with(&server, async |c| tickets::set_state(&Mapped { connection: c, section: &mapping }, &mapping, &id, &name).await)?;
        refresh();
        ctx.note(format!("moved {id} to {moved}"));
        return Ok(());
    }
    let Some(issue) = issue(&id)? else {
        by_session(ctx, &id, &format!("Move {id} to the state \"{name}\""));
        return Ok(());
    };
    let to = state_for(list(&issue["team"]["states"]["nodes"]), &name).ok_or_else(|| format!("{} has no state like {name:?}", text(&issue["team"]["key"])))?;
    if to["id"] == issue["state"]["id"] {
        ctx.note(format!("{id} is {} already", text(&to["name"])));
        return Ok(());
    }
    let token = token().unwrap_or_default();
    update(&token, issue["id"].as_str().unwrap_or(&id), json!({"stateId": to["id"]}))?;
    refresh();
    ctx.note(format!("moved {id} to {}", text(&to["name"])));
    Ok(())
}

/// `assign [ticket, "me" | name]`.
pub fn assign(ctx: &mut Ctx, args: &[Value]) -> Res {
    let id = ticket_id(arg(args, 0)?);
    let who = opt(args, 1).as_str().filter(|w| !w.is_empty()).unwrap_or("me").to_string();
    if let Some((server, mapping)) = served().filter(|(_, m)| m["tools"]["assign"]["tool"].is_string()) {
        use cockpit_connectors::{tickets, tools::Mapped};
        crate::feed::mcp::with(&server, async |c| tickets::assign(&Mapped { connection: c, section: &mapping }, &id, &who).await)?;
        refresh();
        ctx.note(format!("assigned {id} to {}", if who == "me" { "you" } else { &who }));
        return Ok(());
    }
    let Some(token) = token() else {
        by_session(ctx, &id, &format!("Assign {id} to {who}"));
        return Ok(());
    };
    let issue = issue(&id)?.ok_or("no Linear token")?;
    let user = if who == "me" {
        viewer(&token)?
    } else {
        let data = gql(
            &token,
            "query($q: String!) { users(filter: { or: [{ displayName: { eqIgnoreCase: $q } }, { name: { eqIgnoreCase: $q } }, { email: { eqIgnoreCase: $q } }] }) { nodes { id } } }",
            json!({"q": who}),
        )?;
        data["users"]["nodes"][0]["id"].as_str().map(str::to_string).ok_or_else(|| format!("Linear knows no one called {who}"))?
    };
    update(&token, issue["id"].as_str().unwrap_or(&id), json!({"assigneeId": user}))?;
    refresh();
    ctx.note(format!("assigned {id} to {}", if who == "me" { "you" } else { &who }));
    Ok(())
}

/// `file_issue [{team, title, description}]`: a new issue assigned to you; its URL ends the note.
pub fn file_issue(ctx: &mut Ctx, args: &[Value]) -> Res {
    let spec = arg(args, 0)?;
    let title = spec["title"].as_str().map(str::trim).filter(|t| !t.is_empty()).ok_or("an issue needs a title")?.to_string();
    let keys: Vec<String> = list(&ctx.cfg.0["tickets"]["keys"]).iter().map(text).collect();
    let team = spec["team"].as_str().filter(|t| !t.is_empty()).map(str::to_uppercase).or_else(|| keys.first().cloned()).ok_or("say which team files it")?;
    let description = spec["description"].as_str().unwrap_or_default().to_string();
    if let Some((server, mapping)) = served().filter(|(_, m)| m["tools"]["create"]["tool"].is_string()) {
        use cockpit_connectors::{tickets, tools::Mapped};
        let (made, url) = crate::feed::mcp::with(&server, async |c| {
            tickets::create(&Mapped { connection: c, section: &mapping }, &mapping, &team, &title, &description).await
        })?;
        refresh();
        ctx.note(format!("filed {made} {url}").trim().to_string());
        return Ok(());
    }
    let Some(token) = token() else {
        let what = format!(
            "File a new issue in the team {team}, assigned to me, titled \"{title}\"{}",
            if description.is_empty() { String::new() } else { format!(", described as: {description}") }
        );
        by_session(ctx, &team, &what);
        return Ok(());
    };
    let teams = gql(&token, "query($key: String!) { teams(filter: { key: { eq: $key } }) { nodes { id } } }", json!({"key": team}))?;
    let team_id = teams["teams"]["nodes"][0]["id"].as_str().ok_or_else(|| format!("Linear has no team {team}"))?.to_string();
    let me = viewer(&token)?;
    let input = json!({"teamId": team_id, "title": title, "description": description, "assigneeId": me});
    let made = gql(&token, "mutation($input: IssueCreateInput!) { issueCreate(input: $input) { success issue { identifier url } } }", json!({"input": input}))?;
    let issue = &made["issueCreate"]["issue"];
    refresh();
    ctx.note(format!("filed {} {}", text(&issue["identifier"]), text(&issue["url"])));
    Ok(())
}

/// The `ticket_card` details: description, labels, priority, assignee, the team's states.
pub fn card(args: &Value) -> Res<Value> {
    let id = ticket_id(&args["id"]);
    if let Some((server, mapping)) = served() {
        use cockpit_connectors::{tickets, tools::Mapped};
        let mut card = crate::feed::mcp::with(&server, async |c| tickets::details(&Mapped { connection: c, section: &mapping }, &mapping, &id).await)?;
        let mine = list(&crate::store::global().get("inbox")["tickets"]).iter().any(|t| t["id"] == id.as_str());
        card["mine"] = mine.into();
        return Ok(card);
    }
    let Some(issue) = issue(&id)? else {
        return Err("no ticket server nor Linear token: the card shows what the inbox knows".into());
    };
    let mut states: Vec<Value> = list(&issue["team"]["states"]["nodes"]).to_vec();
    let order = |t: &str| ["triage", "backlog", "unstarted", "started", "completed", "canceled"].iter().position(|x| *x == t).unwrap_or(9);
    states.sort_by(|a, b| {
        order(a["type"].as_str().unwrap_or_default())
            .cmp(&order(b["type"].as_str().unwrap_or_default()))
            .then(a["position"].as_f64().unwrap_or(0.0).total_cmp(&b["position"].as_f64().unwrap_or(0.0)))
    });
    let description: String = issue["description"].as_str().unwrap_or_default().chars().take(1200).collect();
    Ok(json!({
        "id": issue["identifier"], "title": issue["title"], "url": issue["url"], "description": description,
        "labels": issue["labels"]["nodes"], "priority": issue["priorityLabel"], "assignee": issue["assignee"]["displayName"],
        "mine": issue["assignee"]["isMe"], "state": issue["state"], "branch": issue["branchName"],
        "states": states.iter().map(|s| json!({"name": s["name"], "type": s["type"]})).collect::<Vec<_>>(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_state_by_its_name_else_its_category() {
        let states = [
            json!({"id": "1", "name": "Todo", "type": "unstarted", "position": 0}),
            json!({"id": "2", "name": "In Progress", "type": "started", "position": 1}),
            json!({"id": "3", "name": "In Review", "type": "started", "position": 2}),
            json!({"id": "4", "name": "Shipped", "type": "completed", "position": 3}),
        ];
        let id = |name: &str| state_for(&states, name).map(|s| s["id"].as_str().unwrap().to_string());
        assert_eq!(id("in review").as_deref(), Some("3"));
        assert_eq!(id("Done").as_deref(), Some("4"));
        assert_eq!(id("Doing").as_deref(), Some("2"));
        assert_eq!(id("Backlog"), None);
    }
}
