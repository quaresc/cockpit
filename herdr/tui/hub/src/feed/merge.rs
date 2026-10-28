//! How each side's part lands in the store's documents. inbox: the hub writes the GitHub fields (and `tickets` given a
//! Linear token) plus its own `hub` record, the mod the rest; `errors` joins both sides'. ci: whole, marked `by: "hub"`
//! when the hub's. While the hub's part is under three minutes old the mod leaves it alone.

use serde_json::{Map, Value, json};

use crate::value::list;

/// The shape of inbox.json the apps read, and of ci.json.
pub const INBOX_VERSION: i64 = 1;
pub const CI_VERSION: i64 = 3;

/// What one round of the feed brings to inbox.json: the parts it fetched, and its errors (the latest of each source).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InboxPart {
    pub github: Option<[Value; 5]>,
    pub tickets: Option<Vec<Value>>,
    pub errors: Vec<String>,
}

const GITHUB_FIELDS: [&str; 5] = ["direct", "team", "botRepos", "mine", "merged"];

pub fn github_fields(github: &super::github::Github) -> [Value; 5] {
    [json!(github.direct), json!(github.team), json!(github.bot_repos), json!(github.mine), json!(github.merged)]
}

/// Whether the inbox holds GitHub data someone fetched, to compare a new fetch against.
pub fn has_github(inbox: &Value) -> bool {
    inbox["version"].as_i64() == Some(INBOX_VERSION) && (inbox["hub"]["github"].is_number() || inbox["modAt"].is_number() || (inbox["hub"].is_null() && inbox["at"].is_number()))
}

/// The inbox with the hub's part merged in; another version's file, or none, starts over.
pub fn merge_inbox(file: &Value, part: &InboxPart, now: u64) -> Value {
    let mut inbox = if file["version"].as_i64() == Some(INBOX_VERSION) {
        file.clone()
    } else {
        json!({"version": INBOX_VERSION, "direct": [], "team": [], "botRepos": [], "mine": [], "tickets": [], "merged": [], "errors": []})
    };
    let mut hub = if inbox["hub"].is_object() { inbox["hub"].clone() } else { json!({}) };
    let theirs: Vec<Value> = list(&inbox["errors"]).iter().filter(|e| !list(&hub["errors"]).contains(e)).cloned().collect();
    if let Some(fields) = &part.github {
        for (name, value) in GITHUB_FIELDS.iter().zip(fields) {
            inbox[*name] = value.clone();
        }
        hub["github"] = json!(now);
    }
    if let Some(tickets) = &part.tickets {
        inbox["tickets"] = json!(tickets);
        hub["tickets"] = json!(now);
    }
    if part.github.is_some() || part.tickets.is_some() || !inbox["at"].is_number() {
        inbox["at"] = json!(now);
    }
    hub["at"] = json!(now);
    hub["errors"] = json!(part.errors);
    inbox["errors"] = json!(theirs.into_iter().chain(part.errors.iter().map(|e| json!(e))).collect::<Vec<_>>());
    inbox["hub"] = hub;
    inbox
}

/// ci.json from the hub's runs, or its error beside the runs it had.
pub fn merge_ci(file: &Value, runs: Result<Vec<Value>, String>, now: u64) -> Value {
    match runs {
        Ok(runs) => json!({"at": now, "runs": runs, "version": CI_VERSION, "by": "hub"}),
        Err(error) => {
            let mut ci = json!({"at": file["at"].as_u64().unwrap_or(0), "runs": file["runs"].as_array().cloned().unwrap_or_default(), "error": error, "by": "hub"});
            if file["version"].as_i64() == Some(CI_VERSION) {
                ci["version"] = json!(CI_VERSION);
            }
            ci
        }
    }
}

