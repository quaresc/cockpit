//! A sandbox for the hub's tests: a fake HOME copied from tests/fixtures/box, a herdr folder whose scripts only log
//! their calls, fake herdr, claude, open, pbcopy, gh and notification receiver that log every call to calls.log, and the
//! hub started on it.

#![allow(dead_code)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

pub fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

pub struct Sandbox {
    _dir: tempfile::TempDir,
    pub root: PathBuf,
}

fn copy_tree(from: &Path, to: &Path, fill: &dyn Fn(&str) -> String) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &target, fill);
        } else {
            match std::fs::read_to_string(entry.path()) {
                Ok(text) => std::fs::write(&target, fill(&text)).unwrap(),
                Err(_) => {
                    std::fs::copy(entry.path(), &target).unwrap();
                }
            }
        }
    }
}

fn executable(path: &Path, content: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, content).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn git(dir: &Path, args: &[&str]) {
    let ok =
        Command::new("git").args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "init.defaultBranch=main"]).args(args).current_dir(dir).output().unwrap();
    assert!(ok.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&ok.stderr));
}

impl Sandbox {
    /// The fixture box with these herdr answers (command prefix → result, or {"fail": message}) and config changes.
    pub fn new(herdr: &Value, config: &Value) -> Self {
        let dir = tempfile::Builder::new().prefix("hub").tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis().to_string();
        let box_ = root.to_string_lossy().into_owned();
        copy_tree(&fixtures().join("box"), &root, &|t| t.replace("@BOX@", &box_).replace("@NOW_MS@", &now_ms).replace("@VERSION@", env!("CARGO_PKG_VERSION")));
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        executable(&bin.join("herdr"), &std::fs::read_to_string(fixtures().join("herdr.sh")).unwrap());
        let logcall = std::fs::read_to_string(fixtures().join("logcall.sh")).unwrap();
        for tool in ["claude", "open", "xdg-open", "pbcopy", "gh", "notify"] {
            executable(&bin.join(tool), &logcall);
        }
        let herdr_dir = root.join("herdr");
        std::fs::create_dir_all(&herdr_dir).unwrap();
        for name in ["ask-session.sh", "meeting-brief.sh"] {
            executable(&herdr_dir.join(name), &logcall);
        }
        let api = root.join("repos/api");
        std::fs::create_dir_all(&api).unwrap();
        git(&api, &["init", "-q"]);
        git(&api, &["commit", "-q", "--allow-empty", "-m", "start"]);
        git(&api, &["checkout", "-q", "-b", "me/eng-401-login"]);
        std::fs::write(root.join("herdr.json"), herdr.to_string()).unwrap();
        let config_path = root.join("home/.config/cockpit/config.json");
        let mut current: Value = serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
        for (k, v) in config.as_object().into_iter().flatten() {
            current[k] = v.clone();
        }
        std::fs::write(&config_path, current.to_string()).unwrap();
        std::fs::write(root.join("calls.log"), "").unwrap();
        Sandbox { _dir: dir, root }
    }

    pub fn home(&self) -> PathBuf {
        self.root.join("home")
    }

    pub fn socket(&self) -> PathBuf {
        self.home().join(".cache/cockpit/hub.sock")
    }

