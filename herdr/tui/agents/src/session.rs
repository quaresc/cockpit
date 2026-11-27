//! Agent sessions cockpit drives itself. The agent runs under `--input-format stream-json`, so what it says, the tools
//! it calls and the permissions it asks for arrive as events instead of as a screen to read, and the person's words go
//! back the same way. A permission is a `can_use_tool` control request on that pipe, answered by a `control_response`
//! that names it.

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use cockpit_core::agent::{self, Agent, Offer, Picked, Setting};
use cockpit_core::seen::now_ms;
use cockpit_core::transcripts::{self, Detail, TaskState, Transcripts, arg_line, background_id, call_task};
use cockpit_core::{accounts, modes};
use serde_json::{Value, json};

/// What an answer is told when the question it names is not the one asked now.
pub const CHANGED: &str = "The question changed — look again";

/// How many settled asks a session remembers, so a new ask never takes an id an old card still shows.
const SETTLED: usize = 64;

/// What a live session holds: the reader thread fills it, the view reads it.
#[derive(Clone, Debug, Default)]
pub struct State {
    /// The agent's own id for the session, once it says: what `--resume` takes.
    pub session_id: String,
    pub agent: String,
    pub cwd: String,
    /// The config folder of the account it runs on (`accounts`), set in the agent's `account_env`; empty for the hub's
    /// own.
    pub account: String,
    /// The conversation in the shape the chat draws: `{role, at, text, tool}`.
    pub messages: Vec<Value>,
    /// The slash commands the agent published, and republishes when they change.
    pub commands: Vec<Value>,
    /// The model the agent answers with, as it names it ("claude-opus-5-5").
    pub model: String,
    /// Where each tool call sits in `messages`, so its result can say whether it worked.
    calls: HashMap<String, usize>,
    pub cost: Option<f64>,
    pub usage: Value,
    pub rate_limit: Value,
    /// When the agent last reported its rate limit, in ms, so a verdict it repeats each turn stays fresh.
    pub rate_limit_at: u64,
    pub mode: String,
    /// When it started, in ms.
    pub started: u64,
    /// True between a prompt and its result.
    pub working: bool,
    /// When it last went from working to resting or back, in ms.
    pub since: u64,
    /// Why its process is over, when it is.
    pub ended: Option<String>,
    /// Why its last turn failed while the session stays open for the next prompt.
    pub failed: Option<String>,
    /// The person stopped its last turn.
    pub stopped: bool,
    /// What the agent is asking permission for, oldest first: `{id, request_id, at, tool_name, input, tool_use_id,
    /// description}`.
    pub pending: Vec<Value>,
    /// The ids of asks already answered or taken back.
    settled: VecDeque<String>,
    /// True between an interrupt and the result it cuts short, which is not a failure.
    pub interrupted: bool,
    /// When that interrupt was sent, in ms: a stop pending past INTERRUPT_HOLDS_MS lets another go.
    pub interrupted_at: u64,
    /// Brought back after a restart with no process: the next prompt resumes it from its transcript.
    pub paused: bool,
    /// When its last process of ours ended, in ms: a transcript written since was written by someone else.
    pub exited_at: u64,
    /// Its failed turn or ended process was told: once, until its next prompt.
    told_failure: bool,
    /// The effort level it answers with, as it reports it, else as it was set; empty when unknown.
    pub effort: String,
    /// Fast mode as the person set it, which each start passes on; None when never set.
    pub fast: Option<bool>,
    /// Fast mode as the agent reports it serving; None until it says.
    pub fast_on: Option<bool>,
    /// Why fast mode does not serve, in the agent's words (`model_not_allowed`), while it says so.
    pub fast_why: Option<String>,
    /// The models its own catalog offers this session: `[{id, label, efforts?, fast?}]`.
    pub models: Vec<Value>,
    /// A setting changed while it ran that its agent takes only at a start: its next message at rest resumes it.
    pub restart: bool,
    /// The control requests sent and not answered yet, by request id.
    asked: HashMap<String, Asked>,
    /// The subagents, background jobs and workflows its turns started, in the shape the views send: `{id, kind, name,
    /// state, started, ended, summary, tool_use_id}`.
    pub tasks: Vec<Value>,
}

/// What a control request cockpit sent asks for, so its answer is read for it.
#[derive(Clone, Debug, PartialEq)]
enum Asked {
    /// A setting, to this value.
    Set(Setting, Value),
    /// The models its catalog offers.
    Catalog,
}

impl State {
    /// The session as the hub serves it, its messages left out: `title` is its first prompt's first line, `count` how
    /// many messages it holds.
    pub fn json(&self) -> Value {
        let title = self.messages.iter().find(|m| m["role"] == "user").and_then(|m| m["text"].as_str()).map(|t| t.lines().next().unwrap_or(t));
        json!({
            "session": self.session_id, "agent": self.agent, "cwd": self.cwd, "account": self.account, "mode": self.mode, "model": self.model,
            "commands": self.commands, "cost": self.cost, "started": self.started,
            "usage": self.usage, "rate_limit": self.rate_limit, "rate_limit_at": self.rate_limit_at, "working": self.working, "ended": self.ended,
            "failed": self.failed, "interrupted": self.stopped, "since": self.since, "pending": self.pending,
            "paused": self.paused, "title": title, "count": self.messages.len(),
            "effort": self.effort, "fast": self.fast, "fast_on": self.fast_on, "fast_why": self.fast_why, "models": self.models,
            "restart": self.restart, "tasks": self.tasks,
        })
    }
}

pub struct Session {
    pub state: Arc<Mutex<State>>,
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    /// The agent's process until it is reaped.
    child: Mutex<Option<Child>>,
    /// The demo world's sessions run no process and answer in memory.
    demo: bool,
}

fn live() -> MutexGuard<'static, HashMap<String, Arc<Session>>> {
    static LIVE: OnceLock<Mutex<HashMap<String, Arc<Session>>>> = OnceLock::new();
    LIVE.get_or_init(|| Mutex::new(demo_sessions())).lock().unwrap_or_else(|e| e.into_inner())
}

static TOUCH: OnceLock<fn()> = OnceLock::new();
static TOUCH_LATER: OnceLock<fn()> = OnceLock::new();

/// The hub asks to be told when a session moves: `now` builds the session view again at once, `later` the views that
/// cost more (the agents view, the chat), at most every SLOW while a turn streams.
pub fn on_change(now: fn(), later: fn()) {
    let _ = TOUCH.set(now);
    let _ = TOUCH_LATER.set(later);
}

/// How often the costlier views are built again while a session streams.
const SLOW: u64 = 250;

fn moved() {
    static LAST: AtomicU64 = AtomicU64::new(0);
    static WAITING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    remember();
    if let Some(t) = TOUCH.get() {
        t();
    }
    let Some(later) = TOUCH_LATER.get().copied() else { return };
    let since = now_ms().saturating_sub(LAST.load(Ordering::SeqCst));
    if since >= SLOW {
        LAST.store(now_ms(), Ordering::SeqCst);
        later();
    } else if !WAITING.swap(true, Ordering::SeqCst) {
        // One build after the burst, so the last event of it is never left out.
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(SLOW - since));
            WAITING.store(false, Ordering::SeqCst);
            LAST.store(now_ms(), Ordering::SeqCst);
            later();
        });
    }
}

/// Where the sessions' logs go.
pub fn dir() -> PathBuf {
    cockpit_core::files::cache().join("driven")
}

/// A line in the session's own log: what it asked, what it was answered and from where.
pub fn note(key: &str, line: &str) {
    // A key comes from the client: only one shaped like a session's names a file here.
    if key.is_empty() || key.len() > 80 || !key.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '-' | '_' | '.')) || key.contains("..") {
        return;
    }
    let dir = dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(format!("{key}.log"));
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        // What a client sent stays on its one line: a newline in it cannot write a line of its own.
        let _ = writeln!(file, "{} {}", chrono::Utc::now().to_rfc3339(), line.replace(['\n', '\r'], " "));
    }
}

/// One line down the agent's stdin.
fn write_line(stdin: &Mutex<Option<ChildStdin>>, line: &Value) -> Result<(), String> {
    let mut held = stdin.lock().unwrap_or_else(|e| e.into_inner());
    let pipe = held.as_mut().ok_or("the session takes no more")?;
    writeln!(pipe, "{line}").map_err(|e| e.to_string())?;
    pipe.flush().map_err(|e| e.to_string())
}

static SEQ: AtomicU64 = AtomicU64::new(0);

type Locks = Mutex<HashMap<String, Arc<Mutex<()>>>>;

fn locks() -> MutexGuard<'static, HashMap<String, Arc<Mutex<()>>>> {
    static LOCKS: OnceLock<Locks> = OnceLock::new();
    LOCKS.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner())
}

/// One launch, End or change of mode at a time per session: the look at whether its agent runs, the start that follows
/// and the words sent with it go together, and an End sees no turn begin between its look and its stop.
fn launching(key: &str) -> Arc<Mutex<()>> {
    Arc::clone(locks().entry(key.to_string()).or_default())
}

/// What words for a session that was ended meanwhile are told: nothing picks it up again.
pub const GONE: &str = "the session was ended";

/// Whether a refusal is `GONE`'s: the session is not there to take anything back.
pub fn is_gone(refusal: &str) -> bool {
    refusal == GONE || refusal == "no such session"
}

/// What a new session starts with besides its mode: a model, an effort level and fast mode, each its agent's own when
/// not given.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Picks {
    pub model: String,
    pub effort: String,
    pub fast: Option<bool>,
    /// The config folder of the account to run on; empty for the hub's own.
    pub account: String,
}

impl Picks {
    /// The picks a `start` act names: `model`, `effort`, `fast` and `account`.
    pub fn from(arg: &Value) -> Picks {
        let text = |k: &str| arg[k].as_str().unwrap_or_default().trim().to_string();
        Picks { model: text("model"), effort: text("effort"), fast: arg["fast"].as_bool(), account: text("account") }
    }

    /// The account's folder as the session keeps it: empty for the hub's own.
    fn account(&self) -> String {
        let dir = accounts::key(&self.account);
        if self.account.is_empty() || dir == accounts::own() { String::new() } else { dir }
    }

    /// Refused when the agent takes one of them not: a model another agent claims, a level it does not list, fast mode
    /// it has not.
    fn check(&self, found: &Agent) -> Result<(), String> {
        if !self.model.is_empty() && !found.takes_model(&self.model) {
            return Err(format!("{} takes no model {}", found.name, self.model));
        }
        if !self.effort.is_empty() && !found.takes_effort(&self.effort) {
            return Err(format!("{} takes no effort level {}", found.name, self.effort));
        }
        if self.fast.is_some() && found.fast.is_none() {
            return Err(format!("{} has no fast mode", found.name));
        }
        let account = self.account();
        if !account.is_empty() {
            if found.account_env.is_none() {
                return Err(format!("{} runs on one account here", found.name));
            }
            if !accounts::all().iter().any(|a| a.dir == account) {
                return Err(format!("no {} account in {}", found.name, accounts::shown(&account)));
            }
        }
        Ok(())
    }
}

/// Starts an agent under the streaming protocol in `cwd` with what was picked, and returns the key it is kept under.
pub fn start(agent_id: &str, cwd: &str, mode: &str, picks: &Picks) -> Result<String, String> {
    let found = agent::all().into_iter().find(|a| a.id == agent_id).ok_or_else(|| format!("no agent {agent_id:?}"))?;
    if found.stream.is_none() {
        return Err(format!("{} has no streaming protocol cockpit speaks yet", found.name));
    }
    picks.check(&found)?;
    // Two started in the same instant still get keys of their own.
    let key = format!("{}:{}-{}", found.id, now_ms(), SEQ.fetch_add(1, Ordering::Relaxed));
    let state = State {
        agent: found.id.clone(),
        cwd: cwd.to_string(),
        account: picks.account(),
        mode: mode.to_string(),
        started: now_ms(),
        session_id: new_session_id(),
        model: picks.model.clone(),
        effort: picks.effort.clone(),
        fast: picks.fast,
        ..Default::default()
    };
    let session = Arc::new(Session { state: Arc::new(Mutex::new(state)), stdin: Arc::new(Mutex::new(None)), child: Mutex::new(None), demo: false });
    let lock = launching(&key);
    let _one = lock.lock().unwrap_or_else(|e| e.into_inner());
    live().insert(key.clone(), Arc::clone(&session));
    launch(&found, &key, &session, false).inspect_err(|_| {
        live().remove(&key);
    })?;
    Ok(key)
}

/// Runs the agent for `session` under the streaming protocol in a process group of its own, under the session id it
/// already has: a new session, or one `resume`d from its transcript. A reader thread folds what the agent says into the
/// state; called with the session's launch lock held.
fn launch(found: &Agent, key: &str, session: &Arc<Session>, resume: bool) -> Result<(), String> {
    let (session_id, cwd, mode, model, effort, fast, account) = {
        let s = session.state.lock().unwrap_or_else(|e| e.into_inner());
        (s.session_id.clone(), s.cwd.clone(), s.mode.clone(), s.model.clone(), s.effort.clone(), s.fast, s.account.clone())
    };
    let picked = Picked { model: Some(model.as_str()), effort: Some(effort.as_str()), fast };
    let argv = found.stream_argv(&session_id, &mode, resume, &picked).ok_or_else(|| format!("{} cannot run in chat in mode {mode}", found.name))?;
    prune_logs();
    let mut command = Command::new(found.path());
    command.args(&argv).current_dir(&cwd).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).process_group(0);
    // A hub started from inside an agent's terminal must not make the session think it is nested in it or that terminal's;
    // its key says whose the views it shows are.
    cockpit_core::run::untie(&mut command).env(cockpit_core::run::DRIVEN_KEY, key);
    // Another account than the hub's: its config folder, in the variable the agent's table row names.
    if let (Some(var), false) = (found.account_env.as_deref(), account.is_empty()) {
        command.env(var, &account);
    }
    let mut child = command.spawn().map_err(|e| format!("{}: {e}", found.path()))?;
    let pid = child.id();
    // An End while it started wins: the session is gone, and so is this process.
    if !live().contains_key(key) {
        end_group(pid, false);
        let _ = child.wait();
        return Err(GONE.into());
    }
    let stdout = child.stdout.take().ok_or("no stdout")?;
    // What it says on stderr goes to the session's log.
    if let Some(stderr) = child.stderr.take() {
        let named = key.to_string();
        std::thread::spawn(move || {
            for line in lines(BufReader::new(stderr)).filter(|l| !l.trim().is_empty()) {
                note(&named, &format!("stderr: {line}"));
            }
        });
    }
    *session.stdin.lock().unwrap_or_else(|e| e.into_inner()) = child.stdin.take();
    *session.child.lock().unwrap_or_else(|e| e.into_inner()) = Some(child);
    // Its own catalog, asked at each start: the models it offers this session, with their effort levels and fast mode.
    let _ = ask(session, json!({"subtype": "list_models"}), Asked::Catalog);
    {
        let mut s = session.state.lock().unwrap_or_else(|e| e.into_inner());
        (s.paused, s.ended, s.interrupted) = (false, None, false);
    }
    let how = if resume { "resumed" } else { "started" };
    note(key, &format!("{how} {} {session_id} in {cwd}, mode {mode}", found.path()));
    let (named, state, stdin, session) = (key.to_string(), Arc::clone(&session.state), Arc::clone(&session.stdin), Arc::clone(session));
    std::thread::spawn(move || {
        for line in lines(BufReader::new(stdout)) {
            let Ok(event) = serde_json::from_str::<Value>(&line) else { continue };
            let mut st = state.lock().unwrap_or_else(|e| e.into_inner());
            let turn = outcome(&event, st.interrupted);
            let taken = take(&mut st, &event);
            drop(st);
            if let Some(reply) = taken.reply {
                let _ = write_line(&stdin, &reply);
            }
            if let Some(line) = taken.note {
                note(&named, &line);
            }
            if let Some(tool) = taken.asked {
                tell_asked(&named, &state, &tool);
            }
            moved();
            if let Some(turn) = turn {
                crate::gui::turn_ended(&named, turn);
            }
            if turn == Some("failed") {
                tell_failed(&state);
            }
        }
        let Some(mut child) = reaped(&session, pid, &named) else { return };
        // Reaped, so an ended session leaves no process behind; taken out first, so nothing waits on its lock meanwhile.
        let _ = child.wait();
        moved();
        // Its process ended on its own: an End or a hand-over took the session out of the live ones first.
        if live().get(&named).is_some_and(|now| Arc::ptr_eq(now, &session)) {
            tell_failed(&session.state);
        }
    });
    moved();
    Ok(())
}