/// The inbox with the mod's fields set; given errors go first and the hub's after them. A part of another version than
/// the inbox's starts it over.
pub fn mod_inbox(doc: &Value, fields: &Map<String, Value>) -> Value {
    let is_other = fields.get("version").is_some_and(|v| v != &doc["version"]);
    let mut inbox = crate::store::patch(if is_other { &Value::Null } else { doc }, fields);
    if fields.contains_key("errors") {
        let errors: Vec<Value> = list(&fields["errors"]).iter().chain(list(&inbox["hub"]["errors"]).iter()).cloned().collect();
        inbox["errors"] = json!(errors);
    }
    inbox
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(errors: &[&str]) -> InboxPart {
        InboxPart {
            github: Some([json!([{"url": "d"}]), json!([]), json!(["api"]), json!([{"url": "m"}]), json!([])]),
            tickets: None,
            errors: errors.iter().map(|e| e.to_string()).collect(),
        }
    }

    #[test]
    fn the_hub_writes_only_its_fields_and_keeps_the_mods() {
        let file = json!({"version": 1, "at": 5, "modAt": 5, "direct": [], "team": [], "botRepos": [], "mine": [], "merged": [],
            "tickets": [{"id": "ENG-1"}], "deploys": {"ENG-1": []}, "previews": [], "errors": ["Previews failed"]});
        let inbox = merge_inbox(&file, &part(&["GitHub: HTTP 502"]), 100);
        assert_eq!(inbox["direct"], json!([{"url": "d"}]));
        assert_eq!(inbox["botRepos"], json!(["api"]));
        assert_eq!((&inbox["tickets"], &inbox["deploys"], &inbox["modAt"]), (&file["tickets"], &file["deploys"], &json!(5)));
        assert_eq!(inbox["errors"], json!(["Previews failed", "GitHub: HTTP 502"]));
        assert_eq!(inbox["hub"], json!({"at": 100, "github": 100, "errors": ["GitHub: HTTP 502"]}));
        assert_eq!(inbox["at"], 100);
        let again = merge_inbox(&inbox, &InboxPart { errors: vec![], ..InboxPart::default() }, 200);
        assert_eq!(again["errors"], json!(["Previews failed"]));
        assert_eq!((&again["at"], &again["hub"]["github"], &again["hub"]["at"]), (&json!(100), &json!(100), &json!(200)));
    }

    #[test]
    fn another_version_or_no_file_starts_over() {
        let inbox = merge_inbox(&json!({"version": 0, "mine": "old"}), &part(&[]), 7);
        assert_eq!(inbox["version"], 1);
        assert_eq!(inbox["tickets"], json!([]));
        assert_eq!(inbox["mine"], json!([{"url": "m"}]));
        assert!(!has_github(&Value::Null));
        assert!(has_github(&inbox));
        assert!(!has_github(&merge_inbox(&Value::Null, &InboxPart::default(), 7)));
        assert!(has_github(&json!({"version": 1, "at": 3})));
    }

    #[test]
    fn tickets_are_the_hubs_only_when_it_fetched_them() {
        let inbox = merge_inbox(&Value::Null, &InboxPart { tickets: Some(vec![json!({"id": "ENG-2"})]), ..InboxPart::default() }, 9);
        assert_eq!(inbox["hub"]["tickets"], 9);
        assert!(inbox["hub"].get("github").is_none());
    }

    #[test]
    fn a_ci_error_keeps_the_runs_and_their_time() {
        let file = json!({"at": 50, "runs": [{"repo": "api"}], "version": 3});
        assert_eq!(merge_ci(&file, Err("GitHub HTTP 502".into()), 99), json!({"at": 50, "runs": [{"repo": "api"}], "error": "GitHub HTTP 502", "version": 3, "by": "hub"}));
        assert_eq!(merge_ci(&file, Ok(vec![]), 99), json!({"at": 99, "runs": [], "version": 3, "by": "hub"}));
    }

    #[test]
    fn the_mods_part_keeps_the_hubs_fields_and_errors() {
        let doc = json!({"version": 1, "direct": [{"url": "d"}], "tickets": [], "errors": ["GitHub: HTTP 502", "Previews failed"],
            "hub": {"at": 5, "github": 5, "errors": ["GitHub: HTTP 502"]}});
        let fields = json!({"version": 1, "modAt": 9, "tickets": [{"id": "ENG-1"}], "errors": ["Deploy status failed"]});
        let inbox = mod_inbox(&doc, fields.as_object().unwrap());
        assert_eq!((&inbox["direct"], &inbox["hub"], &inbox["modAt"]), (&doc["direct"], &doc["hub"], &json!(9)));
        assert_eq!(inbox["tickets"][0]["id"], "ENG-1");
        assert_eq!(inbox["errors"], json!(["Deploy status failed", "GitHub: HTTP 502"]));
        let other = mod_inbox(&json!({"version": 0, "mine": "old"}), fields.as_object().unwrap());
        assert!(other.get("mine").is_none(), "another version starts over");
        let dropped = mod_inbox(&inbox, json!({"tickets": []}).as_object().unwrap());
        assert_eq!((&dropped["tickets"], &dropped["errors"]), (&json!([]), &inbox["errors"]), "a part without errors keeps them");
    }
}
