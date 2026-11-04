//! The routines the hub runs on its own (see cockpit_core::routines): every few seconds it reads their files, starts the
//! ones due on their schedule or set off by the week's events, three at a time, and records each run's end. Everything
//! lives in the `routines` document, so a run started by one hub is finished by the next one after an update.
//!
//! A run is `sh -c '"$@"; echo $? > <id>.exit'` around the routine's command, in its own process group, its output in
//! ~/.cache/cockpit/routines/<slug>/<id>.{out,err} and what the app shows in <id>.txt.

use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use chrono::{Local, NaiveDateTime, TimeZone};
use cockpit_core::agent::{self, Agent, Headless};
use cockpit_core::files;
use cockpit_core::routines::{self, Notify, Routine, Run, When};
use serde_json::{Map, Value, json};

use crate::store::Store;
use crate::value::{cut, list};

pub const DOC: &str = "routines";
const TICK: Duration = Duration::from_secs(5);
/// A scheduled run this late still counts as on time.
const ON_TIME_MS: u64 = 120_000;
const KEEP_RUNS: usize = 20;
const AT_ONCE: usize = 3;
/// One routine runs once per row an event touches in this long: a PR's CI failing and its run failing are one failure.
const EVENT_GAP_MS: u64 = 10 * 60_000;
/// A run stopped that still lives this long after is killed.
const KILL_AFTER_MS: u64 = 10_000;
const PROMPT_SHOWN: usize = 600;
const WEEK_MS: u64 = 7 * 86_400_000;
/// The lightest routines' whole system prompt, in place of Claude Code's: its MCP tools load on demand, and may still
/// be connecting when the run starts.
pub fn light_prompt(r: &Routine) -> String {
    let mcp: Vec<&str> = r.tools.iter().filter(|t| t.starts_with("mcp__")).map(String::as_str).collect();
    let load = if mcp.is_empty() {
        String::new()
    } else {
        format!(
            "Its tools load on demand: first call ToolSearch with query \"select:{}\", then call them as the task says; when one \
             is not found yet, its server is still connecting: call ToolSearch again. Never answer that you lack a tool before \
             that. ",
            mcp.join(",")
        )
    };
    format!("You run one unattended task for cockpit, with only the tools it names. {load}Reply in one short line saying what you did.")
}

/// What a routine's file lacks to do its job unattended: an agent that runs headless, and for Claude Code permissions
/// for what needs approval, and for a lightest one its tools and servers that have them.
fn problems(r: &Routine, agent: Option<&Agent>, servers: &[String]) -> Vec<String> {
    let Some(agent) = agent.filter(|_| r.run == Run::Agent) else {
        return if r.run == Run::Agent { vec![format!("runs agent {:?}, which cockpit does not know", r.agent)] } else { Vec::new() };
    };
    if !agent.can_run() {
        return vec![format!("{} has no headless mode: it cannot run a routine", agent.name)];
    }
    if agent.id != "claude" {
        let claude_only = r.is_lightest() || !r.tools.is_empty() || r.budget.is_some();
        return if claude_only {
            vec![format!("its tools, budget and lightest model are Claude Code's: {} runs it without them", agent.name)]
        } else {
            Vec::new()
        };
    }
    if !r.is_lightest() {
        let unattended = r.tools.is_empty() && r.mode.is_empty();
        return if unattended { vec!["lists no tools and no mode: nobody approves its gh, git or file calls, so they are refused".into()] } else { Vec::new() };
    }
    if r.tools.is_empty() {
        return vec!["lists no tools: a lightest run loads nothing else, so it can call nothing".into()];
    }
    let prefixes: Vec<String> = servers.iter().map(|s| crate::config::tool_prefix(s)).collect();
    r.tools
        .iter()
        .filter(|t| t.starts_with("mcp__") && !servers.is_empty())
        .filter(|t| !prefixes.iter().any(|p| t.starts_with(&format!("{p}__"))))
        .map(|t| format!("{t} belongs to no MCP server Claude Code connects"))
        .collect()
}

/// Whether a tool a run called is one the routine lists: by full name for MCP tools, by name for built-in ones.
fn lists(r: &Routine, called: &str) -> bool {
    r.tools.iter().any(|t| t == called || t.split('(').next().is_some_and(|name| name.trim() == called))
}

/// Where routines are read, where their runs go, and the agents that run them.
pub struct Setup {
    pub routines: PathBuf,
    pub runs: PathBuf,
    /// The agents' table, and the id of the one a routine naming none runs.
    pub agents: Vec<Agent>,
    pub default_agent: String,
    pub repos: String,
    /// Where runs' worktrees go: ~/.herdr/worktrees, as the Herdr apps put theirs.
    pub worktrees: PathBuf,
}

impl Setup {
    pub fn live() -> Setup {
        let worktrees = files::home().join(".herdr/worktrees");
        let (agents, default_agent) = (agent::all(), agent::default().id);
        Setup { routines: routines::dir(), runs: files::cache().join("routines"), agents, default_agent, repos: crate::config::load().repos(), worktrees }
    }

    /// The agent a routine runs: its own when it names one, else the default; None for a script or an unknown agent.
    pub fn agent_of(&self, r: &Routine) -> Option<&Agent> {
        let id = if r.agent.is_empty() { &self.default_agent } else { &r.agent };
        (r.run == Run::Agent).then(|| self.agents.iter().find(|a| &a.id == id || a.aliases.contains(id))).flatten()
    }

    fn run_file(&self, slug: &str, id: &str, ext: &str) -> PathBuf {
        self.runs.join(slug).join(format!("{id}.{ext}"))
    }
}

/// What a run's end tells: a notification, by title, body, click and buttons.
pub type Notifier<'a> = &'a mut dyn FnMut(&str, &str, &str, &[String]);

static WAKE: OnceLock<Mutex<Sender<()>>> = OnceLock::new();

fn wake_channel() -> Option<Receiver<()>> {
    let (tx, rx) = channel();
    WAKE.set(Mutex::new(tx)).ok().map(|_| rx)
}

/// A tick now rather than at the next beat: a routine asked to run, stopped or changed.
pub fn poke() {
    if let Some(tx) = WAKE.get().and_then(|m| m.lock().ok()) {
        let _ = tx.send(());
    }
}

/// Ticks for as long as the hub runs.
pub fn keep() {
    let Some(woken) = wake_channel() else { return };
    loop {
        tick();
        let _ = woken.recv_timeout(TICK);
        while woken.try_recv().is_ok() {}
    }
}

/// One round on the real files, store and clock.
pub fn tick() {
    round(&mut crate::store::global(), &Setup::live(), now_ms(), &mut post);
}

/// A run's end as a system notification.
pub fn post(title: &str, body: &str, action: &str, buttons: &[String]) {
    cockpit_core::notify::post(title, body, action, buttons);
}

