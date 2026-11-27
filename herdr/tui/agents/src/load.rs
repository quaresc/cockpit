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
pub(crate) fn root(l: &Live, session: Option<&str>) -> Option<u32> {
    if let Some(pid) = l.pid {
        return Some(pid);
    }
    if l.frontend == "app" {
        let got: Value = serde_json::from_str(&term::hc_text(&["get", &l.address, "--json"])?).ok()?;
        return got["pid"].as_u64().map(|p| p as u32);
    }
    let session = session.filter(|_| l.agent == "claude")?;
    let dirs = cockpit_core::transcripts::claude_homes().into_iter().map(|h| h.join("sessions"));
    dirs.flat_map(|d| std::fs::read_dir(d).into_iter().flatten().flatten()).find_map(|e| {
        let v = cockpit_core::files::load(&e.path())?;
        (v["sessionId"].as_str() == Some(session)).then(|| v["pid"].as_u64().map(|p| p as u32)).flatten()
    })
}

type Roots = HashMap<String, (Option<u32>, Instant)>;

/// Each app terminal's root process as hc last gave it, and when it asked.
static ROOTS: Mutex<Option<Roots>> = Mutex::new(None);

/// `root` for the load figures: an app terminal keeps its process, so hc is asked again only once that one is gone, or
/// EVERY after it gave none. The demo world's made-up terminals run none.
fn kept_root(l: &Live, session: Option<&str>) -> Option<u32> {
    if l.pid.is_some() || l.frontend != "app" {
        return root(l, session);
    }
    if term::demo_path().is_some() {
        return None;
    }
    kept(&l.address, || root(l, session))
}

/// The root process kept for the app terminal at `address`, `ask` for it when none is.
fn kept(address: &str, ask: impl FnOnce() -> Option<u32>) -> Option<u32> {
    let known = ROOTS.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|r| r.get(address).copied());
    match known {
        Some((Some(pid), _)) if alive(pid) => return Some(pid),
        Some((None, at)) if at.elapsed() < EVERY => return None,
        _ => {}
    }
    let got = ask();
    let mut held = ROOTS.lock().unwrap_or_else(|e| e.into_inner());
    let roots = held.get_or_insert_with(HashMap::new);
    if roots.len() > 500 {
        roots.clear();
    }
    roots.insert(address.to_string(), (got, Instant::now()));
    got
}

/// Whether the process `pid` still runs.
fn alive(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else { return false };
    // SAFETY: signal 0 only checks that the process exists.
    let there = unsafe { libc::kill(pid, 0) } == 0;
    there || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

pub(crate) fn ps() -> HashMap<u32, (u32, f64, u64)> {
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
    let Some(pid) = kept_root(l, session) else { return Value::Null };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_app_terminals_root_is_asked_again_only_once_its_process_is_gone() {
        let asked = std::cell::Cell::new(0);
        let ask = |pid: Option<u32>| {
            asked.set(asked.get() + 1);
            pid
        };
        let own = std::process::id();
        assert_eq!(kept("app:t-load-1", || ask(Some(own))), Some(own));
        assert_eq!(kept("app:t-load-1", || ask(Some(1))), Some(own), "kept while it runs");
        assert_eq!(asked.get(), 1);
        let mut gone = std::process::Command::new("/usr/bin/true").spawn().unwrap();
        let pid = gone.id();
        gone.wait().unwrap();
        assert_eq!(kept("app:t-load-2", || ask(Some(pid))), Some(pid));
        assert_eq!(kept("app:t-load-2", || ask(Some(own))), Some(own), "its process gone, hc is asked again");
        assert_eq!(asked.get(), 3);
        assert_eq!(kept("app:t-load-3", || ask(None)), None);
        assert_eq!(kept("app:t-load-3", || ask(Some(own))), None, "none given: not asked again before EVERY");
        assert_eq!(asked.get(), 4);
        ROOTS.lock().unwrap().as_mut().unwrap().get_mut("app:t-load-3").unwrap().1 -= EVERY;
        assert_eq!(kept("app:t-load-3", || ask(Some(own))), Some(own), "asked again after EVERY");
    }
}
