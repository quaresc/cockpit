//! The `agents` view the hub serves both halves: every agent terminal with its question, recap, whose move it is,
//! ticket, PR, CI, changes and time, built from what a poller keeps (the frontends' lists every two seconds, screens
//! and git on a change) and the store's documents. Its actions answer, recap, snooze, mark seen, fork, focus and stop a
//! turn (`interrupt`).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use cockpit_core::agent::Setting;
use cockpit_core::frontend::{self, APP};
use cockpit_core::peer::Who;
use cockpit_core::seen::now_ms;
use cockpit_core::transcripts::Turn;
use cockpit_core::{files, names};
use regex::Regex;
use serde_json::{Value, json};

use crate::events;
use crate::ledger::{self, Ledger};
use crate::place;
use crate::prompt::{self, Cursor, Question};
use crate::recap;
use crate::session::CHANGED;
use crate::state::{self, State};
use crate::term::{self, Live};

// ---------------------------------------------------------------------------------------------------------------------
// What the poller keeps

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Git {
    pub repo: String,
    pub branch: Option<String>,
    pub files: u64,
    pub add: u64,
    pub del: u64,
    pub ahead: u64,
    pub behind: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Agent {
    live: Live,
    question: Option<Question>,
    said: String,
}

#[derive(Default)]
struct Snap {
    polled: bool,
    agents: Vec<Agent>,
    /// cwd -> (its git, when looked at).
    git: HashMap<String, (Option<Git>, u64)>,
}

fn snap() -> MutexGuard<'static, Snap> {
    static SNAP: OnceLock<Mutex<Snap>> = OnceLock::new();
    SNAP.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner())
}

fn states() -> MutexGuard<'static, State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(State::load(&state::path()))).lock().unwrap_or_else(|e| e.into_inner())
}

fn ledger() -> MutexGuard<'static, Ledger> {
    static LEDGER: OnceLock<Mutex<Ledger>> = OnceLock::new();
    LEDGER.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner())
}

static TOUCH: OnceLock<fn()> = OnceLock::new();

// ---------------------------------------------------------------------------------------------------------------------
// A turn's end, which the queue follows

/// What a turn's end hands on: the agent's handle and how the turn ended (`done`, `failed` or `interrupted`).
pub type TurnEnd = fn(&str, &str);

static TURN_END: OnceLock<TurnEnd> = OnceLock::new();
static RESTED: OnceLock<fn(&str)> = OnceLock::new();

/// The cli says what follows a turn's end: its queue pauses after a failed or stopped one.
pub fn on_turn_end(hook: TurnEnd) {
    let _ = TURN_END.set(hook);
}

/// The cli says what to do with each agent the poller sees at rest, on every look: its queue sends the next prompt.
pub fn on_rest(hook: fn(&str)) {
    let _ = RESTED.set(hook);
}

/// A turn ended (`done`, `failed`, `interrupted`), or a driven session did (`ended`): told at once, since what follows
/// only writes the queue's document.
pub(crate) fn turn_ended(handle: &str, outcome: &'static str) {
    if let Some(hook) = TURN_END.get().copied() {
        hook(handle, outcome);
    }
}

/// How a turn ended, from an agent's status at the last look and now: None while it works on, and when it was not seen
/// before, as after a hub restart.
pub fn turn_end(before: Option<&str>, now: &str) -> Option<&'static str> {
    match (before?, now) {
        ("working", "idle" | "done") => Some("done"),
        ("working", "failed") => Some("failed"),
        _ => None,
    }
}

/// What a pane shows after a stop from cockpit `held` ms ago, and whether the stop still holds: the frontend's own `raw`
/// status out of the turn ends it; still shown working past STOP_PENDING_MS, the pane rests while its transcript's last
/// turn is the stopped one (`marker`), and a new turn there ends the hold.
pub fn after_stop(raw: &str, held: Option<u64>, marker: impl FnOnce() -> Option<bool>) -> (String, bool) {
    match held {
        None => (raw.to_string(), false),
        Some(_) if raw != "working" => (raw.to_string(), false),
        Some(age) if age <= STOP_PENDING_MS => (raw.to_string(), true),
        Some(_) => match marker() {
            Some(true) => ("done".into(), true),
            Some(false) => (raw.to_string(), false),
            None => (raw.to_string(), true),
        },
    }
}

/// A turn that ended at rest was stopped when its transcript says so (`marker`); with no transcript to read, when it was
/// stopped from cockpit (`by_hand`).
pub fn ended_as(outcome: &'static str, by_hand: bool, marker: Option<bool>) -> &'static str {
    match (outcome, marker) {
        ("done", Some(true)) => "interrupted",
        ("done", None) if by_hand => "interrupted",
        (outcome, _) => outcome,
    }
}

/// The terminals stopped from cockpit, by when: the turn that ends next for each is an interrupted one.
fn stops() -> MutexGuard<'static, HashMap<String, u64>> {
    static STOPS: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
    STOPS.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner())
}

/// How long a stop from cockpit names the turn end that follows it.
const STOP_HOLDS_MS: u64 = 60_000;
/// How long a stop from cockpit refuses another, and after which a pane still shown working is read from its transcript.
const STOP_PENDING_MS: u64 = 5_000;

/// How long an agent rests before a queued prompt goes to it on its own: a turn's end settles first.
pub const SETTLE_MS: u64 = 1_500;

/// The keys the agent table says stop an agent's turn (`["esc"]`), null where none was measured.
pub fn interrupt_of(agent: &str) -> Value {
    cockpit_core::agent::get(agent).and_then(|a| a.interrupt).map_or(Value::Null, |keys| json!(keys))
}

/// The keys that stop the turn of an agent at `status`: its agent's measured `interrupt`, and only while it works.
pub fn stop_keys(agent: Option<&cockpit_core::agent::Agent>, status: &str) -> Result<Vec<String>, String> {
    let keys = agent.and_then(|a| a.interrupt.clone()).ok_or("this agent has no known key to stop its turn")?;
    if status != "working" {
        return Err("it is not working: there is no turn to stop".into());
    }
    Ok(keys)
}

/// The view is built again for its subscribers.
fn touch() {
    if let Some(t) = TOUCH.get() {
        t();
    }
}

/// The folders the view reads: the cache, for limits.json.
pub fn dirs() -> Vec<PathBuf> {
    vec![files::cache()]
}

/// Starts the poller in the hub: `touch` asks the hub to build the view again.
pub fn start(touch: fn()) {
    if TOUCH.set(touch).is_err() {
        return;
    }
    std::thread::spawn(|| {
        ledger::trim(&ledger::path(), now_ms());
        // The sender stays here too, so the wait below times out rather than ends when nothing follows the core.
        let (tx, rx) = std::sync::mpsc::channel();
        if term::demo().is_none() {
            events::follow(tx.clone());
        }
        loop {
            let looked = std::time::Instant::now();
            if poll() {
                self::touch();
            }
            let app = frontend::app_home().join("herdr-app.sock").exists();
            let blocked = snap().agents.iter().any(|a| a.live.status == "blocked");
            let tui = app && events::following() && term::tui_running();
            if rx.recv_timeout(events::wait(app, events::following(), tui, blocked)).is_ok() {
                // A burst of changes makes one look, and looks stay a second apart at least.
                std::thread::sleep(Duration::from_secs(1).saturating_sub(looked.elapsed()).max(Duration::from_millis(150)));
                while rx.try_recv().is_ok() {}
            }
        }
    });
}

fn facts_doc() -> Value {
    files::load(&files::cache().join("agents.json")).unwrap_or(Value::Null)
}

/// Ticket keys the config and the inbox know (ENG, ...), as one pattern.
fn ticket_pattern(config: &Value, inbox: &Value) -> Option<Regex> {
    let mut keys: Vec<String> = config["tickets"]["keys"].as_array().into_iter().flatten().filter_map(|k| k.as_str()).map(str::to_uppercase).collect();
    for t in inbox["tickets"].as_array().into_iter().flatten() {
        if let Some((k, _)) = t["id"].as_str().and_then(|i| i.split_once('-')) {
            keys.push(k.to_uppercase());
        }
    }
    keys.sort();
    keys.dedup();
    if keys.is_empty() {
        return None;
    }
    Regex::new(&format!(r"(?i)(?:^|[^a-z])({})-(\d+)", keys.join("|"))).ok()
}

fn ticket_key(branch: Option<&str>, facts: &Value, pattern: Option<&Regex>) -> Option<String> {
    if let Some(t) = facts["ticket"].as_str().filter(|t| !t.is_empty()) {
        return Some(t.to_uppercase());
    }
    let caps = pattern?.captures(branch?)?;
    Some(format!("{}-{}", caps[1].to_uppercase(), &caps[2]))
}

fn git_of(cwd: &str) -> Option<Git> {
    if cwd.is_empty() || !std::path::Path::new(cwd).is_dir() {
        return None;
    }
    let git = |args: &[&str]| {
        let mut argv: Vec<String> = ["git", "-C", cwd, "--no-optional-locks", "-c", "core.fsmonitor=false"].iter().map(|s| s.to_string()).collect();
        argv.extend(args.iter().map(|a| a.to_string()));
        term::output(&argv, 5)
    };
    let status = git(&["status", "--porcelain=v2", "--branch"])?;
    let common = git(&["rev-parse", "--path-format=absolute", "--git-common-dir"]).unwrap_or_default();
    let common = common.trim().trim_end_matches('/');
    let repo = common.strip_suffix("/.git").unwrap_or(common).rsplit('/').next().unwrap_or_default().trim_end_matches(".git").to_string();
    let mut g = Git { repo, ..Default::default() };
    for line in status.lines() {
        if let Some(b) = line.strip_prefix("# branch.head ") {
            g.branch = (b != "(detached)").then(|| b.to_string());
        } else if let Some(ab) = line.strip_prefix("# branch.ab ") {
            let mut it = ab.split(' ').map(|n| n.trim_start_matches(['+', '-']).parse::<u64>().unwrap_or(0));
            g.ahead = it.next().unwrap_or(0);
            g.behind = it.next().unwrap_or(0);
        } else if !line.starts_with('#') && !line.is_empty() {
            g.files += 1;
        }
    }
    if g.files > 0 {
        let stat = git(&["diff", "HEAD", "--shortstat"]).unwrap_or_default();
        let num = |word: &str| stat.split(',').find(|p| p.contains(word)).and_then(|p| p.split_whitespace().next()?.parse().ok()).unwrap_or(0);
        g.add = num("insertion");
        g.del = num("deletion");
    }
    Some(g)
}

/// One look at every agent: statuses, the screens of those that changed or wait, git where something happened, the
/// ledger. True when the view would read differently.
pub fn poll() -> bool {
    static POLLING: Mutex<()> = Mutex::new(());
    let _one = POLLING.lock().unwrap_or_else(|e| e.into_inner());
    let now = now_ms();
    let lives = term::list(None);
    if let Some((spaces, panes)) = term::take_tui_listing() {
        crate::spaces::report(&spaces, &panes, &cockpit_core::actions::pinned());
    }
    let before: HashMap<String, Agent> = snap().agents.iter().map(|a| (a.live.handle.clone(), a.clone())).collect();
    let first = !snap().polled;
    let mut changed = first || before.len() != lives.len();
    let mut agents = Vec::new();
    let mut stale_git: Vec<String> = Vec::new();
    let mut ended: Vec<(String, &'static str, bool)> = Vec::new();
    let facts = facts_doc();
    {
        let mut st = states();
        for live in &lives {
            let old = before.get(&live.handle);
            let held = stops().get(&live.handle).copied();
            let mut live = live.clone();
            let (status, holds) = after_stop(&live.status, held.map(|at| now.saturating_sub(at)), || stopped_in(&live, &facts));
            if held.is_some() && !holds {
                stops().remove(&live.handle);
            }
            live.status = status;
            // A waiting agent's screen is read on every look (its menu moves as keys go in); the others' on a change.
            let mut read: Option<(Question, String)> = None;
            // A screen that could not be read says nothing: the last look's question and status stay.
            if live.status == "blocked"
                && let Some(screen) = term::screen(&live.handle, 0)
            {
                let mut q = prompt::read_for(&screen, &prompt::Shapes::of(&live.agent));
                // A plan approval shows the whole plan from the transcript: its screen scrolls only part of it.
                if q.permission
                    && let Some((plan, session)) = plan_of(&live, &facts[&live.handle])
                {
                    q.with_plan(&plan);
                    // Kept as a view of the session, where the side panel shows it whole and keeps it once carried out.
                    if q.markdown && live.frontend == "app" {
                        q.view = crate::plans::record(&live.address, &session, &live.cwd, &plan);
                    }
                }
                // The frontend's word can stay blocked after the prompt went: the screen and the transcript say what it does.
                if let Some(status) = past_prompt(&q, prompt::menu_shown(&screen), || turn_of(&live)) {
                    live.status = status.into();
                }
                read = Some((q, prompt::last_message(&screen)));
            }
            let live = &live;
            let moved = st.observe(&live.handle, &live.status, live.since, now);
            let mut a = old.cloned().unwrap_or_default();
            let status_changed = old.is_none_or(|o| o.live.status != live.status);
            let unread = live.status == "blocked" && read.is_none();
            if let Some((q, said)) = read.take().filter(|_| live.status == "blocked") {
                count_of(&live.handle, &q);
                // What was dismissed was this prompt: another one brings the agent back among what needs the person.
                changed |= st.asks(&live.handle, &q.identity());
                // Another agent in a reused terminal is not the one dismissed.
                changed |= st.runs(&live.handle, &runs_as(live));
                a.question = Some(q);
                a.said = said;
            } else if status_changed && !unread {
                // It stopped waiting, answered at its terminal or anywhere else: its next prompt is another one.
                if old.is_some_and(|o| o.live.status == "blocked") {
                    answered(&live.handle);
                }
                a.question = None;
                a.said = term::screen(&live.handle, 20).map(|s| prompt::last_message(&s)).unwrap_or_default();
            }
            if status_changed {
                stale_git.push(live.cwd.clone());
            }
            changed |= moved || status_changed || old.is_none_or(|o| o.question != a.question || o.said != a.said || o.live != *live);
            if let Some(outcome) = turn_end(old.map(|o| o.live.status.as_str()), &live.status) {
                ended.push((live.handle.clone(), outcome, held.is_some_and(|at| now.saturating_sub(at) < STOP_HOLDS_MS)));
            }
            a.live = live.clone();
            agents.push(a);
        }
        if changed {
            st.prune(now);
            st.save(&state::path());
        }
    }
    // Git: where a status moved, and each place once a minute.
    let cwds: Vec<String> = {
        let s = snap();
        let driven = crate::session::keys().into_iter().filter_map(|k| crate::session::summary(&k)).filter_map(|s| s["cwd"].as_str().map(str::to_string));
        let mut c: Vec<String> = lives.iter().map(|l| l.cwd.clone()).chain(driven).filter(|c| !c.is_empty()).collect();
        c.sort();
        c.dedup();
        c.into_iter().filter(|c| stale_git.contains(c) || s.git.get(c).is_none_or(|(_, at)| now.saturating_sub(*at) > 60_000)).collect()
    };
    let looked: Vec<(String, Option<Git>)> = if term::demo_path().is_some() { Vec::new() } else { cwds.into_iter().map(|c| (c.clone(), git_of(&c))).collect() };
    {
        let mut s = snap();
        for (cwd, g) in looked {
            changed |= s.git.get(&cwd).is_none_or(|(old, _)| *old != g);
            s.git.insert(cwd, (g, now));
        }
        s.agents = agents;
        s.polled = true;
    }
    // The ledger: who works on what.
    let (config, inbox) = (files::config(), files::load(&files::cache().join("inbox.json")).unwrap_or(Value::Null));
    let pattern = ticket_pattern(&config, &inbox);
    let working: Vec<(String, String)> = {
        let s = snap();
        s.agents
            .iter()
            .filter(|a| a.live.status == "working")
            .map(|a| {
                let branch = s.git.get(&a.live.cwd).and_then(|(g, _)| g.as_ref()?.branch.clone());
                let key = ticket_key(branch.as_deref(), &facts[&a.live.handle], pattern.as_ref()).unwrap_or_default();
                (a.live.handle.clone(), key)
            })
            .collect()
    };
    if term::demo_path().is_none() {
        ledger().tick(&ledger::path(), &working, now);
    }
    auto_recap(&before, &facts, now);
    tell_failed(&before, &snap().agents.clone(), first);
    for (handle, outcome, by_hand) in ended {
        let live = lives.iter().find(|l| l.handle == handle);
        // One turn's end is told once: a status that flaps over the same transcript tells nothing new.
        if live.and_then(|l| transcript_stamp(l, &facts)).is_some_and(|stamp| !first_end(&handle, stamp, outcome)) {
            continue;
        }
        let marker = (outcome == "done").then(|| live.and_then(|l| stopped_in(l, &facts))).flatten();
        turn_ended(&handle, ended_as(outcome, by_hand, marker));
    }
    // Every agent at rest, terminal or driven, is offered to what follows a rest: taken out first, as it looks again.
    if let Some(hook) = RESTED.get().copied() {
        let resting: Vec<String> = snap().agents.iter().filter(|a| matches!(a.live.status.as_str(), "idle" | "done")).map(|a| a.live.handle.clone()).collect();
        for handle in resting.iter().chain(&crate::session::keys()) {
            hook(handle);
        }
    }
    changed
}

/// What a terminal its frontend still calls blocked does now, once its screen shows no prompt: done when its transcript
/// closed the turn, working or at rest when the person's answer is the last thing above its box. None while it shows a
/// menu (`menu`: read or not) or says it reached a limit, and when nothing tells.
pub fn past_prompt(q: &Question, menu: bool, turn: impl FnOnce() -> Option<Turn>) -> Option<&'static str> {
    if menu || !q.choices.is_empty() || q.limit.is_some() {
        return None;
    }
    match (turn(), q.replied) {
        (Some(Turn::Done | Turn::Interrupted), _) => Some("done"),
        (Some(Turn::Working), true) => Some("working"),
        (None, true) => Some("idle"),
        _ => None,
    }
}

