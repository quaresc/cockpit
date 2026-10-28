//! My open Linear tickets through its GraphQL API, given `tickets.token`: the same fields, order and filter as the
//! mod's mapping of the Linear connector gives the inbox.

use serde_json::{Value, json};

use super::http::{self, Failure, Fetched};
use crate::value::list;

/// Linear's categories the inbox lists, in its order.
const ACTIVE: [&str; 4] = ["started", "unstarted", "triage", "backlog"];

const QUERY: &str = r#"query { viewer { assignedIssues(first: 100, orderBy: updatedAt, filter: { state: { type: { in: ["started", "unstarted", "triage", "backlog"] } } }) {
  nodes { identifier title url state { name type } parent { identifier } project { name } } } } }"#;

pub fn fetch(token: &str) -> Fetched<Vec<Value>> {
    let api = http::base("COCKPIT_LINEAR_API", "https://api.linear.app");
    let body = http::post(&format!("{api}/graphql"), &[("Authorization", token)], &json!({"query": QUERY}))?;
    if body["data"].is_null() {
        return Err(Failure::Other(http::message(&body)));
    }
    Ok(parse(&body["data"]))
}

/// The tickets, in progress first, keeping Linear's order (latest update first) within each category.
pub fn parse(data: &Value) -> Vec<Value> {
    let mut tickets: Vec<Value> = list(&data["viewer"]["assignedIssues"]["nodes"])
        .iter()
        .map(|n| {
            json!({
                "id": n["identifier"].as_str().unwrap_or_default(),
                "title": n["title"].as_str().unwrap_or_default(),
                "url": n["url"].as_str().unwrap_or_default(),
                "status": n["state"]["name"].as_str().unwrap_or_default(),
                "statusType": n["state"]["type"].as_str().unwrap_or_default(),
                "parentId": n["parent"]["identifier"],
                "project": n["project"]["name"],
            })
        })
        .filter(|t| rank(t).is_some())
        .collect();
    tickets.sort_by_key(rank);
    tickets
}

fn rank(ticket: &Value) -> Option<usize> {
    ACTIVE.iter().position(|s| ticket["statusType"] == *s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_tickets_in_progress_first_with_parent_and_project() {
        let answer: Value = serde_json::from_str(include_str!("../../tests/fixtures/feed/linear.json")).unwrap();
        let tickets = parse(&answer["data"]);
        let ids: Vec<&str> = tickets.iter().map(|t| t["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["ENG-401", "ENG-402", "ENG-405", "ENG-403"]);
        assert_eq!(
            tickets[1],
            json!({"id": "ENG-402", "title": "Login form", "url": "https://linear.app/acme/issue/ENG-402/login-form", "status": "In Review",
                "statusType": "started", "parentId": "ENG-401", "project": "Sign-in"})
        );
        assert_eq!((&tickets[2]["parentId"], &tickets[2]["project"]), (&Value::Null, &Value::Null));
    }
}
