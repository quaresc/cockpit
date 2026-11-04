//! The show tool for any agent: the view checked as the plugin checks it, written to ~/.cache/cockpit/show and opened
//! beside the agent (`cockpit show` in herdr, the mod's show view in the Herdr app), its answers sent back as prompts.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use cockpit_core::files;
use cockpit_core::frontend::{self, Frontend};
use regex::Regex;
use serde_json::{Map, Value, json};

use super::tools::{BLOCKS, PLACEMENTS};

const NODE_KINDS: [&str; 3] = ["box", "round", "decision"];
const CHART_KINDS: [&str; 4] = ["line", "area", "bar", "donut"];
const CHART_FLAGS: [&str; 4] = ["horizontal", "stacked", "percent", "show_values"];
const MAX_STATS: usize = 8;
const MAX_NODES: usize = 60;
const MAX_ROWS: usize = 5000;
const MAX_PATCH: usize = 500_000;

type Block = Map<String, Value>;

fn list(values: &[&str]) -> String {
    values.iter().map(|v| Value::from(*v).to_string()).collect::<Vec<_>>().join(", ")
}

fn quoted(v: Option<&Value>) -> String {
    v.map_or("undefined".into(), Value::to_string)
}

fn is_text(v: Option<&Value>) -> bool {
    v.is_some_and(Value::is_string)
}

fn text(v: Option<&Value>) -> &str {
    v.and_then(Value::as_str).unwrap_or_default()
}

fn is_count(v: Option<&Value>) -> bool {
    v.is_none_or(|v| v.as_f64().is_some_and(|n| n >= 1.0))
}

fn is_flowchart(v: Option<&Value>) -> bool {
    static FLOWCHART: OnceLock<Regex> = OnceLock::new();
    let re = FLOWCHART.get_or_init(|| Regex::new(r"^(?:---\n[\s\S]*?\n---\s*)?(?:%%.*\n\s*)*(?:flowchart|graph)\b").unwrap());
    v.and_then(Value::as_str).is_some_and(|t| re.is_match(t.trim()))
}

fn is_plain(v: &Value) -> bool {
    !v.is_object() && !v.is_array()
}

