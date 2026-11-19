//! Where a process stands, from its whole line of parents: in a watched agent's tree, under an agent cockpit does not
//! watch, under work cockpit runs for someone (the hub, a `cockpit show` pane, a notification's click, start work's
//! setup), under a window or terminal the person uses, or none of these. A front is known by its executable's real
//! path, never by the name a process gives itself.

use std::cell::RefCell;
use std::collections::HashMap;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

/// Every process as `pid → (parent, name)`, or an empty table when `ps` fails.
pub fn table() -> HashMap<u32, (u32, String)> {
    // By its own path: a PATH the hub was started with cannot hand it another `ps`, or none.
    let text = ps(&["-A", "-o", "pid=,ppid=,comm="]).unwrap_or_default();
    text.lines()
        .filter_map(|l| {
            let mut w = l.split_whitespace();
            let (pid, parent) = (w.next()?.parse().ok()?, w.next()?.parse().ok()?);
            Some((pid, (parent, w.collect::<Vec<_>>().join(" "))))
        })
        .collect()
}

fn ps(args: &[&str]) -> Option<String> {
    let bin = ["/bin/ps", "/usr/bin/ps"].into_iter().find(|p| Path::new(p).exists())?;
    crate::term::output(&std::iter::once(bin).chain(args.iter().copied()).map(str::to_string).collect::<Vec<_>>(), 5)
}

/// What the walk asks of a process beyond the table.
pub trait Look {
    /// The real path of the executable it runs (symlinks resolved), when the system says.
    fn exe(&self, pid: u32) -> Option<PathBuf>;
    /// Its command line, its words split on spaces.
    fn args(&self, pid: u32) -> Vec<String>;
    /// The processes about to be asked about, read at once where the system allows.
    fn prefetch(&self, _pids: &[u32]) {}
    /// The far end of the TCP connection an ssh server process holds, when it holds one cockpit can read.
    fn ssh_from(&self, pid: u32) -> Option<IpAddr>;
    /// The account's home, from the user database rather than the environment.
    fn home(&self) -> Option<PathBuf>;
    /// Whether the executable it started from is known gone from disk since (`listed`, the path the process table names).
    fn gone(&self, pid: u32, listed: &str) -> bool;
}

/// The running system: proc_pidpath or /proc, `ps` and `lsof` by their own paths; command lines read once a walk.
#[derive(Default)]
pub struct System {
    args: RefCell<HashMap<u32, Vec<String>>>,
}

impl Look for System {
    fn exe(&self, pid: u32) -> Option<PathBuf> {
        exe_of(pid)
    }

    fn args(&self, pid: u32) -> Vec<String> {
        if let Some(found) = self.args.borrow().get(&pid) {
            return found.clone();
        }
        self.prefetch(&[pid]);
        self.args.borrow().get(&pid).cloned().unwrap_or_default()
    }

    fn prefetch(&self, pids: &[u32]) {
        let list = pids.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
        let text = ps(&["-o", "pid=,args=", "-p", &list]).unwrap_or_default();
        let mut held = self.args.borrow_mut();
        for pid in pids {
            held.entry(*pid).or_default();
        }
        for line in text.lines() {
            let mut words = line.split_whitespace();
            if let Some(pid) = words.next().and_then(|p| p.parse::<u32>().ok()) {
                held.insert(pid, words.map(str::to_string).collect());
            }
        }
    }

    fn ssh_from(&self, pid: u32) -> Option<IpAddr> {
        let text = cockpit_core::peer::lsof(&["-nP", "-a", "-p", &pid.to_string(), "-iTCP", "-sTCP:ESTABLISHED", "-Fn"])?;
        text.lines().filter_map(|l| l.strip_prefix('n')?.split_once("->")).find_map(|(_, far)| far_end(far))
    }

    fn gone(&self, pid: u32, listed: &str) -> bool {
        gone_from_disk(pid, listed)
    }

    fn home(&self) -> Option<PathBuf> {
        account_home()
    }
}

/// The account's home as the user database has it, its real path.
fn account_home() -> Option<PathBuf> {
    // SAFETY: getpwuid returns null or a record in static storage; its home is copied out at once.
    let home = unsafe {
        let pw = libc::getpwuid(libc::getuid());
        if pw.is_null() || (*pw).pw_dir.is_null() {
            return None;
        }
        std::ffi::CStr::from_ptr((*pw).pw_dir).to_string_lossy().into_owned()
    };
    std::fs::canonicalize(&home).ok()
}

/// The address of `host:port` (`[::1]:22` for IPv6) as lsof prints the far end of a connection.
fn far_end(far: &str) -> Option<IpAddr> {
    let host = far.rsplit_once(':').map_or(far, |(h, _)| h);
    host.trim_start_matches('[').trim_end_matches(']').parse().ok()
}

