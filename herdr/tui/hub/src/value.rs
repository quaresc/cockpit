//! JSON values as the actions read and print them: truthiness (null, false, 0, "" and empty lists or objects are false),
//! a value's text, subscripts whose errors name the key; plus shell quoting and the clock.

use serde_json::Value;

pub type Res<T = ()> = Result<T, String>;

pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// The value as notes and prompts print it: None for null, True/False, 2.0 for a whole float.
pub fn text(v: &Value) -> String {
    match v {
        Value::Null => "None".into(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::String(s) => s.clone(),
        Value::Number(n) => match (n.as_i64(), n.as_u64(), n.as_f64()) {
            (Some(i), _, _) => i.to_string(),
            (_, Some(u), _) => u.to_string(),
            (_, _, Some(f)) if f.is_finite() && f.fract() == 0.0 && f.abs() < 1e16 => {
                format!("{f:.1}")
            }
            (_, _, Some(f)) => f.to_string(),
            _ => n.to_string(),
        },
        other => other.to_string(),
    }
}

/// `value[key]`: a missing key or a null value fails naming the key ("missing 'url'").
pub fn key<'a>(v: &'a Value, k: &str) -> Res<&'a Value> {
    match v {
        Value::Object(map) => map.get(k).ok_or_else(|| format!("missing '{k}'")),
        Value::Null => Err(format!("missing '{k}'")),
        _ => Err(format!("cannot read '{k}' of {v}")),
    }
}

/// The key's text, failing like `key` when it is missing.
pub fn s(v: &Value, k: &str) -> Res<String> {
    key(v, k).map(text)
}

/// `value.get(key)`: null when missing.
pub fn get<'a>(v: &'a Value, k: &str) -> &'a Value {
    static NULL: Value = Value::Null;
    v.get(k).unwrap_or(&NULL)
}

/// The key's text, or `default` when it is missing or false.
pub fn or(v: &Value, k: &str, default: &str) -> String {
    let found = get(v, k);
    if truthy(found) { text(found) } else { default.into() }
}

/// The value's items, none when it is not a list.
pub fn list(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or_default()
}

/// A word for a POSIX shell, single-quoted only when it holds more than letters, digits and `_@%+=:,./-`. Control
/// characters but the newline are dropped (a tab becomes a space): a line editor acts on them inside quotes, so a ^U,
/// ^C or ESC in a word typed into an interactive shell would run what follows it.
pub fn quote(s: &str) -> String {
    let s: String = s
        .chars()
        .filter_map(|c| {
            if c == '\t' {
                Some(' ')
            } else if c.is_control() && c != '\n' {
                None
            } else {
                Some(c)
            }
        })
        .collect();
    if s.is_empty() {
        return "''".into();
    }
    if s.chars().all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c)) {
        return s;
    }
    format!("'{}'", s.replace('\'', "'\"'\"'"))
}

/// Words joined into a shell command line, each quoted as `quote` does.
pub fn join(words: &[String]) -> String {
    words.iter().map(|w| quote(w)).collect::<Vec<_>>().join(" ")
}

