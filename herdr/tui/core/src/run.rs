//! Commands cockpit runs on its own (an agent asked headless, a model listing, a usage command): apart from the terminal
//! and agent session they were started from, their input empty, in a process group of their own that stops whole.

use std::io::Read;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

/// What ties a process to the agent session it was started from, when the hub itself was: a run never takes them.
pub const SESSION_VARS: [&str; 8] = [
    "CLAUDECODE",
    "CLAUDE_PID",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_MESSAGING_SOCKET",
    "CLAUDE_CODE_MESSAGING_TOKEN",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_SSE_PORT",
];

/// The key of the session cockpit drives that a process runs in, so the views it shows are that session's.
pub const DRIVEN_KEY: &str = "COCKPIT_SESSION_KEY";

/// Whether a variable ties a process to a Herdr terminal (`HERDR_*`, `HC_*`), an agent session or a driven one.
pub fn tied(name: &str) -> bool {
    name.starts_with("HERDR_") || name.starts_with("HC_") || SESSION_VARS.contains(&name) || name == DRIVEN_KEY
}

/// Takes the variables that tie a process to a terminal or an agent session out of `cmd`'s environment.
pub fn untie(cmd: &mut Command) -> &mut Command {
    let set: Vec<_> = cmd.get_envs().map(|(name, _)| name.to_owned()).collect();
    for name in std::env::vars_os().map(|(name, _)| name).chain(set) {
        if name.to_str().is_some_and(tied) {
            cmd.env_remove(name);
        }
    }
    cmd
}

/// A run's process group, which `stop` ends whole until its leader is reaped: its id is never signalled once the system
/// may give it to another process.
#[derive(Clone, Debug, Default)]
pub struct Group(Arc<Mutex<Held>>);

#[derive(Debug, Default)]
struct Held {
    pid: Option<u32>,
    stopped: bool,
}

impl Group {
    fn held(&self) -> MutexGuard<'_, Held> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Ends the group now, or as soon as its run starts.
    pub fn stop(&self) {
        let mut held = self.held();
        held.stopped = true;
        if let Some(pid) = held.pid {
            signal(pid);
        }
    }

    /// Whether its leader runs, or ended and was not reaped yet.
    pub fn live(&self) -> bool {
        self.held().pid.is_some()
    }
}

impl PartialEq for Group {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

fn signal(pid: u32) {
    if let Ok(pid) = i32::try_from(pid)
        && pid > 0
    {
        // SAFETY: the group's leader is not reaped yet, so its id is still this run's group.
        unsafe { libc::kill(-pid, libc::SIGKILL) };
    }
}

/// Ends the process group led by `pid` at once, before its leader is reaped.
pub fn kill_group(pid: u32) {
    signal(pid);
}

/// Runs `cmd` apart from any terminal or agent session (`untie`), its input empty, in a process group `group` can stop,
/// and gives its output once it ends, each stream cut at `cap` bytes. Past `limit` the whole group is stopped.
pub fn output(cmd: &mut Command, limit: Duration, cap: usize, group: &Group) -> Result<Output, String> {
    untie(cmd).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).process_group(0);
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    {
        let mut held = group.held();
        held.pid = Some(child.id());
        if held.stopped {
            signal(child.id());
        }
    }
    let (stdout, stderr) = (child.stdout.take(), child.stderr.take());
    let (tx, rx) = channel();
    let ours = group.clone();
    std::thread::spawn(move || {
        let err = std::thread::spawn(move || drain(stderr, cap));
        let out = drain(stdout, cap);
        let err = err.join().unwrap_or_default();
        let _ = tx.send(reap(&mut child, &ours).map(|status| Output { status, stdout: out, stderr: err }));
    });
    match rx.recv_timeout(limit) {
        Ok(done) => done.map_err(|e| e.to_string()),
        Err(_) => {
            group.stop();
            // Every process of the group is gone, those holding its output too, before the reader is waited for.
            let _ = rx.recv_timeout(Duration::from_secs(5));
            Err("it took too long".into())
        }
    }
}