#[cfg(target_os = "macos")]
fn exe_of(pid: u32) -> Option<PathBuf> {
    let mut buf = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    // SAFETY: proc_pidpath writes at most `buf.len()` bytes into `buf` and returns how many it wrote.
    let n = unsafe { libc::proc_pidpath(pid as libc::c_int, buf.as_mut_ptr().cast(), buf.len() as u32) };
    (n > 0).then(|| PathBuf::from(String::from_utf8_lossy(&buf[..n as usize]).into_owned()))
}

/// macOS keeps no mark of a deleted executable: the path the process table names no longer being there says it.
#[cfg(target_os = "macos")]
fn gone_from_disk(_pid: u32, listed: &str) -> bool {
    Path::new(listed).is_absolute() && !Path::new(listed).exists()
}

/// Linux marks a binary replaced or removed since it started: its link reads "(deleted)".
#[cfg(not(target_os = "macos"))]
fn gone_from_disk(pid: u32, _listed: &str) -> bool {
    std::fs::read_link(format!("/proc/{pid}/exe")).is_ok_and(|p| p.to_string_lossy().ends_with(" (deleted)"))
}

#[cfg(not(target_os = "macos"))]
fn exe_of(pid: u32) -> Option<PathBuf> {
    // A binary replaced or removed since it started reads "(deleted)": no known path, so never a front.
    std::fs::read_link(format!("/proc/{pid}/exe")).ok().filter(|p| !p.to_string_lossy().ends_with(" (deleted)"))
}

/// Where a process's parents lead.
#[derive(Debug, PartialEq)]
pub enum Walk {
    /// Into a watched agent's tree: its handle.
    Agent(String),
    /// Past a process running an agent's program cockpit has not placed: that program.
    Unwatched(String),
    /// Past cockpit's own work (the hub, a `cockpit show` pane, `cockpit focus`, `act` or `setup-run`), or from the hub
    /// itself: it runs for an agent, a routine, a view, a click from outside or a repo's setup, never for the person.
    Cockpit,
    /// To a window or terminal the person uses, past no agent.
    Front,
    /// Through an ssh server whose client is this machine itself, or one whose client cockpit cannot read.
    Ssh(Option<IpAddr>),
    /// To the system's root (launchd, pid 1) past no front: started detached (setsid, nohup, launchd).
    Detached,
    /// Past a window or terminal program whose executable is known gone from disk since it started, as after an update:
    /// that program's name. One whose executable is merely unreadable (a root sshd) is passed by, as no front.
    Replaced(String),
    /// Off the process table.
    Unlisted,
}

/// The apps a person types or clicks in: their bundle and the executable its Info.plist names (Warp's is `stable`).
const APPS: [(&str, &str); 11] = [
    ("Terminal.app", "Terminal"),
    ("iTerm.app", "iTerm2"),
    ("iTerm.app", "iTermServer"),
    ("Ghostty.app", "ghostty"),
    ("WezTerm.app", "wezterm-gui"),
    ("kitty.app", "kitty"),
    ("Alacritty.app", "alacritty"),
    ("Warp.app", "stable"),
    ("Herdr.app", "Herdr"),
    ("Herdr Core.app", "herdr-core"),
    // The notifier, which runs a notification's button only once the person clicks it.
    ("Cockpit.app", "cockpit-notify"),
];

/// Terminals and multiplexers outside a bundle: one counts where it is installed (`installed`), so a copy under that
/// name elsewhere does not, whatever PATH the hub runs on.
const PROGRAMS: [&str; 16] = [
    "herdr",
    "herdr-core",
    "tmux",
    "mosh-server",
    "sshd",
    "sshd-session",
    "gnome-terminal-server",
    "konsole",
    "xterm",
    "kitty",
    "alacritty",
    "wezterm-gui",
    "foot",
    "ghostty",
    "xfce4-terminal",
    "tilix",
];

/// Whether `exe`, a process's real executable path, is a window or terminal the person uses.
fn front(exe: &Path, look: &dyn Look) -> bool {
    let parts: Vec<&str> = exe.iter().filter_map(|p| p.to_str()).collect();
    let bundled = match parts.as_slice() {
        [.., bundle, "Contents", "MacOS", name] => APPS.contains(&(*bundle, *name)),
        _ => false,
    };
    // iTerm2 runs its shells under a copy of its server it keeps in its own folder.
    let iterm =
        parts.len() >= 3 && parts[parts.len() - 3..parts.len() - 1] == ["Application Support", "iTerm2"] && parts[parts.len() - 1].starts_with("iTermServer-");
    let name = exe.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    let program = PROGRAMS.contains(&name) && installed(exe, look.home().as_deref());
    bundled || iterm || program
}