/// Where the agent stands in its session, from its transcript's end: None when it has none to read.
fn turn_of(live: &Live) -> Option<Turn> {
    let agent = cockpit_core::agent::of_program(&live.agent).or_else(|| cockpit_core::agent::of_program(&live.program))?;
    cockpit_core::transcripts::turn(agent.transcripts, std::path::Path::new(live.transcript.as_deref()?))
}

/// The plan the agent asks to carry out, whole, from its transcript, where its agent's table says it is kept; with the
/// session it belongs to (the transcript's own name when the session's id is not known).
fn plan_of(live: &Live, facts: &Value) -> Option<(String, String)> {
    let agent = cockpit_core::agent::of_program(&live.agent).or_else(|| cockpit_core::agent::of_program(&live.program))?;
    let session = session_of(live, facts);
    let transcript = live.transcript.as_ref().map(PathBuf::from).or_else(|| transcript_of(live, facts, session.as_deref()?))?;
    let plan = cockpit_core::transcripts::plan(agent.plan?, agent.transcripts, &transcript)?;
    let session = session.or_else(|| transcript.file_stem().map(|s| s.to_string_lossy().into_owned()))?;
    Some((plan, session))
}

/// The argument of the tool `agent` asks through before it carries a plan out, when `tool` is that one.
fn plan_arg(agent: Option<&cockpit_core::agent::Agent>, tool: &str) -> Option<&'static str> {
    match agent?.plan? {
        cockpit_core::agent::Plan::Argument { tool: t, arg } if t == tool => Some(arg),
        _ => None,
    }
}

/// What a blocked agent waits on when it asks nothing cockpit can read: a limit it reached, else its terminal. When the
/// limit ends comes from its usage windows (`says`) when they speak, else from its screen.
pub fn waits_json(limit: Option<&prompt::Limit>, says: &Says, zone: Option<&str>) -> Value {
    let Some(limit) = limit else { return json!({"kind": "terminal", "text": WAITS_AT_TERMINAL, "resets": null}) };
    let resets = match says {
        Says::Full(at) => Some(at.clone()),
        Says::Clear => None,
        Says::Nothing => limit.reset.as_ref().map(|at| match limit.zone.as_deref().filter(|z| Some(*z) != zone) {
            Some(other) => format!("{at} ({other})"),
            None => at.clone(),
        }),
    };
    let text = match &resets {
        Some(at) => format!("{REACHED_LIMIT} · resets {at}"),
        None => REACHED_LIMIT.to_string(),
    };
    json!({"kind": "limit", "text": text, "resets": resets})
}

/// What every surface says of a blocked agent with no question to read, and of one stopped at a limit.
pub const WAITS_AT_TERMINAL: &str = "Waiting at its terminal";
pub const REACHED_LIMIT: &str = "Reached its usage limit";

/// What an agent's usage windows say of its limit: nothing (none read), full until a time, or clear (none full now).
#[derive(Clone, Debug, PartialEq)]
pub enum Says {
    Nothing,
    Full(String),
    Clear,
}

/// What `agent`'s usage windows say of its limit: the last reset among the full ones, as "08:00" today and
/// "12 Oct 08:00" another day, as the screen's are written.
pub fn limit_says(providers: &[cockpit_core::usage::Provider], agent: &str, now: u64) -> Says {
    use chrono::{Local, TimeZone};
    let Some(windows) = providers.iter().find(|p| p.agent == agent).map(|p| &p.windows).filter(|w| !w.is_empty()) else { return Says::Nothing };
    let Some(at) = windows.iter().filter(|w| w.tightness() >= 100.0).filter_map(|w| w.resets_at).filter(|at| *at > now).max() else { return Says::Clear };
    let (Some(t), Some(n)) = (Local.timestamp_millis_opt(at as i64).single(), Local.timestamp_millis_opt(now as i64).single()) else { return Says::Nothing };
    Says::Full(if t.date_naive() == n.date_naive() { t.format("%H:%M").to_string() } else { t.format("%-d %b %H:%M").to_string() })
}

/// A transcript as it stands: its path, size and time.
type Stamp = (PathBuf, u64, std::time::SystemTime);

/// The agent's transcript as it stands: None when there is none to read.
fn transcript_stamp(live: &Live, facts: &Value) -> Option<Stamp> {
    let f = &facts[&live.handle];
    let path = session_of(live, f).and_then(|s| transcript_of(live, f, &s))?;
    let m = std::fs::metadata(&path).ok()?;
    Some((path, m.len(), m.modified().ok()?))
}

/// Whether a turn's end `outcome` at `stamp` is the first such told for the agent at `handle` with its transcript as it
/// stands: another outcome over the same transcript (done after failed) is told.
fn first_end(handle: &str, stamp: Stamp, outcome: &str) -> bool {
    static TOLD: OnceLock<Mutex<HashMap<String, (Stamp, String)>>> = OnceLock::new();
    let mut told = TOLD.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    if told.get(handle).is_some_and(|(s, o)| *s == stamp && o == outcome) {
        return false;
    }
    told.insert(handle.to_string(), (stamp, outcome.to_string()));
    true
}

/// Whether the agent's transcript says its last turn was stopped: None when there is no transcript to read.
fn stopped_in(live: &Live, facts: &Value) -> Option<bool> {
    let f = &facts[&live.handle];
    let path = session_of(live, f).and_then(|s| transcript_of(live, f, &s))?;
    Some(stopped(&live.agent, &path))
}

/// A driven session's transcript, in its agent's own format: kept once found, looked for again at most every 10 s.
fn driven_transcript(agent: &str, session: &str, cwd: &str) -> Option<PathBuf> {
    type Found = HashMap<String, (Option<PathBuf>, u64)>;
    static FOUND: OnceLock<Mutex<Found>> = OnceLock::new();
    let now = now_ms();
    let mut found = FOUND.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    if let Some((path, at)) = found.get(session)
        && (path.as_ref().is_some_and(|p| p.is_file()) || (path.is_none() && now.saturating_sub(*at) < 10_000))
    {
        return path.clone();
    }
    let kind = cockpit_core::agent::get(agent).map(|a| a.transcripts)?;
    let path = cockpit_core::transcripts::find(kind, session, cwd);
    if found.len() > 200 {
        found.clear();
    }
    found.insert(session.to_string(), (path.clone(), now));
    path
}

/// Whether a queued prompt may go to `handle` now, Err saying why not: it rests since after `after` (ms) for SETTLE_MS,
/// shows no menu (a screen it cannot read is a no) and has nothing typed in a pane's box. A driven session that is paused
/// or whose agent ended is ready only when the person `asked`, since its next prompt picks it up.
pub fn ready(handle: &str, after: u64, asked: bool) -> Result<(), String> {
    let now = now_ms();
    let settled = |since: u64| since > after && now.saturating_sub(since) >= SETTLE_MS;
    if let Some(s) = crate::session::summary(handle) {
        // A session that never had a turn has none to settle.
        let fresh = s["since"].as_u64().unwrap_or(0) == 0 && after == 0;
        return match () {
            _ if s["working"] == true => Err("it is working".into()),
            _ if s["pending"].as_array().is_some_and(|p| !p.is_empty()) => Err("it waits on a question".into()),
            _ if s["paused"] == true || !s["ended"].is_null() => match asked {
                true => Ok(()),
                false => Err("its agent is not running: the next prompt by hand picks it up".into()),
            },
            _ if !fresh && !settled(s["since"].as_u64().unwrap_or(0)) => Err("its turn has just ended".into()),
            _ => Ok(()),
        };
    }
    let a = find(handle).ok_or("no such agent")?;
    if !matches!(a.live.status.as_str(), "idle" | "done") {
        return Err(format!("it is {}", a.live.status));
    }
    if !settled(states().agents.get(handle).map_or(0, |e| e.since)) {
        return Err("its turn has just ended".into());
    }
    let screen = term::screen(handle, 0).ok_or("its screen cannot be read")?;
    if prompt::menu_shown(&screen) {
        return Err("it shows a menu".into());
    }
    // An app terminal's send waits out a draft; a herdr pane's would type after it.
    if !handle.starts_with(APP) && prompt::draft(&screen).is_some_and(|d| !d.is_empty()) {
        return Err("something is typed in its box".into());
    }
    Ok(())
}

/// Who a queue was written for: a driven session's id, else the agent and folder a terminal runs.
pub fn owner(handle: &str) -> Option<String> {
    if let Some(s) = crate::session::summary(handle) {
        return s["session"].as_str().filter(|id| !id.is_empty()).map(str::to_string);
    }
    find(handle).filter(|a| !a.live.agent.is_empty()).map(|a| format!("{}@{}", a.live.agent, a.live.cwd))
}

/// How long a finished turn waits for a look before its recap is asked for.
const RECAP_AFTER_MS: u64 = 60_000;

/// Recaps an agent's session on haiku a minute after its turn ends (it asks, or it is done) when nobody looked at it in
/// the meantime and the transcript moved since the last; `agents.recap: "manual"` leaves recaps to the person.
fn auto_recap(before: &HashMap<String, Agent>, facts: &Value, now: u64) {
    static ENDED: Mutex<Option<HashMap<String, u64>>> = Mutex::new(None);
    if files::config()["agents"]["recap"].as_str() == Some("manual") || term::demo_path().is_some() {
        return;
    }
    let s = snap();
    let mut guard = ENDED.lock().unwrap_or_else(|e| e.into_inner());
    let ended = guard.get_or_insert_with(HashMap::new);
    for a in &s.agents {
        if before.get(&a.live.handle).is_some_and(|o| o.live.status == "working" && a.live.status != "working") {
            ended.insert(a.live.handle.clone(), now);
        }
    }
    let st = states();
    ended.retain(|handle, at| {
        let Some(a) = s.agents.iter().find(|a| a.live.handle == *handle && a.live.status != "working") else { return false };
        if now.saturating_sub(*at) < RECAP_AFTER_MS {
            return true;
        }
        let looked = st.agents.get(handle).is_some_and(|e| e.seen());
        if let (false, Some(session)) = (looked, session_of(&a.live, &facts[handle]))
            && let Some(path) = transcript_of(&a.live, &facts[handle], &session)
            && !recap::fresh(&session, &path)
        {
            recap::start(&session, &a.live.cwd, path, Some("haiku"), touch);
        }
        false
    });
}

fn session_of(live: &Live, facts: &Value) -> Option<String> {
    facts["session"].as_str().filter(|s| !s.is_empty()).map(str::to_string).or_else(|| live.session.clone())
}

fn transcript_of(live: &Live, facts: &Value, session: &str) -> Option<PathBuf> {
    facts["transcript"].as_str().map(PathBuf::from).filter(|p| p.exists()).or_else(|| located(session, facts["cwd"].as_str().unwrap_or(&live.cwd)))
}

/// How long a session whose transcript was not found is not looked for again, in ms.
const UNFOUND_MS: u64 = 5_000;

/// A session's transcript in `cwd`: looked for again only once the file found is gone, or UNFOUND_MS after none was,
/// since a look can list every folder an agent keeps transcripts in.
fn located(session: &str, cwd: &str) -> Option<PathBuf> {
    located_with(session, cwd, now_ms(), || recap::transcript(session, cwd))
}

/// `located` at `now` (ms), `find` looking for the transcript when nothing kept answers.
fn located_with(session: &str, cwd: &str, now: u64, find: impl FnOnce() -> Option<PathBuf>) -> Option<PathBuf> {
    type Found = HashMap<(String, String), (Option<PathBuf>, u64)>;
    static FOUND: Mutex<Option<Found>> = Mutex::new(None);
    let key = (session.to_string(), cwd.to_string());
    let known = FOUND.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|f| f.get(&key).cloned());
    match known {
        Some((Some(path), _)) if path.is_file() => return Some(path),
        Some((None, at)) if now.saturating_sub(at) < UNFOUND_MS => return None,
        _ => {}
    }
    let found = find();
    let mut held = FOUND.lock().unwrap_or_else(|e| e.into_inner());
    let all = held.get_or_insert_with(HashMap::new);
    if all.len() > 500 {
        all.clear();
    }
    all.insert(key, (found.clone(), now));
    found
}

// ---------------------------------------------------------------------------------------------------------------------
// The view

pub const STAGES: [(&str, &str); 10] = [
    ("needs_you", "Needs you"),
    ("failed", "Stopped with an error"),
    ("done", "Done"),
    ("fixing", "Needs fixing"),
    ("idle", "Idle"),
    ("working", "Working"),
    ("review", "In review"),
    ("merged", "Merged"),
    ("snoozed", "Snoozed"),
    ("dismissed", "Dismissed"),
];

