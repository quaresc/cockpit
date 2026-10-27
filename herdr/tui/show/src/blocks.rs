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
    Fed(String, i32),
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
    Note(String),
}

pub struct View {
    pub uid: u64,
    pub spec: Value,
    body: Body,
    /// A table or chart's command printing its rows as JSON, and the block with the rows it printed last.
    feed: Option<Log>,
    fed: Option<Value>,
    /// A CI block's caches as last drawn: the mod rewrites them as runs move on.
    seen: Vec<Option<std::time::SystemTime>>,
}

/// When the caches a CI block is drawn from last changed.
fn ci_caches() -> Vec<Option<std::time::SystemTime>> {
    ["ci.json", "inbox.json"].iter().map(|f| std::fs::metadata(cockpit_core::files::cache().join(f)).and_then(|m| m.modified()).ok()).collect()
}

/// Lines of text wrapped to the pane, each a row in `style`.
pub fn wrapped(text: &str, cols: usize, style: ratatui::style::Style) -> Vec<Row> {
    cockpit_core::text::wrap(text, cols).into_iter().map(|line| Row::text(line, style)).collect()
}

/// A long error (git's usage text) shows its first lines only.
pub fn error_rows(text: &str, cols: usize) -> Vec<Row> {
    let mut rows = wrapped(text.trim_end(), cols, fg(RED));
    if rows.len() > 8 {
        rows.truncate(7);
        rows.push(Row::text("…", fg(RED)));
    }
    rows
}

/// A line of show.py's drawn at the pane's width; a longer one (a CI run's PR title) wraps between words like Textual's.
fn fitted(line: &Line<'static>, cols: usize) -> Vec<Row> {
    if line.width() <= cols {
        return vec![Row::from_line(line.clone())];
    }
    let segments = line.spans.iter().map(|s| (s.content.to_string(), line.style.patch(s.style), None)).collect();
    crate::markdown::wrap(segments, cols)
        .into_iter()
        .map(|row| Row::from_line(Line::from(row.into_iter().map(|(text, style, _)| ratatui::text::Span::styled(text, style)).collect::<Vec<_>>())))
        .collect()
}

fn body_for(spec: &Value) -> Body {
    let kind = spec["type"].as_str().unwrap_or_default();
    match kind {
        "markdown" => Body::Markdown(spec["text"].as_str().unwrap_or_default().to_string()),
        "table" => Body::Table(Table::new(spec)),
        "chart" => Body::Picture(Picture::new(if spec["kind"] == "bar" { Draw::Bar } else { Draw::Line })),
        "flow" => Body::Picture(Picture::new(Draw::Flow)),
        "mermaid" if mermaid::is_flowchart(spec["text"].as_str().unwrap_or_default()) => Body::Picture(Picture::new(Draw::Flow)),
        "image" => Body::Picture(Picture::new(Draw::Image)),
        "diff" => Body::Diff(Diff::new(spec)),
        "log" => Body::Log(Log::new(spec, 5.0)),
        "choice" => Body::Choice(Choice::new(spec)),
        "confirm" => Body::Confirm(Confirm::new(spec)),
        "sequence" | "mermaid" | "ci" => Body::Legacy(None, None),
        other => Body::Unknown(other.to_string()),
    }
}

impl View {
    pub fn new(spec: Value, uid: u64) -> Self {
        let command = spec["command"].as_str().filter(|c| !c.is_empty() && (spec["type"] == "table" || spec["type"] == "chart"));
        let (body, feed) = match command {
            Some(command) => (Body::Note(format!("running {command}…")), Some(Log::new(&spec, 30.0))),
            None => (body_for(&spec), None),
        };
        let seen = if spec["type"] == "ci" { ci_caches() } else { Vec::new() };
        Self { uid, spec, body, feed, fed: None, seen }
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
        let block = self.fed.as_ref().unwrap_or(&self.spec);
        match &mut self.body {
            Body::Markdown(text) => markdown(text, cols),
            Body::Table(table) => table.rows(cols, focus.is_some()),
            Body::Picture(picture) => {
                if picture.wants(cols) {
                    let (kind, spec, picker) = (picture.draw, block.clone(), ctx.picker.clone());
                    ctx.spawn(uid, move || Payload::Picture(cols, draw(kind, &spec, cols, &picker)));
                }
                let mut rows = picture.rows(cols);
                if let Some(caption) = block["caption"].as_str().filter(|c| !c.is_empty() && picture.shown.is_some()) {
                    rows.extend(wrapped(caption, cols, dim()));
                }
                rows
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
                    Some(Ok(lines)) => lines.iter().flat_map(|line| fitted(line, cols)).collect(),
                    Some(Err(err)) => error_rows(err, cols),
                    None => vec![],
                }
            }
            Body::Unknown(kind) => vec![Row::text(format!("{kind}: not ported"), dim())],
            Body::Note(text) => wrapped(text, cols, dim()),
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
        if let Some(feed) = self.feed.as_mut()
            && feed.due()
        {
            let (command, cwd) = (feed.command.clone(), ctx.cwd.clone());
            ctx.spawn(uid, move || {
                let (output, code) = run(&command, &cwd);
                Payload::Fed(output, code)
            });
        }
        match &mut self.body {
            Body::Legacy(Some(cols), _) if !self.seen.is_empty() => {
                let now = ci_caches();
                if now != self.seen {
                    self.seen = now;
                    let (spec, cols) = (self.spec.clone(), *cols);
                    ctx.spawn(uid, move || Payload::Legacy(cols, crate::legacy::render(&spec, cols)));
                }
            }
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
        if let Payload::Fed(output, code) = payload {
            return self.fed(&output, code);
        }
        match (&mut self.body, payload) {
            (Body::Picture(picture), Payload::Picture(cols, drawn)) => picture.arrived(cols, drawn),
            (Body::Legacy(asked, lines), Payload::Legacy(cols, drawn)) if *asked == Some(cols) => *lines = Some(drawn),
            (Body::Diff(diff), Payload::Diff(patch, error)) => return diff.arrived(patch, error),
            (Body::Log(log), Payload::Log(output, code)) => log.arrived(&output, code),
            _ => return false,
        }
        true
    }

    /// A command's rows came back: the table or chart is built again when they changed, else its error shows.
    fn fed(&mut self, output: &str, code: i32) -> bool {
        let Some(feed) = self.feed.as_mut() else { return false };
        feed.running = false;
        match crate::value::rows_of(output) {
            Ok((columns, rows)) => {
                let mut block = self.spec.clone();
                (block["columns"], block["rows"]) = (columns, rows);
                if self.fed.as_ref() == Some(&block) {
                    return false;
                }
                self.body = body_for(&block);
                self.fed = Some(block);
            }
            Err(err) => {
                let shown: String = output.trim().chars().take(120).collect();
                self.body = Body::Note(format!("{}: exit {code}, {err}: {shown}", feed.command));
                self.fed = None;
            }
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
