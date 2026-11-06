//! `cockpit setup`: /cockpit:setup in the terminal, for any agent: the frontends, default agent and features written
//! to ~/.config/cockpit/config.json, then herdr, the Herdr app and the other agents wired, each step asked first.

use std::io::Write;
use std::path::Path;
use std::time::Duration;

use cockpit_core::{agent, files};
use serde_json::{Value, json};

use crate::connect;
use crate::herdr_config;
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
    let (tui, gui) = loop {
        let picked = io.ask("\nWhich frontends? 1 herdr (terminal), 2 the Herdr app, 3 both", default);
        match picked.trim() {
            "1" => break (true, false),
            "2" => break (false, true),
            "3" => break (true, true),
            other => println!("{other:?} is not 1, 2 or 3."),
        }
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

    // 4. Features. Connecting a source means mapping its MCP tools, which takes a while, so the quick way out is
    // offered first: GitHub through gh needs nothing mapped and is enough to work from.
    println!("\nWhat cockpit shows you:");
    println!("  1 just my pull requests, to-do and runbooks — nothing to connect, ready in a minute");
    println!("  2 pick what I want, including tickets, CI, meetings, mail, deploys and the release monitor");
    let quick = io.ask("Which?", "1").trim() != "2";
    let picked = if quick {
        println!("\nStarting with pull requests (through gh). Run cockpit setup again any time to add the rest.");
        "1 6".to_string()
    } else {
        println!("\nFeatures: 1 PRs inbox (GitHub, GitLab…), 2 tickets, 3 CI runs (GitHub checks otherwise), 4 meetings, 5 E2E tests in terminal-browser");
        println!("beside the agent, 6 routines space, 7 deploys and previews, 8 release monitor (Datadog, Grafana, Sentry…), 9 mail");
        println!("Tickets, CI, meetings, deploys and the monitor come from MCP servers: cockpit calls them itself, with any agent.");
        io.ask("Which ones, by number (e.g. 1 2 5)?", "1")
    };
    let chosen = numbers(&picked, 9);
    let on = |n: char| chosen.contains(&n);
    let repos = io.ask("\nThe folder holding your git checkouts?", &repos_dir(&home));
    let orgs = said(&["gh", "api", "user/orgs", "--jq", ".[].login"]);
    let org = if on('1') && !orgs.is_empty() {
        let org = io.ask(&format!("Limit your PRs to one GitHub organization ({}, or none)?", orgs.lines().collect::<Vec<_>>().join(", ")), "none");
        if org == "none" { String::new() } else { org }
    } else {
        String::new()
    };
    let mut values: Vec<(&str, Value)> = vec![("reposDir", repos.into()), ("github", json!({"enabled": on('1'), "org": org}))];
    let claude = installed.iter().any(|a| a.id == "claude");
    let mapper = agent_id.as_deref().and_then(agent::get).or_else(|| installed.first().cloned());
    let mut servers = serde_json::Map::new();
    let mut spaces: Vec<&str> = SPACES.to_vec();
    for (n, section) in [('1', "code"), ('2', "tickets"), ('3', "ci"), ('4', "calendar"), ('7', "platform"), ('8', "observability"), ('9', "mail")] {
        // GitHub's PRs come through gh, and need no server.
        if section == "code" && (!on(n) || choices("code").is_empty()) {
            continue;
        }
        if !on(n) {
            let off = if section == "ci" { json!({"enabled": true, "name": "GitHub", "server": ""}) } else { json!({"enabled": false}) };
            if section != "observability" {
                values.push((section, off));
            }
            continue;
        }
        let mut block = match source(io, section, claude, mapper.as_ref()) {
            // Claude's Datadog connector is the monitor's `datadog.server`.
            Source::Claude if section == "observability" => {
                values.push(("datadog", json!({"server": CLAUDE_NAMES[section_at(section)].1})));
                spaces.push("monitor");
                continue;
            }
            Source::Claude => json!({"enabled": true, "name": CLAUDE_NAMES[section_at(section)].0, "server": CLAUDE_NAMES[section_at(section)].1}),
            Source::Server { key, entry, name, mapping, mapped } => {
                servers.insert(key.clone(), entry);
                let mut block = mapping.unwrap_or_else(|| json!({}));
                block["enabled"] = mapped.into();
                block["name"] = name.into();
                block["server"] = key.into();
                block
            }
            Source::Ics(address) => json!({"enabled": true, "name": "Calendar", "server": "", "ics": address}),
            Source::Off if section == "code" => continue,
            Source::Off => json!({"enabled": section == "ci", "name": "GitHub", "server": ""}),
        };
        match section {
            "tickets" if block["enabled"] == true => {
                let keys = io.ask("Ticket key prefixes, such as ENG in ENG-123 (space separated)?", "");
                block["keys"] = json!(keys.split_whitespace().collect::<Vec<_>>());
            }
            "observability" => {
                if let Some(b) = block.as_object_mut() {
                    b.remove("enabled");
                }
                if block["server"].as_str().is_some_and(|s| !s.is_empty()) {
                    spaces.push("monitor");
                }
            }
            _ => {}
        }
        if section == "mail" && block["enabled"] == true {
            spaces.push("mail");
        }
        if section == "calendar" && block["enabled"] != true {
            spaces.retain(|s| *s != "meetings");
        }
        values.push((section, block));
    }
    if !on('4') {
        spaces.retain(|s| *s != "meetings");
    }
    if !servers.is_empty() {
        values.push(("servers", Value::Object(servers)));
    }
    if on('6') {
        spaces.push("routines");
    }
    values.push(("e2eBrowser", if on('5') { "terminal-browser" } else { "chrome" }.into()));
    match files::load(&files::config_dir().join("config.json")).map(|c| c["spaces"].clone()).filter(|s| !s.is_null()) {
        None => values.push(("spaces", json!(spaces))),
        // Spaces already chosen stay as they are; picking mail adds its space.
        Some(Value::Array(mut chosen)) if spaces.contains(&"mail") && !chosen.iter().any(|s| s == "mail" || s["name"] == "mail") => {
            chosen.push("mail".into());
            values.push(("spaces", Value::Array(chosen)));
        }
        Some(_) => {}
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
            herdr_keys(io, &home, &root);
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

/// What each source is called and its server named, for Claude Code's connectors (/cockpit:setup corrects them).
const CLAUDE_NAMES: [(&str, &str); 7] = [
    ("Linear", "claude.ai Linear"),
    ("CircleCI", "circleci"),
    ("Calendar", "claude.ai Google Calendar"),
    ("Platform", ""),
    ("Datadog", "claude.ai Datadog"),
    ("GitHub", ""),
    ("Gmail", "claude.ai Gmail"),
];

fn section_at(section: &str) -> usize {
    ["tickets", "ci", "calendar", "platform", "observability", "code", "mail"].iter().position(|s| *s == section).unwrap_or(0)
}

/// Where a source's data comes from.
enum Source {
    /// Claude Code's connectors, through cockpit's plugin.
    Claude,
    /// An MCP server cockpit calls itself: its `servers` key and entry, its name, the section's mapping, and whether it
    /// is mapped (an unmapped one stays off until `cockpit connector map`).
    Server {
        key: String,
        entry: Value,
        name: String,
        mapping: Option<Value>,
        mapped: bool,
    },
    /// The calendar's secret iCal address.
    Ics(String),
    Off,
}

struct Choice {
    label: String,
    key: String,
    entry: Value,
    name: String,
    mapping: Option<Value>,
}

/// The servers that can feed a section: the ones the agents' configs hold (known ones first, then any whose name tells
/// the section), and the known ones' public addresses.
fn choices(section: &str) -> Vec<Choice> {
    use cockpit_connectors::{Server, presets};
    let mut known = Vec::new();
    let mut other = Vec::new();
    for target in connect::TARGETS.iter().filter(|t| t.installed()) {
        for (name, entry) in target.servers() {
            let Some(server) = Server::from_entry(&entry) else { continue };
            match presets::of(&server).filter(|p| p.sources.contains(&section)) {
                Some(preset) => known.push(Choice {
                    label: format!("{} through \"{name}\" in {}'s config", preset.name, target.name),
                    key: name,
                    entry,
                    name: preset.name.to_string(),
                    mapping: preset.mapping.map(|m| m()),
                }),
                None if presets::of(&server).is_none() => {
                    let text = format!("{name} {}", server.describe()).to_lowercase();
                    if presets::hints(section).iter().any(|h| text.contains(h)) {
                        other.push(Choice {
                            label: format!("\"{name}\" from {}'s config (your agent maps it)", target.name),
                            key: name.clone(),
                            entry,
                            name,
                            mapping: None,
                        });
                    }
                }
                None => {}
            }
        }
    }
    for preset in presets::for_source(section) {
        if !known.iter().any(|c| c.name == preset.name) {
            let mapped = if preset.mapping.is_some() { "" } else { ", your agent maps it" };
            known.push(Choice {
                label: format!("{} ({}{mapped})", preset.name, preset.url),
                key: preset.id.to_string(),
                entry: preset.entry(),
                name: preset.name.to_string(),
                mapping: preset.mapping.map(|m| m()),
            });
        }
    }
    known.extend(other);
    known
}

/// Asks where a source comes from, logs cockpit in to the server picked and maps it when no mapping is known.
fn source(io: &mut Io, section: &str, claude: bool, mapper: Option<&agent::Agent>) -> Source {
    use cockpit_connectors::Server;
    let title =
        ["Tickets", "CI runs", "Meetings", "Deploys and previews", "The release monitor's metrics and errors", "Pull requests", "Mail"][section_at(section)];
    let options = choices(section);
    let mut listed: Vec<String> = Vec::new();
    let claude = claude && section != "code";
    if claude {
        listed.push("Claude Code's connector (/cockpit:setup in Claude Code maps it)".into());
    }
    listed.extend(options.iter().map(|c| c.label.clone()));
    if section == "calendar" {
        listed.push("the calendar's secret iCal address".into());
    }
    listed.push(match section {
        "ci" => "GitHub's checks".into(),
        "code" => "GitHub, through gh".into(),
        _ => "none".into(),
    });
    let numbered: Vec<String> = listed.iter().enumerate().map(|(i, l)| format!("{} {l}", i + 1)).collect();
    let picked = io.ask(&format!("\n{title} from? {}", numbered.join(", ")), "1").parse::<usize>().unwrap_or(1).max(1) - 1;
    let at = picked.checked_sub(usize::from(claude));
    if claude && picked == 0 {
        return Source::Claude;
    }
    let Some(choice) = at.and_then(|i| options.into_iter().nth(i)) else {
        if section == "calendar" && at == Some(listed.len() - usize::from(claude) - 2) {
            let ics = io.secret("The calendar's secret address in iCal format (never shown)?");
            return if ics.is_empty() { Source::Off } else { Source::Ics(ics) };
        }
        return Source::Off;
    };
    // Google's servers only know apps registered with them: the person's own OAuth client (Desktop app) from Google Cloud.
    let mut choice = choice;
    if let Some(preset) = cockpit_connectors::presets::PRESETS.iter().find(|p| p.needs_app() && choice.entry["url"] == p.url)
        && choice.entry["clientId"].is_null()
    {
        println!(
            "{} needs an OAuth client of your own: Google Cloud console → APIs & Services → enable the {} MCP API, then Credentials → Create OAuth client ID → Desktop app.",
            preset.name, preset.name
        );
        let id = io.ask("Its client ID?", "");
        if id.is_empty() {
            return Source::Off;
        }
        let secret = io.secret("Its client secret (never shown)?");
        choice.entry = json!({"url": preset.url, "clientId": id, "clientSecret": secret, "scopes": preset.app_scopes});
    }
    let Some(server) = Server::from_entry(&choice.entry) else { return Source::Off };
    if let Some(url) = server.login_url().map(str::to_string)
        && !server.is_ready()
    {
        if !io.yes(&format!("Log cockpit in to {} now (it opens your browser)?", choice.name), true) {
            println!("Later: cockpit connector login {url}");
            let mapped = choice.mapping.is_some();
            return Source::Server { key: choice.key, entry: choice.entry, name: choice.name, mapping: choice.mapping, mapped };
        }
        let done = cockpit_connectors::block_on(cockpit_connectors::login_server(&server, |link| {
            println!("If the browser does not open: {link}");
            let _ = std::process::Command::new(if cfg!(target_os = "macos") { "open" } else { "xdg-open" }).arg(link).status();
        }));
        match done {
            Ok(()) => println!("Logged in to {}.", choice.name),
            Err(e) => {
                println!("Not logged in ({e}). cockpit connector login {url} tries again; until then the hub leaves it.");
                let mapped = choice.mapping.is_some();
                return Source::Server { key: choice.key, entry: choice.entry, name: choice.name, mapping: choice.mapping, mapped };
            }
        }
    }
    let (mapping, mapped) = match (choice.mapping, section) {
        (Some(mapping), _) => (Some(mapping), true),
        // Datadog's own tools need no mapping.
        (None, "observability") if choice.key == "datadog" || choice.name == "Datadog" => (Some(json!({})), true),
        (None, _) => match mapper {
            Some(agent) => {
                println!("{} maps {}'s tools for cockpit (a minute or two)…", agent.name, choice.name);
                match crate::mapper::map(section, &server, &choice.name, agent) {
                    Ok((mapping, sample)) => {
                        println!("Mapped: {sample}");
                        (Some(mapping), true)
                    }
                    Err(e) => {
                        println!("Not mapped ({e}): cockpit connector map {section} {} tries again.", choice.key);
                        (None, false)
                    }
                }
            }
            None => {
                println!("No agent to map {}: cockpit connector map {section} {} once one is installed.", choice.name, choice.key);
                (None, false)
            }
        },
    };
    Source::Server { key: choice.key, entry: choice.entry, name: choice.name, mapping, mapped }
}

/// cockpit's keys (prefix+i/t/a/d/u/m) and sidebar in herdr's config.toml, merged once the person agrees, then reloaded.
fn herdr_keys(io: &mut Io, home: &Path, root: &str) {
    let path = home.join(".config/herdr/config.toml");
    let theirs = std::fs::read_to_string(&path).unwrap_or_default();
    let ours = std::fs::read_to_string(format!("{root}/config.example.toml")).unwrap_or_default();
    let merged = match herdr_config::merge(&theirs, &ours) {
        Ok(m) if m.text == theirs => return println!("~/.config/herdr/config.toml already has cockpit's keys and sidebar."),
        Ok(m) => m,
        Err(e) => return println!("{e}: merge {root}/config.example.toml into it yourself."),
    };
    let delivery = if merged.delivery { "; herdr's toasts stay in herdr so notifications arrive once" } else { "" };
    let question = format!("\nAdd cockpit's keys and sidebar to ~/.config/herdr/config.toml ({} settings and keys, your own stay{delivery})?", merged.added);
    if !io.yes(&question, true) {
        return println!("Left as it is: merge {root}/config.example.toml into it when you want cockpit's keys.");
    }
    if !theirs.is_empty() {
        let backup = path.with_file_name(format!("config.toml.bak-{}", chrono::Local::now().format("%Y%m%d-%H%M%S")));
        if let Err(e) = std::fs::write(&backup, &theirs) {
            return println!("config.toml not changed: no backup ({e})");
        }
        println!("The old one is {}", backup.file_name().unwrap_or_default().to_string_lossy());
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match std::fs::write(&path, &merged.text) {
        Ok(()) => {
            let reloaded = run(&["herdr", "server", "reload-config"]).is_some_and(|o| o.ok());
            println!("Wrote ~/.config/herdr/config.toml{}", if reloaded { " and reloaded herdr" } else { "; herdr reads it at its next start" });
        }
        Err(e) => println!("config.toml not written: {e}"),
    }
}

/// Writes a file only its owner can read, through a temporary file beside it: the config holds tokens.
/// The numbers an answer like "1 2 5", "1,2,5" or "125" names, within 1..=most; anything else is ignored.
fn numbers(answer: &str, most: u32) -> Vec<char> {
    let mut picked: Vec<char> = Vec::new();
    for c in answer.chars().filter(|c| c.is_ascii_digit()) {
        let n = c.to_digit(10).unwrap_or(0);
        if (1..=most).contains(&n) && !picked.contains(&c) {
            picked.push(c);
        }
    }
    picked
}

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
