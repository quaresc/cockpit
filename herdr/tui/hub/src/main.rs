//! cockpit-hub: answers the apps on ~/.cache/cockpit/hub.sock (see cockpit_core::hub for the protocol). One runs at a
//! time, detached from the app that started it, and exits after 30 minutes with no request.

mod act;
mod config;
mod fetch;
mod go;
mod herdr;
mod history;
mod proc;
mod py;
mod search;
mod server;

use std::os::unix::fs::MetadataExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use cockpit_core::files;

/// Seconds with no request before the hub exits; COCKPIT_HUB_IDLE_SECS shortens it for tests.
fn idle_limit() -> u64 {
    std::env::var("COCKPIT_HUB_IDLE_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(30 * 60)
}

fn main() {
    cockpit_core::answer_version("cockpit-hub");
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
    let last = Arc::new(AtomicU64::new(server::now_secs()));
    std::thread::spawn({
        let last = last.clone();
        move || exit_when_idle(&socket, inode, &last)
    });
    std::thread::spawn(watch);
    for stream in listener.incoming().flatten() {
        last.store(server::now_secs(), Ordering::SeqCst);
        let last = last.clone();
        std::thread::spawn(move || server::serve(stream, last));
    }
}

fn exit_when_idle(socket: &Path, inode: u64, last: &AtomicU64) {
    let limit = idle_limit();
    loop {
        std::thread::sleep(Duration::from_secs((limit / 10).clamp(1, 30)));
        if server::now_secs().saturating_sub(last.load(Ordering::SeqCst)) >= limit {
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
