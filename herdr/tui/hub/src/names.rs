//! Tabs and spaces named after their work: a tab takes the name the agents view gives its agent (`cockpit_core::names`:
//! the session's own title, else its first prompt in a few words), a space its agents' shared ticket, else its repo.
//! Only a label that is a placeholder or the one given here last ever changes, so a name the person typed, or another
//! tool gave, stays; `names: false` in the config turns it off.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use cockpit_core::{frontend, names};
use serde_json::{Value, json};

use crate::config;

const EVERY: Duration = Duration::from_secs(20);

pub fn start() {
    static STARTED: std::sync::Once = std::sync::Once::new();
    if std::env::var("COCKPIT_HUB_FEED").is_ok_and(|v| v == "off") {
        return;
    }
    STARTED.call_once(|| {
        std::thread::spawn(|| {
            loop {
                if names::on(&config::load().0) {
                    round();
                }
                std::thread::sleep(EVERY);
            }
        });
    });
}

fn round() {
    let agents = crate::views::build("agents", &json!({}));
    let inbox = crate::store::global().get("inbox");
    let rows: Vec<Value> = agents["agents"].as_array().cloned().unwrap_or_default();
    if rows.is_empty() {
        return;
    }
    let mut given = names::given();
    let mut changed = false;
    // Tabs: the name of the first agent in each that has more to say than its agent's name.
    let mut tabs: HashMap<String, (String, String, String, names::Source)> = HashMap::new();
    for r in &rows {
        let (Some(tab), Some(frontend)) = (r["tab"]["id"].as_str().filter(|t| !t.is_empty()), r["frontend"].as_str()) else { continue };
        let source = source_of(&r["title_source"]);
        if tabs.get(tab).is_some_and(|(_, _, _, s)| *s != names::Source::Default) {
            continue;
        }
        let (label, name) = (r["tab"]["name"].as_str().unwrap_or_default(), r["title"].as_str().unwrap_or_default());
        tabs.insert(tab.to_string(), (frontend.to_string(), label.to_string(), name.to_string(), source));
    }
    for (id, (frontend, label, name, source)) in &tabs {
        if names::writes(label, given.get(id).map(String::as_str), name, *source) && rename_tab(frontend, id, name) {
            given.insert(id.clone(), name.clone());
            changed = true;
        }
    }
    // Spaces: one ticket for every agent in it, else one repo.
    let mut spaces: HashMap<String, Vec<&Value>> = HashMap::new();
    for r in &rows {
        if let Some(space) = r["space"]["id"].as_str().filter(|s| !s.is_empty()) {
            spaces.entry(space.to_string()).or_default().push(r);
        }
    }
    let pinned = config::load().space_names();
    for (id, members) in spaces {
        let now = members[0]["space"]["name"].as_str().unwrap_or_default().to_string();
        let frontend = members[0]["frontend"].as_str().unwrap_or("app").to_string();
        // Never an empty name: Herdr would take it as "back to the folder's", over a name the person gave.
        let Some(name) = space_name(&members, &inbox).filter(|n| !n.trim().is_empty()) else { continue };
        let folder = members[0]["cwd"].as_str().unwrap_or_default();
        if now != name && is_default_space(&now, folder, &pinned, given.get(&id)) && rename_space(&frontend, &id, &name) {
            given.insert(id, name);
            changed = true;
        }
    }
    if changed {
        names::keep(&given);
    }
}

/// Where a row's name comes from, as the agents view says it.
fn source_of(word: &Value) -> names::Source {
    match word.as_str() {
        Some("person") => names::Source::Person,
        Some("agent") => names::Source::Agent,
        Some("prompt") => names::Source::Prompt,
        _ => names::Source::Default,
    }
}

/// A space name nobody chose: empty, its folder's, or the one this gave it; never a pinned space.
fn is_default_space(now: &str, folder: &str, pinned: &[String], ours: Option<&String>) -> bool {
    let bare = now.trim().trim_start_matches(|c: char| c == '[' || c.is_ascii_digit() || c == ']').trim();
    if pinned.iter().any(|p| bare.ends_with(p.as_str())) {
        return false;
    }
    let base = Path::new(folder).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    bare.is_empty() || bare == base || ours.is_some_and(|o| o == now)
}