/// The first rule a block breaks, said with its path and what would be accepted; None when it is fine.
fn block_error(block: &Block, at: &str) -> Option<String> {
    let kind = text(block.get("type"));
    if !BLOCKS.contains(&kind) {
        return Some(format!("at {at}: unknown block type {}; allowed: {}", quoted(block.get("type")), list(&BLOCKS)));
    }
    if block.get("title").is_some() && !is_text(block.get("title")) {
        return Some(format!("at {at}/title: a title is a string"));
    }
    match kind {
        "blueprint" => {
            return (!is_flowchart(block.get("text"))).then(|| {
                format!("at {at}/text: a blueprint is a Mermaid flowchart: flowchart TD, then its boxes and edges (a --- title header may come first)")
            });
        }
        "markdown" | "mermaid" => return (!is_text(block.get("text"))).then(|| format!("at {at}/text: {kind} needs its text as a string")),
        "flow" => return flow_error(block, at),
        "sequence" => return sequence_error(block, at),
        "choice" => return choice_error(block, at),
        "confirm" => return (!is_text(block.get("summary"))).then(|| format!("at {at}/summary: what is being confirmed, as Markdown")),
        "ci" => return (!(is_text(block.get("repo")) && is_text(block.get("branch")))).then(|| format!("at {at}: a ci block is {{repo, branch}}")),
        "log" => {
            if text(block.get("command")).is_empty() {
                return Some(format!("at {at}/command: the shell command to tail"));
            }
            return (!(is_count(block.get("every")) && is_count(block.get("lines")))).then(|| format!("at {at}: every and lines are numbers of at least 1"));
        }
        "compare" => {
            let picture =
                |v: Option<&Value>| v.and_then(Value::as_str).is_some_and(|t| t.starts_with('/') || t.starts_with("http://") || t.starts_with("https://"));
            return (!(picture(block.get("before")) && picture(block.get("after"))))
                .then(|| format!("at {at}: a compare block is {{before, after}}, each an absolute path or an http(s) URL"));
        }
        "image" => {
            let has_path = text(block.get("path")).starts_with('/');
            let url = text(block.get("url"));
            let has_url = url.starts_with("http://") || url.starts_with("https://");
            if has_path == has_url {
                return Some(format!(
                    "at {at}: an image takes either path (an absolute path) or url (http or https), not {}",
                    if has_path { "both" } else { "neither" }
                ));
            }
            return (!is_count(block.get("width"))).then(|| format!("at {at}/width: a number of columns"));
        }
        "diff" => {
            let has_patch = is_text(block.get("patch"));
            let has_repo = text(block.get("repo")).starts_with('/');
            if has_patch == has_repo {
                return Some(format!(
                    "at {at}: a diff takes either patch (a unified diff) or repo (an absolute path), not {}",
                    if has_patch { "both" } else { "neither" }
                ));
            }
            if has_patch && text(block.get("patch")).chars().count() > MAX_PATCH {
                return Some(format!("at {at}/patch: at most {MAX_PATCH} characters; show a repo diff instead"));
            }
            static REF: OnceLock<Regex> = OnceLock::new();
            let re = REF.get_or_init(|| Regex::new(r"^[\w.~^@{}/][\w./~^@{}-]*$").unwrap());
            if block.get("base").is_some() && !block.get("base").and_then(Value::as_str).is_some_and(|b| re.is_match(b)) {
                return Some(format!("at {at}/base: a git ref such as main or HEAD~3"));
            }
            return None;
        }
        "stats" => return stats_error(block, at),
        "chart" => {
            if !CHART_KINDS.contains(&text(block.get("kind"))) {
                return Some(format!("at {at}/kind: a chart is one of {}", list(&CHART_KINDS)));
            }
            if let Some(flag) = CHART_FLAGS.iter().find(|f| block.get(**f).is_some_and(|v| !v.is_boolean())) {
                return Some(format!("at {at}/{flag}: true or false"));
            }
            if block.get("horizontal") == Some(&Value::Bool(true)) && text(block.get("kind")) != "bar" {
                return Some(format!("at {at}/horizontal: only bars lie down; use kind \"bar\""));
            }
        }
        _ => {}
    }
    // A table or chart gets its rows from the call or from a command; a source needs a Claude session's connectors.
    if block.get("source").is_some() {
        return Some(format!(
            "at {at}/source: Metabase and Datadog sources refresh through a Claude session's connectors; give rows, or a command printing JSON"
        ));
    }
    if let Some(command) = block.get("command") {
        if !command.as_str().is_some_and(|c| !c.is_empty()) {
            return Some(format!("at {at}/command: a shell command printing JSON"));
        }
        return (!is_count(block.get("every"))).then(|| format!("at {at}/every: a number of seconds"));
    }
    let columns: Vec<&str> = match block.get("columns").and_then(Value::as_array) {
        Some(c) if !c.is_empty() && c.iter().all(Value::is_string) => c.iter().filter_map(Value::as_str).collect(),
        _ => return Some(format!("at {at}/columns: a non-empty list of column names, or a command or source to get them from")),
    };
    let Some(rows) = block.get("rows").and_then(Value::as_array) else {
        return Some(format!("at {at}/rows: a list of rows, each a list of {} values", columns.len()));
    };
    if rows.len() > MAX_ROWS {
        return Some(format!("at {at}/rows: at most {MAX_ROWS} rows; aggregate in the query first"));
    }
    if let Some(bad) = rows.iter().position(|r| !r.as_array().is_some_and(|r| r.len() == columns.len() && r.iter().all(is_plain))) {
        return Some(format!("at {at}/rows/{bad}: each row is a list of {} plain values ({})", columns.len(), list(&columns)));
    }
    if kind == "table" {
        return None;
    }
    if let Some(x) = block.get("x")
        && !x.as_str().is_some_and(|x| columns.contains(&x))
    {
        return Some(format!("at {at}/x: one of the columns: {}", list(&columns)));
    }
    let x = block.get("x").and_then(Value::as_str).unwrap_or(columns[0]);
    let y: Vec<Value> = match block.get("y") {
        Some(Value::Array(y)) if !y.is_empty() => y.clone(),
        Some(_) => return Some(format!("at {at}/y: a non-empty list of the columns to draw")),
        None => columns.iter().filter(|c| **c != x).map(|c| Value::from(*c)).collect(),
    };
    if y.is_empty() {
        return Some(format!("at {at}/y: a non-empty list of the columns to draw"));
    }
    if let Some(missing) = y.iter().find(|c| !c.as_str().is_some_and(|c| columns.contains(&c))) {
        return Some(format!("at {at}/y: {missing} is not one of the columns: {}", list(&columns)));
    }
    let indexes: Vec<usize> = y.iter().filter_map(|c| columns.iter().position(|x| Some(*x) == c.as_str())).collect();
    let textual = indexes.into_iter().find(|i| rows.iter().any(|r| r.get(*i).is_some_and(|v| !v.is_null() && !v.is_number())));
    textual.map(|i| format!("at {at}/y: column {} holds values that are not numbers; cast them in the query", Value::from(columns[i])))
}

