//! `cockpit space <app>`: keeps a space's app running in its pane, as a child started again whenever it ends; ctrl+c
//! twice in a row leaves it. An app that keeps dying soon after its start waits longer each time before the next one.

use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use cockpit_core::files;

const USAGE: &str = "Keeps a space's app running in its pane; ctrl+c twice in a row leaves it.

    cockpit space inbox|todo|runbook|meetings|monitor|routines";

const APPS: [&str; 6] = ["inbox", "todo", "runbook", "meetings", "monitor", "routines"];
/// A run at least this long resets the wait before the next one.
const LONG_RUN: Duration = Duration::from_secs(60);
const FIRST_WAIT: Duration = Duration::from_secs(1);
const LONGEST_WAIT: Duration = Duration::from_secs(30);

pub fn main(args: &[String]) {
    let [app] = args else { crate::proc::fail(USAGE) };
    if !APPS.contains(&app.as_str()) {
        crate::proc::fail(USAGE);
    }
    // A ctrl+c while the app runs is the app's; one during the wait after it ends leaves.
    let interrupted = Arc::new(AtomicBool::new(false));
    let _ = signal_hook::flag::register(signal_hook::consts::SIGINT, interrupted.clone());
    // The binary of the first round, which app.sh started this supervisor from.
    let mut own: Option<Identity> = None;
    let mut wait: Option<Duration> = None;
    loop {
        let dir = files::herdr_dir();
        let binary = binary(&dir).or_else(|| fetch(&dir));
        let current = binary.as_deref().and_then(Identity::of);
        // An update replaced the binary: the next rounds run from the new one.
        match (&binary, &own) {
            (Some(binary), Some(_)) if current.is_some() && current != own => {
                let error = Command::new(binary).args(["space", app]).exec();
                eprintln!("cockpit: {} space {app}: {error}", binary.display());
            }
            (_, None) => own = current,
            _ => {}
        }
        // Mouse reporting off first, in case the previous run died without switching it off.
        print!("\x1b[?1003l\x1b[?1006l\x1b[?1000l");
        let _ = std::io::stdout().flush();
        let started = Instant::now();
        match &binary {
            Some(binary) => run(binary, &dir, app),
            None => eprintln!("cockpit: no cockpit binary for this machine yet; /cockpit:doctor says why"),
        }
        let pause = next_wait(wait, started.elapsed());
        wait = Some(pause);
        interrupted.store(false, Ordering::SeqCst);
        if pause_or_leave(pause, &interrupted) {
            return;
        }
    }
}

/// The app as a child, by its full path, which the hub and update.sh look for to restart or stop it; the extra argument
/// marks it as a space's.
fn run(binary: &Path, dir: &Path, app: &str) {
    let mut command = Command::new(binary);
    command.args([app, "cockpit-space"]).env("COCKPIT_HERDR_DIR", dir).current_dir(dir);
    // Run from here, the app is in its own space and reports its count on that space's sidebar row.
    if app != "inbox" {
        command.env("COCKPIT_SPACE", "1");
    }
    if let Err(error) = command.status() {
        eprintln!("cockpit: {}: {error}", binary.display());
    }
}

/// The release's binary in bin/, else a dev build in tui/target/release/.
fn binary(dir: &Path) -> Option<PathBuf> {
    ["bin/cockpit", "tui/target/release/cockpit"].iter().map(|rel| dir.join(rel)).find(|p| crate::proc::executable(p))
}

/// The release's binary, fetched when neither is there.
fn fetch(dir: &Path) -> Option<PathBuf> {
    let _ = Command::new("sh").arg("fetch-apps.sh").current_dir(dir).env("PATH", crate::proc::path()).status();
    Some(dir.join("bin/cockpit")).filter(|p| crate::proc::executable(p))
}

/// The wait after a run: a second at first and after a long run, doubled after each short one, up to 30 seconds.
fn next_wait(previous: Option<Duration>, ran: Duration) -> Duration {
    match previous {
        Some(previous) if ran < LONG_RUN => (previous * 2).min(LONGEST_WAIT),
        _ => FIRST_WAIT,
    }
}

/// Waits, true as soon as a ctrl+c says to leave.
fn pause_or_leave(pause: Duration, interrupted: &AtomicBool) -> bool {
    let end = Instant::now() + pause;
    while Instant::now() < end {
        if interrupted.load(Ordering::SeqCst) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50).min(end.saturating_duration_since(Instant::now())));
    }
    interrupted.load(Ordering::SeqCst)
}

/// Which file a binary is: its resolved path, inode and modification time, so a binary replaced in place differs.
#[derive(PartialEq, Eq)]
struct Identity(PathBuf, u64, u64, i64, i64);

impl Identity {
    fn of(path: &Path) -> Option<Self> {
        let real = path.canonicalize().ok()?;
        let meta = std::fs::metadata(&real).ok()?;
        Some(Identity(real, meta.dev(), meta.ino(), meta.mtime(), meta.mtime_nsec()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wait_doubles_after_short_runs_and_resets_after_a_long_one() {
        let short = Duration::from_secs(2);
        let mut wait = None;
        let mut waits = Vec::new();
        for _ in 0..7 {
            let next = next_wait(wait, short);
            waits.push(next.as_secs());
            wait = Some(next);
        }
        assert_eq!(waits, [1, 2, 4, 8, 16, 30, 30]);
        assert_eq!(next_wait(Some(LONGEST_WAIT), Duration::from_secs(61)), FIRST_WAIT);
    }
}