    /// The test's environment without what ties it to a herdr pane, a Claude session, cockpit settings or the terminal
    /// app, then the sandbox's own.
    pub fn env(&self) -> Vec<(String, String)> {
        let path = format!("{}:{}", self.root.join("bin").display(), std::env::var("PATH").unwrap_or_default());
        let inherited = std::env::vars().filter(|(k, _)| !["HERDR_", "HC_", "COCKPIT_", "CLAUDE", "__CFBundleIdentifier"].iter().any(|p| k.starts_with(p)));
        inherited
            .chain([
                ("HOME".into(), self.home().to_string_lossy().into_owned()),
                ("PATH".into(), path),
                ("HERDR_BIN_PATH".into(), self.root.join("bin/herdr").to_string_lossy().into_owned()),
                ("COCKPIT_TEST_DIR".into(), self.root.to_string_lossy().into_owned()),
                ("COCKPIT_HERDR_DIR".into(), self.root.join("herdr").to_string_lossy().into_owned()),
                // Notifications reach the fake receiver instead of the screen.
                ("COCKPIT_NOTIFY_PROGRAM".into(), self.root.join("bin/notify").to_string_lossy().into_owned()),
                // The feed reaches GitHub on its own: only the feed's tests turn it on, against a fake GitHub.
                ("COCKPIT_HUB_FEED".into(), "off".into()),
                // Nor does it ask GitHub for a newer cockpit.
                ("COCKPIT_PUBLISHED_URL".into(), "off".into()),
                // Nor does the session index download its model.
                ("COCKPIT_EMBED".into(), "off".into()),
                // Nor does the usage thread write the sidebar's `$limits`, which its first pass clears.
                ("COCKPIT_HUB_LIMITS".into(), "off".into()),
                // Nor does it ask the agents installed on this machine which models they list.
                ("COCKPIT_HUB_MODELS".into(), "off".into()),
            ])
            .collect()
    }

    /// Fills @BOX@ in a JSON template with this sandbox's root.
    pub fn fill(&self, template: &str) -> Value {
        serde_json::from_str(&template.replace("@BOX@", &self.root.to_string_lossy())).unwrap()
    }

    /// The calls logged so far, once no new one has come for half a second (the actions' background processes).
    pub fn calls(&self) -> Vec<Value> {
        let log = self.root.join("calls.log");
        let (start, mut last, mut since) = (Instant::now(), 0, Instant::now());
        while start.elapsed() < Duration::from_secs(8) {
            let size = std::fs::metadata(&log).map(|m| m.len()).unwrap_or(0);
            if size != last {
                (last, since) = (size, Instant::now());
            } else if since.elapsed() > Duration::from_millis(500) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        std::fs::read_to_string(&log).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
    }
}

/// Set on the test binary run again as a hub.
const HUB_PROCESS: &str = "COCKPIT_TEST_HUB_PROCESS";

/// The hub as a process of its own: this test binary run again on `hub_process` alone, on the sandbox's environment.
pub fn hub_command(sandbox: &Sandbox) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args(["common::hub_process", "--exact", "--test-threads", "1"]).env_clear().envs(sandbox.env()).env(HUB_PROCESS, "1");
    command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    command
}

/// Set on a test's hub to place every sender in that agent's tree (a handle), else every sender is the person.
pub const SENT_BY: &str = "COCKPIT_TEST_SENT_BY";

/// Set on a test's hub to place every sender by its real line of parents, as the person's hub does (no agent known).
pub const PLACE_REAL: &str = "COCKPIT_TEST_PLACE_REAL";

/// Where a real process stands, the walk's verdict in the words a refusal carries.
fn placed(pid: u32) -> cockpit_core::peer::Who {
    use cockpit_agents::place::{System, Walk, table, walk};
    use cockpit_core::peer::Who;
    match walk(pid, &table(), &[], &[], std::process::id(), &System::default()) {
        Walk::Front => Who::Person,
        Walk::Agent(handle) => Who::Agent(handle),
        Walk::Unwatched(_) => Who::Agent(String::new()),
        Walk::Cockpit => Who::Unplaced("cockpit's own work".into()),
        other => Who::Unplaced(format!("placed {other:?}")),
    }
}

/// The hub, when this binary runs as one; nothing in the test run itself.
#[test]
fn hub_process() {
    if std::env::var_os(HUB_PROCESS).is_some() {
        if std::env::var_os(PLACE_REAL).is_some() {
            cockpit_core::peer::on_place(placed);
        }
        cockpit_core::peer::on_place(|_| match std::env::var(SENT_BY) {
            Ok(handle) => cockpit_core::peer::Who::Agent(handle),
            Err(_) => cockpit_core::peer::Who::Person,
        });
        cockpit_hub::run();
    }
}

/// Set on this binary run again as a client: the request it sends, the socket, and where it writes the answer.
pub const REQUEST: &str = "COCKPIT_TEST_REQUEST";
pub const REQUEST_SOCKET: &str = "COCKPIT_TEST_REQUEST_SOCKET";
pub const REQUEST_REPLY: &str = "COCKPIT_TEST_REQUEST_REPLY";