fn stats_error(block: &Block, at: &str) -> Option<String> {
    let items = block.get("items").and_then(Value::as_array).filter(|i| !i.is_empty() && i.len() <= MAX_STATS);
    let Some(items) = items else {
        return Some(format!("at {at}/items: a list of 1 to {MAX_STATS} figures, each {{label, value, note?, delta?, good?, spark?}}"));
    };
    for (n, item) in items.iter().enumerate() {
        let get = |k: &str| item.get(k);
        if !is_text(get("label")) || !get("value").is_some_and(|v| v.is_string() || v.is_number()) {
            return Some(format!("at {at}/items/{n}: a figure is {{label, value}}, the label a string, the value a string or a number"));
        }
        if get("note").is_some_and(|v| !v.is_string()) || get("delta").is_some_and(|v| !v.is_string() && !v.is_number()) {
            return Some(format!("at {at}/items/{n}: note is a string, delta a string such as \"+4%\" or a number"));
        }
        if get("good").is_some_and(|v| !v.is_boolean()) {
            return Some(format!("at {at}/items/{n}/good: true when the delta is good news, false when bad"));
        }
        if get("spark").is_some_and(|v| !v.as_array().is_some_and(|s| s.iter().all(Value::is_number))) {
            return Some(format!("at {at}/items/{n}/spark: a list of numbers"));
        }
    }
    None
}

fn flow_error(block: &Block, at: &str) -> Option<String> {
    if block.get("direction").is_some_and(|d| d != "down" && d != "right") {
        return Some(format!("at {at}/direction: \"down\" or \"right\""));
    }
    let nodes = block.get("nodes").and_then(Value::as_array).filter(|n| !n.is_empty() && n.len() <= MAX_NODES);
    let Some(nodes) = nodes else { return Some(format!("at {at}/nodes: a list of 1 to {MAX_NODES} nodes, each {{id, label, kind?}}")) };
    let mut ids: Vec<&str> = Vec::new();
    for (n, node) in nodes.iter().enumerate() {
        let id = node.get("id").and_then(Value::as_str).filter(|i| !i.is_empty());
        let Some(id) = id.filter(|_| is_text(node.get("label"))) else { return Some(format!("at {at}/nodes/{n}: a node is {{id, label}} with both strings")) };
        if ids.contains(&id) {
            return Some(format!("at {at}/nodes/{n}/id: {} is already the id of another node", Value::from(id)));
        }
        if node.get("kind").is_some_and(|k| !k.as_str().is_some_and(|k| NODE_KINDS.contains(&k))) {
            return Some(format!("at {at}/nodes/{n}/kind: one of {}", list(&NODE_KINDS)));
        }
        ids.push(id);
    }
    let empty = Vec::new();
    let edges = match block.get("edges") {
        None => &empty,
        Some(Value::Array(e)) => e,
        Some(_) => return Some(format!("at {at}/edges: a list of {{from, to, label?}}")),
    };
    for (n, edge) in edges.iter().enumerate() {
        for name in ["from", "to"] {
            let end = edge.get(name);
            if !end.and_then(Value::as_str).is_some_and(|e| ids.contains(&e)) {
                return Some(format!("at {at}/edges/{n}/{name}: {} is not a node id; ids: {}", quoted(end), list(&ids)));
            }
        }
        if edge.get("label").is_some_and(|l| !l.is_string()) {
            return Some(format!("at {at}/edges/{n}/label: a string"));
        }
    }
    None
}

