//! cockpit_config.py: the config with its defaults, the plugin's scripts, and the URLs and names derived from it.

use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::Duration;

use cockpit_core::files;
use serde_json::{Value, json};

use crate::proc;
use crate::py::{text, truthy};

fn defaults() -> Value {
    json!({
        "reposDir": "~",
        "github": {"enabled": true, "org": ""},
        "tickets": {"enabled": false, "name": "Linear", "server": "claude.ai Linear", "keys": [], "urlTemplate": "", "tools": {}, "fields": {}, "statusTypes": {}},
        "ci": {"enabled": true, "name": "GitHub", "server": "", "imageJob": "", "tools": {}, "fields": {}, "outcomes": {}, "endedPhases": []},
        "platform": {"enabled": false, "server": "", "previewPrefix": "", "previewDashboardUrl": "", "workflowUrl": "", "tools": {}, "fields": {}},
        "calendar": {"enabled": false, "name": "Google Calendar", "server": "claude.ai Google Calendar", "tools": {}, "fields": {}, "skip": {}, "only": {}, "responses": {}},
        "brief": {"enabled": true, "at": "08:30", "weekends": false},
        "meetingBrief": {"enabled": true, "model": "sonnet", "skip": ["lunch", "focus", "break", "wday", "out of office"], "tools": []},
        "notifications": true,
        "spaces": ["inbox", "terminal", "actions", "to-do", "runbook", "meetings"],
        "runbookTools": [],
        "worktrees": false,
        "apps": "full",
    })
}

#[derive(Clone, Debug)]
pub struct Config(pub Value);

/// The config as the mod resolved it, else the raw file; over the defaults either way, read again on each action.
pub fn load() -> Config {
    let raw = files::load(&files::cache().join("config.json"))
        .or_else(|| files::load(&files::config_dir().join("config.json")))
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    let mut config = defaults();
    for (name, value) in raw.as_object().into_iter().flatten() {
        match (config.get_mut(name), value) {
            (Some(Value::Object(base)), Value::Object(over)) => base.extend(over.clone()),
            _ => {
                config[name] = value.clone();
            }
        }
    }
    let repos = config["reposDir"].as_str().unwrap_or("~").to_string();
    let home = files::home().to_string_lossy().into_owned();
    let expanded = if repos == "~" || repos.starts_with("~/") { format!("{home}{}", &repos[1..]) } else { repos };
    let trimmed = expanded.trim_end_matches('/');
    config["reposDir"] = Value::from(if trimmed.is_empty() { home } else { trimmed.to_string() });
    Config(config)
}

impl Config {
    pub fn repos(&self) -> String {
        text(&self.0["reposDir"])
    }

    pub fn space_names(&self) -> Vec<String> {
        crate::py::list(&self.0["spaces"]).iter().map(|s| if s.is_object() { text(&s["name"]) } else { text(s) }).collect()
    }
}

/// One of the plugin's scripts, through the link that follows herdr's updates, else in the herdr folder.
pub fn script(name: &str) -> PathBuf {
    let linked = files::config_dir().join("herdr").join(name);
    if linked.exists() { linked } else { files::herdr_dir().join(name) }
}

pub fn open_url(url: &str) {
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    proc::run(&proc::strings(&[opener, url]), Duration::from_secs(10), None);
}

/// The clipboard: the first of pbcopy, wl-copy, xclip found; false when none is.
pub fn copy(content: &str) -> bool {
    use std::io::Write;
    let tools: [&[&str]; 3] = [&["pbcopy"], &["wl-copy"], &["xclip", "-selection", "clipboard"]];
    for tool in tools {
        let Some(path) = proc::which(tool[0]) else {
            continue;
        };
        let child = std::process::Command::new(path).args(&tool[1..]).stdin(std::process::Stdio::piped()).spawn();
        let Ok(mut child) = child else { return false };
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(content.as_bytes());
        }
        let _ = child.wait();
        return true;
    }
    false
}

/// urllib's quote with nothing safe.
fn url_quote(value: &str) -> String {
    value.bytes().map(|b| if b.is_ascii_alphanumeric() || b"_.-~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// A URL template with `{name}` placeholders filled in; None when the template is empty.
pub fn fill(template: &Value, values: &[(&str, &str)]) -> Option<String> {
    let template = template.as_str().filter(|t| !t.is_empty())?;
    static HOLES: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"\{(\w+)\}").expect("a valid pattern"));
    Some(HOLES.replace_all(template, |c: &regex::Captures| url_quote(values.iter().find(|(k, _)| *k == &c[1]).map(|(_, v)| *v).unwrap_or(""))).into_owned())
}