/// Takes a row the person dismissed out of what needs them: its stage reads `dismissed` until it waits on something
/// else, its status staying what it is. A terminal's dismissal ends in the poller; a driven session's is told from its
/// ask here.
fn set_aside(row: &mut Value, entry: Option<&state::Entry>) {
    let Some(entry) = entry else { return };
    if !matches!(row["stage"].as_str(), Some("needs_you" | "failed")) || !state::dismissable(row["status"].as_str().unwrap_or_default()) {
        return;
    }
    let dismissed = match row["driven"] == true {
        true => entry.dismissed(row["since"].as_u64().unwrap_or(0), row["question"]["ask"].as_str().unwrap_or("failed")),
        false => entry.dismissed.as_ref().is_some_and(|d| d.since == entry.since),
    };
    if dismissed {
        row["stage"] = json!("dismissed");
        row["dismissed"] = json!({"at": entry.dismissed.as_ref().map(|d| d.at)});
        row["unread"] = json!(false);
    }
}

/// Whose move it is: its status while that says something, then its PR once it rests. Done is an attention queue: a
/// finished agent leaves it for what its PR says the moment the person opens it.
pub fn stage(status: &str, snoozed: bool, seen: bool, pr: &Value, ci: &Value, changes: &Value) -> &'static str {
    if snoozed {
        return "snoozed";
    }
    match status {
        "blocked" => return "needs_you",
        "failed" => return "failed",
        "done" if !seen => return "done",
        "working" => return "working",
        _ => {}
    }
    match pr["state"].as_str() {
        Some("merged" | "closed") if changes["files"].as_u64().unwrap_or(0) > 0 || changes["ahead"].as_u64().unwrap_or(0) > 0 => "idle",
        Some("merged" | "closed") => "merged",
        Some("open") if ci["state"] == "fail" || pr["review"] == "CHANGES_REQUESTED" => "fixing",
        Some("open") if pr["draft"] == true => "idle",
        Some("open") => "review",
        _ => "idle",
    }
}

fn pr_of(inbox: &Value, repo: &str, branch: Option<&str>) -> Value {
    let Some(branch) = branch.filter(|b| !b.is_empty()) else { return Value::Null };
    let open = inbox["mine"].as_array().into_iter().flatten().filter(|p| p["branch"] == branch);
    let found = open.clone().find(|p| p["repo"] == repo).or_else(|| open.clone().next());
    if let Some(p) = found {
        return json!({"number": p["number"], "url": p["url"], "title": p["title"], "state": "open", "draft": p["isDraft"] == true, "review": p["review"], "ci": p["ci"], "repo": p["repo"]});
    }
    let merged = inbox["merged"].as_array().into_iter().flatten().find(|p| p["branch"] == branch);
    merged.map(|p| json!({"number": p["number"], "url": p["url"], "title": p["title"], "state": "merged", "draft": false, "review": null, "ci": null, "repo": p["repo"]})).unwrap_or(Value::Null)
}

fn ci_of(ci: &Value, repo: &str, branch: Option<&str>, pr: &Value) -> Value {
    let Some(branch) = branch else { return Value::Null };
    let run = ci["runs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| r["branch"] == branch && (repo.is_empty() || r["repo"] == repo))
        .max_by_key(|r| r["createdAt"].as_str().unwrap_or_default().to_string());
    if let Some(r) = run {
        let (done, total) = (r["done"].as_u64().unwrap_or(0), r["total"].as_u64().unwrap_or(0));
        return match () {
            _ if r["isRunning"] == true => json!({"state": "running", "label": format!("CI {done}/{total}"), "done": done, "total": total}),
            _ if r["outcome"] == "failed" => {
                let job = r["failed"]["job"].as_str().unwrap_or("CI");
                json!({"state": "fail", "label": format!("{job} failing"), "done": done, "total": total})
            }
            _ => json!({"state": "pass", "label": "CI green", "done": done, "total": total}),
        };
    }
    match pr["ci"].as_str() {
        Some("SUCCESS") => json!({"state": "pass", "label": "CI green", "done": null, "total": null}),
        Some("FAILURE" | "ERROR") => json!({"state": "fail", "label": "CI failing", "done": null, "total": null}),
        Some("PENDING" | "EXPECTED") => json!({"state": "running", "label": "CI running", "done": null, "total": null}),
        _ => Value::Null,
    }
}

/// An agent's time today and its ticket's today and this week, in seconds to the nearest minute: the panel shows
/// minutes, and the view moves no more often than they do.
fn time(agent_today: u64, (ticket_today, ticket_week): (u64, u64)) -> Value {
    let minute = |secs: u64| (secs + 30) / 60 * 60;
    json!({"agent_today": minute(agent_today), "ticket_today": minute(ticket_today), "ticket_week": minute(ticket_week)})
}

fn stretches(now: u64) -> Vec<ledger::Stretch> {
    let (_, monday) = ledger::day_and_week_start(now);
    if let Some(world) = term::demo() {
        return world["ledger"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|s| ledger::Stretch {
                start: now.saturating_sub((s["start_min"].as_f64().unwrap_or(0.0) * 60_000.0) as u64),
                end: now.saturating_sub((s["end_min"].as_f64().unwrap_or(0.0) * 60_000.0) as u64),
                ticket: s["ticket"].as_str().unwrap_or_default().to_string(),
                terminal: s["terminal"].as_str().unwrap_or_default().to_string(),
            })
            .collect();
    }
    let mut all = ledger::read(&ledger::path(), monday);
    all.extend(ledger().open_stretches(now));
    all
}

/// The view model: see docs/app.md, "View `agents`".
pub fn view(args: &Value, doc: &dyn Fn(&str) -> Value) -> Value {
    if !snap().polled {
        poll();
    }
    let now = now_ms();
    let only = args["frontend"].as_str();
    let or_file = |name: &str| {
        let v = doc(name);
        if v.is_null() { files::load(&files::cache().join(format!("{name}.json"))).unwrap_or(Value::Null) } else { v }
    };
    let (inbox, ci, facts, config) = (or_file("inbox"), or_file("ci"), or_file("agents"), files::config());
    let pattern = ticket_pattern(&config, &inbox);
    let totals = ledger::totals(&stretches(now), now);
    let demo = term::demo().map(|w| w.clone());
    // Read once for every row: the agent table (the config laid over it) and the views `show` keeps.
    let table = cockpit_core::agent::in_config(&config);
    let shown = crate::links::shown();
    let (auto, given) = (names::on(&config), names::given());
    let usage = cockpit_core::usage::providers(&cockpit_core::usage::read(now), now);
    let zone = prompt::local_zone();
    let s = snap();
    let st = states();
    let mut rows: Vec<Value> = Vec::new();
    for a in s.agents.iter().filter(|a| only.is_none_or(|f| a.live.frontend == f)) {
        let l = &a.live;
        let f = &facts[&l.handle];
        let mock = demo.as_ref().and_then(|w| w["agents"].as_array()?.iter().find(|x| x["terminal"] == l.handle.as_str()).cloned()).unwrap_or(Value::Null);
        let git = s.git.get(&l.cwd).and_then(|(g, _)| g.clone());
        let branch = git
            .as_ref()
            .and_then(|g| g.branch.clone())
            .or_else(|| f["branch"].as_str().map(str::to_string))
            .or_else(|| mock["branch"].as_str().map(str::to_string));
        let repo = git
            .as_ref()
            .map(|g| g.repo.clone())
            .filter(|r| !r.is_empty())
            .or_else(|| f["repo"].as_str().map(str::to_string))
            .or_else(|| mock["repo"].as_str().map(str::to_string))
            .unwrap_or_default();
        let changes = match (&git, &mock["changes"]) {
            (Some(g), _) => json!({"files": g.files, "add": g.add, "del": g.del, "ahead": g.ahead, "behind": g.behind}),
            (None, m) if m.is_object() => m.clone(),
            _ => Value::Null,
        };
        let key = ticket_key(branch.as_deref(), f, pattern.as_ref()).or_else(|| mock["ticket"].as_str().map(str::to_string));
        let ticket = key.as_ref().map(|k| {
            let t = inbox["tickets"].as_array().into_iter().flatten().find(|t| t["id"] == k.as_str()).cloned().unwrap_or(Value::Null);
            json!({"key": k, "title": t["title"], "url": t["url"], "status": t["status"]})
        });
        let pr = pr_of(&inbox, &repo, branch.as_deref());
        let ci = ci_of(&ci, &repo, branch.as_deref(), &pr);
        let entry = st.agents.get(&l.handle).cloned().unwrap_or_default();
        let snoozed = entry.snooze.as_ref().map(|z| json!({"until": z.until, "label": state::until_label(z.until, now)}));
        let seen = entry.seen();
        let stage = stage(&l.status, snoozed.is_some(), seen, &pr, &ci, &changes);
        let unread = matches!(stage, "needs_you" | "failed" | "done") && !seen;
        let session = session_of(l, f).or_else(|| mock["session"].as_str().map(str::to_string));
        let transcript = session.as_ref().and_then(|id| transcript_of(l, f, id));
        let agent = table.iter().find(|x| x.id == l.agent || x.aliases.contains(&l.agent));
        // A turn the person stopped rests as done, and says so.
        let interrupted =
            mock["interrupted"] == true || (matches!(l.status.as_str(), "done" | "idle") && transcript.as_ref().is_some_and(|p| stopped(&l.agent, p)));
        let recap = match &session {
            Some(_) if !mock.is_null() => {
                if mock["recap"].is_string() {
                    json!({"text": mock["recap"], "at": now.saturating_sub(120_000), "fresh": true, "running": false, "error": null})
                } else {
                    Value::Null
                }
            }
            Some(id) => recap::shown(id, transcript.as_deref()),
            None => Value::Null,
        };
        let ticket_secs = key.as_ref().and_then(|k| totals.tickets.get(k)).copied().unwrap_or((0, 0));
        let (title, named_by) = name_of(l, agent, transcript.as_deref(), auto, &given);
        // The fingerprint goes out with the question and comes back with its answer, so an answer meets the question it saw.
        let blocked = l.status == "blocked";
        let asked = a.question.as_ref().filter(|q| blocked && q.asks());
        let question = asked.map(|q| asked_json(q, counted(&l.handle, q))).unwrap_or(Value::Null);
        // Blocked with nothing to answer here: it says what it waits on instead.
        let waits = if blocked && asked.is_none() {
            let says = limit_says(&usage, if l.agent.is_empty() { &l.program } else { &l.agent }, now);
            waits_json(a.question.as_ref().and_then(|q| q.limit.as_ref()), &says, zone.as_deref())
        } else {
            Value::Null
        };
        let links = crate::links::of(ticket.as_ref(), &pr, &ci, &repo, branch.as_deref(), &inbox, &config, transcript.as_deref());
        rows.push(json!({
            "terminal": l.handle, "address": l.address, "frontend": l.frontend,
            "space": {"id": l.space_id, "name": l.space_name}, "tab": {"id": l.tab_id, "name": l.tab_name},
            "title": title, "title_source": named_by.word(), "program": l.program, "agent": l.agent, "status": l.status,
            // Herdr's logo and the name people read, from the agent cockpit knows it as, else what the frontend reports.
            "icon": cockpit_core::agent::icon_in(&table, if l.agent.is_empty() { &l.program } else { &l.agent }),
            "agent_name": cockpit_core::agent::display_name_in(&table, if l.agent.is_empty() { &l.program } else { &l.agent }),
            "reports_limits": cockpit_core::agent::reports(if l.agent.is_empty() { &l.program } else { &l.agent }).is_some(),
            "since": entry.since, "waiting_since": if l.status == "blocked" { json!(entry.since) } else { Value::Null },
            "question": question, "waits": waits, "said": a.said,
            "identity": if blocked { a.question.as_ref().map_or(json!("terminal"), |q| json!(q.identity())) } else { Value::Null }, "recap": recap, "stage": stage,
            "repo": repo, "branch": branch, "cwd": l.cwd, "ticket": ticket, "pr": pr, "ci": ci, "changes": changes,
            // What the session cost so far as cockpit estimates it everywhere, never the figure the agent reports.
            "context": f["context"].as_f64().or(l.context),
            "cost": session.as_deref().and_then(cockpit_core::usage::session_estimate).or(mock["cost"].as_f64()),
            "page": f["page"].as_str().or(mock["page"].as_str()),
            "time": time(totals.agents.get(&l.handle).copied().unwrap_or(0), ticket_secs),
            "snoozed": snoozed, "dismissed": Value::Null, "seen": seen, "unread": unread, "session": session, "interrupted": interrupted,
            "stoppable": stop_keys(agent, &l.status).is_ok(), "interrupt": agent.and_then(|a| a.interrupt.as_ref()).map_or(Value::Null, |keys| json!(keys)),
            "error": if l.status == "failed" { mock["error"].clone() } else { Value::Null },
            "load": crate::load::of(l, session.as_deref()),
            "links": links, "documents": crate::links::documents(&shown, &[&l.handle, &l.address]),
            "picks": agent.map_or(Value::Null, |a| a.catalog_picks(touch)),
        }));
    }
    for summary in crate::session::keys().iter().filter_map(|key| crate::session::summary(key)) {
        let git = s.git.get(summary["cwd"].as_str().unwrap_or_default()).and_then(|(g, _)| g.as_ref());
        rows.push(driven_row(&summary, git, &inbox, &ci, pattern.as_ref(), &config));
    }
    for r in rows.iter_mut() {
        let entry = r["terminal"].as_str().and_then(|handle| st.agents.get(handle));
        r["wait"] = wait_of(r, entry);
        set_aside(r, entry);
    }
    // Whether each row's agent reports its limits here, as the usage sources read now say.
    for r in rows.iter_mut() {
        let agent = r["agent"].as_str().filter(|a| !a.is_empty()).or(r["program"].as_str()).unwrap_or_default().to_string();
        let limits = cockpit_core::usage::limits_of(&usage, &agent, now);
        (r["limits"], r["reports_limits"]) = (json!(limits), json!(limits != "unreported"));
    }
    let rank = |stage: &str| STAGES.iter().position(|(id, _)| *id == stage).unwrap_or(99);
    rows.sort_by(|x, y| {
        let (sx, sy) = (x["stage"].as_str().unwrap_or_default(), y["stage"].as_str().unwrap_or_default());
        let (tx, ty) = (x["since"].as_u64().unwrap_or(0), y["since"].as_u64().unwrap_or(0));
        // Waiting the longest first; elsewhere the latest change first.
        rank(sx).cmp(&rank(sy)).then(if matches!(sx, "needs_you" | "failed") { tx.cmp(&ty) } else { ty.cmp(&tx) })
    });
    let count = |stage: &str| rows.iter().filter(|r| r["stage"] == stage).count();
    let stages: Vec<Value> = STAGES.iter().map(|(id, label)| json!({"id": id, "label": label, "count": count(id)})).collect();
    let mut counts = serde_json::Map::new();
    for (id, _) in STAGES {
        counts.insert(id.to_string(), json!(count(id)));
    }
    counts.insert("unread".into(), json!(rows.iter().filter(|r| r["unread"] == true).count()));
    counts.insert("total".into(), json!(rows.len()));
    let next = rows.iter().find(|r| r["stage"] == "needs_you").map(|r| r["terminal"].clone()).unwrap_or(Value::Null);
    let mut tickets: Vec<Value> = Vec::new();
    for r in &rows {
        let Some(k) = r["ticket"]["key"].as_str() else { continue };
        match tickets.iter_mut().find(|t| t["key"] == k) {
            Some(t) => t["agents"].as_array_mut().expect("made below").push(r["terminal"].clone()),
            None => {
                let (today, week) = totals.tickets.get(k).copied().unwrap_or((0, 0));
                tickets.push(json!({"key": k, "title": r["ticket"]["title"], "url": r["ticket"]["url"], "status": r["ticket"]["status"], "agents": [r["terminal"]], "today": today, "week": week}));
            }
        }
    }
    let week_rows: Vec<Value> = totals.week.iter().map(|(k, secs)| json!({"key": k, "secs": secs, "total": secs.iter().sum::<u64>()})).collect();
    json!({
        "agents": rows,
        "stages": stages,
        "counts": counts,
        "next": next,
        "quota": cockpit_core::usage::quota(&usage, now),
        "tickets": tickets,
        "week": {"days": totals.days, "rows": week_rows},
    })
}

/// Whether an agent's last turn was stopped by the person, from its transcript; read again only when the file changed.
fn stopped(agent: &str, path: &std::path::Path) -> bool {
    type Seen = HashMap<PathBuf, ((u64, std::time::SystemTime), bool)>;
    static SEEN: Mutex<Option<Seen>> = Mutex::new(None);
    let Some(stamp) = std::fs::metadata(path).ok().and_then(|m| Some((m.len(), m.modified().ok()?))) else { return false };
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    let seen = seen.get_or_insert_with(HashMap::new);
    if let Some((s, v)) = seen.get(path)
        && *s == stamp
    {
        return *v;
    }
    let Some(kind) = cockpit_core::agent::get(agent).map(|a| a.transcripts) else { return false };
    let v = cockpit_core::transcripts::turn(kind, path) == Some(cockpit_core::transcripts::Turn::Interrupted);
    if seen.len() > 200 {
        seen.clear();
    }
    seen.insert(path.to_path_buf(), (stamp, v));
    v
}

/// One notification for an app terminal that stopped with an error, the first time the poller sees it so; herdr's own
/// hook tells the TUI's. The failures the first look after a start finds were told by the hub before it.
fn tell_failed(before: &HashMap<String, Agent>, agents: &[Agent], first: bool) {
    static TOLD: Mutex<Vec<(String, u64)>> = Mutex::new(Vec::new());
    if TOUCH.get().is_none() || term::demo_path().is_some() {
        return;
    }
    let st = states();
    let mut told = TOLD.lock().unwrap_or_else(|e| e.into_inner());
    let since = |handle: &str| st.agents.get(handle).map(|e| e.since).unwrap_or(0);
    for a in failures_to_tell(before, agents, &since, &mut told, first) {
        let agent = cockpit_core::agent::of_program(&a.live.program);
        let words = names::agent_names(&a.live.program, agent.as_ref());
        let known: Vec<&str> = words.iter().map(String::as_str).collect();
        let name = agent.map(|x| x.name).unwrap_or_else(|| "The agent".into());
        // A tab labelled only with what runs there says less than its space.
        let place = if names::default_label(&a.live.tab_name, &known) { &a.live.space_name } else { &a.live.tab_name };
        let target = if a.live.tab_id.is_empty() { String::new() } else { format!("tab:{}", a.live.tab_id) };
        cockpit_core::notify::post(&format!("✗ {place}"), &format!("{name} stopped with an error"), &target, &[]);
    }
}

/// The app terminals whose failure is news: failed now and not at the last look, and not told for this failure (`since`)
/// yet. On the `first` look every failure is taken as told.
fn failures_to_tell<'a>(
    before: &HashMap<String, Agent>,
    agents: &'a [Agent],
    since: &dyn Fn(&str) -> u64,
    told: &mut Vec<(String, u64)>,
    first: bool,
) -> Vec<&'a Agent> {
    let mut out = Vec::new();
    for a in agents.iter().filter(|a| a.live.status == "failed" && a.live.handle.starts_with(APP)) {
        let was = before.get(&a.live.handle).map(|o| o.live.status.as_str());
        let at = since(&a.live.handle);
        if was == Some("failed") || told.iter().any(|(h, s)| *h == a.live.handle && *s == at) {
            continue;
        }
        told.push((a.live.handle.clone(), at));
        if !first {
            out.push(a);
        }
    }
    let keep = told.len().saturating_sub(100);
    told.drain(..keep);
    out
}

