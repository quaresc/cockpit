//! The Claude Code accounts on this machine, one per config folder (`CLAUDE_CONFIG_DIR`): the folders found under the
//! home folder that hold a sign-in, those `claude.accounts` names in the config, and which of them is the default.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::{files, transcripts};

/// One Claude Code config folder and the account signed in there.
#[derive(Clone, Debug, PartialEq)]
pub struct Account {
    /// The folder, `~` spelled out and no trailing slash: the key every reading of this account carries.
    pub dir: String,
    /// The name the config gives it, else the folder as `~/.claude-work`.
    pub name: String,
    pub default: bool,
    /// `{emailAddress, organizationName}`, `false` when the folder holds no account, null when it has no settings file.
    pub signed_in: Value,
}

impl Account {
    pub fn json(&self) -> Value {
        json!({"dir": self.dir, "name": self.name, "default": self.default})
    }
}

/// The home folder's `~` spelled out, with no trailing slash.
pub fn key(dir: &str) -> String {
    let dir = dir.trim();
    let full = match dir.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => files::home().join(rest.trim_start_matches('/')),
        _ => PathBuf::from(dir),
    };
    let text = full.to_string_lossy().into_owned();
    match text.trim_end_matches('/') {
        "" => text,
        trimmed => trimmed.to_string(),
    }
}

/// The folder Claude Code uses when `CLAUDE_CONFIG_DIR` is not set.
pub fn home_dir() -> String {
    key(&files::home().join(".claude").to_string_lossy())
}

/// A folder as a person reads it: under the home folder from `~`.
pub fn shown(dir: &str) -> String {
    let home = files::home().to_string_lossy().into_owned();
    match dir.strip_prefix(&home) {
        Some(rest) if !home.is_empty() && (rest.is_empty() || rest.starts_with('/')) => format!("~{rest}"),
        _ => dir.to_string(),
    }
}

/// The settings file that names a folder's account: `.claude.json` inside it, and `~/.claude.json` for `~/.claude`.
pub fn settings_file(dir: &str) -> PathBuf {
    let inside = Path::new(dir).join(".claude.json");
    if dir == home_dir() && !inside.is_file() { files::home().join(".claude.json") } else { inside }
}

/// Claude Code's settings file as far as cockpit keeps it: the account's address and organization.
#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct ClaudeFile {
    oauth_account: Option<OauthAccount>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct OauthAccount {
    email_address: Option<String>,
    organization_name: Option<String>,
}

/// `{emailAddress, organizationName}` from a settings file's text, false when it names no account, None when it is no
/// JSON.
pub fn account_of(bytes: &[u8]) -> Option<Value> {
    let file = serde_json::from_slice::<ClaudeFile>(bytes).ok()?;
    Some(match file.oauth_account {
        Some(OauthAccount { email_address: Some(email), organization_name }) => json!({"emailAddress": email, "organizationName": organization_name}),
        _ => Value::Bool(false),
    })
}

/// A folder's account: null without its settings file, false when that file holds no account.
pub fn signed_in(dir: &str) -> Value {
    std::fs::read(settings_file(dir)).ok().and_then(|b| account_of(&b)).unwrap_or(Value::Null)
}

/// The folders under the home folder named like Claude Code's (`.claude`, `.claude-work`, `.claude_work`) whose
/// settings file names an account.
fn found(home: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(home) else { return out };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let named = name == ".claude" || name.strip_prefix(".claude").is_some_and(|rest| rest.starts_with(['-', '_', '.']) && rest.len() > 1);
        if !named || name == ".claude.json" || !entry.path().is_dir() {
            continue;
        }
        let dir = key(&entry.path().to_string_lossy());
        if signed_in(&dir).is_object() {
            out.push(dir);
        }
    }
    out.sort();
    out
}

/// The accounts from what the config says and the folders found: the config's in its order with its names, then the
/// folder this process runs with, then the folders found; the default is `claude.default`, else this process's folder.
pub fn listed(config: &Value, found: &[String], running: &str) -> Vec<(String, String, bool)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for a in config["claude"]["accounts"].as_array().into_iter().flatten() {
        let Some(dir) = a["dir"].as_str().filter(|d| !d.trim().is_empty()).map(key) else { continue };
        if out.iter().any(|(d, _)| *d == dir) {
            continue;
        }
        let name = a["name"].as_str().map(str::trim).filter(|n| !n.is_empty()).map(str::to_string).unwrap_or_else(|| shown(&dir));
        out.push((dir, name));
    }
    for dir in std::iter::once(running.to_string()).chain(found.iter().cloned()) {
        if !out.iter().any(|(d, _)| *d == dir) {
            let name = shown(&dir);
            out.push((dir, name));
        }
    }
    let chosen = config["claude"]["default"].as_str().filter(|d| !d.trim().is_empty()).map(key).filter(|d| out.iter().any(|(o, _)| o == d));
    let default = chosen.unwrap_or_else(|| running.to_string());
    out.into_iter().map(|(dir, name)| (dir.clone(), name, dir == default)).collect()
}

