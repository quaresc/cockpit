//! A sandbox for the CLI's and the scripts' tests: a fake HOME, a herdr folder holding copies of the plugin's scripts,
//! and fake herdr, terminal-browser, open and a notification receiver that log each call, one line of arguments each,
//! to calls.log.

#![allow(dead_code)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

pub struct Sandbox {
    _dir: tempfile::TempDir,
    pub root: PathBuf,
}

/// The plugin's herdr folder, where the scripts live.
pub fn scripts() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn executable(path: &Path, content: &str) {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(path, content).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A fake command answering by the first matching prefix of its arguments (`*` for any), each call logged as
/// `<name> <args>`; COCKPIT_TEST_ENV names variables logged with it.
pub fn fake(name: &str, log: &Path, answers: &[(&str, &str)]) -> String {
    let mut script = format!("#!/bin/sh\nprintf '%s\\n' \"{name} $*\" >> '{}'\ncase \"$*\" in\n", log.display());
    for (prefix, answer) in answers {
        let pattern = if *prefix == "*" { "*".to_string() } else { format!("\"{prefix}\"*") };
        script += &format!("  {pattern})\n{answer}\n    ;;\n");
    }
    script + "esac\n"
}

/// `echo` of a JSON answer, quoted for sh.
pub fn says(json: &str) -> String {
    format!("    printf '%s\\n' '{}'", json.replace('\'', "'\\''"))
}

impl Sandbox {
    pub fn new() -> Self {
        let dir = tempfile::Builder::new().prefix("cockpit-cli").tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        for sub in ["home/.cache/cockpit", "home/.config/cockpit", "bin", "herdr"] {
            std::fs::create_dir_all(root.join(sub)).unwrap();
        }
        std::fs::write(root.join("calls.log"), "").unwrap();
        let sandbox = Sandbox { _dir: dir, root };
        let log = sandbox.log();
        for tool in ["open", "xdg-open"] {
            executable(&sandbox.root.join("bin").join(tool), &fake(tool, &log, &[]));
        }
        executable(&sandbox.root.join("bin/notify"), &fake("notify", &log, &[]));
        sandbox.herdr(&[]);
        sandbox
    }

    pub fn log(&self) -> PathBuf {
        self.root.join("calls.log")
    }

    pub fn home(&self) -> PathBuf {
        self.root.join("home")
    }

    pub fn cache(&self) -> PathBuf {
        self.home().join(".cache/cockpit")
    }

    /// herdr answering these prefixes, `{"result": {}}` to anything else.
    pub fn herdr(&self, answers: &[(&str, &str)]) {
        let mut all: Vec<(&str, String)> = answers.iter().map(|(p, a)| (*p, says(a))).collect();
        all.push(("*", says(r#"{"result": {}}"#)));
        let refs: Vec<(&str, &str)> = all.iter().map(|(p, a)| (*p, a.as_str())).collect();
        executable(&self.root.join("bin/herdr"), &fake("herdr", &self.log(), &refs));
    }

    /// terminal-browser answering these prefixes with raw sh, each call logged with the pane it was asked from.
    pub fn terminal_browser(&self, answers: &[(&str, &str)]) {
        let script = fake("tb", &self.log(), answers).replacen("\"tb $*\"", "\"tb [$HERDR_PANE_ID] $*\"", 1);
        executable(&self.root.join("bin/terminal-browser"), &script);
    }

    pub fn write(&self, rel: &str, content: &str) {
        let path = self.root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    /// The test's environment without what ties it to a herdr pane, cockpit settings or the terminal app, then the
    /// sandbox's own.
    pub fn env(&self) -> Vec<(String, String)> {
        let inherited =
            std::env::vars().filter(|(k, _)| !["HERDR_", "COCKPIT_", "CLAUDE", "PATH", "HOME", "__CFBundleIdentifier"].iter().any(|p| k.starts_with(p)));
        // jq's own folder too, which the scripts need, without the rest of the folders where real herdr lives.
        let jq = std::env::var("PATH").unwrap_or_default().split(':').find(|d| Path::new(d).join("jq").is_file()).unwrap_or("/usr/bin").to_string();
        let path = format!("{}:/usr/bin:/bin:/usr/sbin:/sbin:{jq}", self.root.join("bin").display());
        inherited
            .chain([
                ("HOME".into(), self.home().to_string_lossy().into_owned()),
                ("PATH".into(), path),
                ("HERDR_BIN_PATH".into(), self.root.join("bin/herdr").to_string_lossy().into_owned()),
                ("COCKPIT_HERDR_DIR".into(), self.root.join("herdr").to_string_lossy().into_owned()),
                ("COCKPIT_TERMINAL_BROWSER".into(), self.root.join("bin/terminal-browser").to_string_lossy().into_owned()),
                // Notifications reach the fake receiver instead of the screen.
                ("COCKPIT_NOTIFY_PROGRAM".into(), self.root.join("bin/notify").to_string_lossy().into_owned()),
                // A hub started here indexes sessions without downloading the search model.
                ("COCKPIT_EMBED".into(), "off".into()),
            ])
            .collect()
    }

    /// Runs the cockpit binary with these arguments and extra variables.
    pub fn cockpit(&self, args: &[&str], extra: &[(&str, &str)]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_cockpit"))
            .args(args)
            .env_clear()
            .envs(self.env())
            .envs(extra.iter().map(|(k, v)| (k.to_string(), v.to_string())))
            .current_dir(&self.root)
            .output()
            .unwrap()
    }

    /// Runs a script of the sandbox's herdr folder with sh.
    pub fn sh(&self, script: &str, args: &[&str], extra: &[(&str, &str)]) -> Output {
        Command::new("sh")
            .arg(self.root.join("herdr").join(script))
            .args(args)
            .env_clear()
            .envs(self.env())
            .envs(extra.iter().map(|(k, v)| (k.to_string(), v.to_string())))
            .output()
            .unwrap()
    }

    /// Copies one of the plugin's scripts into the sandbox's herdr folder.
    pub fn copy_script(&self, name: &str) {
        std::fs::copy(scripts().join(name), self.root.join("herdr").join(name)).unwrap();
    }

    pub fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.log()).unwrap().lines().map(str::to_string).collect()
    }
}

pub fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

pub fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}