fn sequence_error(block: &Block, at: &str) -> Option<String> {
    let actor = |a: &Value| a.is_string() || (a.get("id").is_some_and(Value::is_string) && a.get("label").is_none_or(Value::is_string));
    if block.get("actors").is_some_and(|a| !a.as_array().is_some_and(|a| a.iter().all(actor))) {
        return Some(format!("at {at}/actors: a list of names, or of {{id, label}}"));
    }
    let messages = block.get("messages").and_then(Value::as_array).filter(|m| !m.is_empty());
    let Some(messages) = messages else { return Some(format!("at {at}/messages: a non-empty list of {{from, to, label}} or {{note, over}}")) };
    for (n, m) in messages.iter().enumerate() {
        let is_note = is_text(m.get("note")) && is_text(m.get("over"));
        let is_message = is_text(m.get("from")) && is_text(m.get("to")) && m.get("label").is_none_or(Value::is_string);
        if !is_note && !is_message {
            return Some(format!("at {at}/messages/{n}: a message is {{from, to, label, reply?}}, a note {{note, over}}"));
        }
    }
    None
}

fn choice_error(block: &Block, at: &str) -> Option<String> {
    let options = block.get("options").and_then(Value::as_array).filter(|o| o.len() >= 2);
    let Some(options) = options else { return Some(format!("at {at}/options: at least two {{label, value?, preview?}}")) };
    let bad = options.iter().position(|o| !is_text(o.get("label")) || o.get("preview").is_some_and(|p| !p.is_string()));
    bad.map(|n| format!("at {at}/options/{n}: an option is {{label, value?, preview?}} with label and preview strings"))
}

pub fn view_error(input: &Value) -> Option<String> {
    static PANE: OnceLock<Regex> = OnceLock::new();
    let pane = PANE.get_or_init(|| Regex::new(r"^[a-z0-9][a-z0-9-]{0,39}$").unwrap());
    if !input["pane"].as_str().is_some_and(|p| pane.is_match(p)) {
        return Some("at /pane: a short name of lowercase letters, digits and dashes, such as quota-chart".into());
    }
    if !input["title"].is_string() {
        return Some("at /title: a string".into());
    }
    if input.get("url").is_some() && !input["url"].as_str().is_some_and(|u| u.starts_with("http://") || u.starts_with("https://")) {
        return Some("at /url: an http or https URL".into());
    }
    if input.get("placement").is_some() && !input["placement"].as_str().is_some_and(|p| PLACEMENTS.contains(&p)) {
        return Some(format!("at /placement: one of {}", list(&PLACEMENTS)));
    }
    let Some(blocks) = input["blocks"].as_array().filter(|b| !b.is_empty()) else { return Some("at /blocks: a non-empty list of blocks".into()) };
    if blocks.iter().filter(|b| b["type"] == "blueprint").count() > 1 {
        return Some("at /blocks: one blueprint per view".into());
    }
    for (n, block) in blocks.iter().enumerate() {
        let Some(block) = block.as_object() else { return Some(format!("at /blocks/{n}: a block is an object with a type")) };
        if let Some(error) = block_error(block, &format!("/blocks/{n}")) {
            return Some(error);
        }
    }
    None
}

/// The shell commands a call would have the pane run on its own.
pub fn commands_of(input: &Value) -> Vec<String> {
    input["blocks"].as_array().into_iter().flatten().filter_map(|b| b["command"].as_str().map(str::to_string)).collect()
}

/// A view holding a blueprint is its session's own: the name carries the session's, so two sessions' plans never meet.
pub fn blueprint_name(pane: &str, session: &str) -> String {
    let own: String = session.to_lowercase().chars().filter(char::is_ascii_alphanumeric).take(8).collect();
    if own.is_empty() { pane.into() } else { format!("{}-{own}", &pane[..pane.len().min(31)]) }
}

pub fn dir() -> PathBuf {
    files::cache().join("show")
}

// ---------------------------------------------------------------------------------------------------------------------
// Where the agent runs, and the pane beside it

/// The agent's place: a herdr pane, or a Herdr app terminal with the app's pane that holds it.
#[derive(Clone, Debug, PartialEq)]
pub enum Here {
    Tui { pane: String },
    App { terminal: String, pane: String },
}

impl Here {
    pub fn terminal(&self) -> Option<&str> {
        match self {
            Here::App { terminal, .. } => Some(terminal),
            Here::Tui { .. } => None,
        }
    }
}

