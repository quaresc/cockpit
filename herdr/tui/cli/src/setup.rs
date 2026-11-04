//! `cockpit setup`: /cockpit:setup in the terminal, for any agent: the frontends, default agent and features written
//! to ~/.config/cockpit/config.json, then herdr, the Herdr app and the other agents wired, each step asked first.

use std::io::Write;
use std::path::Path;
use std::time::Duration;

use cockpit_core::{agent, files};
use serde_json::{Value, json};

use crate::connect;
use crate::jsonedit;
use crate::proc;

const SPACES: [&str; 6] = ["inbox", "terminal", "actions", "to-do", "runbook", "meetings"];

struct Io {
    lines: Box<dyn Iterator<Item = String>>,
}

impl Io {
    /// One answer: the default when the line is empty or the input ended.
    fn ask(&mut self, question: &str, default: &str) -> String {
        if default.is_empty() {
            print!("{question} ")
        } else {
            print!("{question} [{default}] ")
        }
        let _ = std::io::stdout().flush();
        let line = self.lines.next().map(|l| l.trim().to_string()).unwrap_or_default();
        println!();
        if line.is_empty() { default.to_string() } else { line }
    }

    fn yes(&mut self, question: &str, default: bool) -> bool {
        let said = self.ask(&format!("{question} (y/n)"), if default { "y" } else { "n" });
        said.to_lowercase().starts_with('y')
    }

    /// An answer not shown as it is typed, when the input is a terminal.
    fn secret(&mut self, question: &str) -> String {
        let echo = Echo::off();
        let said = self.ask(question, "");
        drop(echo);
        said
    }
}

/// The terminal's echo, off while it lives.
struct Echo(Option<libc::termios>);

impl Echo {
    fn off() -> Echo {
        // SAFETY: termios calls on stdin with a zeroed struct the call fills.
        unsafe {
            let mut term: libc::termios = std::mem::zeroed();
            if libc::isatty(0) == 0 || libc::tcgetattr(0, &mut term) != 0 {
                return Echo(None);
            }
            let before = term;
            term.c_lflag &= !libc::ECHO;
            libc::tcsetattr(0, libc::TCSANOW, &term);
            Echo(Some(before))
        }
    }
}

impl Drop for Echo {
    fn drop(&mut self) {
        if let Some(before) = self.0 {
            // SAFETY: puts back the settings tcgetattr read.
            unsafe { libc::tcsetattr(0, libc::TCSANOW, &before) };
        }
    }
}

fn run(argv: &[&str]) -> Option<proc::Out> {
    proc::run(argv, &[], Duration::from_secs(300))
}

fn said(argv: &[&str]) -> String {
    run(argv).filter(proc::Out::ok).map(|o| o.stdout.trim().to_string()).unwrap_or_default()
}

/// The folder holding the git checkouts: the first of the usual places with several.
fn repos_dir(home: &Path) -> String {
    let mut places = vec![home.join("code"), home.join("src"), home.join("dev"), home.join("projects")];
    if let Ok(work) = std::fs::read_dir(home.join("Work")) {
        places.extend(work.flatten().map(|e| e.path()).filter(|p| p.is_dir()));
    }
    let checkouts = |p: &Path| std::fs::read_dir(p).into_iter().flatten().flatten().filter(|e| e.path().join(".git").exists()).count();
    let found = places.into_iter().find(|p| checkouts(p) >= 2);
    found.map(|p| p.strip_prefix(home).map_or(p.display().to_string(), |r| format!("~/{}", r.display()))).unwrap_or_else(|| "~/code".into())
}

