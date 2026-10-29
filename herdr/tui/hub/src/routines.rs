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
use cockpit_core::files;
use cockpit_core::routines::{self, Notify, Routine, Run, When};
use serde_json::{Map, Value, json};

use crate::store::Store;
use crate::value::{cut, list};

pub const DOC: &str = "routines";
const TICK: Duration = Duration::from_secs(5);
/// A run missed while the hub was down still happens when it comes back within this long.
const CATCH_UP_MS: u64 = 12 * 3_600_000;
const KEEP_RUNS: usize = 20;
const AT_ONCE: usize = 3;
/// One routine runs once per row an event touches in this long: a PR's CI failing and its run failing are one failure.
const EVENT_GAP_MS: u64 = 10 * 60_000;
/// A run stopped that still lives this long after is killed.
const KILL_AFTER_MS: u64 = 10_000;
const PROMPT_SHOWN: usize = 600;
const WEEK_MS: u64 = 7 * 86_400_000;
/// The lightest routines' whole system prompt, in place of Claude Code's.
const LIGHT_PROMPT: &str = "You run one unattended task for cockpit, with only the tools it names. Do what it asks, \
                            then reply in one short line saying what you did.";

/// Where routines are read, where their runs go, and what runs Claude.
pub struct Setup {
    pub routines: PathBuf,
    pub runs: PathBuf,
    pub claude: String,
    pub repos: String,
}

impl Setup {
    pub fn live() -> Setup {
        let claude = crate::proc::which("claude").unwrap_or_else(|| files::home().join(".claude/local/claude")).to_string_lossy().into_owned();
        Setup { routines: routines::dir(), runs: files::cache().join("routines"), claude, repos: crate::config::load().repos() }
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
    json!({"version": 1, "cursor": now, "routines": [], "running": {}, "queued": [], "runs": {}, "marks": {}})
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
    finish(&mut doc, setup, &all, now, notify);
    for r in &all {
        let marks = &mut doc["marks"][&r.slug];
        if marks["seen"].is_null() {
            *marks = json!({"seen": now, "fired": 0});
        }
    }
    schedule(&mut doc, &all, now);
    let cursor = doc["cursor"].as_u64().unwrap_or(now);
    let mut events = store.events(cursor + 1, 500);
    events.reverse();
    listen(&mut doc, &all, &events, now);
    start_queued(&mut doc, setup, &all, now, notify);
    doc["routines"] = Value::Array(all.iter().map(|r| shown(r, &doc, now)).collect());
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

/// Queues the routines whose time came: an interval once it passed since the last run, a schedule at its time, or late
/// when the hub was down then, unless that was too long ago.
fn schedule(doc: &mut Value, all: &[Routine], now: u64) {
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
                if now.saturating_sub(due_ms) <= CATCH_UP_MS && !is_busy(doc, &r.slug) {
                    let late = if now.saturating_sub(due_ms) > 120_000 { ", run late" } else { "" };
                    queue(doc, &r.slug, format!("scheduled for {}{late}", due.format("%H:%M")), Value::Null);
                }
            }
            _ => {}
        }
    }
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
        let left_this_week = r.budget.filter(|_| r.run == Run::Claude).map(|b| b - spent(doc, slug, now));
        if let Some(left) = left_this_week.filter(|l| *l < 0.01) {
            over_budget(doc, r, left, notify);
            continue;
        }
        doc["marks"][slug]["over"] = json!(false);
        match start(setup, r, why, &q["event"], now, left_this_week) {
            Ok(run) => doc["running"][slug] = run,
            Err(err) => record(doc, setup, slug, failed_to_start(now, why, &err)),
        }
    }
    doc["queued"] = Value::Array(left);
}

fn failed_to_start(now: u64, why: &str, err: &str) -> Value {
    json!({"id": now.to_string(), "started": now, "ended": now, "ok": false, "code": null, "why": why, "summary": format!("did not start: {err}")})
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

/// The MCP servers Claude Code connected in the routines' runs, as their init events listed them: the lightest runs
/// connect only their own, so the list only grows.
fn known_servers(setup: &Setup) -> Vec<String> {
    files::load(&setup.runs.join("mcp-servers.json")).map(|v| list(&v["servers"]).iter().filter_map(|s| s.as_str().map(str::to_string)).collect()).unwrap_or_default()
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
        argv.extend(["--system-prompt".into(), LIGHT_PROMPT.into()]);
    }
    argv
}