/// "ENG-412 · Store and enforce" when every agent there is on one ticket, else their one repo.
fn space_name(members: &[&Value], inbox: &Value) -> Option<String> {
    let one = |key: &str| -> Option<String> {
        let first = members[0][key].as_str().or(members[0][key]["key"].as_str()).filter(|v| !v.trim().is_empty())?.to_string();
        members.iter().all(|m| m[key].as_str().or(m[key]["key"].as_str()) == Some(first.as_str())).then_some(first)
    };
    // The tracker's and git's words become a name in the window: cleaned like any title (`names::clean`).
    if let Some(ticket) = one("ticket").map(|t| names::cap(&names::clean(&t), 24)).filter(|t| !t.is_empty()) {
        let title = members[0]["ticket"]["title"].as_str().map(str::to_string).or_else(|| {
            inbox["tickets"].as_array().into_iter().flatten().find(|t| t["id"] == ticket.as_str()).and_then(|t| t["title"].as_str()).map(str::to_string)
        });
        let short: String = names::clean(&title.unwrap_or_default()).split_whitespace().take(3).collect::<Vec<_>>().join(" ");
        return Some(names::cap(&if short.is_empty() { ticket } else { format!("{ticket} · {short}") }, 48));
    }
    one("repo").map(|r| names::cap(&names::clean(&r), 48)).filter(|r| !r.is_empty())
}

fn rename_tab(frontend: &str, id: &str, name: &str) -> bool {
    if frontend == "tui" {
        return frontend::herdr(&["tab", "rename", id, name]).is_some();
    }
    frontend::app_do("tab.rename", &json!({"id": id, "name": name})).is_some()
}

fn rename_space(frontend: &str, id: &str, name: &str) -> bool {
    if frontend == "tui" {
        return frontend::herdr(&["workspace", "rename", id, name]).is_some();
    }
    frontend::app_do("space.rename", &json!({"id": id, "name": name})).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_space_names_nobody_chose_change() {
        let pinned = vec!["inbox".to_string()];
        assert!(is_default_space("api", "/r/api", &pinned, None));
        assert!(!is_default_space("[1]  inbox", "/r", &pinned, None));
        assert!(!is_default_space("Release prep", "/r/api", &pinned, None));
    }

    #[test]
    fn a_row_says_where_its_name_comes_from() {
        assert_eq!(source_of(&json!("agent")), names::Source::Agent);
        assert_eq!(source_of(&json!("prompt")), names::Source::Prompt);
        assert_eq!(source_of(&json!("person")), names::Source::Person);
        assert_eq!(source_of(&Value::Null), names::Source::Default);
    }

    #[test]
    fn a_space_takes_its_ticket_else_its_repo() {
        let a = json!({"ticket": {"key": "ENG-412", "title": "Store and enforce daily quotas"}, "repo": "api"});
        let b = json!({"ticket": {"key": "ENG-412", "title": "Store and enforce daily quotas"}, "repo": "web"});
        let c = json!({"ticket": null, "repo": "api"});
        assert_eq!(space_name(&[&a, &b], &Value::Null).as_deref(), Some("ENG-412 · Store and enforce"));
        assert_eq!(space_name(&[&a, &c], &Value::Null).as_deref(), Some("api"), "no shared ticket: the shared repo");
        assert_eq!(space_name(&[&c, &c], &Value::Null).as_deref(), Some("api"));
        let blank = json!({"ticket": null, "repo": ""});
        assert_eq!(space_name(&[&blank, &blank], &Value::Null), None, "no empty name");
        let hostile = json!({"ticket": {"key": "ENG-7\u{202e}", "title": "Store\u{200b} and\u{1b}[2J enforce quotas"}, "repo": "api"});
        assert_eq!(space_name(&[&hostile, &hostile], &Value::Null).as_deref(), Some("ENG-7 · Store and enforce"), "the tracker's words, cleaned");
        let long = json!({"ticket": null, "repo": "a".repeat(200)});
        assert_eq!(space_name(&[&long, &long], &Value::Null).map(|n| n.chars().count()), Some(48));
    }
}