/// The end of the process `pid` on its session: only while it is still that session's process, looked at and marked under
/// the state lock, so a start that put another in its place keeps its own state, asks and pid. Its child, to reap.
fn reaped(session: &Session, pid: u32, named: &str) -> Option<Child> {
    let mut s = session.state.lock().unwrap_or_else(|e| e.into_inner());
    let child = {
        let mut held = session.child.lock().unwrap_or_else(|e| e.into_inner());
        if held.as_ref().is_some_and(|c| c.id() == pid) { held.take() } else { None }
    }?;
    (s.working, s.since, s.interrupted, s.exited_at) = (false, now_ms(), false, now_ms());
    // Nothing waits on them any more, so no card may offer them.
    for ask in std::mem::take(&mut s.pending) {
        note(named, &format!("{} went with the session, unanswered", ask["id"].as_str().unwrap_or("an ask")));
    }
    s.ended.get_or_insert_with(|| "the agent stopped".into());
    acknowledge(&mut s, None);
    Some(child)
}

/// How long an End or an orphan's end waits after SIGTERM before SIGKILL, in ms.
const GRACE_MS: u64 = 3_000;

/// Tells the process group `pgid` leads (a driven session's agent, which always leads its own, and what it runs) to
/// stop, and kills it after GRACE_MS; `wait` holds the caller until it is gone.
fn end_group(pgid: u32, wait: bool) {
    let Ok(pgid) = libc::pid_t::try_from(pgid) else { return };
    // SAFETY: killpg only signals the group; a group that is gone answers ESRCH.
    let signal = move |sig: libc::c_int| unsafe { libc::killpg(pgid, sig) } == 0;
    stop_then_kill(signal, wait);
}

/// SIGTERM through `signal`, then signal 0 until it fails for GRACE_MS, then SIGKILL; on a thread of its own unless `wait`.
fn stop_then_kill(signal: impl Fn(libc::c_int) -> bool + Send + 'static, wait: bool) {
    signal(libc::SIGTERM);
    let finish = move || {
        let started = std::time::Instant::now();
        while started.elapsed() < std::time::Duration::from_millis(GRACE_MS) {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if !signal(0) {
                return;
            }
        }
        signal(libc::SIGKILL);
    };
    match wait {
        true => finish(),
        false => {
            std::thread::spawn(finish);
        }
    }
}

/// A new ask is told the way a terminal's is: one notification a minute per session, from the hub only.
fn tell_asked(key: &str, state: &Mutex<State>, tool: &str) {
    static TOLD: Mutex<Vec<(String, u64)>> = Mutex::new(Vec::new());
    if TOUCH.get().is_none() {
        return;
    }
    let now = now_ms();
    let mut told = TOLD.lock().unwrap_or_else(|e| e.into_inner());
    told.retain(|(_, at)| now.saturating_sub(*at) < 60_000);
    if told.iter().any(|(k, _)| k == key) {
        return;
    }
    told.push((key.to_string(), now));
    let (title, name) = named(&state.lock().unwrap_or_else(|e| e.into_inner()));
    cockpit_core::notify::post(&format!("⏸ {title}"), &format!("{name} asks to run {tool}"), "", &[]);
}

/// The session's title (its first prompt's first line) and its agent's name, as its notifications say them.
fn named(state: &State) -> (String, String) {
    let title = state.messages.iter().find(|m| m["role"] == "user").and_then(|m| m["text"].as_str()).and_then(|t| t.lines().next()).unwrap_or("A session");
    (crate::gui::short(title, 60), agent::get(&state.agent).map(|a| a.name).unwrap_or_else(|| "The agent".into()))
}

/// Whether the session's failure is news: its last turn failed or its process ended, not told since its last prompt.
fn failure_is_news(state: &mut State) -> bool {
    if (state.failed.is_none() && state.ended.is_none()) || state.told_failure {
        return false;
    }
    state.told_failure = true;
    true
}

/// A failed turn, or a process that ended on its own, is told once as a terminal's failure is, from the hub only.
fn tell_failed(state: &Mutex<State>) {
    if TOUCH.get().is_none() {
        return;
    }
    let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
    if !failure_is_news(&mut s) {
        return;
    }
    let (title, name) = named(&s);
    drop(s);
    cockpit_core::notify::post(&format!("✗ {title}"), &format!("{name} stopped with an error"), "", &[]);
}

/// Every line the agent writes, read as bytes: one that is not UTF-8 is read lossily rather than ending the session.
fn lines(mut reader: impl BufRead) -> impl Iterator<Item = String> {
    std::iter::from_fn(move || {
        let mut bytes = Vec::new();
        match reader.read_until(b'\n', &mut bytes) {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(String::from_utf8_lossy(&bytes).trim_end_matches(['\n', '\r']).to_string()),
        }
    })
}

/// How long a session's log is kept after its last line.
const LOGS_FOR: std::time::Duration = std::time::Duration::from_secs(14 * 86_400);

/// Takes away the logs of sessions that ended a while ago.
fn prune_logs() {
    for entry in std::fs::read_dir(dir()).into_iter().flatten().flatten() {
        let old = entry.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > LOGS_FOR);
        if old && entry.path().extension().is_some_and(|e| e == "log") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Whether a session cockpit drives is in a turn or waits on an answer: the hub stays up for it. One at rest does not hold
/// it, since a hub that starts again brings it back paused.
pub fn driving() -> bool {
    live().values().any(|s| {
        let runs = s.child.lock().unwrap_or_else(|e| e.into_inner()).is_some();
        let state = s.state.lock().unwrap_or_else(|e| e.into_inner());
        runs && state.ended.is_none() && (state.working || !state.pending.is_empty())
    })
}

/// The sessions cockpit drives that still run, each with its process: whose tools a request comes from.
pub fn keyed_pids() -> Vec<(String, u32)> {
    let sessions: Vec<(String, Arc<Session>)> = live().iter().map(|(k, s)| (k.clone(), Arc::clone(s))).collect();
    sessions
        .iter()
        .filter(|(_, s)| s.state.lock().unwrap_or_else(|e| e.into_inner()).ended.is_none())
        .filter_map(|(k, s)| s.child.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|c| (k.clone(), c.id())))
        .collect()
}

/// What folding one event asks for besides the state: a line back to the agent, a line in the log.
#[derive(Debug, Default)]
struct Taken {
    reply: Option<Value>,
    note: Option<String>,
    /// A new ask, by the tool it asks for: the person is told.
    asked: Option<String>,
}

/// The id an ask is answered by: its tool call's, else its request's, else a fresh one; never one in use or settled.
fn ask_id(state: &State, request: &Value, request_id: &str) -> String {
    let taken = |id: &str| state.pending.iter().any(|p| p["id"] == id) || state.settled.iter().any(|s| s == id);
    let candidates = [request["tool_use_id"].as_str().unwrap_or_default(), request_id];
    if let Some(id) = candidates.into_iter().find(|id| !id.is_empty() && !taken(id)) {
        return id.to_string();
    }
    loop {
        let id = format!("p{}-{}", now_ms(), SEQ.fetch_add(1, Ordering::Relaxed));
        if !taken(&id) {
            return id;
        }
    }
}

fn settle_id(state: &mut State, id: &str) {
    state.settled.push_back(id.to_string());
    while state.settled.len() > SETTLED {
        state.settled.pop_front();
    }
}

/// How the turn a `result` event ends went: stopped by the person when an interrupt was sent, else failed or done as
/// the event says; None for any other event.
pub fn outcome(event: &Value, interrupted: bool) -> Option<&'static str> {
    (event["type"] == "result").then_some(match (interrupted, event["is_error"] == true) {
        (true, _) => "interrupted",
        (false, true) => "failed",
        (false, false) => "done",
    })
}

/// Marks what was sent as arrived: the message with these words (any, for None) stops waiting on its `echo`. True when
/// one was waiting.
fn acknowledge(state: &mut State, text: Option<&str>) -> bool {
    let waiting = |m: &Value| m["echo"] == true && text.is_none_or(|t| m["text"] == t);
    let Some(at) = state.messages.iter().position(waiting) else { return false };
    if text.is_some() {
        state.messages[at]["echo"] = Value::Null;
        return true;
    }
    for m in state.messages.iter_mut().filter(|m| m["echo"] == true) {
        m["echo"] = Value::Null;
    }
    true
}

/// The end of a turn: words sent before it no longer wait; words sent during it wait for their own replay in the next
/// turn, and the end of that one.
fn settle_echoes(state: &mut State) {
    for m in state.messages.iter_mut().filter(|m| m["echo"] == true) {
        if m["later"] == true {
            m["later"] = Value::Null;
        } else {
            m["echo"] = Value::Null;
        }
    }
}

/// One event from the agent, folded into the state.
fn take(state: &mut State, event: &Value) -> Taken {
    let at = chrono::Utc::now().to_rfc3339();
    let mut taken = Taken::default();
    // A subagent's own lines come down the same pipe, marked with the call that started it: they are its conversation,
    // never the parent's, and only say what it is on.
    if let Some(parent) = event["parent_tool_use_id"].as_str().filter(|p| !p.is_empty()) {
        task_progress(state, parent, event);
        return taken;
    }
    match event["type"].as_str().unwrap_or_default() {
        "system" => {
            match event["subtype"].as_str().unwrap_or_default() {
                "init" => {
                    if let Some(id) = event["session_id"].as_str().filter(|id| !id.is_empty()) {
                        state.session_id = id.to_string();
                    }
                    if let Some(cwd) = event["cwd"].as_str() {
                        state.cwd = cwd.to_string();
                    }
                    if let Some(model) = event["model"].as_str() {
                        state.model = model.to_string();
                    }
                    // Each turn's init says the effort level its next message is sent with, and how fast mode stands.
                    if let Some(effort) = event["effort"].as_str().filter(|e| !e.is_empty()) {
                        state.effort = effort.to_string();
                    }
                    fast_reported(state, event);
                    // The names the agent opens with. Some sessions never publish commands_changed, so this is the only
                    // list they get.
                    let named: Vec<Value> = event["slash_commands"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|c| c.as_str())
                        .map(|name| json!({"name": name, "description": ""}))
                        .collect();
                    if !named.is_empty() {
                        state.commands = named;
                        cockpit_core::commands::describe(&state.agent, &state.cwd, &mut state.commands);
                    }
                }
                "commands_changed" => {
                    let sent = event["commands"].as_array().cloned().unwrap_or_default();
                    if !sent.is_empty() {
                        state.commands = sent;
                        cockpit_core::commands::describe(&state.agent, &state.cwd, &mut state.commands);
                    }
                }
                // The tasks a turn starts beside itself: each is told when it starts, moves, ends and is reported, and a
                // snapshot of those still running comes after each change.
                "task_started" => {
                    let id = event["task_id"].as_str().unwrap_or_default();
                    let tool_use_id = event["tool_use_id"].as_str().unwrap_or_default();
                    let kind = task_kind_word(event);
                    let name = event["description"].as_str().or(event["subagent_type"].as_str()).unwrap_or_default();
                    match state.tasks.iter_mut().find(|t| (!tool_use_id.is_empty() && t["tool_use_id"] == tool_use_id) || t["id"] == id) {
                    Some(task) => {
                        task["id"] = json!(id);
                        if !name.is_empty() {
                            task["name"] = json!(name);
                        }
                        task["kind"] = json!(kind);
                    }
                    None => state.tasks.push(json!({"id": id, "kind": kind, "name": name, "state": "running", "started": at, "ended": null, "summary": null, "tool_use_id": tool_use_id})),
                }
                }
                "task_progress" => {
                    let id = event["task_id"].as_str().unwrap_or_default();
                    if let Some(task) = state.tasks.iter_mut().find(|t| t["id"] == id) {
                        task["progress"] = event["description"].clone();
                        task["last_tool"] = event["last_tool_name"].clone();
                    }
                }
                "task_updated" => {
                    let id = event["task_id"].as_str().unwrap_or_default();
                    if let Some(status) = event["patch"]["status"].as_str()
                        && let Some(task) = state.tasks.iter_mut().find(|t| t["id"] == id)
                    {
                        task_ended(task, status, &at);
                    }
                }
                "task_notification" => {
                    let id = event["task_id"].as_str().unwrap_or_default();
                    if let Some(task) = state.tasks.iter_mut().find(|t| t["id"] == id) {
                        task_ended(task, event["status"].as_str().unwrap_or("completed"), &at);
                        if let Some(summary) = event["summary"].as_str().filter(|s| !s.trim().is_empty()) {
                            task["summary"] = json!(summary);
                        }
                    }
                }
                "background_tasks_changed" => {
                    let listed = event["tasks"].as_array().cloned().unwrap_or_default();
                    // A task still known by its call alone is not named yet: the snapshot cannot say it is over.
                    let named = |t: &Value| t["id"] != t["tool_use_id"];
                    for task in state.tasks.iter_mut().filter(|t| t["state"] == "running" && named(t)) {
                        if !listed.iter().any(|l| l["task_id"] == task["id"]) {
                            task_ended(task, "completed", &at);
                        }
                    }
                    for l in listed {
                        let id = l["task_id"].as_str().unwrap_or_default();
                        if id.is_empty() || state.tasks.iter().any(|t| t["id"] == id) {
                            continue;
                        }
                        let kind = task_kind_word(&l);
                        let name = l["description"].as_str().or(l["subagent_type"].as_str()).unwrap_or_default();
                        // The call that started it is there unnamed: this is its id.
                        match state.tasks.iter_mut().find(|t| t["state"] == "running" && !named(t) && t["kind"] == kind && (name.is_empty() || t["name"] == name)) {
                        Some(task) => task["id"] = json!(id),
                        None => state.tasks.push(json!({"id": id, "kind": kind, "name": name, "state": "running", "started": at, "ended": null, "summary": null, "tool_use_id": ""})),
                    }
                    }
                }
                _ => {}
            }
        }
        "rate_limit_event" => {
            state.rate_limit = event["rate_limit_info"].clone();
            state.rate_limit_at = now_ms();
        }
        // The agent asks before a tool runs and waits for the control response naming this request.
        "control_request" => {
            let request = &event["request"];
            let request_id = event["request_id"].as_str().unwrap_or_default();
            match request["subtype"].as_str().unwrap_or_default() {
                "can_use_tool" if !request_id.is_empty() => {
                    let id = ask_id(state, request, request_id);
                    let tool = request["tool_name"].as_str().unwrap_or("a tool");
                    taken.note = Some(format!("asked {id} for {tool}: {}", arg_line(&request["input"])));
                    taken.asked = Some(tool.to_string());
                    state.pending.push(json!({
                        "id": id, "request_id": request_id, "at": now_ms(),
                        "tool_name": request["tool_name"].clone(), "input": request["input"].clone(),
                        "tool_use_id": request["tool_use_id"].clone(), "description": request["description"].clone(),
                    }));
                }
                // Anything else it could ask is refused at once rather than left hanging.
                other => {
                    taken.note = Some(format!("refused a control request {other:?} cockpit does not answer"));
                    taken.reply = Some(json!({"type": "control_response", "response": {
                        "subtype": "error", "request_id": request_id, "error": format!("cockpit does not answer {other}")}}));
                }
            }
        }
        // An interrupt takes back the ask it was waiting on.
        "control_cancel_request" => {
            let request_id = event["request_id"].as_str().unwrap_or_default();
            let gone: Vec<Value> = state.pending.iter().filter(|p| p["request_id"] == request_id).cloned().collect();
            state.pending.retain(|p| p["request_id"] != request_id);
            let ids: Vec<String> = gone.iter().map(|ask| ask["id"].as_str().unwrap_or_default().to_string()).collect();
            for id in &ids {
                settle_id(state, id);
            }
            if !ids.is_empty() {
                taken.note = Some(format!("the agent took back {}", ids.join(", ")));
            }
        }
        // The agent confirms a control request by answering it; the mode it reports back is the one that took.
        "control_response" => {
            let request_id = event["response"]["request_id"].as_str().unwrap_or_default();
            let ok = event["response"]["subtype"] == "success";
            match state.asked.remove(request_id) {
                Some(Asked::Catalog) if ok => state.models = agent::catalog(&state.agent, &event["response"]["response"]).iter().map(Offer::json).collect(),
                // What it reported of fast mode was about the model and the setting before: the next report says again.
                Some(Asked::Set(setting, value)) if ok => match setting {
                    Setting::Effort => state.effort = value.as_str().unwrap_or_default().to_string(),
                    Setting::Fast => (state.fast, state.fast_on, state.fast_why) = (value.as_bool(), None, None),
                    // Its answer names the model it took, read below.
                    Setting::Model => (state.fast_on, state.fast_why) = (None, None),
                },
                Some(Asked::Set(setting, value)) => {
                    let why = event["response"]["error"].as_str().unwrap_or("no reason given");
                    taken.note = Some(format!("the agent did not take {} {value}: {why}", setting.id()));
                }
                _ => {}
            }
            if let Some(mode) = event["response"]["response"]["mode"].as_str() {
                state.mode = mode.to_string();
            }
            // Only a success names the model it took; an error names the one it kept, under a different key.
            if let Some(model) = event["response"]["response"]["model"].as_str() {
                state.model = model.to_string();
            }
            if event["response"]["response"]["still_queued"].is_array() {
                state.working = false;
            }
        }
        "assistant" => {
            if !state.working {
                state.since = now_ms();
            }
            state.working = true;
            for block in event["message"]["content"].as_array().into_iter().flatten() {
                match block["type"].as_str().unwrap_or_default() {
                    "text" => {
                        let text = block["text"].as_str().unwrap_or_default();
                        if !text.trim().is_empty() {
                            state.messages.push(json!({"role": "assistant", "at": at, "text": text, "tool": null}));
                        }
                    }
                    "tool_use" => {
                        let name = block["name"].as_str().unwrap_or_default();
                        let summary = arg_line(&block["input"]);
                        let id = block["id"].as_str().unwrap_or_default();
                        let tool = json!({"id": id, "name": name, "summary": summary, "ok": null});
                        if !id.is_empty() {
                            state.calls.insert(id.to_string(), state.messages.len());
                            task_of_call(state, id, name, &block["input"], &summary, &at);
                        }
                        state.messages.push(json!({"role": "assistant", "at": at, "text": "", "tool": tool}));
                    }
                    _ => {}
                }
            }
        }
        "user" => {
            for block in event["message"]["content"].as_array().into_iter().flatten() {
                match block["type"].as_str().unwrap_or_default() {
                    "text" => {
                        let text = block["text"].as_str().unwrap_or_default();
                        // The agent replays what it was sent: the words already there stop waiting rather than repeat.
                        if !text.trim().is_empty() && !acknowledge(state, Some(text)) {
                            state.messages.push(json!({"role": "user", "at": at, "text": text, "tool": null}));
                        }
                    }
                    "tool_result" => {
                        let ok = block["is_error"] != true;
                        if let Some(id) = block["tool_use_id"].as_str() {
                            // The call is over, however it went: an ask still waiting on it is one nobody can answer.
                            let gone: Vec<String> =
                                state.pending.iter().filter(|p| p["tool_use_id"] == id).filter_map(|p| p["id"].as_str().map(str::to_string)).collect();
                            state.pending.retain(|p| p["tool_use_id"] != id);
                            for g in gone {
                                settle_id(state, &g);
                            }
                            if let Some(at) = state.calls.get(id).copied()
                                && let Some(m) = state.messages.get_mut(at)
                            {
                                m["tool"]["ok"] = json!(ok);
                            }
                            task_result(state, id, ok, block, &event["tool_use_result"], &at);
                        }
                    }
                    _ => {}
                }
            }
        }
        "result" => {
            state.working = false;
            state.since = now_ms();
            // A turn that ended took what was sent before it, replayed or not; what was sent during it waits for its replay.
            settle_echoes(state);
            state.cost = event["total_cost_usd"].as_f64().or(state.cost);
            state.usage = event["usage"].clone();
            fast_reported(state, event);
            // A turn that failed says why in `result` as well as in its words, so this records it without saying it twice.
            // An interrupted turn reports the same error and is not one: the session takes the next prompt.
            let interrupted = std::mem::take(&mut state.interrupted);
            state.stopped = interrupted;
            state.failed = (event["is_error"] == true && !interrupted).then(|| event["result"].as_str().unwrap_or("the turn failed").to_string());
        }
        _ => {}
    }
    taken
}

