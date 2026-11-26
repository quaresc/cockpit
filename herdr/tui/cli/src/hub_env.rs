//! The hub's environment: the person's own hub runs on what their login shell sets, never on what the process that
//! happened to start it carried (a PATH, a herdr or hc binary, a demo world, a test's stand-ins).

use std::collections::BTreeMap;
use std::ffi::{CStr, OsString};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// What the starter may hand on: where its ssh agent and temp folder are, its language, and the desktop session it runs
/// in (the display, its auth file, the session bus): a bare login shell has none of them, and without them a browser the
/// hub opens on Linux cannot reach the screen.
fn from_starter(key: &str) -> bool {
    matches!(
        key,
        "SSH_AUTH_SOCK"
            | "TMPDIR"
            | "LANG"
            | "LC_ALL"
            | "DISPLAY"
            | "XAUTHORITY"
            | "WAYLAND_DISPLAY"
            | "XDG_RUNTIME_DIR"
            | "DBUS_SESSION_BUS_ADDRESS"
            | "XDG_CURRENT_DESKTOP"
            | "XDG_SESSION_TYPE"
    ) || key.starts_with("LC_")
}

/// What the hub never takes, even from the login shell: a terminal's or a window's identity, the binaries and sockets
/// cockpit's tests and sandbox stand in, a session's own markers.
fn never(key: &str) -> bool {
    const STAND_INS: [&str; 12] = [
        "COCKPIT_AGENTS_DEMO",
        "COCKPIT_HC_BIN",
        "COCKPIT_NOTIFY_PROGRAM",
        "COCKPIT_HUB_FEED",
        "COCKPIT_HERDR_DIR",
        "COCKPIT_FRONTEND",
        "COCKPIT_SHOT_DIR",
        "COCKPIT_EMBED",
        "COCKPIT_TOASTS",
        "COCKPIT_SPACE",
        "COCKPIT_CLAUDE",
        "COCKPIT_HUB_IDLE_SECS",
    ];
    key.starts_with("HC_")
        || key.starts_with("HERDR_")
        || key.starts_with("COCKPIT_TEST")
        || STAND_INS.contains(&key)
        || key.starts_with("CLAUDE_CODE_")
        || matches!(key, "CLAUDECODE" | "PWD" | "OLDPWD" | "SHLVL" | "_" | "TERM_SESSION_ID" | "TMUX" | "TMUX_PANE")
}

/// The PATH when the login shell gives none: Homebrew's, the user's and the system's folders.
fn default_path(home: &Path) -> String {
    format!("/opt/homebrew/bin:/usr/local/bin:{}/.local/bin:/usr/bin:/bin:/usr/sbin:/sbin", home.display())
}

/// The account's home, shell and name, from the user database rather than the environment.
fn account() -> Option<(PathBuf, PathBuf, String)> {
    // SAFETY: getpwuid returns null or a record in static storage; its strings are copied out at once, before anything
    // else could call it (the hub has no other thread yet).
    unsafe {
        let pw = libc::getpwuid(libc::getuid());
        if pw.is_null() || (*pw).pw_dir.is_null() || (*pw).pw_shell.is_null() || (*pw).pw_name.is_null() {
            return None;
        }
        let text = |p: *const libc::c_char| CStr::from_ptr(p).to_string_lossy().into_owned();
        Some((PathBuf::from(text((*pw).pw_dir)), PathBuf::from(text((*pw).pw_shell)), text((*pw).pw_name)))
    }
}

const MARK: &str = "--cockpit-env--";

/// What the login shell sets, run on nothing but the account's own names: five seconds at most, else nothing.
fn login(shell: &Path, home: &Path, user: &str) -> Vec<(String, String)> {
    let script = format!("printf '\\n%s\\n' '{MARK}'; /usr/bin/env -0");
    let child = Command::new(shell)
        .args(["-l", "-c", &script])
        .env_clear()
        .env("HOME", home)
        .env("USER", user)
        .env("LOGNAME", user)
        .env("SHELL", shell)
        .env("TERM", "dumb")
        .env("PATH", default_path(home))
        .current_dir(home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else { return Vec::new() };
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < Duration::from_secs(5) => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Vec::new();
            }
        }
    }
    let mut out = Vec::new();
    let _ = child.stdout.take().map(|mut o| o.read_to_end(&mut out));
    parse(&String::from_utf8_lossy(&out))
}