/// Whether the hub has a routine to run on its own, which keeps it from exiting when idle.
pub fn wanted() -> bool {
    routines::read_all(&routines::dir()).iter().any(|r| !r.paused && matches!(r.when, Ok(When::Every(_) | When::Cron(_) | When::On { .. })))
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn local(ms: u64) -> NaiveDateTime {
    Local.timestamp_millis_opt(ms as i64).single().map(|t| t.naive_local()).unwrap_or_default()
}

fn epoch_ms(at: NaiveDateTime) -> u64 {
    Local.from_local_datetime(&at).earliest().map(|t| t.timestamp_millis() as u64).unwrap_or(0)
}

/// The document as it is, or a new one whose events start now.
fn current(store: &Store, now: u64) -> Value {
    let doc = store.get(DOC);
    if doc["version"] == 1 {
        return doc;
    }
    json!({"version": 1, "cursor": now, "inboxFrom": now, "routines": [], "running": {}, "queued": [], "runs": {}, "marks": {}})
}

/// Writes the document when it says something new; `at` alone changing writes nothing.
fn save(store: &mut Store, mut doc: Value, now: u64) {
    store.update(DOC, |before| {
        doc["at"] = before["at"].clone();
        if *before == doc {
            return before.clone();
        }
        doc["at"] = json!(now);
        doc
    });
}

/// Finishes the runs that ended, queues what is due and what the events set off, starts what the queue allows, and
/// writes the document.
pub fn round(store: &mut Store, setup: &Setup, now: u64, notify: Notifier) {
    let all = routines::read_all(&setup.routines);
    if all.is_empty() && store.get(DOC).is_null() {
        return;
    }
    let mut doc = current(store, now);
    if doc["inboxFrom"].is_null() {
        doc["inboxFrom"] = json!(now);
    }
    finish(&mut doc, setup, &all, now, notify);
    for r in &all {
        let marks = &mut doc["marks"][&r.slug];
        if marks["seen"].is_null() {
            *marks = json!({"seen": now, "fired": 0});
        }
    }
    for (r, due) in schedule(&mut doc, &all, now) {
        let run = missed(due);
        tell(r, &run, notify);
        record(&mut doc, setup, &r.slug, run);
    }
    let cursor = doc["cursor"].as_u64().unwrap_or(now);
    let mut events = store.events(cursor + 1, 500);
    events.reverse();
    listen(&mut doc, &all, &events, now);
    start_queued(&mut doc, setup, &all, now, notify);
    let servers = known_servers(setup);
    doc["routines"] = Value::Array(all.iter().map(|r| shown(r, setup, &doc, now, &servers)).collect());
    let installed: Vec<Value> = setup.agents.iter().filter(|a| a.can_run() && a.installed()).map(|a| json!({"id": a.id, "name": a.name})).collect();
    doc["agents"] = json!({"default": setup.default_agent, "installed": installed});
    save(store, doc, now);
}

fn mark(doc: &Value, slug: &str, field: &str) -> u64 {
    doc["marks"][slug][field].as_u64().unwrap_or(0)
}

fn is_busy(doc: &Value, slug: &str) -> bool {
    !doc["running"][slug].is_null() || list(&doc["queued"]).iter().any(|q| q["slug"] == slug)
}

fn queue(doc: &mut Value, slug: &str, why: String, event: Value) {
    if let Some(queued) = doc["queued"].as_array_mut() {
        queued.push(json!({"slug": slug, "why": why, "event": event}));
    }
}

/// The record of a scheduled run that did not happen: the hub was down at its time, past its catch-up window.
fn missed(due: NaiveDateTime) -> Value {
    let at = epoch_ms(due);
    let summary = format!("missed {}: cockpit was not running then", due.format("%a %H:%M"));
    json!({"id": at.to_string(), "started": at, "ended": at, "ok": false, "missed": true, "code": null, "why": format!("scheduled for {}", due.format("%H:%M")), "summary": summary})
}

/// Queues the routines whose time came: an interval once it passed since the last run, a schedule at its time, or late
/// when the hub was down then, within the routine's catch-up window. The scheduled runs missed past it come back, with
/// their time, to be recorded as missed.
fn schedule<'a>(doc: &mut Value, all: &'a [Routine], now: u64) -> Vec<(&'a Routine, NaiveDateTime)> {
    let mut gone = Vec::new();
    for r in all.iter().filter(|r| !r.paused) {
        let base = mark(doc, &r.slug, "seen").max(mark(doc, &r.slug, "fired"));
        match &r.when {
            Ok(When::Every(secs)) if now >= base + secs * 1000 => {
                doc["marks"][&r.slug]["fired"] = json!(now);
                if !is_busy(doc, &r.slug) {
                    queue(doc, &r.slug, r.when_shown(), Value::Null);
                }
            }
            Ok(when @ When::Cron(_)) => {
                let Some(due) = when.last_due(local(base), local(now)) else { continue };
                let due_ms = epoch_ms(due);
                doc["marks"][&r.slug]["fired"] = json!(due_ms.max(base));
                let late = now.saturating_sub(due_ms);
                if is_busy(doc, &r.slug) {
                    continue;
                }
                if late <= (r.catch_up * 60_000).max(ON_TIME_MS) {
                    let words = if late > ON_TIME_MS { ", run late" } else { "" };
                    queue(doc, &r.slug, format!("scheduled for {}{words}", due.format("%H:%M")), Value::Null);
                    if late > ON_TIME_MS
                        && let Some(last) = doc["queued"].as_array_mut().and_then(|q| q.last_mut())
                    {
                        last["late"] = json!(true);
                    }
                } else {
                    gone.push((r, due));
                }
            }
            _ => {}
        }
    }
    gone
}

/// Queues the routines the events set off, oldest event first, and moves the cursor past them; the routines' own
/// events set off nothing.
fn listen(doc: &mut Value, all: &[Routine], events: &[Value], now: u64) {
    for event in events.iter().filter(|e| e["doc"] != DOC) {
        let key = event["key"].as_str().unwrap_or_default();
        for r in all.iter().filter(|r| !r.paused && r.when.as_ref().is_ok_and(|w| w.is_set_off_by(event))) {
            let last = doc["marks"][&r.slug]["events"][key].as_u64().unwrap_or(0);
            if now.saturating_sub(last) < EVENT_GAP_MS || is_busy(doc, &r.slug) {
                continue;
            }
            doc["marks"][&r.slug]["events"][key] = json!(now);
            queue(doc, &r.slug, event["title"].as_str().unwrap_or("an event").to_string(), event.clone());
        }
    }
    let cursor = doc["cursor"].as_u64().unwrap_or(now);
    doc["cursor"] = json!(events.iter().filter_map(|e| e["at"].as_u64()).fold(cursor, u64::max));
    // Rows set off long ago are forgotten.
    for marks in doc["marks"].as_object_mut().into_iter().flat_map(|m| m.values_mut()) {
        if let Some(seen) = marks["events"].as_object_mut() {
            seen.retain(|_, at| now.saturating_sub(at.as_u64().unwrap_or(0)) < EVENT_GAP_MS);
        }
    }
}

/// What a routine's runs cost over the last seven days, as Claude estimated them.
fn spent(doc: &Value, slug: &str, now: u64) -> f64 {
    list(&doc["runs"][slug]).iter().filter(|r| now.saturating_sub(r["started"].as_u64().unwrap_or(0)) < WEEK_MS).filter_map(|r| r["cost"].as_f64()).sum()
}

/// A run held back by its weekly budget: it is dropped, and the first one of a stretch says so.
fn over_budget(doc: &mut Value, r: &Routine, left: f64, notify: Notifier) {
    if doc["marks"][&r.slug]["over"] == true {
        return;
    }
    doc["marks"][&r.slug]["over"] = json!(true);
    let budget = r.budget.unwrap_or_default();
    let body = format!("its ${budget:.2} for the week is spent (${:.2}): it waits until older runs fall out of the week", budget - left);
    notify(&format!("⏸ {}", r.name), &body, &format!("routine:{}", r.slug), &[format!("Open=routine:{}", r.slug)]);
}

/// Starts queued runs, oldest first, while fewer than three run; one whose routine is gone or broken is dropped, and
/// so is one over its weekly budget.
fn start_queued(doc: &mut Value, setup: &Setup, all: &[Routine], now: u64, notify: Notifier) {
    let queued: Vec<Value> = list(&doc["queued"]).to_vec();
    let mut left = Vec::new();
    for q in queued {
        let slug = q["slug"].as_str().unwrap_or_default();
        let Some(r) = all.iter().find(|r| r.slug == slug && r.when.is_ok()) else { continue };
        let running = doc["running"].as_object().map_or(0, Map::len);
        if running >= AT_ONCE || !doc["running"][slug].is_null() {
            left.push(q);
            continue;
        }
        let why = q["why"].as_str().unwrap_or_default();
        let left_this_week = r.budget.filter(|_| is_claude(setup, r)).map(|b| b - spent(doc, slug, now));
        if let Some(left) = left_this_week.filter(|l| *l < 0.01) {
            over_budget(doc, r, left, notify);
            continue;
        }
        doc["marks"][slug]["over"] = json!(false);
        match start(setup, r, why, &q["event"], now, left_this_week) {
            Ok(mut run) => {
                run["retry"] = q["retry"].clone();
                run["late"] = q["late"].clone();
                doc["running"][slug] = run;
            }
            Err(err) => record(doc, setup, slug, failed_to_start(now, why, &err)),
        }
    }
    doc["queued"] = Value::Array(left);
}

fn failed_to_start(now: u64, why: &str, err: &str) -> Value {
    json!({"id": now.to_string(), "started": now, "ended": now, "ok": false, "code": null, "why": why, "summary": format!("did not start: {err}")})
}

fn is_claude(setup: &Setup, r: &Routine) -> bool {
    setup.agent_of(r).is_some_and(|a| a.id == "claude")
}

/// A random version 4 UUID, for the session a Claude run is asked to use.
fn uuid() -> String {
    let mut bytes = [0u8; 16];
    if let Ok(mut random) = std::fs::File::open("/dev/urandom") {
        let _ = std::io::Read::read_exact(&mut random, &mut bytes);
    } else {
        bytes = (now_ms() as u128 ^ (std::process::id() as u128) << 64).to_le_bytes();
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..])
}