/// A task's kind as the views word it, from a stream event's `task_type`.
fn task_kind_word(event: &Value) -> &'static str {
    match event["task_type"].as_str().unwrap_or_default() {
        "local_bash" => "background",
        "workflow" => "workflow",
        _ => "subagent",
    }
}

/// A task over, as its status says: completed is done, anything else failed.
fn task_ended(task: &mut Value, status: &str, at: &str) {
    if task["state"] == "running" {
        task["state"] = json!(if status == "completed" { TaskState::Done.word() } else { TaskState::Failed.word() });
        task["ended"] = json!(at);
    }
}

/// Starts the task a call begins, when its agent's table names the tool.
fn task_of_call(state: &mut State, id: &str, name: &str, input: &Value, summary: &str, at: &str) {
    let Some(table) = agent::get(&state.agent).and_then(|a| a.tasks) else { return };
    let background = input["run_in_background"] == true;
    let Some((kind, title)) = call_task(&table, name, input["description"].as_str(), input["subagent_type"].as_str(), background, summary) else { return };
    state
        .tasks
        .push(json!({"id": id, "kind": kind.word(), "name": title, "state": "running", "started": at, "ended": null, "summary": null, "tool_use_id": id}));
}

/// How a call's result moves its task: a subagent or job launched gets its id and stays running, anything else is over.
fn task_result(state: &mut State, id: &str, ok: bool, block: &Value, result: &Value, at: &str) {
    let Some(task) = state.tasks.iter_mut().find(|t| t["tool_use_id"] == id) else { return };
    let head: String = match &block["content"] {
        Value::String(s) => s.chars().take(200).collect(),
        Value::Array(blocks) => blocks.iter().filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join(" ").chars().take(200).collect(),
        _ => String::new(),
    };
    let launched = match task["kind"].as_str() {
        Some("background") => result["backgroundTaskId"].as_str().map(str::to_string).or_else(|| background_id(&head)),
        _ => result["agentId"].as_str().filter(|_| result["isAsync"] == true).map(str::to_string),
    };
    match launched {
        Some(launched) => task["id"] = json!(launched),
        None => task_ended(task, if ok { "completed" } else { "failed" }, at),
    }
}

/// What a subagent's own line says of the task it works for: its last call.
fn task_progress(state: &mut State, parent: &str, event: &Value) {
    let Some(task) = state.tasks.iter_mut().find(|t| t["tool_use_id"] == parent) else { return };
    for block in event["message"]["content"].as_array().into_iter().flatten() {
        if block["type"] == "tool_use" {
            task["last_tool"] = block["name"].clone();
            task["progress"] = json!(arg_line(&block["input"]));
        }
    }
}

/// Takes the ask `id` out of the pending ones and words the control response that answers it; an id that is not
/// pending, because it was answered, taken back or never asked, is refused.
fn settle(state: &mut State, id: &str, allow: bool) -> Result<(Value, Value), String> {
    let at = state.pending.iter().position(|p| p["id"] == id).ok_or(CHANGED)?;
    let ask = state.pending.remove(at);
    settle_id(state, id);
    // An allow carries the arguments back unchanged, taken from the ask rather than from whoever answers.
    let decision = match allow {
        true => json!({"behavior": "allow", "updatedInput": ask["input"].clone()}),
        false => json!({"behavior": "deny", "message": "The person said no to this."}),
    };
    let reply = json!({"type": "control_response", "response": {"subtype": "success", "request_id": ask["request_id"].clone(), "response": decision}});
    Ok((reply, ask))
}

/// Answers the ask `id` of session `key`, as `source` (`<view>:<click|key|command>`) chose; logged either way.
pub fn answer(key: &str, id: &str, allow: bool, source: &str) -> Result<String, String> {
    if let Err(why) = crate::gui::by_a_person(crate::gui::BY_A_PERSON) {
        note(key, &format!("refused an answer to {id} from {source}: {why}"));
        return Err(why);
    }
    let Some(session) = live().get(key).cloned() else {
        note(key, &format!("refused an answer to {id} from {source}: no such session"));
        return Err(CHANGED.into());
    };
    let settled = settle(&mut session.state.lock().unwrap_or_else(|e| e.into_inner()), id, allow);
    let (reply, ask) = match settled {
        Ok(done) => done,
        Err(why) => {
            note(key, &format!("refused an answer to {id} from {source}: {why}"));
            // The card that sent it was drawn from an older view: the views are built again so it redraws.
            moved();
            return Err(why);
        }
    };
    let verdict = if allow { "allowed" } else { "denied" };
    let tool = ask["tool_name"].as_str().unwrap_or("a tool").to_string();
    let sent = match !session.demo {
        true => write_line(&session.stdin, &reply),
        false => {
            demo_answered(&mut session.state.lock().unwrap_or_else(|e| e.into_inner()), &ask, allow);
            Ok(())
        }
    };
    note(key, &format!("{id} {verdict} ({tool}) from {source}{}", sent.as_ref().err().map(|e| format!(", not delivered: {e}")).unwrap_or_default()));
    moved();
    sent.map(|()| verdict.to_string())
}

/// The session that has the ask `id` waiting.
pub fn asking(id: &str) -> Option<String> {
    let owners: Vec<String> = live()
        .iter()
        .filter(|(_, s)| s.state.lock().unwrap_or_else(|e| e.into_inner()).pending.iter().any(|p| p["id"] == id))
        .map(|(k, _)| k.clone())
        .collect();
    // Two sessions waiting on the same id: which one the answer meant cannot be told.
    (owners.len() == 1).then(|| owners[0].clone())
}

/// A control request down the same pipe the words go: the agent answers it without asking the model.
pub fn control(key: &str, request: Value) -> Result<(), String> {
    let session = live().get(key).cloned().ok_or("no such session")?;
    if session.demo {
        // A demo session takes it as the agent would have answered.
        let mut state = session.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(mode) = request["mode"].as_str() {
            state.mode = mode.to_string();
        }
        if let Some(id) = request["task_id"].as_str() {
            let at = chrono::Utc::now().to_rfc3339();
            for task in state.tasks.iter_mut().filter(|t| t["id"] == id) {
                task_ended(task, "killed", &at);
            }
        }
        drop(state);
        moved();
        return Ok(());
    }
    let lock = launching(key);
    let one = lock.lock().unwrap_or_else(|e| e.into_inner());
    if session.child.lock().unwrap_or_else(|e| e.into_inner()).is_none() {
        // Nothing runs, as after a restart: a mode or a model waits for the next start, set under the launch lock so a
        // start never takes the old one while the view shows the new; an interrupt has nothing to stop.
        let mut state = session.state.lock().unwrap_or_else(|e| e.into_inner());
        if request["subtype"] == "interrupt" {
            return Err("it is not running: there is no turn to stop".into());
        }
        if request["subtype"] == "stop_task" {
            return Err("it is not running: there is no task to stop".into());
        }
        let found = agent::get(&state.agent).ok_or("an agent cockpit does not know")?;
        if let Some(mode) = request["mode"].as_str() {
            found.mode_argv(mode).ok_or_else(|| format!("{} takes no mode {mode}", found.name))?;
            state.mode = mode.to_string();
        }
        if let Some(model) = request["model"].as_str() {
            if !found.takes_model(model) {
                return Err(format!("{} takes no model {model}", found.name));
            }
            state.model = model.to_string();
        }
        drop(state);
        drop(one);
        moved();
        return Ok(());
    }
    drop(one);
    if request["subtype"] == "interrupt" {
        session.state.lock().unwrap_or_else(|e| e.into_inner()).interrupted = true;
    }
    let line = json!({"type": "control_request", "request_id": format!("c{}-{}", now_ms(), SEQ.fetch_add(1, Ordering::Relaxed)), "request": request});
    write_line(&session.stdin, &line)
}

/// What a pick answers when it waits for the session's next message.
pub const NEXT: &str = "from your next message";

/// A model, an effort level or fast mode (a bool) for session `key`: sent over its stream at once where its agent takes
/// that setting live and its process runs, else kept for the start its next message makes. Answers "set", or NEXT when
/// it waits.
pub fn pick(key: &str, setting: Setting, value: &Value) -> Result<String, String> {
    let session = live().get(key).cloned().ok_or("no such session")?;
    let lock = launching(key);
    let _one = lock.lock().unwrap_or_else(|e| e.into_inner());
    let agent_id = session.state.lock().unwrap_or_else(|e| e.into_inner()).agent.clone();
    let found = agent::get(&agent_id).ok_or("an agent cockpit does not know")?;
    let word = taken_value(&found, setting, value)?;
    let runs = !session.demo && session.child.lock().unwrap_or_else(|e| e.into_inner()).as_mut().is_some_and(|c| c.try_wait().is_ok_and(|s| s.is_none()));
    if session.demo || !runs || !found.live.contains(&setting) {
        let mut state = session.state.lock().unwrap_or_else(|e| e.into_inner());
        set(&mut state, setting, value);
        // A process that runs takes it only at its next start.
        state.restart |= runs;
        drop(state);
        note(key, &format!("{} {word} {}", setting.id(), if session.demo { "set" } else { "kept for its next start" }));
        moved();
        return Ok(if session.demo { "set".into() } else { NEXT.into() });
    }
    let request = match setting {
        Setting::Model => json!({"subtype": "set_model", "model": word}),
        Setting::Effort => json!({"subtype": "apply_flag_settings", "settings": {"effortLevel": word}}),
        Setting::Fast => json!({"subtype": "apply_flag_settings", "settings": {"fastMode": value}}),
    };
    ask(&session, request, Asked::Set(setting, value.clone()))?;
    note(key, &format!("asked it for {} {word}", setting.id()));
    Ok("set".into())
}

/// The value as words for the log, refused when the agent does not take it.
fn taken_value(found: &Agent, setting: Setting, value: &Value) -> Result<String, String> {
    match setting {
        Setting::Model => {
            let model = value.as_str().filter(|m| !m.is_empty()).ok_or("which model? pass {model}")?;
            found.takes_model(model).then(|| model.to_string()).ok_or_else(|| format!("{} takes no model {model}", found.name))
        }
        Setting::Effort => {
            let effort = value.as_str().filter(|e| !e.is_empty()).ok_or("which effort level? pass {effort}")?;
            found.takes_effort(effort).then(|| effort.to_string()).ok_or_else(|| format!("{} takes no effort level {effort}", found.name))
        }
        Setting::Fast => {
            let on = value.as_bool().ok_or("fast mode on or off? pass {fast: true} or {fast: false}")?;
            let has = found.fast.is_some() || found.live.contains(&Setting::Fast);
            has.then(|| if on { "on" } else { "off" }.to_string()).ok_or_else(|| format!("{} has no fast mode", found.name))
        }
    }
}

/// The setting as its next start passes it on.
fn set(state: &mut State, setting: Setting, value: &Value) {
    let text = value.as_str().unwrap_or_default().to_string();
    match setting {
        Setting::Model => state.model = text,
        Setting::Effort => state.effort = text,
        // What it reported serving waits for its next report.
        Setting::Fast => (state.fast, state.fast_on, state.fast_why) = (value.as_bool(), None, None),
    }
}