/// Where this server's agent sits, from the variables herdr and the Herdr app give their terminals.
pub fn here() -> Option<Here> {
    let terminal = std::env::var("HC_TERMINAL_ID").ok().filter(|t| !t.is_empty());
    let herdr_pane = std::env::var("HERDR_PANE_ID").ok().filter(|p| !p.is_empty() && std::env::var("HERDR_ENV").is_ok_and(|e| e == "1"));
    match (terminal, herdr_pane) {
        (Some(terminal), pane) if pane.is_none() || frontend::detect() == Frontend::App => {
            let pane = app_pane(&terminal)?;
            Some(Here::App { terminal, pane })
        }
        (_, Some(pane)) => Some(Here::Tui { pane }),
        _ => None,
    }
}

/// The app's pane holding a terminal, from `herdr-app ls`.
fn app_pane(terminal: &str) -> Option<String> {
    let spaces = frontend::app_spaces();
    let found = spaces
        .iter()
        .flat_map(|s| s["tabs"].as_array().into_iter().flatten())
        .flat_map(|t| t["terminals"].as_array().into_iter().flatten())
        .find(|t| t["terminal"] == terminal);
    found.and_then(|t| t["pane"].as_str()).map(str::to_string)
}

/// Whether a pane shows the view now: the app's view follows the hub while it is drawn, `cockpit show` keeps a pid file.
fn is_open(dir: &Path, name: &str) -> bool {
    if dir.join(format!("{name}.app")).exists() {
        let shown = cockpit_core::hub::request(&json!({"op": "shown", "view": "show"}), Duration::from_secs(5)).unwrap_or_default();
        return shown["args"].as_array().into_iter().flatten().any(|a| a["name"] == name);
    }
    let pid = std::fs::read_to_string(dir.join(format!("{name}.pid"))).unwrap_or_default();
    // SAFETY: signal 0 only asks whether the process exists.
    pid.trim().parse::<i32>().is_ok_and(|pid| pid > 0 && unsafe { libc::kill(pid, 0) } == 0)
}

fn open_pane(dir: &Path, here: &Here, name: &str, session: &str, placement: &str) -> Result<(), String> {
    let direction = if placement == "down" { "down" } else { "right" };
    match here {
        Here::App { terminal, pane } => {
            let props = json!({"app": "show", "name": name, "session": session});
            let made = frontend::app_do("pane.split", &json!({"direction": direction, "pane": pane, "mod": "cockpit", "props": props}));
            let Some(new) = made.as_ref().and_then(|m| m["pane"].as_str()).map(str::to_string) else {
                return Err("herdr-app pane.split failed: is the cockpit mod installed? (cockpit app install)".into());
            };
            files::write(&dir.join(format!("{name}.app")), &json!({"pane": new, "terminal": terminal})).map_err(|e| e.to_string())?;
            // The keyboard stays with the conversation.
            if placement == "zoomed" {
                frontend::app_do("pane.zoom", &json!({"pane": new}))
            } else {
                frontend::app_do("pane.focus", &json!({"terminal": terminal}))
            };
            Ok(())
        }
        Here::Tui { pane } => {
            let _ = std::fs::remove_file(dir.join(format!("{name}.app")));
            let split = frontend::herdr(&["pane", "split", pane, "--direction", direction]);
            let Some(new) = split.as_ref().and_then(|s| s["pane"]["pane_id"].as_str()).map(str::to_string) else {
                return Err("herdr pane split failed".into());
            };
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            // exec, so the pane closes with the app.
            frontend::herdr(&["pane", "run", &new, &format!("exec {} show {}", crate::proc::quote(&exe.to_string_lossy()), crate::proc::quote(name))]);
            if placement == "zoomed" {
                frontend::herdr(&["pane", "zoom", &new, "--on"]);
            } else {
                frontend::herdr(&["pane", "focus", "--pane", &new, "--direction", if placement == "down" { "up" } else { "left" }]);
            }
            Ok(())
        }
    }
}