/// The first `n` characters, like `text[:n]`.
pub fn cut(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

pub fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

pub fn now_ms() -> i64 {
    (now() * 1000.0) as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn text_of_null_numbers_and_strings() {
        assert_eq!(text(&json!(null)), "None");
        assert_eq!(text(&json!(2)), "2");
        assert_eq!(text(&json!(2.0)), "2.0");
        assert_eq!(text(&json!(2.5)), "2.5");
        assert_eq!(text(&json!("x")), "x");
    }

    #[test]
    fn quote_for_the_shell() {
        assert_eq!(quote(""), "''");
        assert_eq!(quote("a/b-c.d"), "a/b-c.d");
        assert_eq!(quote("it's"), "'it'\"'\"'s'");
        assert_eq!(join(&["claude".into(), "/code-review 12".into()]), "claude '/code-review 12'");
        assert_eq!(quote("a\u{15}b\u{3}c\u{1b}qd\u{7f}e\u{9b}f\tg\nh\r"), "'abcqdef g\nh'", "C0, DEL and C1 out, a tab a space, lines kept");
        assert_eq!(quote("\u{3}\u{4}"), "''", "nothing left but quotes");
    }

    /// Types `line` and Enter into `shell`, interactive on a terminal of its own as herdr's `pane run` does: whether
    /// `marker` exists afterwards.
    fn typed_runs(shell: &[&str], line: &str, marker: &std::path::Path) -> bool {
        use std::io::{Read, Write};
        use std::os::fd::FromRawFd;
        use std::os::unix::process::CommandExt;
        let (mut master, mut slave) = (0, 0);
        // openpty is not safe to call from two threads at once (ptsname's buffer is shared).
        static OPEN: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let held = OPEN.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: openpty fills the two descriptors it is given and reads no name, termios or size.
        let opened = unsafe { libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()) };
        drop(held);
        assert_eq!(opened, 0, "no terminal for the shell");
        // SAFETY: each descriptor is this test's own, owned by one File from here on.
        let (mut master, slave) = unsafe { (std::fs::File::from_raw_fd(master), std::fs::File::from_raw_fd(slave)) };
        let mut command = std::process::Command::new(shell[0]);
        command.args(&shell[1..]).env("PS1", "$ ").env("TERM", "xterm-256color");
        command.stdin(slave.try_clone().unwrap()).stdout(slave.try_clone().unwrap()).stderr(slave);
        // SAFETY: setsid and the ioctl that makes the terminal the session's are async-signal-safe, between fork and exec.
        unsafe {
            command.pre_exec(|| {
                libc::setsid();
                libc::ioctl(0, libc::TIOCSCTTY as _, 0);
                Ok(())
            });
        }
        let mut child = command.spawn().unwrap();
        drop(command);
        let mut reader = master.try_clone().unwrap();
        std::thread::spawn(move || std::io::copy(&mut reader, &mut std::io::sink()));
        std::thread::sleep(std::time::Duration::from_millis(800));
        master.write_all(format!("{line}\r").as_bytes()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1200));
        let _ = child.kill();
        let _ = child.wait();
        let _ = master.read(&mut [0; 1]);
        marker.exists()
    }

    #[test]
    fn a_quoted_word_typed_into_an_interactive_shell_runs_nothing_whatever_controls_it_held() {
        let shells: Vec<Vec<&str>> =
            [vec!["zsh", "-f", "-i"], vec!["bash", "--norc", "--noprofile", "-i"]].into_iter().filter(|s| crate::proc::which(s[0]).is_some()).collect();
        assert!(!shells.is_empty(), "no zsh or bash to type into");
        let dir = tempfile::tempdir().unwrap();
        // The agent's argv as a launch typed it, `true` standing in for the agent: a mail's body, a monitor's message.
        let payloads = [
            ("^U after a newline", "Draft a reply.\n\nFrom: x\n\nHi,\n\u{15}touch {M}\r"),
            ("^C after a newline", "Draft a reply.\n\nHi,\n\u{3}touch {M}\r"),
            ("^G", "Draft a reply.\n\nHi,\n\u{7}touch {M}\r"),
            ("ESC q", "Draft a reply.\n\nHi,\n\u{1b}qtouch {M}\r"),
            ("^D", "Draft a reply.\n\nHi,\n\u{4}touch {M}\r"),
            ("^C on one line", "Investigate api: error X: \u{3}touch {M}\r"),
            ("^U on one line", "Investigate api: error X: \u{15}touch {M}\r"),
            ("plain quoting", "Draft... it's $(touch {M}) `touch {M}`"),
        ];
        let mut runs = Vec::new();
        for (n, shell) in shells.iter().enumerate() {
            // The harness itself: a command typed as it is does run.
            let marker = dir.path().join(format!("sanity-{n}"));
            runs.push((format!("{} sanity", shell[0]), shell.clone(), format!("touch {}", marker.display()), marker, true));
            for (i, (name, text)) in payloads.iter().enumerate() {
                let marker = dir.path().join(format!("pwned-{n}-{i}"));
                let line = format!("true {}", quote(&text.replace("{M}", &marker.to_string_lossy())));
                runs.push((format!("{} {name}", shell[0]), shell.clone(), line, marker, false));
            }
        }
        let outcomes: Vec<(String, bool, bool)> = std::thread::scope(|scope| {
            let handles: Vec<_> =
                runs.iter().map(|(name, shell, line, marker, expect)| scope.spawn(move || (name.clone(), typed_runs(shell, line, marker), *expect))).collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        let wrong: Vec<&String> = outcomes.iter().filter(|(_, ran, expect)| ran != expect).map(|(name, _, _)| name).collect();
        assert!(wrong.is_empty(), "these ran, or the harness typed nothing: {wrong:?}");
    }

    #[test]
    fn truthiness() {
        assert!(!truthy(&json!({})));
        assert!(!truthy(&json!("")));
        assert!(!truthy(&json!(0)));
        assert!(truthy(&json!({"a": 1})));
    }

    #[test]
    fn a_missing_key_fails_with_its_name() {
        assert_eq!(key(&json!({}), "url"), Err("missing 'url'".to_string()));
    }
}
