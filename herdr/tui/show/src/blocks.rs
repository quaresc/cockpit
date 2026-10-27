//! One block of the view, whatever its type: its rows at a width, its focus stops, its keys and clicks, and the
//! background work (diff reads, log runs, image and show.py renders) whose results come back as messages.

use std::path::PathBuf;
use std::sync::mpsc::Sender;

use crossterm::event::KeyEvent;
use ratatui::text::Line;
use ratatui_image::picker::Picker;
use serde_json::Value;

use cockpit_core::theme::{RED, dim, fg};

use crate::diff::{Diff, git_diff};
use crate::log::{Log, run};
use crate::markdown::markdown;
use crate::mermaid;
use crate::picture::{Draw, Drawn, Picture, draw};
use crate::question::{Choice, Confirm, Key};
use crate::row::{Act, Row, Target};
use crate::table::Table;

pub enum Payload {
    Picture(usize, Drawn),
    Legacy(usize, Result<Vec<Line<'static>>, String>),
    Diff(Option<String>, Option<String>),
    Log(String, i32),
}

/// A background result for the block with this uid; a block replaced since then drops it.
pub struct Msg {
    pub uid: u64,
    pub payload: Payload,
}

pub struct Ctx {
    pub tx: Sender<Msg>,
    pub picker: Picker,
    pub cwd: PathBuf,
}

impl Ctx {
    fn spawn(&self, uid: u64, work: impl FnOnce() -> Payload + Send + 'static) {
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Msg { uid, payload: work() });
        });
    }
}

enum Body {
    Markdown(String),
    Table(Table),
    Picture(Picture),
    Diff(Diff),
    Log(Log),
    Choice(Choice),
    Confirm(Confirm),
    Legacy(Option<usize>, Option<Result<Vec<Line<'static>>, String>>),
    Unknown(String),
}

pub struct View {
    pub uid: u64,
    pub spec: Value,
    body: Body,
}

impl View {
    pub fn new(spec: Value, uid: u64) -> Self {
        let kind = spec["type"].as_str().unwrap_or_default();
        let body = match kind {
            "markdown" => Body::Markdown(spec["text"].as_str().unwrap_or_default().to_string()),
            "table" => Body::Table(Table::new(&spec)),
            "chart" => Body::Picture(Picture::new(if spec["kind"] == "bar" { Draw::Bar } else { Draw::Line })),
            "flow" => Body::Picture(Picture::new(Draw::Flow)),
            "mermaid" if mermaid::is_flowchart(spec["text"].as_str().unwrap_or_default()) => Body::Picture(Picture::new(Draw::Flow)),
            "image" => Body::Picture(Picture::new(Draw::Image)),
            "diff" => Body::Diff(Diff::new(&spec)),
            "log" => Body::Log(Log::new(&spec)),
            "choice" => Body::Choice(Choice::new(&spec)),
            "confirm" => Body::Confirm(Confirm::new(&spec)),
            "sequence" | "mermaid" | "ci" => Body::Legacy(None, None),
            other => Body::Unknown(other.to_string()),
        };
        Self { uid, spec, body }
    }

    pub fn kind(&self) -> &str {
        self.spec["type"].as_str().unwrap_or_default()
    }

    pub fn title(&self) -> String {
        self.spec["title"].as_str().filter(|t| !t.is_empty()).unwrap_or(self.kind()).to_string()
    }

    pub fn table(&self) -> Option<&Table> {
        if let Body::Table(table) = &self.body { Some(table) } else { None }
    }

    /// The repo of a diff read from the working tree, for reviewr.
    pub fn repo(&self) -> Option<&str> {
        if let Body::Diff(diff) = &self.body { diff.repo.as_deref() } else { None }
    }

    pub fn image(&self) -> Option<&ratatui_image::sliced::SlicedProtocol> {
        if let Body::Picture(picture) = &self.body { picture.shown.as_ref() } else { None }
    }