/// A control request down the session's stdin, remembered so its answer is read for what it asked.
fn ask(session: &Session, request: Value, asked: Asked) -> Result<(), String> {
    let id = format!("c{}-{}", now_ms(), SEQ.fetch_add(1, Ordering::Relaxed));
    {
        let mut state = session.state.lock().unwrap_or_else(|e| e.into_inner());
        // Requests the agent never answered are not kept for ever.
        if state.asked.len() >= 64 {
            state.asked.clear();
        }
        state.asked.insert(id.clone(), asked);
    }
    write_line(&session.stdin, &json!({"type": "control_request", "request_id": id, "request": request})).inspect_err(|_| {
        session.state.lock().unwrap_or_else(|e| e.into_inner()).asked.remove(&id);
    })
}

/// How fast mode stands, as an init or a result says: serving (on, or cooling down after a limit) or not, and why not.
fn fast_reported(state: &mut State, event: &Value) {
    if let Some(now) = event["fast_mode_state"].as_str() {
        state.fast_on = Some(now != "off");
        state.fast_why = event["fast_mode_disabled_reason"].as_str().filter(|_| now == "off").map(str::to_string);
    }
}

/// How long a stop sent to a session refuses another while its result has not come.
const INTERRUPT_HOLDS_MS: u64 = 5_000;

/// Stops the session's turn: refused when it is not in one, or when a stop sent less than INTERRUPT_HOLDS_MS ago waits
/// on its result.
pub fn interrupt(key: &str) -> Result<(), String> {
    let session = live().get(key).cloned().ok_or("no such session")?;
    {
        // Looked at and marked under one lock, so two stops never both go.
        let mut state = session.state.lock().unwrap_or_else(|e| e.into_inner());
        if !state.working {
            return Err("it is not working: there is no turn to stop".into());
        }
        if state.interrupted && now_ms().saturating_sub(state.interrupted_at) < INTERRUPT_HOLDS_MS {
            return Err("it is being stopped already".into());
        }
        (state.interrupted, state.interrupted_at) = (!session.demo, now_ms());
    }
    let line =
        json!({"type": "control_request", "request_id": format!("c{}-{}", now_ms(), SEQ.fetch_add(1, Ordering::Relaxed)), "request": {"subtype": "interrupt"}});
    match session.demo {
        true => Ok(()),
        false => write_line(&session.stdin, &line).inspect_err(|_| session.state.lock().unwrap_or_else(|e| e.into_inner()).interrupted = false),
    }
}

/// Stops one task of session `key` (a subagent, a background job) through its stream: the agent ends it and reports
/// it killed.
pub fn stop_task(key: &str, task_id: &str) -> Result<String, String> {
    if task_id.is_empty() {
        return Err("which task? pass {task}".into());
    }
    control(key, json!({"subtype": "stop_task", "task_id": task_id}))?;
    note(key, &format!("asked it to stop task {task_id}"));
    Ok("asked it to stop the task".into())
}

/// The person's words, as the protocol takes them; a session whose agent is not running (paused by a restart, ended
/// on its own, or just exited) is picked up first.
pub fn send(key: &str, text: &str) -> Result<(), String> {
    if let Some(done) = slash(key, text) {
        return done;
    }
    let session = live().get(key).cloned().ok_or("no such session")?;
    let lock = launching(key);
    let _one = lock.lock().unwrap_or_else(|e| e.into_inner());
    // An End while it waited for the lock wins: nothing is picked up again.
    if !live().get(key).is_some_and(|now| Arc::ptr_eq(now, &session)) {
        return Err(GONE.into());
    }
    let gone = !session.demo && session.child.lock().unwrap_or_else(|e| e.into_inner()).as_mut().is_none_or(|c| c.try_wait().is_ok_and(|s| s.is_some()));
    // A setting its agent takes only at a start waits for a message sent at rest, so no turn is cut short.
    let restart = !gone && !session.demo && {
        let s = session.state.lock().unwrap_or_else(|e| e.into_inner());
        s.restart && !s.working && s.pending.is_empty()
    };
    if gone {
        resume(key, &session)?;
    } else if restart {
        relaunch(key, &session)?;
    }
    let line = json!({"type": "user", "message": {"role": "user", "content": [{"type": "text", "text": text}]}});
    if !session.demo {
        write_line(&session.stdin, &line)?;
    }
    let mut state = session.state.lock().unwrap_or_else(|e| e.into_inner());
    // Sent, waiting for the agent to replay it (`echo`); the demo's sessions have nothing to replay. Words sent while a
    // turn runs (`later`) are the next turn's: the end of this one says nothing about them.
    let echo = if session.demo { Value::Null } else { json!(true) };
    let later = if state.working && !session.demo { json!(true) } else { Value::Null };
    state.messages.push(json!({"role": "user", "at": chrono::Utc::now().to_rfc3339(), "text": text, "tool": null, "echo": echo, "later": later}));
    state.working = true;
    // A new turn: the last one's failure or stop is behind it.
    (state.failed, state.stopped, state.since, state.told_failure) = (None, false, now_ms(), false);
    drop(state);
    moved();
    Ok(())
}

/// Its process ended and started again on the same conversation, so a setting its agent takes only at a start takes
/// effect; called at rest, with its launch lock held. Taken out of the session first, the old process is no failure.
fn relaunch(key: &str, session: &Arc<Session>) -> Result<(), String> {
    let child = session.child.lock().unwrap_or_else(|e| e.into_inner()).take();
    session.stdin.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(mut child) = child {
        end_group(child.id(), true);
        let _ = child.wait();
    }
    {
        let mut s = session.state.lock().unwrap_or_else(|e| e.into_inner());
        (s.restart, s.exited_at) = (false, now_ms());
    }
    note(key, "started again: a setting it takes only at a start changed");
    resume(key, session)
}

/// Words for a session cockpit drives, as `deliver` hands them over: None when `handle` names none. In the hub the
/// session takes them, unless it waits on a permission; elsewhere the hub is asked to.
pub fn deliver_to(handle: &str, text: &str) -> Option<Result<(), String>> {
    // Taken out first: the lock on the sessions must be let go before `send` takes it again.
    let found = live().get(handle).cloned();
    if let Some(session) = found {
        let state = session.state.lock().unwrap_or_else(|e| e.into_inner());
        let name = agent::get(&state.agent).map(|a| a.name).unwrap_or_else(|| "The agent".into());
        let asking = !state.pending.is_empty();
        drop(state);
        return Some(if asking { Err(cockpit_core::deliver::waiting_note(&name)) } else { send(handle, text) });
    }
    if !driven_key(handle) {
        return None;
    }
    let call = json!({"op": "act", "app": "session", "action": "deliver", "args": [{"key": handle, "text": text}]});
    Some(cockpit_core::hub::request(&call, std::time::Duration::from_secs(20)).and_then(|answer| match answer["error"].as_str().filter(|e| !e.is_empty()) {
        Some(why) => Err(why.trim_start_matches("deliver: ").to_string()),
        None => Ok(()),
    }))
}

/// A slash command sent to a driven session: a setting cockpit applies itself (`/model opus`), a command only a
/// terminal can show (refused, with the way out), one the agent lists (None: it goes as words), or none it knows.
fn slash(key: &str, text: &str) -> Option<Result<(), String>> {
    let line = text.trim();
    let rest = line.strip_prefix('/')?;
    if rest.is_empty() || rest.starts_with(char::is_whitespace) || line.contains('\n') {
        return None;
    }
    let (name, arg) = rest.split_once(char::is_whitespace).map(|(n, a)| (n, a.trim())).unwrap_or((rest, ""));
    let session = live().get(key).cloned()?;
    let (agent_id, commands) = {
        let state = session.state.lock().unwrap_or_else(|e| e.into_inner());
        (state.agent.clone(), state.commands.clone())
    };
    let agent_name = agent::get(&agent_id).map(|a| a.name).unwrap_or_else(|| "The agent".into());
    if let Some(setting) = agent::slash_setting(&agent_id, name) {
        let value = match setting {
            Setting::Fast => match arg {
                "on" | "true" | "1" => json!(true),
                "off" | "false" | "0" => json!(false),
                _ => return Some(Err(format!("/{name} on or off: pick it from the chip, or say which"))),
            },
            _ if arg.is_empty() => return Some(Err(format!("/{name} alone opens a picker only a terminal shows: pick it from the chip, or say which"))),
            _ => json!(arg),
        };
        return Some(pick(key, setting, &value).map(|_| ()));
    }
    if agent::slash_needs_terminal(&agent_id, name) {
        return Some(Err(format!("/{name} opens a dialog only {agent_name}'s terminal shows: hand the session over to a terminal first")));
    }
    if commands.iter().any(|c| c["name"].as_str().is_some_and(|n| n == name || n.rsplit(':').next() == Some(name))) {
        return None;
    }
    Some(Err(format!("this session has no /{name}")))
}

/// A session key: an agent's id, a colon, then its start (`claude:1794…`); a herdr pane id (`w9:p1`) is not one.
fn driven_key(handle: &str) -> bool {
    handle.split_once(':').is_some_and(|(id, rest)| !rest.is_empty() && !handle.starts_with(cockpit_core::frontend::APP) && agent::get(id).is_some())
}

/// Ends a session and forgets it: its stdin closes, its process group is told to stop and killed after a grace period,
/// and its queue goes with it.
pub fn stop(key: &str) {
    let Some(session) = live().remove(key) else { return };
    locks().remove(key);
    session.stdin.lock().unwrap_or_else(|e| e.into_inner()).take();
    let pid = session.child.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(Child::id);
    if let Some(pid) = pid {
        end_group(pid, false);
    }
    note(key, "stopped");
    moved();
    crate::gui::turn_ended(key, "ended");
}

/// Ends it as `stop` does, and returns once its process group is gone: the agent's own resume then has the session alone.
fn stop_and_wait(key: &str) {
    let pid = live().get(key).and_then(|s| s.child.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(Child::id));
    stop(key);
    if let Some(pid) = pid {
        end_group(pid, true);
    }
}

/// End: refused while it works unless `confirm`ed, looked at and done under its launch lock so no turn starts between.
pub fn end(key: &str, confirm: bool) -> Result<(), String> {
    let lock = launching(key);
    let _one = lock.lock().unwrap_or_else(|e| e.into_inner());
    if !confirm && summary(key).is_some_and(|s| s["working"] == true) {
        return Err("it is working: ending it stops this turn too, pass {confirm: true}".into());
    }
    stop(key);
    Ok(())
}

/// One session as the hub serves it, every message included.
pub fn snapshot(key: &str) -> Option<Value> {
    let session = live().get(key).cloned()?;
    let state = session.state.lock().unwrap_or_else(|e| e.into_inner());
    let mut out = state.json();
    out["key"] = json!(key);
    out["messages"] = json!(state.messages);
    Some(out)
}

/// One session without its messages: what the agents view, the queue and the usage read.
pub fn summary(key: &str) -> Option<Value> {
    let session = live().get(key).cloned()?;
    let mut out = session.state.lock().unwrap_or_else(|e| e.into_inner()).json();
    out["key"] = json!(key);
    Some(out)
}

/// How many of a session's latest messages the view sends unless asked for more.
pub const PAGE: usize = 400;
/// However many are asked for, one page holds no more.
const PAGE_CAP: usize = 20_000;

/// One session with a page of its messages: the `limit` latest before message `before` (else the latest), and where
/// the page sits, `cursor: {from, to, total, more}`, `more` saying older messages come before it.
pub fn page(key: &str, limit: usize, before: Option<usize>) -> Option<Value> {
    let session = live().get(key).cloned()?;
    let state = session.state.lock().unwrap_or_else(|e| e.into_inner());
    let total = state.messages.len();
    let to = before.unwrap_or(total).min(total);
    let from = to.saturating_sub(limit.clamp(1, PAGE_CAP));
    let mut out = state.json();
    out["key"] = json!(key);
    out["messages"] = json!(state.messages[from..to]);
    out["cursor"] = json!({"from": from, "to": to, "total": total, "more": from > 0});
    Some(out)
}

/// Every live session, the most recently started first, whichever agent runs it.
pub fn keys() -> Vec<String> {
    // The sessions are taken out first: each state is locked with the list of sessions let go.
    let all: Vec<(String, Arc<Session>)> = live().iter().map(|(k, s)| (k.clone(), Arc::clone(s))).collect();
    let mut started: Vec<(u64, String)> = all.into_iter().map(|(k, s)| (s.state.lock().unwrap_or_else(|e| e.into_inner()).started, k)).collect();
    started.sort_by(|a, b| b.cmp(a));
    started.into_iter().map(|(_, k)| k).collect()
}

/// The session handed over to a terminal of its own: stopped here first, so one process writes it, then resumed with
/// its agent's own command line in a space of its folder (the one open under its name, else a new one).
pub fn open_in_terminal(key: &str, confirmed: bool, frontend: Option<cockpit_core::frontend::Frontend>) -> Result<String, String> {
    // Looked at and ended under its launch lock, so no turn starts between.
    let lock = launching(key);
    let _one = lock.lock().unwrap_or_else(|e| e.into_inner());
    let s = summary(key).ok_or("no such session")?;
    at_rest(key, &s)?;
    let cwd = s["cwd"].as_str().filter(|c| !c.is_empty()).ok_or("the session has no folder to open it in")?;
    let found = agent::get(s["agent"].as_str().unwrap_or("claude")).ok_or("an agent cockpit does not know")?;
    let id = s["session"].as_str().unwrap_or_default();
    let line = cockpit_core::resume::resume_line(cwd, &found, id).ok_or_else(|| format!("{} cannot resume a session", found.name))?;
    if !confirmed {
        return Err("it leaves cockpit's chat for a terminal: pass {confirm: true}".into());
    }
    let label = format!("↩ {}", s["title"].as_str().map(|t| crate::gui::short(t, 40)).unwrap_or_else(|| found.name.clone()));
    cockpit_core::frontend::with(frontend, || {
        // Its terminal first, then the session ended here, then the line: never a session ended with nowhere to go on.
        // The line runs from a script file, so whichever shell the terminal opens reads it the same.
        let script = handover_script(key, &line).ok_or("its resume line could not be written down: it goes on here")?;
        let pane = cockpit_core::frontend::new_space(cwd, &label).ok_or("no terminal opened for it: it goes on here")?;
        stop_and_wait(key);
        if !cockpit_core::frontend::run_in(&pane, &format!("sh '{}'", script.display())) {
            return Err(format!("it was ended here, but its terminal took no command: run `{line}` there"));
        }
        note(key, "handed over to a terminal");
        Ok(format!("it goes on in a terminal: {line}"))
    })
}

/// The resume line in a script of its own under the cache, its path free of quotes: a new file of a name no other
/// process could have made first (never one already there, nor a link), the person's alone; a week's are kept.
fn handover_script(key: &str, line: &str) -> Option<std::path::PathBuf> {
    handover_script_in(&cockpit_core::files::cache().join("handover"), key, line)
}

fn handover_script_in(dir: &std::path::Path, key: &str, line: &str) -> Option<std::path::PathBuf> {
    use std::hash::{BuildHasher, Hasher};
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    std::fs::create_dir_all(dir).ok()?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).ok()?;
    // A handover's script is done with once its terminal ran it: those of a week ago go.
    for old in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        if old.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age.as_secs() > 7 * 86_400) {
            let _ = std::fs::remove_file(old.path());
        }
    }
    let name: String = key.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let mut random = std::collections::hash_map::RandomState::new().build_hasher();
    random.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
    let path = dir.join(format!("{name}-{:016x}.sh", random.finish()));
    (!path.to_string_lossy().contains('\'')).then_some(())?;
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).custom_flags(libc::O_NOFOLLOW).open(&path).ok()?;
    file.write_all(format!("#!/bin/sh\n{line}\n").as_bytes()).ok()?;
    Some(path)
}

