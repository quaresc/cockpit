//! The palette in the Herdr app: its pages (the doctor's report, today's brief) as view models, and what a found item
//! does in the Claude session the palette opened over, the same as the TUI's ctrl keys.

use std::path::PathBuf;
use std::time::SystemTime;

use cockpit_core::files;
use cockpit_core::frontend::APP;
use cockpit_core::hub::Found;
use serde_json::{Value, json};

use crate::report::Report;
use crate::{Over, into_session};

/// `{report: "doctor" | "brief"}`: that page; anything else, the palette's own commands.
pub fn view(args: &Value, _doc: &dyn Fn(&str) -> Value) -> Value {
    match args["report"].as_str() {
        Some("doctor") => {
            let exe = std::env::current_exe().unwrap_or_else(|_| "cockpit".into());
            match std::process::Command::new(exe).arg("doctor").stdin(std::process::Stdio::null()).output() {
                Ok(out) => Report::doctor_done(&String::from_utf8_lossy(&out.stdout)).json(),
                Err(error) => json!({"title": "Cockpit doctor", "rows": [{"mark": "✗", "tone": "bad", "text": format!("cockpit doctor did not run: {error}")}]}),
            }
        }
        Some("brief") => {
            let brief = files::load(&files::cache().join("brief.json")).unwrap_or(Value::Null);
            Report::brief(&brief, &chrono::Local::now().format("%Y-%m-%d").to_string()).json()
        }
        _ => json!({"pinned": cockpit_core::actions::pinned()}),
    }
}

pub fn dirs() -> Vec<PathBuf> {
    Vec::new()
}

/// `insert`, `ask` or `fork` with `[over, found]`: over = {terminal, cwd, title, session?} the app's Claude terminal,
/// found = an entry of the hub's search.
pub fn act(action: &str, args: &[Value]) -> Option<Result<String, String>> {
    let key = match action {
        "insert" => 'r',
        "ask" => 'a',
        "fork" => 'f',
        _ => return None,
    };
    let (Some(over), Some(found)) = (args.first(), args.get(1)) else { return Some(Err("needs [over, found]".into())) };
    let over = over_of(over);
    let found = Found {
        app: text(&found["app"]),
        kind: text(&found["kind"]),
        title: text(&found["title"]),
        help: text(&found["help"]),
        target: found["target"].clone(),
    };
    let inbox = files::load(&files::cache().join("inbox.json")).unwrap_or(Value::Null);
    let Some(then) = into_session(key, &over, &found, &inbox, &files::config()) else {
        return Some(Err("no Claude session to fork there".into()));
    };
    Some(then.run().map(|()| match key {
        'r' => format!("put \"{}\" in the prompt", found.title),
        'a' => format!("asked about \"{}\"", found.title),
        _ => format!("forked the session to look into \"{}\"", found.title),
    }))
}

fn text(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

/// The app terminal as the palette names it; the session it runs is the label's, else its folder's latest transcript.
pub fn over_of(over: &Value) -> Over {
    let cwd = text(&over["cwd"]);
    let session = over["session"].as_str().filter(|s| !s.is_empty()).map(str::to_string).or_else(|| latest_session(&projects_dir(), &cwd));
    Over { pane: format!("{APP}{}", text(&over["terminal"])), is_claude: true, session, cwd, title: text(&over["title"]) }
}

fn projects_dir() -> PathBuf {
    std::env::var("CLAUDE_CONFIG_DIR").map(PathBuf::from).unwrap_or_else(|_| files::home().join(".claude")).join("projects")
}

/// The id of the newest transcript Claude keeps for this folder (`<projects>/<folder, each other character a dash>/`).
pub fn latest_session(projects: &std::path::Path, cwd: &str) -> Option<String> {
    if cwd.is_empty() {
        return None;
    }
    let folder: String = cwd.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let entries = std::fs::read_dir(projects.join(folder)).ok()?;
    entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok().unwrap_or(SystemTime::UNIX_EPOCH), e.path())))
        .max_by_key(|(at, _)| *at)
        .and_then(|(_, p)| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_session_is_the_folders_newest_transcript() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("-repos-api-v2");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("old-1.jsonl"), "{}").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(folder.join("new-2.jsonl"), "{}").unwrap();
        assert_eq!(latest_session(dir.path(), "/repos/api.v2"), Some("new-2".into()));
        assert_eq!(latest_session(dir.path(), "/elsewhere"), None);
        let over = over_of(&json!({"terminal": "t3", "cwd": "/repos/api", "title": "Sign-in", "session": "4f1c-77"}));
        assert_eq!((over.pane.as_str(), over.session.as_deref()), ("app:t3", Some("4f1c-77")));
    }

    #[test]
    fn a_report_reads_as_rows_with_tones() {
        let page = Report::doctor_done("ok    herdr 0.9.5\nfail  jq is missing\n").json();
        assert_eq!(page["rows"][0], json!({"mark": "✗", "tone": "bad", "text": "jq is missing", "url": null}));
    }
}