/// Writes the view and makes sure a pane shows it: the open one redraws, else one opens beside the agent.
pub fn show(input: &Value, here: Option<&Here>, session: &str) -> Result<String, String> {
    if let Some(error) = view_error(input) {
        return Err(error);
    }
    let dir = dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let blocks = input["blocks"].as_array().cloned().unwrap_or_default();
    let has_plan = blocks.iter().any(|b| b["type"] == "blueprint");
    let pane = input["pane"].as_str().unwrap_or_default();
    let name = if has_plan { blueprint_name(pane, session) } else { pane.to_string() };
    // A browser page is the Claude plugin's: here the view opens in a pane on the right.
    let asked = input["placement"].as_str().unwrap_or("right");
    let placement = if asked == "browser" { "right" } else { asked };
    let cwd = std::env::current_dir().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
    let mut spec = json!({"title": input["title"], "blocks": blocks, "at": crate::find::now_ms(), "session": session, "placement": placement, "cwd": cwd});
    if let Some(url) = input["url"].as_str() {
        spec["url"] = url.into();
    }
    if let Some(terminal) = here.and_then(Here::terminal) {
        spec["terminal"] = terminal.into();
    }
    let plan = dir.join(format!("{name}.mmd"));
    if has_plan {
        // The call's text is the plan until the agent writes the file.
        let _ = std::fs::remove_file(&plan);
    }
    files::write(&dir.join(format!("{name}.json")), &spec).map_err(|e| format!("the view was not written: {e}"))?;
    let mut said = if is_open(&dir, &name) {
        format!("Updated the {name} pane.")
    } else if let Some(here) = here {
        open_pane(&dir, here, &name, session, placement)?;
        format!("Opened the {name} pane beside this session; q hides it.")
    } else {
        format!("Wrote the view to {}, but this session is in neither herdr nor the Herdr app, so no pane opened.", dir.join(format!("{name}.json")).display())
    };
    let commands = commands_of(input);
    if !commands.is_empty() {
        said += &format!(" The pane runs, again and again: {}.", commands.join(" ; "));
    }
    if asked == "browser" {
        said += " Browser pages are drawn by the Claude plugin only, so it shows in a pane.";
    }
    if has_plan {
        said += &format!(
            " Keep the blueprint current: as steps start and finish, write the whole flowchart to {} (class X done, class Y active); each save redraws the pane, with no further show call. A box the person clicks comes back as their next message.",
            plan.display()
        );
    }
    Ok(said)
}

// ---------------------------------------------------------------------------------------------------------------------
// Answers given in a pane, handed to the agent as its next prompt

/// Answers a block takes again and again: a diff's comments, a blueprint's change requests.
const REPEATED: [&str; 3] = ["comment", "review", "blueprint"];

/// What the person's answer says to the agent, in their words.
pub fn answer_text(name: &str, answer: &Value, block: Option<&Value>) -> String {
    let label = text(answer.get("label"));
    match text(answer.get("type")) {
        "comment" => {
            let v = &answer["value"];
            format!("In the {name} pane I commented on {}:{}: {}", text(v.get("path")), v["line"], text(v.get("text")))
        }
        "review" => format!("In the {name} pane I commented on the diff: {label}"),
        "blueprint" => {
            let v = &answer["value"];
            let node = text(v.get("node"));
            let boxed = if !node.is_empty() && node != label { format!(" (box {node})") } else { String::new() };
            format!("In the blueprint, about «{label}»{boxed}: {}", text(v.get("text")))
        }
        kind => {
            let asked = block.and_then(|b| ["prompt", "summary", "title"].iter().find_map(|k| b[*k].as_str())).unwrap_or_default();
            if kind == "confirm" {
                let first = asked.lines().next().unwrap_or_default();
                let what = if answer["value"].as_bool().unwrap_or(false) { "confirmed" } else { "declined" };
                format!("In the {name} pane I {what}{}.", if asked.is_empty() { String::new() } else { format!(": {first}") })
            } else {
                let asked = asked.trim_end_matches(['.', '?', '!', ':']);
                format!("In the {name} pane I picked \"{label}\"{}.", if asked.is_empty() { String::new() } else { format!(" for: {asked}") })
            }
        }
    }
}