/// A driven session's status (an ask first, then working, an ended or failed turn, a stopped one resting as done) and
/// since when: its oldest ask, else its start, so it takes its turn among the agents that wait.
fn driven_status(s: &Value) -> (&'static str, Option<u64>) {
    let asking = s["pending"].as_array().and_then(|p| p.first());
    let failed = s["ended"].as_str().or(s["failed"].as_str());
    let status = match (asking, s["working"] == true, failed) {
        (Some(_), _, _) => "blocked",
        (None, true, _) => "working",
        (None, false, Some(_)) => "failed",
        (None, false, None) if s["interrupted"] == true => "done",
        _ => "idle",
    };
    (status, asking.and_then(|a| a["at"].as_u64()).or(s["since"].as_u64().filter(|t| *t > 0)).or(s["started"].as_u64()))
}

/// A session cockpit drives as a row of the agents view, from its summary and the git of its folder: a permission it
/// waits on reads as its question, and its repo, branch, ticket, PR and CI are found as a terminal's are.
pub fn driven_row(s: &Value, git: Option<&Git>, inbox: &Value, ci: &Value, pattern: Option<&Regex>, config: &Value) -> Value {
    let table = cockpit_core::agent::in_config(config);
    let cwd = s["cwd"].as_str().unwrap_or_default().to_string();
    let asking = s["pending"].as_array().and_then(|p| p.first()).cloned();
    let session = s["session"].as_str().unwrap_or_default();
    let transcript = (!session.is_empty()).then(|| driven_transcript(s["agent"].as_str().unwrap_or_default(), session, &cwd)).flatten();
    let (title, named_by) = driven_name(s, transcript.as_deref(), names::on(config));
    let recap = if session.is_empty() { Value::Null } else { recap::shown(session, transcript.as_deref()) };
    let failed = s["ended"].as_str().or(s["failed"].as_str()).map(str::to_string);
    let (status, since) = driven_status(s);
    let agent = cockpit_core::agent::of_program(s["agent"].as_str().unwrap_or_default());
    let mut question = asking.clone().map(|a| permission_question(a, agent.as_ref())).unwrap_or(Value::Null);
    // A plan asked about is kept as a view of the session, as a terminal's is.
    if question["markdown"] == true {
        let plan = question["preview"].as_str().unwrap_or_default();
        question["view"] = json!(crate::plans::record(s["key"].as_str().unwrap_or_default(), session, &cwd, plan));
    }
    let branch = git.and_then(|g| g.branch.clone());
    let repo = git.map(|g| g.repo.clone()).unwrap_or_default();
    let changes = git.map_or(Value::Null, |g| json!({"files": g.files, "add": g.add, "del": g.del, "ahead": g.ahead, "behind": g.behind}));
    let ticket = ticket_key(branch.as_deref(), &Value::Null, pattern).map(|k| {
        let t = inbox["tickets"].as_array().into_iter().flatten().find(|t| t["id"] == k.as_str()).cloned().unwrap_or(Value::Null);
        json!({"key": k, "title": t["title"], "url": t["url"], "status": t["status"]})
    });
    let pr = pr_of(inbox, &repo, branch.as_deref());
    let ci = ci_of(ci, &repo, branch.as_deref(), &pr);
    let links = crate::links::of(ticket.as_ref(), &pr, &ci, &repo, branch.as_deref(), inbox, config, transcript.as_deref());
    json!({
        "terminal": s["key"], "address": s["key"], "frontend": "app", "driven": true,
        "space": {"id": "", "name": "In chat"}, "tab": {"id": "", "name": title},
        "title": title, "title_source": named_by.word(), "program": s["agent"], "agent": s["agent"], "status": status,
        "icon": cockpit_core::agent::icon_in(&table, s["agent"].as_str().unwrap_or_default()),
        "agent_name": cockpit_core::agent::display_name_in(&table, s["agent"].as_str().unwrap_or_default()),
        "reports_limits": cockpit_core::agent::reports(s["agent"].as_str().unwrap_or_default()).is_some(),
        "since": since, "waiting_since": if asking.is_some() { json!(since) } else { Value::Null },
        // A stopped turn waits for a look as done; at rest, its PR says whose move it is.
        "stage": stage(status, false, status != "done", &pr, &ci, &changes),
        "interrupted": status == "done", "error": failed, "stoppable": status == "working",
        "interrupt": interrupt_of(s["agent"].as_str().unwrap_or_default()),
        "question": question, "waits": Value::Null,
        "said": "", "recap": recap, "paused": s["paused"], "cwd": cwd, "branch": branch, "repo": repo,
        "ticket": ticket, "pr": pr, "ci": ci, "changes": changes,
        "cost": s["session"].as_str().and_then(cockpit_core::usage::session_estimate), "session": s["session"].clone(), "seen": true,
        "unread": asking_unread(s) || status == "failed",
        "links": links, "documents": crate::links::documents(&crate::links::shown(), &[s["key"].as_str().unwrap_or_default()]), "snoozed": Value::Null,
        "dismissed": Value::Null,
        "picks": driven_picks(s),
    })
}

/// What a person picks for a driven session and how each reaches it: its agent's picks, with the models the running
/// session says it can switch to where it listed them.
fn driven_picks(s: &Value) -> Value {
    let Some(agent) = cockpit_core::agent::get(s["agent"].as_str().unwrap_or_default()) else { return Value::Null };
    let mut picks = agent.catalog_picks(touch);
    if s["models"].as_array().is_some_and(|m| !m.is_empty()) {
        picks["models"] = s["models"].clone();
    }
    picks
}

fn asking_unread(session: &Value) -> bool {
    session["pending"].as_array().is_some_and(|p| !p.is_empty())
}

/// A question as the view sends it: its shape, and the fingerprint its answer must bring back.
fn asked_json(q: &Question, count: u64) -> Value {
    let mut out = q.json();
    out["asked"] = json!(q.fingerprint(count));
    out
}

/// One terminal's prompts: the shape it shows, how many it has shown (the count goes into the fingerprint, so a prompt
/// that reads like the one before it is still another one), and a new shape read once, not yet taken for a prompt.
#[derive(Default)]
struct Prompts {
    shape: String,
    count: u64,
    seen_once: Option<String>,
}

fn prompts() -> MutexGuard<'static, HashMap<String, Prompts>> {
    static PROMPTS: OnceLock<Mutex<HashMap<String, Prompts>>> = OnceLock::new();
    PROMPTS.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner())
}

/// The count of `handle`'s prompt `q`: the prompt it shows keeps it, another one takes the next once it is read twice
/// in a row, so a screen caught mid-redraw does not move the count.
fn count_of(handle: &str, q: &Question) -> u64 {
    let mut all = prompts();
    let p = all.entry(handle.to_string()).or_default();
    let shape = q.shape();
    if p.shape == shape {
        p.seen_once = None;
        return p.count;
    }
    if p.seen_once.as_deref() == Some(shape.as_str()) {
        (p.shape, p.count, p.seen_once) = (shape, p.count + 1, None);
        return p.count;
    }
    p.seen_once = Some(shape);
    p.count + 1
}

/// The count `q`'s fingerprint goes out with, the way `count_of` reads it: a shape not taken yet counts as the next.
fn counted(handle: &str, q: &Question) -> u64 {
    prompts().get(handle).map(|p| if p.shape == q.shape() { p.count } else { p.count + 1 }).unwrap_or(1)
}

/// Its prompt went (the agent left blocked): the next prompt is a new one, even one that reads the same. An answer
/// alone does not say so, since the screen may still show the prompt it answered until it redraws.
fn answered(handle: &str) {
    if let Some(p) = prompts().get_mut(handle) {
        p.shape.clear();
    }
}

/// The agent's permission prompt shapes from the table; for a program it does not know, every agent's.
#[cfg(test)]
fn shapes_of(agent: &str) -> Vec<String> {
    let all = cockpit_core::agent::all();
    match all.iter().find(|a| a.id == agent).map(|a| a.permission.clone()).filter(|p| !p.is_empty()) {
        Some(own) => own,
        None => all.into_iter().flat_map(|a| a.permission).collect(),
    }
}

/// One answer at a time per terminal: from the look at its screen to the last key, another waits, then looks again.
fn answering(handle: &str) -> Arc<Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();
    let mut all = LOCKS.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    Arc::clone(all.entry(handle.to_string()).or_default())
}

/// What only a person does (answering, a mode that runs tools unasked, starting or focusing an agent) is refused to a
/// request from an agent's process tree, from work cockpit runs for someone, and from one cockpit cannot place.
pub fn by_a_person(refused: &str) -> Result<(), String> {
    cockpit_core::peer::on_place(place);
    cockpit_core::peer::person(refused)
}

/// What an agent's own tools may do through cockpit: act on their own agent (a `show` view's answer reaches it), never
/// on another; a person may act on any.
pub fn may_touch(target: &str) -> Result<(), String> {
    cockpit_core::peer::on_place(place);
    cockpit_core::peer::may_touch(target)
}

/// Where the process `pid` stands, from its whole line of parents (see `place::walk`): the agents are looked at again
/// when it runs under an agent's program none of them is known by, so a new agent's own tools act on it at once.
pub fn place(pid: u32) -> Who {
    // A hub that has not looked at its agents yet looks now: an agent it does not know cannot be told apart.
    let polled = snap().polled;
    if !polled {
        poll();
    }
    let mut walked = walk_now(pid);
    if matches!(walked, place::Walk::Unwatched(_)) {
        poll();
        walked = walk_now(pid);
    }
    match walked {
        place::Walk::Agent(handle) => Who::Agent(handle),
        place::Walk::Unwatched(_) => Who::Agent(String::new()),
        place::Walk::Front => Who::Person,
        place::Walk::Cockpit => Who::Unplaced(format!("{UNPLACED}: cockpit runs it for an agent, a routine, a view, a click or a repo's setup")),
        place::Walk::Ssh(Some(ip)) if ip.is_loopback() && files::config()["trust_loopback_ssh"] == true => Who::Person,
        place::Walk::Ssh(Some(_)) => {
            Who::Unplaced(format!("{UNPLACED}: it comes through ssh from this machine itself (\"trust_loopback_ssh\": true in the config takes it)"))
        }
        place::Walk::Ssh(None) => Who::Unplaced(format!("{UNPLACED}: cockpit cannot read where its ssh session comes from")),
        place::Walk::Detached => Who::Unplaced(format!("{UNPLACED}: it reaches no terminal or window cockpit can check")),
        place::Walk::Replaced(app) => Who::Unplaced(format!("{UNPLACED}: {app} was updated while it runs, so cockpit cannot check it; restart {app}")),
        place::Walk::Unlisted => Who::Unplaced(format!("{UNPLACED}: the process table does not list it")),
    }
}

/// One walk from `pid` with what the poller knows now: every agent's root processes (its terminal's, its foreground's,
/// a driven session's) and every agent program's name, the table's and the agents' own.
fn walk_now(pid: u32) -> place::Walk {
    let lives: Vec<Live> = snap().agents.iter().map(|a| a.live.clone()).collect();
    let base = |p: &str| std::path::Path::new(p).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut programs: Vec<String> = cockpit_core::agent::all().into_iter().flat_map(|a| std::iter::once(base(&a.bin)).chain(a.aliases)).collect();
    let mut roots: Vec<(String, u32)> = Vec::new();
    for l in &lives {
        roots.extend(l.roots.iter().map(|p| (l.handle.clone(), *p)));
        if l.roots.is_empty()
            && let Some(root) = crate::load::root(l, l.session.as_deref())
        {
            roots.push((l.handle.clone(), root));
        }
        programs.push(base(&l.program));
    }
    roots.extend(crate::session::keyed_pids());
    programs.retain(|p| !p.is_empty());
    place::walk(pid, &place::table(), &roots, &programs, std::process::id(), &place::System::default())
}

