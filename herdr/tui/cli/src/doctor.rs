//! `cockpit doctor`: checks every part of cockpit and prints one line per check, ok, warn or fail, with what it found
//! and the fix. `--json` gives the same checks as data (each with an id, its category and its fix), the steps of
//! getting started they make, and what each agent can do, and keeps it in the cache for the apps.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use cockpit_core::{agent, files, transcripts};
use serde_json::{Value, json};

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering as Atomic};

use crate::proc::{self, Out};

const SESSION: &str = "no Claude session with cockpit is running";
const NO_FEED: &str = "neither a cockpit app (its hub) nor a Claude session with cockpit is running";
const SPACES: [&str; 6] = ["inbox", "terminal", "actions", "to-do", "runbook", "meetings"];
/// Puts the `cockpit` a shell runs back on the current install.
const LINK_BIN: &str = "ln -sfn ~/.config/cockpit/herdr/bin/cockpit ~/.local/bin/cockpit";
/// The shape of `--json`; a change of it bumps this.
const JSON_VERSION: u64 = 1;
/// How old a kept report `--cached` still answers with.
const CACHED_FOR: Duration = Duration::from_secs(10 * 60);

/// What the run found: the ok lines are held back unless every check is asked for, so what needs doing stands out.
static ALL: AtomicBool = AtomicBool::new(false);
/// `--json`: the checks are kept, not printed.
static QUIET: AtomicBool = AtomicBool::new(false);
static OK: AtomicUsize = AtomicUsize::new(0);
static WARN: AtomicUsize = AtomicUsize::new(0);
static FAIL: AtomicUsize = AtomicUsize::new(0);
static CHECKS: Mutex<Vec<Check>> = Mutex::new(Vec::new());

/// One check as it ran: `id` is `<category>.<what>`, stable from run to run.
#[derive(Clone, Debug, PartialEq)]
struct Check {
    id: String,
    status: &'static str,
    message: String,
    /// What the line says after its arrow, and the command or hub action that does it, when one does.
    fix: Option<Value>,
}

impl Check {
    fn json(&self) -> Value {
        json!({"id": self.id, "category": self.id.split('.').next().unwrap_or_default(), "status": self.status, "message": self.message, "fix": self.fix})
    }
}

fn checks() -> std::sync::MutexGuard<'static, Vec<Check>> {
    CHECKS.lock().unwrap_or_else(|e| e.into_inner())
}

/// The check just reported, to say which command or hub action fixes it.
struct Reported;

impl Reported {
    /// The command that fixes it, typed into a tab of the actions space by the apps' Fix button.
    fn run(self, command: impl Into<String>) -> Reported {
        if let Some(fix) = checks().last_mut().and_then(|c| c.fix.as_mut()) {
            fix["command"] = Value::from(command.into());
        }
        self
    }

    /// The hub action that fixes it: `{app, action, args}`.
    fn act(self, app: &str, action: &str, args: Value) -> Reported {
        if let Some(fix) = checks().last_mut().and_then(|c| c.fix.as_mut()) {
            fix["act"] = json!({"app": app, "action": action, "args": args});
        }
        self
    }
}

fn report(status: &'static str, id: &str, line: &str) -> Reported {
    let (message, label) = match line.split_once(" → ") {
        Some((message, label)) => (message, Some(label)),
        None => (line, None),
    };
    checks().push(Check { id: id.to_string(), status, message: message.to_string(), fix: label.map(|l| json!({"label": l})) });
    let shown = match status {
        "pass" => ALL.load(Atomic::Relaxed),
        _ => true,
    };
    if shown && !QUIET.load(Atomic::Relaxed) {
        let word = match status {
            "pass" => "ok  ",
            "warn" => "warn",
            _ => "fail",
        };
        println!("{word}  {line}");
    }
    Reported
}

fn ok(id: &str, line: &str) -> Reported {
    OK.fetch_add(1, Atomic::Relaxed);
    report("pass", id, line)
}

fn warn(id: &str, line: &str) -> Reported {
    WARN.fetch_add(1, Atomic::Relaxed);
    report("warn", id, line)
}

fn fail(id: &str, line: &str) -> Reported {
    FAIL.fetch_add(1, Atomic::Relaxed);
    report("fail", id, line)
}

/// The last line: what the run found, and how to see the checks that passed.
fn verdict() {
    let (ok, warn, fail) = (OK.load(Atomic::Relaxed), WARN.load(Atomic::Relaxed), FAIL.load(Atomic::Relaxed));
    let count = |n: usize, one: &str| format!("{n} {}{}", one, if n == 1 { "" } else { "s" });
    if warn + fail == 0 {
        println!("\nEvery check passed ({}).", count(ok, "check"));
    } else {
        let found = format!("{}, {}", count(fail, "failure"), count(warn, "warning"));
        println!("\n{found}{}", if ALL.load(Atomic::Relaxed) { String::new() } else { format!("; {} passed, cockpit doctor --all lists them", ok) });
    }
}

/// What the checks read and run with.
struct Doctor {
    /// The herdr plugin folder this binary belongs to, its links resolved.
    root: PathBuf,
    /// ~/.config/cockpit/herdr, the link kept on the current install.
    link: PathBuf,
    /// The plugin folder herdr's registry names.
    registered: String,
    cache: PathBuf,
    /// The config as the mod resolved it, else the raw file; None when neither reads as JSON.
    settings: Option<Value>,
    settings_path: PathBuf,
    /// ~/.config/cockpit/config.json as written.
    config: Option<Value>,
    herdr: String,
    path: String,
    now: i64,
    /// herdr answers its --version: the terminal half is there.
    has_herdr: bool,
    /// The Herdr desktop app's command, when it is installed.
    app: Option<PathBuf>,
}

pub fn main(args: &[String]) {
    let has = |flag: &str| args.iter().any(|a| a == flag);
    ALL.store(has("--all") || has("-v"), Atomic::Relaxed);
    let as_json = has("--json");
    QUIET.store(as_json, Atomic::Relaxed);
    let kept = files::cache().join("doctor.json");
    let config_changed = std::fs::metadata(files::config_dir().join("config.json"))
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as u64);
    if as_json
        && has("--cached")
        && let Some(report) = load(&kept).filter(|r| is_recent(r, now_ms(), config_changed))
    {
        println!("{report}");
        return;
    }
    let doctor = Doctor::new();
    doctor.tools();
    doctor.installs();
    doctor.herdr_app();
    doctor.agents();
    // The Claude plugin and its sessions only matter when Claude Code is the default agent or installed.
    let claude = cockpit_core::agent::get("claude");
    let is_default = cockpit_core::agent::default().id == "claude";
    if is_default || claude.is_some_and(|c| c.installed()) {
        let installed = doctor.claude_plugin(is_default);
        doctor.sessions(&installed);
        doctor.app_sessions(&installed);
    }
    doctor.caches();
    doctor.spaces();
    doctor.notifications();
    doctor.terminal_browser();
    doctor.connectors();
    if as_json {
        let report = doctor.report(&checks());
        files::locked(&kept, || files::write(&kept, &report)).ok();
        println!("{report}");
        return;
    }
    if ALL.load(Atomic::Relaxed) {
        print_agents();
    }
    verdict();
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or_default()
}

/// A kept report of this shape and version, younger than `CACHED_FOR` and than the config, with no fix run since (the hub
/// marks it `stale`); one dated in the future is not trusted.
fn is_recent(report: &Value, now: u64, config_changed: u64) -> bool {
    report["version"] == JSON_VERSION
        && report["cockpit"] == env!("CARGO_PKG_VERSION")
        && report["stale"] != true
        && report["at"].as_u64().is_some_and(|at| at <= now && at >= config_changed && now - at < CACHED_FOR.as_millis() as u64)
}

/// The steps of getting started, each the check that says whether it is done: set up, an agent, its tools, GitHub,
/// tickets, the Herdr app's layout and notifications. A step whose check did not run is left out.
fn getting_started(checks: &[Check]) -> Vec<Value> {
    let by_id = |id: &str| checks.iter().find(|c| c.id == id);
    let agent = agent::default();
    let mut steps: Vec<Value> = Vec::new();
    let step = |steps: &mut Vec<Value>, id: &str, label: &str, check: Option<&Check>| {
        if let Some(c) = check {
            steps.push(json!({"id": id, "label": label, "done": c.status == "pass", "status": c.status, "check": c.id, "fix": c.fix}));
        }
    };
    step(&mut steps, "setup", "Set up cockpit: frontends, agent and sources", by_id("tools.config"));
    step(&mut steps, "agent", "Pick the agent cockpit starts", by_id("agents.default"));
    step(&mut steps, "tools", &format!("Give {} cockpit's tools", agent.name), by_id(&tools_check(&agent)));
    // GitHub is read through gh, or through a code server of cockpit's own.
    let code = by_id("sources.server.code").filter(|c| c.status == "pass");
    step(&mut steps, "github", "Sign in to GitHub for your pull requests", code.or_else(|| by_id("tools.gh")));
    step(&mut steps, "tickets", "Connect your tickets", by_id("sources.tickets"));
    step(&mut steps, "app", "Put cockpit's layouts in the Herdr app", by_id("app.layout"));
    step(&mut steps, "notifications", "Let cockpit notify you", by_id("notifications.allowed"));
    steps
}