/// Takes the answers waiting for this session's views: each is removed, a question's answer written back into its view
/// so a redrawn pane does not ask again, and the texts returned in the order found.
pub fn take_answers(dir: &Path, session: &str) -> Vec<String> {
    static ANSWER: OnceLock<Regex> = OnceLock::new();
    let re = ANSWER.get_or_init(|| Regex::new(r"^([a-z0-9][a-z0-9-]*)\.(\d+)\.answer$").unwrap());
    let mut names: Vec<String> =
        std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| re.is_match(n)).collect();
    names.sort();
    let mut out = Vec::new();
    for file in names {
        let Some(found) = re.captures(&file) else { continue };
        let view = found[1].to_string();
        let spec_path = dir.join(format!("{view}.json"));
        let Some(mut spec) = files::load(&spec_path) else { continue };
        if spec["session"] != session {
            continue;
        }
        let path = dir.join(&file);
        let answer = files::load(&path);
        let _ = std::fs::remove_file(&path);
        let Some(answer) = answer else { continue };
        let n = answer["block"].as_u64().unwrap_or(u64::MAX) as usize;
        let block = spec["blocks"].get(n).cloned();
        if let Some(b) = spec["blocks"].get_mut(n)
            && !REPEATED.contains(&text(answer.get("type")))
        {
            b["answered"] = answer["label"].clone();
            let _ = files::write(&spec_path, &spec);
        }
        out.push(answer_text(&view, &answer, block.as_ref()));
    }
    out
}

/// Hands a text to the agent at `here` as its next prompt.
pub fn prompt(here: &Here, text: &str) -> bool {
    match here {
        Here::App { terminal, .. } => frontend::app_prompt(terminal, text),
        Here::Tui { pane } => frontend::herdr(&["agent", "prompt", pane, text]).is_some(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_view_is_checked_as_the_plugin_checks_it() {
        let cases = super::super::tools::json_in(include_str!("../../../../../plugin/tests/fixtures/show-cases.ts"), "SHOW_CASES", ']');
        for case in cases.as_array().unwrap() {
            let expected = case.get("mcp").unwrap_or(&case["error"]).as_str().map(str::to_string);
            assert_eq!(view_error(&case["spec"]), expected, "{}", case["name"]);
        }
    }

    #[test]
    fn a_blueprint_is_named_after_its_session() {
        assert_eq!(blueprint_name("blueprint", "mcp-4242"), "blueprint-mcp4242");
        assert_eq!(blueprint_name("plan", ""), "plan");
    }

    #[test]
    fn answers_read_as_the_persons_words() {
        let block = json!({"type": "choice", "prompt": "Which env?"});
        assert_eq!(
            answer_text("deploy", &json!({"type": "choice", "label": "staging", "value": "staging"}), Some(&block)),
            "In the deploy pane I picked \"staging\" for: Which env."
        );
        assert_eq!(
            answer_text("deploy", &json!({"type": "confirm", "value": true}), Some(&json!({"summary": "Ship it\nnow"}))),
            "In the deploy pane I confirmed: Ship it."
        );
        assert_eq!(
            answer_text("diff", &json!({"type": "comment", "value": {"path": "a.rs", "line": 3, "text": "why?"}}), None),
            "In the diff pane I commented on a.rs:3: why?"
        );
        assert_eq!(
            answer_text("plan", &json!({"type": "blueprint", "label": "Read", "value": {"node": "r", "text": "split it"}}), None),
            "In the blueprint, about «Read» (box r): split it"
        );
    }

    #[test]
    fn answers_are_taken_once_and_only_for_this_session() {
        let dir = tempfile::tempdir().unwrap();
        let spec = json!({"title": "t", "session": "mcp-1", "blocks": [{"type": "choice", "prompt": "Pick", "options": [{"label": "a"}, {"label": "b"}]}]});
        files::write(&dir.path().join("mine.json"), &spec).unwrap();
        files::write(&dir.path().join("theirs.json"), &json!({"session": "other", "blocks": [{"type": "confirm", "summary": "x"}]})).unwrap();
        files::write(&dir.path().join("mine.0.answer"), &json!({"block": 0, "type": "choice", "value": "b", "label": "b"})).unwrap();
        files::write(&dir.path().join("theirs.0.answer"), &json!({"block": 0, "type": "confirm", "value": true, "label": "yes"})).unwrap();
        assert_eq!(take_answers(dir.path(), "mcp-1"), ["In the mine pane I picked \"b\" for: Pick."]);
        assert!(take_answers(dir.path(), "mcp-1").is_empty());
        assert!(dir.path().join("theirs.0.answer").exists());
        assert_eq!(files::load(&dir.path().join("mine.json")).unwrap()["blocks"][0]["answered"], "b");
    }
}