/// What a request cockpit cannot place is told.
const UNPLACED: &str = "cockpit cannot tell whether a person sent this, so it takes no answer from it";

/// What an agent asks to be allowed, in full for the card to show: the command, the diff, the file, the plan (where
/// `plan` names its argument), the arguments of an MCP tool.
pub fn what_it_runs(tool: &str, input: &Value, plan: Option<&str>) -> String {
    let field = |k: &str| input[k].as_str().unwrap_or_default().to_string();
    let lines = |prefix: &str, text: &str| text.lines().map(|l| format!("{prefix}{l}")).collect::<Vec<_>>().join("\n");
    if let Some(arg) = plan {
        return field(arg).trim().to_string();
    }
    let shown = match tool {
        "Bash" => [field("command"), field("description")].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n"),
        "Edit" => format!("{}\n{}\n{}", field("file_path"), lines("- ", &field("old_string")), lines("+ ", &field("new_string"))),
        "MultiEdit" => {
            let edits = input["edits"].as_array().into_iter().flatten().map(|e| {
                format!("{}\n{}", lines("- ", e["old_string"].as_str().unwrap_or_default()), lines("+ ", e["new_string"].as_str().unwrap_or_default()))
            });
            std::iter::once(field("file_path")).chain(edits).collect::<Vec<_>>().join("\n")
        }
        "Write" => format!("{}\n{}", field("file_path"), field("content")),
        "NotebookEdit" => format!("{}\n{}", field("notebook_path"), field("new_source")),
        "WebFetch" => [field("url"), field("prompt")].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n"),
        _ => serde_json::to_string_pretty(input).unwrap_or_default(),
    };
    shown.trim().to_string()
}

/// A permission request in the shape the question card draws: what it wants to run, and the two answers, neither of
/// them picked. A plan, told by `agent`'s table, is drawn as Markdown.
fn permission_question(ask: Value, agent: Option<&cockpit_core::agent::Agent>) -> Value {
    let tool = ask["tool_name"].as_str().unwrap_or("A tool");
    let detail = cockpit_core::transcripts::arg_line(&ask["input"]);
    let text = if detail.is_empty() { format!("May it run {tool}?") } else { format!("May it run {tool}: {detail}?") };
    let plan = plan_arg(agent, tool);
    json!({
        "text": text, "kind": "single", "permission": true, "cursor": null, "ask": ask["id"].clone(), "unasked": [], "cut": false, "terminal_only": null,
        "preview": what_it_runs(tool, &ask["input"], plan), "markdown": plan.is_some(),
        "choices": [{"n": 1, "label": "Allow once"}, {"n": 2, "label": "Deny"}],
    })
}

// ---------------------------------------------------------------------------------------------------------------------
// Actions

fn find(handle: &str) -> Option<Agent> {
    snap().agents.iter().find(|a| a.live.handle == handle).cloned()
}

/// The status the poller last saw an agent's terminal in: None when it knows no such terminal.
pub fn status(handle: &str) -> Option<String> {
    find(handle).map(|a| a.live.status)
}

/// `deliver`'s typing into a terminal: refused while the agent waits on a question or a permission (the poller saw it
/// blocked, or its screen shows a menu), else `send` runs under the lock its answers take, so none slips in between.
pub fn waiting(handle: &str, send: &mut dyn FnMut() -> bool) -> Result<(), String> {
    let lock = answering(handle);
    let _one = lock.lock().unwrap_or_else(|e| e.into_inner());
    let known = find(handle);
    let blocked = known.as_ref().is_some_and(|a| a.live.status == "blocked");
    let screen = term::screen(handle, 0);
    // Words that go on their own look again here, under the lock: an unreadable screen or a draft in a pane's box stops
    // them.
    if AUTOMATIC.with(std::cell::Cell::get) {
        let screen = screen.as_deref().ok_or(UNREAD)?;
        if !handle.starts_with(APP) && prompt::draft(screen).is_some_and(|d| !d.is_empty()) {
            return Err(TYPED.into());
        }
    }
    let menu = screen.is_some_and(|s| prompt::menu_shown(&s));
    let program = known.as_ref().map(|a| a.live.program.clone()).unwrap_or_default();
    let name = known.as_ref().map(title_of).filter(|n| !n.is_empty() && !n.chars().all(|c| c.is_ascii_digit()));
    let name = name.or_else(|| cockpit_core::agent::of_program(&program).map(|a| a.name)).unwrap_or_else(|| "The agent".into());
    // Blocked with no question on its screen: a limit it reached, or a prompt cockpit cannot read.
    let asks = known.as_ref().and_then(|a| a.question.as_ref()).is_none_or(|q| q.asks());
    match () {
        _ if blocked && !menu && !asks => Err(format!("{name} waits at its terminal: nothing goes to it from here")),
        _ if blocked || menu => Err(cockpit_core::deliver::waiting_note(&name)),
        _ if send() => Ok(()),
        _ => Err(format!("{name} did not take it")),
    }
}

thread_local! {
    static AUTOMATIC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// What words that go on their own are told when the agent's screen cannot be read, and when something is typed in its
/// box: both pass, so the next look tries again.
const UNREAD: &str = "its screen cannot be read, so nothing goes on its own";
const TYPED: &str = "something is typed in its box";

/// Whether a refusal of words that go on their own passes by itself (the screen, a draft), so the next look tries again.
pub fn passes(refusal: &str) -> bool {
    refusal == UNREAD || refusal == TYPED
}

/// Runs `f` as words that go on their own, not typed by the person: `deliver` looks again under the answer lock before
/// typing them.
pub fn automatically<T>(f: impl FnOnce() -> T) -> T {
    AUTOMATIC.with(|a| a.set(true));
    let out = f();
    AUTOMATIC.with(|a| a.set(false));
    out
}

/// What the views call an agent, so what is said about it names it the same way.
fn title_of(a: &Agent) -> String {
    let facts = facts_doc();
    let f = &facts[&a.live.handle];
    let transcript = session_of(&a.live, f).and_then(|s| transcript_of(&a.live, f, &s));
    let agent = cockpit_core::agent::of_program(&a.live.agent).or_else(|| cockpit_core::agent::of_program(&a.live.program));
    name_of(&a.live, agent.as_ref(), transcript.as_deref(), names::on(&files::config()), &names::given()).0
}

/// An agent terminal's name and where it comes from (`names::pick`): its tab's label when the person or another tool
/// gave it, the session's own names from its transcript, its terminal's title, else its agent's name.
fn name_of(
    l: &Live,
    agent: Option<&cockpit_core::agent::Agent>,
    transcript: Option<&std::path::Path>,
    auto: bool,
    given: &HashMap<String, String>,
) -> (String, names::Source) {
    let read = transcript.filter(|_| auto).and_then(|p| names::read(transcripts_kind(agent, p), p)).unwrap_or_default();
    let words = names::agent_names(&l.program, agent);
    let known: Vec<&str> = words.iter().map(String::as_str).chain(Some(l.agent.as_str())).collect();
    let sources = names::Sources {
        label: &l.tab_name,
        given: given.get(&l.tab_id).map(String::as_str),
        renamed: read.renamed.as_deref(),
        titled: read.titled.as_deref(),
        terminal: &l.title,
        opening: read.opening.as_deref(),
        agent: known,
        agent_name: agent.map_or("", |a| a.name.as_str()),
        folder: &l.cwd,
    };
    names::pick(&sources, auto)
}

/// A driven session's name: what its transcript says, else its first prompt, else its agent's name.
fn driven_name(s: &Value, transcript: Option<&std::path::Path>, auto: bool) -> (String, names::Source) {
    let agent = cockpit_core::agent::get(s["agent"].as_str().unwrap_or_default());
    let read = transcript.filter(|_| auto).and_then(|p| names::read(transcripts_kind(agent.as_ref(), p), p)).unwrap_or_default();
    let first = s["title"].as_str().filter(|t| !t.trim().is_empty());
    let mut known: Vec<&str> = s["agent"].as_str().into_iter().collect();
    if let Some(a) = &agent {
        known.extend([a.id.as_str(), a.name.as_str()]);
    }
    let sources = names::Sources {
        renamed: read.renamed.as_deref(),
        titled: read.titled.as_deref(),
        opening: read.opening.as_deref().or(first),
        agent: known,
        agent_name: agent.as_ref().map_or("", |a| a.name.as_str()),
        folder: s["cwd"].as_str().unwrap_or_default(),
        ..Default::default()
    };
    match names::pick(&sources, auto) {
        // With names off, a session without a tab still reads as what it was asked first.
        (_, names::Source::Default) if !auto && first.is_some() => (short(first.unwrap_or_default(), 60), names::Source::Prompt),
        named => named,
    }
}

/// The format a transcript is in: its path says, else the agent's own.
fn transcripts_kind(agent: Option<&cockpit_core::agent::Agent>, path: &std::path::Path) -> cockpit_core::transcripts::Transcripts {
    cockpit_core::transcripts::kind_of(path).or(agent.map(|a| a.transcripts)).unwrap_or_default()
}

/// `text` cut to `max` characters at a word, with "…" when it was longer.
pub(crate) fn short(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max).collect();
    let at = cut.rfind(char::is_whitespace).filter(|&i| cut[..i].chars().count() > max / 2).unwrap_or(cut.len());
    format!("{}…", cut[..at].trim_end())
}

/// The screen read again: what the agent asks now.
fn reread(handle: &str) -> Question {
    let agent = find(handle).map(|a| a.live.agent).unwrap_or_default();
    let q = prompt::read_for(&term::screen(handle, 0).unwrap_or_default(), &prompt::Shapes::of(&agent));
    if let Some(a) = snap().agents.iter_mut().find(|a| a.live.handle == handle) {
        a.question = Some(q.clone());
    }
    q
}

fn settle() {
    std::thread::sleep(Duration::from_millis(250));
}

/// Answers the question on the agent's screen; see docs/app.md for the shapes. The answer carries the question it
/// answers: a driven session's `ask`, a terminal's `asked` fingerprint, checked again after every look at the screen.
fn answer(handle: &str, arg: &Value) -> Result<String, String> {
    let source = arg["source"].as_str().filter(|s| !s.is_empty()).unwrap_or("unknown");
    if crate::session::summary(handle).is_some() {
        // A permission is allowed or denied and nothing else: words, ticks or a missing choice are not an allow.
        let allow = match (arg["choice"].as_u64(), arg["text"].is_string() || arg["choices"].is_array()) {
            (Some(1), false) => true,
            (Some(2), false) => false,
            _ => return Err("a permission takes 1 to allow or 2 to deny".into()),
        };
        let ask = arg["ask"].as_str().filter(|a| !a.is_empty()).ok_or(CHANGED)?;
        return crate::session::answer(handle, ask, allow, source);
    }
    by_a_person(BY_A_PERSON)?;
    let a = find(handle).ok_or("no such agent")?;
    let name = title_of(&a);
    let asked = arg["asked"].as_str().unwrap_or_default();
    // Another answer to this terminal finishes first; this one then looks at the screen as it is after it.
    let lock = answering(handle);
    let _one = lock.lock().unwrap_or_else(|e| e.into_inner());
    // The screen still asks what the card showed; when it moved on, the views are built again and nothing is pressed.
    let same = |q: &Question| match !asked.is_empty() && q.fingerprint(count_of(handle, q)) == asked {
        true => Ok(()),
        false => {
            touch();
            Err(CHANGED.to_string())
        }
    };
    let mut q = reread(handle);
    // Nothing on its screen reads as a question, and the answer names none it was shown: a menu cockpit could not
    // read, a limit, a prompt of another shape.
    if !q.asks() && asked.is_empty() {
        let menu = term::screen(handle, 0).is_some_and(|s| prompt::menu_shown(&s));
        return Err(if menu {
            format!("{name} shows a menu cockpit cannot read: answer it in its terminal")
        } else {
            format!("{name} asks nothing cockpit can read: go to its terminal")
        });
    }
    same(&q)?;
    let text = arg["text"].as_str().map(str::trim).filter(|t| !t.is_empty());
    if q.permission {
        permitted(&q, arg)?;
    }
    // Whatever the menu, a choice that runs every tool unasked is picked only once confirmed.
    unasked(&q, arg)?;
    let picked = |n: &Value| n.as_u64().and_then(|n| u32::try_from(n).ok()).filter(|n| q.choices.iter().any(|c| c.n == *n));
    let label = |q: &Question, n: u32| q.choices.iter().find(|c| c.n == n).map(|c| c.label.clone()).unwrap_or_else(|| n.to_string());
    let note = if q.choices.is_empty() {
        let text = text.ok_or("it asks for words: send text")?;
        // A menu it could not read is still a menu: words are not its answer.
        if term::screen(handle, 0).is_some_and(|s| prompt::menu_shown(&s)) {
            return Err(format!("{name} shows a menu cockpit cannot read: answer it in its terminal"));
        }
        term::paste(handle, text, false)?;
        format!("Answered {name}: {text}")
    } else if !q.multi() {
        let n = match &arg["choice"] {
            Value::Null => q.free.filter(|_| text.is_some()).ok_or("pick a choice")?,
            choice => picked(choice).ok_or("no such choice on its screen")?,
        };
        match text {
            // The free field: its number puts the cursor in it, the paste and its Enter answer.
            Some(text) if Some(n) == q.free => {
                if q.cursor != Some(Cursor::On(n)) {
                    term::keys(handle, &[n.to_string()])?;
                }
                term::paste(handle, text, true)?;
                format!("Answered {name}: {text}")
            }
            _ => {
                let mut keys = prompt::move_keys(&q, Some(n));
                keys.push("enter".into());
                term::keys(handle, &keys)?;
                format!("Answered {name}: {}", label(&q, n))
            }
        }
    } else {
        let asked_for = arg["choices"].as_array().cloned().unwrap_or_default();
        let wanted: Vec<u32> = asked_for.iter().filter_map(picked).collect();
        if wanted.len() != asked_for.len() {
            return Err("no such choice on its screen".into());
        }
        // In a multiple choice a number toggles its box; from the free field it would be typed, so ↑ leaves it first.
        if q.cursor.is_some() && q.cursor == q.free.map(Cursor::On) {
            term::keys(handle, &["up".into()])?;
            settle();
            q = reread(handle);
            same(&q)?;
        }
        let toggles: Vec<String> =
            q.choices.iter().filter(|c| Some(c.n) != q.free && c.checked != Some(wanted.contains(&c.n))).map(|c| c.n.to_string()).collect();
        term::keys(handle, &toggles)?;
        if let (Some(text), Some(free)) = (text, q.free) {
            settle();
            q = reread(handle);
            same(&q)?;
            term::keys(handle, &prompt::move_keys(&q, Some(free)))?;
            term::paste(handle, text, true)?;
            settle();
            q = reread(handle);
            same(&q)?;
            let checked = q.choices.iter().find(|c| c.n == free).and_then(|c| c.checked) == Some(true);
            term::keys(handle, &if checked { vec!["up".into()] } else { vec!["up".into(), free.to_string()] })?;
        }
        settle();
        q = reread(handle);
        same(&q)?;
        if q.submit_after.is_some() {
            let mut keys = prompt::move_keys(&q, None);
            keys.push("enter".into());
            term::keys(handle, &keys)?;
        }
        let picked: Vec<String> = wanted.iter().map(|n| label(&q, *n)).chain(text.map(str::to_string)).collect();
        format!("Answered {name}: {}", picked.join(", "))
    };
    settle();
    poll();
    states().mark_seen(handle);
    Ok(note)
}

