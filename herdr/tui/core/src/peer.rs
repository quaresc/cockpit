//! Which process sent the request the hub is serving, and where it stands: what only a person may do (answer a
//! question or a permission, turn on a mode that runs tools unasked, start an agent) is refused to an agent's own tools,
//! to work cockpit runs on someone's behalf, and to a sender cockpit cannot place.

use std::cell::RefCell;
use std::sync::RwLock;

/// The other end of the connection a hub thread serves.
#[derive(Clone, Debug, PartialEq)]
pub enum Peer {
    /// A unix socket, which names its process when the system says.
    Unix(Option<u32>),
    /// A TCP connection on loopback, known by its source port and the hub's port.
    Tcp { port: u16, hub: u16 },
    /// Another thread's sender, carried into work it started.
    Resolved(Sender),
}

/// Who sent a request, as far as the system says.
#[derive(Clone, Debug, PartialEq)]
pub enum Sender {
    /// Cockpit's own code acting for the person, said so with `trusted`.
    InProcess,
    Process(u32),
    /// A connection whose process the system does not name.
    Unknown,
    /// Where the sender stood when the request came, carried into work it started.
    Placed(Who),
}

/// Where a sender stands.
#[derive(Clone, Debug, PartialEq)]
pub enum Who {
    /// A process whose parents reach a window or terminal the person uses, past no agent.
    Person,
    /// A process in an agent's tree: its handle, or an empty one for an agent cockpit does not watch.
    Agent(String),
    /// Refused whatever it asks of a person: why cockpit cannot tell who sent it.
    Unplaced(String),
}

/// Places a process: who it is, as the agents crate reads the process table.
pub type Place = fn(u32) -> Who;

static PLACE: RwLock<Option<Place>> = RwLock::new(None);

thread_local! {
    static PEER: RefCell<Option<Peer>> = const { RefCell::new(None) };
}

/// The cli (or a test's hub) says how to place a process; the first one registered stays.
pub fn on_place(place: Place) {
    let mut held = PLACE.write().unwrap_or_else(|e| e.into_inner());
    if held.is_none() {
        *held = Some(place);
    }
}

/// The hub notes the unix peer of the connection this thread serves.
pub fn set(pid: Option<u32>) {
    PEER.with(|p| *p.borrow_mut() = Some(Peer::Unix(pid)));
}

/// The hub notes the TCP peer of the connection this thread serves.
pub fn set_tcp(port: u16, hub: u16) {
    PEER.with(|p| *p.borrow_mut() = Some(Peer::Tcp { port, hub }));
}

/// This thread acts for the person by cockpit's own decision (a test, or hub work the person's checked request
/// started): only code that says so is taken for them.
pub fn trusted() {
    adopt(Sender::InProcess);
}

/// Work this thread starts on another thread carries the sender with it: `adopt(carry())` there.
pub fn adopt(sender: Sender) {
    PEER.with(|p| *p.borrow_mut() = Some(Peer::Resolved(sender)));
}

/// Runs `f` as `sender` would be served, then puts back what this thread served before.
pub fn with<T>(sender: Sender, f: impl FnOnce() -> T) -> T {
    let before = PEER.with(|p| p.borrow_mut().replace(Peer::Resolved(sender)));
    let out = f();
    PEER.with(|p| *p.borrow_mut() = before);
    out
}

/// Whether this thread runs cockpit's own work: it serves no connection and carries no sender, or said it acts for the
/// person (`trusted`). What a client asks for never reads so.
pub fn in_process() -> bool {
    PEER.with(|p| matches!(*p.borrow(), None | Some(Peer::Resolved(Sender::InProcess))))
}

/// Who sent the request being served; a TCP peer's process is looked up once per connection, when something first
/// asks. A thread that serves no connection and said nothing is unknown: what only a person may do fails closed there.
pub fn current() -> Sender {
    let peer = PEER.with(|p| p.borrow().clone());
    match peer {
        None => Sender::Unknown,
        Some(Peer::Resolved(sender)) => sender,
        Some(Peer::Unix(Some(pid))) => Sender::Process(pid),
        Some(Peer::Unix(None)) => Sender::Unknown,
        Some(Peer::Tcp { port, hub }) => {
            let found = tcp_pid(port, hub);
            PEER.with(|p| *p.borrow_mut() = Some(Peer::Unix(found)));
            found.map_or(Sender::Unknown, Sender::Process)
        }
    }
}

