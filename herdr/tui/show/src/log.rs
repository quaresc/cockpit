//! A command run every few seconds in the background, its last lines shown with errors in red and warnings in orange,
//! then a status line.

use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use regex::Regex;
use serde_json::Value;

use cockpit_core::text::fit;
use cockpit_core::theme::{ORANGE, RED, dim, fg};

use crate::row::Row;

/// Textual's `Log { max-height: 12 }`: the last lines of the kept ones show.
const SHOWN: usize = 12;
const TIMEOUT: Duration = Duration::from_secs(120);
static ANSI: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\x1b\[[0-9;?]*[A-Za-z]|\x1b\][^\x07]*\x07").unwrap());
static ERROR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b(error|fail(ed|ure)?|exception|fatal)\b").unwrap());
static WARN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bwarn(ing)?\b").unwrap());

pub struct Log {
    pub command: String,
    pub every: f64,
    keep: usize,
    result: Option<(Vec<String>, i32, String)>,
    pub running: bool,
    ran_at: Option<Instant>,
}

/// Runs the command through sh in `cwd`: its output (stdout then stderr) and exit code.
pub fn run(command: &str, cwd: &Path) -> (String, i32) {
    let child = Command::new("sh").args(["-c", command]).current_dir(cwd).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(err) => return (err.to_string(), 1),
    };
    let started = Instant::now();
    let read = |pipe: Option<Box<dyn std::io::Read + Send>>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut bytes);
            }
            String::from_utf8_lossy(&bytes).into_owned()
        })
    };
    let out = read(child.stdout.take().map(|p| Box::new(p) as Box<dyn std::io::Read + Send>));
    let err = read(child.stderr.take().map(|p| Box::new(p) as Box<dyn std::io::Read + Send>));
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return (out.join().unwrap_or_default() + &err.join().unwrap_or_default(), status.code().unwrap_or(1)),
            Ok(None) if started.elapsed() < TIMEOUT => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                return (format!("timed out after {}s", TIMEOUT.as_secs()), 1);
            }
        }
    }
}

impl Log {
    /// The block's command, run every `every` seconds (`default` when the block says nothing), at least every second.
    pub fn new(block: &Value, default: f64) -> Self {
        Self {
            command: block["command"].as_str().unwrap_or_default().to_string(),
            every: block["every"].as_f64().filter(|e| *e > 0.0).unwrap_or(default).max(1.0),
            keep: block["lines"].as_f64().filter(|n| *n >= 1.0).map_or(40, |n| n as usize),
            result: None,
            running: false,
            ran_at: None,
        }
    }

    pub fn due(&mut self) -> bool {
        if self.running || self.ran_at.is_some_and(|at| at.elapsed() < Duration::from_secs_f64(self.every)) {
            return false;
        }
        self.running = true;
        self.ran_at = Some(Instant::now());
        true
    }

    pub fn arrived(&mut self, output: &str, code: i32) {
        self.running = false;
        let clean = ANSI.replace_all(output, "");
        let lines: Vec<String> = clean.trim_end_matches('\n').lines().map(str::to_string).collect();
        let lines = lines[lines.len().saturating_sub(self.keep)..].to_vec();
        self.result = Some((lines, code, chrono::Local::now().format("%H:%M:%S").to_string()));
    }

    pub fn rows(&self, cols: usize) -> Vec<Row> {
        let Some((lines, code, at)) = &self.result else {
            return vec![Row::text(format!("running {}…", self.command), dim())];
        };
        let mut out: Vec<Row> = lines[lines.len().saturating_sub(SHOWN)..]
            .iter()
            .map(|text| {
                let low = text.to_lowercase();
                let style = if ERROR.is_match(&low) { fg(RED) } else if WARN.is_match(&low) { fg(ORANGE) } else { ratatui::style::Style::new() };
                Row::text(fit(&text.replace('\t', "    "), cols).trim_end().to_string(), style)
            })
            .collect();
        let mark = if *code == 0 { "✓".to_string() } else { format!("✗ exit {code}") };
        out.push(Row::text(format!("{mark} · every {}s · {at}", self.every), if *code == 0 { dim() } else { fg(RED) }));
        out
    }
}
