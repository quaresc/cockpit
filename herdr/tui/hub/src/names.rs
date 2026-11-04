//! Tabs and spaces named after their work: a tab takes its agent's task (the session's own title, else a few words from
//! haiku while Claude Code is installed), a space its agents' shared ticket, else its repo. Only names still at their
//! default or set here earlier ever change, and `names: false` in the config turns it off.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::Duration;

use cockpit_core::transcripts::{self, Transcripts};
use cockpit_core::{agent, files, frontend};
use serde_json::{Value, json};

use crate::config;

const EVERY: Duration = Duration::from_secs(20);
const TAB_MAX: usize = 32;

pub fn start() {
    static STARTED: std::sync::Once = std::sync::Once::new();
    if std::env::var("COCKPIT_HUB_FEED").is_ok_and(|v| v == "off") {
        return;
    }
    STARTED.call_once(|| {
        std::thread::spawn(|| {
            loop {
                if config::load().0["names"] != false {
                    round();
                }
                std::thread::sleep(EVERY);
            }
        });
    });
}

fn dir() -> PathBuf {
    files::cache().join("names")
}

/// What this gave each tab and space last, by id: a name still equal to it is ours to change again.
fn given() -> HashMap<String, String> {
    files::load(&dir().join("given.json")).and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default()
}

