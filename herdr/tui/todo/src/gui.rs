//! The to-do list for the Herdr app's cockpit mod: the view model the hub pushes, and the edits it runs on the file
//! (the same `Doc` edits the TUI makes, each read fresh from disk, undoable while nobody else wrote the file).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use cockpit_core::files;
use serde_json::{Value, json};

use crate::app::todo_path;
use crate::doc::{self, Doc};
use crate::links;

/// The folders the view reads: the list's and the cache's (the inbox gives ticket keys their URLs).
pub fn dirs() -> Vec<PathBuf> {
    vec![files::config_dir()]
}

/// The list as the mod draws it: every item with its text cut into plain parts and links, open ones first in file
/// order, and whether an undo is waiting.
pub fn view(_args: &Value, doc: &dyn Fn(&str) -> Value) -> Value {
    let inbox = doc("inbox");
    let inbox = if inbox.is_null() { files::load(&files::cache().join("inbox.json")).unwrap_or(Value::Null) } else { inbox };
    model(&todo_path(), &files::config(), &inbox)
}

pub fn model(path: &Path, config: &Value, inbox: &Value) -> Value {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let tokens = links::token_pattern(&links::team_keys(config, inbox));
    let items: Vec<Value> = Doc::parse(&text)
        .items()
        .into_iter()
        .map(|item| {
            let undated = doc::undated(&item.text);
            let done_on = item.text.get(undated.len()..).and_then(|rest| rest.trim().strip_prefix("(done ")).and_then(|d| d.strip_suffix(')'));
            let parts: Vec<Value> = links::split(&item.text, &tokens)
                .into_iter()
                .map(|(part, is_token)| {
                    let url = is_token.then(|| links::url(part, config, inbox)).flatten();
                    let label = if is_token && part.to_lowercase().starts_with("http") { "link".to_string() } else { part.to_uppercase() };
                    json!({"text": part, "token": is_token, "url": url, "label": if is_token { Value::from(label) } else { Value::Null }})
                })
                .collect();
            json!({"key": item.text, "line": item.line, "done": item.done, "text": item.text, "undated": undated, "done_on": done_on, "parts": parts})
        })
        .collect();
    let open = items.iter().filter(|i| i["done"] == false).count();
    json!({
        "path": path.to_string_lossy(),
        "items": items,
        "open": open,
        "done": items.len() - open,
        "can_undo": can_undo(path),
    })
}

// ---------------------------------------------------------------------------------------------------------------------
// Edits

const UNDO_DEPTH: usize = 50;

/// Per file: (the text before an edit, the text it wrote), newest last.
static UNDO: Mutex<Vec<(PathBuf, String, String)>> = Mutex::new(Vec::new());