/// Whether the session can leave cockpit now: no turn running or being stopped, no ask waiting, no prompt queued, and
/// something said, so its agent has a conversation to go on with.
fn at_rest(key: &str, s: &Value) -> Result<(), String> {
    let stopping = live().get(key).is_some_and(|x| x.state.lock().unwrap_or_else(|e| e.into_inner()).interrupted);
    if s["working"] == true || stopping {
        return Err("it is working: stop its turn and wait for it to end, then open it in a terminal".into());
    }
    if s["pending"].as_array().is_some_and(|p| !p.is_empty()) {
        return Err("it waits on your answer: answer it first, then open it in a terminal".into());
    }
    let queued = QUEUED.get().map_or(0, |count| count(key));
    if queued > 0 {
        let prompts = if queued == 1 { "1 queued prompt".to_string() } else { format!("{queued} queued prompts") };
        return Err(format!("it has {prompts}: send or remove them first"));
    }
    if s["count"].as_u64().unwrap_or(0) == 0 {
        return Err("it has said nothing yet, so there is no conversation to go on with: send it something first".into());
    }
    Ok(())
}

/// How many prompts wait in a session's queue, as the queue says.
static QUEUED: OnceLock<fn(&str) -> usize> = OnceLock::new();

/// The queue says how many prompts wait for a session: a hand-over waits until none do.
pub fn on_queued(count: fn(&str) -> usize) {
    let _ = QUEUED.set(count);
}

// ---------------------------------------------------------------------------------------------------------------------
// The hub's view

/// The folders the view reads: none of its own, the sessions tell it when they move.
pub fn dirs() -> Vec<std::path::PathBuf> {
    Vec::new()
}

/// Every session without its messages, and the one `key` names (else the newest) with a page of them:
/// `{sessions: [...], session: {..., messages, cursor}}`. `limit` and `before` pick the page, as `page` says.
pub fn view(args: &Value, _doc: &dyn Fn(&str) -> Value) -> Value {
    let keys = keys();
    let wanted = args["key"].as_str().filter(|k| !k.is_empty()).or(keys.first().map(String::as_str));
    let limit = args["limit"].as_u64().map_or(PAGE, |n| n as usize);
    let before = args["before"].as_u64().map(|n| n as usize);
    let one = wanted.and_then(|k| page(k, limit, before));
    // A chat following one session needs no list of them all: its pills come from the agents view.
    match args["key"].as_str().is_some_and(|k| !k.is_empty()) {
        true => json!({"session": one}),
        false => json!({"sessions": keys.iter().filter_map(|k| summary(k)).collect::<Vec<_>>(), "session": one}),
    }
}

/// A mode that runs tools without asking (`modes::unasked`) is set on purpose, by a person: an action that would set
/// it says it was confirmed, and comes from no agent's process. One that only asks less (`loud`) is the person's too,
/// and one the table does not know is taken for one that runs tools unasked.
fn confirmed(agent: &str, mode: &str, arg: &Value) -> Result<(), String> {
    let known = modes::of(agent, mode);
    if known.is_none_or(|m| m.unasked) && arg["confirm"] != true {
        return Err(format!("{mode} runs tools without asking: it is set only once it is confirmed, pass {{confirm: true}}"));
    }
    if known.is_some_and(|m| !m.loud) {
        return Ok(());
    }
    crate::gui::by_a_person(&format!("{mode} is set by a person, not by an agent's tools"))
}

/// `start {agent, cwd, mode, model?, effort?, fast?}`, `send {key, text}`, `deliver {key, text}`, `stop {key, confirm?}`
/// (ends it: confirmed while it works), `mode {key, mode?}`, `model {key, model}`, `effort {key, effort}`, `fast {key,
/// fast}`, `interrupt {key}`, `terminal {key, confirm, frontend?}` (handed over to a terminal) and `answer {key?, ask,
/// allow, source}`.
pub fn act(action: &str, args: &[Value]) -> Option<Result<String, String>> {
    let arg = args.first().cloned().unwrap_or(Value::Null);
    let key = arg["key"].as_str().unwrap_or_default().to_string();
    // An agent's own tools may act on their own session only, and start none, in a terminal or not.
    let checked = match action {
        "start" => crate::gui::by_a_person("a session is started by a person, not by an agent's tools"),
        // The hand-over ends the session and resumes it in a terminal: a start too, a person's to do.
        "terminal" => crate::gui::by_a_person("a session is handed to a terminal by a person, not by an agent's tools"),
        // Fast mode spends the plan's usage faster: a person turns it on.
        "fast" if arg["fast"] == true => {
            crate::gui::by_a_person("fast mode is turned on by a person, not by an agent's tools").and(crate::gui::may_touch(&key))
        }
        "send" | "deliver" | "stop" | "model" | "effort" | "fast" | "mode" | "interrupt" => crate::gui::may_touch(&key),
        _ => Ok(()),
    };
    if let Err(why) = checked {
        return Some(Err(why));
    }
    Some(match action {
        "start" => {
            // The first agent of the table cockpit can drive, unless one is named.
            let streaming = || agent::all().into_iter().find(|a| a.stream.is_some()).map(|a| a.id);
            let Some(agent) = arg["agent"].as_str().map(str::to_string).or_else(streaming) else { return Some(Err("no agent cockpit can drive".into())) };
            let agent = agent.as_str();
            let cwd = arg["cwd"].as_str().unwrap_or_default();
            let mode = arg["mode"].as_str().unwrap_or("default");
            if cwd.is_empty() {
                return Some(Err("start where? pass {cwd}".into()));
            }
            let words = agent::get(agent).map(|a| modes::words_of(&a).to_string()).unwrap_or_else(|| agent.to_string());
            if let Err(why) = confirmed(&words, mode, &arg) {
                return Some(Err(why));
            }
            if arg["fast"] == true
                && let Err(why) = crate::gui::by_a_person("fast mode is turned on by a person, not by an agent's tools")
            {
                return Some(Err(why));
            }
            start(agent, cwd, mode, &Picks::from(&arg)).map(|k| format!("started {k}"))
        }
        "send" => {
            let text = arg["text"].as_str().unwrap_or_default().trim().to_string();
            if text.is_empty() {
                return Some(Err("nothing to send".into()));
            }
            // Like any words for it: refused while it waits on a permission.
            deliver_to(&key, &text).unwrap_or(Err("no such session".into())).map(|()| String::new())
        }
        "stop" => end(&key, arg["confirm"] == true).map(|()| "the session is over".into()),
        "mode" => {
            let now = summary(&key);
            // A session's key starts with its agent's id: the modes are known of one that is gone too.
            let agent = now.as_ref().and_then(|s| s["agent"].as_str()).or_else(|| key.split(':').next()).unwrap_or_default().to_string();
            let agent = agent::get(&agent).map(|a| modes::words_of(&a).to_string()).unwrap_or(agent);
            let want = match arg["mode"].as_str() {
                Some(mode) => mode.to_string(),
                // ⇧⇥: the next mode of its agent's cycle.
                None => match modes::next(&agent, now.as_ref().and_then(|s| s["mode"].as_str()).unwrap_or("default")) {
                    Some(next) => next.to_string(),
                    None => return Some(Err("no such session, or its agent has no mode cycle".into())),
                },
            };
            if let Err(why) = confirmed(&agent, &want, &arg) {
                return Some(Err(why));
            }
            control(&key, json!({"subtype": "set_permission_mode", "mode": want.clone()})).map(|()| want)
        }
        "model" | "effort" | "fast" => {
            let setting = Setting::of(action).unwrap_or(Setting::Model);
            pick(&key, setting, &arg[action])
        }
        "interrupt" => interrupt(&key).map(|()| "asked it to stop".into()),
        "terminal" => {
            let frontend = match arg["frontend"].as_str() {
                Some("tui") => Some(cockpit_core::frontend::Frontend::Tui),
                Some("app") => Some(cockpit_core::frontend::Frontend::App),
                _ => None,
            };
            open_in_terminal(&key, arg["confirm"] == true, frontend)
        }
        // What `deliver` hands over from another process: refused while the session waits on a permission.
        "deliver" => {
            let known = live().contains_key(&key);
            match known {
                true => deliver_to(&key, arg["text"].as_str().unwrap_or_default()).unwrap_or(Err("no such session".into())).map(|()| "sent".into()),
                false => Err("no such session".into()),
            }
        }
        // An answer names the ask it answers and says which way: a missing ask is refused like a stale one.
        "answer" => {
            let Some(id) = arg["ask"].as_str().filter(|a| !a.is_empty()) else { return Some(Err(CHANGED.into())) };
            let Some(allow) = arg["allow"].as_bool() else { return Some(Err("allow or deny? pass {allow: true} or {allow: false}".into())) };
            let source = arg["source"].as_str().filter(|s| !s.is_empty()).unwrap_or("unknown");
            let owner = if key.is_empty() { asking(id) } else { Some(key) };
            match owner {
                Some(key) => answer(&key, id, allow, source),
                None => Err(CHANGED.into()),
            }
        }
        _ => return None,
    })
}

// ---------------------------------------------------------------------------------------------------------------------
// Surviving a restart

/// A new session's id, a random UUID the agent takes with `--session-id`, so the registry knows it from the start.
fn new_session_id() -> String {
    let mut b = [0u8; 16];
    let read = std::fs::File::open("/dev/urandom").and_then(|mut f| std::io::Read::read_exact(&mut f, &mut b));
    if read.is_err() {
        let seed = (u128::from(now_ms()) << 64) | (u128::from(std::process::id()) << 32) | u128::from(SEQ.fetch_add(1, Ordering::Relaxed));
        b = seed.to_le_bytes();
    }
    (b[6], b[8]) = ((b[6] & 0x0f) | 0x40, (b[8] & 0x3f) | 0x80);
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

/// Where the sessions cockpit drives are noted, so a hub that starts again brings them back.
pub fn registry() -> PathBuf {
    dir().join("registry.json")
}

static HANDED: OnceLock<fn() -> bool> = OnceLock::new();

/// The hub says how to tell it handed over to another: from then on the registry is the next hub's to write.
pub fn on_hand_over(handed: fn() -> bool) {
    let _ = HANDED.set(handed);
}

/// The registry written again, atomically, when what it would hold changed: `{sessions: [{key, agent, session_id, cwd,
/// mode, model, effort, fast, pid, hub, prompted, started}]}`, `hub` the pid of the hub its process hangs off, a paused one with no pid.
/// A real session started in the demo world is kept like any other; the demo's own made-up ones never are.
fn remember() {
    static WRITTEN: Mutex<String> = Mutex::new(String::new());
    if TOUCH.get().is_none() || HANDED.get().is_some_and(|handed| handed()) {
        return;
    }
    let all: Vec<(String, Arc<Session>)> = live().iter().filter(|(_, s)| !s.demo).map(|(k, s)| (k.clone(), Arc::clone(s))).collect();
    let mut sessions: Vec<Value> = all
        .iter()
        .map(|(key, s)| {
            let pid = s.child.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(Child::id);
            let st = s.state.lock().unwrap_or_else(|e| e.into_inner());
            json!({"key": key, "agent": st.agent, "session_id": st.session_id, "cwd": st.cwd, "mode": st.mode, "model": st.model,
                   "effort": st.effort, "fast": st.fast, "account": st.account,
                   "pid": pid, "hub": std::process::id(), "prompted": !st.messages.is_empty(), "started": st.started})
        })
        .collect();
    sessions.sort_by(|a, b| a["key"].as_str().cmp(&b["key"].as_str()));
    let doc = json!({"sessions": sessions});
    let text = doc.to_string();
    let mut written = WRITTEN.lock().unwrap_or_else(|e| e.into_inner());
    if *written == text {
        return;
    }
    let _ = std::fs::create_dir_all(dir());
    if cockpit_core::files::write(&registry(), &doc).is_ok() {
        *written = text;
    }
}

/// After a restart: each session the registry names comes back paused, its history read from its transcript; every one
/// is in place before a process its last hub left running is ended (so a registry written meanwhile drops none), and its
/// history is read again after. One never prompted comes back empty; one whose transcript is gone or older than its log
/// is kept is dropped, and the person is told.
pub fn restore() {
    let Some(doc) = cockpit_core::files::load(&registry()) else { return };
    let mut dropped = 0;
    let mut left: Vec<(String, u32, Option<u32>)> = Vec::new();
    for e in doc["sessions"].as_array().into_iter().flatten() {
        let (key, id, cwd) = (e["key"].as_str().unwrap_or_default(), e["session_id"].as_str().unwrap_or_default(), e["cwd"].as_str().unwrap_or_default());
        if key.is_empty() || id.is_empty() || live().contains_key(key) {
            continue;
        }
        let agent = agent::get(e["agent"].as_str().unwrap_or_default());
        let kind = agent.as_ref().map_or(Transcripts::Claude, |a| a.transcripts);
        let found = transcripts::find(kind, id, cwd).filter(recent);
        if found.is_none() && e["prompted"] != false {
            note(key, &format!("not brought back: the transcript of {id} is gone or old"));
            dropped += 1;
            continue;
        }
        let text = |k: &str| e[k].as_str().unwrap_or_default().to_string();
        // A mode that asks less (`loud`: tools or edits unasked) is set by a person each time: a restart does not carry it over.
        let words = agent.as_ref().map_or_else(|| e["agent"].as_str().unwrap_or_default().to_string(), |a| modes::words_of(a).to_string());
        let mode = e["mode"].as_str().filter(|m| modes::of(&words, m).is_none_or(|known| !known.loud)).unwrap_or("default").to_string();
        let mut state = State {
            session_id: id.to_string(),
            agent: text("agent"),
            cwd: cwd.to_string(),
            account: text("account"),
            mode,
            model: text("model"),
            effort: text("effort"),
            fast: e["fast"].as_bool(),
            started: e["started"].as_u64().unwrap_or(0),
            paused: true,
            // What the last hub's process wrote is ours: only a write after now is someone else's.
            exited_at: now_ms(),
            ..Default::default()
        };
        read_history(&mut state, kind);
        let session = Session { state: Arc::new(Mutex::new(state)), stdin: Arc::new(Mutex::new(None)), child: Mutex::new(None), demo: false };
        live().insert(key.to_string(), Arc::new(session));
        note(key, "brought back paused after a restart");
        if let Some(pid) = e["pid"].as_u64().and_then(|p| u32::try_from(p).ok()) {
            left.push((key.to_string(), pid, e["hub"].as_u64().and_then(|p| u32::try_from(p).ok())));
        }
    }
    moved();
    for (key, pid, hub) in left {
        let Some(session) = live().get(&key).cloned() else { continue };
        let (id, agent) = {
            let s = session.state.lock().unwrap_or_else(|e| e.into_inner());
            (s.session_id.clone(), agent::get(&s.agent))
        };
        let Some(agent) = agent else { continue };
        if end_orphan(pid, &id, hub, &agent) {
            // What it wrote before it went is part of the conversation, and ours.
            let mut state = session.state.lock().unwrap_or_else(|e| e.into_inner());
            read_history(&mut state, agent.transcripts);
            state.exited_at = now_ms();
        }
    }
    if dropped > 0 {
        let what = if dropped == 1 {
            "1 old conversation was not brought back: its transcript is gone".to_string()
        } else {
            format!("{dropped} old conversations were not brought back: their transcripts are gone")
        };
        cockpit_core::notify::post("Sessions in chat", &what, "", &[]);
    }
    moved();
}

/// Whether a transcript was written within the time a session's log is kept.
fn recent(path: &PathBuf) -> bool {
    std::fs::metadata(path).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age < LOGS_FOR)
}

/// The session's history, read from its transcript in its agent's format `kind`, and when it last moved.
fn read_history(state: &mut State, kind: Transcripts) {
    let Some(path) = transcripts::find(kind, &state.session_id, &state.cwd) else { return };
    if let Some(t) = transcripts::read_detail(kind, &path, Detail::Tools) {
        state.messages = t.messages.iter().map(crate::chat::message).collect();
        state.tasks = t.tasks.iter().map(crate::chat::task_json_plain).collect();
    }
    state.since = modified_ms(&path);
    if state.started == 0 {
        state.started = state.since;
    }
}

/// When a file last changed, in ms; 0 when it cannot be told.
fn modified_ms(path: &std::path::Path) -> u64 {
    std::fs::metadata(path).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis() as u64)
}