/// Whether tickets come in: read by the hub lately (its freshness record), or brought by a Claude session's plugin
/// within 15 minutes; `hub_reads` when a server of cockpit's own or a token lets the hub read them.
fn tickets(inbox: &Value, hub_reads: bool, now: u64) {
    use cockpit_core::fresh::{self, Freshness, State};
    let record = Freshness::from_json(&inbox["hub"]["fresh"]["tickets"]);
    let brought = inbox["modFetched"]["tickets"].as_u64();
    let another = "cockpit setup gives the hub a ticket server of its own";
    match (record.state(now), &record.error) {
        (State::Fresh, _) => ok("sources.tickets", &format!("tickets read by the hub {}", fresh::ago(record.at.unwrap_or(now), now))),
        _ if brought.is_some_and(|at| now.saturating_sub(at) < 15 * 60_000) => {
            ok("sources.tickets", &format!("tickets brought by a Claude session {}", fresh::ago(brought.unwrap_or(now), now)))
        }
        (State::Failing, Some((_, message, _))) => {
            fail("sources.tickets", &format!("tickets: {message} → sign in to them again")).act("sources", "sign_in", json!(["tickets"]))
        }
        (State::Stale, _) => warn("sources.tickets", &format!("tickets: last read {} → read them again", fresh::ago(record.at.unwrap_or(now), now))).act(
            "sources",
            "refresh",
            json!([]),
        ),
        _ if hub_reads => warn("sources.tickets", "tickets: not read yet → read them now").act("sources", "refresh", json!([])),
        _ => match brought {
            Some(at) => warn("sources.tickets", &format!("tickets: last brought {} → {SESSION} ({another})", fresh::ago(at, now))),
            None => warn("sources.tickets", &format!("tickets: never fetched → cockpit setup ({another})")).run("cockpit setup"),
        },
    };
}

/// The check that says whether an agent has cockpit's tools: its plugin's, else its MCP config's.
fn tools_check(agent: &agent::Agent) -> String {
    if agent.has_plugin() { format!("{}.plugin", agent.id) } else { format!("agents.tools.{}", agent.id) }
}

/// What each agent cockpit knows can do, the installed ones first and the default one first of them.
fn agents_json(checks: &[Check]) -> Vec<Value> {
    let default = agent::default().id;
    let mut all = agent::all();
    all.sort_by_key(|a| (a.id != default, !a.installed()));
    all.iter()
        .map(|a| {
            let installed = a.installed();
            // cockpit's tools reach an agent through cockpit's plugin, else through its MCP config.
            let tools = if a.has_plugin() {
                Some("plugin")
            } else if crate::connect::TARGETS.iter().any(|t| t.id == a.id) {
                Some("mcp")
            } else {
                None
            };
            let connected = checks.iter().find(|c| c.id == tools_check(a)).map(|c| c.status == "pass");
            let reads = a.transcripts != transcripts::Transcripts::None;
            json!({
                "id": a.id, "name": a.name, "icon": agent::icon_in(&all, &a.id), "installed": installed, "default": a.id == default,
                "path": if installed { Value::from(a.path()) } else { Value::Null },
                "can": {
                    "start": !a.start.is_empty(), "resume": !a.resume.is_empty(), "fork": !a.fork.is_empty(), "headless": a.can_run(),
                    "edits": !a.edits.is_empty(), "model": !a.model.is_empty(),
                    // Driven sessions go over the streaming protocol the table's `stream` column measured.
                    "driven": a.stream.is_some(),
                },
                "sessions": reads,
                "usage": match (reads, cockpit_core::usage::counts_tokens(a.transcripts)) {
                    (true, true) => Value::from("its session files: tokens and context"),
                    (true, false) => Value::from("its session files: context"),
                    _ => Value::Null,
                },
                "limits": limits_of(a),
                "tools": tools, "connected": connected,
            })
        })
        .collect()
}

/// Where cockpit reads an agent's plan limits, in the agents table's words; null when it reports none.
fn limits_of(a: &agent::Agent) -> Value {
    match agent::reports(&a.id) {
        Some(r) => Value::from(r.sources.iter().map(|(_, words)| *words).collect::<Vec<_>>().join(", or ")),
        None => Value::Null,
    }
}

/// The agents and what each can do, as `--all` lists them after the checks.
fn print_agents() {
    println!("\nAgents");
    for a in agents_json(&checks()) {
        let can = &a["can"];
        // `driven` reads "in chat": Cockpit Chat runs the agent itself, without a terminal.
        let words: Vec<&str> = [("start", "start"), ("resume", "resume"), ("fork", "fork"), ("headless", "headless"), ("driven", "in chat")]
            .into_iter()
            .filter(|(key, _)| can[*key] == true)
            .map(|(_, word)| word)
            .collect();
        let state = if a["installed"] == true { if a["default"] == true { "default" } else { "installed" } } else { "not installed" };
        if a["installed"] == true {
            println!("      {} ({state}): {}", raw(&a["name"]), words.join(", "));
        }
    }
}

impl Doctor {
    fn new() -> Self {
        let home = files::home();
        let path = format!("/opt/homebrew/bin:/usr/local/bin:{0}/.local/bin:{0}/.claude/local:{1}", home.display(), std::env::var("PATH").unwrap_or_default());
        let herdr_dir = files::herdr_dir();
        let root = herdr_dir.canonicalize().unwrap_or(herdr_dir);
        let link = files::config_dir().join("herdr");
        let plugins = load(&home.join(".config/herdr/plugins.json")).unwrap_or(Value::Null);
        let registered = values(&plugins).filter(|p| p["plugin_id"] == "cockpit").map(|p| raw(&p["plugin_root"])).collect::<Vec<_>>().join("\n");
        // herdr installs each update in a new folder: only the folder its registry names moves the link.
        if registered == root.to_string_lossy() && readlink(&link) != registered {
            if let Some(parent) = link.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if std::fs::symlink_metadata(&link).is_ok_and(|m| m.file_type().is_symlink()) {
                let _ = std::fs::remove_file(&link);
            }
            let _ = std::os::unix::fs::symlink(&root, &link);
        }
        let config_path = files::config_dir().join("config.json");
        let resolved = files::cache().join("config.json");
        let settings_path = if resolved.is_file() { resolved } else { config_path.clone() };
        let settings = load(&settings_path);
        let herdr = std::env::var("HERDR_BIN_PATH").ok().filter(|h| !h.is_empty()).unwrap_or_else(|| "herdr".into());
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or_default();
        let mut doctor = Doctor {
            root,
            link,
            registered,
            cache: files::cache(),
            settings,
            settings_path,
            config: load(&config_path),
            herdr,
            path,
            now,
            has_herdr: false,
            app: None,
        };
        doctor.has_herdr = !doctor.output(&[&doctor.herdr, "--version"]).is_empty();
        doctor.app = doctor.which("herdr-app");
        doctor
    }

    fn set(&self) -> &Value {
        self.settings.as_ref().unwrap_or(&Value::Null)
    }

    /// `sh <the link>/<name>`, the link quoted as a shell reads it.
    fn script(&self, name: &str) -> String {
        format!("sh {}/{name}", quote(&self.link.to_string_lossy()))
    }

    /// `--json`: the checks, the steps of getting started they make and the agents.
    fn report(&self, checks: &[Check]) -> Value {
        let count = |status: &str| checks.iter().filter(|c| c.status == status).count();
        json!({
            "version": JSON_VERSION, "at": now_ms(), "cockpit": env!("CARGO_PKG_VERSION"),
            "summary": {"pass": count("pass"), "warn": count("warn"), "fail": count("fail")},
            "checks": checks.iter().map(Check::json).collect::<Vec<_>>(),
            "getting_started": getting_started(checks),
            "agents": agents_json(checks),
        })
    }

    fn run(&self, argv: &[&str], timeout: Duration) -> Option<Out> {
        proc::run(argv, &[("PATH", &self.path)], timeout).ok()
    }

    /// A command's output as `$(…)` reads it: stdout without its trailing newlines, empty when it cannot run.
    fn output(&self, argv: &[&str]) -> String {
        self.run(argv, Duration::from_secs(60)).map(|o| o.stdout.trim_end_matches('\n').to_string()).unwrap_or_default()
    }

    fn succeeds(&self, argv: &[&str]) -> bool {
        self.run(argv, Duration::from_secs(60)).is_some_and(|o| o.ok())
    }

    fn herdr_json(&self, args: &[&str]) -> Value {
        let argv: Vec<&str> = std::iter::once(self.herdr.as_str()).chain(args.iter().copied()).collect();
        serde_json::from_str(&self.output(&argv)).unwrap_or(Value::Null)
    }

    fn running(&self, pattern: &str) -> bool {
        self.succeeds(&["pgrep", "-f", pattern])
    }

    /// The first executable of that name on the doctor's PATH.
    fn which(&self, name: &str) -> Option<PathBuf> {
        self.path.split(':').filter(|d| !d.is_empty()).map(|d| Path::new(d).join(name)).find(|p| proc::executable(p))
    }

    fn enabled(&self, source: &str) -> bool {
        is_true(or(&self.set()[source]["enabled"], &Value::Bool(false)))
    }

    fn has_token(&self, source: &str) -> bool {
        self.config.as_ref().is_some_and(|c| length(&c[source]["token"]) > 0.0)
    }

    /// The calendar's secret iCal address, which the hub fetches itself; never printed.
    fn has_ics(&self) -> bool {
        self.config.as_ref().is_some_and(|c| c["calendar"]["ics"].as_str().is_some_and(|ics| !ics.trim().is_empty()))
    }

    // -----------------------------------------------------------------------------------------------------------------
    // Tools and config