fn can_undo(path: &Path) -> bool {
    let disk = std::fs::read_to_string(path).unwrap_or_default();
    UNDO.lock().unwrap_or_else(|e| e.into_inner()).iter().rev().find(|(p, _, _)| p == path).is_some_and(|(_, _, after)| *after == disk)
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let mut tmp = path.to_path_buf().into_os_string();
    tmp.push(".tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// One change to the file read fresh from disk: written, remembered for undo, its note returned.
fn change(path: &Path, edit: impl FnOnce(&mut Doc) -> Result<String, String>) -> Result<String, String> {
    let before = std::fs::read_to_string(path).unwrap_or_default();
    let mut doc = Doc::parse(&before);
    if doc.items().is_empty() && before.trim().is_empty() {
        doc = Doc::parse(doc::NEW_FILE);
    }
    let note = edit(&mut doc)?;
    let after = doc.text();
    if after == before {
        return Ok(note);
    }
    write(path, &after)?;
    let mut undo = UNDO.lock().unwrap_or_else(|e| e.into_inner());
    undo.push((path.to_path_buf(), before, after));
    if undo.len() > UNDO_DEPTH {
        undo.remove(0);
    }
    Ok(note)
}

fn texts(arg: Option<&Value>) -> Vec<String> {
    match arg {
        Some(Value::Array(list)) => list.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        Some(Value::String(one)) => vec![one.clone()],
        _ => Vec::new(),
    }
}

fn line_of(doc: &Doc, text: &str) -> Result<usize, String> {
    doc.items().into_iter().find(|i| i.text == text).map(|i| i.line).ok_or_else(|| format!("not on the list any more: {text}"))
}

/// The mod's edits: `add [text]`, `edit [from, to]`, `toggle [texts]`, `delete [texts]`, `move [text, ±1]`, `place
/// [text, index among the open ones]` (a drag), `undo []`.
/// None for an action this view does not know (`open_link` is the hub's).
pub fn act(action: &str, args: &[Value]) -> Option<Result<String, String>> {
    act_on(&todo_path(), action, args, &chrono::Local::now().format("%Y-%m-%d").to_string())
}

pub fn act_on(path: &Path, action: &str, args: &[Value], today: &str) -> Option<Result<String, String>> {
    let first = args.first().and_then(Value::as_str).unwrap_or_default().trim().to_string();
    let done = match action {
        "add" if first.is_empty() => Ok("nothing to add".into()),
        "add" => change(path, |doc| {
            doc.add(&first);
            Ok(format!("added \u{201c}{first}\u{201d}"))
        }),
        "edit" => {
            let to = args.get(1).and_then(Value::as_str).unwrap_or_default().trim().to_string();
            change(path, |doc| {
                let line = line_of(doc, &first)?;
                let done = doc.items().into_iter().any(|i| i.line == line && i.done);
                if !to.is_empty() && to != first {
                    doc.set(line, done, &to);
                }
                Ok(String::new())
            })
        }
        "toggle" => {
            let texts = texts(args.first());
            change(path, |doc| {
                // Last first: each one done goes on top of the done ones, so they keep their order.
                for text in texts.iter().rev() {
                    let line = line_of(doc, text)?;
                    doc.toggle(line, today);
                }
                Ok(if texts.len() > 1 { format!("{} to-dos toggled", texts.len()) } else { String::new() })
            })
        }
        "delete" => {
            let texts = texts(args.first());
            change(path, |doc| {
                let mut lines = texts.iter().map(|t| line_of(doc, t)).collect::<Result<Vec<_>, _>>()?;
                lines.sort_unstable_by(|a, b| b.cmp(a));
                for line in lines {
                    doc.remove(line);
                }
                Ok(if texts.len() == 1 { "deleted · u undoes".into() } else { format!("{} to-dos deleted · u undoes", texts.len()) })
            })
        }
        "move" => {
            let by = args.get(1).and_then(Value::as_i64).unwrap_or(0);
            change(path, |doc| {
                let open: Vec<usize> = doc.items().into_iter().filter(|i| !i.done).map(|i| i.line).collect();
                let line = line_of(doc, &first)?;
                let at = open.iter().position(|l| *l == line).ok_or("only open to-dos move")?;
                let target = at as i64 + by;
                if (0..open.len() as i64).contains(&target) {
                    doc.swap(line, open[target as usize]);
                }
                Ok(String::new())
            })
        }
        "place" => {
            let to = args.get(1).and_then(Value::as_u64).unwrap_or(0) as usize;
            change(path, |doc| {
                let open: Vec<usize> = doc.items().into_iter().filter(|i| !i.done).map(|i| i.line).collect();
                let line = line_of(doc, &first)?;
                if !open.contains(&line) {
                    return Err("only open to-dos move".into());
                }
                if let Some(target) = open.get(to.min(open.len().saturating_sub(1))) {
                    doc.place(line, *target);
                }
                Ok(String::new())
            })
        }
        "undo" => undo(path),
        _ => return None,
    };
    Some(done)
}

fn undo(path: &Path) -> Result<String, String> {
    let disk = std::fs::read_to_string(path).unwrap_or_default();
    let mut undo = UNDO.lock().unwrap_or_else(|e| e.into_inner());
    let Some(at) = undo.iter().rposition(|(p, _, _)| p == path) else { return Ok("nothing to undo".into()) };
    if undo[at].2 != disk {
        // Undoing past another writer's change would drop it.
        undo.retain(|(p, _, _)| p != path);
        return Ok("the list changed since: nothing to undo".into());
    }
    let (_, before, _) = undo.remove(at);
    write(path, &before)?;
    Ok("undone".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_through_the_view_and_undoes_them() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("todo.md");
        let run = |action: &str, args: Value| act_on(&path, action, args.as_array().unwrap(), "2026-11-02").unwrap().unwrap();
        run("add", json!(["ship ENG-12"]));
        run("add", json!(["write docs"]));
        run("toggle", json!([["ship ENG-12"]]));
        let config = json!({"tickets": {"keys": ["ENG"], "urlTemplate": "https://t.io/{id}"}});
        let view = model(&path, &config, &Value::Null);
        assert_eq!((view["open"].as_u64(), view["done"].as_u64()), (Some(1), Some(1)));
        let done = &view["items"][1];
        assert_eq!((done["undated"].as_str(), done["done_on"].as_str()), (Some("ship ENG-12"), Some("2026-11-02")));
        assert_eq!(done["parts"][1]["url"], "https://t.io/ENG-12");
        assert_eq!(view["can_undo"], true);
        assert_eq!(run("undo", json!([])), "undone");
        assert_eq!(model(&path, &config, &Value::Null)["done"], 0);
        run("add", json!(["third"]));
        run("move", json!(["third", 1]));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# To-do\n\n- [ ] write docs\n- [ ] third\n- [ ] ship ENG-12\n");
        run("place", json!(["ship ENG-12", 0]));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# To-do\n\n- [ ] ship ENG-12\n- [ ] write docs\n- [ ] third\n");
        run("place", json!(["ship ENG-12", 2]));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# To-do\n\n- [ ] write docs\n- [ ] third\n- [ ] ship ENG-12\n");
        run("edit", json!(["third", "3rd"]));
        assert_eq!(run("delete", json!([["3rd", "write docs"]])), "2 to-dos deleted · u undoes");
        std::fs::write(&path, "# To-do\n\n- [ ] by hand\n").unwrap();
        assert_eq!(run("undo", json!([])), "the list changed since: nothing to undo");
        assert!(act_on(&path, "open_link", &[], "x").is_none());
    }
}
