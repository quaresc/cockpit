//! A command run every few seconds in the background, its lines shown with errors in red and warnings in orange, then a
//! status line. Focused, the block scrolls on its own (j/k/g/G); it follows the end until scrolled up, and again at G.

use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use regex::Regex;
use serde_json::Value;

use crossterm::event::{KeyCode, KeyEvent};

use cockpit_core::text::fit;
use cockpit_core::theme::{LINE, ORANGE, RED, dim, fg};

use crate::row::{Row, Target};

/// How many lines show at once; the block scrolls through the rest.
const SHOWN: usize = 20;
/// How many lines are kept when the block does not say.
const KEPT: usize = 500;
const THUMB: ratatui::style::Color = ratatui::style::Color::Rgb(122, 98, 80);
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
    /// The first line shown when scrolled up; None follows the end.
    top: Option<usize>,
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
            keep: block["lines"].as_f64().filter(|n| *n >= 1.0).map_or(KEPT, |n| n as usize),
            result: None,
            running: false,
            ran_at: None,
            top: None,
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

    /// Keeps the output and scroll of the block this one replaces, so a refresh does not blank it.
    pub fn inherit(&mut self, old: Log) {
        (self.result, self.top, self.ran_at) = (old.result, old.top, old.ran_at);
    }

    /// A run came back; false when it printed the same as the last one.
    pub fn arrived(&mut self, output: &str, code: i32) -> bool {
        self.running = false;
        let clean = ANSI.replace_all(output, "");
        let lines: Vec<String> = clean.trim_end_matches('\n').lines().map(|l| l.replace('\t', "    ")).collect();
        let lines = lines[lines.len().saturating_sub(self.keep)..].to_vec();
        let same = self.result.as_ref().is_some_and(|(old, old_code, _)| *old == lines && *old_code == code);
        self.result = Some((lines, code, chrono::Local::now().format("%H:%M:%S").to_string()));
        !same
    }

    fn total(&self) -> usize {
        self.result.as_ref().map_or(0, |r| r.0.len())
    }

    pub fn targets(&self) -> Vec<Target> {
        if self.result.is_some() { vec![Target::Log] } else { vec![] }
    }

    /// j/k a line, d/u half the block, g the first line, G back to following the end.
    pub fn key(&mut self, key: KeyEvent) -> bool {
        let last = self.total().saturating_sub(SHOWN);
        let top = self.top.unwrap_or(last);
        let to = |by: isize| top.saturating_add_signed(by).min(last);
        let top = match key.code {
            KeyCode::Char('k') | KeyCode::Up => to(-1),
            KeyCode::Char('j') | KeyCode::Down => to(1),
            KeyCode::Char('u') | KeyCode::PageUp => to(-(SHOWN as isize) / 2),
            KeyCode::Char('d') | KeyCode::PageDown => to(SHOWN as isize / 2),
            KeyCode::Char('g') | KeyCode::Home => 0,
            KeyCode::Char('G') | KeyCode::End => last,
            _ => return false,
        };
        self.top = (top < last).then_some(top);
        true
    }

    pub fn rows(&self, cols: usize, focused: bool) -> Vec<Row> {
        let Some((lines, code, at)) = &self.result else {
            return vec![Row::text(format!("running {}…", self.command), dim())];
        };
        let total = lines.len();
        let last = total.saturating_sub(SHOWN);
        let top = self.top.unwrap_or(last).min(last);
        let shown = total.min(SHOWN);
        let scrolls = total > SHOWN;
        let room = if scrolls { cols.saturating_sub(2) } else { cols };
        let (size, start) = ((shown * shown / total.max(1)).max(1), if total > shown { top * (shown - (shown * shown / total).max(1)) / last.max(1) } else { 0 });
        let mut out: Vec<Row> = lines[top..top + shown]
            .iter()
            .enumerate()
            .map(|(n, text)| {
                let low = text.to_lowercase();
                let style = if ERROR.is_match(&low) { fg(RED) } else if WARN.is_match(&low) { fg(ORANGE) } else { ratatui::style::Style::new() };
                let mut row = Row::text(fit(text, room), style);
                if scrolls {
                    let thumb = (start..start + size).contains(&n);
                    row = row.add(" ", style).add(if thumb { "┃" } else { "│" }, fg(if thumb && focused { ORANGE } else if thumb { THUMB } else { LINE }));
                }
                row.target(Target::Log)
            })
            .collect();
        out.push(Row::new().target(Target::Log));
        let mark = if *code == 0 { "✓".to_string() } else { format!("✗ exit {code}") };
        let mut status = Row::text(format!("{mark} · every {}s · {at}", self.every), if *code == 0 { dim() } else { fg(RED) });
        if scrolls {
            let place = if self.top.is_some() { format!("   lines {}–{} of {total} · G follows", top + 1, top + shown) } else { format!("   following the last {shown} of {total}") };
            status = status.add(place, dim());
        }
        out.push(status.target(Target::Log));
        out
    }
}