/// Starts a run in its own process group: its entry in `running`.
fn start(setup: &Setup, r: &Routine, why: &str, event: &Value, now: u64, cap: Option<f64>) -> Result<Value, String> {
    let id = now.to_string();
    let dir = setup.runs.join(&r.slug);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let session = if r.run == Run::Claude { uuid() } else { String::new() };
    let command = match r.run {
        Run::Claude => claude_argv(&setup.claude, r, prompt(r, why, event, now), &session, cap, &known_servers(setup)),
        Run::Shell => vec!["sh".into(), "-c".into(), r.body.clone()],
    };
    // A Claude run without a folder gets its own, where no project's CLAUDE.md and memory are read.
    let cwd = match (r.cwd.is_empty(), r.run) {
        (false, _) => r.cwd.clone(),
        (true, Run::Claude) => dir.to_string_lossy().into_owned(),
        (true, Run::Shell) => setup.repos.clone(),
    };
    if !Path::new(&cwd).is_dir() {
        return Err(format!("no folder {cwd}"));
    }
    let exit = setup.run_file(&r.slug, &id, "exit");
    let mut argv = vec!["sh".to_string(), "-c".into(), "\"$@\"; echo \"$?\" > \"$0\"".into(), exit.to_string_lossy().into_owned()];
    argv.extend(command);
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
    Ok(json!({"id": id, "pid": pid, "started": now, "why": why, "session": session, "event": event, "timeout": r.timeout}))
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
        let ended = ended(setup, &slug, &run, code, r.map_or(Run::Shell, |r| r.run), now);
        if let Some(r) = r {
            tell(r, &ended, notify);
        }
        if let Some(running) = doc["running"].as_object_mut() {
            running.remove(&slug);
        }
        record(doc, setup, &slug, ended);
    }
}

/// The run's record from what it left: Claude's JSON answer or the script's output, in <id>.txt for the app.
fn ended(setup: &Setup, slug: &str, run: &Value, code: Option<i32>, kind: Run, now: u64) -> Value {
    let id = run["id"].as_str().unwrap_or_default();
    let out = std::fs::read_to_string(setup.run_file(slug, id, "out")).unwrap_or_default();
    let err = std::fs::read_to_string(setup.run_file(slug, id, "err")).unwrap_or_default();
    let mut record = json!({
        "id": id, "started": run["started"], "ended": now, "code": code, "why": run["why"], "session": run["session"], "ok": code == Some(0),
    });
    let events: Vec<Value> = if kind == Run::Claude { out.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).collect() } else { Vec::new() };
    let answer = events.iter().rev().find(|e| e["type"] == "result").cloned();
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
        for ext in ["out", "err", "exit", "txt"] {
            let _ = std::fs::remove_file(setup.run_file(slug, old["id"].as_str().unwrap_or_default(), ext));
        }
    }
    runs.truncate(KEEP_RUNS);
    doc["runs"][slug] = Value::Array(runs);
}

/// The run's end as a notification, as the routine asks: a click opens it in the routines space; a Claude run can be
/// talked over.
fn tell(r: &Routine, run: &Value, notify: Notifier) {
    let ok = run["ok"] == true;
    if r.notify == Notify::Never || (r.notify == Notify::Failed && ok) {
        return;
    }
    let title = if ok { format!("✅ {}", r.name) } else { format!("❌ {}", r.name) };
    let mut buttons = vec![format!("Open=routine:{}", r.slug)];
    if run["session"].as_str().is_some_and(|s| !s.is_empty()) {
        buttons.push(format!("Talk=act:routine:{}", r.slug));
    }
    notify(&title, run["summary"].as_str().unwrap_or_default(), &format!("routine:{}", r.slug), &buttons);
}

