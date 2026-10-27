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

/// The herdr plugin folder, where the text apps and bridge.py live: app.sh says so, else the binary's own place.
pub fn herdr_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("COCKPIT_HERDR_DIR") {
        return PathBuf::from(dir);
    }
    let exe = std::env::current_exe().unwrap_or_default();
    exe.ancestors().nth(4).map(Path::to_path_buf).unwrap_or_default()
}

pub fn load(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// Writes through a temporary file, so a reader never sees half of it.
pub fn write(path: &Path, value: &Value) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_string(value)?)?;
    std::fs::rename(tmp, path)
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
        }
    });
    let Ok(mut watcher) = watcher else { return (None, rx) };
    for dir in dirs {
        let _ = std::fs::create_dir_all(dir);
        let _ = watcher.watch(dir, RecursiveMode::NonRecursive);
    }
    (Some(watcher), rx)
}

pub fn open_url(url: &str) {
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    let _ = std::process::Command::new(opener).arg(url).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
}

/// Puts text on the clipboard; false when no clipboard tool answers.
pub fn copy(text: &str) -> bool {
    use std::io::Write;
    let tool = if cfg!(target_os = "macos") { vec!["pbcopy"] } else { vec!["wl-copy"] };
    let Ok(mut child) = std::process::Command::new(tool[0]).stdin(std::process::Stdio::piped()).spawn() else { return false };
    let _ = child.stdin.take().map(|mut stdin| stdin.write_all(text.as_bytes()));
    child.wait().map(|s| s.success()).unwrap_or(false)
}

/// The herdr CLI's JSON answer, or None when it fails.
pub fn herdr(args: &[&str]) -> Option<Value> {
    let bin = std::env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".into());
    let out = std::process::Command::new(bin).args(args).output().ok()?;
    serde_json::from_slice(&out.stdout).ok()
}
