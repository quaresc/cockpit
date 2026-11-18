//! A session cockpit drives across `cockpit hub stop` and the next hub, on the path the window takes: Start work makes it
//! from a prompt, the registry names it, the hub ends on its signal, and the hub started after brings it back paused.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// Set on this test binary run again as `cockpit hub`.
const HUB: &str = "COCKPIT_TEST_HUB_PROCESS";
/// Set on this test binary run again as `cockpit hub stop`.
const STOP: &str = "COCKPIT_TEST_HUB_STOP";

/// `cockpit hub` as shipped, when this binary runs as the hub: only who sends a request is told, every sender the person.
#[test]
fn hub_process() {
    if std::env::var_os(HUB).is_some() {
        cockpit_core::peer::on_place(|_| cockpit_core::peer::Who::Person);
        crate::gui::hub();
    }
}

/// `cockpit hub stop` as shipped, when this binary runs as it.
#[test]
fn hub_stopper() {
    if std::env::var_os(STOP).is_some() {
        crate::gui::hub_stop();
    }
}

struct Sandbox {
    _dir: tempfile::TempDir,
    root: PathBuf,
    demo: bool,
}

fn executable(path: &Path, content: &str) {
    std::fs::write(path, content).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn git(dir: &Path, args: &[&str]) {
    let out =
        Command::new("git").args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "init.defaultBranch=main"]).args(args).current_dir(dir).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

impl Sandbox {
    /// A fake HOME with the acme `api` repo (its origin a bare clone), herdr, the Herdr app and hc that answer nothing,
    /// and the stub agent as Claude; `demo` adds the demo world, with a made-up driven session of its own.
    fn new(demo: bool) -> Sandbox {
        let dir = tempfile::Builder::new().prefix("hub-restart").tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let home = root.join("home");
        for sub in [".cache/cockpit", ".config/cockpit", "code/api"] {
            std::fs::create_dir_all(home.join(sub)).unwrap();
        }
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        for tool in ["herdr", "herdr-app", "hc", "gh", "open"] {
            executable(&bin.join(tool), "#!/bin/sh\nexit 1\n");
        }
        executable(&bin.join("notify"), "#!/bin/sh\nexit 0\n");
        let api = home.join("code/api");
        git(&api, &["init", "-q"]);
        std::fs::write(api.join("README.md"), "# acme/api\n").unwrap();
        git(&api, &["add", "README.md"]);
        git(&api, &["commit", "-q", "-m", "API: a readme"]);
        git(&root, &["clone", "-q", "--bare", "home/code/api", "api-origin.git"]);
        git(&api, &["remote", "add", "origin", &root.join("api-origin.git").to_string_lossy()]);
        git(&api, &["fetch", "-q", "origin"]);
        let config = json!({"reposDir": home.join("code").to_string_lossy()});
        std::fs::write(home.join(".config/cockpit/config.json"), config.to_string()).unwrap();
        let world = json!({"agents": [], "driven": [{"key": "claude:demo-421", "agent": "claude", "session": "demo-421", "cwd": "/Users/me/code/api",
                                                     "title": "Make the daily quota configurable", "messages": [{"role": "user", "min": 3, "text": "Make the daily quota configurable"}]}]});
        std::fs::write(root.join("world.json"), world.to_string()).unwrap();
        Sandbox { _dir: dir, root, demo }
    }

    fn home(&self) -> PathBuf {
        self.root.join("home")
    }

    fn cache(&self) -> PathBuf {
        self.home().join(".cache/cockpit")
    }

    /// Where the hub listens, as `cockpit_core::hub::socket` names it for this home.
    fn socket(&self) -> PathBuf {
        use std::hash::{Hash, Hasher};
        let path = self.cache().join("hub.sock");
        if path.as_os_str().len() < 100 {
            return path;
        }
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.cache().hash(&mut hasher);
        std::env::temp_dir().join(format!("cockpit-hub-{:016x}.sock", hasher.finish()))
    }