/// A routine as the app shows it, with its state and its next run.
fn shown(r: &Routine, doc: &Value, now: u64) -> Value {
    let base = mark(doc, &r.slug, "seen").max(mark(doc, &r.slug, "fired"));
    let next = match &r.when {
        _ if r.paused => None,
        Ok(When::Every(secs)) => Some(base + secs * 1000),
        Ok(when @ When::Cron(_)) => when.next_after(local(base.max(now))).map(epoch_ms),
        _ => None,
    };
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
        "error": r.when.as_ref().err(), "run": if r.run == Run::Claude { "claude" } else { "shell" }, "cwd": r.cwd, "model": r.model,
        "tools": r.tools, "mode": r.mode, "budget": r.budget, "timeout": r.timeout, "paused": r.paused, "prompt": prompt, "next": next,
        "spentWeek": spent(doc, &r.slug, now), "lastCost": last_cost(doc, &r.slug), "over": r.run == Run::Claude && r.budget.is_some_and(|b| spent(doc, &r.slug, now) > b - 0.01),
        "notify": match r.notify { Notify::Always => "always", Notify::Failed => "failed", Notify::Never => "never" },
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
            let setup = Setup {
                routines: dir.path().join("routines"),
                runs: dir.path().join("runs"),
                claude: dir.path().join("claude").to_string_lossy().into_owned(),
                repos: dir.path().to_string_lossy().into_owned(),
            };
            std::fs::create_dir_all(&setup.routines).unwrap();
            // A Claude that answers like `claude -p --output-format stream-json --verbose`, keeping what it was asked.
            std::fs::write(
                &setup.claude,
                format!(
                    "#!/bin/sh\nprintf '%s\\n' \"$@\" > {}/argv\necho '{{\"type\":\"system\",\"subtype\":\"init\",\"mcp_servers\":[{{\"name\":\"claude.ai Slack\"}},{{\"name\":\"claude.ai Datadog (2)\"}},{{\"name\":\"backstage\"}}]}}'\nprintf '{{\"type\":\"result\",\"is_error\":false,\"result\":\"Swept 3 PRs\\\\nall green\",\"session_id\":\"%s\",\"total_cost_usd\":0.12}}\\n' \"$7\"\n",
                    dir.path().display()
                ),
            )
            .unwrap();
            std::process::Command::new("chmod").args(["+x", &setup.claude]).status().unwrap();
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

        /// Rounds until nothing runs, a second at most.
        fn settle(&mut self, now: u64) -> Value {
            for _ in 0..100 {
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
        let doc = w.settle(ms("2026-11-02 08:00"));
        assert_eq!(doc["runs"]["hello"].as_array().unwrap().len(), 2);
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
        assert_eq!((&run["ok"], &run["code"], &run["summary"], &run["why"]), (&json!(false), &json!(3), &json!("ci_failing u1"), &json!("🔴 CI failing: api#1")));
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
    fn the_claude_command_carries_the_routines_settings() {
        let r = Routine::parse("s", Path::new("/r/s.md"), "---\nname: S\nmodel: opus\nmode: acceptEdits\nbudget: 1.5\ntools: Read, Bash(gh:*), mcp__x__y\n---\nGo.");
        let servers = ["x".to_string(), "claude.ai Datadog (2)".to_string()];
        let argv = claude_argv("claude", &r, "P".into(), "id", Some(1.234), &servers);
        let tail: Vec<&str> = argv[3..].iter().map(String::as_str).collect();
        assert_eq!(
            tail,
            [
                "--output-format", "stream-json", "--verbose", "--session-id", "id", "--name", "routine: S", "--model", "opus", "--permission-mode",
                "acceptEdits", "--max-budget-usd", "1.23", "--disable-slash-commands", "--tools", "ToolSearch,Read,Bash", "--allowedTools", "Read",
                "Bash(gh:*)", "mcp__x__y"
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
                "--model", "haiku", "--effort", "low", "--disable-slash-commands", "--tools", "ToolSearch", "--allowedTools", "mcp__x__y", "--settings",
                r#"{"deniedMcpServers":[{"serverName":"claude.ai Datadog (2)"}]}"#, "--system-prompt", LIGHT_PROMPT
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
        assert_eq!(known_servers(&w.setup), ["claude.ai Slack", "claude.ai Datadog (2)", "backstage"]);
        std::fs::write(w.setup.runs.join("mcp-servers.json"), r#"{"servers": ["cockpit"]}"#).unwrap();
        run_now(&mut w.store, &w.setup, "dm", 4_000, &mut |_, _, _, _| {});
        w.settle(5_000);
        let argv = std::fs::read_to_string(w._dir.path().join("argv")).unwrap();
        assert!(argv.contains(r#"{"deniedMcpServers":[{"serverName":"cockpit"}]}"#), "{argv}");
        assert_eq!(known_servers(&w.setup), ["cockpit", "claude.ai Slack", "claude.ai Datadog (2)", "backstage"], "the list only grows");
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
