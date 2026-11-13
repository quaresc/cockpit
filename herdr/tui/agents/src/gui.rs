//! The `agents` view the hub serves both halves: every agent terminal with its question, recap, whose move it is,
//! ticket, PR, CI, changes and time, built from what a poller keeps (the frontends' lists every two seconds, screens
//! and git on a change) and the store's documents. Its actions answer, recap, snooze, mark seen, fork and focus.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use cockpit_core::files;
use cockpit_core::frontend::{self, APP};
use cockpit_core::seen::now_ms;
use regex::Regex;
use serde_json::{Value, json};

use crate::ledger::{self, Ledger};
use crate::prompt::{self, Cursor, Question};
use crate::recap;
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
        loop {
            if poll() {
                self::touch();
            }
            let app = frontend::app_home().join("herdr-app.sock").exists();
            std::thread::sleep(Duration::from_secs(if app { 2 } else { 4 }));
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
    let mut changed = !snap().polled || before.len() != lives.len();
    let mut agents = Vec::new();
    let mut stale_git: Vec<String> = Vec::new();
    {
        let mut st = states();
        for live in &lives {
            let old = before.get(&live.handle);
            let moved = st.observe(&live.handle, &live.status, live.since, now);
            let mut a = old.cloned().unwrap_or_default();
            let status_changed = old.is_none_or(|o| o.live.status != live.status);
            // A waiting agent's screen is read on every look (its menu moves as keys go in); the others' on a change.
            if live.status == "blocked" {
                let screen = term::screen(&live.handle, 0).unwrap_or_default();
                a.question = Some(prompt::read(&screen));
                a.said = prompt::last_message(&screen);
            } else if status_changed {
                a.question = None;
                a.said = term::screen(&live.handle, 20).map(|s| prompt::last_message(&s)).unwrap_or_default();
            }
            if status_changed {
                stale_git.push(live.cwd.clone());
            }
            changed |= moved || status_changed || old.is_none_or(|o| o.question != a.question || o.said != a.said || o.live != *live);
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
        let mut c: Vec<String> = lives.iter().map(|l| l.cwd.clone()).filter(|c| !c.is_empty()).collect();
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
    let (config, inbox, facts) = (files::config(), files::load(&files::cache().join("inbox.json")).unwrap_or(Value::Null), facts_doc());
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
    changed
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
    facts["transcript"].as_str().map(PathBuf::from).filter(|p| p.exists()).or_else(|| recap::transcript(session, facts["cwd"].as_str().unwrap_or(&live.cwd)))
}

// ---------------------------------------------------------------------------------------------------------------------
// The view

pub const STAGES: [(&str, &str); 8] = [
    ("needs_you", "Needs you"),
    ("done", "Done"),
    ("fixing", "Needs fixing"),
    ("idle", "Idle"),
    ("working", "Working"),
    ("review", "In review"),
    ("merged", "Merged"),
    ("snoozed", "Snoozed"),
];

/// Whose move it is: its status while that says something, then its PR once it rests. Done is an attention queue: a
/// finished agent leaves it for what its PR says the moment the person opens it.
pub fn stage(status: &str, snoozed: bool, seen: bool, pr: &Value, ci: &Value, changes: &Value) -> &'static str {
    if snoozed {
        return "snoozed";
    }
    match status {
        "blocked" => return "needs_you",
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

/// Claude's rate limits with how far through its window each is (the pace tick) and whether use runs ahead of it.
/// An agent's time today and its ticket's today and this week, in seconds to the nearest minute: the panel shows
/// minutes, and the view moves no more often than they do.
fn time(agent_today: u64, (ticket_today, ticket_week): (u64, u64)) -> Value {
    let minute = |secs: u64| (secs + 30) / 60 * 60;
    json!({"agent_today": minute(agent_today), "ticket_today": minute(ticket_today), "ticket_week": minute(ticket_week)})
}

pub fn quota(limits: &Value, now: u64) -> Value {
    let fresh = limits["at"].as_u64().is_some_and(|at| now.saturating_sub(at) < 6 * 3_600_000);
    if !fresh {
        return json!([]);
    }
    let bars: Vec<Value> = limits["limits"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|l| {
            let name = l["name"].as_str().unwrap_or_default();
            let window: Option<u64> = match name {
                "5h" => Some(5 * 3_600_000),
                "7d" => Some(7 * 86_400_000),
                _ => None,
            };
            let resets = match &l["resetsAt"] {
                Value::Number(n) => n.as_u64().map(|s| if s < 10_000_000_000 { s * 1000 } else { s }),
                Value::String(s) => chrono::DateTime::parse_from_rfc3339(s).ok().map(|d| d.timestamp_millis() as u64),
                _ => None,
            };
            // To the percent the bar draws, so the view moves with it rather than every second.
            let elapsed = match (window, resets) {
                (Some(w), Some(r)) => Some(((1.0 - r.saturating_sub(now) as f64 / w as f64).clamp(0.0, 1.0) * 100.0).round() / 100.0),
                _ => None,
            };
            let percent = l["percent"].as_f64().unwrap_or(0.0);
            let ahead = elapsed.is_some_and(|e| percent / 100.0 > e + 0.05);
            json!({"name": name, "percent": percent.round(), "resets_at": resets, "elapsed": elapsed, "ahead": ahead})
        })
        .collect();
    json!(bars)
}

fn limits() -> Value {
    if let Some(world) = term::demo() {
        let mut l = world["limits"].clone();
        if l.is_object() {
            let now = now_ms();
            l["at"] = json!(now);
            for x in l["limits"].as_array_mut().into_iter().flatten() {
                let mins = x["resets_in_min"].as_f64().unwrap_or(60.0);
                x["resetsAt"] = json!(now + (mins * 60_000.0) as u64);
            }
        }
        return l;
    }
    files::load(&files::cache().join("limits.json")).unwrap_or(Value::Null)
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
        let unread = matches!(stage, "needs_you" | "done") && !seen;
        let session = session_of(l, f).or_else(|| mock["session"].as_str().map(str::to_string));
        let recap = match &session {
            Some(_) if !mock.is_null() => {
                if mock["recap"].is_string() {
                    json!({"text": mock["recap"], "at": now.saturating_sub(120_000), "fresh": true, "running": false, "error": null})
                } else {
                    Value::Null
                }
            }
            Some(id) => recap::shown(id, transcript_of(l, f, id).as_deref()),
            None => Value::Null,
        };
        let ticket_secs = key.as_ref().and_then(|k| totals.tickets.get(k)).copied().unwrap_or((0, 0));
        let title = f["task"]
            .as_str()
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .or_else(|| (!l.title.is_empty()).then(|| l.title.clone()))
            .unwrap_or_else(|| l.tab_name.clone());
        let question = a.question.as_ref().map(Question::json).unwrap_or(Value::Null);
        let transcript = session.as_ref().and_then(|id| transcript_of(l, f, id));
        let links = crate::links::of(ticket.as_ref(), &pr, &ci, &repo, branch.as_deref(), &inbox, &config, transcript.as_deref());
        rows.push(json!({
            "terminal": l.handle, "address": l.address, "frontend": l.frontend,
            "space": {"id": l.space_id, "name": l.space_name}, "tab": {"id": l.tab_id, "name": l.tab_name},
            "title": title, "program": l.program, "agent": l.agent, "status": l.status,
            "since": entry.since, "waiting_since": if l.status == "blocked" { json!(entry.since) } else { Value::Null },
            "question": if l.status == "blocked" { question } else { Value::Null }, "said": a.said, "recap": recap, "stage": stage,
            "repo": repo, "branch": branch, "cwd": l.cwd, "ticket": ticket, "pr": pr, "ci": ci, "changes": changes,
            "context": f["context"].as_f64().or(l.context), "cost": f["cost"].as_f64().or(mock["cost"].as_f64()),
            "page": f["page"].as_str().or(mock["page"].as_str()),
            "time": time(totals.agents.get(&l.handle).copied().unwrap_or(0), ticket_secs),
            "snoozed": snoozed, "seen": seen, "unread": unread, "session": session,
            "load": crate::load::of(l, session.as_deref()),
            "links": links, "documents": crate::links::documents(&[&l.handle, &l.address]),
        }));
    }
    let rank = |stage: &str| STAGES.iter().position(|(id, _)| *id == stage).unwrap_or(99);
    rows.sort_by(|x, y| {
        let (sx, sy) = (x["stage"].as_str().unwrap_or_default(), y["stage"].as_str().unwrap_or_default());
        let (tx, ty) = (x["since"].as_u64().unwrap_or(0), y["since"].as_u64().unwrap_or(0));
        // Waiting the longest first; elsewhere the latest change first.
        rank(sx).cmp(&rank(sy)).then(if sx == "needs_you" { tx.cmp(&ty) } else { ty.cmp(&tx) })
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
        "quota": quota(&limits(), now),
        "tickets": tickets,
        "week": {"days": totals.days, "rows": week_rows},
    })
}

// ---------------------------------------------------------------------------------------------------------------------
// Actions

fn find(handle: &str) -> Option<Agent> {
    snap().agents.iter().find(|a| a.live.handle == handle).cloned()
}

/// The screen read again: what the agent asks now.
fn reread(handle: &str) -> Question {
    let q = prompt::read(&term::screen(handle, 0).unwrap_or_default());
    if let Some(a) = snap().agents.iter_mut().find(|a| a.live.handle == handle) {
        a.question = Some(q.clone());
    }
    q
}

fn settle() {
    std::thread::sleep(Duration::from_millis(250));
}

/// Answers the question on the agent's screen; see docs/app.md for the shapes.
fn answer(handle: &str, arg: &Value) -> Result<String, String> {
    let a = find(handle).ok_or("no such agent")?;
    let tab = &a.live.tab_name;
    let name = if tab.is_empty() || tab.chars().all(|c| c.is_ascii_digit()) { a.live.space_name.clone() } else { tab.clone() };
    let mut q = reread(handle);
    let text = arg["text"].as_str().map(str::trim).filter(|t| !t.is_empty());
    let label = |q: &Question, n: u32| q.choices.iter().find(|c| c.n == n).map(|c| c.label.clone()).unwrap_or_else(|| n.to_string());
    let note = if q.choices.is_empty() {
        let text = text.ok_or("it asks for words: send text")?;
        term::paste(handle, text)?;
        format!("Answered {name}: {text}")
    } else if !q.multi() {
        let n = arg["choice"].as_u64().map(|n| n as u32).or(q.free.filter(|_| text.is_some())).ok_or("pick a choice")?;
        if !q.choices.iter().any(|c| c.n == n) {
            return Err(format!("no choice {n} any more"));
        }
        match text {
            // The free field: its number puts the cursor in it, the paste and its Enter answer.
            Some(text) if Some(n) == q.free => {
                if q.cursor != Some(Cursor::On(n)) {
                    term::keys(handle, &[n.to_string()])?;
                }
                term::paste(handle, text)?;
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
        let wanted: Vec<u32> = arg["choices"].as_array().map(|c| c.iter().filter_map(|n| n.as_u64()).map(|n| n as u32).collect()).unwrap_or_default();
        // In a multiple choice a number toggles its box; from the free field it would be typed, so ↑ leaves it first.
        if q.cursor.is_some() && q.cursor == q.free.map(Cursor::On) {
            term::keys(handle, &["up".into()])?;
            settle();
            q = reread(handle);
        }
        let toggles: Vec<String> =
            q.choices.iter().filter(|c| Some(c.n) != q.free && c.checked != Some(wanted.contains(&c.n))).map(|c| c.n.to_string()).collect();
        term::keys(handle, &toggles)?;
        if let (Some(text), Some(free)) = (text, q.free) {
            settle();
            q = reread(handle);
            term::keys(handle, &prompt::move_keys(&q, Some(free)))?;
            term::paste(handle, text)?;
            settle();
            q = reread(handle);
            let checked = q.choices.iter().find(|c| c.n == free).and_then(|c| c.checked) == Some(true);
            term::keys(handle, &if checked { vec!["up".into()] } else { vec!["up".into(), free.to_string()] })?;
        }
        settle();
        q = reread(handle);
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

fn recap_now(handle: &str, force: bool) -> Result<String, String> {
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

/// The view's actions, one object argument each: answer, recap, snooze, seen, fork, focus.
pub fn act(action: &str, args: &[Value]) -> Option<Result<String, String>> {
    let arg = args.first().cloned().unwrap_or(Value::Null);
    let handle = arg["terminal"].as_str().or(arg.as_str()).unwrap_or_default().to_string();
    let known = ["answer", "recap", "snooze", "seen", "fork", "focus", "refresh"];
    if !known.contains(&action) {
        return None;
    }
    if handle.is_empty() && action != "refresh" {
        return Some(Err("which agent? pass {terminal}".into()));
    }
    let done = match action {
        "answer" => answer(&handle, &arg),
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
        "fork" => fork(&handle, arg["place"].as_str().unwrap_or("tab")),
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
    fn whose_move_follows_the_status_then_the_pr() {
        let open = json!({"state": "open", "review": "REVIEW_REQUIRED"});
        assert_eq!(stage("blocked", false, false, &open, &Value::Null, &Value::Null), "needs_you");
        assert_eq!(stage("blocked", true, false, &open, &Value::Null, &Value::Null), "snoozed");
        assert_eq!(stage("idle", false, false, &open, &Value::Null, &Value::Null), "review");
        assert_eq!(stage("idle", false, false, &open, &json!({"state": "fail"}), &Value::Null), "fixing");
        assert_eq!(stage("idle", false, false, &json!({"state": "merged"}), &Value::Null, &json!({"files": 0, "ahead": 0})), "merged");
        assert_eq!(stage("idle", false, false, &json!({"state": "merged"}), &Value::Null, &json!({"files": 2})), "idle");
        assert_eq!(stage("done", false, false, &open, &Value::Null, &Value::Null), "done");
        assert_eq!(stage("done", false, true, &open, &Value::Null, &Value::Null), "review", "opened, it leaves the queue");
    }

    #[test]
    fn a_quota_bar_moves_with_whole_percents_of_its_window_not_each_second() {
        let now = 10 * 3_600_000;
        let limits = json!({"at": now, "limits": [{"name": "5h", "percent": 40, "resetsAt": (now + 3_600_000) / 1000}]});
        assert_eq!(quota(&limits, now), quota(&limits, now + 1_000), "a second is 0.006% of five hours");
        assert_eq!(quota(&limits, now + 180_000)[0]["elapsed"], json!(0.81));
    }

    #[test]
    fn agent_times_move_by_the_minute_they_are_shown_in() {
        assert_eq!(time(125, (61, 3_599)), json!({"agent_today": 120, "ticket_today": 60, "ticket_week": 3_600}));
        assert_eq!(time(125, (61, 3_599)), time(140, (75, 3_610)));
    }

    #[test]
    fn quota_bars_carry_their_pace() {
        let now = 10 * 3_600_000;
        let q = quota(&json!({"at": now, "limits": [{"name": "5h", "percent": 80, "resetsAt": (now + 3_600_000) / 1000}]}), now);
        assert!((q[0]["elapsed"].as_f64().unwrap() - 0.8).abs() < 1e-9);
        assert_eq!(q[0]["ahead"], false);
        let q = quota(&json!({"at": now, "limits": [{"name": "5h", "percent": 90, "resetsAt": (now + 4 * 3_600_000) / 1000}]}), now);
        assert_eq!(q[0]["ahead"], true);
        assert_eq!(quota(&json!({"at": 0, "limits": [{"name": "5h"}]}), now), json!([]));
    }

    #[test]
    fn a_ticket_comes_from_the_facts_else_the_branch() {
        let p = ticket_pattern(&json!({"tickets": {"keys": ["ENG"]}}), &Value::Null);
        assert_eq!(ticket_key(Some("me/eng-412-daily-quotas"), &Value::Null, p.as_ref()).as_deref(), Some("ENG-412"));
        assert_eq!(ticket_key(Some("main"), &json!({"ticket": "eng-9"}), p.as_ref()).as_deref(), Some("ENG-9"));
        assert_eq!(ticket_key(Some("main"), &Value::Null, p.as_ref()), None);
    }
}