    /// The test's environment without what ties it to a herdr pane, an agent or cockpit settings, then the sandbox's own.
    fn env(&self) -> Vec<(String, String)> {
        let at = |p: &str| self.root.join(p).to_string_lossy().into_owned();
        let stub = Path::new(env!("CARGO_MANIFEST_DIR")).join("../agents/tests/fixtures/stub-claude");
        let path = format!("{}:{}", at("bin"), std::env::var("PATH").unwrap_or_default());
        let inherited = std::env::vars().filter(|(k, _)| !["HERDR_", "HC_", "COCKPIT_", "CLAUDE"].iter().any(|p| k.starts_with(p)));
        let mut env: Vec<(String, String)> = inherited
            .chain([
                ("HOME".into(), self.home().to_string_lossy().into_owned()),
                ("PATH".into(), path),
                ("HERDR_BIN_PATH".into(), at("bin/herdr")),
                ("HERDR_APP_BIN".into(), at("bin/herdr-app")),
                ("HERDR_APP_HOME".into(), at("herdr-home")),
                ("COCKPIT_HC_BIN".into(), at("bin/hc")),
                ("COCKPIT_FRONTEND".into(), "app".into()),
                ("COCKPIT_NOTIFY_PROGRAM".into(), at("bin/notify")),
                ("COCKPIT_HUB_FEED".into(), "off".into()),
                ("COCKPIT_PUBLISHED_URL".into(), "off".into()),
                ("COCKPIT_EMBED".into(), "off".into()),
                ("COCKPIT_HUB_LIMITS".into(), "off".into()),
                ("COCKPIT_HUB_MODELS".into(), "off".into()),
                ("COCKPIT_CLAUDE".into(), stub.to_string_lossy().into_owned()),
            ])
            .collect();
        if self.demo {
            env.push(("COCKPIT_AGENTS_DEMO".into(), at("world.json")));
        }
        env
    }

    /// This test binary run again on the sandbox's environment, as `role` says.
    fn this(&self, test: &str, role: &str) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.args([test, "--exact", "--test-threads", "1"]).env_clear().envs(self.env()).env(role, "1");
        command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        command
    }

    /// A hub, as the window's Start it brings one up: its command line ends as `cockpit hub`'s does, which is how
    /// `cockpit hub stop` tells a hub (libtest takes the two words as names to match exactly, which no test has).
    fn hub(&self) -> Hub {
        let _ = std::fs::remove_file(self.cache().join("hub.stopped"));
        let child = self.this("hub_restart::hub_process", HUB).args(["cockpit", "hub"]).spawn().unwrap();
        let started = Instant::now();
        while UnixStream::connect(self.socket()).is_err() {
            assert!(started.elapsed() < Duration::from_secs(10), "the hub never listened");
            std::thread::sleep(Duration::from_millis(20));
        }
        Hub { child, socket: self.socket() }
    }

    /// `cockpit hub stop`: the stopped mark, then SIGTERM to every hub of this home, waited out.
    fn stop_hub(&self) {
        let ran = self.this("hub_restart::hub_stopper", STOP).status().unwrap();
        assert!(ran.success(), "cockpit hub stop did not run");
    }
}

struct Hub {
    child: Child,
    socket: PathBuf,
}

impl Hub {
    fn request(&self, request: &Value) -> Value {
        let mut stream = UnixStream::connect(&self.socket).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
        writeln!(stream, "{request}").unwrap();
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }

    fn act(&self, app: &str, action: &str, args: Value) -> Value {
        self.request(&json!({"op": "act", "app": app, "action": action, "args": [args]}))
    }

    fn view(&self, name: &str, args: Value) -> Value {
        self.request(&json!({"op": "view", "name": name, "args": args}))["value"].clone()
    }