/// The config's text with these top-level members set, everything else kept as written.
fn merged(text: &str, values: &[(&str, Value)], agent: Option<&str>, frontend: Option<&str>) -> Result<String, String> {
    let mut text = if text.trim().is_empty() { "{}\n".to_string() } else { text.to_string() };
    for (key, value) in values {
        let current = jsonedit::get(&text, &[], key)?;
        // An object keeps the members the person set beyond the ones asked here.
        let value = match (current, value) {
            (Some(Value::Object(mut had)), Value::Object(new)) => {
                had.extend(new.clone());
                Value::Object(had)
            }
            _ => value.clone(),
        };
        text = jsonedit::set(&text, &[], key, &value)?;
    }
    if let Some(agent) = agent {
        text = jsonedit::set(&text, &["agents"], "default", &agent.into())?;
    }
    text = match frontend {
        Some(f) => jsonedit::set(&text, &[], "frontend", &f.into())?,
        None => jsonedit::remove(&text, &[], "frontend")?.unwrap_or(text),
    };
    Ok(text)
}

pub fn main(_: &[String]) {
    let mut io = Io { lines: Box::new(std::io::stdin().lines().map_while(Result::ok)) };
    setup(&mut io);
}

fn setup(io: &mut Io) {
    let home = files::home();
    println!("cockpit setup: the frontends, your default agent and the features, then ~/.config/cockpit/config.json.\n");

    // 1. Prerequisites
    let herdr = said(&["herdr", "--version"]);
    let app = proc::which("herdr-app");
    let gh = run(&["gh", "auth", "status"]).is_some_and(|o| o.ok());
    let jq = proc::which("jq").is_some();
    println!("herdr: {}", if herdr.is_empty() { "not installed (https://herdr.dev)" } else { &herdr });
    println!("Herdr app: {}", if app.is_some() { "installed" } else { "not installed (https://herdr.dev)" });
    println!("gh: {}", if gh { "signed in" } else { "missing or signed out → brew install gh && gh auth login" });
    println!("jq: {}", if jq { "installed" } else { "missing → brew install jq" });
    if herdr.is_empty() && app.is_none() {
        println!("\ncockpit runs in herdr or in the Herdr app: install one, then run cockpit setup again.");
        std::process::exit(1);
    }

    // 2. Frontends
    let default = match (herdr.is_empty(), app.is_none()) {
        (false, false) => "3",
        (false, true) => "1",
        _ => "2",
    };
    let picked = io.ask("\nWhich frontends? 1 herdr (terminal), 2 the Herdr app, 3 both", default);
    let (tui, gui) = match picked.as_str() {
        "1" => (true, false),
        "2" => (false, true),
        _ => (true, true),
    };
    let frontend = match (tui, gui) {
        (true, false) => Some("tui"),
        (false, true) => Some("app"),
        _ => None,
    };

    // 3. The default agent
    let installed: Vec<agent::Agent> = agent::all().into_iter().filter(agent::Agent::installed).collect();
    let current = files::config()["agents"]["default"].as_str().map(str::to_string);
    let agent_id = if installed.is_empty() {
        println!("\nNo coding agent cockpit knows is installed; `agents.default` stays unset.");
        None
    } else {
        let listed: Vec<String> = installed.iter().enumerate().map(|(i, a)| format!("{} {}", i + 1, a.name)).collect();
        let at = current.as_ref().and_then(|c| installed.iter().position(|a| &a.id == c)).unwrap_or(0);
        let picked = io.ask(&format!("\nYour default agent, the one cockpit starts? {}", listed.join(", ")), &(at + 1).to_string());
        let n = picked.parse::<usize>().ok().filter(|n| (1..=installed.len()).contains(n)).unwrap_or(at + 1);
        Some(installed[n - 1].id.clone())
    };

    // 4. Features
    println!("\nFeatures: 1 GitHub PRs inbox, 2 Linear tickets (an API key), 3 CircleCI runs (a token; GitHub checks otherwise),");
    println!("4 meetings from a calendar's secret iCal address, 5 E2E tests in terminal-browser beside the agent, 6 routines space");
    let picked = io.ask("Which ones, by number (e.g. 1 2 5)?", "1");
    let on = |n: char| picked.contains(n);
    let repos = io.ask("\nThe folder holding your git checkouts?", &repos_dir(&home));
    let orgs = said(&["gh", "api", "user/orgs", "--jq", ".[].login"]);
    let org = if on('1') && !orgs.is_empty() {
        let org = io.ask(&format!("Limit your PRs to one GitHub organization ({}, or none)?", orgs.lines().collect::<Vec<_>>().join(", ")), "none");
        if org == "none" { String::new() } else { org }
    } else {
        String::new()
    };
    let mut values: Vec<(&str, Value)> = vec![("reposDir", repos.into()), ("github", json!({"enabled": on('1'), "org": org}))];
    if on('2') {
        let keys = io.ask("Ticket key prefixes, such as ENG in ENG-123 (space separated)?", "");
        let mut tickets = json!({"enabled": true, "name": "Linear", "server": "", "keys": keys.split_whitespace().collect::<Vec<_>>(), "urlTemplate": ""});
        let token = io.secret("A Linear personal API key (Settings → Security & access; never shown, optional)?");
        if !token.is_empty() {
            tickets["token"] = token.into();
        }
        values.push(("tickets", tickets));
    } else {
        values.push(("tickets", json!({"enabled": false})));
    }
    let mut ci = json!({"enabled": true, "name": "GitHub", "server": "", "imageJob": ""});
    if on('3') {
        let token = io.secret("A CircleCI personal API token (never shown)?");
        ci = json!({"enabled": true, "name": "CircleCI", "server": "circleci", "imageJob": ""});
        if !token.is_empty() {
            ci["token"] = token.into();
        }
    }
    values.push(("ci", ci));
    let mut spaces: Vec<&str> = SPACES.to_vec();
    if on('4') {
        let ics = io.secret("The calendar's secret address in iCal format (never shown)?");
        values.push(("calendar", json!({"enabled": !ics.is_empty(), "name": "Calendar", "server": "", "ics": ics})));
    } else {
        values.push(("calendar", json!({"enabled": false})));
        spaces.retain(|s| *s != "meetings");
    }
    if on('6') {
        spaces.push("routines");
    }
    values.push(("e2eBrowser", if on('5') { "terminal-browser" } else { "chrome" }.into()));
    if files::load(&files::config_dir().join("config.json")).is_none_or(|c| c["spaces"].is_null()) {
        values.push(("spaces", json!(spaces)));
    }

    // 5. The config
    let path = files::config_dir().join("config.json");
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let next = merged(&text, &values, agent_id.as_deref(), frontend)
        .unwrap_or_else(|e| proc::fail(&format!("~/.config/cockpit/config.json does not read as JSON ({e}): fix it, then run cockpit setup again")));
    if next != text {
        let _ = std::fs::create_dir_all(files::config_dir());
        if !text.is_empty() {
            let _ = write_private(&path.with_file_name("config.json.bak"), &text);
        }
        write_private(&path, &next).unwrap_or_else(|e| proc::fail(&format!("config.json not written: {e}")));
        println!("Wrote ~/.config/cockpit/config.json{}", if text.is_empty() { "" } else { " (the old one is config.json.bak)" });
    }
    if on('5') && proc::which("terminal-browser").is_none() {
        println!("terminal-browser is not installed → brew install --cask terminal-browser, then terminal-browser config set render.transport auto");
    }

    let exe = std::env::current_exe().unwrap_or_default();
    let exe = exe.to_string_lossy().into_owned();
    // 6. herdr: its plugin, the binary in it and the pinned spaces
    if tui && !herdr.is_empty() {
        let plugins = files::load(&home.join(".config/herdr/plugins.json")).unwrap_or(Value::Null);
        let registered =
            plugins.as_array().into_iter().flatten().find(|p| p["plugin_id"] == "cockpit").and_then(|p| p["plugin_root"].as_str()).map(str::to_string);
        let root = match registered {
            Some(root) => Some(root),
            None if io.yes("\nInstall cockpit's herdr plugin (herdr plugin install quaresc/cockpit/herdr)?", true) => {
                run(&["herdr", "plugin", "install", "quaresc/cockpit/herdr", "--yes"]);
                let plugins = files::load(&home.join(".config/herdr/plugins.json")).unwrap_or(Value::Null);
                plugins.as_array().into_iter().flatten().find(|p| p["plugin_id"] == "cockpit").and_then(|p| p["plugin_root"].as_str()).map(str::to_string)
            }
            None => None,
        };
        if let Some(root) = root {
            for script in ["fetch-apps.sh", "ensure-spaces.sh", "pin-spaces.sh"] {
                let ok = run(&["sh", &format!("{root}/{script}")]).is_some_and(|o| o.ok());
                println!("{script}: {}", if ok { "done" } else { "failed; cockpit doctor says why" });
            }
            println!(
                "Merge {root}/config.example.toml into ~/.config/herdr/config.toml for cockpit's keys (prefix+i/t/a/d/u/m), then herdr server reload-config."
            );
        }
    }

    // 7. The Herdr app: the cockpit layout, mod and keys
    if gui && app.is_some() && io.yes("\nInstall the cockpit layout, mod and keys in the Herdr app?", true) {
        let out = run(&[&exe, "app", "install"]);
        println!("{}", out.map(|o| format!("{}{}", o.stdout, o.stderr)).unwrap_or_default().trim());
    }

    // 8. cockpit's tools in the other agents
    let bin = connect::binary();
    let open: Vec<&connect::Target> = connect::TARGETS.iter().filter(|t| t.installed() && !t.is_connected(&bin)).collect();
    if !open.is_empty() {
        let names: Vec<&str> = open.iter().map(|t| t.name).collect();
        if io.yes(&format!("\nGive {} cockpit's tools (show, search, changes) through their MCP config?", names.join(", ")), true) {
            for t in open {
                match t.connect(&bin) {
                    Ok(line) => println!("{line}"),
                    Err(e) => println!("{}: {e}", t.name),
                }
            }
        }
    }
    if installed.iter().any(|a| a.id == "claude") {
        println!("\nClaude Code: /plugin install cockpit --marketplace quaresc/cockpit, then /cockpit:setup maps your connectors (tickets, CI, calendar).");
    }
    println!("\nDone. cockpit doctor checks the whole setup at any time.");
}