/// What an answer is told when it comes from an agent's own process.
pub(crate) const BY_A_PERSON: &str = "a question is answered by a person, not by an agent's tools";

/// A terminal's permission prompt is answered by the numbered choice the person picked, and nothing else: words, ticks
/// or no choice are refused, a number the prompt does not show is too, and a choice that runs tools unasked is confirmed.
fn permitted(q: &Question, arg: &Value) -> Result<(), String> {
    if arg["text"].is_string() || arg["choices"].is_array() {
        return Err("a permission takes a choice, not words".into());
    }
    let n = arg["choice"].as_u64().ok_or("a permission takes a choice")?;
    if u32::try_from(n).ok().and_then(|n| q.choices.iter().find(|c| c.n == n)).is_none() {
        return Err("no such choice on its screen".into());
    }
    // What it would run is not all on the screen read: no card showed it in full, so no card answers it.
    match q.cut || q.preview.trim().is_empty() {
        true => Err(prompt::CUT.into()),
        false => Ok(()),
    }
}

/// A picked choice that runs every tool without asking (its label, or a yes to a prompt that says so) is taken only
/// with `confirm: true`.
fn unasked(q: &Question, arg: &Value) -> Result<(), String> {
    let Some(choice) = arg["choice"].as_u64().and_then(|n| u32::try_from(n).ok()).and_then(|n| q.choices.iter().find(|c| c.n == n)) else { return Ok(()) };
    match q.unasked().contains(&choice.n) && arg["confirm"] != true {
        true => Err(format!("“{}” runs tools without asking: it is picked only once it is confirmed", choice.label)),
        false => Ok(()),
    }
}

/// Every delivery asked of the hub, taken or refused, with who asked and from where: a line in the answers log.
fn log_delivery(handle: &str, arg: &Value, done: &Result<(), String>) {
    let sender = format!("{:?}", cockpit_core::peer::current());
    let note = match done {
        Ok(()) => Ok(format!("delivered {} chars, asked by {sender}", arg["text"].as_str().unwrap_or_default().chars().count())),
        Err(why) => Err(format!("{why}, asked by {sender}")),
    };
    log_answer(handle, &json!({"source": arg["source"].as_str().unwrap_or("deliver")}), &note);
}

/// Every answer, taken or refused, with where it came from: one line in `agents/answers.log` under the cache.
fn log_answer(handle: &str, arg: &Value, done: &Result<String, String>) {
    let path = files::cache().join("agents/answers.log");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 512 * 1024) {
        let _ = std::fs::rename(&path, path.with_extension("log.1"));
    }
    let what = match (&arg["choice"], &arg["choices"], arg["text"].as_str()) {
        (Value::Number(n), _, None) => format!("choice {n}"),
        (_, Value::Array(c), _) => format!("choices {}", Value::Array(c.clone())),
        (_, _, Some(t)) => format!("words ({} chars)", t.chars().count()),
        _ => "nothing".into(),
    };
    let named = arg["ask"]
        .as_str()
        .map(|a| format!("ask {a}"))
        .or_else(|| arg["asked"].as_str().map(|a| format!("asked {a}")))
        .unwrap_or_else(|| "no question named".into());
    let outcome = match done {
        Ok(note) => format!("taken: {note}"),
        Err(why) => format!("refused: {why}"),
    };
    let source = arg["source"].as_str().unwrap_or("unknown");
    // What a client sent stays on its one line: a newline in it cannot write a line of its own.
    let line = format!("{} {handle} from {source}: {what}, {named}, {outcome}", chrono::Utc::now().to_rfc3339()).replace(['\n', '\r'], " ") + "\n";
    let _ = std::fs::create_dir_all(path.parent().unwrap_or(&path));
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        use std::io::Write;
        let _ = file.write_all(line.as_bytes());
    }
}

fn recap_now(handle: &str, force: bool) -> Result<String, String> {
    // A driven session's transcript is found by its id, under the folder it runs in.
    if let Some(s) = crate::session::summary(handle) {
        let (session, cwd) = (s["session"].as_str().unwrap_or_default(), s["cwd"].as_str().unwrap_or_default());
        let path = driven_transcript(s["agent"].as_str().unwrap_or_default(), session, cwd).ok_or("nothing to recap yet: the session has no transcript")?;
        if !force && recap::fresh(session, &path) {
            return Ok("the recap is current".into());
        }
        recap::start(session, cwd, path, None, touch);
        return Ok("recapping…".into());
    }
    let a = find(handle).ok_or("no such agent")?;
    let facts = facts_doc();
    let f = &facts[handle];
    let session = session_of(&a.live, f).ok_or("no session known for this agent yet")?;
    let path = transcript_of(&a.live, f, &session).ok_or("its transcript is not on this machine")?;
    if !force && recap::fresh(&session, &path) {
        return Ok("the recap is current".into());
    }
    recap::start(&session, &a.live.cwd, path, None, touch);
    Ok("recapping…".into())
}

/// Stops the agent's turn: a driven session by its interrupt request, a terminal by its agent's measured keys, pressed
/// between answers. The turn that ends then pauses its queue.
fn interrupt(handle: &str) -> Result<String, String> {
    if crate::session::summary(handle).is_some() {
        return crate::session::interrupt(handle).map(|()| "asked it to stop".into());
    }
    let a = find(handle).ok_or("no such agent")?;
    // Under the lock answers take: no answer is half typed when the stop key goes in.
    let lock = answering(handle);
    let _one = lock.lock().unwrap_or_else(|e| e.into_inner());
    if stops().get(handle).is_some_and(|at| now_ms().saturating_sub(*at) < STOP_PENDING_MS) {
        return Err("it is being stopped already".into());
    }
    let keys = stop_keys(cockpit_core::agent::get(&a.live.agent).as_ref(), &a.live.status)?;
    // Looked at again under the lock: the key that stops a turn would answer a question instead.
    let screen = term::screen(handle, 0).ok_or("its screen cannot be read, so nothing is pressed")?;
    if prompt::menu_shown(&screen) {
        return Err(cockpit_core::deliver::waiting_note(&title_of(&a)));
    }
    stops().insert(handle.to_string(), now_ms());
    term::keys(handle, &keys).inspect_err(|_| {
        stops().remove(handle);
    })?;
    Ok("asked it to stop".into())
}

fn fork(handle: &str, place: &str) -> Result<String, String> {
    let a = find(handle).ok_or("no such agent")?;
    let facts = facts_doc();
    let agent = cockpit_core::agent::of_program(&a.live.program).ok_or_else(|| format!("cockpit does not know the agent {}", a.live.program))?;
    let session = session_of(&a.live, &facts[handle]).ok_or_else(|| format!("no {} session known for this agent yet", agent.name))?;
    let cwd = if a.live.cwd.is_empty() { files::home().to_string_lossy().into_owned() } else { a.live.cwd.clone() };
    let argv = agent.fork_argv(&session).ok_or_else(|| format!("{} cannot fork a session", agent.name))?;
    let label = format!("fork · {}", if a.live.title.is_empty() { &a.live.tab_name } else { &a.live.title });
    let made = match handle.strip_prefix(APP) {
        Some(_) if place == "tab" => frontend::app_do("tab.new", &json!({"space": a.live.space_id, "folder": cwd, "argv": argv, "name": label})).is_some(),
        Some(terminal) => {
            frontend::app_do("pane.focus", &json!({"terminal": terminal}));
            frontend::app_do("terminal.open", &json!({"argv": argv, "folder": cwd, "place": place})).is_some()
        }
        None => {
            let line = cockpit_core::agent::shell_line(&agent.typed(&argv));
            let created = if place == "tab" {
                frontend::herdr(&["tab", "create", "--workspace", &a.live.space_id, "--cwd", &cwd, "--label", &label, "--focus"])
            } else {
                frontend::herdr(&["pane", "split", handle, "--direction", place, "--cwd", &cwd])
            };
            let pane = created
                .as_ref()
                .and_then(|c| c["root_pane"]["pane_id"].as_str().or(c["pane"]["pane_id"].as_str()).or(c["pane_id"].as_str()).map(str::to_string));
            pane.is_some_and(|p| frontend::run_in(&p, &line))
        }
    };
    if made {
        Ok(format!("forked {} into a new {}", a.live.tab_name, if place == "tab" { "tab" } else { "pane" }))
    } else {
        Err("the window did not open it".into())
    }
}

/// A model, an effort level or fast mode (`value`, a bool for fast mode) for the agent at `handle`: a driven session
/// takes it itself; a terminal's agent gets its own command typed as its next prompt, only while it rests. A setting its
/// terminal changes only by hand is refused with the command to run there.
fn pick(handle: &str, arg: &Value) -> Result<String, String> {
    let setting = arg["setting"].as_str().and_then(Setting::of).ok_or("which setting? pass {setting: model, effort or fast}")?;
    let value = &arg["value"];
    if crate::session::summary(handle).is_some() {
        return crate::session::pick(handle, setting, value);
    }
    let a = find(handle).ok_or("no such agent")?;
    let agent = cockpit_core::agent::of_program(&a.live.program)
        .or_else(|| cockpit_core::agent::get(&a.live.agent))
        .ok_or_else(|| format!("cockpit does not know the agent {}", a.live.program))?;
    let word = match setting {
        Setting::Fast => match value.as_bool() {
            Some(on) if agent.fast.is_some() => if on { "on" } else { "off" }.to_string(),
            Some(_) => return Err(format!("{} has no fast mode", agent.name)),
            None => return Err("fast mode on or off? pass {value: true} or {value: false}".into()),
        },
        // Typed after its command as one word: only a well-formed name gets there.
        Setting::Model => value
            .as_str()
            .filter(|m| cockpit_core::agent::model_id(m) && agent.takes_model(m))
            .ok_or_else(|| format!("{} takes no model {value}", agent.name))?
            .to_string(),
        Setting::Effort => value.as_str().filter(|e| agent.takes_effort(e)).ok_or_else(|| format!("{} takes no effort level {value}", agent.name))?.to_string(),
    };
    let line = match agent.typed_for(setting, &word) {
        Some(Ok(line)) => line,
        Some(Err(by_hand)) => return Err(format!("change it in {}'s terminal: {by_hand}", agent.name)),
        None => return Err(format!("{} cannot change its {} while it runs", agent.name, setting.id())),
    };
    // Typed into a turn, the command would wait as a prompt, or land in a menu.
    if !matches!(a.live.status.as_str(), "idle" | "done" | "failed") {
        return Err(format!("{} is {}: change it once it rests", agent.name, if a.live.status == "blocked" { "waiting on a question" } else { "working" }));
    }
    let done = cockpit_core::deliver::deliver(handle, &line);
    log_delivery(handle, &json!({"text": line, "source": arg["source"].as_str().unwrap_or("pick")}), &done);
    done.map(|()| format!("typed {line}"))
}

/// The agent in a terminal, as cockpit knows it, for an action typed into that terminal; never a session cockpit drives.
fn terminal_agent(handle: &str) -> Result<(Agent, cockpit_core::agent::Agent), String> {
    if crate::session::summary(handle).is_some() {
        return Err("a session cockpit drives changes it from its chat".into());
    }
    let a = find(handle).ok_or("no such agent")?;
    let agent = cockpit_core::agent::of_program(&a.live.program)
        .or_else(|| cockpit_core::agent::get(&a.live.agent))
        .ok_or_else(|| format!("cockpit does not know the agent {}", a.live.program))?;
    Ok((a, agent))
}

/// Moves an agent to permission mode `mode` in its own terminal: its cycle keys pressed until its footer shows that mode,
/// never more than once round the cycle, so a mode it does not offer leaves it where it was.
fn set_mode(handle: &str, arg: &Value) -> Result<String, String> {
    let want = arg["mode"].as_str().filter(|m| !m.is_empty()).ok_or("which mode? pass {mode}")?;
    let (a, agent) = terminal_agent(handle)?;
    let keys = agent.cycle.clone().ok_or_else(|| format!("{} has no known key to change its mode", agent.name))?;
    if a.live.status == "blocked" {
        return Err(format!("{} is waiting on a question: change its mode once it is answered", agent.name));
    }
    let shapes = prompt::Shapes::of(&a.live.agent);
    let shown = || term::screen(handle, 0).and_then(|s| prompt::activity(&s, &shapes).mode);
    let start = shown().ok_or("its footer shows no mode")?;
    let mut now = start.clone();
    while now != want {
        term::keys(handle, &keys)?;
        let moved = (0..20).find_map(|_| {
            std::thread::sleep(std::time::Duration::from_millis(100));
            shown().filter(|m| *m != now)
        });
        now = moved.ok_or("its footer did not change")?;
        if now == start {
            return Err(format!("{} offers no {want} mode here", agent.name));
        }
    }
    Ok(format!("in {want} mode"))
}

/// Turns an agent's focus view on or off in its own terminal (Claude Code's `/focus`), once it rests: typed into a turn
/// it would wait as a prompt.
fn toggle_focus(handle: &str) -> Result<String, String> {
    let (a, agent) = terminal_agent(handle)?;
    let line = agent.focus.clone().ok_or_else(|| format!("{} has no focus view", agent.name))?;
    if !matches!(a.live.status.as_str(), "idle" | "done" | "failed") {
        return Err(format!("{} is {}: change it once it rests", agent.name, if a.live.status == "blocked" { "waiting on a question" } else { "working" }));
    }
    let done = cockpit_core::deliver::deliver(handle, &line);
    log_delivery(handle, &json!({"text": line, "source": "focus"}), &done);
    done.map(|()| format!("typed {line}"))
}

/// What an agent's tools asking to dismiss a card are told.
pub(crate) const DISMISSED_BY_A_PERSON: &str = "a card is dismissed by a person, not by an agent's tools";

/// What an agent waits on now, for a dismissal to name: its status, since when, what it asks (a terminal's question as
/// `Question::identity`, a driven session's ask, "failed" for one stopped with an error) and what runs it.
fn waiting_on(handle: &str, st: &mut State, now: u64) -> Option<Waiting> {
    if let Some(s) = crate::session::summary(handle) {
        let (status, since) = driven_status(&s);
        let asked = s["pending"].as_array().and_then(|p| p.first()).and_then(|a| a["id"].as_str()).unwrap_or("failed");
        return Some(Waiting { status: status.to_string(), since: since.unwrap_or(0), asked: asked.to_string(), runs: handle.to_string() });
    }
    let a = find(handle)?;
    st.observe(handle, &a.live.status, a.live.since, now);
    let asked = match a.live.status.as_str() {
        "blocked" => a.question.as_ref().map_or_else(|| "terminal".into(), Question::identity),
        _ => "failed".into(),
    };
    let since = st.agents.get(handle).map_or(now, |e| e.since);
    Some(Waiting { runs: runs_as(&a.live), status: a.live.status, since, asked })
}

/// What an agent waits on, as `waiting_on` reads it.
struct Waiting {
    status: String,
    since: u64,
    asked: String,
    runs: String,
}

