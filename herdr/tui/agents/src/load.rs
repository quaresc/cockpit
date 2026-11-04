//! What each agent's processes cost the machine: CPU and memory of the process tree under it, with a short history for
//! a sparkline. One `ps` per sample, at most every few seconds whatever asks.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::term::{self, Live};

const EVERY: Duration = Duration::from_secs(5);
const KEEP: usize = 24;

struct State {
    at: Option<Instant>,
    /// pid → (ppid, %cpu, rss KB), as `ps` gave them last.
    procs: HashMap<u32, (u32, f64, u64)>,
    history: HashMap<String, VecDeque<f64>>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

/// The process an agent's tree starts at: its terminal's, as the frontend gives it (asked of hc for an app terminal not
/// looked at yet), else for Claude Code the pid its own registry notes for the session.
fn root(l: &Live, session: Option<&str>) -> Option<u32> {
    if let Some(pid) = l.pid {
        return Some(pid);
    }
    if l.frontend == "app" {
        let got: Value = serde_json::from_str(&term::hc_text(&["get", &l.address, "--json"])?).ok()?;
        return got["pid"].as_u64().map(|p| p as u32);
    }
    let session = session.filter(|_| l.agent == "claude")?;
    let dir = cockpit_core::transcripts::claude_home().join("sessions");
    std::fs::read_dir(dir).into_iter().flatten().flatten().find_map(|e| {
        let v = cockpit_core::files::load(&e.path())?;
        (v["sessionId"].as_str() == Some(session)).then(|| v["pid"].as_u64().map(|p| p as u32)).flatten()
    })
}

fn ps() -> HashMap<u32, (u32, f64, u64)> {
    let text = term::output(&["ps".into(), "-A".into(), "-o".into(), "pid=,ppid=,%cpu=,rss=".into()], 5).unwrap_or_default();
    text.lines()
        .filter_map(|l| {
            let mut w = l.split_whitespace();
            Some((w.next()?.parse().ok()?, (w.next()?.parse().ok()?, w.next()?.replace(',', ".").parse().ok()?, w.next()?.parse().ok()?)))
        })
        .collect()
}

/// `{cpu, mem, history}` for an agent (CPU in percent of one core, memory in MB), or null when its process is unknown.
pub fn of(l: &Live, session: Option<&str>) -> Value {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let state = guard.get_or_insert_with(|| State { at: None, procs: HashMap::new(), history: HashMap::new() });
    let fresh = state.at.is_none_or(|t| t.elapsed() >= EVERY);
    if fresh {
        state.procs = ps();
        state.at = Some(Instant::now());
    }
    let Some(pid) = root(l, session) else { return Value::Null };
    if !state.procs.contains_key(&pid) {
        return Value::Null;
    }
    let mut kids: HashMap<u32, Vec<u32>> = HashMap::new();
    for (p, (pp, _, _)) in &state.procs {
        kids.entry(*pp).or_default().push(*p);
    }
    let (mut cpu, mut rss, mut todo) = (0.0, 0u64, vec![pid]);
    while let Some(p) = todo.pop() {
        if let Some((_, c, r)) = state.procs.get(&p) {
            cpu += c;
            rss += r;
        }
        todo.extend(kids.get(&p).into_iter().flatten().copied());
    }
    let cpu = cpu.round();
    let history = state.history.entry(l.handle.clone()).or_default();
    if fresh || history.is_empty() {
        history.push_back(cpu);
        while history.len() > KEEP {
            history.pop_front();
        }
    }
    // Rounded so the view only changes when the numbers do.
    json!({"cpu": cpu, "mem": (rss as f64 / 1024.0 / 10.0).round() * 10.0, "history": history.iter().collect::<Vec<_>>()})
}