/// A client of the hub, when this binary runs as one: it sends REQUEST and writes the answer where REQUEST_REPLY says.
#[test]
fn requester() {
    if let (Ok(request), Some(socket), Some(reply)) = (std::env::var(REQUEST), std::env::var_os(REQUEST_SOCKET), std::env::var_os(REQUEST_REPLY)) {
        let answer = request_on(Path::new(&socket), &serde_json::from_str(&request).unwrap());
        std::fs::write(reply, answer.to_string()).unwrap();
    }
}

/// Set on this binary run again as a program that only waits: how many seconds it waits.
const WAIT: &str = "COCKPIT_TEST_WAIT";

/// A program that only waits, when this binary runs as one.
#[test]
fn waiter() {
    if let Some(secs) = std::env::var(WAIT).ok().and_then(|s| s.parse().ok()) {
        std::thread::sleep(Duration::from_secs(secs));
    }
}

/// This test binary linked (else copied) into `dir` as `name`, waiting `secs` seconds in `cwd`: a program of that name
/// in that folder. A copy of a system program may not run under another name, as a multi-call coreutils refuses to.
pub fn waiting(dir: &Path, name: &str, cwd: &Path, secs: u32) -> Child {
    let program = dir.join(name);
    if !program.exists() {
        let exe = std::env::current_exe().unwrap();
        if std::fs::hard_link(&exe, &program).is_err() {
            std::fs::copy(&exe, &program).unwrap();
        }
    }
    Command::new(&program)
        .args(["common::waiter", "--exact", "--test-threads", "1"])
        .env(WAIT, secs.to_string())
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

/// `request` sent by this test binary copied into `dir` as `cockpit` and run as `cockpit <word> …`: the hub's answer.
pub fn sent_as_cockpit(dir: &Path, word: &str, socket: &Path, request: &Value) -> Value {
    let copy = dir.join("cockpit");
    if !copy.exists() {
        std::fs::copy(std::env::current_exe().unwrap(), &copy).unwrap();
    }
    let reply = dir.join(format!("{word}.reply"));
    // libtest takes `word` as one more name to match exactly, which no test has.
    let ran = Command::new(&copy)
        .args([word, "common::requester", "--exact", "--test-threads", "1"])
        .env(REQUEST, request.to_string())
        .env(REQUEST_SOCKET, socket)
        .env(REQUEST_REPLY, &reply)
        .stdout(Stdio::null())
        .status()
        .unwrap();
    assert!(ran.success(), "cockpit {word} did not run");
    serde_json::from_str(&std::fs::read_to_string(&reply).unwrap()).unwrap()
}

pub struct Hub {
    child: Child,
    socket: PathBuf,
}

impl Hub {
    pub fn start(sandbox: &Sandbox, extra: &[(&str, &str)]) -> Hub {
        let child = hub_command(sandbox).envs(extra.iter().map(|(k, v)| (k.to_string(), v.to_string()))).spawn().unwrap();
        let socket = sandbox.socket();
        let start = Instant::now();
        // Ready once it answers a ping: a stale socket file, or a hub still opening its store, takes connections it does not answer yet.
        while !pings(&socket) {
            assert!(start.elapsed() < Duration::from_secs(10), "the hub never answered on {}", socket.display());
            std::thread::sleep(Duration::from_millis(20));
        }
        Hub { child, socket }
    }

    pub fn request(&self, request: &Value) -> Value {
        request_on(&self.socket, request)
    }

    pub fn is_running(&mut self) -> bool {
        self.child.try_wait().unwrap().is_none()
    }
}

impl Drop for Hub {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn pings(socket: &Path) -> bool {
    let Ok(mut stream) = UnixStream::connect(socket) else { return false };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    if writeln!(stream, r#"{{"op": "ping"}}"#).is_err() {
        return false;
    }
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).is_ok() && serde_json::from_str::<Value>(&line).is_ok_and(|v| v["ok"] == true)
}

pub fn request_on(socket: &Path, request: &Value) -> Value {
    let mut stream = UnixStream::connect(socket).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
    writeln!(stream, "{request}").unwrap();
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}