impl Waiting {
    /// The name a card sends back with its dismissal: that wait, and what it asked.
    fn named(&self) -> String {
        format!("{}:{}", self.since, self.asked)
    }
}

/// What runs in a terminal: its agent and its process (else its session).
fn runs_as(live: &Live) -> String {
    let who = if live.agent.is_empty() { &live.program } else { &live.agent };
    let run = live.pid.map(|p| p.to_string()).or_else(|| live.session.clone()).unwrap_or_default();
    format!("{who}:{run}")
}

/// The wait a row's card names when it dismisses it: null for a row with nothing to dismiss.
fn wait_of(row: &Value, entry: Option<&state::Entry>) -> Value {
    let status = row["status"].as_str().unwrap_or_default();
    if !state::dismissable(status) {
        return Value::Null;
    }
    let asked = match (row["driven"] == true, status) {
        (true, "blocked") => row["question"]["ask"].as_str().unwrap_or("failed").to_string(),
        (_, "failed") => "failed".to_string(),
        _ => row["identity"].as_str().unwrap_or("terminal").to_string(),
    };
    let since = if row["driven"] == true { row["since"].as_u64().unwrap_or(0) } else { entry.map_or(0, |e| e.since) };
    json!(format!("{since}:{asked}"))
}

/// The person dismisses what an agent waits on: it leaves what needs them until it waits on something else. `wait` is
/// what the card they acted on showed (its `wait`); another one now is refused. `undo` brings it back.
fn dismiss(handle: &str, wait: Option<&str>, undo: bool) -> Result<String, String> {
    let now = now_ms();
    let mut st = states();
    if undo {
        let was = st.restore(handle);
        st.save(&state::path());
        return if was { Ok("back in Needs you".into()) } else { Err("it was not dismissed".into()) };
    }
    let now_waits = waiting_on(handle, &mut st, now).ok_or("no such agent")?;
    if !state::dismissable(&now_waits.status) {
        return Err("it does not wait on you: there is nothing to dismiss".into());
    }
    if wait.is_some_and(|w| w != now_waits.named()) {
        touch();
        return Err(CHANGED.into());
    }
    st.dismiss(handle, &now_waits.status, now_waits.since, &now_waits.asked, &now_waits.runs, now);
    st.save(&state::path());
    Ok("dismissed".into())
}