/// Reads a stream to its end, keeping its first `cap` bytes, so a run never blocks on a full pipe.
fn drain(stream: Option<impl Read>, cap: usize) -> Vec<u8> {
    let mut kept = Vec::new();
    let Some(mut stream) = stream else { return kept };
    let mut buf = [0u8; 8192];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => kept.extend_from_slice(&buf[..n.min(cap.saturating_sub(kept.len()))]),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    kept
}

/// Waits for the leader to end without reaping it, its id staying its own meanwhile; then forgets that id, so no `stop`
/// can reach it, and reaps it.
fn reap(child: &mut Child, group: &Group) -> std::io::Result<ExitStatus> {
    let pid = child.id();
    loop {
        // SAFETY: waitid only writes the siginfo it is handed; WNOWAIT leaves the child to be reaped below.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let r = unsafe { libc::waitid(libc::P_PID, pid as libc::id_t, &mut info, libc::WEXITED | libc::WNOWAIT) };
        if r == 0 || std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
            break;
        }
    }
    let mut held = group.held();
    held.pid = None;
    child.wait()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_s_output_comes_back_capped_and_its_group_is_forgotten_once_reaped() {
        let group = Group::default();
        let out = output(Command::new("sh").args(["-c", "printf 0123456789; printf oops >&2"]), Duration::from_secs(10), 4, &group).unwrap();
        assert!(out.status.success());
        assert_eq!((out.stdout.as_slice(), out.stderr.as_slice()), (&b"0123"[..], &b"oops"[..]));
        assert!(!group.live(), "reaped: its id is no longer this run's to signal");
        group.stop();
        let big = output(Command::new("sh").args(["-c", "head -c 300000 /dev/zero"]), Duration::from_secs(10), 1024, &Group::default()).unwrap();
        assert_eq!(big.stdout.len(), 1024, "the rest is read and dropped, so the run never blocks on its pipe");
    }

    #[test]
    fn a_run_leaves_the_terminal_and_session_it_was_started_from() {
        assert!(tied("HC_TERMINAL_ID") && tied("HERDR_ENV") && tied("CLAUDE_CODE_SESSION_ID") && tied("CLAUDECODE") && tied(DRIVEN_KEY));
        assert!(!tied("HOME") && !tied("PATH") && !tied("CLAUDE_CONFIG_DIR"));
        let mut cmd = Command::new("env");
        cmd.env("HC_SCOPE", "w1").env("CLAUDE_CODE_MESSAGING_TOKEN", "x").env("ACME_KEPT", "1");
        let out = output(&mut cmd, Duration::from_secs(10), 1 << 20, &Group::default()).unwrap();
        let env = String::from_utf8_lossy(&out.stdout);
        assert!(env.contains("ACME_KEPT=1") && !env.contains("HC_SCOPE") && !env.contains("CLAUDE_CODE_MESSAGING_TOKEN"), "{env}");
    }

    #[test]
    fn a_run_past_its_time_or_stopped_ends_with_its_descendants() {
        let dir = tempfile::tempdir().unwrap();
        let escaped = dir.path().join("escaped");
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "(sleep 1; touch \"$1\") & wait", "sh"]).arg(&escaped);
        assert_eq!(output(&mut cmd, Duration::from_millis(50), 1024, &Group::default()).unwrap_err(), "it took too long");
        let ready = dir.path().join("ready");
        let group = Group::default();
        let stopper = group.clone();
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "(sleep 1; touch \"$1\") & touch \"$2\"; wait", "sh"]).arg(&escaped).arg(&ready);
        let running = std::thread::spawn(move || output(&mut cmd, Duration::from_secs(30), 1024, &group));
        let until = std::time::Instant::now() + Duration::from_secs(5);
        while !ready.exists() && std::time::Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
        }
        stopper.stop();
        assert!(!running.join().unwrap().unwrap().status.success(), "stopped");
        std::thread::sleep(Duration::from_millis(1100));
        assert!(!escaped.exists(), "no descendant outlives its run");
        let stopped = Group::default();
        stopped.stop();
        let out = output(Command::new("sh").args(["-c", "sleep 5"]), Duration::from_secs(10), 1024, &stopped).unwrap();
        assert!(!out.status.success(), "a group stopped before its run starts ends at once");
    }
}