/// What cannot be placed for want of a process is told.
pub const UNNAMED: &str = "cockpit cannot tell which process sent this, so it takes no answer from it";

/// Where the sender of the request being served stands, placed now.
pub fn who() -> Who {
    match current() {
        Sender::InProcess => Who::Person,
        Sender::Unknown => Who::Unplaced(UNNAMED.into()),
        Sender::Placed(who) => who,
        Sender::Process(pid) => match *PLACE.read().unwrap_or_else(|e| e.into_inner()) {
            Some(place) => place(pid),
            None => Who::Unplaced("cockpit cannot read where this process comes from here".into()),
        },
    }
}

/// The sender as work started for it carries it: where it stood when it asked, not a pid that may be gone by then.
pub fn carry() -> Sender {
    match current() {
        Sender::InProcess => Sender::InProcess,
        _ => Sender::Placed(who()),
    }
}

/// What only a person does: refused to an agent's tools (with `refused`) and to a sender cockpit cannot place.
pub fn person(refused: &str) -> Result<(), String> {
    match who() {
        Who::Person => Ok(()),
        Who::Agent(_) => Err(refused.into()),
        Who::Unplaced(why) => Err(why),
    }
}

/// What a request from an agent's tools that targets another agent is told.
pub const NOT_YOURS: &str = "an agent's tools act on their own agent only, never on another";

/// What an agent's own tools may do through cockpit: act on their own agent (a `show` view's answer reaches it), never
/// on another; a person may act on any.
pub fn may_touch(target: &str) -> Result<(), String> {
    match who() {
        Who::Person => Ok(()),
        Who::Agent(own) if !own.is_empty() && own == target => Ok(()),
        Who::Agent(_) => Err(NOT_YOURS.into()),
        Who::Unplaced(why) => Err(why),
    }
}

/// The one process other than the hub holding the loopback socket whose source port is `port`.
fn tcp_pid(port: u16, hub: u16) -> Option<u32> {
    #[cfg(target_os = "linux")]
    {
        linux_tcp_pid(port, hub)
    }
    #[cfg(not(target_os = "linux"))]
    {
        // Every connection to the hub's port, whatever loopback address it took; the client's own side is the one
        // whose local port is the source port.
        let text = lsof(&["-nP", "-b", "-w", "-sTCP:ESTABLISHED", &format!("-iTCP:{hub}"), "-Fpn"])?;
        let (own, mut at, mut pids) = (std::process::id(), None, Vec::new());
        for line in text.lines() {
            if let Some(pid) = line.strip_prefix('p') {
                at = pid.parse::<u32>().ok();
            } else if let (Some(name), Some(pid)) = (line.strip_prefix('n'), at)
                && name.split("->").next().is_some_and(|local| local.ends_with(&format!(":{port}")))
                && pid != own
                && !pids.contains(&pid)
            {
                pids.push(pid);
            }
        }
        match pids.as_slice() {
            [one] => Some(*one),
            _ => None,
        }
    }
}

/// `lsof` by its own path, given a few seconds at most: what it printed, or None.
pub fn lsof(args: &[&str]) -> Option<String> {
    let bin = ["/usr/sbin/lsof", "/usr/bin/lsof", "/sbin/lsof"].into_iter().find(|p| std::path::Path::new(p).exists())?;
    let mut child = std::process::Command::new(bin)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let mut out = child.stdout.take()?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut text = String::new();
        let _ = std::io::Read::read_to_string(&mut out, &mut text);
        let _ = tx.send(text);
    });
    let text = rx.recv_timeout(std::time::Duration::from_secs(3)).ok();
    let _ = child.kill();
    let _ = child.wait();
    text
}