/// The prompt with what the run needs to know: who runs it, why and when, and that nobody answers questions.
pub fn prompt(r: &Routine, why: &str, event: &Value, now: u64) -> String {
    let mut out = format!(
        "{}\n\n---\ncockpit runs this as the routine \"{}\" ({}): {why}, at {}. Nobody is watching: do what it asks without \
         waiting for answers, then reply with what you did or found, its first line a one-line summary.",
        r.body.trim(),
        r.name,
        r.when_shown(),
        local(now).format("%Y-%m-%d %H:%M")
    );
    if let Some(title) = event["title"].as_str() {
        out.push_str(&format!("\nWhat set it off: {title} ({}).", event["key"].as_str().unwrap_or_default()));
    }
    out
}

/// The headless command of another agent: its model when the routine names one it takes, and file edits when the
/// routine gives a permission mode that allows them.
pub fn agent_argv(agent: &Agent, r: &Routine, prompt: &str) -> Result<Vec<String>, String> {
    let edits = matches!(r.mode.as_str(), "acceptEdits" | "auto" | "bypassPermissions" | "dontAsk");
    let model = Some(r.model.as_str()).filter(|m| !m.is_empty() && !r.is_lightest());
    agent
        .headless(prompt, &Headless { model, edits, ..Default::default() })
        .ok_or_else(|| format!("{} has no headless mode: it cannot run a routine", agent.name))
}