/// Package managers' folders, each executable at `<folder>/<package>/<version>/bin/<name>`: Homebrew's Cellars.
const CELLARS: [&str; 3] = ["/opt/homebrew/Cellar", "/usr/local/Cellar", "/home/linuxbrew/.linuxbrew/Cellar"];

/// Whether `exe`, a real path, is where a terminal program is installed: the system's own folders (`/usr` but
/// `/usr/local`, `/bin`, `/sbin`), MacPorts, Nix, snaps, Homebrew's Cellars, and the account's `~/.local/bin` and
/// `~/.cargo/bin`.
fn installed(exe: &Path, home: Option<&Path>) -> bool {
    let system = (exe.starts_with("/usr/") && !exe.starts_with("/usr/local/")) || exe.starts_with("/bin/") || exe.starts_with("/sbin/");
    let managed = ["/opt/local/bin/", "/opt/local/sbin/", "/nix/store/", "/run/current-system/sw/", "/snap/"].iter().any(|d| exe.starts_with(d));
    let cellar =
        CELLARS.iter().any(|c| exe.strip_prefix(c).is_ok_and(|rest| matches!(rest.iter().collect::<Vec<_>>().as_slice(), [_, _, bin, _] if *bin == "bin")));
    let own = home.is_some_and(|h| [".local/bin", ".cargo/bin"].iter().any(|d| exe.parent() == Some(h.join(d).as_path())));
    system || managed || cellar || own
}

fn ssh_server(exe: &Path) -> bool {
    exe.file_name().is_some_and(|n| n == "sshd" || n == "sshd-session")
}

/// The interpreters an agent's program may run under: its script's path then names it.
const INTERPRETERS: [&str; 6] = ["node", "bun", "deno", "python", "python3", "ruby"];

/// The agent program `pid` runs, when it runs one of `programs`: by the name it gives itself, its executable or, under
/// an interpreter, its script. A tool cannot rename a parent, so these names only ever add refusals.
fn agent_program(pid: u32, name: &str, programs: &[String], look: &dyn Look) -> Option<String> {
    let stem = |w: &str| Path::new(w).file_stem().and_then(|s| s.to_str()).map(str::to_string).unwrap_or_default();
    let base = |w: &str| Path::new(w).file_name().and_then(|s| s.to_str()).map(str::to_string).unwrap_or_default();
    let known = |w: &str| programs.iter().find(|p| p.as_str() == w).cloned();
    if let Some(found) = known(&base(name)) {
        return Some(found);
    }
    let args = look.args(pid);
    let exe = look.exe(pid).map(|p| base(&p.to_string_lossy())).unwrap_or_default();
    if let Some(found) = known(&exe).or_else(|| args.first().and_then(|a| known(&base(a)))) {
        return Some(found);
    }
    let interpreted = args.first().is_some_and(|a| INTERPRETERS.contains(&stem(a).trim_end_matches(|c: char| c.is_ascii_digit() || c == '.')));
    let script = args.iter().skip(1).find(|a| !a.starts_with('-'))?;
    if !interpreted {
        return None;
    }
    known(&stem(script)).or_else(|| Path::new(script).iter().filter_map(|c| c.to_str()).find_map(known))
}

/// The commands of cockpit's own work, by the word after `cockpit`, and whether the process itself is that work as well
/// as what it starts. The hub and a `cockpit show` pane run commands an agent or a routine gave them; `focus` and `act`
/// run what a notification's click or a window command names, from outside the person's hands; `setup-run` runs start
/// work's setup command, the repo's code.
const COCKPIT_WORK: [(&str, bool); 5] = [("hub", false), ("show", false), ("focus", true), ("act", true), ("setup-run", true)];

/// Whether `pid` is cockpit's own work: the hub (`own`) or a command of `COCKPIT_WORK`; `sender` when it is the process
/// that sent the request rather than one above it.
fn cockpit_work(pid: u32, own: u32, sender: bool, look: &dyn Look) -> bool {
    if pid == own {
        return true;
    }
    if !look.exe(pid).is_some_and(|p| p.file_name().is_some_and(|n| n == "cockpit")) {
        return false;
    }
    // The command line comes split on spaces: the word after the program's own, wherever a space in its path cut it.
    let args = look.args(pid);
    let word = args.iter().position(|a| a.ends_with("cockpit")).and_then(|at| args.get(at + 1)).map(String::as_str).unwrap_or_default();
    COCKPIT_WORK.iter().any(|(w, itself)| *w == word && (*itself || !sender))
}