pub fn read_inbox() -> Value {
    files::load(&files::cache().join("inbox.json")).unwrap_or_else(|| json!({}))
}

/// A ticket's URL: the configured template's, else an assigned ticket's URL with the id swapped.
pub fn ticket_url(id: &str, config: &Config, inbox: Option<&Value>) -> Option<String> {
    let upper = id.to_uppercase();
    if let Some(filled) = fill(&config.0["tickets"]["urlTemplate"], &[("id", &upper)]) {
        return Some(filled);
    }
    let read;
    let inbox = match inbox {
        Some(inbox) => inbox,
        None => {
            read = read_inbox();
            &read
        }
    };
    for ticket in crate::py::list(&inbox["tickets"]) {
        let (url, known) = (ticket["url"].as_str().unwrap_or_default(), ticket["id"].as_str().unwrap_or_default());
        let needle = format!("/{known}");
        if !known.is_empty()
            && let Some(at) = url.rfind(&needle)
        {
            return Some(format!("{}{upper}", &url[..at + 1]));
        }
    }
    None
}

/// Whether a branch names the ticket as a whole token: eng-4-login does, eng-42-login does not.
pub fn has_ticket(branch: &str, id: &str) -> bool {
    let (branch, id) = (branch.to_lowercase(), id.to_lowercase());
    if id.is_empty() {
        return false;
    }
    branch.match_indices(&id).any(|(at, _)| {
        let before = branch[..at].chars().next_back();
        let after = branch[at + id.len()..].chars().next();
        before.is_none_or(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit())) && after.is_none_or(|c| !c.is_ascii_digit())
    })
}

/// A PR's URL from the inbox cache, else from the configured GitHub organization.
pub fn pr_url(repo: &str, number: &Value, config: &Config, inbox: &Value) -> Option<String> {
    for field in ["mine", "direct", "team", "merged"] {
        for pr in crate::py::list(&inbox[field]) {
            if pr["repo"].as_str() == Some(repo) && text(&pr["number"]) == text(number) && truthy(&pr["url"]) {
                return Some(text(&pr["url"]));
            }
        }
    }
    let org = text(&config.0["github"]["org"]);
    (!org.is_empty() && org != "None").then(|| format!("https://github.com/{org}/{repo}/pull/{}", text(number)))
}

/// The tool-name prefix Claude Code gives an MCP server, e.g. "claude.ai Linear" -> mcp__claude_ai_Linear.
pub fn tool_prefix(server: &str) -> String {
    let cleaned: String = server.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect();
    let squeezed = regex::Regex::new("_+").expect("a valid pattern").replace_all(&cleaned, "_");
    format!("mcp__{}", squeezed.trim_end_matches('_'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_ticket_matches_whole_tokens() {
        assert!(has_ticket("me/eng-4-login", "ENG-4"));
        assert!(!has_ticket("me/eng-42-login", "ENG-4"));
        assert!(!has_ticket("me/xeng-4", "ENG-4"));
        assert!(has_ticket("eng-4", "eng-4"));
        assert!(!has_ticket("main", ""));
    }

    #[test]
    fn tool_prefix_like_claude_code() {
        assert_eq!(tool_prefix("claude.ai Linear"), "mcp__claude_ai_Linear");
        assert_eq!(tool_prefix("circleci"), "mcp__circleci");
        assert_eq!(tool_prefix("a  b!"), "mcp__a_b");
    }

    #[test]
    fn fill_quotes_values() {
        assert_eq!(fill(&json!("https://x/{id}/y"), &[("id", "A B/1")]).as_deref(), Some("https://x/A%20B%2F1/y"));
        assert_eq!(fill(&json!(""), &[]), None);
    }

    #[test]
    fn ticket_url_from_an_assigned_ticket() {
        let config = Config(json!({"tickets": {"urlTemplate": ""}}));
        let inbox = json!({"tickets": [{"id": "OPS-1", "url": "https://linear.app/acme/issue/OPS-1/title"}]});
        assert_eq!(ticket_url("ops-9", &config, Some(&inbox)).as_deref(), Some("https://linear.app/acme/issue/OPS-9"));
    }
}
