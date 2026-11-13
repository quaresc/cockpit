//! `cockpit hub`: answers the apps on ~/.cache/cockpit/hub.sock (see cockpit_core::hub for the protocol), is the only
//! writer of the documents they share (see store) and runs the routines. One runs at a time, detached from the app that
//! started it, and exits after 30 minutes with no request, no subscriber and no routine to run on its own.

mod act;
mod config;
mod events;
mod feed;
mod fetch;
mod fetcher;
mod go;
mod herdr;
mod history;
mod names;
pub mod outbox;
mod proc;
mod routines;
mod search;
mod server;
pub mod store;
mod value;
pub mod views;

use std::io::Write;
use std::net::TcpListener;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
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
/// the past agent sessions once instead (see search::sessions).
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
    // Before anything is touched: a hub without its TCP side is one `cockpit hub start` stops.
    let tcp = match listen() {
        Ok(tcp) => tcp,
        Err(why) => {
            eprintln!("cockpit hub: cannot listen on COCKPIT_HUB_LISTEN: {why}");
            return;
        }
    };
    // Nothing answers on it: a hub that died left it behind.
    let _ = std::fs::remove_file(&socket);
    if let Some(dir) = socket.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let listener = match UnixListener::bind(&socket) {
        Ok(listener) => listener,
        // Said on stderr, which the app that started the hub points at ~/.cache/cockpit/hub.log.
        Err(why) => {
            eprintln!("cockpit hub: cannot listen on {}: {why}", socket.display());
            return;
        }
    };
    let inode = std::fs::metadata(&socket).map(|m| m.ino()).unwrap_or_default();
    let _ = cockpit_core::notify::GUI.set(views::notify);
    // The store opens before the first request, waiting out a hub that hands over to this one.
    drop(store::global());
    let last = Arc::new(AtomicU64::new(server::now_secs()));
    std::thread::spawn({
        let (last, socket) = (last.clone(), socket.clone());
        move || exit_when_idle(&socket, inode, &last)
    });
    std::thread::spawn(watch);
    std::thread::spawn(search::sessions::keep);
    std::thread::spawn(routines::keep);
    std::thread::spawn(cockpit_core::resume::keep);
    std::thread::spawn(move || {
        let dir = files::herdr_dir();
        cockpit_update::keep_current(&dir, |binary| hand_over(&dir, binary, &socket, inode));
    });
    if let Err(error) = serve_tcp(tcp, last.clone()) {
        eprintln!("cockpit hub: cannot start TCP: {error}");
    }
    let gate = server::Gate { token: None, frontend: None, read_only: false };
    for stream in listener.incoming().flatten() {
        last.store(server::now_secs(), Ordering::SeqCst);
        let (last, gate) = (last.clone(), gate.clone());
        std::thread::spawn(move || server::serve(stream, last, gate));
    }
}

/// Where the TCP side is: {port, token, pid}, readable by the user alone.
pub fn tcp_file() -> PathBuf {
    files::cache().join("hub.json")
}

/// A random token from the system's source, as hex.
fn token() -> std::io::Result<String> {
    let mut bytes = [0u8; 16];
    std::io::Read::read_exact(&mut std::fs::File::open("/dev/urandom")?, &mut bytes)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Takes the desktop protocol's port on loopback, or on COCKPIT_HUB_LISTEN's explicit interface.
fn listen() -> std::io::Result<(TcpListener, std::net::IpAddr)> {
    let host = std::env::var("COCKPIT_HUB_LISTEN").unwrap_or_else(|_| "127.0.0.1".into());
    let host: std::net::IpAddr = host.parse().map_err(|_| std::io::Error::other("COCKPIT_HUB_LISTEN must be an IP address"))?;
    if host.is_unspecified() || host.is_multicast() {
        return Err(std::io::Error::other("COCKPIT_HUB_LISTEN must name one interface, not a wildcard or multicast address"));
    }
    Ok((TcpListener::bind((host, 0))?, host))
}

/// Serves that port: hub.json carries the token each request needs, and off loopback the connection reads only.
fn serve_tcp((listener, host): (TcpListener, std::net::IpAddr), last: Arc<AtomicU64>) -> std::io::Result<()> {
    let port = listener.local_addr()?.port();
    let token = token()?;
    let info = serde_json::json!({"host": host.to_string(), "port": port, "token": token, "pid": std::process::id()});
    let path = tcp_file();
    let tmp = path.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp);
    let written = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp).and_then(|mut f| f.write_all(info.to_string().as_bytes()));
    written.and_then(|_| std::fs::rename(&tmp, &path))?;
    let gate = server::Gate { token: Some(token), frontend: Some(cockpit_core::frontend::Frontend::App), read_only: !host.is_loopback() };
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            last.store(server::now_secs(), Ordering::SeqCst);
            let (last, gate) = (last.clone(), gate.clone());
            std::thread::spawn(move || server::serve(stream, last, gate));
        }
    });
    Ok(())
}