/// `env -0`'s output after the mark: what a profile printed before it is left out.
fn parse(out: &str) -> Vec<(String, String)> {
    let Some((_, listed)) = out.split_once(&format!("\n{MARK}\n")) else { return Vec::new() };
    listed.split('\0').filter_map(|entry| entry.split_once('=')).map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

/// The environment the person's hub runs on: the login shell's, less what it never takes, the starter's ssh agent,
/// temp folder and language, and the account's own home, shell and name.
fn settled(starter: &[(String, String)], login: &[(String, String)], home: &Path, shell: &Path, user: &str) -> BTreeMap<String, String> {
    let mut env: BTreeMap<String, String> = starter.iter().filter(|(k, _)| from_starter(k)).cloned().collect();
    env.extend(login.iter().filter(|(k, _)| !never(k)).cloned());
    env.insert("HOME".into(), home.to_string_lossy().into_owned());
    env.insert("SHELL".into(), shell.to_string_lossy().into_owned());
    env.insert("USER".into(), user.into());
    env.insert("LOGNAME".into(), user.into());
    env.entry("PATH".into()).or_insert_with(|| default_path(home));
    env
}

/// Sets the hub's environment, when this is the person's own hub (its cache is the account's, however HOME spells it):
/// one started under another HOME, the sandbox's or a test's, serves only its starter and keeps what it was given.
/// Called before any thread starts.
pub fn settle() {
    let Some((home, shell, user)) = account() else { return };
    let starter: Vec<(String, String)> = std::env::vars_os().filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?))).collect();
    let Some(env) = plan(&starter, &home, &shell, &user, &|| login(&shell, &home, &user)) else { return };
    let keys: Vec<OsString> = std::env::vars_os().map(|(k, _)| k).collect();
    for key in keys {
        // SAFETY: the hub calls this first thing, before it starts any thread, so nothing reads the environment meanwhile.
        unsafe { std::env::remove_var(key) };
    }
    for (key, value) in env {
        // SAFETY: as above, no other thread runs yet.
        unsafe { std::env::set_var(key, value) };
    }
}

/// The environment the hub settles on, or None when its HOME's cache is not the account's.
fn plan(starter: &[(String, String)], home: &Path, shell: &Path, user: &str, login: &dyn Fn() -> Vec<(String, String)>) -> Option<BTreeMap<String, String>> {
    let given = starter.iter().find(|(k, _)| k == "HOME").map(|(_, v)| PathBuf::from(v))?;
    persons(&given, home).then(|| settled(starter, &login(), home, shell, user))
}

/// Whether HOME `given` reaches the account's own cache: the same folder once links, `.` and `..` are resolved
/// (`/Users/me/../me`, `/Users//me/`, a link to the home or to its `.cache`).
fn persons(given: &Path, home: &Path) -> bool {
    let cache = |h: &Path| resolved(&h.join(".cache/cockpit"));
    given.is_absolute() && cache(given).is_some_and(|c| Some(c) == cache(home))
}

