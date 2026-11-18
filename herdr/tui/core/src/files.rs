//! Where cockpit keeps its files, JSON read and written whole, the config, and a watcher that says when files change.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde_json::Value;

pub fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
}

pub fn cache() -> PathBuf {
    home().join(".cache/cockpit")
}

pub fn config_dir() -> PathBuf {
    home().join(".config/cockpit")
}

/// The herdr plugin folder, where its scripts live: app.sh says so, else the binary's own place.
pub fn herdr_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("COCKPIT_HERDR_DIR") {
        return PathBuf::from(dir);
    }
    // A release binary sits in herdr/bin/, a dev build in herdr/tui/target/release/; a link to it (~/.local/bin) is followed.
    let exe = std::env::current_exe().map(|e| std::fs::canonicalize(&e).unwrap_or(e)).unwrap_or_default();
    let released = exe.parent().and_then(Path::file_name).is_some_and(|name| name == "bin");
    exe.ancestors().nth(if released { 2 } else { 4 }).map(Path::to_path_buf).unwrap_or_default()
}

/// An id from outside (a session, a terminal) as a file name: letters, digits, `-` and `_` kept, anything else `-`, so no
/// `/` or `..` reaches a path.
pub fn file_key(id: &str) -> String {
    let key: String = id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect();
    if key.is_empty() { "_".into() } else { key }
}

pub fn load(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// Writes through a temporary file, so a reader never sees half of it.
pub fn write(path: &Path, value: &Value) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // A temporary file of this write's own: two writers of one file never mix their bytes, the last rename wins.
    static WRITES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = WRITES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = path.with_extension(format!("{}-{n}.tmp", std::process::id()));
    std::fs::write(&tmp, serde_json::to_string(value)?)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// Runs `f` while this process holds `<path>.lock`, so one writer at a time, across processes, reads and writes
/// `path`. A lock held past 10 s is taken as left behind by a writer that died; a lock that cannot be made at all (an
/// unwritable folder), or not within 20 s, leaves `f` to run unlocked, so its own write fails and says why.
pub fn locked<T>(path: &Path, f: impl FnOnce() -> T) -> T {
    locked_within(path, std::time::Duration::from_secs(10), std::time::Duration::from_secs(20), f)
}

fn locked_within<T>(path: &Path, stale: std::time::Duration, give_up: std::time::Duration, f: impl FnOnce() -> T) -> T {
    let lock = path.with_extension("lock");
    if let Some(dir) = lock.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let start = std::time::Instant::now();
    let pause = || std::thread::sleep(std::time::Duration::from_millis(20));
    let held = loop {
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&lock) {
            Ok(_) => break true,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if start.elapsed() >= give_up {
                    break false;
                }
                let old = std::fs::metadata(&lock).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age >= stale);
                // A lock left behind is taken over; another writer that got there first is waited for.
                if (old || start.elapsed() >= stale) && std::fs::remove_file(&lock).is_ok() {
                    pause();
                    continue;
                }
                pause();
            }
            Err(_) => break false,
        }
    };
    let out = f();
    if held {
        let _ = std::fs::remove_file(&lock);
    }
    out
}

/// The config as the mod resolved it, or the raw file before any session has run.
pub fn config() -> Value {
    load(&cache().join("config.json")).or_else(|| load(&config_dir().join("config.json"))).unwrap_or(Value::Null)
}

/// Fires once per change to anything in these folders; the watcher stops when dropped.
pub fn watch(dirs: &[PathBuf]) -> (Option<RecommendedWatcher>, Receiver<()>) {
    let (tx, rx) = channel();
    let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if event.is_ok() {
            let _ = tx.send(());
            crate::term::wake();
        }
    });
    let Ok(mut watcher) = watcher else { return (None, rx) };
    for dir in dirs {
        let _ = std::fs::create_dir_all(dir);
        let _ = watcher.watch(dir, RecursiveMode::NonRecursive);
    }
    (Some(watcher), rx)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Density {
    /// Rows with room, their actions in the panel under the list.
    Comfortable,
    /// Denser rows, their actions on the selected row itself.
    Compact,
}

/// The config's `density`: comfortable unless it says compact.
pub fn density() -> Density {
    if config()["density"].as_str() == Some("compact") { Density::Compact } else { Density::Comfortable }
}

pub fn open_url(url: &str) {
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    let _ = std::process::Command::new(opener).arg(url).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
}

/// Puts text on the clipboard: pbcopy on macOS, wl-copy or xclip on Linux; false when none answers.
pub fn copy(text: &str) -> bool {
    use std::io::Write;
    let tools: &[&[&str]] = &[&["pbcopy"], &["wl-copy"], &["xclip", "-selection", "clipboard"]];
    for tool in tools {
        let Ok(mut child) = std::process::Command::new(tool[0]).args(&tool[1..]).stdin(std::process::Stdio::piped()).spawn() else { continue };
        let _ = child.stdin.take().map(|mut stdin| stdin.write_all(text.as_bytes()));
        return child.wait().map(|s| s.success()).unwrap_or(false);
    }
    false
}

/// The herdr CLI's JSON answer, or None when it fails.
pub fn herdr(args: &[&str]) -> Option<Value> {
    let bin = std::env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".into());
    let out = std::process::Command::new(bin).args(args).output().ok()?;
    serde_json::from_slice(&out.stdout).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn a_lock_that_cannot_be_made_runs_the_write_unlocked_at_once() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("ro");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
        let start = Instant::now();
        let wrote = locked(&dir.join("config.json"), || std::fs::write(dir.join("config.json"), "{}"));
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(wrote.unwrap_err().kind(), std::io::ErrorKind::PermissionDenied, "the write itself says why");
        assert!(start.elapsed() < Duration::from_secs(2), "no wait on a lock nobody can make");
    }

    #[test]
    fn a_lock_held_is_waited_for_and_one_left_behind_taken_over() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.json");
        let lock = path.with_extension("lock");
        std::fs::write(&lock, "").unwrap();
        // Held past the stale age: taken over, and released after.
        let start = Instant::now();
        assert_eq!(locked_within(&path, Duration::from_millis(200), Duration::from_secs(5), || 7), 7);
        assert!(start.elapsed() >= Duration::from_millis(200) && start.elapsed() < Duration::from_secs(3));
        assert!(!lock.exists());
        // One that is never released and cannot be taken over is given up on: the work runs all the same.
        std::fs::create_dir(&lock).unwrap();
        let start = Instant::now();
        assert_eq!(locked_within(&path, Duration::from_secs(60), Duration::from_millis(300), || 8), 8);
        assert!(start.elapsed() < Duration::from_secs(3));
        assert!(lock.is_dir(), "a lock it did not make is not removed");
    }
}
