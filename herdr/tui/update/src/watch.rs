//! The hub keeping the spaces on the plugin's version: every 10 minutes it installs the version's binary once the release
//! has it, then hands over to it.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::{dev_build, installed, releases, update, wanted};

/// Seconds between checks; COCKPIT_UPDATE_SECS shortens it for tests.
fn every() -> Duration {
    Duration::from_secs(std::env::var("COCKPIT_UPDATE_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(600))
}

/// Checks `dir` forever; `hand_over` gets bin/cockpit once it holds the plugin's version and this process runs another.
pub fn keep_current(dir: &Path, hand_over: impl Fn(&Path)) {
    loop {
        std::thread::sleep(every());
        if dev_build(dir) {
            continue;
        }
        let _ = update(dir, &releases());
        if let Some(binary) = newer_binary(dir) {
            hand_over(&binary);
        }
    }
}

/// bin/cockpit when it is on the plugin's version, says so itself, and this process is not.
pub(crate) fn newer_binary(dir: &Path) -> Option<std::path::PathBuf> {
    let version = wanted(dir)?;
    if installed(dir)? != version || version == env!("CARGO_PKG_VERSION") {
        return None;
    }
    let binary = dir.join("bin/cockpit");
    let said = Command::new(&binary).arg("--version").stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    (String::from_utf8_lossy(&said.stdout).trim() == format!("cockpit {version}")).then_some(binary)
}

/// Stops the space apps run from `dir`'s bin/cockpit (`cockpit space` starts them by its full path); their supervisors
/// start them again on the binary now there.
pub fn restart_apps(dir: &Path) {
    let Ok(ours) = dir.join("bin/cockpit").canonicalize() else { return };
    let Ok(listed) = Command::new("ps").args(["-axo", "pid=,command="]).stderr(Stdio::null()).output() else { return };
    for pid in space_apps(&String::from_utf8_lossy(&listed.stdout), &ours) {
        // SAFETY: kill only sends SIGTERM to a process ps just listed.
        unsafe { libc::kill(pid, libc::SIGTERM) };
    }
}

/// The pids of the `<binary> <app> cockpit-space` lines of `ps -o pid=,command=` whose binary is `ours`.
pub(crate) fn space_apps(listed: &str, ours: &Path) -> Vec<i32> {
    listed
        .lines()
        .filter_map(|line| {
            let (pid, command) = line.trim_start().split_once(' ')?;
            let binary = ["inbox", "todo", "runbook", "meetings", "monitor", "routines"].iter().find_map(|app| command.strip_suffix(&format!(" {app} cockpit-space")))?;
            (Path::new(binary).canonicalize().ok()? == ours).then(|| pid.parse().ok())?
        })
        .collect()
}
