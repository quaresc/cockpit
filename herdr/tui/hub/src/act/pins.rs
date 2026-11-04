//! The pinned apps' order, dragged in the Herdr app's sidebar: the config's `spaces` list follows it, and herdr's pinned
//! spaces are moved to match.

use cockpit_core::files;
use serde_json::Value;

use super::{Ctx, arg};
use crate::value::{Res, text};

/// The spaces with the named ones in their new order, each in a slot one of them held, the others where they were.
pub fn reordered(spaces: &[Value], order: &[String]) -> Vec<Value> {
    let name = |s: &Value| if s.is_object() { text(&s["name"]) } else { text(s) };
    let moved: Vec<Value> = order.iter().filter_map(|n| spaces.iter().find(|s| name(s) == *n).cloned()).collect();
    let mut next = moved.into_iter();
    spaces.iter().map(|s| if order.contains(&name(s)) { next.next().unwrap_or_else(|| s.clone()) } else { s.clone() }).collect()
}

/// `pin_order [[names]]`.
pub fn pin_order(ctx: &mut Ctx, args: &[Value]) -> Res {
    let order: Vec<String> = arg(args, 0)?.as_array().ok_or("pin_order takes the list of names")?.iter().map(text).collect();
    let path = files::config_dir().join("config.json");
    let mut config = files::load(&path).ok_or("no config.json")?;
    let spaces = config["spaces"].as_array().cloned().unwrap_or_else(|| ctx.cfg.space_names().into_iter().map(Value::from).collect());
    config["spaces"] = Value::Array(reordered(&spaces, &order));
    write_private(&path, &config).map_err(|e| format!("config.json not written: {e}"))?;
    let script = crate::config::script("pin-spaces.sh");
    let _ = std::process::Command::new("sh").arg(script).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
    ctx.note("pinned spaces reordered".to_string());
    Ok(())
}

/// The config written through a file only its owner reads, as it holds tokens.
fn write_private(path: &std::path::Path, value: &Value) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let tmp = path.with_file_name(".config.json.tmp");
    let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
    file.write_all(format!("{}\n", serde_json::to_string_pretty(value).unwrap_or_default()).as_bytes())?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn moved_spaces_take_each_others_slots() {
        let spaces = vec![json!("inbox"), json!("terminal"), json!("todo"), json!({"name": "notes"}), json!("meetings"), json!("mail")];
        let order = ["mail", "inbox", "todo", "meetings"].map(String::from);
        let next = reordered(&spaces, &order);
        assert_eq!(next, vec![json!("mail"), json!("terminal"), json!("inbox"), json!({"name": "notes"}), json!("todo"), json!("meetings")]);
    }
}