/// Follows `pid`'s whole line of parents in `table`: a watched agent's root anywhere on it (`roots`) makes it that
/// agent's, then an agent program anywhere, then cockpit's own work above it; else the first front decides, an ssh
/// server by where its client is. `own` is the hub's pid, `programs` every agent program's name.
pub fn walk(pid: u32, table: &HashMap<u32, (u32, String)>, roots: &[(String, u32)], programs: &[String], own: u32, look: &dyn Look) -> Walk {
    if !table.contains_key(&pid) {
        return Walk::Unlisted;
    }
    let mut chain = vec![pid];
    while let Some((parent, _)) = table.get(chain.last().unwrap_or(&0)) {
        if *parent <= 1 || chain.contains(parent) || chain.len() >= 64 {
            break;
        }
        chain.push(*parent);
    }
    if let Some(handle) = chain.iter().find_map(|p| roots.iter().find(|(_, root)| root == p).map(|(h, _)| h.clone())) {
        return Walk::Agent(handle);
    }
    look.prefetch(&chain);
    let name = |p: &u32| table.get(p).map_or("", |(_, n)| n.as_str());
    if let Some(program) = chain.iter().find_map(|p| agent_program(*p, name(p), programs, look)) {
        return Walk::Unwatched(program);
    }
    if pid == own || chain.iter().enumerate().any(|(i, p)| cockpit_work(*p, own, i == 0, look)) {
        return Walk::Cockpit;
    }
    let mut replaced = None;
    for (i, p) in chain.iter().enumerate() {
        let Some(exe) = look.exe(*p) else {
            let program = Path::new(name(p)).file_name().and_then(|n| n.to_str()).unwrap_or_default();
            let front_named = APPS.iter().any(|(_, exe)| *exe == program) || PROGRAMS.contains(&program);
            if replaced.is_none() && front_named && look.gone(*p, name(p)) {
                replaced = Some(program.to_string());
            }
            continue;
        };
        if !front(&exe, look) {
            continue;
        }
        if !ssh_server(&exe) {
            return Walk::Front;
        }
        // The server's own process for this login holds the connection: it or one above it.
        let from = chain[i..].iter().take_while(|q| look.exe(**q).is_some_and(|e| ssh_server(&e))).find_map(|q| look.ssh_from(*q));
        return match from {
            Some(ip) if !ip.is_loopback() => Walk::Front,
            from => Walk::Ssh(from),
        };
    }
    replaced.map_or(Walk::Detached, Walk::Replaced)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A system made up: executables, command lines and ssh clients by pid, and the account's home.
    #[derive(Default)]
    struct Fake {
        exes: HashMap<u32, &'static str>,
        args: HashMap<u32, &'static str>,
        ssh: HashMap<u32, IpAddr>,
        home: Option<&'static str>,
        gone: Vec<u32>,
    }

    impl Look for Fake {
        fn exe(&self, pid: u32) -> Option<PathBuf> {
            self.exes.get(&pid).map(PathBuf::from)
        }
        fn args(&self, pid: u32) -> Vec<String> {
            self.args.get(&pid).map(|a| a.split_whitespace().map(str::to_string).collect()).unwrap_or_default()
        }
        fn ssh_from(&self, pid: u32) -> Option<IpAddr> {
            self.ssh.get(&pid).copied()
        }
        fn home(&self) -> Option<PathBuf> {
            self.home.map(PathBuf::from)
        }
        fn gone(&self, pid: u32, _listed: &str) -> bool {
            self.gone.contains(&pid)
        }
    }

    fn programs() -> Vec<String> {
        ["claude", "claude-code", "codex"].map(str::to_string).to_vec()
    }

    fn table(rows: &[(u32, u32, &str)]) -> HashMap<u32, (u32, String)> {
        rows.iter().map(|(p, pp, n)| (*p, (*pp, n.to_string()))).collect()
    }

    #[test]
    fn an_app_updated_on_disk_while_it_runs_is_named_not_trusted() {
        let t = table(&[(945, 1, "/Users/me/Applications/Herdr.app/Contents/MacOS/Herdr"), (946, 945, "cockpit")]);
        let updated = Fake { gone: vec![945], ..Fake::default() };
        assert_eq!(walk(946, &t, &[], &programs(), 999, &updated), Walk::Replaced("Herdr".into()));
        assert_eq!(walk(946, &t, &[], &programs(), 999, &Fake::default()), Walk::Detached, "an executable merely unreadable is no update");
        let root_sshd = table(&[(70, 1, "/usr/sbin/sshd"), (71, 70, "sshd-session"), (72, 71, "cockpit")]);
        assert_eq!(walk(72, &root_sshd, &[], &programs(), 999, &Fake::default()), Walk::Detached, "a root sshd cockpit cannot read");
        let t = table(&[(20, 1, "launchd-helper"), (21, 20, "cockpit")]);
        assert_eq!(walk(21, &t, &[], &programs(), 999, &Fake::default()), Walk::Detached, "an unknown program stays detached");
    }

    #[test]
    fn a_front_is_its_executable_path_never_its_name() {
        let t = table(&[(10, 1, "Ghostty"), (11, 10, "zsh"), (12, 11, "Terminal"), (13, 12, "cockpit")]);
        let mut look = Fake::default();
        look.exes.insert(10, "/usr/local/bin/sleep");
        look.exes.insert(12, "/tmp/x/Terminal");
        assert_eq!(walk(13, &t, &[], &programs(), 999, &look), Walk::Detached, "names alone reach no front");
        look.exes.insert(10, "/Applications/Ghostty.app/Contents/MacOS/ghostty");
        assert_eq!(walk(13, &t, &[], &programs(), 999, &look), Walk::Front);
        look.exes.insert(10, "/Applications/Warp.app/Contents/MacOS/stable");
        assert_eq!(walk(13, &t, &[], &programs(), 999, &look), Walk::Front, "Warp's executable is `stable`");
        look.exes.insert(10, "/Users/me/Library/Application Support/iTerm2/iTermServer-3.7.6");
        assert_eq!(walk(13, &t, &[], &programs(), 999, &look), Walk::Front, "iTerm2's server, under launchd");
        // A program counts where it is installed, never as a copy of that name elsewhere, whatever the PATH says.
        look.home = Some("/Users/me");
        for (exe, placed) in [
            ("/tmp/x/herdr", Walk::Detached),
            ("/opt/homebrew/bin/tmux", Walk::Detached),
            ("/Users/me/tools/tmux", Walk::Detached),
            ("/Users/me/.local/bin/sub/tmux", Walk::Detached),
            ("/Users/other/.local/bin/tmux", Walk::Detached),
            ("/opt/homebrew/Cellar/herdr/0.9.5/bin/herdr", Walk::Front),
            ("/opt/homebrew/Cellar/mosh/1.4.0/bin/mosh-server", Walk::Front),
            ("/opt/homebrew/Cellar/tmux/3.5/libexec/tmux", Walk::Detached),
            ("/usr/local/bin/tmux", Walk::Detached),
            ("/usr/bin/tmux", Walk::Front),
            ("/nix/store/abc-tmux-3.5/bin/tmux", Walk::Front),
            ("/Users/me/.local/bin/herdr", Walk::Front),
            ("/Users/me/.cargo/bin/herdr", Walk::Front),
        ] {
            look.exes.insert(10, exe);
            assert_eq!(walk(13, &t, &[], &programs(), 999, &look), placed, "{exe}");
        }
        // VS Code and Cursor run their own agents in their terminals: not fronts.
        look.exes.insert(10, "/Applications/Visual Studio Code.app/Contents/MacOS/Code");
        assert_eq!(walk(13, &t, &[], &programs(), 999, &look), Walk::Detached);
    }

    #[test]
    fn an_agent_anywhere_above_wins_over_any_front_below_it() {
        // herdr → zsh → claude → "Terminal" (a tool's own name) → cockpit: the agent's, whatever the tool calls itself.
        let t = table(&[(10, 1, "herdr"), (11, 10, "zsh"), (12, 11, "claude"), (13, 12, "Terminal"), (14, 13, "cockpit")]);
        let mut look = Fake::default();
        look.exes.insert(10, "/usr/bin/herdr");
        look.exes.insert(13, "/System/Applications/Utilities/Terminal.app/Contents/MacOS/Terminal");
        assert_eq!(walk(14, &t, &[("w1:p1".into(), 11)], &programs(), 999, &look), Walk::Agent("w1:p1".into()));
        assert_eq!(walk(14, &t, &[], &programs(), 999, &look), Walk::Unwatched("claude".into()), "not placed yet: still an agent");
        // Under node, the script names it.
        let t = table(&[(10, 1, "herdr"), (11, 10, "node"), (12, 11, "sh"), (13, 12, "cockpit")]);
        look.args.insert(11, "node /opt/lib/node_modules/@anthropic-ai/claude-code/cli.js --resume x");
        assert_eq!(walk(13, &t, &[], &programs(), 999, &look), Walk::Unwatched("claude-code".into()));
        look.args.insert(11, "node /opt/lib/node_modules/@openai/codex/bin/codex.js");
        assert_eq!(walk(13, &t, &[], &programs(), 999, &look), Walk::Unwatched("codex".into()));
        look.args.insert(11, "node /srv/app/server.js");
        assert_eq!(walk(13, &t, &[], &programs(), 999, &look), Walk::Front, "any other script is no agent");
    }

    #[test]
    fn what_cockpit_runs_for_someone_is_never_the_person() {
        // herdr → cockpit show deploy → sh (a log block's command) → cockpit.
        let t = table(&[(10, 1, "herdr"), (11, 10, "cockpit"), (12, 11, "sh"), (13, 12, "cockpit"), (20, 10, "cockpit"), (21, 20, "cockpit")]);
        let mut look = Fake::default();
        look.exes.insert(10, "/usr/bin/herdr");
        for pid in [11, 13, 20, 21] {
            look.exes.insert(pid, "/Users/me/.config/cockpit/herdr/bin/cockpit");
        }
        look.args.insert(11, "/Users/me/.config/cockpit/herdr/bin/cockpit show deploy");
        assert_eq!(walk(13, &t, &[], &programs(), 999, &look), Walk::Cockpit);
        assert_eq!(walk(11, &t, &[], &programs(), 999, &look), Walk::Front, "the pane itself is the person's");
        assert_eq!(walk(12, &t, &[], &programs(), 11, &look), Walk::Cockpit, "a child of the hub");
        assert_eq!(walk(11, &t, &[], &programs(), 11, &look), Walk::Cockpit, "the hub itself");
        // A space app under its supervisor (`cockpit space inbox`) is the person's.
        look.args.insert(20, "/Users/me/.config/cockpit/herdr/bin/cockpit space inbox");
        look.args.insert(21, "/Users/me/.config/cockpit/herdr/bin/cockpit inbox cockpit-space");
        assert_eq!(walk(21, &t, &[], &programs(), 999, &look), Walk::Front);
    }

    #[test]
    fn a_click_from_outside_and_a_repo_s_setup_are_cockpit_s_work_not_the_person_s() {
        // Herdr → cockpit focus act:merge:… (a toast's click, or `herdr-app do cockpit.target:…`) → cockpit act merge.
        let t =
            table(&[(10, 1, "Herdr"), (11, 10, "cockpit"), (12, 11, "cockpit"), (13, 10, "cockpit"), (14, 10, "zsh"), (15, 14, "cockpit"), (16, 15, "npm")]);
        let mut look = Fake::default();
        look.exes.insert(10, "/Applications/Herdr.app/Contents/MacOS/Herdr");
        for pid in [11, 12, 13, 15] {
            look.exes.insert(pid, "/Users/me/.config/cockpit/herdr/bin/cockpit");
        }
        look.args.insert(11, "/Users/me/.config/cockpit/herdr/bin/cockpit focus act:merge:https://github.com/acme/api/pull/4");
        look.args.insert(12, "/Users/me/.config/cockpit/herdr/bin/cockpit act merge https://github.com/acme/api/pull/4");
        assert_eq!(walk(12, &t, &[], &programs(), 999, &look), Walk::Cockpit, "under cockpit focus");
        assert_eq!(walk(11, &t, &[], &programs(), 999, &look), Walk::Cockpit, "cockpit focus itself");
        // The app's `cockpit()` helper runs `cockpit act` straight from the window: it is the sender.
        look.args.insert(13, "/Users/me/.config/cockpit/herdr/bin/cockpit act review https://github.com/acme/api/pull/4");
        assert_eq!(walk(13, &t, &[], &programs(), 999, &look), Walk::Cockpit);
        // Start work's setup in the person's terminal: `cockpit setup-run sh -c 'npm ci'` and what it runs.
        look.args.insert(15, "/Users/me/.config/cockpit/herdr/bin/cockpit setup-run /bin/sh -c npm ci");
        assert_eq!(walk(16, &t, &[], &programs(), 999, &look), Walk::Cockpit);
        assert_eq!(walk(15, &t, &[], &programs(), 999, &look), Walk::Cockpit);
        assert_eq!(walk(14, &t, &[], &programs(), 999, &look), Walk::Front, "the terminal's own shell stays the person's");
        // A space in the binary's path cuts the command line where ps splits it: the word after the program still counts.
        look.exes.insert(13, "/Users/Sam Lee/.local/bin/cockpit");
        look.args.insert(13, "/Users/Sam Lee/.local/bin/cockpit focus act:merge:https://github.com/acme/api/pull/4");
        assert_eq!(walk(13, &t, &[], &programs(), 999, &look), Walk::Cockpit);
        // The person's own cockpit commands stay theirs.
        look.args.insert(13, "/Users/me/.config/cockpit/herdr/bin/cockpit hub act review send [] ");
        assert_eq!(walk(13, &t, &[], &programs(), 999, &look), Walk::Front, "a request the hub's own command line sends");
        look.args.insert(13, "/Users/me/.config/cockpit/herdr/bin/cockpit agents answer w1:p1 1");
        assert_eq!(walk(13, &t, &[], &programs(), 999, &look), Walk::Front);
    }

    #[test]
    fn ssh_counts_from_another_machine_only() {
        let t = table(&[(10, 1, "sshd"), (11, 10, "sshd-session"), (12, 11, "sshd-session"), (13, 12, "zsh"), (14, 13, "cockpit")]);
        let mut look = Fake::default();
        look.exes.insert(10, "/usr/sbin/sshd");
        look.exes.insert(11, "/usr/libexec/sshd-session");
        look.exes.insert(12, "/usr/libexec/sshd-session");
        assert_eq!(walk(14, &t, &[], &programs(), 999, &look), Walk::Ssh(None), "a client cockpit cannot read");
        look.ssh.insert(12, "127.0.0.1".parse().unwrap());
        assert_eq!(walk(14, &t, &[], &programs(), 999, &look), Walk::Ssh(Some("127.0.0.1".parse().unwrap())));
        look.ssh.insert(12, "192.0.2.7".parse().unwrap());
        assert_eq!(walk(14, &t, &[], &programs(), 999, &look), Walk::Front);
        assert_eq!(far_end("[::1]:22"), Some("::1".parse().unwrap()));
        assert_eq!(far_end("10.0.0.2:54012"), Some("10.0.0.2".parse().unwrap()));
    }

    #[test]
    fn off_the_table_or_up_to_launchd_is_nobody() {
        let t = table(&[(10, 1, "sleep")]);
        assert_eq!(walk(42, &t, &[], &programs(), 999, &Fake::default()), Walk::Unlisted);
        assert_eq!(walk(10, &t, &[], &programs(), 999, &Fake::default()), Walk::Detached);
        let looped = table(&[(30, 31, "sh"), (31, 30, "sh")]);
        assert_eq!(walk(30, &looped, &[], &programs(), 999, &Fake::default()), Walk::Detached, "a cycle ends the walk, refused");
        assert_eq!(walk(12, &HashMap::new(), &[], &programs(), 999, &Fake::default()), Walk::Unlisted, "no table, nothing let through");
        let orphan = table(&[(13, 12, "cockpit")]);
        assert_eq!(walk(13, &orphan, &[], &programs(), 999, &Fake::default()), Walk::Detached, "a parent gone from the table ends the walk");
    }

    /// Set on this test binary run again as a stand-in parent.
    const STAND_IN: &str = "COCKPIT_TEST_STAND_IN";

    /// A parent of a few seconds' sleep, when this binary runs as one: it writes the sleep's pid where STAND_IN says.
    #[test]
    fn stand_in() {
        if let Some(at) = std::env::var_os(STAND_IN) {
            let mut child = std::process::Command::new("sleep").arg("5").spawn().unwrap();
            std::fs::write(at, child.id().to_string()).unwrap();
            let _ = child.wait();
        }
    }

    /// Spawns `script` under bash; what it prints on its first line is a pid.
    fn spawned(script: &str) -> (std::process::Child, u32) {
        use std::io::BufRead;
        let mut child =
            std::process::Command::new("bash").args(["-c", script]).stdout(std::process::Stdio::piped()).stdin(std::process::Stdio::null()).spawn().unwrap();
        let mut line = String::new();
        std::io::BufReader::new(child.stdout.as_mut().unwrap()).read_line(&mut line).unwrap();
        (child, line.trim().parse().unwrap())
    }

    #[test]
    fn real_processes_named_like_a_front_are_placed_by_what_they_run() {
        // A tool that calls itself Terminal, under an agent's root (this test process stands in for it).
        let (mut tool, pid) = spawned("exec -a Terminal sleep 5 & echo $!; wait");
        let own = std::process::id();
        let walked = walk(pid, &super::table(), &[("app:t1".into(), own)], &programs(), 0, &System::default());
        assert_eq!(walked, Walk::Agent("app:t1".into()));
        let _ = std::process::Command::new("kill").arg(pid.to_string()).status();
        let _ = tool.kill();
        let _ = tool.wait();
        // Detached from everything under a front's name: launchd adopts it, and its name makes it no front.
        let (mut shell, pid) = spawned("(exec -a Ghostty sleep 5 & echo $!)");
        let _ = shell.wait();
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert_eq!(walk(pid, &super::table(), &[], &programs(), 0, &System::default()), Walk::Detached);
        let _ = std::process::Command::new("kill").arg(pid.to_string()).status();
        // A program copied under herdr's name, detached, starting the request: not where herdr is installed.
        let dir = tempfile::tempdir().unwrap();
        let (pid, parent) = detached(dir.path(), "herdr", None);
        assert_eq!(System::default().exe(parent).and_then(|p| p.file_name().map(|n| n.to_owned())), Some("herdr".into()));
        assert_eq!(walk(pid, &super::table(), &[], &programs(), 0, &System::default()), Walk::Detached);
        let _ = std::process::Command::new("kill").arg(pid.to_string()).status();
    }

    /// Set on this test binary run again to walk from a pid, with the result written where WALK_OUT says.
    const WALK: &str = "COCKPIT_TEST_WALK";
    const WALK_OUT: &str = "COCKPIT_TEST_WALK_OUT";

    /// A walk from WALK's pid in a process of its own, whose environment (its PATH) the test chose.
    #[test]
    fn walker() {
        if let (Some(pid), Some(out)) = (std::env::var(WALK).ok().and_then(|p| p.parse().ok()), std::env::var_os(WALK_OUT)) {
            let walked = walk(pid, &super::table(), &[], &programs(), 0, &System::default());
            std::fs::write(out, format!("{walked:?}")).unwrap();
        }
    }

    /// This test binary copied into `dir` as `name` and run detached as a stand-in, `word` after its own name: the
    /// sleep it starts and the stand-in's pid. The system lets the copy run from anywhere.
    fn detached(dir: &Path, name: &str, word: Option<&str>) -> (u32, u32) {
        let copy = dir.join(name);
        std::fs::copy(std::env::current_exe().unwrap(), &copy).unwrap();
        let at = dir.join(format!("{name}.pid"));
        // libtest takes `word` as one more name to match exactly, which no test has.
        let script = format!("('{}' {} place::tests::stand_in --exact --test-threads 1 >/dev/null 2>&1 &)", copy.display(), word.unwrap_or(""));
        let ran = std::process::Command::new("bash").args(["-c", &script]).env(STAND_IN, &at).status().unwrap();
        assert!(ran.success());
        let started = std::time::Instant::now();
        let pid: u32 = loop {
            if let Some(pid) = std::fs::read_to_string(&at).ok().and_then(|t| t.trim().parse().ok()) {
                break pid;
            }
            assert!(started.elapsed() < std::time::Duration::from_secs(10), "the stand-in never started");
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        std::thread::sleep(std::time::Duration::from_millis(100));
        let parent = super::table().get(&pid).map(|(p, _)| *p).unwrap_or_default();
        (pid, parent)
    }

    #[test]
    fn a_copy_named_like_a_terminal_first_on_the_hub_s_path_is_no_front() {
        // A detached copy named `tmux` in a folder first on the PATH a hub started with HOME=<home>/../<user> would keep.
        let dir = tempfile::tempdir().unwrap();
        let (pid, parent) = detached(dir.path(), "tmux", None);
        let out = dir.path().join("walked");
        let path = format!("{}:/usr/bin:/bin", dir.path().display());
        let ran = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["place::tests::walker", "--exact", "--test-threads", "1"])
            .env("PATH", &path)
            .env(WALK, parent.to_string())
            .env(WALK_OUT, &out)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(ran.success());
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "Detached", "placed with PATH={path}");
        let _ = std::process::Command::new("kill").arg(pid.to_string()).status();
    }

    #[test]
    fn real_cockpit_focus_act_and_setup_runs_are_cockpit_s_own_work() {
        let dir = tempfile::tempdir().unwrap();
        for word in ["focus", "act", "setup-run"] {
            let folder = dir.path().join(word);
            std::fs::create_dir_all(&folder).unwrap();
            let (pid, parent) = detached(&folder, "cockpit", Some(word));
            let t = super::table();
            assert_eq!(walk(pid, &t, &[], &programs(), 0, &System::default()), Walk::Cockpit, "what cockpit {word} runs");
            assert_eq!(walk(parent, &t, &[], &programs(), 0, &System::default()), Walk::Cockpit, "cockpit {word} itself");
            let _ = std::process::Command::new("kill").arg(pid.to_string()).status();
        }
        // `cockpit show` is cockpit's work for what it runs, not for itself: the pane is the person's.
        let show = dir.path().join("show");
        std::fs::create_dir_all(&show).unwrap();
        let (pid, parent) = detached(&show, "cockpit", Some("show"));
        let t = super::table();
        assert_eq!(walk(pid, &t, &[], &programs(), 0, &System::default()), Walk::Cockpit);
        assert_eq!(walk(parent, &t, &[], &programs(), 0, &System::default()), Walk::Detached, "detached here, so no front either");
        let _ = std::process::Command::new("kill").arg(pid.to_string()).status();
    }
}