/// /proc/net/tcp names the socket's inode by its two ports; the process is the one with that socket among its files.
#[cfg(target_os = "linux")]
fn linux_tcp_pid(port: u16, hub: u16) -> Option<u32> {
    let port_of = |addr: &str| addr.rsplit(':').next().and_then(|p| u16::from_str_radix(p, 16).ok());
    let inode = ["/proc/net/tcp", "/proc/net/tcp6"].iter().filter_map(|f| std::fs::read_to_string(f).ok()).find_map(|text| {
        text.lines().skip(1).find_map(|l| {
            let w: Vec<&str> = l.split_whitespace().collect();
            (w.len() > 9 && port_of(w[1]) == Some(port) && port_of(w[2]) == Some(hub)).then(|| w[9].to_string())
        })
    })?;
    let wanted = format!("socket:[{inode}]");
    let own = std::process::id();
    let holders: Vec<u32> = std::fs::read_dir("/proc")
        .ok()?
        .flatten()
        .filter_map(|p| {
            let pid: u32 = p.file_name().to_str()?.parse().ok()?;
            let fds = std::fs::read_dir(p.path().join("fd")).ok()?;
            fds.flatten().any(|fd| std::fs::read_link(fd.path()).is_ok_and(|l| l.to_string_lossy() == wanted)).then_some(pid)
        })
        .filter(|p| *p != own)
        .collect();
    match holders.as_slice() {
        [one] => Some(*one),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A TCP peer's lookup is tests/peer_tcp.rs's, alone in its binary: a socket another test's child inherits would
    /// name that child.
    #[test]
    fn an_unnamed_sender_is_unknown_and_a_carried_one_is_kept() {
        set(None);
        assert_eq!(current(), Sender::Unknown);
        set(Some(42));
        assert_eq!(current(), Sender::Process(42));
        // A thread that serves nothing is unknown unless it says it acts for the person, or carries a sender.
        assert_eq!(std::thread::spawn(current).join().unwrap(), Sender::Unknown, "fails closed");
        let carried = std::thread::spawn(|| {
            adopt(Sender::Process(42));
            current()
        });
        assert_eq!(carried.join().unwrap(), Sender::Process(42));
        trusted();
        assert_eq!(current(), Sender::InProcess);
        // Cockpit's own work is a thread that serves nobody or said so; any connection's, or a sender carried, is not.
        assert!(in_process());
        assert!(std::thread::spawn(in_process).join().unwrap(), "a thread of the hub's own");
        for peer in [Peer::Unix(Some(42)), Peer::Unix(None), Peer::Resolved(Sender::Placed(Who::Person))] {
            assert!(
                !std::thread::spawn(move || {
                    PEER.with(|p| *p.borrow_mut() = Some(peer));
                    in_process()
                })
                .join()
                .unwrap()
            );
        }
    }

    #[test]
    fn a_verdict_is_carried_and_answers_what_only_a_person_or_the_agent_itself_may_do() {
        type Checks = (Result<(), String>, Result<(), String>, Result<(), String>);
        let on = |who: Who| -> Checks {
            std::thread::spawn(move || {
                adopt(Sender::Placed(who));
                (person("not yours"), may_touch("app:t3"), may_touch("app:t4"))
            })
            .join()
            .unwrap()
        };
        assert_eq!(on(Who::Person), (Ok(()), Ok(()), Ok(())));
        assert_eq!(on(Who::Agent("app:t3".into())), (Err("not yours".into()), Ok(()), Err(NOT_YOURS.into())), "its own agent only");
        let unwatched = on(Who::Agent(String::new()));
        assert_eq!(unwatched, (Err("not yours".into()), Err(NOT_YOURS.into()), Err(NOT_YOURS.into())), "an agent cockpit does not watch owns nothing");
        let unplaced = on(Who::Unplaced("no idea".into()));
        assert_eq!((unplaced.0, unplaced.1), (Err("no idea".into()), Err("no idea".into())));
        // Nothing said, nothing served: refused.
        assert_eq!(std::thread::spawn(|| person("x")).join().unwrap(), Err(UNNAMED.into()));
        // What a thread carries is the verdict reached when the request came.
        let carried = std::thread::spawn(|| {
            adopt(Sender::Placed(Who::Agent("app:t3".into())));
            carry()
        })
        .join()
        .unwrap();
        assert_eq!(carried, Sender::Placed(Who::Agent("app:t3".into())));
    }
}