    pub fn rows(&mut self, cols: usize, focus: Option<&Target>, ctx: &Ctx) -> Vec<Row> {
        let uid = self.uid;
        match &mut self.body {
            Body::Markdown(text) => markdown(text, cols),
            Body::Table(table) => table.rows(cols, focus.is_some()),
            Body::Picture(picture) => {
                if picture.wants(cols) {
                    let (kind, spec, picker) = (picture.draw, self.spec.clone(), ctx.picker.clone());
                    ctx.spawn(uid, move || Payload::Picture(cols, draw(kind, &spec, cols, &picker)));
                }
                picture.rows()
            }
            Body::Diff(diff) => diff.rows(cols, focus),
            Body::Log(log) => log.rows(cols),
            Body::Choice(choice) => choice.rows(cols, focus),
            Body::Confirm(confirm) => confirm.rows(cols, focus),
            Body::Legacy(asked, lines) => {
                if *asked != Some(cols) {
                    *asked = Some(cols);
                    let spec = self.spec.clone();
                    ctx.spawn(uid, move || Payload::Legacy(cols, crate::legacy::render(&spec, cols)));
                }
                match lines {
                    Some(Ok(lines)) => lines.iter().cloned().map(Row::from_line).collect(),
                    Some(Err(err)) => vec![Row::text(err.clone(), fg(RED))],
                    None => vec![],
                }
            }
            Body::Unknown(kind) => vec![Row::text(format!("{kind}: not ported"), dim())],
        }
    }

    pub fn targets(&self) -> Vec<Target> {
        match &self.body {
            Body::Table(_) => vec![Target::Table],
            Body::Diff(diff) => diff.targets(),
            Body::Choice(choice) => choice.targets(),
            Body::Confirm(confirm) => confirm.targets(),
            _ => vec![],
        }
    }

    /// Starts the background work that is due: a repo diff to read, a log command to run.
    pub fn tick(&mut self, ctx: &Ctx) {
        let uid = self.uid;
        match &mut self.body {
            Body::Diff(diff) => {
                if !diff.due() {
                    return;
                }
                let (repo, base) = (diff.repo.clone().unwrap_or_default(), diff.base.clone());
                ctx.spawn(uid, move || {
                    let (patch, error) = git_diff(&repo, base.as_deref());
                    Payload::Diff(patch, error)
                });
            }
            Body::Log(log) => {
                if !log.due() {
                    return;
                }
                let (command, cwd) = (log.command.clone(), ctx.cwd.clone());
                ctx.spawn(uid, move || {
                    let (output, code) = run(&command, &cwd);
                    Payload::Log(output, code)
                });
            }
            _ => {}
        }
    }

    /// A background result; false when it changed nothing on screen.
    pub fn arrived(&mut self, payload: Payload) -> bool {
        match (&mut self.body, payload) {
            (Body::Picture(picture), Payload::Picture(cols, drawn)) => picture.arrived(cols, drawn),
            (Body::Legacy(asked, lines), Payload::Legacy(cols, drawn)) if *asked == Some(cols) => *lines = Some(drawn),
            (Body::Diff(diff), Payload::Diff(patch, error)) => return diff.arrived(patch, error),
            (Body::Log(log), Payload::Log(output, code)) => log.arrived(&output, code),
            _ => return false,
        }
        true
    }

    pub fn act(&mut self, act: &Act) -> Key {
        match &mut self.body {
            Body::Table(table) => table.act(act),
            Body::Diff(diff) => diff.act(act),
            Body::Choice(choice) => return choice.act(act),
            Body::Confirm(confirm) => return confirm.act(act),
            _ => return Key::Unused,
        }
        Key::Used
    }

    pub fn key(&mut self, target: &Target, key: KeyEvent) -> Key {
        let used = match &mut self.body {
            Body::Table(table) => table.key(key),
            Body::Diff(diff) => diff.key(target, key),
            Body::Choice(choice) => return choice.key(target, key),
            Body::Confirm(confirm) => return confirm.key(target, key),
            _ => false,
        };
        if used { Key::Used } else { Key::Unused }
    }
}
