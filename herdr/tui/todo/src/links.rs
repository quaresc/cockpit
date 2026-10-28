//! Ticket keys and URLs in an item's text, and where a ticket key leads, from the config's tickets settings.

use regex::Regex;
use serde_json::Value;

/// The configured ticket key prefixes and those of my tickets, as `team_keys` gives them.
pub fn team_keys(config: &Value, inbox: &Value) -> Vec<String> {
    let mut keys: Vec<String> = config["tickets"]["keys"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_uppercase).collect();
    for ticket in inbox["tickets"].as_array().into_iter().flatten() {
        if let Some((prefix, _)) = ticket["id"].as_str().and_then(|id| id.split_once('-')) {
            keys.push(prefix.to_string());
        }
    }
    let shape = Regex::new(r"^[A-Z][A-Z0-9]{1,9}$").unwrap();
    keys.retain(|k| shape.is_match(k));
    keys.sort();
    keys.dedup();
    keys
}

/// URLs, and ticket keys of these teams, in any case.
pub fn token_pattern(teams: &[String]) -> Regex {
    let mut pattern = r"https?://\S+".to_string();
    if !teams.is_empty() {
        let alternatives: Vec<String> = teams.iter().map(|t| regex::escape(t)).collect();
        pattern.push_str(&format!(r"|\b(?:{})-\d+\b", alternatives.join("|")));
    }
    Regex::new(&format!("(?i){pattern}")).unwrap()
}

/// The text cut into plain parts and tokens, in order: (text, is a token).
pub fn split<'a>(text: &'a str, tokens: &Regex) -> Vec<(&'a str, bool)> {
    let mut parts = Vec::new();
    let mut at = 0;
    for found in tokens.find_iter(text) {
        if found.start() > at {
            parts.push((&text[at..found.start()], false));
        }
        parts.push((found.as_str(), true));
        at = found.end();
    }
    if at < text.len() {
        parts.push((&text[at..], false));
    }
    parts
}

/// Percent-encoding: every byte but ASCII letters, digits and `_.-~` escaped.
fn quote(value: &str) -> String {
    value
        .bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"_.-~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

/// Where a token leads: itself for a URL; for a ticket key, the configured template's URL, else one of my tickets' URL
/// with its id swapped. None when no URL is known.
pub fn url(token: &str, config: &Value, inbox: &Value) -> Option<String> {
    if token.to_lowercase().starts_with("http") {
        return Some(token.to_string());
    }
    let id = token.to_uppercase();
    if let Some(template) = config["tickets"]["urlTemplate"].as_str().filter(|t| !t.is_empty()) {
        let placeholder = Regex::new(r"\{(\w+)\}").unwrap();
        return Some(placeholder.replace_all(template, |c: &regex::Captures| if &c[1] == "id" { quote(&id) } else { String::new() }).into_owned());
    }
    inbox["tickets"].as_array().into_iter().flatten().find_map(|ticket| {
        let (url, known) = (ticket["url"].as_str()?, ticket["id"].as_str()?);
        let at = url.rfind(&format!("/{known}")).filter(|_| !known.is_empty())?;
        Some(format!("{}{id}", &url[..at + 1]))
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn finds_tokens_and_their_urls() {
        let config = json!({"tickets": {"keys": ["eng"], "urlTemplate": ""}});
        let inbox = json!({"tickets": [{"id": "OPS-12", "url": "https://linear.app/acme/issue/OPS-12/title"}]});
        let teams = team_keys(&config, &inbox);
        assert_eq!(teams, ["ENG", "OPS"]);
        let tokens = token_pattern(&teams);
        let parts = split("fix eng-398 then see https://x.y/z and OPS-3", &tokens);
        let found: Vec<&str> = parts.iter().filter(|p| p.1).map(|p| p.0).collect();
        assert_eq!(found, ["eng-398", "https://x.y/z", "OPS-3"]);
        assert_eq!(url("eng-398", &config, &inbox).as_deref(), Some("https://linear.app/acme/issue/ENG-398"));
        let templated = json!({"tickets": {"urlTemplate": "https://t.io/{id}"}});
        assert_eq!(url("eng-1", &templated, &inbox).as_deref(), Some("https://t.io/ENG-1"));
    }
}
