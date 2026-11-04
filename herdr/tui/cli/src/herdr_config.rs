//! cockpit's part of herdr's config.toml merged into the person's: every key they set stays, missing ones are added,
//! `[[keys.command]]` entries are added by `key`, and toasts are delivered by herdr so none arrives twice.

use toml_edit::{DocumentMut, InlineTable, Item, Table, Value};

pub struct Merged {
    pub text: String,
    /// Settings and key commands added.
    pub added: usize,
    /// `ui.toast.delivery` was something else and is now "herdr".
    pub delivery: bool,
}

pub fn merge(theirs: &str, ours: &str) -> Result<Merged, String> {
    let mut doc: DocumentMut = theirs.parse().map_err(|e: toml_edit::TomlError| format!("~/.config/herdr/config.toml does not parse: {e}"))?;
    let ours: DocumentMut = ours.parse().map_err(|e: toml_edit::TomlError| format!("config.example.toml does not parse: {e}"))?;
    let mut added = 0;
    merge_table(doc.as_table_mut(), ours.as_table(), &mut added);
    let delivery = doc.get("ui").and_then(|u| u.get("toast")).and_then(|t| t.get("delivery")).and_then(Item::as_str) != Some("herdr");
    if delivery {
        doc["ui"]["toast"]["delivery"] = toml_edit::value("herdr");
    }
    show_pinned(&mut doc);
    Ok(Merged { text: doc.to_string(), added, delivery })
}

fn merge_table(dst: &mut Table, src: &Table, added: &mut usize) {
    for (key, item) in src.iter() {
        match (dst.get_mut(key), item) {
            (None, _) => {
                dst.insert(key, item.clone());
                *added += settings(item);
            }
            (Some(Item::Table(d)), Item::Table(s)) => merge_table(d, s, added),
            (Some(Item::ArrayOfTables(d)), Item::ArrayOfTables(s)) => {
                let have: Vec<String> = d.iter().filter_map(|t| t.get("key").and_then(Item::as_str).map(str::to_string)).collect();
                for table in s.iter() {
                    if table.get("key").and_then(Item::as_str).is_none_or(|k| !have.iter().any(|h| h == k)) {
                        d.push(table.clone());
                        *added += 1;
                    }
                }
            }
            _ => {}
        }
    }
}

/// How many settings an added item holds: each value of a table, each entry of an array of tables.
fn settings(item: &Item) -> usize {
    match item {
        Item::Table(t) => t.iter().map(|(_, i)| settings(i)).sum(),
        Item::ArrayOfTables(a) => a.len(),
        Item::None => 0,
        Item::Value(_) => 1,
    }
}

/// The sidebar's `workspace` token shows pinned spaces only through a `{ contains = "  " }` rule ahead of the hiding one.
fn show_pinned(doc: &mut DocumentMut) {
    let Some(rows) = doc.get_mut("ui").and_then(|u| u.get_mut("sidebar")).and_then(|s| s.get_mut("spaces")).and_then(|s| s.get_mut("rows")) else {
        return;
    };
    let Some(rows) = rows.as_array_mut() else { return };
    for row in rows.iter_mut() {
        let Some(row) = row.as_array_mut() else { continue };
        for cell in row.iter_mut() {
            let Some(cell) = cell.as_inline_table_mut() else { continue };
            if cell.get("token").and_then(Value::as_str) != Some("workspace") {
                continue;
            }
            let Some(rules) = cell.get_mut("rules").and_then(Value::as_array_mut) else { continue };
            let shown = rules.iter().any(|r| r.as_inline_table().and_then(|t| t.get("contains")).and_then(Value::as_str) == Some("  "));
            if !shown {
                let mut rule = InlineTable::new();
                rule.insert("contains", "  ".into());
                rules.insert(0, rule);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OURS: &str = r#"
[ui]
sidebar_width = 34
[ui.toast]
delivery = "herdr"
[[keys.command]]
key = "prefix+space"
command = "palette"
[[keys.command]]
key = "prefix+i"
command = "inbox"
"#;

    #[test]
    fn their_keys_stay_and_ours_are_added_once() {
        let theirs = "# mine\n[ui]\nsidebar_width = 50\n[ui.toast]\ndelivery = \"system\"\n[[keys.command]]\nkey = \"prefix+i\"\ncommand = \"mine\"\n";
        let m = merge(theirs, OURS).unwrap();
        let doc: DocumentMut = m.text.parse().unwrap();
        assert!(m.text.starts_with("# mine"));
        assert_eq!(doc["ui"]["sidebar_width"].as_integer(), Some(50));
        assert_eq!(doc["ui"]["toast"]["delivery"].as_str(), Some("herdr"));
        assert!(m.delivery);
        let commands: Vec<&str> = doc["keys"]["command"].as_array_of_tables().unwrap().iter().map(|t| t["command"].as_str().unwrap()).collect();
        assert_eq!(commands, ["mine", "palette"]);
        assert_eq!(m.added, 1);
        let again = merge(&m.text, OURS).unwrap();
        assert_eq!((again.text, again.added, again.delivery), (m.text, 0, false));
    }

    #[test]
    fn an_empty_config_takes_everything_and_pinned_spaces_show() {
        let m = merge("", OURS).unwrap();
        assert!(m.text.contains("sidebar_width = 34") && m.text.contains("prefix+space"));
        let theirs = "[ui.sidebar.spaces]\nrows = [[{ token = \"workspace\", rules = [{ contains = \"\", hide = true }] }]]\n";
        let doc: DocumentMut = merge(theirs, "").unwrap().text.parse().unwrap();
        let rules = doc["ui"]["sidebar"]["spaces"]["rows"].as_array().unwrap().get(0).unwrap().as_array().unwrap().get(0).unwrap();
        let first = rules.as_inline_table().unwrap().get("rules").unwrap().as_array().unwrap().get(0).unwrap();
        assert_eq!(first.as_inline_table().unwrap().get("contains").and_then(Value::as_str), Some("  "));
    }
}