    fn tools(&self) {
        let said = self.output(&[&self.herdr, "--version"]);
        let v = said.lines().map(|l| l.split_whitespace().nth(1).unwrap_or_default()).collect::<Vec<_>>().join("\n");
        let v = v.trim_end_matches('\n');
        if v.is_empty() && self.app.is_some() {
            ok("tools.herdr", "herdr not installed: cockpit runs in the Herdr app alone");
        } else if v.is_empty() || glob("0.[0-8].*", v) {
            fail("tools.herdr", &format!("herdr {}: cockpit needs 0.9 or later → herdr update", if v.is_empty() { "missing" } else { v })).run("herdr update");
        } else {
            ok("tools.herdr", &format!("herdr {v}"));
        }
        if self.succeeds(&["gh", "auth", "status"]) {
            ok("tools.gh", "gh signed in")
        } else {
            fail("tools.gh", "gh not signed in → gh auth login").run("gh auth login")
        };

        // The release's binary, else a dev build: it runs every app and the hub.
        let root = quote(&self.root.to_string_lossy());
        let mut dir = self.root.join("bin");
        let dev = self.root.join("tui/target/release");
        if !proc::executable(&dir.join("cockpit")) && proc::executable(&dev.join("cockpit")) {
            dir = dev;
        }
        let toml = std::fs::read_to_string(self.root.join("herdr-plugin.toml")).unwrap_or_default();
        let want = toml.lines().filter_map(|l| l.strip_prefix("version = \"")?.strip_suffix('"')).collect::<Vec<_>>().join("\n");
        let have = std::fs::read_to_string(dir.join(".version")).unwrap_or_default().trim_end_matches('\n').to_string();
        let binary = dir.join("cockpit");
        let runs = self.output(&[&binary.to_string_lossy(), "--version"]);
        if !proc::executable(&binary) {
            fail("tools.binary", &format!("cockpit binary missing, the spaces stay empty → sh {root}/fetch-apps.sh, or cargo build --release in {root}/tui"))
                .run(format!("sh {root}/fetch-apps.sh"));
        } else if dir == self.root.join("bin") && have != want {
            let have = if have.is_empty() { "an unknown version" } else { &have };
            warn(
                "tools.binary",
                &format!("cockpit binary from {have}, the plugin is on {want}: the hub installs it once the release has it → sh {root}/fetch-apps.sh"),
            )
            .run(format!("sh {root}/fetch-apps.sh"));
        } else if runs.is_empty() {
            fail("tools.binary", &format!("cockpit binary in {} does not run on this machine → rm -rf {root}/bin && sh {root}/fetch-apps.sh", dir.display()))
                .run(format!("rm -rf {root}/bin && sh {root}/fetch-apps.sh"));
        } else {
            ok("tools.binary", &format!("cockpit binary in {} ({})", dir.display(), runs.strip_prefix("cockpit ").unwrap_or(&runs)));
        }
        self.hub(&binary);
        let config = files::config_dir().join("config.json");
        if !config.is_file() {
            fail("tools.config", "no ~/.config/cockpit/config.json → cockpit setup (/cockpit:setup in Claude Code)").run("cockpit setup");
        } else if std::fs::read_to_string(&config).is_ok_and(|text| serde_json::Deserializer::from_str(&text).into_iter::<Value>().all(|v| v.is_ok())) {
            ok("tools.config", "config.json");
        } else {
            fail("tools.config", "config.json is not valid JSON → fix it or rerun cockpit setup").run("cockpit setup");
        }
    }

    /// The hub: stopped on purpose, which nothing starts again; else, while a space app runs, not started yet, answering,
    /// or left with a dead socket that keeps every action and search waiting.
    fn hub(&self, binary: &Path) {
        if self.cache.join("hub.stopped").is_file() {
            warn("tools.hub", "the hub was stopped: run cockpit hub start → cockpit hub start").run("cockpit hub start");
            return;
        }
        if !self.running(&format!("cockpit ({}) cockpit-space", cockpit_core::SPACE_APPS.join("|"))) {
            return;
        }
        if !self.running("(^|/)cockpit hub$") {
            ok("tools.hub", "hub not started yet: the apps start it at their first action or search");
        } else if self.succeeds(&[&binary.to_string_lossy(), "hub", "ping"]) {
            ok("tools.hub", "hub answers");
        } else {
            fail("tools.hub", "hub not answering → pkill -f '(^|/)cockpit hub$'; the apps start a new one").run("pkill -f '(^|/)cockpit hub$'");
        }
    }

    // -----------------------------------------------------------------------------------------------------------------
    // Both halves installed, and the link on the current herdr install

    fn installs(&self) {
        self.folders();
        self.binary_link();
    }

    /// The `cockpit` a shell runs, which has to go through the link: an update installs in a new folder and prunes the
    /// one a link named.
    fn binary_link(&self) {
        let bin = files::home().join(".local/bin/cockpit");
        if std::fs::symlink_metadata(&bin).is_err() {
            return;
        }
        let through = self.link.join("bin/cockpit");
        let fix = format!("→ {LINK_BIN}");
        match (bin.canonicalize(), through.canonicalize()) {
            (Ok(linked), Ok(current)) if linked == current => ok("install.bin_link", "link ~/.local/bin/cockpit is current"),
            (Ok(linked), _) => warn("install.bin_link", &format!("~/.local/bin/cockpit runs {} instead of this install {fix}", linked.display())).run(LINK_BIN),
            (Err(_), _) => {
                fail("install.bin_link", &format!("~/.local/bin/cockpit points at a folder that is gone, so no shell finds cockpit {fix}")).run(LINK_BIN)
            }
        };
    }