    /// The view `name` once `done` says so, within 20 s.
    fn wait(&self, name: &str, args: Value, what: &str, done: impl Fn(&Value) -> bool) -> Value {
        let started = Instant::now();
        loop {
            let now = self.view(name, args.clone());
            if done(&now) {
                return now;
            }
            assert!(started.elapsed() < Duration::from_secs(20), "waited too long for {what}: {now:#}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Whether its process ended, within 5 s.
    fn ended(&mut self) -> bool {
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(5) {
            if self.child.try_wait().unwrap().is_some() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }
}

impl Drop for Hub {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn said(session: &Value) -> Vec<String> {
    session["messages"].as_array().into_iter().flatten().filter_map(|m| m["text"].as_str().filter(|t| !t.is_empty()).map(str::to_string)).collect()
}

/// The processes running the stub agent on session `id`.
fn agents_of(id: &str) -> usize {
    let out = Command::new("/bin/ps").args(["-axo", "command="]).output().unwrap();
    String::from_utf8_lossy(&out.stdout).lines().filter(|l| l.contains("stub-claude") && l.contains(id)).count()
}

fn sessions(registry: &Value) -> Vec<String> {
    registry["sessions"].as_array().into_iter().flatten().filter_map(|e| e["key"].as_str().map(str::to_string)).collect()
}

/// Start work from a prompt, driven, as Cockpit Chat's empty box asks for it; `cockpit hub stop`; a new hub; a send.
fn survives_a_stop_and_start(demo: bool) {
    if !Command::new("/usr/bin/python3").arg("--version").output().is_ok_and(|o| o.status.success()) {
        eprintln!("no /usr/bin/python3: the stub agent cannot run here");
        return;
    }
    let sandbox = Sandbox::new(demo);
    let mut hub = sandbox.hub();
    let request = json!({"id": "g1", "text": "say hello", "repo": "api", "worktree": "new", "driven": true, "agent": "", "mode": "", "model": "",
                         "base": "", "branch": ""});
    let answer = hub.act("start", "work", request);
    assert!(answer["error"].is_null(), "{answer}");
    let run = hub.wait("start", json!({"id": "g1"}), "the start's session", |v| v["run"]["state"] != "running" && !v["run"].is_null())["run"].clone();
    assert_eq!(run["state"], "done", "{run:#}");
    let key = run["made"]["pane"].as_str().expect("the start names the session it made").to_string();
    let first =
        hub.wait("session", json!({"key": key}), "the first turn", |v| v["session"]["working"] == false && said(&v["session"]).len() == 2)["session"].clone();
    let id = first["session"].as_str().unwrap().to_string();

    let registry = sandbox.cache().join("driven/registry.json");
    let written = cockpit_core::files::load(&registry).expect("the registry is written once the session starts");
    assert_eq!(sessions(&written), [key.as_str()], "it names the session Start work made, and no session of the demo world");
    assert_eq!(written["sessions"][0]["session_id"], id.as_str());

    // The signal path: `cockpit hub stop` ends the hub, which leaves the registry behind.
    sandbox.stop_hub();
    assert!(hub.ended(), "cockpit hub stop ends the hub");
    assert!(sandbox.cache().join("hub.stopped").exists(), "and says so to the windows");
    assert_eq!(sessions(&cockpit_core::files::load(&registry).expect("the registry outlives the hub")), [key.as_str()]);
    drop(hub);

    // Start it: the next hub brings the session back paused, its history read from its transcript.
    let hub = sandbox.hub();
    let back = hub.wait("session", json!({"key": key}), "the session back", |v| v["session"]["key"] == key.as_str())["session"].clone();
    assert_eq!((back["paused"].clone(), back["working"].clone(), back["session"].clone()), (json!(true), json!(false), json!(id)), "{back:#}");
    assert_eq!(said(&back), said(&first), "its history is intact");
    let agents = hub.view("agents", json!({}));
    let row = agents["agents"].as_array().into_iter().flatten().find(|a| a["terminal"] == key.as_str()).cloned();
    assert!(row.as_ref().is_some_and(|r| r["driven"] == true), "the agents view lists it again: {agents:#}");
    if demo {
        assert!(hub.view("session", json!({"key": "claude:demo-421"}))["session"].is_object(), "the demo world's own session is there as before");
    }

    // The next send picks it up under the same id, in one process.
    let sent = hub.act("session", "send", json!({"key": key, "text": "And after midnight?"}));
    assert!(sent["error"].is_null(), "{sent}");
    let resumed =
        hub.wait("session", json!({"key": key}), "the resumed turn", |v| v["session"]["working"] == false && said(&v["session"]).len() == 4)["session"].clone();
    assert_eq!((resumed["paused"].clone(), resumed["session"].clone()), (json!(false), json!(id)));
    assert_eq!(agents_of(&id), 1, "one agent runs the session");

    let ended = hub.act("session", "stop", json!({"key": key, "confirm": true}));
    assert!(ended["error"].is_null(), "{ended}");
    let started = Instant::now();
    while agents_of(&id) > 0 && started.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(agents_of(&id), 0, "ended, no process is left");
}

#[test]
fn a_driven_start_comes_back_paused_after_cockpit_hub_stop_and_a_new_hub() {
    survives_a_stop_and_start(false);
}

#[test]
fn a_driven_start_in_the_demo_world_comes_back_paused_after_cockpit_hub_stop_and_a_new_hub() {
    survives_a_stop_and_start(true);
}