/// The MCP servers Claude Code connected in the routines' runs, as their init events listed them: the lightest runs
/// connect only their own, so the list only grows.
fn known_servers(setup: &Setup) -> Vec<String> {
    files::load(&setup.runs.join("mcp-servers.json"))
        .map(|v| list(&v["servers"]).iter().filter_map(|s| s.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

/// The headless Claude command for a routine, `cap` the dollars it may still spend this week. A routine that lists its
/// tools gets those built-in tools alone (and ToolSearch, which loads the MCP ones it names), and skills only when it
/// lists Skill: each run then reads a fraction of the tool and skill definitions. The lightest one runs haiku at low
/// effort, on a one-line system prompt, with every other MCP server of `servers` denied, so neither their tools nor
/// their instructions are read.
pub fn claude_argv(claude: &str, r: &Routine, prompt: String, session: &str, cap: Option<f64>, servers: &[String]) -> Vec<String> {
    let mut argv: Vec<String> =
        vec![claude.into(), "-p".into(), prompt, "--output-format".into(), "stream-json".into(), "--verbose".into(), "--session-id".into(), session.into()];
    argv.extend(["--name".into(), format!("routine: {}", r.name)]);
    if r.is_lightest() {
        argv.extend(["--model".into(), "haiku".into(), "--effort".into(), "low".into()]);
    } else if !r.model.is_empty() {
        argv.extend(["--model".into(), r.model.clone()]);
    }
    if !r.mode.is_empty() {
        argv.extend(["--permission-mode".into(), r.mode.clone()]);
    }
    if let Some(cap) = cap {
        argv.extend(["--max-budget-usd".into(), format!("{cap:.2}")]);
    }
    if !r.tools.is_empty() || r.is_lightest() {
        let mut builtin = vec!["ToolSearch".to_string()];
        for tool in r.tools.iter().filter(|t| !t.starts_with("mcp__")) {
            let name = tool.split('(').next().unwrap_or(tool).trim().to_string();
            if !builtin.contains(&name) {
                builtin.push(name);
            }
        }
        if !builtin.iter().any(|t| t == "Skill") {
            argv.push("--disable-slash-commands".into());
        }
        argv.extend(["--tools".into(), builtin.join(",")]);
        if !r.tools.is_empty() {
            argv.push("--allowedTools".into());
            argv.extend(r.tools.iter().cloned());
        }
    }
    if r.is_lightest() {
        let unused: Vec<Value> = servers
            .iter()
            .filter(|s| {
                let prefix = crate::config::tool_prefix(s);
                !r.tools.iter().any(|t| *t == prefix || t.starts_with(&format!("{prefix}__")))
            })
            .map(|s| json!({"serverName": s}))
            .collect();
        if !unused.is_empty() {
            argv.extend(["--settings".into(), json!({"deniedMcpServers": unused}).to_string()]);
        }
        argv.extend(["--append-system-prompt".into(), light_prompt(r)]);
    }
    argv
}

/// Starts a run in its own process group: its entry in `running`.
fn start(setup: &Setup, r: &Routine, why: &str, event: &Value, now: u64, cap: Option<f64>) -> Result<Value, String> {
    let id = now.to_string();
    let dir = setup.runs.join(&r.slug);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let agent = setup.agent_of(r);
    let is_claude = agent.is_some_and(|a| a.id == "claude");
    let session = if is_claude { uuid() } else { String::new() };
    let command = match agent {
        Some(a) if is_claude => claude_argv(&a.path(), r, prompt(r, why, event, now), &session, cap, &known_servers(setup)),
        Some(a) => agent_argv(a, r, &prompt(r, why, event, now))?,
        None if r.run == Run::Agent => return Err(format!("no agent {:?}", r.agent)),
        None => vec!["sh".into(), "-c".into(), r.body.clone()],
    };
    // An agent's run without a folder gets its own, where no project's instructions and memory are read.
    let cwd = match (r.cwd.is_empty(), r.run) {
        (false, _) => r.cwd.clone(),
        (true, Run::Agent) => dir.to_string_lossy().into_owned(),
        (true, Run::Shell) => setup.repos.clone(),
    };
    if !Path::new(&cwd).is_dir() {
        return Err(format!("no folder {cwd}"));
    }
    let worktree = if r.worktree { Some(worktree(&setup.worktrees, &cwd, &r.slug, &id)?) } else { None };
    let cwd = worktree.as_ref().and_then(|w| w["cwd"].as_str()).map_or(cwd.clone(), str::to_string);
    let exit = setup.run_file(&r.slug, &id, "exit");
    let mut argv = vec!["sh".to_string(), "-c".into(), "\"$@\"; echo \"$?\" > \"$0\"".into(), exit.to_string_lossy().into_owned()];
    argv.extend(command);
    // The command as run, to replay a run that went wrong.
    let _ = files::write(&setup.run_file(&r.slug, &id, "cmd"), &json!({"argv": argv, "cwd": cwd}));
    let out = std::fs::File::create(setup.run_file(&r.slug, &id, "out")).map_err(|e| e.to_string())?;
    let err = std::fs::File::create(setup.run_file(&r.slug, &id, "err")).map_err(|e| e.to_string())?;
    let mut child = Command::new(&argv[0]);
    child.args(&argv[1..]).current_dir(&cwd).stdin(Stdio::null()).stdout(out).stderr(err).process_group(0);
    // A hub started from inside a Claude session must not make the run think it is nested in it.
    for var in ["CLAUDECODE", "CLAUDE_CODE_ENTRYPOINT", "CLAUDE_CODE_SSE_PORT"] {
        child.env_remove(var);
    }
    let path = std::env::var("PATH").unwrap_or_default();
    let home = files::home().to_string_lossy().into_owned();
    child.env("PATH", format!("{path}:/opt/homebrew/bin:/usr/local/bin:{home}/.local/bin:{home}/.claude/local"));
    child.env("COCKPIT_ROUTINE", &r.slug).env("COCKPIT_ROUTINE_WHY", why);
    for (var, field) in [("COCKPIT_EVENT", "title"), ("COCKPIT_EVENT_KEY", "key"), ("COCKPIT_EVENT_KIND", "kind")] {
        child.env(var, event[field].as_str().unwrap_or_default());
    }
    let mut child = child.spawn().map_err(|e| e.to_string())?;
    let pid = child.id();
    // Reaped as soon as it ends; its end is read from its exit file.
    std::thread::spawn(move || child.wait());
    let agent = agent.map(|a| a.id.clone());
    Ok(
        json!({"id": id, "pid": pid, "started": now, "why": why, "session": session, "agent": agent, "event": event, "timeout": r.timeout, "worktree": worktree}),
    )
}

/// git in a folder, never taking its index lock: its output, or what it said on failing.
fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out =
        Command::new("git").arg("-C").arg(dir).args(["--no-optional-locks", "-c", "core.fsmonitor=false"]).args(args).output().map_err(|e| e.to_string())?;
    let text = |b: &[u8]| String::from_utf8_lossy(b).trim().to_string();
    if out.status.success() { Ok(text(&out.stdout)) } else { Err(text(&out.stderr)) }
}

/// A git worktree of the folder's repository for one run, on a new branch `auto/<slug>-<id>`, at
/// `<root>/<repo>/<branch>`: {path, cwd (the folder's place in it), branch, repo, base}.
fn worktree(root: &Path, cwd: &str, slug: &str, id: &str) -> Result<Value, String> {
    let folder = std::fs::canonicalize(cwd).map_err(|e| e.to_string())?;
    let top = git(&folder, &["rev-parse", "--show-toplevel"]).map_err(|_| format!("worktree: {cwd} is in no git repository"))?;
    let top = PathBuf::from(top);
    let name = top.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let branch = format!("auto/{slug}-{id}");
    let path = root.join(&name).join(&branch);
    let base = git(&top, &["rev-parse", "HEAD"]).map_err(|e| format!("worktree: {e}"))?;
    git(&top, &["worktree", "add", "-q", "-b", &branch, &path.to_string_lossy(), "HEAD"]).map_err(|e| format!("worktree: {e}"))?;
    let inside = folder.strip_prefix(std::fs::canonicalize(&top).unwrap_or(top.clone())).map(|rel| path.join(rel)).unwrap_or(path.clone());
    Ok(json!({"path": path, "cwd": inside, "branch": branch, "repo": top, "base": base}))
}

/// What a run left in its worktree: commits past where it started and changes not committed. One that left nothing
/// is removed with its branch at once.
pub fn settle(wt: &Value) -> Value {
    let mut wt = wt.clone();
    let path = PathBuf::from(wt["path"].as_str().unwrap_or_default());
    if !path.is_dir() {
        wt["removed"] = json!(true);
        return wt;
    }
    let dirty = git(&path, &["status", "--porcelain"]).map(|s| !s.is_empty()).unwrap_or(true);
    let commits: u64 =
        git(&path, &["rev-list", "--count", &format!("{}..HEAD", wt["base"].as_str().unwrap_or("HEAD"))]).ok().and_then(|n| n.parse().ok()).unwrap_or(0);
    wt["dirty"] = json!(dirty);
    wt["commits"] = json!(commits);
    if !dirty && commits == 0 && remove_worktree(&wt).is_ok() {
        wt["removed"] = json!(true);
    }
    wt
}

/// Whether a kept run worktree's work landed: nothing uncommitted and its branch inside the repository's default one.
pub fn landed(wt: &Value) -> bool {
    let (path, repo) = (PathBuf::from(wt["path"].as_str().unwrap_or_default()), PathBuf::from(wt["repo"].as_str().unwrap_or_default()));
    if !path.is_dir() {
        return true;
    }
    if git(&path, &["status", "--porcelain"]).map_or(true, |s| !s.is_empty()) {
        return false;
    }
    let default = git(&repo, &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"]).ok().filter(|d| !d.is_empty());
    let branch = wt["branch"].as_str().unwrap_or_default();
    let into: Vec<String> = default.into_iter().chain(["main".to_string(), "master".to_string()]).collect();
    into.iter().any(|d| git(&repo, &["merge-base", "--is-ancestor", branch, d]).is_ok())
}

/// The worktree and its branch removed as git allows: never forced, so uncommitted work or unmerged commits stay.
pub fn remove_worktree(wt: &Value) -> Result<(), String> {
    let (path, repo) = (wt["path"].as_str().unwrap_or_default(), PathBuf::from(wt["repo"].as_str().unwrap_or_default()));
    if Path::new(path).is_dir() {
        git(&repo, &["worktree", "remove", path])?;
    }
    let branch = wt["branch"].as_str().unwrap_or_default();
    let base = wt["base"].as_str().unwrap_or_default();
    // A branch with no commits of its own is deleted as it is; one with commits only once merged.
    let own: u64 = git(&repo, &["rev-list", "--count", &format!("{base}..{branch}")]).ok().and_then(|n| n.parse().ok()).unwrap_or(1);
    if own == 0 { git(&repo, &["branch", "-D", branch]).map(|_| ()) } else { git(&repo, &["branch", "-d", branch]).map(|_| ()) }
}

fn alive(pid: u64) -> bool {
    // SAFETY: signal 0 only asks whether the process exists.
    pid > 0 && unsafe { libc::kill(pid as i32, 0) } == 0
}

/// Sends the run's process group a signal.
fn signal(pid: u64, sig: i32) {
    if pid > 0 {
        // SAFETY: the group is the run's own, started with process_group(0).
        unsafe { libc::kill(-(pid as i32), sig) };
    }
}

/// Asks a running routine to stop: the run ends as stopped.
pub fn stop(doc: &mut Value, slug: &str, now: u64) -> bool {
    let running = &mut doc["running"][slug];
    let Some(pid) = running["pid"].as_u64() else { return false };
    running["stopping"] = json!(now);
    signal(pid, libc::SIGTERM);
    true
}

/// Records the runs that ended (their exit file written, or their process gone), stops the ones over their time and
/// kills the ones a stop did not end.
fn finish(doc: &mut Value, setup: &Setup, all: &[Routine], now: u64, notify: Notifier) {
    let running: Vec<(String, Value)> = doc["running"].as_object().map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect()).unwrap_or_default();
    for (slug, run) in running {
        let id = run["id"].as_str().unwrap_or_default();
        let pid = run["pid"].as_u64().unwrap_or(0);
        let code = std::fs::read_to_string(setup.run_file(&slug, id, "exit")).ok().and_then(|c| c.trim().parse::<i32>().ok());
        let started = run["started"].as_u64().unwrap_or(now);
        if code.is_none() && alive(pid) {
            let stopping = run["stopping"].as_u64();
            if stopping.is_none() && now > started + run["timeout"].as_u64().unwrap_or(30) * 60_000 {
                doc["running"][&slug]["timedOut"] = json!(true);
                stop(doc, &slug, now);
            } else if stopping.is_some_and(|at| now > at + KILL_AFTER_MS) {
                signal(pid, libc::SIGKILL);
            }
            continue;
        }
        let r = all.iter().find(|r| r.slug == slug);
        let ended = ended(setup, &slug, &run, code, r, now);
        if let Some(running) = doc["running"].as_object_mut() {
            running.remove(&slug);
        }
        // A lightest run that called none of its tools most likely started before their server connected: once more.
        let again = ended["missedTools"] == true && run["retry"] != true;
        if let Some(r) = r.filter(|_| !again) {
            tell(r, &ended, notify);
        }
        record(doc, setup, &slug, ended);
        if again {
            let why = format!("again: {}", run["why"].as_str().unwrap_or_default());
            queue(doc, &slug, why, run["event"].clone());
            if let Some(last) = doc["queued"].as_array_mut().and_then(|q| q.last_mut()) {
                last["retry"] = json!(true);
            }
        }
    }
}

/// The run's record from what it left: Claude Code's JSON answer, another agent's answer or the script's output, in
/// <id>.txt for the app. A lightest run that called none of the tools its routine lists failed, whatever it answered.
fn ended(setup: &Setup, slug: &str, run: &Value, code: Option<i32>, r: Option<&Routine>, now: u64) -> Value {
    // A run from before runs named their agent was Claude Code's when it had a session.
    let claude = run["agent"] == "claude" || (run["agent"].is_null() && run["session"].as_str().is_some_and(|s| !s.is_empty()));
    let id = run["id"].as_str().unwrap_or_default();
    let out = std::fs::read_to_string(setup.run_file(slug, id, "out")).unwrap_or_default();
    let err = std::fs::read_to_string(setup.run_file(slug, id, "err")).unwrap_or_default();
    let mut record = json!({
        "id": id, "started": run["started"], "ended": now, "code": code, "why": run["why"], "session": run["session"], "ok": code == Some(0),
    });
    if !run["agent"].is_null() {
        record["agent"] = run["agent"].clone();
    }
    // Another agent's session, as its headless run printed it.
    if !claude
        && !run["agent"].is_null()
        && let Some(session) = agent::session_in(&err)
    {
        record["session"] = json!(session);
    }
    if run["late"] == true {
        record["late"] = json!(true);
    }
    if !run["worktree"].is_null() {
        record["worktree"] = settle(&run["worktree"]);
    }
    let events: Vec<Value> = if claude { out.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).collect() } else { Vec::new() };
    let answer = events.iter().rev().find(|e| e["type"] == "result").cloned();
    let called: Vec<&str> = events
        .iter()
        .filter(|e| e["type"] == "assistant")
        .flat_map(|e| list(&e["message"]["content"]))
        .filter(|c| c["type"] == "tool_use")
        .filter_map(|c| c["name"].as_str())
        .filter(|name| *name != "ToolSearch")
        .collect();
    record["called"] = json!(called);
    let missed = r.is_some_and(|r| r.is_lightest() && !r.tools.is_empty() && !called.iter().any(|c| lists(r, c)));
    // The servers this run's Claude connected, which the lightest routines deny but for their own.
    if let Some(init) = events.iter().find(|e| e["type"] == "system" && e["subtype"] == "init") {
        let servers: Vec<&Value> = list(&init["mcp_servers"]).iter().map(|s| &s["name"]).filter(|n| n.is_string()).collect();
        if !servers.is_empty() {
            let mut known = known_servers(setup);
            known.extend(servers.iter().filter_map(|s| s.as_str()).map(str::to_string).filter(|s| !known.contains(s)).collect::<Vec<_>>());
            let _ = files::write(&setup.runs.join("mcp-servers.json"), &json!({"servers": known, "at": now}));
        }
    }
    let shown = match &answer {
        Some(answer) => {
            record["cost"] = answer["total_cost_usd"].clone();
            if let Some(session) = answer["session_id"].as_str() {
                record["session"] = json!(session);
            }
            if answer["is_error"] == true {
                record["ok"] = json!(false);
            }
            let result = answer["result"].as_str().unwrap_or_default().to_string();
            let stopped = match answer["subtype"].as_str().unwrap_or_default() {
                "error_max_budget_usd" => "stopped at its weekly budget".to_string(),
                "error_max_turns" => "stopped at its turn limit".to_string(),
                "error_during_execution" => "failed while running".to_string(),
                other => other.replace('_', " "),
            };
            if result.is_empty() { stopped } else { result }
        }
        None if out.trim().is_empty() => err.clone(),
        None => out.clone(),
    };
    let _ = std::fs::write(setup.run_file(slug, id, "txt"), if shown.trim().is_empty() { &err } else { &shown });
    let first = shown.lines().map(str::trim).find(|l| !l.is_empty()).map(|l| l.trim_start_matches(['#', ' ']).to_string());
    let last_err = err.lines().map(str::trim).rfind(|l| !l.is_empty()).map(str::to_string);
    let summary = if run["timedOut"] == true {
        format!("timed out after {} min", run["timeout"].as_u64().unwrap_or(30))
    } else if !run["stopping"].is_null() {
        record["stopped"] = json!(true);
        "stopped".into()
    } else if missed && answer.is_some() {
        record["ok"] = json!(false);
        record["missedTools"] = json!(true);
        format!("called none of its tools: {}", first.unwrap_or_default())
    } else if record["ok"] == true {
        first.unwrap_or_else(|| "done".into())
    } else {
        first.or(last_err).unwrap_or_else(|| code.map_or("ended with no exit status".into(), |c| format!("exit {c}")))
    };
    record["summary"] = json!(cut(&summary, 160));
    if run["timedOut"] == true {
        record["timedOut"] = json!(true);
    }
    record
}

/// The run on top of its routine's past runs; the files of the ones that fall off go.
fn record(doc: &mut Value, setup: &Setup, slug: &str, run: Value) {
    let runs = doc["runs"][slug].as_array().cloned().unwrap_or_default();
    let mut runs: Vec<Value> = std::iter::once(run).chain(runs).collect();
    for old in runs.iter().skip(KEEP_RUNS) {
        for ext in ["out", "err", "exit", "txt", "cmd"] {
            let _ = std::fs::remove_file(setup.run_file(slug, old["id"].as_str().unwrap_or_default(), ext));
        }
    }
    runs.truncate(KEEP_RUNS);
    doc["runs"][slug] = Value::Array(runs);
}

/// The run's end as a notification, as the routine asks: a click opens it in the routines space; an agent's run with a
/// session can be talked over.
fn tell(r: &Routine, run: &Value, notify: Notifier) {
    let ok = run["ok"] == true;
    if r.notify == Notify::Never || (r.notify == Notify::Failed && ok) {
        return;
    }
    let title = match (ok, run["missed"] == true) {
        (true, _) => format!("✅ {}", r.name),
        (false, true) => format!("⏭ {}", r.name),
        (false, false) => format!("❌ {}", r.name),
    };
    let mut buttons = vec![format!("Open=routine:{}", r.slug)];
    if run["session"].as_str().is_some_and(|s| !s.is_empty()) {
        buttons.push(format!("Talk=act:routine:{}", r.slug));
    }
    notify(&title, run["summary"].as_str().unwrap_or_default(), &format!("routine:{}", r.slug), &buttons);
}

/// A routine as the app shows it, with its state and its next run.
fn shown(r: &Routine, setup: &Setup, doc: &Value, now: u64, servers: &[String]) -> Value {
    let agent = setup.agent_of(r);
    let claude = agent.is_some_and(|a| a.id == "claude");
    let base = mark(doc, &r.slug, "seen").max(mark(doc, &r.slug, "fired"));
    let next = match &r.when {
        _ if r.paused => None,
        Ok(When::Every(secs)) => Some(base + secs * 1000),
        Ok(when @ When::Cron(_)) => when.next_after(local(base.max(now))).map(epoch_ms),
        _ => None,
    };
    let next_runs = next.map_or_else(Vec::new, |first| match &r.when {
        Ok(When::Every(secs)) => (0..3).map(|n| first + n * secs * 1000).collect(),
        Ok(when) => std::iter::successors(Some(first), |at| when.next_after(local(*at)).map(epoch_ms)).take(3).collect(),
        Err(_) => Vec::new(),
    });
    let trigger = match &r.when {
        Ok(When::Manual) => "manual",
        Ok(When::Every(_)) => "every",
        Ok(When::Cron(_)) => "schedule",
        Ok(When::On { .. }) => "event",
        Err(_) => "broken",
    };
    let mut prompt: String = r.body.chars().take(PROMPT_SHOWN).collect();
    if r.body.chars().count() > PROMPT_SHOWN {
        prompt.push('…');
    }
    json!({
        "slug": r.slug, "name": r.name, "path": r.path, "when": r.when_shown(), "whenText": r.when_text, "trigger": trigger,
        "error": r.when.as_ref().err(), "run": if r.run == Run::Agent { "agent" } else { "shell" }, "agent": agent.map(|a| &a.id),
        "agentName": agent.map(|a| &a.name), "agentSet": r.agent, "cwd": r.cwd, "model": r.model,
        "tools": r.tools, "mode": r.mode, "budget": r.budget, "timeout": r.timeout, "paused": r.paused, "prompt": prompt, "next": next,
        "spentWeek": spent(doc, &r.slug, now), "lastCost": last_cost(doc, &r.slug), "problems": problems(r, agent, servers), "over": claude && r.budget.is_some_and(|b| spent(doc, &r.slug, now) > b - 0.01),
        "notify": match r.notify { Notify::Always => "always", Notify::Failed => "failed", Notify::Never => "never" },
        "worktree": r.worktree, "catchUp": r.catch_up, "nextRuns": next_runs,
    })
}

/// What its latest run that cost anything cost.
fn last_cost(doc: &Value, slug: &str) -> Option<f64> {
    list(&doc["runs"][slug]).iter().filter_map(|r| r["cost"].as_f64()).find(|c| *c > 0.0)
}

/// A routine's runs, queue and marks gone with it, and its files.
pub fn forget(doc: &mut Value, setup: &Setup, slug: &str) {
    for field in ["runs", "marks", "running"] {
        if let Some(map) = doc[field].as_object_mut() {
            map.remove(slug);
        }
    }
    if let Some(queued) = doc["queued"].as_array_mut() {
        queued.retain(|q| q["slug"] != slug);
    }
    let _ = std::fs::remove_dir_all(setup.runs.join(slug));
}

/// Runs a routine now, by hand: false when it already runs or waits.
pub fn run_now(store: &mut Store, setup: &Setup, slug: &str, now: u64, notify: Notifier) -> bool {
    let mut doc = current(store, now);
    if is_busy(&doc, slug) {
        return false;
    }
    queue(&mut doc, slug, "run by hand".into(), Value::Null);
    save(store, doc, now);
    round(store, setup, now, notify);
    true
}

/// Changes the document through these steps, the store held throughout.
pub fn change(store: &mut Store, now: u64, edit: impl FnOnce(&mut Value)) {
    let mut doc = current(store, now);
    edit(&mut doc);
    save(store, doc, now);
}

/// Where a run's output is, for the app and the pager.
pub fn output(setup: &Setup, slug: &str, id: &str) -> PathBuf {
    setup.run_file(slug, id, "txt")
}

#[cfg(test)]
mod tests {
    use super::*;

    struct World {
        _dir: tempfile::TempDir,
        store: Store,
        setup: Setup,
        told: Vec<String>,
    }

    impl World {
        fn new() -> World {
            let dir = tempfile::tempdir().unwrap();
            let bin = |name: &str| dir.path().join(name).to_string_lossy().into_owned();
            let agents = agent::builtin()
                .into_iter()
                .map(|mut a| {
                    a.bin = bin(&a.id);
                    a
                })
                .collect();
            let setup = Setup {
                routines: dir.path().join("routines"),
                runs: dir.path().join("runs"),
                agents,
                default_agent: "claude".into(),
                repos: dir.path().to_string_lossy().into_owned(),
                worktrees: dir.path().join("worktrees"),
            };
            std::fs::create_dir_all(&setup.routines).unwrap();
            // A Codex that answers like `codex exec`: its session on stderr, its answer on stdout, keeping what it was asked.
            std::fs::write(
                bin("codex"),
                format!(
                    "#!/bin/sh\nprintf '%s\\n' \"$@\" > {}/codex-argv\necho 'session id: 01a1a225-4c95-7690-83de-431af47ed55f' >&2\necho 'Looked at 2 PRs'\necho 'both green'\n",
                    dir.path().display()
                ),
            )
            .unwrap();
            std::process::Command::new("chmod").args(["+x", &bin("codex")]).status().unwrap();
            // A Claude that answers like `claude -p --output-format stream-json --verbose`, keeping what it was asked.
            std::fs::write(
                bin("claude"),
                format!(
                    "#!/bin/sh\nprintf '%s\\n' \"$@\" > {}/argv\necho '{{\"type\":\"system\",\"subtype\":\"init\",\"mcp_servers\":[{{\"name\":\"claude.ai Slack\"}},{{\"name\":\"claude.ai Datadog\"}},{{\"name\":\"platform\"}}]}}'\necho '{{\"type\":\"assistant\",\"message\":{{\"content\":[{{\"type\":\"tool_use\",\"name\":\"mcp__claude_ai_Slack__slack_send_message\"}}]}}}}'\nprintf '{{\"type\":\"result\",\"is_error\":false,\"result\":\"Swept 3 PRs\\\\nall green\",\"session_id\":\"%s\",\"total_cost_usd\":0.12}}\\n' \"$7\"\n",
                    dir.path().display()
                ),
            )
            .unwrap();
            std::process::Command::new("chmod").args(["+x", &bin("claude")]).status().unwrap();
            let store = Store::open(&dir.path().join("cache"), Duration::ZERO);
            World { _dir: dir, store, setup, told: Vec::new() }
        }

        fn write(&self, slug: &str, text: &str) {
            std::fs::write(self.setup.routines.join(format!("{slug}.md")), text).unwrap();
        }

        fn round(&mut self, now: u64) -> Value {
            let told = &mut self.told;
            round(&mut self.store, &self.setup, now, &mut |title, body, _, buttons| told.push(format!("{title} | {body} | {}", buttons.join(" "))));
            self.store.get(DOC)
        }

        /// Rounds until nothing runs, five seconds at most: the runs are processes, slow while every test runs at once.
        fn settle(&mut self, now: u64) -> Value {
            for _ in 0..500 {
                let doc = self.round(now);
                if doc["running"].as_object().is_none_or(Map::is_empty) {
                    return doc;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            panic!("still running: {}", self.store.get(DOC)["running"]);
        }
    }

    fn ms(text: &str) -> u64 {
        epoch_ms(NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M").unwrap())
    }

    #[test]
    fn a_schedule_runs_at_its_time_once_and_late_after_a_short_absence_only() {
        let mut w = World::new();
        w.write("hello", "---\nwhen: daily 09:00\nrun: shell\n---\necho hello from $COCKPIT_ROUTINE");
        let doc = w.round(ms("2026-10-29 08:30"));
        assert_eq!(doc["routines"][0]["next"], json!(ms("2026-10-29 09:00")));
        assert!(doc["running"].as_object().unwrap().is_empty(), "nothing due yet");
        w.round(ms("2026-10-29 09:00") + 3000);
        let doc = w.settle(ms("2026-10-29 09:01"));
        let run = &doc["runs"]["hello"][0];
        assert_eq!((&run["ok"], &run["summary"], &run["why"]), (&json!(true), &json!("hello from hello"), &json!("scheduled for 09:00")));
        assert_eq!(w.told, ["✅ hello | hello from hello | Open=routine:hello"]);
        assert_eq!(doc["routines"][0]["next"], json!(ms("2026-10-30 09:00")));
        let doc = w.settle(ms("2026-10-29 12:00"));
        assert_eq!(doc["runs"]["hello"].as_array().unwrap().len(), 1, "once a day");
        // The hub down from 08:00 to 18:00: the 09:00 run happens late; down a whole day, it is skipped.
        w.settle(ms("2026-10-30 18:00"));
        let doc = w.settle(ms("2026-10-30 18:01"));
        assert_eq!(doc["runs"]["hello"][0]["why"], "scheduled for 09:00, run late");
        // Down far longer than its catch-up window: the last time it was due is recorded as missed, and told.
        let doc = w.settle(ms("2026-11-02 08:00"));
        let missed = &doc["runs"]["hello"][0];
        assert_eq!((doc["runs"]["hello"].as_array().unwrap().len(), &missed["missed"], &missed["ok"]), (3, &json!(true), &json!(false)));
        assert_eq!(missed["summary"], "missed Sun 09:00: cockpit was not running then");
        assert!(w.told.last().unwrap().starts_with("⏭ hello | missed Sun 09:00"));
        assert_eq!(doc["runs"]["hello"][1]["late"], true);
        // With no catch-up, a run late by more than two minutes is missed rather than run.
        w.write("on-time", "---\nwhen: daily 09:00\nrun: shell\ncatch-up: never\n---\ntrue");
        w.settle(ms("2026-11-02 08:01"));
        let doc = w.settle(ms("2026-11-02 09:10"));
        assert_eq!(doc["runs"]["on-time"][0]["missed"], true);
    }

    #[test]
    fn a_run_in_its_own_worktree_leaves_it_only_with_work_in_it() {
        let mut w = World::new();
        let repo = w.setup.repos.clone() + "/repo";
        let git = |args: &[&str]| assert!(Command::new("git").arg("-C").arg(&repo).args(args).output().unwrap().status.success(), "git {args:?}");
        std::fs::create_dir_all(&repo).unwrap();
        git(&["init", "-q", "-b", "main"]);
        git(&["-c", "user.email=a@b", "-c", "user.name=a", "commit", "-q", "--allow-empty", "-m", "start"]);
        w.write("look", &format!("---\nrun: shell\ncwd: {repo}\nworktree: true\n---\npwd"));
        w.write("work", &format!("---\nrun: shell\ncwd: {repo}\nworktree: true\n---\necho x > new.txt"));
        run_now(&mut w.store, &w.setup, "look", 1_000, &mut |_, _, _, _| {});
        run_now(&mut w.store, &w.setup, "work", 1_000, &mut |_, _, _, _| {});
        let doc = w.settle(2_000);
        let (look, work) = (&doc["runs"]["look"][0], &doc["runs"]["work"][0]);
        assert!(look["summary"].as_str().unwrap().contains("worktrees/repo/auto/look-1000"), "{look}");
        assert_eq!((&look["worktree"]["removed"], &look["worktree"]["branch"]), (&json!(true), &json!("auto/look-1000")));
        assert_eq!((&work["worktree"]["removed"], &work["worktree"]["dirty"]), (&Value::Null, &json!(true)));
        assert!(!landed(&work["worktree"]), "uncommitted work has not landed");
        assert!(Path::new(work["worktree"]["path"].as_str().unwrap()).join("new.txt").is_file());
        std::fs::remove_file(Path::new(work["worktree"]["path"].as_str().unwrap()).join("new.txt")).unwrap();
        assert!(landed(&work["worktree"]));
        remove_worktree(&work["worktree"]).unwrap();
        assert!(!Path::new(work["worktree"]["path"].as_str().unwrap()).exists());
    }

    #[test]
    fn an_interval_counts_from_the_last_run_and_a_paused_routine_waits() {
        let mut w = World::new();
        w.write("tick", "---\nwhen: every 30m\nrun: shell\nnotify: never\n---\ntrue");
        let t0 = ms("2026-10-29 10:00");
        assert_eq!(w.round(t0)["routines"][0]["next"], json!(t0 + 1_800_000));
        w.settle(t0 + 1_800_000);
        let doc = w.settle(t0 + 1_900_000);
        assert_eq!(doc["runs"]["tick"].as_array().unwrap().len(), 1);
        assert_eq!(doc["routines"][0]["next"], json!(t0 + 3_600_000));
        w.write("tick", "---\nwhen: every 30m\nrun: shell\npaused: true\n---\ntrue");
        let doc = w.settle(t0 + 10_000_000);
        assert_eq!((doc["runs"]["tick"].as_array().unwrap().len(), &doc["routines"][0]["next"]), (1, &Value::Null));
        assert!(w.told.is_empty(), "notify: never");
    }

    #[test]
    fn an_event_sets_off_its_routines_once_per_row() {
        let mut w = World::new();
        w.write("fix", "---\nwhen: on ci-failed api\nrun: shell\nnotify: failed\n---\necho \"$COCKPIT_EVENT_KIND $COCKPIT_EVENT_KEY\"; exit 3");
        w.round(1_000);
        let fail = |n: u32| json!({"version": 1, "mine": [{"repo": "api", "number": 1, "url": "u1", "ci": if n % 2 == 1 { "FAILURE" } else { "PENDING" }}]});
        w.store.update("inbox", |_| fail(0));
        std::thread::sleep(Duration::from_millis(5));
        w.store.update("inbox", |_| fail(1));
        let now = now_ms();
        w.round(now);
        let doc = w.settle(now);
        let run = &doc["runs"]["fix"][0];
        assert_eq!(
            (&run["ok"], &run["code"], &run["summary"], &run["why"]),
            (&json!(false), &json!(3), &json!("ci_failing u1"), &json!("🔴 CI failing: api#1"))
        );
        assert_eq!(w.told, ["❌ fix | ci_failing u1 | Open=routine:fix"]);
        // The same PR failing again within ten minutes sets off nothing more.
        w.store.update("inbox", |_| fail(2));
        std::thread::sleep(Duration::from_millis(5));
        w.store.update("inbox", |_| fail(3));
        let doc = w.settle(now + 60_000);
        assert_eq!(doc["runs"]["fix"].as_array().unwrap().len(), 1);
        let ran = w.store.events(0, 50).into_iter().filter(|e| e["doc"] == DOC).count();
        assert_eq!(ran, 1, "the run's end is an event, which sets off nothing");
    }

    #[test]
    fn a_claude_run_keeps_its_answer_session_and_cost() {
        let mut w = World::new();
        w.write("sweep", "---\nname: Morning sweep\nmodel: sonnet\ntools: Read, Bash(gh:*)\n---\nSweep my PRs.");
        w.round(1_000);
        assert!(run_now(&mut w.store, &w.setup, "sweep", 2_000, &mut |_, _, _, _| {}));
        assert!(!run_now(&mut w.store, &w.setup, "sweep", 2_000, &mut |_, _, _, _| {}), "already running");
        let doc = w.settle(3_000);
        let run = &doc["runs"]["sweep"][0];
        assert_eq!((&run["ok"], &run["summary"], &run["cost"], &run["why"]), (&json!(true), &json!("Swept 3 PRs"), &json!(0.12), &json!("run by hand")));
        assert_eq!(run["session"].as_str().unwrap().len(), 36);
        let shown = std::fs::read_to_string(output(&w.setup, "sweep", run["id"].as_str().unwrap())).unwrap();
        assert_eq!(shown, "Swept 3 PRs\nall green");
    }

    #[test]
    fn a_routine_runs_on_the_agent_it_names_headless_without_claudes_settings() {
        let mut w = World::new();
        w.write("sweep", "---\nname: Sweep\nagent: codex\nmodel: sonnet\nmode: acceptEdits\ntools: Read\nbudget: $1/week\n---\nSweep my PRs.");
        let doc = w.round(1_000);
        assert_eq!(
            (&doc["routines"][0]["agent"], &doc["routines"][0]["agentName"], &doc["routines"][0]["run"]),
            (&json!("codex"), &json!("Codex"), &json!("agent"))
        );
        assert_eq!(doc["routines"][0]["problems"], json!(["its tools, budget and lightest model are Claude Code's: Codex runs it without them"]));
        run_now(&mut w.store, &w.setup, "sweep", 2_000, &mut |_, _, _, _| {});
        let doc = w.settle(3_000);
        let run = &doc["runs"]["sweep"][0];
        assert_eq!((&run["ok"], &run["summary"], &run["agent"], &run["cost"]), (&json!(true), &json!("Looked at 2 PRs"), &json!("codex"), &Value::Null));
        assert_eq!(run["session"], "01a1a225-4c95-7690-83de-431af47ed55f", "talked over by resuming it");
        let argv = std::fs::read_to_string(w._dir.path().join("codex-argv")).unwrap();
        let argv: Vec<&str> = argv.lines().collect();
        assert_eq!(argv[..4], ["exec", "--skip-git-repo-check", "--sandbox", "workspace-write"], "a Claude model is never passed to Codex: {argv:?}");
        assert_eq!((argv[4], argv[6], argv[7].starts_with("cockpit runs this as the routine")), ("Sweep my PRs.", "---", true), "{argv:?}");
        assert_eq!(w.told, ["✅ Sweep | Looked at 2 PRs | Open=routine:sweep Talk=act:routine:sweep"]);
        // The default agent runs a routine naming none; one that names an agent cockpit does not know does not start.
        w.setup.default_agent = "codex".into();
        w.write("plain", "---\nnotify: never\n---\nSay hi.");
        w.write("nobody", "---\nagent: nobody\nnotify: never\n---\nSay hi.");
        w.round(4_000);
        run_now(&mut w.store, &w.setup, "plain", 5_000, &mut |_, _, _, _| {});
        run_now(&mut w.store, &w.setup, "nobody", 5_000, &mut |_, _, _, _| {});
        let doc = w.settle(6_000);
        assert_eq!((&doc["runs"]["plain"][0]["agent"], &doc["runs"]["plain"][0]["ok"]), (&json!("codex"), &json!(true)));
        assert_eq!(doc["runs"]["nobody"][0]["summary"], "did not start: no agent \"nobody\"");
    }

    #[test]
    fn an_agent_without_a_headless_mode_is_refused() {
        let r = Routine::parse("s", Path::new("/r/s.md"), "---\nagent: x\n---\nGo.");
        let mut x = agent::builtin().remove(0);
        x.run.clear();
        assert_eq!(agent_argv(&x, &r, "Go."), Err("Claude Code has no headless mode: it cannot run a routine".to_string()));
        assert_eq!(problems(&r, Some(&x), &[]), ["Claude Code has no headless mode: it cannot run a routine"]);
    }

    #[test]
    fn the_claude_command_carries_the_routines_settings() {
        let r = Routine::parse(
            "s",
            Path::new("/r/s.md"),
            "---\nname: S\nmodel: opus\nmode: acceptEdits\nbudget: 1.5\ntools: Read, Bash(gh:*), mcp__x__y\n---\nGo.",
        );
        let servers = ["x".to_string(), "claude.ai Datadog".to_string()];
        let argv = claude_argv("claude", &r, "P".into(), "id", Some(1.234), &servers);
        let tail: Vec<&str> = argv[3..].iter().map(String::as_str).collect();
        assert_eq!(
            tail,
            [
                "--output-format",
                "stream-json",
                "--verbose",
                "--session-id",
                "id",
                "--name",
                "routine: S",
                "--model",
                "opus",
                "--permission-mode",
                "acceptEdits",
                "--max-budget-usd",
                "1.23",
                "--disable-slash-commands",
                "--tools",
                "ToolSearch,Read,Bash",
                "--allowedTools",
                "Read",
                "Bash(gh:*)",
                "mcp__x__y"
            ],
            "only the lightest routines deny servers"
        );
        let open = Routine::parse("o", Path::new("/r/o.md"), "---\ntools: Skill, mcp__x__y\n---\nGo.");
        let argv = claude_argv("claude", &open, "P".into(), "id", None, &[]);
        assert!(!argv.contains(&"--disable-slash-commands".to_string()) && argv.contains(&"ToolSearch,Skill".to_string()), "{argv:?}");
        let bare = Routine::parse("b", Path::new("/r/b.md"), "Go.");
        assert!(!claude_argv("claude", &bare, "P".into(), "id", None, &[]).contains(&"--tools".to_string()), "no tools listed: Claude's defaults");
        let light = Routine::parse("l", Path::new("/r/l.md"), "---\nmodel: lightest\ntools: mcp__x__y\n---\nGo.");
        let argv = claude_argv("claude", &light, "P".into(), "id", None, &servers);
        let tail: Vec<&str> = argv[10..].iter().map(String::as_str).collect();
        assert_eq!(
            tail,
            [
                "--model",
                "haiku",
                "--effort",
                "low",
                "--disable-slash-commands",
                "--tools",
                "ToolSearch",
                "--allowedTools",
                "mcp__x__y",
                "--settings",
                r#"{"deniedMcpServers":[{"serverName":"claude.ai Datadog"}]}"#,
                "--append-system-prompt",
                &light_prompt(&light)
            ]
        );
        let p = prompt(&r, "🔴 CI failed: api main", &json!({"title": "🔴 CI failed: api main", "key": "u1"}), ms("2026-10-29 09:00"));
        assert!(p.starts_with("Go.\n\n---\ncockpit runs this as the routine \"S\" (by hand): 🔴 CI failed: api main, at 2026-10-29 09:00."), "{p}");
        assert!(p.ends_with("What set it off: 🔴 CI failed: api main (u1)."), "{p}");
    }

    #[test]
    fn a_stopped_or_too_long_run_ends_as_such_and_three_run_at_once() {
        let mut w = World::new();
        for n in 0..4 {
            w.write(&format!("slow{n}"), "---\nrun: shell\ntimeout: 1\nnotify: never\n---\nsleep 30");
        }
        let now = now_ms();
        w.round(now);
        for n in 0..4 {
            run_now(&mut w.store, &w.setup, &format!("slow{n}"), now, &mut |_, _, _, _| {});
        }
        let doc = w.store.get(DOC);
        assert_eq!((doc["running"].as_object().unwrap().len(), doc["queued"].as_array().unwrap().len()), (3, 1));
        change(&mut w.store, now, |doc| {
            stop(doc, "slow0", now);
        });
        std::thread::sleep(Duration::from_millis(200));
        let doc = w.round(now + 1000);
        assert_eq!((&doc["runs"]["slow0"][0]["summary"], &doc["runs"]["slow0"][0]["stopped"]), (&json!("stopped"), &json!(true)));
        assert!(!doc["running"]["slow3"].is_null(), "the queued one took its place");
        w.round(now + 61_000);
        std::thread::sleep(Duration::from_millis(200));
        let doc = w.settle(now + 62_000);
        assert_eq!(doc["runs"]["slow1"][0]["summary"], "timed out after 1 min");
    }

    #[test]
    fn a_weekly_budget_caps_each_run_and_holds_back_runs_once_spent() {
        let mut w = World::new();
        w.write("sweep", "---\nname: Sweep\nbudget: $0.20/week\n---\nSweep.");
        let now = now_ms();
        w.round(now);
        run_now(&mut w.store, &w.setup, "sweep", now, &mut |_, _, _, _| {});
        let doc = w.settle(now);
        assert_eq!((&doc["routines"][0]["spentWeek"], &doc["routines"][0]["over"]), (&json!(0.12), &json!(false)));
        run_now(&mut w.store, &w.setup, "sweep", now + 1, &mut |_, _, _, _| {});
        let doc = w.settle(now + 2);
        assert_eq!((doc["runs"]["sweep"].as_array().unwrap().len(), &doc["routines"][0]["over"]), (2, &json!(true)));
        w.told.clear();
        let told = &mut w.told;
        run_now(&mut w.store, &w.setup, "sweep", now + 3, &mut |title, body, _, _| told.push(format!("{title} | {body}")));
        run_now(&mut w.store, &w.setup, "sweep", now + 4, &mut |title, body, _, _| told.push(format!("{title} | {body}")));
        let doc = w.settle(now + 5);
        assert_eq!(doc["runs"]["sweep"].as_array().unwrap().len(), 2, "held back");
        assert_eq!(w.told, ["⏸ Sweep | its $0.20 for the week is spent ($0.24): it waits until older runs fall out of the week"], "said once");
        let doc = w.settle(now + WEEK_MS + 10);
        assert_eq!((&doc["routines"][0]["spentWeek"], &doc["routines"][0]["lastCost"]), (&json!(0.0), &json!(0.12)));
    }

    #[test]
    fn a_claude_run_records_the_servers_the_lightest_routines_then_deny() {
        let mut w = World::new();
        w.write("dm", "---\nmodel: lightest\ntools: mcp__claude_ai_Slack__slack_send_message\n---\nSay hi.");
        w.round(1_000);
        run_now(&mut w.store, &w.setup, "dm", 2_000, &mut |_, _, _, _| {});
        w.settle(3_000);
        assert!(!std::fs::read_to_string(w._dir.path().join("argv")).unwrap().contains("deniedMcpServers"), "nothing known to deny yet");
        assert_eq!(known_servers(&w.setup), ["claude.ai Slack", "claude.ai Datadog", "platform"]);
        std::fs::write(w.setup.runs.join("mcp-servers.json"), r#"{"servers": ["cockpit"]}"#).unwrap();
        run_now(&mut w.store, &w.setup, "dm", 4_000, &mut |_, _, _, _| {});
        w.settle(5_000);
        let argv = std::fs::read_to_string(w._dir.path().join("argv")).unwrap();
        assert!(argv.contains(r#"{"deniedMcpServers":[{"serverName":"cockpit"}]}"#), "{argv}");
        assert_eq!(known_servers(&w.setup), ["cockpit", "claude.ai Slack", "claude.ai Datadog", "platform"], "the list only grows");
    }

    #[test]
    fn a_lightest_run_that_calls_none_of_its_tools_runs_once_more_then_fails() {
        let mut w = World::new();
        w.write("other", "---\nname: Other\nmodel: lightest\ntools: mcp__claude_ai_Linear__get_issue\n---\nGet ENG-1.");
        w.round(1_000);
        run_now(&mut w.store, &w.setup, "other", 2_000, &mut |_, _, _, _| {});
        let doc = w.settle(3_000);
        let runs: Vec<(&str, &str)> = list(&doc["runs"]["other"]).iter().map(|r| (r["why"].as_str().unwrap(), r["summary"].as_str().unwrap())).collect();
        assert_eq!(runs, [("again: run by hand", "called none of its tools: Swept 3 PRs"), ("run by hand", "called none of its tools: Swept 3 PRs")]);
        assert_eq!(w.told, ["❌ Other | called none of its tools: Swept 3 PRs | Open=routine:other Talk=act:routine:other"], "told once, after the second run");
        w.write("dm", "---\nmodel: lightest\ntools: mcp__claude_ai_Slack__slack_send_message\nnotify: never\n---\nSay hi.");
        w.round(4_000);
        run_now(&mut w.store, &w.setup, "dm", 5_000, &mut |_, _, _, _| {});
        let doc = w.settle(6_000);
        assert_eq!((&doc["runs"]["dm"][0]["ok"], &doc["runs"]["dm"][0]["called"]), (&json!(true), &json!(["mcp__claude_ai_Slack__slack_send_message"])));
    }

    fn problems_of(r: &Routine, servers: &[String]) -> Vec<String> {
        let claude = agent::builtin().remove(0);
        problems(r, Some(&claude), servers)
    }

    #[test]
    fn a_lightest_routine_says_what_its_file_lacks() {
        let servers = ["claude.ai Slack".to_string()];
        let parse = |text: &str| Routine::parse("r", Path::new("/r/r.md"), text);
        assert_eq!(
            problems_of(&parse("---\nmodel: lightest\n---\nGo."), &servers),
            ["lists no tools: a lightest run loads nothing else, so it can call nothing"]
        );
        assert_eq!(
            problems_of(&parse("---\nmodel: lightest\ntools: mcp__claude_ai_Slack__slack_send_message, mcp__claude_ai_Slak__x, Read\n---\nGo."), &servers),
            ["mcp__claude_ai_Slak__x belongs to no MCP server Claude Code connects"]
        );
        assert!(problems_of(&parse("---\nmodel: lightest\ntools: mcp__a__b\n---\nGo."), &[]).is_empty(), "no server known yet: nothing to say");
        assert_eq!(
            problems_of(&parse("---\nmodel: haiku\n---\nGo."), &servers),
            ["lists no tools and no mode: nobody approves its gh, git or file calls, so they are refused"]
        );
        assert!(problems_of(&parse("---\nmodel: haiku\nmode: auto\n---\nGo."), &servers).is_empty(), "a mode decides for it");
        assert!(problems_of(&parse("---\nrun: shell\n---\necho hi"), &servers).is_empty(), "a script asks nobody");
        let prompt = light_prompt(&parse("---\nmodel: lightest\ntools: mcp__a__b, Read\n---\nGo."));
        assert!(prompt.contains("ToolSearch with query \"select:mcp__a__b\""), "{prompt}");
    }

    #[test]
    fn a_routine_that_cannot_start_says_why() {
        let mut w = World::new();
        w.write("nowhere", "---\nrun: shell\ncwd: /no/such/folder\n---\ntrue");
        w.round(1_000);
        run_now(&mut w.store, &w.setup, "nowhere", 2_000, &mut |_, _, _, _| {});
        assert_eq!(w.store.get(DOC)["runs"]["nowhere"][0]["summary"], "did not start: no folder /no/such/folder");
    }
}
