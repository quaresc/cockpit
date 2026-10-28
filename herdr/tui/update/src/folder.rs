//! The herdr folder as an update sees it: the plugin's version, the one in bin/, a dev build, and the lock that keeps
//! two updates (this one and fetch-apps.sh's) from writing bin/ at once.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// The plugin's version, from herdr-plugin.toml's `version = "…"` line.
pub fn wanted(dir: &Path) -> Option<String> {
    let manifest = std::fs::read_to_string(dir.join("herdr-plugin.toml")).ok()?;
    manifest.lines().find_map(|line| Some(line.strip_prefix("version = \"")?.strip_suffix('"')?.to_string())).filter(|v| !v.is_empty())
}

/// The version bin/ holds, from bin/.version.
pub fn installed(dir: &Path) -> Option<String> {
    std::fs::read_to_string(dir.join("bin/.version")).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

/// A checkout that builds its own binary runs that one and needs no release.
pub fn dev_build(dir: &Path) -> bool {
    dir.join("tui/target/release/cockpit").is_file()
}

pub fn has_binary(dir: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(dir.join("bin/cockpit")).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// `fetch.lock`, the folder fetch-apps.sh takes too; one left by a killed run expires after 10 minutes.
pub struct Lock(PathBuf);

impl Lock {
    pub fn take(dir: &Path) -> Option<Lock> {
        let path = dir.join("fetch.lock");
        let stale = std::fs::metadata(&path).and_then(|m| m.modified()).is_ok_and(|at| at.elapsed().unwrap_or_default() > Duration::from_secs(600));
        if stale {
            let _ = std::fs::remove_dir(&path);
        }
        std::fs::create_dir(&path).ok().map(|_| Lock(path))
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.0);
    }
}