/// Writes a file only its owner can read, through a temporary file beside it: the config holds tokens.
fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let tmp = path.with_file_name(format!(".{}.tmp", path.file_name().unwrap_or_default().to_string_lossy()));
    let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    file.write_all(text.as_bytes())?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_config_is_its_owners_alone() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, "{}").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        write_private(&path, "{\"github\": {\"token\": \"x\"}}").unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"github\": {\"token\": \"x\"}}");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1, "no temporary file left");
    }

    #[test]
    fn the_config_keeps_what_the_person_set() {
        let text = "{\n  \"github\": {\"enabled\": false, \"extra\": 1},\n  \"density\": \"compact\",\n  \"frontend\": \"tui\"\n}\n";
        let done = merged(text, &[("github", json!({"enabled": true, "org": ""})), ("e2eBrowser", json!("chrome"))], Some("codex"), None).unwrap();
        let read = jsonedit::plain(&done).unwrap();
        assert_eq!(read["github"], json!({"enabled": true, "extra": 1, "org": ""}));
        assert_eq!(read["density"], "compact");
        assert_eq!(read["agents"]["default"], "codex");
        assert!(read.get("frontend").is_none());
        assert!(done.starts_with("{\n  \"github\""));
        assert_eq!(jsonedit::plain(&merged("", &[], None, Some("app")).unwrap()).unwrap(), json!({"frontend": "app"}));
    }
}