/// The accounts on this machine, the default first; looked for again every 30 seconds.
pub fn all() -> Vec<Account> {
    type Kept = OnceLock<Mutex<Option<(Instant, Vec<Account>)>>>;
    static LAST: Kept = OnceLock::new();
    let mut last = LAST.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, list)) = last.as_ref()
        && at.elapsed() < Duration::from_secs(30)
    {
        return list.clone();
    }
    let running = key(&transcripts::claude_home().to_string_lossy());
    let mut list: Vec<Account> = listed(&files::config(), &found(&files::home()), &running)
        .into_iter()
        .map(|(dir, name, default)| Account { signed_in: signed_in(&dir), dir, name, default })
        .collect();
    list.sort_by_key(|a| !a.default);
    *last = Some((Instant::now(), list.clone()));
    list
}

/// The default account's folder.
pub fn default_dir() -> String {
    all().into_iter().find(|a| a.default).map(|a| a.dir).unwrap_or_else(|| key(&transcripts::claude_home().to_string_lossy()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_is_keyed_with_its_home_spelled_out_and_no_trailing_slash() {
        let home = files::home().to_string_lossy().into_owned();
        assert_eq!(key("~/.claude-work/"), format!("{home}/.claude-work"));
        assert_eq!(key("/srv/claude"), "/srv/claude");
        assert_eq!(shown(&format!("{home}/.claude-work")), "~/.claude-work");
        assert_eq!(shown("/srv/claude"), "/srv/claude");
    }

    #[test]
    fn the_settings_file_names_the_account_or_says_it_is_signed_out() {
        let file = br#"{"numStartups": 9, "oauthAccount": {"accountUuid": "a-1", "emailAddress": "me@acme.example", "organizationName": "Acme"}}"#;
        assert_eq!(account_of(file), Some(json!({"emailAddress": "me@acme.example", "organizationName": "Acme"})));
        assert_eq!(account_of(br#"{"numStartups": 1}"#), Some(json!(false)));
        assert_eq!(account_of(b"not json"), None);
    }

    #[test]
    fn folders_like_claude_codes_with_a_sign_in_are_found() {
        let home = tempfile::tempdir().unwrap();
        let signed = br#"{"oauthAccount": {"emailAddress": "me@acme.example"}}"#;
        for (dir, file) in [(".claude-work", Some(&signed[..])), (".claude_lab", Some(b"{}")), (".claudette", Some(&signed[..])), (".claude-empty", None)] {
            std::fs::create_dir(home.path().join(dir)).unwrap();
            if let Some(text) = file {
                std::fs::write(home.path().join(dir).join(".claude.json"), text).unwrap();
            }
        }
        let found = found(home.path());
        assert_eq!(found, vec![key(&home.path().join(".claude-work").to_string_lossy())], "signed in, and named like Claude Code's");
    }

    #[test]
    fn the_config_names_accounts_and_picks_the_default() {
        let home = files::home().to_string_lossy().into_owned();
        let (claude, work, lab) = (format!("{home}/.claude"), format!("{home}/.claude-work"), format!("{home}/.claude-lab"));
        let config = json!({"claude": {"accounts": [{"dir": "~/.claude-work", "name": "Work"}, {"dir": "~/.claude-work/", "name": "Twice"}], "default": "~/.claude-work"}});
        assert_eq!(
            listed(&config, std::slice::from_ref(&lab), &claude),
            vec![(work.clone(), "Work".to_string(), true), (claude.clone(), "~/.claude".to_string(), false), (lab.clone(), "~/.claude-lab".to_string(), false)]
        );
        // Without `claude.default`, or with one that names no account, the folder this process runs with is the default.
        let unnamed = json!({"claude": {"default": "~/.claude-gone"}});
        assert_eq!(
            listed(&unnamed, std::slice::from_ref(&work), &claude),
            vec![(claude, "~/.claude".to_string(), true), (work, "~/.claude-work".to_string(), false)]
        );
    }
}
