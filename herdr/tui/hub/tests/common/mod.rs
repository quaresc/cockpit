//! A sandbox for the hub's tests: a fake HOME copied from tests/fixtures/box, a herdr folder with the text apps' Python
//! and logging stand-ins for its scripts, fake herdr, claude, open, pbcopy and gh that log every call to calls.log, and
//! the hub started on it.

#![allow(dead_code)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

pub fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// The plugin's herdr folder, where the Python originals live.
pub fn herdr_src() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The python3 interpreter itself, not a version manager's shim that would look for its version in the fake HOME.
pub fn python() -> Option<String> {
    static FOUND: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    FOUND
        .get_or_init(|| {
            let out = Command::new("python3").args(["-c", "import sys; print(sys.executable)"]).output().ok()?;
            let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
            (out.status.success() && !path.is_empty()).then_some(path)
        })
        .clone()
}

pub fn has_python() -> bool {
    python().is_some()
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
        copy_tree(&fixtures().join("box"), &root, &|t| t.replace("@BOX@", &box_).replace("@NOW_MS@", &now_ms));
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let python = python().unwrap_or_else(|| "python3".into());
        let shebang = |text: String| text.replacen("#!/usr/bin/env python3", &format!("#!{python}"), 1);
        executable(&bin.join("herdr"), &shebang(std::fs::read_to_string(fixtures().join("herdr.py")).unwrap()));
        let logcall = shebang(std::fs::read_to_string(fixtures().join("logcall.py")).unwrap());
        for tool in ["logcall", "claude", "open", "xdg-open", "pbcopy", "gh"] {
            executable(&bin.join(tool), &logcall);
        }
        let herdr_dir = root.join("herdr");
        std::fs::create_dir_all(&herdr_dir).unwrap();
        for entry in std::fs::read_dir(herdr_src()).unwrap().flatten() {
            if entry.path().extension().is_some_and(|x| x == "py") {
                std::fs::copy(entry.path(), herdr_dir.join(entry.file_name())).unwrap();
            }
        }
        let script = std::fs::read_to_string(fixtures().join("script.sh")).unwrap().replace("exec python3", &format!("exec {python}"));
        for name in ["ask-session.sh", "notify.sh", "meeting-brief.sh"] {
            executable(&herdr_dir.join(name), &script);
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

    /// The test's environment without what ties it to a herdr pane, a Claude session or cockpit settings, then the
    /// sandbox's own.
    pub fn env(&self) -> Vec<(String, String)> {
        let path = format!("{}:{}", self.root.join("bin").display(), std::env::var("PATH").unwrap_or_default());
        let inherited = std::env::vars().filter(|(k, _)| !["HERDR_", "COCKPIT_", "CLAUDE"].iter().any(|p| k.starts_with(p)));
        inherited
            .chain([
                ("HOME".into(), self.home().to_string_lossy().into_owned()),
                ("PATH".into(), path),
                ("HERDR_BIN_PATH".into(), self.root.join("bin/herdr").to_string_lossy().into_owned()),
                ("COCKPIT_TEST_DIR".into(), self.root.to_string_lossy().into_owned()),
                ("COCKPIT_HERDR_DIR".into(), self.root.join("herdr").to_string_lossy().into_owned()),
            ])
            .collect()
    }

    /// Fills @BOX@ in a JSON template with this sandbox's root.
    pub fn fill(&self, template: &str) -> Value {
        serde_json::from_str(&template.replace("@BOX@", &self.root.to_string_lossy())).unwrap()
    }

    /// Runs one action through the Python bridge.
    pub fn python(&self, app: &str, action: &str, args: &Value, armed: Option<&str>) -> Value {
        let mut child = Command::new(python().expect("python3"))
            .arg(self.root.join("herdr/bridge.py"))
            .current_dir(self.root.join("herdr"))
            .env_clear()
            .envs(self.env())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let request = json!({"module": app, "function": action, "args": args, "armed": armed});
        child.stdin.take().unwrap().write_all(request.to_string().as_bytes()).unwrap();
        let out = child.wait_with_output().unwrap();
        serde_json::from_slice(&out.stdout).unwrap_or_else(|_| panic!("bridge.py: {}", String::from_utf8_lossy(&out.stderr)))
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

pub struct Hub {
    child: Child,
    socket: PathBuf,
}

impl Hub {
    pub fn start(sandbox: &Sandbox, extra: &[(&str, &str)]) -> Hub {
        let child = Command::new(env!("CARGO_BIN_EXE_cockpit-hub"))
            .env_clear()
            .envs(sandbox.env())
            .envs(extra.iter().map(|(k, v)| (k.to_string(), v.to_string())))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let socket = sandbox.socket();
        let start = Instant::now();
        while UnixStream::connect(&socket).is_err() {
            assert!(start.elapsed() < Duration::from_secs(5), "the hub never listened on {}", socket.display());
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

pub fn request_on(socket: &Path, request: &Value) -> Value {
    let mut stream = UnixStream::connect(socket).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
    writeln!(stream, "{request}").unwrap();
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}