/// How recently a transcript written by someone else stops a resume: another process is in that conversation.
const WRITTEN_MS: u64 = 5_000;

/// A paused session's agent started again under the same session id: on its transcript, or new when it was never
/// prompted. Refused while another process may run that session, or wrote its transcript since our last process ended.
fn resume(key: &str, session: &Arc<Session>) -> Result<(), String> {
    let (agent_id, id, cwd, exited) = {
        let s = session.state.lock().unwrap_or_else(|e| e.into_inner());
        (s.agent.clone(), s.session_id.clone(), s.cwd.clone(), s.exited_at)
    };
    let found = agent::all().into_iter().find(|a| a.id == agent_id).ok_or_else(|| format!("no agent {agent_id:?}"))?;
    if running(&found, &id, &cwd) {
        return Err(
            "another process may run this session (the last hub's agent still exiting, or a terminal it was opened or continued in): try again in a moment, or end it there"
                .into(),
        );
    }
    let transcript = transcripts::find(found.transcripts, &id, &cwd);
    if transcript.as_ref().is_some_and(|p| written_by_another(modified_ms(p), exited, now_ms())) {
        return Err("another process is writing this conversation: end it there first".into());
    }
    launch(&found, key, session, transcript.is_some())
}

/// Whether a transcript last written at `written` (ms) was written by someone else: within WRITTEN_MS of `now`, and after
/// our last process of it ended at `exited` (0: none ran in this hub).
pub fn written_by_another(written: u64, exited: u64, now: u64) -> bool {
    now.saturating_sub(written) < WRITTEN_MS && written > exited.saturating_add(500)
}

/// The processes on this machine and their command lines.
fn processes() -> Vec<(u32, u32, String)> {
    let out =
        Command::new("/bin/ps").args(["-axo", "pid=,ppid=,command="]).output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    out.lines()
        .filter_map(|l| {
            let mut w = l.split_whitespace();
            let (pid, ppid) = (w.next()?.parse().ok()?, w.next()?.parse().ok()?);
            Some((pid, ppid, w.collect::<Vec<_>>().join(" ")))
        })
        .collect()
}

/// Whether a running process may take the session `id` up as its own: one naming it, or the agent continuing the latest
/// session of the session's folder `cwd`. A fork of it (a recap) writes another.
fn running(agent: &Agent, id: &str, cwd: &str) -> bool {
    processes().iter().any(|(pid, _, command)| {
        let fork = command.split_whitespace().any(|w| w == "--fork-session");
        !fork && (names(command, id) || (continues(command, agent) && cwd_of(*pid).is_some_and(|c| same_folder(&c, cwd))))
    })
}

/// Whether `command` opens or resumes the session `id`: `--resume <id>`, `-r <id>`, `--resume=<id>`, `--session-id <id>`
/// or `--session-id=<id>`.
pub fn names(command: &str, id: &str) -> bool {
    let words: Vec<&str> = command.split_whitespace().collect();
    let joined = words.iter().any(|w| w.strip_prefix("--resume=").or_else(|| w.strip_prefix("--session-id=")) == Some(id));
    joined || words.windows(2).any(|w| matches!(w[0], "--session-id" | "--resume" | "-r") && w[1] == id)
}

/// Whether `command` runs `agent` on the latest session of its folder: `--continue` or `-c`.
pub fn continues(command: &str, agent: &Agent) -> bool {
    let words: Vec<&str> = command.split_whitespace().collect();
    let program = words.iter().take(2).any(|w| {
        let name = std::path::Path::new(w).file_name().and_then(|n| n.to_str()).unwrap_or_default();
        name == agent.bin || name == agent.id || agent.aliases.iter().any(|a| a == name)
    });
    program && words.iter().any(|w| matches!(*w, "--continue" | "-c"))
}

/// A process's working folder, when the system says.
fn cwd_of(pid: u32) -> Option<String> {
    if let Ok(path) = std::fs::read_link(format!("/proc/{pid}/cwd")) {
        return Some(path.to_string_lossy().into_owned());
    }
    let out = Command::new("/usr/sbin/lsof").args(["-a", "-p", &pid.to_string(), "-d", "cwd", "-Fn"]).stderr(Stdio::null()).output().ok()?;
    String::from_utf8_lossy(&out.stdout).lines().find_map(|l| l.strip_prefix('n').map(str::to_string))
}

/// Whether two paths name one folder, through links (/tmp and /private/tmp on macOS).
fn same_folder(a: &str, b: &str) -> bool {
    let real = |p: &str| std::fs::canonicalize(p).unwrap_or_else(|_| PathBuf::from(p));
    real(a) == real(b)
}

