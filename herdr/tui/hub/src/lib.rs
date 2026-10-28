//! `cockpit hub`: answers the apps on ~/.cache/cockpit/hub.sock (see cockpit_core::hub for the protocol) and is the
//! only writer of the documents they share (see store). One runs at a time, detached from the app that started it, and
//! exits after 30 minutes with no request and no subscriber.

mod act;
mod config;
mod feed;
mod fetch;
mod go;
mod herdr;
mod history;
mod proc;
mod search;
mod server;
mod store;
mod value;

use std::os::unix::fs::MetadataExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use cockpit_core::files;

/// Seconds with no request before the hub exits; COCKPIT_HUB_IDLE_SECS shortens it for tests.
fn idle_limit() -> u64 {
    std::env::var("COCKPIT_HUB_IDLE_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(30 * 60)
}

/// Serves until idle; returns at once when another hub already answers on the socket. Run with HUB_TASK set, it indexes
/// the past Claude sessions once instead (see search::sessions).
pub fn run() {
    if std::env::var_os(search::sessions::HUB_TASK).is_some() {
        search::sessions::index_once();
        return;
    }
    // Its own session: closing the pane of the app that started it does not take the hub down.
    // SAFETY: setsid has no memory effects; it fails harmlessly when the hub already leads a group.
    unsafe { libc::setsid() };
    let socket = cockpit_core::hub::socket();
    if UnixStream::connect(&socket).is_ok() {
        return;
    }
    // Nothing answers on it: a hub that died left it behind.
    let _ = std::fs::remove_file(&socket);
    if let Some(dir) = socket.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let Ok(listener) = UnixListener::bind(&socket) else {
        return;
    };
    let inode = std::fs::metadata(&socket).map(|m| m.ino()).unwrap_or_default();
    // The store opens before the first request, waiting out a hub that hands over to this one.
    drop(store::global());
    let last = Arc::new(AtomicU64::new(server::now_secs()));
    std::thread::spawn({
        let (last, socket) = (last.clone(), socket.clone());
        move || exit_when_idle(&socket, inode, &last)
    });
    std::thread::spawn(watch);
    std::thread::spawn(search::sessions::keep);
    std::thread::spawn(move || {
        let dir = files::herdr_dir();
        cockpit_update::keep_current(&dir, |binary| hand_over(&dir, binary, &socket, inode));
    });
    for stream in listener.incoming().flatten() {
        last.store(server::now_secs(), Ordering::SeqCst);
        let last = last.clone();
        std::thread::spawn(move || server::serve(stream, last));
    }
}

/// A newer binary went into bin/: a hub run from it takes the socket over, the space apps restart on it, this one ends.
fn hand_over(dir: &Path, binary: &Path, socket: &Path, inode: u64) {
    if std::fs::metadata(socket).is_ok_and(|m| m.ino() == inode) {
        let _ = std::fs::remove_file(socket);
    }
    store::global().close();
    let _ = std::process::Command::new(binary).arg("hub").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
    cockpit_update::restart_apps(dir);
    std::process::exit(0);
}

/// Whether a pinned space's app runs: the hub keeps fetching for it and keeping its binary current, requests or not.
fn space_apps_running() -> bool {
    std::process::Command::new("pgrep")
        .args(["-f", "cockpit (inbox|runbook|todo|meetings|monitor) cockpit-space"])
        .stdout(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn exit_when_idle(socket: &Path, inode: u64, last: &AtomicU64) {
    let limit = idle_limit();
    loop {
        std::thread::sleep(Duration::from_secs((limit / 10).clamp(1, 30)));
        let is_idle = server::now_secs().saturating_sub(last.load(Ordering::SeqCst)) >= limit;
        if is_idle && !store::global().has_subscribers() && !space_apps_running() {
            // Only our own socket goes: a newer hub may have taken the path.
            if std::fs::metadata(socket).is_ok_and(|m| m.ino() == inode) {
                let _ = std::fs::remove_file(socket);
            }
            std::process::exit(0);
        }
    }
}

/// Follows the files the search index and the history read: a change rebuilds the index and records history.
fn watch() {
    let cache = files::cache();
    let dirs: Vec<PathBuf> = vec![cache.clone(), cache.join("costs"), cache.join("show"), act::runbooks_dir(), files::config_dir()];
    let (_watcher, changes) = files::watch(&dirs);
    history::record(&cache);
    while changes.recv().is_ok() {
        // A burst of events (a file written through its .tmp) settles before anything is read again.
        while changes.recv_timeout(Duration::from_millis(300)).is_ok() {}
        search::invalidate();
        history::record(&cache);
    }
}