    fn folders(&self) {
        let registered = &self.registered;
        let at = quote(registered);
        if !self.has_herdr {
            // The Herdr app alone runs the scripts install.sh put in place, else the Claude plugin's marketplace copy.
            let installed = files::home().join(".local/share/cockpit/herdr");
            let shipped =
                if installed.join("cockpit.sh").is_file() { installed } else { transcripts::claude_home().join("plugins/marketplaces/cockpit/herdr") };
            let linked = readlink(&self.link);
            if Path::new(&linked).join("cockpit.sh").is_file() {
                ok("install.link", &format!("link ~/.config/cockpit/herdr points at {linked}"));
            } else {
                let shipped = quote(&shipped.to_string_lossy());
                fail("install.link", &format!("no ~/.config/cockpit/herdr link → ln -sfn {shipped} ~/.config/cockpit/herdr"))
                    .run(format!("ln -sfn {shipped} ~/.config/cockpit/herdr"));
            }
            return;
        }
        if !registered.is_empty() && Path::new(registered).is_dir() {
            ok("install.plugin", &format!("herdr plugin at {registered}"));
        } else {
            fail("install.plugin", "herdr plugin not installed → herdr plugin install quaresc/cockpit/herdr --yes")
                .run("herdr plugin install quaresc/cockpit/herdr --yes");
        }
        let linked = readlink(&self.link);
        if linked == *registered {
            ok("install.link", "link ~/.config/cockpit/herdr is current");
        } else {
            fail("install.link", &format!("link ~/.config/cockpit/herdr points at {linked} → sh {at}/ensure-spaces.sh"))
                .run(format!("sh {at}/ensure-spaces.sh"));
        }
        let is_github_install = registered.strip_suffix("/checkout/herdr").is_some_and(|rest| rest.contains("/github-installations/"));
        if is_github_install {
            let install = Path::new(registered).parent().and_then(Path::parent).unwrap_or(Path::new("/"));
            let installs = install.parent().unwrap_or(Path::new("/"));
            let old =
                std::fs::read_dir(installs).into_iter().flatten().flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir()) && e.path() != install).count();
            if old == 0 {
                ok("install.old", "no old herdr installs");
            } else {
                warn("install.old", &format!("{old} old herdr installs → sh {at}/ensure-spaces.sh prunes them")).run(format!("sh {at}/ensure-spaces.sh"));
            }
        }
    }

    // -----------------------------------------------------------------------------------------------------------------
    // The agents: the default one installed, and cockpit's tools in each installed agent's MCP config

    fn agents(&self) {
        let set = self.config.as_ref().map_or(Value::Null, |c| c["agents"]["default"].clone());
        match set.as_str().map(|id| (id, cockpit_core::agent::get(id))) {
            Some((_, Some(agent))) if agent.installed() => ok("agents.default", &format!("default agent {} ({})", agent.name, agent.path())),
            Some((id, Some(_))) => fail("agents.default", &format!("default agent {id} is not installed → install it, or pick another with cockpit setup")),
            Some((id, None)) => {
                fail("agents.default", &format!("default agent {id} is unknown → cockpit setup picks one of the installed agents")).run("cockpit setup")
            }
            None => {
                let agent = cockpit_core::agent::default();
                warn("agents.default", &format!("no agents.default in config.json: cockpit starts {} → cockpit setup picks yours", agent.name))
                    .run("cockpit setup")
            }
        };
        let bin = crate::connect::binary();
        for target in crate::connect::TARGETS.iter().filter(|t| t.installed()) {
            match target.current() {
                Err(e) => fail(
                    &format!("agents.tools.{}", target.id),
                    &format!("{}: {} does not read ({e}) → fix it, then cockpit agents connect {}", target.name, target.path().display(), target.id),
                ),
                _ if target.is_connected(&bin) => ok(&format!("agents.tools.{}", target.id), &format!("{} has cockpit's tools", target.name)),
                Ok(Some(_)) => warn(
                    &format!("agents.tools.{}", target.id),
                    &format!("{} runs cockpit's tools from another binary → cockpit agents connect {}", target.name, target.id),
                )
                .run(format!("cockpit agents connect {}", target.id)),
                Ok(None) => warn(
                    &format!("agents.tools.{}", target.id),
                    &format!("{} has no cockpit tools (show, search, changes) → cockpit agents connect {}", target.name, target.id),
                )
                .run(format!("cockpit agents connect {}", target.id)),
            };
        }
        // A permission pattern that does not compile is left out; the measured ones stand in when none is left.
        for (id, pattern) in cockpit_core::agent::unreadable_permission() {
            warn(
                &format!("agents.permission.{id}"),
                &format!("agents.commands.{id}.permission: {pattern:?} is not a regex cockpit can read, so it is left out → fix it in config.json"),
            );
        }
    }

    // -----------------------------------------------------------------------------------------------------------------
    // The Herdr app half: the layout and mod in its home, on this binary's version, loading cleanly

    fn herdr_app(&self) {
        let Some(app) = &self.app else {
            ok("app.installed", "Herdr app not installed: the herdr half alone (https://herdr.dev installs the app)");
            return;
        };
        let home = cockpit_core::frontend::app_home();
        let fix = format!("{} app install", self.script("cockpit.sh"));
        if !home.join("layouts/cockpit.json").is_file() || !home.join("mods/cockpit/main.js").is_file() {
            warn("app.layout", &format!("Herdr app without the cockpit layout and mod → {fix}")).run(fix.clone());
            return;
        }
        if !home.join("layouts/cockpit-chat.json").is_file() || !home.join("themes/cockpit-chat.json").is_file() {
            warn("app.chat", &format!("Herdr app without the Cockpit Chat layout or its theme → {fix}")).run(fix.clone());
        }
        let noted = load(&home.join("cockpit.json")).unwrap_or(Value::Null);
        let bin = raw(&noted["bin"]);
        let ours = env!("CARGO_PKG_VERSION");
        if bin.is_empty() || !proc::executable(Path::new(&bin)) {
            fail("app.layout", &format!("the cockpit mod names no cockpit binary that runs ({}) → {fix}", if bin.is_empty() { "none" } else { &bin }))
                .run(fix.clone());
        } else if raw(&noted["version"]) != ours {
            warn("app.layout", &format!("the cockpit mod is from {}, this cockpit is {ours} → {fix}", raw(&noted["version"]))).run(fix.clone());
        } else {
            ok("app.layout", &format!("Herdr app: cockpit layout and mod {ours}"));
        }
        let checked = self.run(&[&app.to_string_lossy(), "check", &home.join("mods/cockpit").to_string_lossy()], Duration::from_secs(30));
        match checked {
            Some(o) if o.ok() => ok("app.mod", "Herdr app: the cockpit mod loads"),
            Some(o) => fail(
                "app.mod",
                &format!(
                    "Herdr app: the cockpit mod does not load: {} → {fix}",
                    o.stderr.lines().chain(o.stdout.lines()).find(|l| !l.trim().is_empty()).unwrap_or("no reason given")
                ),
            )
            .run(fix.clone()),
            None => warn("app.mod", "Herdr app: herdr-app check did not answer"),
        };
    }

    /// Each Claude session in a Herdr app terminal should beat with the installed version, as `app:<terminal>`.
    fn app_sessions(&self, installed: &str) {
        let Some(app) = &self.app else { return };
        let listed: Value = serde_json::from_str(&self.output(&[&app.to_string_lossy(), "ls"])).unwrap_or(Value::Null);
        let all = beats(&self.cache.join("sessions"));
        for space in values(&listed["spaces"]) {
            for tab in values(&space["tabs"]) {
                for terminal in values(&tab["terminals"]) {
                    let address = raw(&terminal["terminal"]);
                    let program = cockpit_core::frontend::hc(&["get", &address]).map(|t| raw(&t["foreground"]["name"])).unwrap_or_default();
                    if !program.contains("claude") {
                        continue;
                    }
                    let label = format!("{} / {}", raw(&space["name"]), raw(&tab["name"]));
                    let handle = format!("app:{address}");
                    let beat = max_by_at(&all.iter().filter(|b| b["pane"] == handle.as_str()).collect::<Vec<_>>()).cloned();
                    match beat.map(|b| raw(&b["version"])) {
                        None => warn(
                            &format!("sessions.{address}"),
                            &format!("session \"{label}\" ({address}) runs no cockpit → /reload-plugins in it, or restart it"),
                        ),
                        Some(v) if !installed.is_empty() && v != installed => warn(
                            &format!("sessions.{address}"),
                            &format!("session \"{label}\" runs cockpit {v}, {installed} is installed → {}", self.script("reload.sh")),
                        )
                        .run(self.script("reload.sh")),
                        Some(v) => ok(&format!("sessions.{address}"), &format!("session \"{label}\" on cockpit {v}")),
                    };
                }
            }
        }
    }

    /// The Claude plugin's installed version, empty when it is not installed (a failure when Claude is the default agent).
    fn claude_plugin(&self, is_default: bool) -> String {
        let listed = self.output(&["claude", "plugin", "list"]);
        let mut found = false;
        let mut installed = String::new();
        for line in listed.lines() {
            found |= line.contains("cockpit@cockpit");
            if found && line.contains("Version:") {
                installed = line.split_whitespace().nth(1).unwrap_or_default().to_string();
                break;
            }
        }
        if installed.is_empty() {
            let line = "Claude plugin not installed → /plugin install cockpit --marketplace quaresc/cockpit";
            if is_default {
                fail("claude.plugin", line);
            } else {
                warn("claude.plugin", line);
            }
        } else {
            ok("claude.plugin", &format!("Claude plugin cockpit {installed}"));
        }
        let update = load(&self.cache.join("update.json")).unwrap_or(Value::Null);
        let latest = if truthy(&update["latest"]) { raw(&update["latest"]) } else { String::new() };
        if !latest.is_empty() && !installed.is_empty() && latest != installed && newest(&latest, &installed) == latest {
            warn("claude.update", &format!("cockpit {latest} is published → {}", self.script("update.sh"))).run(self.script("update.sh"));
        }
        installed
    }

    // -----------------------------------------------------------------------------------------------------------------
    // Claude sessions: each one in herdr should beat with the installed version

    fn sessions(&self, installed: &str) {
        let dir = self.cache.join("sessions");
        prune(&dir, Duration::from_secs(5 * 60));
        let panes = self.herdr_json(&["pane", "list"]);
        let claude: Vec<&Value> = panes["result"]["panes"].as_array().into_iter().flatten().filter(|p| p["agent"] == "claude").collect();
        if claude.is_empty() {
            return;
        }
        let spaces = self.herdr_json(&["workspace", "list"]);
        let all = beats(&dir);
        for pane in claude {
            let (pane, ws) = (raw(&pane["pane_id"]), &pane["workspace_id"]);
            let label = values(&spaces["result"]["workspaces"]).filter(|w| w["workspace_id"] == *ws).map(|w| raw(&w["label"])).collect::<Vec<_>>().join("\n");
            let label = strip_index(&label);
            // A pane can hold beats from more than one instance (the conversation before a /clear on old code): the one
            // on the installed version is the session; any other still beating is a leftover.
            let beats: Vec<&Value> = all.iter().filter(|b| b["pane"] == pane.as_str()).collect();
            let current: Vec<&Value> = beats.iter().copied().filter(|b| b["version"] == installed).collect();
            let beat = max_by_at(&current).or_else(|| max_by_at(&beats));
            let version = beat.map(|b| &b["version"]).filter(|v| truthy(v)).map(raw).unwrap_or_default();
            let others = beats.iter().filter(|b| b["version"] != version.as_str()).count();
            if beat.is_none() {
                warn(&format!("sessions.{pane}"), &format!("session \"{label}\" ({pane}) runs no cockpit → /reload-plugins in it, or restart it"));
            } else if !installed.is_empty() && version != installed {
                warn(
                    &format!("sessions.{pane}"),
                    &format!("session \"{label}\" runs cockpit {version}, {installed} is installed → {}", self.script("reload.sh")),
                )
                .run(self.script("reload.sh"));
            } else if others > 0 {
                warn(
                    &format!("sessions.{pane}"),
                    &format!(
                        "session \"{label}\" on cockpit {version}, but an older cockpit still runs in its pane → restart Claude there (exit, then claude --resume)"
                    ),
                );
            } else {
                ok(&format!("sessions.{pane}"), &format!("session \"{label}\" on cockpit {version}"));
            }
        }
    }

    // -----------------------------------------------------------------------------------------------------------------
    // Shared caches: fresh, and without errors. The hub fetches GitHub (and Linear or CircleCI given a token) while an
    // app runs; the rest comes from a Claude session.

    fn check_cache(&self, file: &str, name: &str, limit: i64, by: &str) {
        let path = self.cache.join(file);
        let id = format!("sources.cache.{}", file.trim_end_matches(".json"));
        if !path.is_file() {
            warn(&id, &format!("{name}: no data yet → read it now")).act("sources", "refresh", json!([]));
            return;
        }
        let data = load(&path).unwrap_or(Value::Null);
        // The hub stamps a source's first failure before any data came: its record says what failed, not an age.
        let failing: Vec<String> =
            data["hub"]["fresh"].as_object().into_iter().flatten().filter_map(|(_, r)| r["error"]["message"].as_str().map(str::to_string)).collect();
        if !truthy(&data["at"]) {
            match failing.first() {
                Some(message) => fail(&id, &format!("{name}: {message}")),
                None => warn(&id, &format!("{name}: no data yet → read it now")).act("sources", "refresh", json!([])),
            };
            return;
        }
        let age = self.now - millis_to_secs(&data["at"]);
        let mut errors: Vec<String> = match &data["errors"] {
            Value::Array(items) => items.iter().map(joined).collect(),
            Value::Object(items) => items.values().map(joined).collect(),
            _ => Vec::new(),
        };
        if truthy(&data["error"]) {
            errors.push(joined(&data["error"]));
        }
        let errors = errors.join("; ");
        if !errors.is_empty() {
            fail(&id, &format!("{name}: {errors}"));
        } else if age > limit {
            warn(&id, &format!("{name}: last refresh {} min ago → {by}", age / 60)).act("sources", "refresh", json!([]));
        } else {
            ok(&id, &format!("{name}: refreshed {} min ago", age / 60));
        }
    }

    fn caches(&self) {
        self.check_cache("inbox.json", "inbox", 900, NO_FEED);
        // Each source with a server of cockpit's own, and tickets from a Claude session's connector otherwise.
        let inbox = self.cache.join("inbox.json");
        let mut hub_tickets = false;
        for section in ["code", "tickets", "ci", "calendar", "platform", "observability", "datadog", "mail"] {
            let name = self.config.as_ref().map(|c| raw(&c[section]["server"])).unwrap_or_default();
            let entry = self.config.as_ref().map(|c| c["servers"][name.as_str()].clone()).filter(Value::is_object);
            let on = ["observability", "datadog", "code"].contains(&section) || self.config.as_ref().is_some_and(|c| c[section]["enabled"] != false);
            let Some(entry) = entry.filter(|_| on) else { continue };
            use cockpit_connectors::Server;
            match Server::from_entry(&entry) {
                Some(server) if server.is_ready() => {
                    hub_tickets |= section == "tickets";
                    ok(&format!("sources.server.{section}"), &format!("{section} from \"{name}\" through cockpit's own connection"))
                }
                Some(server) if server.login_url().is_some() => {
                    let url = server.login_url().unwrap_or_default();
                    fail(&format!("sources.server.{section}"), &format!("{section}: cockpit is not logged in to {url} → cockpit connector login {name}"))
                        .run(format!("cockpit connector login {}", quote(&name)))
                }
                _ => fail(&format!("sources.server.{section}"), &format!("servers.\"{name}\" has no url or command → cockpit setup")).run("cockpit setup"),
            };
        }
        if self.enabled("tickets") {
            let hub_reads = hub_tickets || self.has_token("tickets");
            tickets(&load(&inbox).unwrap_or(Value::Null), hub_reads, now_ms());
        }
        if self.settings.is_some() && self.set()["ci"]["enabled"] != false {
            let server = &self.set()["ci"]["server"];
            let by = if !truthy(server) || raw(server).is_empty() || self.has_token("ci") {
                NO_FEED.to_string()
            } else {
                format!("{SESSION} (cockpit setup gives the hub a CI server of its own)")
            };
            self.check_cache("ci.json", "CI", 600, &by);
        }
        // Meetings come from the calendar connector, in a Claude session, unless calendar.ics lets the hub fetch them.
        if self.enabled("calendar") {
            let by = if self.has_ics() { NO_FEED.to_string() } else { format!("{SESSION} (calendar.ics lets the hub fetch the calendar without one)") };
            self.check_cache("meetings.json", "meetings", 900, &by);
        }
        // The monitor space is opt-in: its document is checked only when the space is pinned.
        if pinned_spaces(self.settings.as_ref()).iter().any(|s| s == "monitor") {
            self.check_cache("monitor.json", "monitor", 900, SESSION);
        }
    }

    // -----------------------------------------------------------------------------------------------------------------
    // Pinned spaces and their apps

    fn spaces(&self) {
        if !self.has_herdr {
            return;
        }
        let listed = self.herdr_json(&["workspace", "list"]);
        // The Herdr app alone runs no herdr server: the pinned spaces are the herdr TUI's, checked while it runs.
        if listed["error"]["code"] == "server_not_running" {
            ok("spaces", "herdr is not running: its pinned spaces are checked while it runs");
            return;
        }
        let labels: Vec<String> = values(&listed["result"]["workspaces"]).map(|w| raw(&w["label"])).collect();
        let herdr_config = std::fs::read_to_string(files::home().join(".config/herdr/config.toml")).ok();
        let link = quote(&self.link.to_string_lossy());
        for space in pinned_spaces(self.settings.as_ref()) {
            if space.is_empty() {
                continue;
            }
            if !labels.iter().any(|l| l.ends_with(&format!("  {space}"))) {
                fail(&format!("spaces.{space}"), &format!("space {space} missing → sh {link}/ensure-spaces.sh")).run(format!("sh {link}/ensure-spaces.sh"));
                continue;
            }
            let shown = herdr_config.as_ref().is_some_and(|c| c.contains("contains = \"  \"") || c.contains(&format!("contains = \"  {space}\"")));
            if !shown {
                warn(
                    &format!("spaces.{space}.shown"),
                    &format!(
                        "space {space} is hidden in the sidebar → add {{ contains = \"  \" }} before the hiding rule of ui.sidebar.spaces in ~/.config/herdr/config.toml"
                    ),
                );
            }
            if space == "review" && herdr_config.as_ref().is_some_and(|c| !c.contains("$review_space")) {
                warn(
                    "spaces.review.token",
                    "space review shows no count in the sidebar → cockpit setup adds { token = \"$review_space\", dim = true } to ui.sidebar.spaces",
                );
            }
            let Some(app) = cockpit_core::SPACE_APPS.iter().find(|app| cockpit_core::space_of(app) == space) else {
                ok(&format!("spaces.{space}"), &format!("space {space}"));
                continue;
            };
            if self.running(&format!("(^|/)cockpit space {app}$")) {
                ok(&format!("spaces.{space}"), &format!("space {space}, app running"));
            } else {
                fail(&format!("spaces.{space}"), &format!("space {space} has no app running → sh {link}/ensure-spaces.sh"))
                    .run(format!("sh {link}/ensure-spaces.sh"));
            }
        }
    }

    // -----------------------------------------------------------------------------------------------------------------
    // Notifications

    fn notifications(&self) {
        if self.settings.is_none() || !is_true(or(&self.set()["notifications"], &Value::Bool(true))) {
            return;
        }
        let build = format!("{}/notifier/build.sh", self.root.display());
        let app = files::config_dir().join("Cockpit.app/Contents/MacOS/cockpit-notify");
        if let Some(program) = std::env::var_os(cockpit_core::notify::TEST_PROGRAM).filter(|p| !p.is_empty()) {
            // Notifications go to that program (tests, the sandbox): no notifier is built or asked.
            ok("notifications.allowed", &format!("notifications go to {}", program.to_string_lossy()));
        } else if cfg!(target_os = "macos") && QUIET.load(Atomic::Relaxed) && !proc::executable(&app) {
            // A run for the apps never builds: the build may ask to install Apple's command line tools.
            warn("notifications.allowed", &format!("notifier not built → sh {}", quote(&build))).run(format!("sh {}", quote(&build)));
        } else if cfg!(target_os = "macos") {
            let built = QUIET.load(Atomic::Relaxed) || self.run(&["sh", &build], Duration::from_secs(600)).is_some_and(|o| o.ok());
            if built {
                let status = self.output(&[&app.to_string_lossy(), "status"]);
                if status == "allowed" {
                    ok("notifications.allowed", "notifications allowed");
                } else {
                    let status = if status.is_empty() { "unknown" } else { &status };
                    fail("notifications.allowed", &format!("notifications {status} → System Settings → Notifications → Cockpit → Allow"))
                        .run("open x-apple.systempreferences:com.apple.Notifications-Settings.extension");
                }
            } else {
                fail("notifications.allowed", &format!("notifier not built → xcode-select --install, then sh {}", quote(&build)));
            }
        } else if self.which("notify-send").is_some() {
            ok("notifications.allowed", "notify-send");
        } else {
            warn("notifications.allowed", "notify-send missing: herdr's toasts are used instead");
        }
        if !self.has_herdr {
            return;
        }
        let herdr_config = std::fs::read_to_string(files::home().join(".config/herdr/config.toml")).unwrap_or_default();
        let delivery = toast_delivery(&herdr_config);
        if delivery == "herdr" {
            ok("notifications.toasts", "herdr toasts in-app");
        } else {
            let delivery = if delivery.is_empty() { "system" } else { &delivery };
            warn("notifications.toasts", &format!("herdr ui.toast.delivery is \"{delivery}\": notifications arrive twice → set it to \"herdr\""));
        }
    }

    // -----------------------------------------------------------------------------------------------------------------
    // terminal-browser for an agent's E2E tests beside it: optional, they run in Claude in Chrome without it

    fn terminal_browser(&self) {
        if self.which("terminal-browser").is_none() {
            ok(
                "browser.installed",
                "terminal-browser not installed: E2E tests run in Claude in Chrome (brew install --cask terminal-browser runs them beside the agent)",
            );
            return;
        }
        let said = self.output(&["terminal-browser", "--version"]);
        ok(
            "browser.installed",
            &format!("terminal-browser {}", said.lines().map(|l| l.split_whitespace().nth(1).unwrap_or_default()).collect::<Vec<_>>().join("\n")),
        );
        let mut installs: Vec<PathBuf> = self
            .path
            .split(':')
            .filter(|d| !d.is_empty())
            .map(|d| Path::new(d).join("terminal-browser"))
            .filter(|p| proc::executable(p))
            .filter_map(|p| p.canonicalize().ok())
            .collect();
        installs.sort();
        installs.dedup();
        if installs.len() > 1 {
            warn(
                "browser.installs",
                &format!(
                    "{} terminal-browser installs, each with its own sign-ins → keep the brew one: rm ~/.local/bin/terminal-browser && rm -rf ~/.local/share/terminal-browser",
                    installs.len()
                ),
            );
        }
        let herdr_config = std::fs::read_to_string(files::home().join(".config/herdr/config.toml")).unwrap_or_default();
        let graphics_off = herdr_config.lines().any(|l| {
            let l = l.trim_start();
            l.strip_prefix("kitty_graphics").is_some_and(|rest| rest.trim_start().strip_prefix('=').is_some_and(|v| v.trim_start().starts_with("false")))
        });
        if graphics_off {
            fail("browser.graphics", "herdr kitty_graphics is off: terminal-browser panes stay blank → set it to true under [experimental]");
        } else {
            ok("browser.graphics", "herdr kitty graphics on");
        }
        let left = self.output(&["sh", &format!("{}/prune-browsers.sh", self.root.display()), "--count"]);
        if left.trim().parse::<i64>() == Ok(0) {
            ok("browser.daemons", "no agent-browser daemons left by closed browsers");
        } else {
            warn("browser.daemons", &format!("{left} agent-browser daemons left by closed browsers → {}", self.script("prune-browsers.sh")))
                .run(self.script("prune-browsers.sh"));
        }
        let transport = self.output(&["terminal-browser", "config", "get", "render.transport"]);
        if transport == "auto" {
            ok("browser.transport", "terminal-browser frame transport auto");
        } else {
            let transport = if transport.is_empty() { "unset" } else { &transport };
            warn("browser.transport", &format!(
                "terminal-browser render.transport is {transport}: herdr cannot hand its frames to the terminal as files → terminal-browser config set render.transport auto"
            ))
            .run("terminal-browser config set render.transport auto");
        }
        let fps = self.output(&["terminal-browser", "config", "get", "render.fps"]);
        if fps == "60" {
            ok("browser.fps", "terminal-browser at 60 frames per second");
        } else {
            let fps = if fps.is_empty() { "display" } else { &fps };
            warn(
                "browser.fps",
                &format!("terminal-browser render.fps is {fps}: 60 scrolls smoothly and leaves herdr room → terminal-browser config set render.fps 60"),
            )
            .run("terminal-browser config set render.fps 60");
        }
        // herdr hands frames to Ghostty as files in /tmp, which Ghostty reads on macOS from 1.3.2 on (1.3.1 and older
        // refuse them).
        let ghostty = "/Applications/Ghostty.app/Contents/MacOS/ghostty";
        if cfg!(target_os = "macos") && proc::executable(Path::new(ghostty)) {
            let info = self.output(&[ghostty, "--version"]);
            let field = |name: &str| info.lines().filter_map(|l| l.trim_start_matches(' ').strip_prefix(name)).collect::<Vec<_>>().join("\n");
            let (version, channel) = (field("- version: "), field("- channel: "));
            let said = format!("{channel}:{version}");
            if ["tip:*", "*:1.3.[2-9]*", "*:1.[4-9].*", "*:[2-9].*"].iter().any(|p| glob(p, &said)) {
                ok("browser.ghostty", &format!("Ghostty {version} ({channel}) reads herdr's frame files"));
            } else {
                warn(
                    "browser.ghostty",
                    &format!(
                        "Ghostty {version} ({channel}) refuses herdr's frame files, so terminal-browser panes are slow → update Ghostty to 1.4.0 or newer: Ghostty → Check for Updates…, or ghostty.org/download"
                    ),
                );
            }
        }
    }

    // -----------------------------------------------------------------------------------------------------------------
    // Connectors: their tools as /cockpit:setup mapped them, and the permission rules their background calls need

    fn connectors(&self) {
        let set = self.set();
        let config = self.config.as_ref().unwrap_or(&Value::Null);
        // The platform connector's tools, mapped to what cockpit does with deploys and previews.
        if self.enabled("platform") {
            let tools = &set["platform"]["tools"];
            let mut mapped: Vec<&str> =
                tools.as_object().into_iter().flatten().filter(|(_, v)| length(or(&v["tool"], &Value::from(""))) > 0.0).map(|(k, _)| k.as_str()).collect();
            let order = platform_tools_order(&self.settings_path);
            mapped.sort_by_key(|name| order.iter().position(|o| o == name).unwrap_or(usize::MAX));
            if mapped.is_empty() {
                warn("connectors.platform", "platform is on but no tool is mapped: no deploys or previews → /cockpit:setup or cockpit connector map maps them");
            } else {
                ok("connectors.platform", &format!("platform tools mapped: {}", mapped.join(", ")));
            }
        }
        // The ticket connector's tools (Linear's by default).
        if truthy(&config["linear"]) && !truthy(&config["tickets"]) {
            warn("connectors.tickets", "config has the old linear block: it still works as Linear's mapping → /cockpit:setup rewrites it as tickets");
        } else if self.enabled("tickets") {
            // A mapping with no tool listing the states a ticket moves to holds them in `states` instead (GitHub's open
            // and closed), which cockpit reads in its place.
            let listed = set["tickets"]["states"].as_array().is_some_and(|states| !states.is_empty());
            let wanted: &[&str] = if listed { &["listMine", "get", "setState"] } else { &["listMine", "get", "listStates", "setState"] };
            let missing = unmapped(&set["tickets"]["tools"], wanted);
            if missing.is_empty() {
                ok("connectors.tickets", "ticket tools mapped");
            } else {
                warn(
                    "connectors.tickets",
                    &format!("ticket tools not mapped: {missing}; cockpit skips what they do → /cockpit:setup or cockpit connector map maps them"),
                );
            }
        }
        // The CI: GitHub's checks through gh, or a CI connector.
        if truthy(&config["circleci"]) && !truthy(&config["ci"]) {
            warn("connectors.ci", "config has the old circleci block: it still works as CircleCI's mapping → /cockpit:setup rewrites it as ci");
        } else if truthy(&set["ci"]["server"]) && !raw(&set["ci"]["server"]).is_empty() {
            let missing = unmapped(&set["ci"]["tools"], &["listRuns", "listJobs"]);
            if missing.is_empty() {
                ok("connectors.ci", "CI tools mapped");
            } else {
                warn("connectors.ci", &format!("CI tools not mapped: {missing}; no CI runs → /cockpit:setup or cockpit connector map maps them"));
            }
        } else {
            ok("connectors.ci", "CI from GitHub checks");
        }
        // The calendar's secret iCal address, which the hub fetches instead of a session asking the connector.
        if self.has_ics() {
            let ics = raw(&config["calendar"]["ics"]);
            let ics = ics.trim();
            if !self.enabled("calendar") {
                warn("connectors.calendar", "calendar.ics is set but the calendar is off: the hub leaves it → set calendar.enabled to true");
            } else if !ics.starts_with("https://") && !ics.starts_with("webcal://") {
                warn(
                    "connectors.calendar",
                    "calendar.ics is not an https:// address: no meetings from the hub → paste the calendar's \"Secret address in iCal format\"",
                );
            } else {
                ok("connectors.calendar", "calendar fetched by the hub from its secret iCal address");
            }
        }

        // Permission rules for the background MCP calls, which only Claude Code sessions make.
        let claude = load(&transcripts::claude_home().join("settings.json"));
        if claude.is_none() {
            return;
        }
        let allow: Vec<&str> = claude.as_ref().map(|c| values(&c["permissions"]["allow"]).filter_map(Value::as_str).collect()).unwrap_or_default();
        let mut servers: Vec<String> = Vec::new();
        for source in ["tickets", "ci", "platform", "calendar"] {
            let block = &set[source];
            if !is_true(or(&block["enabled"], &Value::Bool(false))) {
                continue;
            }
            let fallback = Value::from(if source == "tickets" { "claude.ai Linear" } else { "" });
            let server = raw(or(&block["server"], &fallback));
            if !server.is_empty() && !servers.iter().any(|s| rule_name(s) == rule_name(&server)) {
                servers.push(server);
            }
        }
        // One check per server: two sources read through the same one share its rule.
        for server in servers {
            let rule = format!("mcp__{}", rule_name(&server));
            if claude.is_some() && allow.iter().any(|a| *a == rule || a.starts_with(&format!("{rule}__"))) {
                ok(&format!("rules.{}", rule_name(&server)), &format!("allow rule for {server}"));
            } else {
                fail(
                    &format!("rules.{}", rule_name(&server)),
                    &format!("no allow rule for {server}: background refreshes are refused → add \"{rule}\" to permissions.allow"),
                );
            }
        }
        // Mail through a Claude connector: its mapped tools one by one, so the server's sending tools stay asked.
        let mail = &set["mail"];
        let mail_server = raw(or(&mail["server"], &Value::from("")));
        let hub_mail = self.config.as_ref().is_some_and(|c| c["servers"][mail_server.as_str()].is_object());
        if is_true(or(&mail["enabled"], &Value::Bool(false))) && !mail_server.is_empty() && !hub_mail {
            let prefix = format!("mcp__{}", rule_name(&mail_server));
            let mut tools: Vec<&str> = ["listInbox", "get", "archive", "markRead"].iter().filter_map(|v| mail["tools"][*v]["tool"].as_str()).collect();
            // One check per tool, however many verbs it serves.
            let mut seen = std::collections::HashSet::new();
            tools.retain(|t| seen.insert(*t));
            for tool in tools {
                let rule = format!("{prefix}__{tool}");
                if allow.iter().any(|a| *a == rule || *a == prefix) {
                    ok(&format!("rules.{rule}"), &format!("allow rule for {rule}"));
                } else {
                    fail(&format!("rules.{rule}"), &format!("no allow rule for {rule}: mail refreshes and actions are refused → add it to permissions.allow"));
                }
            }
        }
        // The read-only tools live show panes call, and the monitor space's when it is pinned.
        let mut tools = vec![("datadog", "get_datadog_metric"), ("metabase", "execute_query"), ("metabase", "run_saved_question")];
        if pinned_spaces(self.settings.as_ref()).iter().any(|s| s == "monitor") {
            tools.extend([("datadog", "search_datadog_monitors"), ("datadog", "search_datadog_error_tracking_issues")]);
        }
        // Only for the sources the person set up: nobody else needs Datadog or Metabase rules.
        for (source, tool) in tools.into_iter().filter(|(source, _)| config[*source].is_object()) {
            let server = &set[source]["server"];
            let server = if truthy(server) && !raw(server).is_empty() { raw(server) } else { source.replacen("datadog", "claude.ai Datadog", 1) };
            let rule = format!("mcp__{}__{tool}", rule_name(&server));
            if allow.iter().any(|a| rule == *a || rule.starts_with(&format!("{a}__"))) {
                ok(&format!("rules.{rule}"), &format!("allow rule for {rule}"));
            } else {
                warn(&format!("rules.{rule}"), &format!("no allow rule for {rule}: its live pane asks or is refused → add it to permissions.allow"));
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Reading values the way the checks always have

fn load(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// A word as a shell reads it: quoted when it holds anything but letters, digits and `_@%+=:,./-`.
fn quote(word: &str) -> String {
    if !word.is_empty() && word.chars().all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c)) {
        return word.to_string();
    }
    format!("'{}'", word.replace('\'', "'\"'\"'"))
}

fn readlink(path: &Path) -> String {
    std::fs::read_link(path).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The items of an array, or the values of an object.
fn values(value: &Value) -> Box<dyn Iterator<Item = &Value> + '_> {
    match value {
        Value::Array(items) => Box::new(items.iter()),
        Value::Object(items) => Box::new(items.values()),
        _ => Box::new(std::iter::empty()),
    }
}

/// A value printed raw: a string as is, anything else as JSON.
fn raw(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// A value joined into a list: null as nothing, a string as is, anything else as JSON.
fn joined(value: &Value) -> String {
    if value.is_null() { String::new() } else { raw(value) }
}

fn truthy(value: &Value) -> bool {
    !matches!(value, Value::Null | Value::Bool(false))
}

/// The value, else the fallback when it is null or false.
fn or<'a>(value: &'a Value, fallback: &'a Value) -> &'a Value {
    if truthy(value) { value } else { fallback }
}

fn is_true(value: &Value) -> bool {
    *value == true || *value == "true"
}

/// A string's characters, a list's items, a number's size; 0 for null and anything else.
fn length(value: &Value) -> f64 {
    match value {
        Value::String(s) => s.chars().count() as f64,
        Value::Array(items) => items.len() as f64,
        Value::Object(items) => items.len() as f64,
        Value::Number(n) => n.as_f64().unwrap_or_default().abs(),
        _ => 0.0,
    }
}

fn millis_to_secs(value: &Value) -> i64 {
    (value.as_f64().unwrap_or_default() / 1000.0).floor() as i64
}

/// The keys of the settings' `platform.tools` in the order the file writes them.
fn platform_tools_order(path: &Path) -> Vec<String> {
    #[derive(serde::Deserialize)]
    struct Settings {
        platform: Option<Platform>,
    }
    #[derive(serde::Deserialize)]
    struct Platform {
        tools: Option<Keys>,
    }
    struct Keys(Vec<String>);
    struct KeysVisitor;
    impl<'de> serde::de::Visitor<'de> for KeysVisitor {
        type Value = Keys;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("an object")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Keys, A::Error> {
            let mut keys = Vec::new();
            while let Some((key, _)) = map.next_entry::<String, serde::de::IgnoredAny>()? {
                keys.push(key);
            }
            Ok(Keys(keys))
        }
    }
    impl<'de> serde::Deserialize<'de> for Keys {
        fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            d.deserialize_map(KeysVisitor)
        }
    }
    let text = std::fs::read_to_string(path).unwrap_or_default();
    serde_json::from_str::<Settings>(&text).ok().and_then(|s| s.platform?.tools).map(|k| k.0).unwrap_or_default()
}

/// The tools of a connector's mapping with no tool named, as "a, b"; empty when there is no mapping at all.
fn unmapped(tools: &Value, wanted: &[&str]) -> String {
    let Some(tools) = tools.as_object().filter(|_| truthy(tools)) else { return String::new() };
    wanted
        .iter()
        .filter(|name| raw(or(&tools.get(**name).unwrap_or(&Value::Null)["tool"], &Value::from(""))).is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(", ")
}

/// A server's name as Claude Code writes it in a permission rule: every character outside `A-Za-z0-9_-` as `_`, runs
/// of `_` as one, without a trailing one.
fn rule_name(server: &str) -> String {
    let mut name = String::new();
    for c in server.chars() {
        let c = if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' };
        if !(c == '_' && name.ends_with('_')) {
            name.push(c);
        }
    }
    name.strip_suffix('_').map(str::to_string).unwrap_or(name)
}

/// A space label without its "[N] " position.
fn strip_index(label: &str) -> String {
    label
        .lines()
        .map(|line| {
            line.strip_prefix('[')
                .map(|rest| rest.trim_start_matches(|c: char| c.is_ascii_digit()))
                .and_then(|rest| rest.strip_prefix(']'))
                .map_or(line, |rest| rest.trim_start_matches(' '))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The pinned space names: each is "name" (a built-in, or a space in reposDir) or { name, … }.
fn pinned_spaces(settings: Option<&Value>) -> Vec<String> {
    let defaults = || SPACES.iter().map(|s| s.to_string()).collect();
    let Some(settings) = settings else { return defaults() };
    match &settings["spaces"] {
        Value::Array(spaces) => spaces.iter().map(|s| if s.is_object() { joined(&s["name"]) } else { joined(s) }).collect(),
        Value::Null | Value::Bool(false) => defaults(),
        _ => defaults(),
    }
}

/// The `delivery` of herdr's `[ui.toast]` table.
fn toast_delivery(config: &str) -> String {
    let mut inside = false;
    let mut found = Vec::new();
    for line in config.lines() {
        let in_table = if inside {
            inside = !line.starts_with('[');
            true
        } else {
            inside = line.starts_with("[ui.toast]");
            inside
        };
        if !in_table {
            continue;
        }
        let Some(rest) = line.strip_prefix("delivery") else { continue };
        let Some(rest) = rest.trim_start_matches(' ').strip_prefix('=') else { continue };
        let Some(rest) = rest.trim_start_matches(' ').strip_prefix('"') else { continue };
        if let Some(end) = rest.rfind('"') {
            found.push(format!("{}{}", &rest[..end], &rest[end + 1..]));
        }
    }
    found.join("\n")
}

/// Deletes the session beats older than `age`.
fn prune(dir: &Path, age: Duration) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            prune(&path, age);
        } else if path.extension().is_some_and(|e| e == "json")
            && entry.metadata().and_then(|m| m.modified()).is_ok_and(|at| at.elapsed().unwrap_or_default() > age)
        {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Every beat of the session files; none when one does not read as JSON.
fn beats(dir: &Path) -> Vec<Value> {
    let mut names: Vec<PathBuf> =
        std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "json")).collect();
    names.sort();
    let mut all = Vec::new();
    for name in names {
        let text = std::fs::read_to_string(&name).unwrap_or_default();
        for value in serde_json::Deserializer::from_str(&text).into_iter::<Value>() {
            let Ok(value) = value else { return Vec::new() };
            all.push(value);
        }
    }
    all
}

/// The beat with the greatest `at`, the last of equals.
fn max_by_at<'a>(beats: &[&'a Value]) -> Option<&'a Value> {
    beats.iter().copied().fold(None, |best: Option<&Value>, b| match best {
        Some(best) if order(&b["at"], &best["at"]) == Ordering::Less => Some(best),
        _ => Some(b),
    })
}

/// Values in jq's order: null, false, true, numbers, strings, arrays, objects.
fn order(a: &Value, b: &Value) -> Ordering {
    let rank = |v: &Value| match v {
        Value::Null => 0,
        Value::Bool(false) => 1,
        Value::Bool(true) => 2,
        Value::Number(_) => 3,
        Value::String(_) => 4,
        Value::Array(_) => 5,
        Value::Object(_) => 6,
    };
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64().unwrap_or_default().total_cmp(&y.as_f64().unwrap_or_default()),
        (Value::String(x), Value::String(y)) => x.cmp(y),
        (Value::Array(x), Value::Array(y)) => x.iter().zip(y).map(|(x, y)| order(x, y)).find(|o| o.is_ne()).unwrap_or(x.len().cmp(&y.len())),
        _ => rank(a).cmp(&rank(b)),
    }
}