/// Takes hub.json away when it names this hub.
fn drop_tcp_file() {
    let path = tcp_file();
    if files::load(&path).is_some_and(|v| v["pid"] == std::process::id()) {
        let _ = std::fs::remove_file(path);
    }
}

/// A newer binary went into bin/: a hub run from it takes the socket over, the space apps restart on it, this one ends.
fn hand_over(dir: &Path, binary: &Path, socket: &Path, inode: u64) {
    drop_tcp_file();
    if std::fs::metadata(socket).is_ok_and(|m| m.ino() == inode) {
        let _ = std::fs::remove_file(socket);
    }
    store::global().close();
    let _ = std::process::Command::new(binary).arg("hub").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
    cockpit_update::restart_apps(dir);
    std::process::exit(0);
}

/// Whether a pinned space's app runs, which keeps the hub fetching for it, requests or not. Only this hub's own count:
/// one under another HOME (a test sandbox) talks to a hub of its own.
fn space_apps_running() -> bool {
    let Ok(out) = std::process::Command::new("pgrep").args(["-f", "cockpit (inbox|runbook|todo|meetings|monitor|routines|mail) cockpit-space"]).output() else {
        return false;
    };
    let home = files::home();
    String::from_utf8_lossy(&out.stdout).lines().filter_map(|l| l.trim().parse::<u32>().ok()).any(|pid| home_of(pid).is_none_or(|h| h == home))
}

/// A process's HOME, when the system says: /proc on Linux, `ps e` on macOS.
fn home_of(pid: u32) -> Option<PathBuf> {
    if let Ok(environ) = std::fs::read(format!("/proc/{pid}/environ")) {
        return environ.split(|b| *b == 0).find_map(|v| v.strip_prefix(b"HOME=")).map(|h| PathBuf::from(String::from_utf8_lossy(h).into_owned()));
    }
    let out = std::process::Command::new("ps").args(["eww", "-o", "command=", "-p", &pid.to_string()]).output().ok()?;
    let line = String::from_utf8_lossy(&out.stdout).into_owned();
    line.split(' ').find_map(|w| w.strip_prefix("HOME=")).map(PathBuf::from)
}

/// Whether the socket path still holds this hub's own socket: gone (its folder removed) or taken by a newer hub, this
/// one has no one left to serve.
fn socket_is_ours(socket: &Path, inode: u64) -> bool {
    std::fs::metadata(socket).is_ok_and(|m| m.ino() == inode)
}

fn exit_when_idle(socket: &Path, inode: u64, last: &AtomicU64) {
    let limit = idle_limit();
    loop {
        std::thread::sleep(Duration::from_secs((limit / 10).clamp(1, 30)));
        if !socket_is_ours(socket, inode) {
            drop_tcp_file();
            std::process::exit(0);
        }
        let is_idle = server::now_secs().saturating_sub(last.load(Ordering::SeqCst)) >= limit;
        if is_idle && !store::global().has_subscribers() && !views::has_subscribers() && !space_apps_running() && !routines::wanted() {
            drop_tcp_file();
            let _ = std::fs::remove_file(socket);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_process_home_is_read_from_the_system() {
        let home = std::env::var("HOME").map(PathBuf::from).unwrap();
        assert_eq!(home_of(std::process::id()), Some(home));
    }

    #[test]
    fn a_socket_gone_is_no_longer_ours() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("hub.sock");
        let _listener = UnixListener::bind(&socket).unwrap();
        let inode = std::fs::metadata(&socket).unwrap().ino();
        assert!(socket_is_ours(&socket, inode));
        drop(dir);
        assert!(!socket_is_ours(&socket, inode), "its folder removed, as a test sandbox's is");
    }
}
