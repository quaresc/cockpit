//! A blueprint: the Mermaid flowchart of a session's plan, kept in `<name>.mmd` beside the view's spec, which the agent
//! rewrites as the work moves. A block `{type: "blueprint", text}` reads that file, its text the seed until it exists.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde_json::{Value, json};

pub fn path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.mmd"))
}

/// The blueprint's text, the file it lives in and when it last changed (ms); other blocks come back as they are.
pub fn living(dir: &Path, name: &str, block: &Value) -> Value {
    if block["type"] != "blueprint" {
        return block.clone();
    }
    let file = path(dir, name);
    let text = std::fs::read_to_string(&file).ok().filter(|t| !t.trim().is_empty());
    let changed = std::fs::metadata(&file).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64);
    let mut out = block.clone();
    out["text"] = json!(text.unwrap_or_else(|| block["text"].as_str().unwrap_or_default().to_string()));
    out["file"] = json!(file.to_string_lossy());
    out["updated"] = changed.map_or(Value::Null, |ms| json!(ms));
    out
}

/// The blueprint as a terminal draws it: a Mermaid block of its current text.
pub fn as_mermaid(dir: &Path, name: &str, block: &Value) -> Value {
    if block["type"] != "blueprint" {
        return block.clone();
    }
    let live = living(dir, name, block);
    json!({"type": "mermaid", "title": block["title"].as_str().unwrap_or("Blueprint"), "text": live["text"]})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_wins_over_the_seed() {
        let root = tempfile::tempdir().unwrap();
        let block = json!({"type": "blueprint", "text": "flowchart TD\n A --> B"});
        assert_eq!(living(root.path(), "plan", &block)["text"], "flowchart TD\n A --> B");
        assert!(living(root.path(), "plan", &block)["updated"].is_null());
        std::fs::write(path(root.path(), "plan"), "flowchart TD\n A --> C\n class A done").unwrap();
        let live = living(root.path(), "plan", &block);
        assert!(live["text"].as_str().unwrap().contains("A --> C"));
        assert!(live["updated"].is_u64());
        assert_eq!(as_mermaid(root.path(), "plan", &block)["type"], "mermaid");
        assert_eq!(living(root.path(), "plan", &json!({"type": "markdown"})), json!({"type": "markdown"}));
    }
}