/// The newer of two versions, by their first three numbers, then as text.
fn newest(a: &str, b: &str) -> String {
    let key = |v: &str| -> Vec<u64> {
        let mut fields = v.split('.');
        (0..3).map(|_| fields.next().map_or(0, |f| f.chars().take_while(char::is_ascii_digit).collect::<String>().parse().unwrap_or(0))).collect()
    };
    match key(a).cmp(&key(b)).then_with(|| a.cmp(b)) {
        Ordering::Greater => a.to_string(),
        _ => b.to_string(),
    }
}

/// Whether text matches a shell `case` pattern: `*`, `?` and `[…]` classes with ranges.
fn glob(pattern: &str, text: &str) -> bool {
    fn matches(p: &[char], t: &[char]) -> bool {
        match p.first() {
            None => t.is_empty(),
            Some('*') => (0..=t.len()).any(|i| matches(&p[1..], &t[i..])),
            Some('?') => !t.is_empty() && matches(&p[1..], &t[1..]),
            Some('[') => {
                let Some(close) = p.iter().skip(2).position(|c| *c == ']').map(|i| i + 2) else { return t.first() == Some(&'[') && matches(&p[1..], &t[1..]) };
                let Some(c) = t.first() else { return false };
                let (negated, class) = match p[1] {
                    '!' | '^' => (true, &p[2..close]),
                    _ => (false, &p[1..close]),
                };
                let mut hit = false;
                let mut i = 0;
                while i < class.len() {
                    if i + 2 < class.len() && class[i + 1] == '-' {
                        hit |= (class[i]..=class[i + 2]).contains(c);
                        i += 3;
                    } else {
                        hit |= class[i] == *c;
                        i += 1;
                    }
                }
                hit != negated && matches(&p[close + 1..], &t[1..])
            }
            Some(c) => t.first() == Some(c) && matches(&p[1..], &t[1..]),
        }
    }
    let (p, t): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
    matches(&p, &t)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn case_patterns_match_like_the_shell() {
        assert!(glob("0.[0-8].*", "0.8.2"));
        assert!(!glob("0.[0-8].*", "0.10.0"));
        assert!(!glob("0.[0-8].*", "0.9.1"));
        assert!(glob("*:1.3.[2-9]*", "stable:1.3.2"));
        assert!(!glob("*:1.3.[2-9]*", "stable:1.3.1"));
        assert!(glob("tip:*", "tip:1.2.0"));
        assert!(glob("*:[2-9].*", "stable:2.0.0"));
    }

    #[test]
    fn the_newest_version_by_its_numbers() {
        assert_eq!(newest("0.18.0", "0.9.3"), "0.18.0");
        assert_eq!(newest("0.17.2", "0.18.0"), "0.18.0");
        assert_eq!(newest("1.0.0", "1.0.0"), "1.0.0");
    }

    #[test]
    fn permission_rule_names() {
        assert_eq!(rule_name("claude.ai Linear"), "claude_ai_Linear");
        assert_eq!(rule_name("my  server!"), "my_server");
        assert_eq!(rule_name("a__b-c"), "a_b-c");
    }

    #[test]
    fn labels_lose_their_position() {
        assert_eq!(strip_index("[3] x  inbox"), "x  inbox");
        assert_eq!(strip_index("[] eng-4"), "eng-4");
        assert_eq!(strip_index("eng-4"), "eng-4");
    }

    #[test]
    fn the_toast_delivery_of_its_table() {
        assert_eq!(toast_delivery("[ui]\ndelivery = \"x\"\n[ui.toast]\ndelivery = \"herdr\"\n[keys]\ndelivery = \"y\"\n"), "herdr");
        assert_eq!(toast_delivery("[ui.toast]\nsound = true\n"), "");
    }

    #[test]
    fn the_latest_beat_wins_the_last_of_equals() {
        let (a, b, c) = (json!({"at": 1, "n": "a"}), json!({"at": 3, "n": "b"}), json!({"at": 3, "n": "c"}));
        assert_eq!(max_by_at(&[&a, &b, &c]).unwrap()["n"], "c");
        assert_eq!(max_by_at(&[]), None);
    }

    fn check(id: &str, status: &'static str, line: &str) -> Check {
        let (message, label) = line.split_once(" → ").map_or((line, None), |(m, l)| (m, Some(l)));
        Check { id: id.into(), status, message: message.into(), fix: label.map(|l| json!({"label": l})) }
    }

    #[test]
    fn getting_started_is_the_checks_that_say_each_step_is_done() {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let checks = [
            check("tools.config", "pass", "config.json"),
            check("agents.default", "pass", "default agent Claude Code"),
            check("claude.plugin", "fail", "Claude plugin not installed → /plugin install cockpit"),
            check("tools.gh", "fail", "gh not signed in → gh auth login"),
            check("notifications.allowed", "pass", "notifications allowed"),
        ];
        // The default agent is whatever the machine has; the step for its tools follows it.
        let steps = getting_started(&checks);
        let ids: Vec<&str> = steps.iter().map(|s| s["id"].as_str().unwrap()).collect();
        assert!(ids.starts_with(&["setup", "agent"]) && ids.ends_with(&["github", "notifications"]), "{ids:?}");
        let github = steps.iter().find(|s| s["id"] == "github").unwrap();
        assert_eq!((&github["done"], &github["check"], &github["fix"]["label"]), (&json!(false), &json!("tools.gh"), &json!("gh auth login")));
        assert!(!ids.contains(&"app") && !ids.contains(&"tickets"), "a step whose check did not run is left out");
        let mut through_code = checks.to_vec();
        through_code.push(check("sources.server.code", "pass", "code from \"GitLab\" through cockpit's own connection"));
        let github = getting_started(&through_code).into_iter().find(|s| s["id"] == "github").unwrap();
        assert_eq!((&github["done"], &github["check"]), (&json!(true), &json!("sources.server.code")), "a code server reads GitHub without gh");
        // Every step names a check the hub's `doctor fix` can find.
        checks_reset();
        tickets(&json!({}), false, 1_000);
        let mut with_tickets = checks.to_vec();
        with_tickets.extend(taken());
        let tickets = getting_started(&with_tickets).into_iter().find(|s| s["id"] == "tickets").unwrap();
        assert_eq!((&tickets["done"], &tickets["check"], &tickets["fix"]["command"]), (&json!(false), &json!("sources.tickets"), &json!("cockpit setup")));
    }

    /// The checks the global list holds, emptied.
    fn taken() -> Vec<Check> {
        std::mem::take(&mut *checks())
    }

    fn checks_reset() {
        taken();
    }

    #[test]
    fn tickets_are_connected_once_they_come_in_not_once_a_token_is_set() {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let now = 100 * 60_000;
        let said = |inbox: Value, hub_reads: bool| {
            checks_reset();
            tickets(&inbox, hub_reads, now);
            let c = taken().pop().unwrap();
            (c.status, c.message, c.fix.map(|f| f["act"]["action"].as_str().or(f["command"].as_str()).unwrap_or_default().to_string()))
        };
        let record = |r: Value| json!({"hub": {"fresh": {"tickets": r}}});
        assert_eq!(said(json!({}), true), ("warn", "tickets: not read yet".into(), Some("refresh".into())), "a token alone is not tickets");
        assert_eq!(said(record(json!({"at": now - 60_000, "every": 60_000})), true).0, "pass");
        let failing = record(json!({"at": now - 60_000, "every": 60_000, "error": {"kind": "auth", "message": "HTTP 401: token refused", "since": now}}));
        assert_eq!(said(failing, true), ("fail", "tickets: HTTP 401: token refused".into(), Some("sign_in".into())));
        assert_eq!(said(record(json!({"at": now - 60 * 60_000, "every": 60_000})), true).0, "warn", "stale");
        assert_eq!(said(json!({"modFetched": {"tickets": now - 5 * 60_000}}), false).0, "pass", "a session brought them");
        assert_eq!(said(json!({"modFetched": {"tickets": now - 50 * 60_000}}), false).0, "warn");
    }

    /// The checks list is global: tests that fill it run one at a time.
    static SERIAL: Mutex<()> = Mutex::new(());

    #[test]
    fn a_document_stamped_before_any_data_says_its_failure_not_an_age() {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let doctor = Doctor {
            root: dir.path().into(),
            link: dir.path().into(),
            registered: String::new(),
            cache: dir.path().into(),
            settings: None,
            settings_path: dir.path().join("config.json"),
            config: None,
            herdr: "herdr".into(),
            path: String::new(),
            now: 1_794_000_000,
            has_herdr: false,
            app: None,
        };
        let said = |doc: Value| {
            files::write(&dir.path().join("monitor.json"), &doc).unwrap();
            checks_reset();
            doctor.check_cache("monitor.json", "monitor", 900, SESSION);
            let c = taken().pop().unwrap();
            (c.status, c.message)
        };
        let stamped = json!({"hub": {"fresh": {"monitor": {"source": "monitor", "error": {"kind": "network", "message": "platform: connection refused"}}}}});
        assert_eq!(said(stamped), ("fail", "monitor: platform: connection refused".to_string()));
        assert_eq!(said(json!({"hub": {"fresh": {"monitor": {"source": "monitor"}}}})), ("warn", "monitor: no data yet".to_string()));
        assert_eq!(said(json!({"at": 1_794_000_000_000_u64 - 120_000})).0, "pass");
    }

    #[test]
    fn a_hub_stopped_on_purpose_is_a_warning_with_its_start_as_the_fix() {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let doctor = Doctor {
            root: dir.path().into(),
            link: dir.path().into(),
            registered: String::new(),
            cache: dir.path().into(),
            settings: None,
            settings_path: dir.path().join("config.json"),
            config: None,
            herdr: "herdr".into(),
            path: String::new(),
            now: 1_794_000_000,
            has_herdr: false,
            app: None,
        };
        std::fs::write(dir.path().join("hub.stopped"), "").unwrap();
        checks_reset();
        doctor.hub(Path::new("/nowhere/cockpit"));
        let c = taken().pop().unwrap();
        assert_eq!(
            c.json(),
            json!({"id": "tools.hub", "category": "tools", "status": "warn", "message": "the hub was stopped: run cockpit hub start",
                   "fix": {"label": "cockpit hub start", "command": "cockpit hub start"}})
        );
    }

    #[test]
    fn a_kept_report_answers_while_it_is_recent_and_of_this_version() {
        let report = |at: u64| json!({"version": JSON_VERSION, "cockpit": env!("CARGO_PKG_VERSION"), "at": at});
        let now = 1_000_000_000;
        assert!(is_recent(&report(now - 60_000), now, 0));
        assert!(!is_recent(&report(now - CACHED_FOR.as_millis() as u64), now, 0));
        assert!(!is_recent(&json!({"version": JSON_VERSION, "cockpit": "0.0.1", "at": now}), now, 0), "another cockpit's report runs again");
        assert!(!is_recent(&report(now + 60_000), now, 0), "a report from the future");
        assert!(!is_recent(&report(now - 60_000), now, now - 30_000), "the config changed since");
        let mut fixed = report(now - 60_000);
        fixed["stale"] = json!(true);
        assert!(!is_recent(&fixed, now, 0), "a fix ran since");
    }

    #[test]
    fn a_check_says_its_fix_apart_from_what_it_found() {
        let c = check("tools.gh", "fail", "gh not signed in → gh auth login");
        assert_eq!(
            c.json(),
            json!({"id": "tools.gh", "category": "tools", "status": "fail", "message": "gh not signed in", "fix": {"label": "gh auth login"}})
        );
        assert_eq!(quote("claude.ai Linear"), "'claude.ai Linear'");
        assert_eq!(quote("Linear"), "Linear");
    }

    #[test]
    fn unmapped_tools() {
        let tools = json!({"listMine": {"tool": "x"}, "get": {"tool": ""}, "listStates": {}});
        assert_eq!(unmapped(&tools, &["listMine", "get", "listStates", "setState"]), "get, listStates, setState");
        assert_eq!(unmapped(&Value::Null, &["a"]), "");
    }
}