/// The real path of `path`, through its deepest folder that exists: what is left below it is joined on as written, and
/// a `..` there resolves nothing.
fn resolved(path: &Path) -> Option<PathBuf> {
    let mut rest = Vec::new();
    for at in path.ancestors() {
        if let Ok(real) = std::fs::canonicalize(at) {
            let mut out = real;
            for part in rest.iter().rev() {
                match part {
                    Component::Normal(p) => out.push(p),
                    Component::CurDir => {}
                    _ => return None,
                }
            }
            return Some(out);
        }
        rest.extend(at.components().next_back());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn the_desktop_session_of_the_starter_reaches_the_hub() {
        let (home, shell) = (std::path::Path::new("/Users/me"), std::path::Path::new("/bin/zsh"));
        let starter = pairs(&[
            ("DISPLAY", ":0"),
            ("XAUTHORITY", "/run/user/1000/gdm/Xauthority"),
            ("DBUS_SESSION_BUS_ADDRESS", "unix:path=/run/user/1000/bus"),
            ("XDG_RUNTIME_DIR", "/run/user/1000"),
            ("WAYLAND_DISPLAY", "wayland-0"),
            ("HERDR_PANE_ID", "w1:p1"),
        ]);
        let env = settled(&starter, &[], home, shell, "me");
        for kept in ["DISPLAY", "XAUTHORITY", "DBUS_SESSION_BUS_ADDRESS", "XDG_RUNTIME_DIR", "WAYLAND_DISPLAY"] {
            assert!(env.contains_key(kept), "{kept}");
        }
        assert!(!env.contains_key("HERDR_PANE_ID"), "a window's identity still stays out");
        // What the person's own profile sets wins over the starter's.
        let env = settled(&starter, &pairs(&[("DISPLAY", ":1")]), home, shell, "me");
        assert_eq!(env["DISPLAY"], ":1");
    }

    #[test]
    fn the_hub_runs_on_the_login_shell_s_environment_never_the_starter_s() {
        let starter = pairs(&[
            ("PATH", "/tmp/evil:/usr/bin"),
            ("HERDR_BIN_PATH", "/tmp/evil/herdr"),
            ("HC_BIN", "/tmp/evil/hc"),
            ("COCKPIT_AGENTS_DEMO", "/tmp/demo.json"),
            ("SSH_AUTH_SOCK", "/tmp/agent.sock"),
            ("LANG", "en_GB.UTF-8"),
            ("CLAUDECODE", "1"),
        ]);
        let login = pairs(&[("PATH", "/opt/homebrew/bin:/usr/bin"), ("CLAUDE_CONFIG_DIR", "/Users/me/.claude-work"), ("HC_SESSION", "x"), ("PWD", "/")]);
        let env = settled(&starter, &login, std::path::Path::new("/Users/me"), std::path::Path::new("/bin/zsh"), "me");
        assert_eq!(env["PATH"], "/opt/homebrew/bin:/usr/bin");
        assert_eq!(env["CLAUDE_CONFIG_DIR"], "/Users/me/.claude-work", "the person's own settings come from their profile");
        assert_eq!((env["SSH_AUTH_SOCK"].as_str(), env["LANG"].as_str()), ("/tmp/agent.sock", "en_GB.UTF-8"));
        for gone in ["HERDR_BIN_PATH", "HC_BIN", "COCKPIT_AGENTS_DEMO", "CLAUDECODE", "HC_SESSION", "PWD"] {
            assert!(!env.contains_key(gone), "{gone}");
        }
        assert_eq!((env["HOME"].as_str(), env["SHELL"].as_str(), env["USER"].as_str()), ("/Users/me", "/bin/zsh", "me"));
        // A login shell that said nothing still leaves a PATH, never the starter's.
        let bare = settled(&starter, &[], std::path::Path::new("/Users/me"), std::path::Path::new("/bin/zsh"), "me");
        assert!(bare["PATH"].starts_with("/opt/homebrew/bin:") && !bare["PATH"].contains("evil"));
    }

    #[test]
    fn what_a_profile_prints_before_the_mark_is_left_out() {
        let out = format!("Welcome!\nPATH=/fake\n\n{MARK}\nPATH=/usr/bin\0HOME=/Users/me\0EMPTY=\0");
        assert_eq!(parse(&out), pairs(&[("PATH", "/usr/bin"), ("HOME", "/Users/me"), ("EMPTY", "")]));
        assert!(parse("no mark at all").is_empty());
    }

    #[test]
    fn a_login_shell_is_read_on_the_account_s_names_alone() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(home.path().join(".profile"), "echo hello from the profile\nexport CLAUDE_CONFIG_DIR=\"$HOME/.claude-work\"\n").unwrap();
        let env = login(std::path::Path::new("/bin/sh"), home.path(), "me");
        let get = |k: &str| env.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());
        assert_eq!(get("CLAUDE_CONFIG_DIR"), Some(format!("{}/.claude-work", home.path().display())));
        assert_eq!(get("USER").as_deref(), Some("me"));
        assert!(get("HERDR_BIN_PATH").is_none() && get("HC_TERMINAL_ID").is_none(), "nothing of this process's own");
    }

    #[test]
    fn the_person_s_hub_settles_however_home_spells_it() {
        let root = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(root.path()).unwrap();
        let home = root.join("me");
        std::fs::create_dir_all(home.join(".cache/cockpit")).unwrap();
        let link = root.join("home-link");
        std::os::unix::fs::symlink(&home, &link).unwrap();
        // Another HOME whose `.cache` is a link to the account's: the same socket and hub.json.
        let other = root.join("other");
        std::fs::create_dir_all(&other).unwrap();
        std::os::unix::fs::symlink(home.join(".cache"), other.join(".cache")).unwrap();
        let h = home.display();
        for given in [format!("{h}"), format!("{h}/"), format!("{h}/."), h.to_string().replacen("/me", "//me", 1), format!("{h}/../me")] {
            assert!(persons(Path::new(&given), &home), "HOME={given} is the account's");
        }
        assert!(persons(&link, &home), "a link to the home");
        assert!(persons(&other, &home), "a home whose cache is the account's");
        // A home of its own, the sandbox's or a test's, even inside the account's, keeps its starter's environment.
        let sandbox = home.join("sandbox/home");
        std::fs::create_dir_all(&sandbox).unwrap();
        for given in [sandbox.clone(), root.join("elsewhere"), PathBuf::from("me"), root.join("gone/../me")] {
            assert!(!persons(&given, &home), "HOME={} is not the account's", given.display());
        }
        // The cache not made yet: the spelling still resolves through the home.
        let fresh = root.join("fresh");
        std::fs::create_dir_all(&fresh).unwrap();
        assert!(persons(&root.join("fresh/../fresh/."), &fresh));
    }

    #[test]
    fn a_hub_started_with_home_spelled_otherwise_drops_the_starter_s_environment() {
        let root = tempfile::tempdir().unwrap();
        let home = std::fs::canonicalize(root.path()).unwrap().join("me");
        std::fs::create_dir_all(&home).unwrap();
        let alias = format!("{}/../me", home.display());
        let starter = pairs(&[("HOME", alias.as_str()), ("PATH", "/tmp/evil:/usr/bin"), ("HERDR_BIN_PATH", "/tmp/evil/herdr"), ("HC_BIN", "/tmp/evil/hc")]);
        let login = || pairs(&[("PATH", "/opt/homebrew/bin:/usr/bin")]);
        let env = plan(&starter, &home, Path::new("/bin/zsh"), "me", &login).expect("the person's hub settles");
        assert_eq!((env["HOME"].as_str(), env["PATH"].as_str()), (home.to_str().unwrap(), "/opt/homebrew/bin:/usr/bin"));
        assert!(!env.contains_key("HERDR_BIN_PATH") && !env.contains_key("HC_BIN"), "{env:?}");
        let sandbox = pairs(&[("HOME", "/tmp/cc-sb/home"), ("PATH", "/tmp/cc-sb/bin:/usr/bin")]);
        assert_eq!(plan(&sandbox, &home, Path::new("/bin/zsh"), "me", &login), None, "a sandbox keeps what it was given");
        assert_eq!(plan(&pairs(&[("PATH", "/usr/bin")]), &home, Path::new("/bin/zsh"), "me", &login), None, "no HOME, nothing to settle");
    }
}
