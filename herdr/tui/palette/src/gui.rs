//! The palette in the Herdr app: its pages (the doctor's report, today's brief) as view models, the agents in the order
//! every palette lists them, and what a found item does in the agent session the palette opened over, the same as the
//! TUI's ctrl keys.

use std::path::PathBuf;
use std::time::SystemTime;

use cockpit_core::files;
use cockpit_core::frontend::APP;
use cockpit_core::hub::Found;
use serde_json::{Value, json};

use crate::report::Report;
use crate::{Jump, Over, into_session, jumps, listed};

/// `{report: "doctor" | "brief"}`: that page; anything else, the palette's own commands, and with `{agents: "app" |
/// "tui" | "all", rows?}` that frontend's agents in the palette's order (the `rows` given, else the agents view built
/// here) and how many an empty query lists.
pub fn view(args: &Value, doc: &dyn Fn(&str) -> Value) -> Value {
    match args["report"].as_str() {
        Some("doctor") => {
            let exe = std::env::current_exe().unwrap_or_else(|_| "cockpit".into());
            match std::process::Command::new(exe).arg("doctor").stdin(std::process::Stdio::null()).output() {
                Ok(out) => Report::doctor_done(&String::from_utf8_lossy(&out.stdout)).json(),
                Err(error) => {
                    json!({"title": "Cockpit doctor", "rows": [{"mark": "✗", "tone": "bad", "text": format!("cockpit doctor did not run: {error}")}]})
                }
            }
        }
        Some("brief") => {
            let brief = files::load(&files::cache().join("brief.json")).unwrap_or(Value::Null);
            Report::brief(&brief, &chrono::Local::now().format("%Y-%m-%d").to_string()).json()
        }
        _ => {
            let mut out = json!({"pinned": cockpit_core::actions::pinned()});
            if let Some(frontend) = args["agents"].as_str() {
                let only = Some(frontend).filter(|f| *f != "all");
                let model = match &args["rows"] {
                    Value::Array(rows) => json!({"agents": rows}),
                    _ => cockpit_agents::gui::view(&json!({"frontend": only}), doc),
                };
                let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or_default();
                let list = jumps(&model, only, now);
                out["listed"] = json!(listed(&list));
                out["agents"] = list.iter().map(Jump::json).collect();
            }
            out
        }
    }
}

pub fn dirs() -> Vec<PathBuf> {
    Vec::new()
}

/// `insert`, `ask` or `fork` with `[over, found]`: over = {terminal, cwd, title, agent?, session?} the app's agent
/// terminal, found = an entry of the hub's search.
pub fn act(action: &str, args: &[Value]) -> Option<Result<String, String>> {
    let key = match action {
        "insert" => 'r',
        "ask" => 'a',
        "fork" => 'f',
        _ => return None,
    };
    let (Some(over), Some(found)) = (args.first(), args.get(1)) else { return Some(Err("needs [over, found]".into())) };
    let over = over_of(over);
    // Words asked of an agent reach it only from a person or its own tools; typing into its prompt takes the window and a
    // fork starts an agent, which only a person does.
    let checked = match key {
        'a' => cockpit_agents::gui::may_touch(&over.pane),
        'r' => cockpit_agents::gui::by_a_person("an agent's prompt is typed into by a person, not by an agent's tools"),
        _ => cockpit_agents::gui::by_a_person("an agent is started by a person, not by an agent's tools"),
    };
    if let Err(why) = checked {
        return Some(Err(why));
    }
    let found = Found {
        app: text(&found["app"]),
        kind: text(&found["kind"]),
        title: text(&found["title"]),
        help: text(&found["help"]),
        target: found["target"].clone(),
    };
    let inbox = files::load(&files::cache().join("inbox.json")).unwrap_or(Value::Null);
    let Some(then) = into_session(key, &over, &found, &inbox, &files::config()) else {
        return Some(Err("no agent session to fork there".into()));
    };
    // Typed into a prompt that shows a menu, the words would pick its choices: refused while the agent waits on one.
    let done = match key {
        'r' => {
            let mut failed = None;
            let typed = cockpit_agents::gui::waiting(&over.pane, &mut || match then.run() {
                Ok(()) => true,
                Err(why) => {
                    failed = Some(why);
                    false
                }
            });
            typed.map_err(|why| failed.unwrap_or(why))
        }
        _ => then.run(),
    };
    Some(done.map(|()| match key {
        'r' => format!("put \"{}\" in the prompt", found.title),
        'a' => format!("asked about \"{}\"", found.title),
        _ => format!("forked the session to look into \"{}\"", found.title),
    }))
}

fn text(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

/// The app terminal as the palette names it, its agent Claude Code unless it says; the session it runs is the label's,
/// else for Claude Code its folder's latest transcript.
pub fn over_of(over: &Value) -> Over {
    let cwd = text(&over["cwd"]);
    let agent = cockpit_core::agent::of_program(over["agent"].as_str().unwrap_or("claude")).map(|a| a.id);
    let latest = || latest_of_all(&cwd).filter(|_| agent.as_deref() == Some("claude"));
    let session = over["session"].as_str().filter(|s| !s.is_empty()).map(str::to_string).or_else(latest);
    Over { pane: format!("{APP}{}", text(&over["terminal"])), agent, session, cwd, title: text(&over["title"]) }
}

/// The newest session Claude Code keeps for this folder, across every account's folder.
fn latest_of_all(cwd: &str) -> Option<String> {
    let newest = |projects: &std::path::Path| latest_session(projects, cwd);
    cockpit_core::transcripts::roots(cockpit_core::transcripts::Transcripts::Claude)
        .iter()
        .filter_map(|projects| {
            let id = newest(projects)?;
            let at = std::fs::metadata(projects.join(claude_folder(cwd)).join(format!("{id}.jsonl"))).and_then(|m| m.modified()).ok()?;
            Some((at, id))
        })
        .max_by_key(|(at, _)| *at)
        .map(|(_, id)| id)
}

fn claude_folder(cwd: &str) -> String {
    cwd.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

/// The id of the newest transcript Claude keeps for this folder (`<projects>/<folder, each other character a dash>/`).
pub fn latest_session(projects: &std::path::Path, cwd: &str) -> Option<String> {
    if cwd.is_empty() {
        return None;
    }
    let entries = std::fs::read_dir(projects.join(claude_folder(cwd))).ok()?;
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
        assert_eq!((over.pane.as_str(), over.session.as_deref(), over.agent.as_deref()), ("app:t3", Some("4f1c-77"), Some("claude")));
        let codex = over_of(&json!({"terminal": "t4", "cwd": "/repos/api", "agent": "codex", "session": "01a1-77"}));
        assert_eq!((codex.agent.as_deref(), codex.forkable()), (Some("codex"), Some(("codex".into(), "01a1-77".into()))));
    }

    #[test]
    fn a_report_reads_as_rows_with_tones() {
        let page = Report::doctor_done("ok    herdr 0.9.5\nfail  jq is missing\n").json();
        assert_eq!(page["rows"][0], json!({"mark": "✗", "tone": "bad", "text": "jq is missing", "url": null}));
    }
}