/// Ends what a hub that went away left running for the session `id`, true when it did, only while that pid still runs
/// the session's agent; one its `hub` still holds, as in an update's hand-over, gets a few seconds to go with it first.
/// Its group goes when it leads one, else the process alone, its command line looked at again before each signal.
fn end_orphan(pid: u32, id: &str, hub: Option<u32>, agent: &Agent) -> bool {
    let look = |pid: u32| processes().into_iter().find(|(p, _, _)| *p == pid);
    let alive = |pid: u32| look(pid).is_some();
    if let Some((_, parent, _)) = look(pid)
        && Some(parent) == hub
        && hub != Some(std::process::id())
    {
        let started = std::time::Instant::now();
        while alive(pid) && hub.is_some_and(alive) && started.elapsed() < std::time::Duration::from_secs(5) {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
    let stream = agent.stream.clone().unwrap_or_default();
    let Some((_, parent, command)) = look(pid) else { return false };
    if !orphan(&command, parent, hub, id, &stream) {
        return false;
    }
    // Gone before the session is picked up, so two never write one transcript.
    let Ok(raw) = libc::pid_t::try_from(pid) else { return false };
    // SAFETY: getpgid only reads the process's group.
    match unsafe { libc::getpgid(raw) } == raw {
        true => end_group(pid, true),
        false => {
            let id = id.to_string();
            let still = move || processes().into_iter().find(|(p, _, _)| *p == pid).is_some_and(|(_, parent, c)| orphan(&c, parent, hub, &id, &stream));
            // SAFETY: kill signals this one process, after its command line was looked at again.
            stop_then_kill(move |sig| still() && unsafe { libc::kill(raw, sig) } == 0, true);
        }
    }
    true
}

/// Whether a process running `command` under the pid `parent` is the session `id`'s agent left behind: its command line
/// opens the stream (`stream`, the agent's switches) and names that id, and it hangs off no live hub (`hub` the pid of
/// the one that started it, gone or not its parent now).
pub fn orphan(command: &str, parent: u32, hub: Option<u32>, id: &str, stream: &[String]) -> bool {
    let driven = !stream.is_empty() && command.contains(&stream.join(" ")) && names(command, id);
    driven && Some(parent) != hub.filter(|h| *h != std::process::id())
}

// ---------------------------------------------------------------------------------------------------------------------
// The demo world

/// The demo world's driven sessions (`driven` in COCKPIT_AGENTS_DEMO's file), answered in memory. Ages are minutes
/// before now.
fn demo_sessions() -> HashMap<String, Arc<Session>> {
    let Some(world) = crate::term::demo() else { return HashMap::new() };
    let now = now_ms();
    let ago = |min: &Value| now.saturating_sub((min.as_f64().unwrap_or(0.0) * 60_000.0) as u64);
    let stamp = |ms: u64| chrono::DateTime::from_timestamp_millis(ms as i64).map(|d| d.to_rfc3339()).unwrap_or_default();
    let mut out = HashMap::new();
    for d in world["driven"].as_array().into_iter().flatten() {
        let Some(key) = d["key"].as_str() else { continue };
        let messages = d["messages"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|m| json!({"role": m["role"], "at": stamp(ago(&m["min"])), "text": m["text"].as_str().unwrap_or_default(), "tool": m["tool"]}))
            .collect();
        let pending = d["ask"]
            .is_object()
            .then(|| {
                let a = &d["ask"];
                let id = a["tool_use_id"].as_str().unwrap_or("demo-ask");
                json!({"id": id, "request_id": format!("demo-{id}"), "at": ago(&a["min"]), "tool_name": a["tool_name"], "input": a["input"],
                       "tool_use_id": id, "description": a["input"]["description"]})
            })
            .into_iter()
            .collect();
        let state = State {
            session_id: d["session"].as_str().unwrap_or_default().to_string(),
            agent: d["agent"].as_str().unwrap_or("claude").to_string(),
            cwd: d["cwd"].as_str().unwrap_or_default().to_string(),
            mode: d["mode"].as_str().unwrap_or("default").to_string(),
            model: d["model"].as_str().unwrap_or_default().to_string(),
            cost: d["cost"].as_f64(),
            started: ago(&d["started_min"]),
            working: d["ask"].is_object(),
            messages,
            pending,
            ..Default::default()
        };
        let session = Session { state: Arc::new(Mutex::new(state)), stdin: Arc::new(Mutex::new(None)), child: Mutex::new(None), demo: true };
        out.insert(key.to_string(), Arc::new(session));
    }
    out
}

/// A demo session that got its answer: the call it waited on ran, or did not, and the turn ends.
fn demo_answered(state: &mut State, ask: &Value, allow: bool) {
    let at = chrono::Utc::now().to_rfc3339();
    for m in state.messages.iter_mut().rev() {
        if m["tool"].is_object() && m["tool"]["ok"].is_null() {
            m["tool"]["ok"] = json!(allow);
            break;
        }
    }
    let said = match allow {
        true => format!("Ran {}: all 34 tests pass.", ask["tool_name"].as_str().unwrap_or("it")),
        false => "Left it alone, as you asked. Tell me what to do instead.".to_string(),
    };
    state.messages.push(json!({"role": "assistant", "at": at, "text": said, "tool": null}));
    state.working = false;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_result_says_how_the_turn_went_and_a_success_clears_the_last_failure() {
        let failed = json!({"type": "result", "is_error": true, "result": "credit balance too low"});
        let fine = json!({"type": "result", "is_error": false, "result": "Done."});
        assert_eq!(outcome(&failed, false), Some("failed"));
        assert_eq!(outcome(&failed, true), Some("interrupted"), "a stop reports an error that is not one");
        assert_eq!(outcome(&fine, false), Some("done"));
        assert_eq!(outcome(&fine, true), Some("interrupted"));
        assert_eq!(outcome(&json!({"type": "assistant"}), false), None);
        let mut state = State::default();
        take(&mut state, &failed);
        assert_eq!(state.failed.as_deref(), Some("credit balance too low"));
        take(&mut state, &fine);
        assert_eq!(state.failed, None, "a later success clears it");
    }

    /// A failed turn is told once, its process ending right after it too, and the next prompt's failure again.
    #[test]
    fn a_failure_is_told_once_until_the_next_prompt() {
        let mut state = State::default();
        take(&mut state, &json!({"type": "result", "is_error": false, "result": "Done."}));
        assert!(!failure_is_news(&mut state), "a turn that went well is no failure");
        take(&mut state, &json!({"type": "result", "is_error": true, "result": "credit balance too low"}));
        assert!(failure_is_news(&mut state));
        assert!(!failure_is_news(&mut state), "told once");
        state.ended = Some("the agent stopped".into());
        assert!(!failure_is_news(&mut state), "its process ending after the failed turn is the same failure");
        (state.failed, state.ended, state.told_failure) = (None, None, false);
        state.ended = Some("the agent stopped".into());
        assert!(failure_is_news(&mut state), "after another prompt, an ended process is news");
    }

    #[test]
    fn a_stop_needs_a_turn_and_goes_once() {
        let key = "claude:stop-test-0";
        let state = State { working: true, ..Default::default() };
        live().insert(
            key.into(),
            Arc::new(Session { state: Arc::new(Mutex::new(state)), stdin: Arc::new(Mutex::new(None)), child: Mutex::new(None), demo: false }),
        );
        assert_eq!(interrupt(key).unwrap_err(), "the session takes no more", "no process to tell");
        assert!(!live()[key].state.lock().unwrap().interrupted, "a stop that did not go is not pending");
        live()[key].state.lock().unwrap().interrupted = true;
        assert_eq!(interrupt(key).unwrap_err(), "it is being stopped already");
        live()[key].state.lock().unwrap().working = false;
        assert_eq!(interrupt(key).unwrap_err(), "it is not working: there is no turn to stop");
        live().remove(key);
    }

    #[test]
    fn a_session_is_named_however_its_agent_is_told_to_take_it_up() {
        let id = "00000000-acme-4000-8000-000000000421";
        for command in [
            format!("claude --resume {id}"),
            format!("claude -r {id}"),
            format!("claude --resume={id}"),
            format!("claude -p --session-id {id}"),
            format!("claude -p --session-id={id}"),
        ] {
            assert!(names(&command, id), "{command}");
        }
        assert!(!names(&format!("claude --resume {id}x"), id), "another id");
        assert!(!names(&format!("vim -r notes-{id}"), id), "the id inside another word");
        let claude = agent::builtin().into_iter().find(|a| a.id == "claude").unwrap();
        assert!(continues("/Users/me/.local/bin/claude --continue", &claude));
        assert!(continues("node /Users/me/.local/lib/claude -c", &claude), "the agent run by node");
        assert!(!continues("claude --resume x", &claude));
        assert!(!continues("cat -c notes.txt", &claude), "another program's -c");
    }

    #[test]
    fn a_transcript_written_since_our_last_process_ended_is_someone_else_s() {
        let now = 100_000;
        assert!(written_by_another(now - 1_000, 0, now), "written a second ago, no process of ours ran: /resume elsewhere");
        assert!(!written_by_another(now - 1_000, now - 1_200, now), "our own process wrote it as it ended");
        assert!(written_by_another(now - 1_000, now - 4_000, now), "written well after ours ended");
        assert!(!written_by_another(now - 60_000, 0, now), "written long ago: nobody is in it now");
    }

    #[test]
    fn an_old_process_ending_leaves_the_state_of_the_one_that_took_its_place() {
        let fresh = State { working: true, pending: vec![json!({"id": "toolu_acme_1"})], ..Default::default() };
        let newer = Command::new("/bin/sleep").arg("5").spawn().unwrap();
        let newer_pid = newer.id();
        let session = Session { state: Arc::new(Mutex::new(fresh)), stdin: Arc::new(Mutex::new(None)), child: Mutex::new(Some(newer)), demo: false };
        assert!(reaped(&session, newer_pid + 1, "claude:reap-test").is_none(), "not its process");
        {
            let s = session.state.lock().unwrap();
            assert!(s.working && s.pending.len() == 1 && s.ended.is_none(), "the newer start's turn, ask and process are kept");
        }
        assert!(session.child.lock().unwrap().is_some(), "and its pid stays known, so its tools are told apart");
        let mut own = reaped(&session, newer_pid, "claude:reap-test").expect("its own process");
        let _ = own.kill();
        let _ = own.wait();
        let s = session.state.lock().unwrap();
        assert!(!s.working && s.pending.is_empty() && s.ended.is_some() && s.exited_at > 0, "its own end is marked");
    }

    #[test]
    fn words_that_waited_for_an_end_pick_nothing_up() {
        let key = "claude:gone-test-0";
        let state = State { session_id: "00000000-acme-4000-8000-000000000777".into(), agent: "claude".into(), paused: true, ..Default::default() };
        live().insert(
            key.into(),
            Arc::new(Session { state: Arc::new(Mutex::new(state)), stdin: Arc::new(Mutex::new(None)), child: Mutex::new(None), demo: false }),
        );
        let lock = launching(key);
        let held = lock.lock().unwrap();
        let words = std::thread::spawn(|| send(key, "and then?"));
        std::thread::sleep(std::time::Duration::from_millis(100));
        live().remove(key);
        drop(held);
        assert_eq!(words.join().unwrap(), Err(GONE.to_string()), "the End that came first wins: no agent is started for it");
        assert!(!live().contains_key(key));
    }

    #[test]
    fn only_the_session_s_own_agent_left_by_a_hub_is_an_orphan() {
        let id = "00000000-acme-4000-8000-000000000421";
        let stream = agent::builtin().into_iter().find(|a| a.id == "claude").and_then(|a| a.stream).unwrap();
        let agent = format!("/Users/me/.local/bin/claude {} --permission-mode default --resume {id}", stream.join(" "));
        let old_hub = Some(4242);
        assert!(orphan(&agent, 1, old_hub, id, &stream), "its hub is gone: it hangs off launchd");
        assert!(orphan(&agent.replace("--resume", "--session-id"), 7, old_hub, id, &stream), "a new session, under a subreaper");
        assert!(!orphan(&agent, 4242, old_hub, id, &stream), "the hub that started it still holds it");
        assert!(!orphan("/bin/sleep 30", 1, old_hub, id, &stream), "a process that took the pid since");
        assert!(!orphan(&agent, 1, old_hub, "00000000-acme-4000-8000-000000000999", &stream), "another session's agent");
        assert!(!orphan(&format!("vim notes-{id}.md --resume {id}"), 1, old_hub, id, &stream), "the id alone is not enough");
        assert!(!orphan(&agent, 1, old_hub, id, &[]), "an agent cockpit does not drive");
    }

    #[test]
    fn a_new_session_id_is_a_uuid_of_its_own() {
        let (a, b) = (new_session_id(), new_session_id());
        assert_ne!(a, b);
        assert_eq!(a.len(), 36);
        assert_eq!(a.as_bytes()[14], b'4', "version 4");
    }

    #[test]
    fn the_view_sends_a_page_of_messages_and_where_it_sits() {
        let key = "claude:page-test-0";
        let messages = (0..1000).map(|i| json!({"role": if i == 0 { "user" } else { "assistant" }, "at": "", "text": format!("m{i}"), "tool": null})).collect();
        let state = State { session_id: "page-test".into(), agent: "claude".into(), messages, ..Default::default() };
        live().insert(
            key.into(),
            Arc::new(Session { state: Arc::new(Mutex::new(state)), stdin: Arc::new(Mutex::new(None)), child: Mutex::new(None), demo: false }),
        );
        let texts = |v: &Value| v["messages"].as_array().unwrap().iter().map(|m| m["text"].as_str().unwrap().to_string()).collect::<Vec<_>>();

        let latest = view(&json!({"key": key}), &|_| Value::Null);
        assert_eq!(latest["session"]["cursor"], json!({"from": 600, "to": 1000, "total": 1000, "more": true}));
        assert_eq!(texts(&latest["session"]).first().map(String::as_str), Some("m600"));
        assert_eq!(latest["session"]["title"], "m0", "the title is the first prompt, wherever the page is");
        assert!(latest["sessions"].is_null(), "a view of one session leaves the list out");
        let all = view(&json!({}), &|_| Value::Null);
        let listed = all["sessions"].as_array().unwrap().iter().find(|s| s["key"] == key).unwrap();
        assert!(listed["messages"].is_null() && listed["count"] == 1000, "the list carries counts, not messages");

        let older = view(&json!({"key": key, "before": 600, "limit": 250}), &|_| Value::Null);
        assert_eq!(older["session"]["cursor"], json!({"from": 350, "to": 600, "total": 1000, "more": true}));
        let whole = page(key, 5000, None).unwrap();
        assert_eq!((whole["cursor"]["from"].clone(), whole["cursor"]["more"].clone(), texts(&whole).len()), (json!(0), json!(false), 1000));
        live().remove(key);
    }

    #[test]
    fn a_session_goes_to_a_terminal_only_at_rest_and_once_the_person_says_so() {
        let key = "claude:handover-test-0";
        // The queue holds two prompts for one session only.
        on_queued(|k| if k == "claude:handover-queued" { 2 } else { 0 });
        let put = |key: &str, working: bool, interrupted: bool, pending: Vec<Value>, said: bool| {
            let messages = if said { vec![json!({"role": "user", "text": "Fix the export"})] } else { Vec::new() };
            let state = State {
                session_id: "0c8b2f4e-handover".into(),
                agent: "claude".into(),
                cwd: "/r/api".into(),
                working,
                interrupted,
                pending,
                messages,
                ..Default::default()
            };
            live().insert(
                key.into(),
                Arc::new(Session { state: Arc::new(Mutex::new(state)), stdin: Arc::new(Mutex::new(None)), child: Mutex::new(None), demo: true }),
            );
        };
        let refused = |key: &str| open_in_terminal(key, true, None).unwrap_err();
        put(key, true, false, Vec::new(), true);
        assert!(refused(key).contains("working"), "two writers on one session never");
        put(key, false, true, Vec::new(), true);
        assert!(refused(key).contains("working"), "an interrupt answered is not the turn ended: its result is");
        put(key, false, false, vec![json!({"id": "toolu_1"})], true);
        assert!(refused(key).contains("answer"));
        put("claude:handover-queued", false, false, Vec::new(), true);
        assert_eq!(refused("claude:handover-queued"), "it has 2 queued prompts: send or remove them first", "no prompt is dropped");
        put(key, false, false, Vec::new(), false);
        assert!(refused(key).contains("said nothing"), "nothing to go on with");
        put(key, false, false, Vec::new(), true);
        assert!(open_in_terminal(key, false, None).unwrap_err().contains("confirm"), "it leaves the chat only once confirmed");
        assert!(live().contains_key(key), "refused, it runs on here");
        live().remove(key);
        live().remove("claude:handover-queued");
    }

    #[test]
    fn words_sent_during_a_turn_wait_for_their_own_replay_past_its_end() {
        let mut state = State::default();
        state.messages.push(json!({"role": "user", "text": "Make it configurable", "echo": true}));
        state.messages.push(json!({"role": "user", "text": "And the docs", "echo": true, "later": true}));
        take(&mut state, &json!({"type": "result", "is_error": false, "result": "Done."}));
        assert_eq!(state.messages[0]["echo"], Value::Null, "the turn's own prompt arrived");
        assert_eq!(state.messages[1]["echo"], true, "the one sent during it waits for the next turn");
        take(&mut state, &json!({"type": "user", "message": {"content": [{"type": "text", "text": "And the docs"}]}}));
        assert_eq!(state.messages.len(), 2, "its replay is the message already there, never a second one");
        assert_eq!(state.messages[1]["echo"], Value::Null);
    }

    fn fold(events: &[Value]) -> State {
        let mut state = State::default();
        for e in events {
            take(&mut state, e);
        }
        state
    }

    #[test]
    fn a_slash_command_sent_to_a_driven_session_is_a_setting_a_refusal_or_words() {
        let key = "claude:slash-test-0";
        let state = State { session_id: "0c8b2f4e-slash".into(), agent: "claude".into(), cwd: "/r/api".into(), model: "sonnet".into(), ..Default::default() };
        let session = Arc::new(Session { state: Arc::new(Mutex::new(state)), stdin: Arc::new(Mutex::new(None)), child: Mutex::new(None), demo: true });
        live().insert(key.into(), Arc::clone(&session));
        let read = |f: &dyn Fn(&State) -> Value| f(&session.state.lock().unwrap_or_else(|e| e.into_inner()));
        assert!(send(key, "/model opus").is_ok(), "a setting cockpit applies itself");
        assert_ne!(read(&|s| json!(s.model)), json!("sonnet"), "the model moved");
        assert!(send(key, "/model").unwrap_err().contains("pick it from the chip"), "its picker is a terminal's");
        assert!(send(key, "/resume").unwrap_err().contains("hand the session over to a terminal"));
        assert!(send(key, "/nope").unwrap_err().contains("no /nope"));
        session.state.lock().unwrap_or_else(|e| e.into_inner()).commands = vec![json!({"name": "review", "description": "Review it"})];
        assert!(send(key, "/review the branch").is_ok(), "a command the agent lists goes as words");
        assert_eq!(read(&|s| s.messages.last().cloned().unwrap_or_default())["text"], "/review the branch");
        live().remove(key);
    }

    #[test]
    fn a_session_with_no_commands_changed_still_knows_the_slash_commands() {
        let state = fold(&[json!({"type": "system", "subtype": "init", "slash_commands": ["review", "ship"]})]);
        assert_eq!(
            state.commands.iter().map(|c| c["name"].as_str().unwrap_or("")).collect::<Vec<_>>(),
            ["review", "ship"],
            "some sessions publish them only on init"
        );
    }

    #[test]
    fn the_stream_becomes_the_conversation_the_chat_draws() {
        let state = fold(&[
            json!({"type": "system", "subtype": "init", "session_id": "s-1", "cwd": "/r/api"}),
            json!({"type": "system", "subtype": "commands_changed", "commands": [{"name": "review", "description": "Review it"}]}),
            json!({"type": "assistant", "message": {"content": [{"type": "thinking", "thinking": "hm"}, {"type": "text", "text": "Reading it."}]}}),
            json!({"type": "assistant", "message": {"content": [{"type": "tool_use", "id": "t1", "name": "Bash", "input": {"command": "mix test"}}]}}),
            json!({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": "t1", "is_error": true}]}}),
            json!({"type": "rate_limit_event", "rate_limit_info": {"rateLimitType": "five_hour"}}),
            json!({"type": "result", "total_cost_usd": 0.25, "usage": {"output_tokens": 9}}),
        ]);
        assert_eq!((state.session_id.as_str(), state.cwd.as_str()), ("s-1", "/r/api"));
        assert_eq!(state.commands.len(), 1, "the agent's own commands are kept");
        assert_eq!(
            state.messages.iter().map(|m| (m["role"].as_str().unwrap_or(""), m["text"].as_str().unwrap_or(""), m["tool"]["name"].as_str())).collect::<Vec<_>>(),
            [("assistant", "Reading it.", None), ("assistant", "", Some("Bash"))],
            "thinking is left out, words and calls are kept in order"
        );
        assert_eq!(state.messages[1]["tool"]["summary"], "mix test");
        assert_eq!(state.messages[1]["tool"]["ok"], json!(false), "the result says the call failed");
        assert_eq!((state.working, state.cost), (false, Some(0.25)));
        assert_eq!(state.rate_limit["rateLimitType"], "five_hour");
        assert!(state.rate_limit_at > 0, "stamped when it came");
        assert!(state.ended.is_none());
    }

    #[test]
    fn the_tasks_a_turn_starts_are_folded_from_the_stream_and_a_subagent_s_lines_stay_out_of_the_conversation() {
        let mut state = State { agent: "claude".into(), ..State::default() };
        let events = [
            json!({"type": "assistant", "message": {"content": [{"type": "tool_use", "id": "toolu_agent", "name": "Agent", "input": {"description": "List files in repo", "subagent_type": "Explore", "prompt": "List them"}}]}, "parent_tool_use_id": null}),
            json!({"type": "system", "subtype": "background_tasks_changed", "tasks": [{"task_id": "a1f2e3d4", "run_id": "r1", "task_type": "local_agent", "subagent_type": "Explore", "description": "List files in repo"}]}),
            json!({"type": "system", "subtype": "task_started", "task_id": "a1f2e3d4", "run_id": "r1", "tool_use_id": "toolu_agent", "description": "List files in repo", "subagent_type": "Explore", "is_backgrounded": true, "task_type": "local_agent"}),
            json!({"type": "user", "message": {"content": [{"tool_use_id": "toolu_agent", "type": "tool_result", "content": [{"type": "text", "text": "Async agent launched successfully."}]}]}, "parent_tool_use_id": null, "tool_use_result": {"isAsync": true, "status": "async_launched", "agentId": "a1f2e3d4"}}),
            json!({"type": "assistant", "message": {"content": [{"type": "tool_use", "id": "toolu_bash", "name": "Bash", "input": {"command": "sleep 3; echo done", "description": "Sleep then echo", "run_in_background": true}}]}, "parent_tool_use_id": null}),
            json!({"type": "assistant", "message": {"content": [{"type": "tool_use", "id": "toolu_sub", "name": "Bash", "input": {"command": "ls -la"}}]}, "parent_tool_use_id": "toolu_agent", "agent_id": "a1f2e3d4"}),
            json!({"type": "user", "message": {"content": [{"tool_use_id": "toolu_sub", "type": "tool_result", "content": "total 0"}]}, "parent_tool_use_id": "toolu_agent", "agent_id": "a1f2e3d4"}),
            json!({"type": "assistant", "message": {"content": [{"type": "text", "text": "The directory is empty."}]}, "parent_tool_use_id": "toolu_agent", "agent_id": "a1f2e3d4"}),
            json!({"type": "system", "subtype": "task_progress", "task_id": "a1f2e3d4", "run_id": "r1", "tool_use_id": "toolu_agent", "description": "Running ls", "usage": {"tool_uses": 1}, "last_tool_name": "Bash"}),
            json!({"type": "system", "subtype": "task_started", "task_id": "b7k2m9xq1", "run_id": "r2", "tool_use_id": "toolu_bash", "description": "Sleep then echo", "is_backgrounded": true, "task_type": "local_bash"}),
            json!({"type": "user", "message": {"content": [{"tool_use_id": "toolu_bash", "type": "tool_result", "content": "Command running in background with ID: b7k2m9xq1.", "is_error": false}]}, "parent_tool_use_id": null, "tool_use_result": {"backgroundTaskId": "b7k2m9xq1"}}),
        ];
        for e in &events {
            take(&mut state, e);
        }
        let shown = |s: &State| {
            s.tasks
                .iter()
                .map(|t| {
                    (
                        t["id"].as_str().unwrap_or("").to_string(),
                        t["kind"].as_str().unwrap_or("").to_string(),
                        t["name"].as_str().unwrap_or("").to_string(),
                        t["state"].as_str().unwrap_or("").to_string(),
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            shown(&state),
            [
                ("a1f2e3d4".to_string(), "subagent".into(), "List files in repo".into(), "running".into()),
                ("b7k2m9xq1".to_string(), "background".into(), "Sleep then echo".into(), "running".into())
            ],
            "a call starts its task, task_started and the result name it"
        );
        assert_eq!(state.tasks[0]["tool_use_id"], "toolu_agent");
        assert_eq!((state.tasks[0]["progress"].as_str(), state.tasks[0]["last_tool"].as_str()), (Some("Running ls"), Some("Bash")));
        assert_eq!(
            state.messages.iter().map(|m| m["tool"]["name"].as_str().unwrap_or(m["text"].as_str().unwrap_or(""))).collect::<Vec<_>>(),
            ["Agent", "Bash"],
            "the subagent's calls and words never enter the parent's conversation"
        );
        assert_eq!(state.messages[0]["tool"]["id"], "toolu_agent");
        take(
            &mut state,
            &json!({"type": "system", "subtype": "task_updated", "task_id": "a1f2e3d4", "run_id": "r1", "patch": {"status": "completed", "end_time": 1795603567075u64}}),
        );
        take(
            &mut state,
            &json!({"type": "system", "subtype": "task_notification", "task_id": "a1f2e3d4", "run_id": "r1", "tool_use_id": "toolu_agent", "status": "completed", "summary": "The directory is empty."}),
        );
        assert_eq!((state.tasks[0]["state"].as_str(), state.tasks[0]["summary"].as_str()), (Some("done"), Some("The directory is empty.")));
        assert!(state.tasks[0]["ended"].is_string());
        take(&mut state, &json!({"type": "system", "subtype": "background_tasks_changed", "tasks": []}));
        assert_eq!(state.tasks[1]["state"], "done", "a snapshot without it ends the job");
        take(&mut state, &json!({"type": "system", "subtype": "task_notification", "task_id": "b7k2m9xq1", "status": "failed", "summary": "exit code 1"}));
        assert_eq!(state.tasks[1]["state"], "done", "over is over: a later word does not move it");
        assert_eq!(state.json()["tasks"].as_array().map(Vec::len), Some(2), "the view carries them");

        // A snapshot that names a task nobody announced, and a killed one.
        let mut other = State { agent: "claude".into(), ..State::default() };
        take(
            &mut other,
            &json!({"type": "system", "subtype": "background_tasks_changed", "tasks": [{"task_id": "z9", "task_type": "local_bash", "description": "Watch the logs"}]}),
        );
        take(&mut other, &json!({"type": "system", "subtype": "task_updated", "task_id": "z9", "patch": {"status": "killed"}}));
        assert_eq!(shown(&other), [("z9".to_string(), "background".into(), "Watch the logs".into(), "failed".into())]);
        // A spawn whose result came back whole, and a job not sent to the background, start nothing that outlives the call.
        let mut sync = State { agent: "claude".into(), ..State::default() };
        take(
            &mut sync,
            &json!({"type": "assistant", "message": {"content": [{"type": "tool_use", "id": "t1", "name": "Agent", "input": {"description": "Check it", "prompt": "x"}}, {"type": "tool_use", "id": "t2", "name": "Bash", "input": {"command": "ls"}}]}}),
        );
        take(
            &mut sync,
            &json!({"type": "user", "message": {"content": [{"tool_use_id": "t1", "type": "tool_result", "content": "Checked.", "is_error": true}]}}),
        );
        assert_eq!(shown(&sync), [("t1".to_string(), "subagent".into(), "Check it".into(), "failed".into())]);
        assert_eq!(stop_task("claude:nobody", "a1"), Err("no such session".into()));
    }

    #[test]
    fn a_restored_session_brings_its_tasks_back_from_its_transcript() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("-r-api/c-9.jsonl");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let lines = [
            json!({"type": "user", "cwd": "/r/api", "sessionId": "c-9", "timestamp": "2026-11-03T10:00:00Z", "message": {"role": "user", "content": "Find the slow test"}}),
            json!({"type": "assistant", "timestamp": "2026-11-03T10:00:05Z", "message": {"id": "m1", "content": [{"type": "tool_use", "id": "toolu_agent", "name": "Agent", "input": {"description": "Find the slow test", "prompt": "Find it"}}]}}),
            json!({"type": "user", "timestamp": "2026-11-03T10:00:06Z", "message": {"role": "user", "content": [{"tool_use_id": "toolu_agent", "type": "tool_result", "content": "Async agent launched successfully."}]}, "toolUseResult": {"isAsync": true, "agentId": "a1f2e3d4"}}),
        ];
        std::fs::write(&path, lines.iter().map(Value::to_string).collect::<Vec<_>>().join("\n")).unwrap();
        let mut state = State { session_id: "c-9".into(), agent: "claude".into(), cwd: "/r/api".into(), ..State::default() };
        let t = transcripts::read_detail(Transcripts::Claude, &path, Detail::Tools).unwrap();
        state.messages = t.messages.iter().map(crate::chat::message).collect();
        state.tasks = t.tasks.iter().map(crate::chat::task_json_plain).collect();
        assert_eq!(state.tasks.len(), 1);
        assert_eq!(
            (state.tasks[0]["id"].as_str(), state.tasks[0]["kind"].as_str(), state.tasks[0]["state"].as_str(), state.tasks[0]["started"].as_str()),
            (Some("a1f2e3d4"), Some("subagent"), Some("running"), Some("2026-11-03T10:00:05Z"))
        );
        assert_eq!(state.messages[1]["tool"]["id"], "toolu_agent");
    }

    #[test]
    fn an_ask_whose_call_came_back_stops_waiting() {
        let mut state = State::default();
        take(
            &mut state,
            &json!({"type": "assistant", "message": {"content": [{"type": "tool_use", "id": "t1", "name": "Write", "input": {"file_path": "/r/x"}}]}}),
        );
        state.pending.push(json!({"id": "p1", "tool_use_id": "t1", "tool_name": "Write"}));
        take(&mut state, &json!({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": "t1", "is_error": true}]}}));
        assert!(state.pending.is_empty(), "the agent gave up on it, so the card goes with it");
    }

    #[test]
    fn an_interrupted_turn_leaves_the_session_open() {
        let mut state = State { interrupted: true, ..Default::default() };
        take(&mut state, &json!({"type": "result", "is_error": true, "subtype": "error_during_execution"}));
        assert_eq!(
            (state.ended.as_deref(), state.failed.as_deref(), state.stopped),
            (None, None, true),
            "the person stopped the turn, the agent takes the next prompt"
        );
        take(&mut state, &json!({"type": "result", "is_error": true, "result": "credit balance too low"}));
        assert_eq!((state.failed.as_deref(), state.stopped), (Some("credit balance too low"), false), "a real failure is the turn's, not the process's");
        assert_eq!(state.ended, None, "the process still runs");
    }

    #[test]
    fn the_model_is_the_one_the_agent_settled_on() {
        let state = fold(&[
            json!({"type": "system", "subtype": "init", "model": "claude-opus-5-5"}),
            json!({"type": "control_response", "response": {"subtype": "success", "request_id": "c1", "response": {"model": "claude-sonnet-5-5"}}}),
        ]);
        assert_eq!(state.model, "claude-sonnet-5-5");
    }

    #[test]
    fn the_mode_cycles_and_a_control_answer_is_believed() {
        assert_eq!(modes::next("claude", "default"), Some("acceptEdits"));
        assert_eq!(modes::next("claude", "acceptEdits"), Some("plan"));
        assert_eq!(modes::next("claude", "plan"), Some("default"));
        assert_eq!(modes::next("claude", "bypassPermissions"), Some("acceptEdits"), "a mode outside the cycle starts it again");
        let state = fold(&[json!({"type": "control_response", "response": {"subtype": "success", "request_id": "c1", "response": {"mode": "plan"}}})]);
        assert_eq!(state.mode, "plan");
    }

    const MEASURED: &str = include_str!("../tests/fixtures/claude-permission.jsonl");
    const CANCELLED: &str = include_str!("../tests/fixtures/claude-permission-cancelled.jsonl");

    fn events(text: &str) -> Vec<Value> {
        text.lines().filter(|l| !l.trim().is_empty()).map(|l| serde_json::from_str(l).expect("a JSON event")).collect()
    }

    fn can_use_tool(request_id: &str, tool_use_id: Option<&str>) -> Value {
        json!({"type": "control_request", "request_id": request_id,
               "request": {"subtype": "can_use_tool", "tool_name": "Bash", "input": {"command": "mix test"}, "tool_use_id": tool_use_id}})
    }

    #[test]
    fn the_agent_asks_on_its_own_pipe_and_the_answer_names_its_request() {
        let all = events(MEASURED);
        let at = all.iter().position(|e| e["type"] == "control_request").expect("the measured ask");
        let mut state = State::default();
        for e in &all[..=at] {
            take(&mut state, e);
        }
        assert_eq!(state.pending.len(), 1);
        let ask = state.pending[0].clone();
        assert_eq!(ask["id"], "toolu_01AcmeQuotaTests000000001", "an ask is known by its tool call");
        assert_eq!(ask["request_id"], "a3c1e2d4-5b6f-4a7b-8c9d-0e1f2a3b4c5d");
        let (reply, _) = settle(&mut state, "toolu_01AcmeQuotaTests000000001", true).expect("it is pending");
        assert_eq!(
            reply,
            json!({"type": "control_response", "response": {"subtype": "success", "request_id": "a3c1e2d4-5b6f-4a7b-8c9d-0e1f2a3b4c5d",
                   "response": {"behavior": "allow", "updatedInput": {"command": "mix test test/api/quota_test.exs", "description": "Run the quota tests"}}}}),
            "the allow goes back on the request it answers, with the ask's own arguments"
        );
        for e in &all[at + 1..] {
            take(&mut state, e);
        }
        assert!(state.pending.is_empty() && !state.working && state.ended.is_none());
        assert_eq!(state.messages.last().map(|m| m["text"].clone()), Some(json!("All 34 quota tests pass.")));
        assert_eq!(settle(&mut state, "toolu_01AcmeQuotaTests000000001", true).unwrap_err(), CHANGED, "an ask is answered once");
    }

    #[test]
    fn an_interrupt_takes_back_the_ask_it_was_waiting_on() {
        let mut state = State { interrupted: true, ..Default::default() };
        let mut notes = Vec::new();
        for e in events(CANCELLED) {
            notes.extend(take(&mut state, &e).note);
        }
        assert!(state.pending.is_empty(), "the agent cancelled it: no card may offer it");
        assert!(notes.iter().any(|n| n == "the agent took back toolu_01AcmeQuotaTests000000002"), "{notes:?}");
        assert_eq!(settle(&mut state, "toolu_01AcmeQuotaTests000000002", true).unwrap_err(), CHANGED);
        assert_eq!((state.ended.as_deref(), state.failed.as_deref(), state.stopped), (None, None, true), "an interrupted turn leaves the session open");
    }

    #[test]
    fn two_asks_in_the_same_millisecond_get_distinct_ids() {
        let mut state = State::default();
        take(&mut state, &can_use_tool("r1", Some("toolu_a")));
        take(&mut state, &can_use_tool("r2", Some("toolu_b")));
        // An agent that names neither, twice in the same instant.
        take(&mut state, &can_use_tool("r3", None));
        take(&mut state, &can_use_tool("r3", None));
        let ids: Vec<String> = state.pending.iter().map(|p| p["id"].as_str().unwrap_or_default().to_string()).collect();
        assert_eq!(&ids[..3], ["toolu_a", "toolu_b", "r3"]);
        assert!(ids[3].starts_with('p') && ids[3] != ids[2], "{ids:?}");
        // An id once settled is never handed to a new ask: a card still showing it cannot answer the new one.
        settle(&mut state, "toolu_a", false).expect("pending");
        take(&mut state, &can_use_tool("r4", Some("toolu_a")));
        assert_eq!(state.pending.last().map(|p| p["id"] == "r4"), Some(true));
    }

    #[test]
    fn an_answer_names_its_ask_or_is_refused() {
        let mut state = State::default();
        take(&mut state, &can_use_tool("r1", Some("toolu_old")));
        take(&mut state, &can_use_tool("r2", Some("toolu_new")));
        assert_eq!(settle(&mut state, "toolu_nobody_asked", true).unwrap_err(), CHANGED);
        let (deny, ask) = settle(&mut state, "toolu_new", false).expect("pending");
        assert_eq!((deny["response"]["request_id"].clone(), deny["response"]["response"]["behavior"].clone()), (json!("r2"), json!("deny")));
        assert_eq!(ask["tool_name"], "Bash");
        assert_eq!(state.pending.iter().map(|p| p["id"].clone()).collect::<Vec<_>>(), [json!("toolu_old")], "the other ask still waits");
    }

    #[test]
    fn a_control_request_cockpit_does_not_answer_is_refused_at_once() {
        let mut state = State::default();
        let taken = take(&mut state, &json!({"type": "control_request", "request_id": "r9", "request": {"subtype": "hook_callback"}}));
        let reply = taken.reply.expect("an answer, so the agent does not wait");
        assert_eq!((reply["response"]["subtype"].clone(), reply["response"]["request_id"].clone()), (json!("error"), json!("r9")));
        assert!(state.pending.is_empty());
    }

    #[test]
    fn a_driven_key_is_told_from_a_pane_id_and_a_terminal() {
        assert!(driven_key("claude:1794424409994"));
        assert!(driven_key("claude:demo-421"));
        assert!(!driven_key("w9:p1"), "a herdr pane");
        assert!(!driven_key("app:t3"), "a Herdr app terminal");
        assert!(!driven_key("claude:"));
        assert_eq!(deliver_to("w9:p1", "run the tests"), None, "not a session: the terminal takes it");
    }

    #[test]
    fn full_access_is_set_only_once_confirmed() {
        for from in ["default", "acceptEdits", "plan", "bypassPermissions", "dontAsk", "auto", ""] {
            assert_ne!(modes::next("claude", from), Some("bypassPermissions"), "the one-key cycle never reaches it from {from:?}");
        }
        cockpit_core::peer::trusted();
        let refused = |action: &str, arg: Value| act(action, &[arg]).expect("a session action").unwrap_err();
        assert!(refused("mode", json!({"key": "claude:0", "mode": "bypassPermissions"})).contains("confirm"));
        assert!(refused("start", json!({"cwd": "/r/api", "mode": "bypassPermissions"})).contains("confirm"));
        assert!(refused("mode", json!({"key": "claude:0", "mode": "auto"})).contains("confirm"), "auto runs tools without asking too");
    }

    #[test]
    fn an_answer_without_its_ask_or_its_verdict_is_refused() {
        cockpit_core::peer::trusted();
        let refused = |arg: Value| act("answer", &[arg]).expect("a session action").unwrap_err();
        assert_eq!(refused(json!({"allow": true})), CHANGED, "no ask named");
        assert!(refused(json!({"ask": "toolu_a"})).starts_with("allow or deny"), "no verdict is not an allow");
        assert_eq!(refused(json!({"ask": "toolu_nobody_asked", "allow": true})), CHANGED);
    }

    #[test]
    fn a_line_that_is_not_utf8_does_not_end_the_session() {
        let mut bytes = b"{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"s-1\"}\n".to_vec();
        bytes.extend_from_slice(b"\xff\xfe not text\n");
        bytes.extend_from_slice(b"{\"type\":\"result\",\"total_cost_usd\":0.1}\n");
        let read: Vec<String> = lines(std::io::Cursor::new(bytes)).collect();
        assert_eq!(read.len(), 3, "{read:?}");
        let mut state = State::default();
        for line in &read {
            if let Ok(event) = serde_json::from_str::<Value>(line) {
                take(&mut state, &event);
            }
        }
        assert_eq!((state.session_id.as_str(), state.cost), ("s-1", Some(0.1)), "the line after it is still read");
    }

    #[test]
    fn a_turn_that_fails_says_why_without_saying_it_twice() {
        let state = fold(&[
            json!({"type": "assistant", "message": {"content": [{"type": "text", "text": "Not logged in"}]}}),
            json!({"type": "result", "is_error": true, "result": "Not logged in"}),
        ]);
        assert_eq!(state.messages.len(), 1, "the words are already in the stream");
        assert_eq!(state.failed.as_deref(), Some("Not logged in"));
    }

    #[test]
    fn what_was_sent_waits_until_the_agent_replays_it_and_is_never_said_twice() {
        let mut state = State::default();
        state.messages.push(json!({"role": "user", "text": "Make it configurable", "echo": true}));
        take(&mut state, &json!({"type": "user", "message": {"content": [{"type": "text", "text": "Make it configurable"}]}}));
        assert_eq!(state.messages.len(), 1, "the replay is the message already there");
        assert_eq!(state.messages[0]["echo"], Value::Null, "and it no longer waits");
        // Sent while a turn runs: that turn's words are no sign it arrived, its own replay or the turn's end is.
        state.messages.push(json!({"role": "user", "text": "And the docs", "echo": true}));
        take(&mut state, &json!({"type": "assistant", "message": {"content": [{"type": "text", "text": "On it."}]}}));
        assert_eq!(state.messages[1]["echo"], true, "the turn in progress answers something else");
        take(&mut state, &json!({"type": "result", "is_error": true, "result": "the turn failed"}));
        assert_eq!(state.messages[1]["echo"], Value::Null, "a turn that ended, even failing, leaves nothing on its way");
        take(&mut state, &json!({"type": "user", "message": {"content": [{"type": "text", "text": "Something it says itself"}]}}));
        assert_eq!(state.messages.len(), 4, "words nobody sent from here are kept as they come");
    }

    #[test]
    fn a_handover_script_is_a_new_file_no_other_process_made_first() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("handover");
        // A link planted where the old name was: never written through.
        std::fs::create_dir_all(&folder).unwrap();
        let target = dir.path().join("planted");
        std::os::unix::fs::symlink(&target, folder.join("claude-0.sh")).unwrap();
        let one = handover_script_in(&folder, "claude:0", "cd '/r/api' && claude --resume a").unwrap();
        let two = handover_script_in(&folder, "claude:0", "cd '/r/api' && claude --resume b").unwrap();
        assert_ne!(one, two, "a name of each handover's own");
        assert!(!target.exists(), "nothing written through a link");
        assert_eq!(std::fs::read_to_string(&one).unwrap(), "#!/bin/sh\ncd '/r/api' && claude --resume a\n");
        assert_eq!(std::fs::metadata(&one).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::metadata(&folder).unwrap().permissions().mode() & 0o777, 0o700);
    }
}