fn round() {
    let agents = crate::views::build("agents", &json!({}));
    let facts = crate::store::global().get("agents");
    let inbox = crate::store::global().get("inbox");
    let rows: Vec<Value> = agents["agents"].as_array().cloned().unwrap_or_default();
    if rows.is_empty() {
        return;
    }
    let mut given = given();
    let mut changed = false;
    // Tabs: the first agent in each with a task name.
    let mut tabs: HashMap<String, (String, String, String)> = HashMap::new();
    for r in &rows {
        let (Some(tab), Some(frontend)) = (r["tab"]["id"].as_str(), r["frontend"].as_str()) else { continue };
        if tabs.contains_key(tab) {
            continue;
        }
        let Some(task) = task_of(r, &facts[r["terminal"].as_str().unwrap_or_default()]) else { continue };
        tabs.insert(tab.to_string(), (frontend.to_string(), r["tab"]["name"].as_str().unwrap_or_default().to_string(), task));
    }
    for (id, (frontend, now, name)) in &tabs {
        if now != name && is_default_tab(now, frontend, given.get(id)) && rename_tab(frontend, id, name) {
            given.insert(id.clone(), name.clone());
            changed = true;
        }
    }
    // Spaces: one ticket for every agent in it, else one repo.
    let mut spaces: HashMap<String, Vec<&Value>> = HashMap::new();
    for r in &rows {
        if let Some(space) = r["space"]["id"].as_str() {
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
        let _ = std::fs::create_dir_all(dir());
        let _ = files::write(&dir().join("given.json"), &json!(given));
    }
}

/// A tab name nobody chose: empty, its number, the program's (a shell or an agent), or the one this gave it.
fn is_default_tab(now: &str, frontend: &str, ours: Option<&String>) -> bool {
    let now = now.trim();
    let program = || ["zsh", "bash", "fish", "sh"].contains(&now) || agent::of_program(now).is_some();
    now.is_empty() || now.chars().all(|c| c.is_ascii_digit()) || ours.is_some_and(|o| o == now) || (frontend == "tui" && program())
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

/// The agent's task in a few words: the session's own title, else haiku's.
fn task_of(row: &Value, facts: &Value) -> Option<String> {
    let session = row["session"].as_str().or(facts["session"].as_str())?;
    // The facts' path follows the checkout the session works in, not always the folder Claude keeps it under.
    let cwd = row["cwd"].as_str().unwrap_or_default();
    let found = || transcripts::find(Transcripts::Claude, session, cwd);
    let Some(transcript) = facts["transcript"].as_str().map(PathBuf::from).filter(|p| p.is_file()).or_else(found) else {
        // Another agent's session: the title its own transcript gives.
        let (kind, path) = transcripts::locate(session, cwd, None)?;
        return transcripts::read(kind, &path).map(|t| tidy(&t.title)).filter(|t| !t.is_empty());
    };
    if let Some(title) = session_title(&transcript) {
        return Some(tidy(&title));
    }
    named_by_haiku(session, &transcript).map(|t| tidy(&t))
}

/// The newest title Claude Code wrote into the transcript: the person's own (/rename) over its generated one.
fn session_title(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let (mut custom, mut ai) = (None, None);
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        if !line.contains("-title\"") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
        match v["type"].as_str() {
            Some("custom-title") => custom = v["customTitle"].as_str().map(str::to_string),
            Some("ai-title") => ai = v["aiTitle"].as_str().map(str::to_string),
            _ => {}
        }
    }
    custom.or(ai).filter(|t| !t.trim().is_empty())
}

/// "e2e-test-cockpit-herdr" → "E2e test cockpit herdr", cut to a tab's width.
fn tidy(title: &str) -> String {
    let words = title.replace(['-', '_'], " ").split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out: String = words.chars().take(TAB_MAX).collect();
    if words.chars().count() > TAB_MAX {
        out = out.trim_end().to_string() + "…";
    }
    let mut chars = out.chars();
    chars.next().map(|c| c.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
}

/// A few words from haiku for a session Claude Code has not titled: once per session, cached, on its first prompt only,
/// with no tools, MCP servers, settings or skills. Its transcripts are Claude Code's, so only it is asked.
fn named_by_haiku(session: &str, transcript: &Path) -> Option<String> {
    let claude = agent::get("claude").filter(agent::Agent::installed)?.path();
    let cache = dir().join(format!("{session}.json"));
    if let Some(v) = files::load(&cache) {
        return v["name"].as_str().map(str::to_string);
    }
    let first = first_prompt(transcript)?;
    let _ = std::fs::create_dir_all(dir());
    // Written before the run, so a slow or failed one is never asked again.
    let _ = files::write(&cache, &json!({"name": null}));
    let prompt = format!("Name this coding task in 2 to 5 plain words, no quotes, no punctuation:\n\n{}", first.chars().take(1500).collect::<String>());
    let out = std::process::Command::new(claude)
        .args(["-p", &prompt, "--model", "haiku", "--output-format", "json", "--tools", "", "--no-session-persistence", "--strict-mcp-config"])
        .args(["--setting-sources", "", "--disable-slash-commands", "--system-prompt", "You name tasks in a few words."])
        .env_remove("CLAUDECODE")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    let reply: Value = serde_json::from_slice(&out.stdout).ok()?;
    let name = reply["result"].as_str()?.lines().next()?.trim().trim_matches(['"', '.', '\'']).to_string();
    if name.is_empty() || name.len() > 60 {
        return None;
    }
    let _ = files::write(&cache, &json!({"name": name}));
    Some(name)
}

fn first_prompt(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    BufReader::new(file).lines().map_while(Result::ok).find_map(|line| {
        let v: Value = serde_json::from_str(&line).ok()?;
        (v["type"] == "user").then(|| v["message"]["content"].as_str().map(str::to_string)).flatten().filter(|t| !t.starts_with('<'))
    })
}

/// "ENG-412 · Store and enforce" when every agent there is on one ticket, else their one repo.
fn space_name(members: &[&Value], inbox: &Value) -> Option<String> {
    let one = |key: &str| -> Option<String> {
        let first = members[0][key].as_str().or(members[0][key]["key"].as_str()).filter(|v| !v.trim().is_empty())?.to_string();
        members.iter().all(|m| m[key].as_str().or(m[key]["key"].as_str()) == Some(first.as_str())).then_some(first)
    };
    if let Some(ticket) = one("ticket") {
        let title = members[0]["ticket"]["title"].as_str().map(str::to_string).or_else(|| {
            inbox["tickets"].as_array().into_iter().flatten().find(|t| t["id"] == ticket.as_str()).and_then(|t| t["title"].as_str()).map(str::to_string)
        });
        let short: String = title.unwrap_or_default().split_whitespace().take(3).collect::<Vec<_>>().join(" ");
        return Some(if short.is_empty() { ticket } else { format!("{ticket} · {short}") });
    }
    one("repo")
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
    fn a_title_reads_as_words() {
        assert_eq!(tidy("e2e-test-cockpit-herdr"), "E2e test cockpit herdr");
        assert_eq!(tidy("a-very-long-session-title-that-goes-on-and-on"), "A very long session title that g…");
    }

    #[test]
    fn only_names_nobody_chose_change() {
        let ours = "Fix quotas".to_string();
        assert!(is_default_tab("", "app", None));
        assert!(is_default_tab("2", "app", None));
        assert!(is_default_tab("zsh", "tui", None));
        assert!(is_default_tab("Fix quotas", "app", Some(&ours)));
        assert!(!is_default_tab("my tab", "app", Some(&ours)));
        let pinned = vec!["inbox".to_string()];
        assert!(is_default_space("api", "/r/api", &pinned, None));
        assert!(!is_default_space("[1]  inbox", "/r", &pinned, None));
        assert!(!is_default_space("Release prep", "/r/api", &pinned, None));
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
    }
}
