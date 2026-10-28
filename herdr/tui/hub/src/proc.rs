//! Child processes: one run to its end with a timeout, or one left running in its own process group, reaped by a thread
//! so it never lingers as a zombie.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::channel;
use std::time::Duration;

use std::os::unix::process::CommandExt;

pub struct Out {
    /// False when it could not start or ran out of time.
    pub started: bool,
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Out {
    pub fn ok(&self) -> bool {
        self.code == 0
    }
}

/// Runs argv to its end; a failure to start or a timeout reads as exit code 1 with the reason on stderr.
pub fn run(argv: &[String], timeout: Duration, cwd: Option<&Path>) -> Out {
    let Some((program, args)) = argv.split_first() else {
        return Out { started: false, code: 1, stdout: String::new(), stderr: "no command".into() };
    };
    let mut command = Command::new(program);
    command.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let child = match command.spawn() {
        Ok(child) => child,
        Err(err) => {
            return Out { started: false, code: 1, stdout: String::new(), stderr: err.to_string() };
        }
    };
    let pid = child.id();
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    match rx.recv_timeout(timeout) {
        Ok(Ok(out)) => Out {
            started: true,
            code: out.status.code().unwrap_or(1),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        },
        Ok(Err(err)) => Out { started: false, code: 1, stdout: String::new(), stderr: err.to_string() },
        Err(_) => {
            // SAFETY: kill only sends a signal to the child this function started and still owns.
            unsafe { libc::kill(pid as i32, libc::SIGKILL) };
            Out { started: false, code: 1, stdout: String::new(), stderr: format!("Command '{program}' timed out after {} seconds", timeout.as_secs()) }
        }
    }
}

/// Runs argv to its end, 15 seconds at most, like inbox.run.
pub fn run15(argv: &[String]) -> Out {
    run(argv, Duration::from_secs(15), None)
}

/// Starts argv in its own process group, its output to `log` or nowhere; `done` runs with its exit code once it ends.
pub fn spawn(argv: &[String], cwd: Option<&Path>, log: Option<std::fs::File>, done: impl FnOnce(i32) + Send + 'static) -> std::io::Result<u32> {
    let (program, args) = argv.split_first().ok_or_else(|| std::io::Error::other("no command"))?;
    let mut command = Command::new(program);
    command.args(args).stdin(Stdio::null()).process_group(0);
    match log {
        Some(file) => {
            let err = file.try_clone()?;
            command.stdout(file).stderr(err);
        }
        None => {
            command.stdout(Stdio::null()).stderr(Stdio::null());
        }
    }
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let mut child = command.spawn()?;
    let pid = child.id();
    std::thread::spawn(move || {
        let code = child.wait().ok().and_then(|s| s.code()).unwrap_or(1);
        done(code);
    });
    Ok(pid)
}

/// shutil.which: the program's path on PATH, if any.
pub fn which(name: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|dir| dir.join(name)).find(|p| p.is_file() && p.metadata().is_ok_and(|m| m.permissions().mode() & 0o111 != 0))
}

pub fn strings(words: &[&str]) -> Vec<String> {
    words.iter().map(|w| w.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_reports_code_and_output() {
        let out = run(&strings(&["sh", "-c", "echo hi; echo no >&2; exit 3"]), Duration::from_secs(5), None);
        assert_eq!((out.code, out.stdout.trim(), out.stderr.trim()), (3, "hi", "no"));
    }

    #[test]
    fn run_times_out() {
        let out = run(&strings(&["sleep", "5"]), Duration::from_millis(200), None);
        assert_eq!(out.code, 1);
        assert!(out.stderr.contains("timed out"));
    }

    #[test]
    fn missing_program_is_a_failure() {
        assert_eq!(run(&strings(&["/nonexistent/x"]), Duration::from_secs(1), None).code, 1);
    }

    #[test]
    fn spawn_reports_the_exit_code() {
        let (tx, rx) = channel();
        spawn(&strings(&["sh", "-c", "exit 4"]), None, None, move |code| tx.send(code).unwrap()).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), 4);
    }
}
