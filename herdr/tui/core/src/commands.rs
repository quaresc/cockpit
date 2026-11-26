//! The descriptions of the commands a person added to an agent (`~/.claude/commands/*.md`, a plugin's skills, Codex's
//! prompts…), read from their files' front matter, for a driven session whose agent lists its commands by name only.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use serde_json::Value;

/// Fills the empty `description` of each `{name, description}` in `commands` from the files agent `id` reads in
/// `cwd`; a command that no file describes keeps its empty description.
pub fn describe(id: &str, cwd: &str, commands: &mut [Value]) {
    describe_in(id, &std::env::var("HOME").unwrap_or_default(), cwd, commands)
}

/// `describe` with the home folder given.
pub fn describe_in(id: &str, home: &str, cwd: &str, commands: &mut [Value]) {
    if commands.iter().all(|c| c["description"].as_str().is_some_and(|d| !d.is_empty())) {
        return;
    }
    let mut known: HashMap<String, String> = HashMap::new();
    for pattern in crate::agent::command_files(id) {
        let pattern = pattern.replace("{home}", home).replace("{cwd}", cwd);
        for path in matching(&pattern) {
            if let Some((name, description)) = described(&path, &pattern) {
                known.entry(name).or_insert(description);
            }
        }
    }
    for c in commands.iter_mut() {
        if c["description"].as_str().is_some_and(|d| !d.is_empty()) {
            continue;
        }
        let name = c["name"].as_str().unwrap_or_default();
        let bare = name.rsplit(':').next().unwrap_or(name);
        if let Some(d) = known.get(name).or_else(|| known.get(bare)) {
            c["description"] = Value::from(d.as_str());
        }
    }
}

/// The files a pattern names: `*` one name, `**` any folders, in a fixed prefix.
fn matching(pattern: &str) -> Vec<PathBuf> {
    let parts: Vec<&str> = pattern.split('/').collect();
    let mut found = vec![PathBuf::from("/")];
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        let last = i + 1 == parts.len();
        let mut next = Vec::new();
        for dir in &found {
            match *part {
                "**" => next.extend(folders_under(dir, 4)),
                p if p.contains('*') => {
                    let (prefix, suffix) = p.split_once('*').unwrap_or((p, ""));
                    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                        let name = entry.file_name().to_string_lossy().into_owned();
                        let is_dir = entry.path().is_dir();
                        if name.starts_with(prefix) && name.ends_with(suffix) && (last != is_dir) {
                            next.push(entry.path());
                        }
                    }
                }
                p => {
                    let path = dir.join(p);
                    if path.exists() {
                        next.push(path);
                    }
                }
            }
        }
        found = next;
        if found.is_empty() {
            break;
        }
    }
    found.into_iter().filter(|p| p.is_file()).collect()
}

/// A folder and every folder under it, a few levels down.
fn folders_under(dir: &Path, depth: usize) -> Vec<PathBuf> {
    let mut out = vec![dir.to_path_buf()];
    if depth == 0 {
        return out;
    }
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() && !entry.file_name().to_string_lossy().starts_with('.') {
            out.extend(folders_under(&path, depth - 1));
        }
    }
    out
}

/// A command's name and description from its file, kept until the file changes: the name is the file's stem (a
/// skill's folder), namespaced with the folder between the pattern's `**` and the file as Claude Code does
/// (`frontend:deploy`, `tools:ship`); the description is the front matter's, else the first line of words.
fn described(path: &Path, pattern: &str) -> Option<(String, String)> {
    type Seen = HashMap<PathBuf, (Option<SystemTime>, Option<(String, String)>)>;
    static SEEN: Mutex<Option<Seen>> = Mutex::new(None);
    let stamp = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    let seen = seen.get_or_insert_with(HashMap::new);
    if let Some((at, found)) = seen.get(path)
        && *at == stamp
    {
        return found.clone();
    }
    let found = std::fs::read_to_string(path).ok().map(|text| (name_of(path, pattern), description_in(&text)));
    if seen.len() > 2000 {
        seen.clear();
    }
    seen.insert(path.to_path_buf(), (stamp, found.clone()));
    found
}

fn name_of(path: &Path, pattern: &str) -> String {
    let skill = path.file_name().is_some_and(|n| n == "SKILL.md");
    let stem = if skill {
        path.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    } else {
        path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    };
    if !pattern.contains("**") {
        return stem;
    }
    // The folders between the pattern's fixed head and the file name it: `commands/frontend/deploy.md` is `frontend:deploy`.
    let head = pattern.split("**").next().unwrap_or("").trim_end_matches('/');
    let rest = path.to_string_lossy().strip_prefix(head).map(|r| r.trim_start_matches('/').to_string()).unwrap_or_default();
    let dirs: Vec<&str> = rest.split('/').collect();
    let keep = dirs.len().saturating_sub(if skill { 2 } else { 1 });
    let between: Vec<&str> = dirs[..keep].iter().copied().filter(|d| !d.is_empty() && *d != "commands" && *d != "skills" && !d.contains('.')).collect();
    match between.last() {
        Some(ns) => format!("{ns}:{stem}"),
        None => stem,
    }
}

/// The `description:` of a YAML or TOML front matter, else the first line that is words.
fn description_in(text: &str) -> String {
    let fenced = matches!(text.lines().next().map(str::trim), Some("---") | Some("+++"));
    if fenced {
        for l in text.lines().skip(1) {
            let t = l.trim();
            if t == "---" || t == "+++" {
                break;
            }
            if let Some(d) = t.strip_prefix("description") {
                let d = d.trim_start_matches([':', '=', ' ']).trim().trim_matches(['"', '\'']);
                return d.chars().take(200).collect();
            }
        }
    }
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with("---") && !l.starts_with("+++"))
        .map(|l| l.chars().take(200).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn descriptions_come_from_the_files_the_agent_reads() {
        let home = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        let at = |root: &Path, rel: &str, text: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        };
        at(home.path(), ".claude/commands/review.md", "---\ndescription: Review the branch\n---\nDo it.");
        at(home.path(), ".claude/commands/frontend/deploy.md", "Deploy the front.\n\nMore.");
        at(cwd.path(), ".claude/skills/quotas/SKILL.md", "---\nname: quotas\ndescription: \"Daily quotas per account\"\n---");
        at(home.path(), ".claude/plugins/cache/acme/tools/1.0/skills/ship/SKILL.md", "---\ndescription: Ship it\n---");
        let mut commands = vec![
            json!({"name": "review", "description": ""}),
            json!({"name": "frontend:deploy", "description": ""}),
            json!({"name": "quotas", "description": ""}),
            json!({"name": "tools:ship", "description": ""}),
            json!({"name": "help", "description": ""}),
            json!({"name": "compact", "description": "Keep it"}),
        ];
        describe_in("claude", home.path().to_str().unwrap(), cwd.path().to_str().unwrap(), &mut commands);
        let got: Vec<&str> = commands.iter().map(|c| c["description"].as_str().unwrap()).collect();
        assert_eq!(got, ["Review the branch", "Deploy the front.", "Daily quotas per account", "Ship it", "", "Keep it"]);
        let mut none = [json!({"name": "x", "description": ""})];
        describe_in("pig", home.path().to_str().unwrap(), "", &mut none);
        assert_eq!(none[0]["description"], "", "an agent with no files");
    }
}