/// The view's actions, one object argument each: answer, recap, snooze, seen, dismiss, fork, focus, interrupt, pick.
pub fn act(action: &str, args: &[Value]) -> Option<Result<String, String>> {
    let arg = args.first().cloned().unwrap_or(Value::Null);
    let handle = arg["terminal"].as_str().or(arg.as_str()).unwrap_or_default().to_string();
    let known = ["answer", "deliver", "recap", "snooze", "seen", "dismiss", "fork", "focus", "interrupt", "refresh", "pick", "mode", "focus_mode"];
    if !known.contains(&action) {
        return None;
    }
    if handle.is_empty() && action != "refresh" {
        return Some(Err("which agent? pass {terminal}".into()));
    }
    // A fork starts an agent and a focus takes the window: a person's to do. An agent's tools touch their own row only.
    let checked = match action {
        "fork" => by_a_person("an agent is started by a person, not by an agent's tools"),
        "focus" => by_a_person("the window is brought to an agent by a person, not by an agent's tools"),
        "recap" | "snooze" | "seen" => may_touch(&handle),
        // What needs the person leaves their sight only by their own hand, never by the agent that asks.
        "dismiss" => by_a_person(DISMISSED_BY_A_PERSON),
        // Fast mode spends the plan's usage faster: a person turns it on.
        "pick" if arg["setting"] == "fast" && arg["value"] == true => {
            by_a_person("fast mode is turned on by a person, not by an agent's tools").and_then(|()| may_touch(&handle))
        }
        "pick" | "focus_mode" => may_touch(&handle),
        // A mode lets an agent do more or less without asking: a person changes it.
        "mode" => by_a_person("an agent's mode is changed by a person, not by an agent's tools").and_then(|()| may_touch(&handle)),
        _ => Ok(()),
    };
    if let Err(why) = checked {
        return Some(Err(why));
    }
    let done = match action {
        "answer" => {
            let done = answer(&handle, &arg);
            log_answer(&handle, &arg, &done);
            done
        }
        // Words from a process that is not the hub: the hub looks at the agent and types them, or refuses.
        "deliver" => {
            let done = may_touch(&handle).and_then(|()| cockpit_core::deliver::deliver(&handle, arg["text"].as_str().unwrap_or_default()));
            log_delivery(&handle, &arg, &done);
            done.map(|()| "sent".into())
        }
        "recap" => recap_now(&handle, arg["force"] == true),
        "snooze" => {
            let now = now_ms();
            let until = match arg["until"].is_null() {
                true => None,
                false => match state::parse_until(&arg["until"], now) {
                    Some(u) => Some(u),
                    None => return Some(Err("snooze until when? 1h, 18:00, tomorrow, done".into())),
                },
            };
            let mut st = states();
            st.observe(&handle, &find(&handle).map(|a| a.live.status).unwrap_or_default(), None, now);
            st.snooze(&handle, until);
            st.save(&state::path());
            Ok(match until {
                None => "woken".to_string(),
                Some(u) => format!("snoozed {}", state::until_label(u, now)),
            })
        }
        "seen" => {
            let mut st = states();
            st.mark_seen(&handle);
            st.save(&state::path());
            Ok("seen".into())
        }
        "dismiss" => dismiss(&handle, arg["wait"].as_str(), arg["undo"] == true),
        "fork" => fork(&handle, arg["place"].as_str().unwrap_or("tab")),
        "interrupt" => may_touch(&handle).and_then(|()| interrupt(&handle)),
        "pick" => pick(&handle, &arg),
        "mode" => set_mode(&handle, &arg),
        "focus_mode" => toggle_focus(&handle),
        "focus" => {
            let shown = term::demo_path().is_some() || frontend::focus(&handle);
            let mut st = states();
            st.mark_seen(&handle);
            st.save(&state::path());
            if shown { Ok(String::new()) } else { Err("that agent is gone".into()) }
        }
        _ => {
            poll();
            Ok(String::new())
        }
    };
    Some(done)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failure_is_told_once_and_never_again_by_a_hub_that_starts() {
        let failed = |handle: &str| Agent { live: Live { handle: handle.into(), status: "failed".into(), ..Live::default() }, ..Agent::default() };
        let names = |found: Vec<&Agent>| found.iter().map(|a| a.live.handle.clone()).collect::<Vec<_>>();
        let (mut told, since) = (Vec::new(), |_: &str| 100);
        let now = [failed("app:t1")];
        assert!(failures_to_tell(&HashMap::new(), &now, &since, &mut told, true).is_empty(), "the first look after a start tells nothing");
        let before: HashMap<String, Agent> = now.iter().map(|a| (a.live.handle.clone(), a.clone())).collect();
        assert!(failures_to_tell(&before, &now, &since, &mut told, false).is_empty(), "still failed: nothing new");
        let more = [failed("app:t1"), failed("app:t2")];
        assert_eq!(names(failures_to_tell(&before, &more, &since, &mut told, false)), ["app:t2"], "a new failure is told");
        assert!(failures_to_tell(&HashMap::new(), &more, &since, &mut told, false).is_empty(), "and only once");
        assert!(failures_to_tell(&HashMap::new(), &[failed("w9:p1")], &since, &mut told, false).is_empty(), "herdr's own hook tells the TUI's");
    }

    /// A row names the keys that stop its agent, so a Stop can say which it sends; an agent with none measured has none.
    #[test]
    fn a_row_says_which_keys_stop_its_agent() {
        assert_eq!(interrupt_of("claude"), json!(["esc"]));
        assert_eq!(interrupt_of("opencode"), Value::Null, "not measured");
        assert_eq!(interrupt_of("nobody"), Value::Null);
        let summary = json!({"key": "claude:1794-0", "agent": "claude", "cwd": "/w/api", "session": "", "pending": [], "started": 1});
        assert_eq!(driven_row(&summary, None, &Value::Null, &Value::Null, None, &Value::Null)["interrupt"], json!(["esc"]), "a driven row too");
    }

    #[test]
    fn a_transcript_found_is_kept_while_it_is_there_and_one_not_found_is_looked_for_again_later() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s-kept.jsonl");
        std::fs::write(&path, "{}\n").unwrap();
        let looked = std::cell::Cell::new(0);
        let find = |p: Option<PathBuf>| {
            looked.set(looked.get() + 1);
            p
        };
        assert_eq!(located_with("s-kept", "/w", 1_000, || find(Some(path.clone()))), Some(path.clone()));
        assert_eq!(located_with("s-kept", "/w", 99_000, || find(None)), Some(path.clone()), "kept while the file is there");
        std::fs::remove_file(&path).unwrap();
        assert_eq!(located_with("s-kept", "/w", 99_000, || find(None)), None, "gone: looked for again");
        assert_eq!(looked.get(), 2);
        assert_eq!(located_with("s-kept", "/w", 99_000 + UNFOUND_MS - 1, || find(Some(path.clone()))), None, "not found: not looked for again at once");
        assert_eq!(looked.get(), 2);
        assert_eq!(located_with("s-kept", "/w", 99_000 + UNFOUND_MS, || find(Some(path.clone()))), Some(path), "then looked for again");
        assert_eq!(looked.get(), 3);
    }

    #[test]
    fn only_a_working_agent_with_a_measured_key_can_be_stopped() {
        let table = cockpit_core::agent::builtin();
        let get = |id: &str| table.iter().find(|a| a.id == id).cloned();
        let (claude, codex, opencode) = (get("claude"), get("codex"), get("opencode"));
        assert_eq!(stop_keys(claude.as_ref(), "working"), Ok(vec!["esc".to_string()]));
        assert_eq!(stop_keys(codex.as_ref(), "working"), Ok(vec!["esc".to_string()]));
        assert!(stop_keys(opencode.as_ref(), "working").is_err(), "not measured: no Stop");
        assert!(stop_keys(None, "working").is_err(), "an unknown program");
        for status in ["idle", "done", "blocked", "failed"] {
            assert!(stop_keys(claude.as_ref(), status).is_err(), "{status}: no turn to stop");
        }
    }

    #[test]
    fn a_turn_ends_when_a_known_agent_stops_working() {
        assert_eq!(turn_end(Some("working"), "idle"), Some("done"));
        assert_eq!(turn_end(Some("working"), "done"), Some("done"));
        assert_eq!(turn_end(Some("working"), "failed"), Some("failed"));
        assert_eq!(turn_end(Some("working"), "blocked"), None, "it asks: the turn goes on");
        assert_eq!(turn_end(None, "idle"), None, "not seen before, as after a hub restart");
        assert_eq!(turn_end(Some("idle"), "idle"), None);
    }

    #[test]
    fn a_pane_still_shown_working_after_a_stop_rests_while_its_transcript_says_so() {
        assert_eq!(after_stop("working", None, || Some(true)), ("working".into(), false), "no stop from cockpit");
        assert_eq!(after_stop("working", Some(1_000), || Some(true)), ("working".into(), true), "the frontend gets its moment");
        assert_eq!(after_stop("working", Some(9_000), || Some(true)), ("done".into(), true), "and keeps resting on every look");
        assert_eq!(after_stop("working", Some(60_000), || Some(true)), ("done".into(), true), "however long it stays so");
        assert_eq!(after_stop("working", Some(9_000), || Some(false)), ("working".into(), false), "a new turn began: its own status");
        assert_eq!(after_stop("working", Some(9_000), || None), ("working".into(), true), "nothing to read: the stop still holds");
        assert_eq!(after_stop("idle", Some(9_000), || Some(true)), ("idle".into(), false), "the frontend says it rests: done holding");
    }

    #[test]
    fn a_terminal_called_blocked_past_its_prompt_reads_from_its_screen_and_transcript() {
        let claude = prompt::Shapes::of("claude");
        let menu = prompt::read_for(include_str!("../tests/fixtures/screens/claude-bash.txt"), &claude);
        let replied = prompt::read_for(include_str!("../tests/fixtures/screens/claude-replied.txt"), &claude);
        let rested = prompt::read_for(include_str!("../tests/fixtures/screens/claude-rested.txt"), &claude);
        let limit = prompt::read_for(include_str!("../tests/fixtures/screens/claude-limit.txt"), &claude);
        let asks = prompt::read("⏺ Shall I open the PR now, or wait for the backfill?\n");
        // A menu on its screen is the prompt, whatever its transcript says; so is a limit it reached.
        assert_eq!(past_prompt(&menu, false, || Some(Turn::Done)), None);
        // A menu cockpit could not read is a prompt too: Esc, a stop or words would land in it.
        assert_eq!(past_prompt(&asks, true, || Some(Turn::Done)), None);
        assert_eq!(past_prompt(&replied, true, || Some(Turn::Working)), None);
        assert_eq!(past_prompt(&limit, false, || Some(Turn::Done)), None, "stopped at its limit: it still waits");
        // The transcript closed the turn since: it is done, not waiting.
        assert_eq!(past_prompt(&rested, false, || Some(Turn::Done)), Some("done"));
        assert_eq!(past_prompt(&asks, false, || Some(Turn::Interrupted)), Some("done"));
        // The person's answer is the last thing above its box: the prompt is gone, the transcript says what it does.
        assert_eq!(past_prompt(&replied, false, || Some(Turn::Working)), Some("working"));
        assert_eq!(past_prompt(&replied, false, || None), Some("idle"));
        assert_eq!(past_prompt(&replied, false, || Some(Turn::Blocked)), None, "its transcript records an ask: still blocked");
        // Nothing tells: the frontend's word stays.
        assert_eq!(past_prompt(&asks, false, || None), None);
        assert_eq!(past_prompt(&rested, false, || Some(Turn::Working)), None);
        assert_eq!(past_prompt(&rested, false, || None), None);
    }

    #[test]
    fn a_blocked_agent_with_nothing_to_read_says_what_it_waits_on() {
        assert_eq!(waits_json(None, &Says::Nothing, None), json!({"kind": "terminal", "text": "Waiting at its terminal", "resets": null}));
        let on_screen = prompt::Limit { reset: Some("08:00".into()), zone: None };
        assert_eq!(waits_json(Some(&on_screen), &Says::Nothing, None)["text"], "Reached its usage limit · resets 08:00");
        assert_eq!(
            waits_json(Some(&on_screen), &Says::Full("12 Oct 09:30".into()), None)["text"],
            "Reached its usage limit · resets 12 Oct 09:30",
            "its usage window knows the day"
        );
        assert_eq!(waits_json(Some(&on_screen), &Says::Clear, None)["text"], "Reached its usage limit", "its windows say nothing is full now");
        // A time in another zone than this machine's says which.
        let elsewhere = prompt::Limit { reset: Some("15:30".into()), zone: Some("America/New_York".into()) };
        assert_eq!(waits_json(Some(&elsewhere), &Says::Nothing, Some("Europe/Paris"))["resets"], "15:30 (America/New_York)");
        assert_eq!(waits_json(Some(&elsewhere), &Says::Nothing, Some("America/New_York"))["resets"], "15:30");
        assert_eq!(
            waits_json(Some(&prompt::Limit::default()), &Says::Nothing, None),
            json!({"kind": "limit", "text": "Reached its usage limit", "resets": null}),
            "no reset known: none is made up"
        );
        // Its usage windows give the reset once one is full: the last of the full ones, never one with room left.
        use chrono::{Local, TimeZone};
        let now = Local.with_ymd_and_hms(2026, 11, 2, 15, 0, 0).unwrap().timestamp_millis() as u64;
        let window = |name: &str, percent: f64, resets_in_min: u64| cockpit_core::usage::Window {
            agent: "claude".into(),
            name: name.into(),
            percent: Some(percent),
            status: None,
            resets_at: Some(now + resets_in_min * 60_000),
            length: None,
            as_of: now,
            source: "statusline",
        };
        let provider = |windows: Vec<cockpit_core::usage::Window>| cockpit_core::usage::Provider {
            agent: "claude".into(),
            name: "Claude".into(),
            letter: "C".into(),
            reports: true,
            unlimited: false,
            windows,
            identity: Value::Null,
            credits: Value::Null,
            error: None,
        };
        assert_eq!(limit_says(&[provider(vec![window("5h", 100.0, 60), window("7d", 38.0, 5000)])], "claude", now), Says::Full("16:00".into()));
        assert_eq!(limit_says(&[provider(vec![window("5h", 100.0, 60), window("7d", 100.0, 17 * 60)])], "claude", now), Says::Full("3 Nov 08:00".into()));
        assert_eq!(limit_says(&[provider(vec![window("5h", 64.0, 60)])], "claude", now), Says::Clear, "no window is full");
        assert_eq!(limit_says(&[provider(vec![window("5h", 100.0, 60)])], "codex", now), Says::Nothing, "another agent's windows");
        assert_eq!(limit_says(&[provider(vec![])], "claude", now), Says::Nothing, "no window read");
    }

    #[test]
    fn a_driven_session_stays_dismissed_until_it_asks_again() {
        let ask = |id: &str| {
            json!({"terminal": "claude:demo-421", "driven": true, "status": "blocked", "stage": "needs_you", "since": 7000, "unread": true,
                   "question": {"ask": id, "text": "May it run Bash: mix test?"}, "dismissed": null})
        };
        let mut st = State::default();
        let mut row = ask("toolu_a");
        set_aside(&mut row, st.agents.get("claude:demo-421"));
        assert_eq!(row["stage"], "needs_you", "nothing was dismissed");
        st.dismiss("claude:demo-421", "blocked", 7000, "toolu_a", "claude:demo-421", 8000);
        set_aside(&mut row, st.agents.get("claude:demo-421"));
        assert_eq!((row["stage"].as_str(), row["dismissed"]["at"].as_u64(), row["unread"].as_bool()), (Some("dismissed"), Some(8000), Some(false)));
        assert_eq!(row["status"], "blocked", "its real status stays");
        // Its next ask is another one, and so is the same ask in a later wait.
        let mut next = ask("toolu_b");
        set_aside(&mut next, st.agents.get("claude:demo-421"));
        assert_eq!((next["stage"].as_str(), next["dismissed"].clone()), (Some("needs_you"), Value::Null));
        let mut later = ask("toolu_a");
        later["since"] = json!(9000);
        set_aside(&mut later, st.agents.get("claude:demo-421"));
        assert_eq!(later["stage"], "needs_you");
        // At rest or working, a dismissal says nothing of the row.
        let mut rests = json!({"terminal": "claude:demo-421", "driven": true, "status": "idle", "stage": "idle", "since": 7000, "question": null});
        set_aside(&mut rests, st.agents.get("claude:demo-421"));
        assert_eq!(rests["stage"], "idle");
        // A session that failed is dismissed as that.
        let mut failed =
            json!({"terminal": "claude:demo-9", "driven": true, "status": "failed", "stage": "failed", "since": 100, "question": null, "unread": true});
        st.dismiss("claude:demo-9", "failed", 100, "failed", "claude:demo-9", 200);
        set_aside(&mut failed, st.agents.get("claude:demo-9"));
        assert_eq!(failed["stage"], "dismissed");
        assert_eq!(driven_status(&json!({"pending": [{"id": "toolu_a", "at": 7000}], "started": 1})), ("blocked", Some(7000)));
        assert_eq!(driven_status(&json!({"pending": [], "failed": "exit 1", "since": 40})), ("failed", Some(40)));
        assert_eq!(STAGES.last(), Some(&("dismissed", "Dismissed")), "after the snoozed ones");
    }

    #[test]
    fn a_turn_end_over_the_same_transcript_is_told_once() {
        let at = std::time::SystemTime::UNIX_EPOCH;
        let stamp = |len: u64| (PathBuf::from("/Users/me/.claude/projects/-Users-me-code-api/acme.jsonl"), len, at);
        assert!(first_end("app:t-once", stamp(10), "done"));
        assert!(!first_end("app:t-once", stamp(10), "done"), "a status that flaps tells nothing new");
        assert!(first_end("app:t-once", stamp(42), "done"), "the next turn wrote more");
        assert!(first_end("app:t-once", stamp(50), "failed"));
        assert!(first_end("app:t-once", stamp(50), "done"), "done after failed over the same transcript is told");
    }

    #[test]
    fn a_stopped_turn_is_told_by_its_transcript_first_then_by_a_stop_from_cockpit() {
        assert_eq!(ended_as("done", false, Some(true)), "interrupted", "stopped at its terminal");
        assert_eq!(ended_as("done", true, Some(false)), "done", "the transcript says it finished: the stop came too late");
        assert_eq!(ended_as("done", true, None), "interrupted", "no transcript: the stop from cockpit tells");
        assert_eq!(ended_as("done", false, None), "done");
        assert_eq!(ended_as("failed", true, Some(true)), "failed", "an error is an error");
    }

    #[test]
    fn whose_move_follows_the_status_then_the_pr() {
        let open = json!({"state": "open", "review": "REVIEW_REQUIRED"});
        assert_eq!(stage("blocked", false, false, &open, &Value::Null, &Value::Null), "needs_you");
        assert_eq!(stage("failed", false, true, &open, &Value::Null, &Value::Null), "failed", "stopped with an error, seen or not");
        assert_eq!(STAGES[1], ("failed", "Stopped with an error"), "right after needs you");
        assert_eq!(stage("blocked", true, false, &open, &Value::Null, &Value::Null), "snoozed");
        assert_eq!(stage("idle", false, false, &open, &Value::Null, &Value::Null), "review");
        assert_eq!(stage("idle", false, false, &open, &json!({"state": "fail"}), &Value::Null), "fixing");
        assert_eq!(stage("idle", false, false, &json!({"state": "merged"}), &Value::Null, &json!({"files": 0, "ahead": 0})), "merged");
        assert_eq!(stage("idle", false, false, &json!({"state": "merged"}), &Value::Null, &json!({"files": 2})), "idle");
        assert_eq!(stage("done", false, false, &open, &Value::Null, &Value::Null), "done");
        assert_eq!(stage("done", false, true, &open, &Value::Null, &Value::Null), "review", "opened, it leaves the queue");
    }

    #[test]
    fn agent_times_move_by_the_minute_they_are_shown_in() {
        assert_eq!(time(125, (61, 3_599)), json!({"agent_today": 120, "ticket_today": 60, "ticket_week": 3_600}));
        assert_eq!(time(125, (61, 3_599)), time(140, (75, 3_610)));
    }

    #[test]
    fn a_long_title_is_cut_at_a_word() {
        let long = "Make the daily quota configurable per plan, and run the quota tests when it builds";
        assert_eq!(short(long, 60), "Make the daily quota configurable per plan, and run the…");
        assert_eq!(short("Exports", 60), "Exports");
        assert_eq!(short(&"x".repeat(70), 60), format!("{}…", "x".repeat(60)), "one long word is cut where it must be");
    }

    #[test]
    fn a_prompt_is_counted_once_it_reads_the_same_twice_and_again_after_it_leaves() {
        let ask = prompt::read("Do you want to proceed?\n❯ 1. Yes\n  2. No, and tell Claude what to do differently\n");
        let misread = prompt::read("Do you want to pro\n❯ 1. Yes\n  2. No\n");
        let h = "app:t-redraw";
        let first = count_of(h, &ask);
        assert_eq!((first, counted(h, &ask)), (1, 1), "a new prompt counts as the next from its first read");
        assert_eq!(count_of(h, &ask), 1, "and keeps that count once taken");
        // A screen caught mid-redraw does not move the count of the prompt it shows.
        count_of(h, &misread);
        assert_eq!(count_of(h, &ask), 1);
        assert_eq!(ask.fingerprint(counted(h, &ask)), ask.fingerprint(count_of(h, &ask)), "the view and the answer agree");
        // Answered at the terminal: the agent left blocked, and the same ask after it is another prompt.
        answered(h);
        assert_eq!((count_of(h, &ask), count_of(h, &ask)), (2, 2));
        assert_ne!(ask.fingerprint(1), ask.fingerprint(2));
    }

    #[test]
    fn a_terminal_permission_takes_the_choice_the_person_picked() {
        let frame = "─".repeat(100);
        let q = prompt::read_as(
            &format!(
                "{frame}\nBash command\n  rm -rf build\n\nDo you want to proceed?\n❯ 1. Yes\n  2. Yes, and don't ask again for rm commands\n  3. No, and tell Claude what to do differently (esc)\n"
            ),
            &shapes_of("claude"),
        );
        assert!(q.permission);
        assert!(permitted(&q, &json!({"choice": 1})).is_ok());
        assert!(permitted(&q, &json!({"choice": 2})).is_ok(), "“Yes, and don't ask again”, picked on purpose, is taken");
        assert!(permitted(&q, &json!({"choice": 3})).is_ok(), "saying no is always taken");
        assert!(permitted(&q, &json!({"choice": 4})).is_err(), "a choice the prompt does not show");
        assert!(permitted(&q, &json!({"choice": 4_294_967_298u64})).is_err(), "a number past u32 never wraps round to 2");
        assert!(permitted(&q, &json!({"text": "yes"})).is_err(), "words are not an allow");
        assert!(permitted(&q, &json!({"choices": [1]})).is_err());
        assert!(permitted(&q, &json!({})).is_err(), "nothing picked is not an allow");
        // A plan approval that runs every tool unasked is picked only once confirmed, as the driven mode is.
        let plan = prompt::read_as(
            &format!(
                "{frame}\nHere is Claude's plan:\n1) Add the column\n\nWould you like to proceed?\n❯ 1. Yes, and bypass permissions\n  2. Yes, and manually approve edits\n  3. No, keep planning\n"
            ),
            &shapes_of("claude"),
        );
        assert!(plan.permission && permitted(&plan, &json!({"choice": 1})).is_ok());
        assert!(unasked(&plan, &json!({"choice": 1})).unwrap_err().contains("runs tools without asking"));
        assert!(unasked(&plan, &json!({"choice": 1, "confirm": true})).is_ok());
        assert!(unasked(&plan, &json!({"choice": 2})).is_ok());
        // Codex's full access and Claude's bypass-mode warning, menu or not, and a plain yes to a prompt about one.
        let codex = prompt::read("Allow Codex to work in this folder?\n❯ 1. Yes, and allow Codex to work without asking for approval\n  2. No\n");
        assert!(unasked(&codex, &json!({"choice": 1})).is_err());
        let warning = prompt::read("WARNING: Claude Code running in Bypass Permissions mode\n❯ 1. No, exit\n  2. Yes, I accept\n");
        assert!(unasked(&warning, &json!({"choice": 2})).is_err() && unasked(&warning, &json!({"choice": 1})).is_ok());
        // A permission whose screen showed nothing of what it would run is answered at the agent, not from a card.
        let bare = prompt::read_as("Do you want to proceed?\n❯ 1. Yes\n  2. No, and tell Claude what to do differently (esc)\n", &shapes_of("claude"));
        assert_eq!(permitted(&bare, &json!({"choice": 2})).unwrap_err(), prompt::CUT);
        // So is one whose frame is off the screen, whatever line reads like its question.
        let unframed = prompt::read_as("  rm -rf ~\nDo you want to proceed?\n  echo hi\nDo you want to proceed?\n❯ 1. Yes\n  2. No\n", &shapes_of("claude"));
        assert_eq!(permitted(&unframed, &json!({"choice": 1})).unwrap_err(), prompt::CUT);
        // Codex's full access, by its label or its detail, is a step of its own on every surface.
        let approvals = prompt::read("Select Approval Mode\n❯ 1. Read Only\n  2. Ask\n  3. Full Access\n");
        assert_eq!(approvals.unasked(), [3]);
        assert_eq!(approvals.json()["unasked"], json!([3]));
        assert_eq!(plan.unasked(), [1], "bypass is; manual approval is not");
    }

    #[test]
    fn a_driven_ask_is_drawn_with_nothing_picked_and_all_it_would_do() {
        let claude = cockpit_core::agent::of_program("claude");
        let q = permission_question(
            json!({"id": "toolu_a", "tool_name": "Bash", "input": {"command": "mix test", "description": "Run the tests"}}),
            claude.as_ref(),
        );
        assert_eq!((q["cursor"].clone(), q["ask"].clone(), q["permission"].clone()), (Value::Null, json!("toolu_a"), json!(true)));
        assert_eq!(q["preview"], "mix test\nRun the tests");
        assert_eq!(q["markdown"], false);
        let plan =
            permission_question(json!({"id": "toolu_p", "tool_name": "ExitPlanMode", "input": {"plan": "# Quota\n\n1. Add the column"}}), claude.as_ref());
        assert_eq!((plan["preview"].clone(), plan["markdown"].clone()), (json!("# Quota\n\n1. Add the column"), json!(true)), "a plan is drawn as Markdown");
        let unknown = permission_question(json!({"id": "toolu_p", "tool_name": "ExitPlanMode", "input": {"plan": "# Quota"}}), None);
        assert_eq!(unknown["markdown"], false, "an agent with no plan tool in the table shows the input as it is");
        let edit = what_it_runs("Edit", &json!({"file_path": "/r/api/lib/quota.ex", "old_string": "@limit 100", "new_string": "@limit 200"}), None);
        assert_eq!(edit, "/r/api/lib/quota.ex\n- @limit 100\n+ @limit 200");
        assert_eq!(what_it_runs("ExitPlanMode", &json!({"plan": "1. Add the column\n2. Read it"}), Some("plan")), "1. Add the column\n2. Read it");
        assert!(what_it_runs("mcp__acme__deploy", &json!({"service": "api", "env": "prod"}), None).contains("\"env\": \"prod\""), "an MCP tool's whole input");
    }

    #[test]
    fn a_prompt_that_reads_like_the_last_one_is_counted_as_another() {
        let q = prompt::read("Which limit should existing accounts start with?\n❯ 1. The plan's default\n  2. Their peak\n");
        let first = count_of("app:t-count", &q);
        assert_eq!(count_of("app:t-count", &q), first, "the same prompt, seen again, keeps its count");
        answered("app:t-count");
        assert_eq!(count_of("app:t-count", &q), first + 1, "after an answer the same words are a new prompt");
        assert_ne!(q.fingerprint(first), q.fingerprint(first + 1));
    }

    #[test]
    fn a_ticket_comes_from_the_facts_else_the_branch() {
        let p = ticket_pattern(&json!({"tickets": {"keys": ["ENG"]}}), &Value::Null);
        assert_eq!(ticket_key(Some("me/eng-412-daily-quotas"), &Value::Null, p.as_ref()).as_deref(), Some("ENG-412"));
        assert_eq!(ticket_key(Some("main"), &json!({"ticket": "eng-9"}), p.as_ref()).as_deref(), Some("ENG-9"));
        assert_eq!(ticket_key(Some("main"), &Value::Null, p.as_ref()), None);
    }
}
