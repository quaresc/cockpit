//! Runbook files and handoffs as runbook.py reads them: a title, an `updated:` line, a step table and notes, parsed to
//! the same JSON the runbook app passes whole; handoffs by id.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use cockpit_core::files;
use serde_json::{Value, json};

/// `COCKPIT_RUNBOOKS_DIR`, else ~/.config/cockpit/runbooks.
pub fn runbooks_dir() -> PathBuf {
    std::env::var("COCKPIT_RUNBOOKS_DIR").map(PathBuf::from).unwrap_or_else(|_| files::config_dir().join("runbooks"))
}

pub fn handoffs_dir() -> PathBuf {
    files::cache().join("handoffs")
}

const BULLETS: &[char] = &['-', '*', '•'];

pub fn parse(path: &Path) -> std::io::Result<Value> {
    let content = std::fs::read_to_string(path)?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    let slug = name.strip_suffix(".md").unwrap_or(name);
    let (mut title, mut updated, mut steps, mut notes) = (String::new(), String::new(), Vec::new(), Vec::new());
    let mut in_notes = false;
    for raw in content.lines() {
        let line = raw.trim();
        if let Some(rest) = line.strip_prefix("# ").filter(|_| title.is_empty()) {
            title = rest.to_string();
        } else if line.to_lowercase().starts_with("updated:") {
            updated = line.split_once(':').map(|(_, v)| v.trim().to_string()).unwrap_or_default();
        } else if line.starts_with("## ") {
            in_notes = line.to_lowercase().contains("note");
        } else if line.starts_with('|') {
            let mut cells: Vec<String> = line.trim_matches('|').split('|').map(|c| c.trim().to_string()).collect();
            let is_rule = cells.concat().chars().all(|c| "-: ".contains(c));
            if cells.len() >= 4 && !is_rule && cells[0] != "#" {
                cells.resize(cells.len().max(5), String::new());
                let command = cells[4].trim_matches('`').to_string();
                steps.push(json!({"n": cells[0], "step": cells[1], "where": cells[2], "status": cells[3], "command": command}));
            }
        } else if in_notes && line.starts_with(BULLETS) {
            notes.push(line.trim_start_matches(|c| BULLETS.contains(&c) || c == ' ').trim().to_string());
        }
    }
    Ok(json!({"path": path.to_string_lossy(), "slug": slug, "title": title, "updated": updated, "steps": steps, "notes": notes}))
}

/// The runbooks, newest first; one that cannot be read or is not UTF-8 is left out.
pub fn read_books(dir: &Path) -> Vec<Value> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .map(|p| (std::fs::metadata(&p).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH), p))
        .collect();
    paths.sort_by_key(|p| std::cmp::Reverse(p.0));
    paths.into_iter().filter_map(|(_, p)| parse(&p).ok()).collect()
}

pub fn read_handoffs() -> HashMap<String, Value> {
    let Ok(entries) = std::fs::read_dir(handoffs_dir()) else {
        return HashMap::new();
    };
    entries
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".json"))
        .filter_map(|e| files::load(&e.path()))
        .filter_map(|h| Some((h.get("id")?.as_str()?.to_string(), h)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_runbook() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("eng-401.md");
        std::fs::write(
            &path,
            "# ENG-401 · Login\nupdated: 2026-10-28 09:00\n\n| # | Step | Where | Status | Command |\n|---|---|---|---|---|\n| 1 | Merge | api#4 | ✅ done | |\n| 2 | Deploy | prod | ⬜ to do | `mix run x` |\n| 3 | Short | x | ⏳ waiting |\n\n## Notes\n- first\n* second\n",
        )
        .unwrap();
        let book = parse(&path).unwrap();
        assert_eq!(book["slug"], "eng-401");
        assert_eq!(book["title"], "ENG-401 · Login");
        assert_eq!(book["updated"], "2026-10-28 09:00");
        assert_eq!(book["steps"].as_array().unwrap().len(), 3);
        assert_eq!(book["steps"][1]["command"], "mix run x");
        assert_eq!(book["steps"][2]["command"], "");
        assert_eq!(book["notes"], json!(["first", "second"]));
    }

    #[test]
    fn non_utf8_runbooks_are_left_out() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("bad.md"), [0xff, 0xfe, 0x00]).unwrap();
        std::fs::write(dir.path().join("good.md"), "# Good\n").unwrap();
        let books = read_books(dir.path());
        assert_eq!(books.len(), 1);
        assert_eq!(books[0]["slug"], "good");
    }
}
