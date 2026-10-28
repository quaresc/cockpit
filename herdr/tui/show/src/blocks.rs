//! One block of the view, whatever its type: its rows at a width, its focus stops, its keys, clicks and hovers, and the
//! background work (diff reads, log runs, picture draws and text drawings) whose results come back as messages.

use std::cell::Cell;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::time::{Duration, SystemTime};

use crossterm::event::KeyEvent;
use ratatui_image::picker::Picker;
use ratatui_image::sliced::SlicedProtocol;
use serde_json::Value;

use cockpit_core::theme::{RED, dim, fg};

use crate::diff::{Diff, Source, hash};
use crate::log::{Log, run};
use crate::markdown::markdown;
use crate::mermaid;
use crate::patch::{File, git_diff, prepare};
use crate::picture::{Draw, Drawn, Picture, draw};
use crate::question::{Choice, Confirm, Key};
use crate::row::{Act, Row, Target};
use crate::table::Table;

pub enum Payload {
    Picture(Result<Drawn, String>),
    TextDrawing(usize, crate::text_drawing::Drawn),
    /// A diff read: its patch's hash, its files when they changed, or the error that stopped it.
    Diff(u64, Option<Vec<File>>, Option<String>),
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
    /// The config's compact density: less room between and inside blocks.
    pub compact: bool,
    /// Background jobs sent and not back yet.
    pub running: Cell<usize>,
}

impl Ctx {
    fn spawn(&self, uid: u64, work: impl FnOnce() -> Payload + Send + 'static) {
        let tx = self.tx.clone();
        self.running.set(self.running.get() + 1);
        std::thread::spawn(move || {
            let _ = tx.send(Msg { uid, payload: work() });
            cockpit_core::term::wake();
        });
    }

    fn draw(&self, uid: u64, picture: &mut Picture, cols: Option<usize>) {
        let Some(job) = cols.and_then(|cols| picture.job(cols)) else { return };
        let picker = self.picker.clone();
        self.spawn(uid, move || {
            let drawn = draw(job, &picker);
            crate::raster::give_back();
            Payload::Picture(drawn)
        });
    }
}

enum Body {
    /// The text and its rows at the last width, which only a width change draws again.
    Markdown(String, Option<(usize, Vec<Row>)>),
    Table(Table),
    Picture(Picture),
    Diff(Diff),
    Log(Log),
    Choice(Choice),
    Confirm(Confirm),
    /// A block drawn as text, at the width it was last asked for.
    TextDrawing(Option<usize>, Option<crate::text_drawing::Drawn>),
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
    seen: Vec<Option<SystemTime>>,
}

/// When the caches a CI block is drawn from last changed.
fn ci_caches() -> Vec<Option<SystemTime>> {
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

/// A line drawn as text at the pane's width; a longer one (a CI run's PR title, a crowded diagram) wraps between words.
fn fitted(line: &[crate::markdown::Segment], cols: usize) -> Vec<Row> {
    let shown: usize = line.iter().map(|(t, _, _)| cockpit_core::text::width(t)).sum();
    let rows = if shown <= cols { vec![line.to_vec()] } else { crate::markdown::wrap(line.to_vec(), cols) };
    rows.into_iter().map(|row| crate::markdown::add_segments(Row::new(), row)).collect()
}

fn body_for(spec: &Value) -> Body {
    let kind = spec["type"].as_str().unwrap_or_default();
    let picture = |draw| Body::Picture(Picture::new(draw, spec.clone()));
    match kind {
        "markdown" => Body::Markdown(spec["text"].as_str().unwrap_or_default().to_string(), None),
        "table" => Body::Table(Table::new(spec)),
        "chart" => picture(if spec["kind"] == "bar" { Draw::Bar } else { Draw::Line }),
        "flow" => picture(Draw::Flow),
        "mermaid" if mermaid::is_flowchart(spec["text"].as_str().unwrap_or_default()) => picture(Draw::Flow),
        "image" => picture(Draw::Image),
        "compare" => picture(Draw::Compare),
        "diff" => Body::Diff(Diff::new(spec)),
        "log" => Body::Log(Log::new(spec, 5.0)),
        "choice" => Body::Choice(Choice::new(spec)),
        "confirm" => Body::Confirm(Confirm::new(spec)),
        "sequence" | "mermaid" | "ci" => Body::TextDrawing(None, None),
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

    /// Keeps what the block it replaces showed while the new one gets ready: a picture's image (a chart's values glide
    /// to the new ones), a log's output and scroll.
    pub fn inherit(&mut self, old: View) {
        match (&mut self.body, old.body) {
            (Body::Picture(new), Body::Picture(old)) => new.inherit(old),
            (Body::Log(new), Body::Log(old)) if new.command == old.command => new.inherit(old),
            _ => {}
        }
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
        if let Body::Diff(diff) = &self.body { diff.repo() } else { None }
    }

    pub fn image(&self) -> Option<&SlicedProtocol> {
        if let Body::Picture(picture) = &self.body { picture.protocol() } else { None }
    }

    /// True while a comment is typed in the block: every key goes to it.
    pub fn is_typing(&self) -> bool {
        matches!(&self.body, Body::Diff(diff) if diff.is_typing())
    }

    pub fn rows(&mut self, cols: usize, focus: Option<&Target>, ctx: &Ctx) -> Vec<Row> {
        let uid = self.uid;
        match &mut self.body {
            Body::Markdown(text, drawn) => {
                if drawn.as_ref().is_none_or(|(at, _)| *at != cols) {
                    *drawn = Some((cols, markdown(text, cols)));
                }
                drawn.as_ref().map(|(_, rows)| rows.clone()).unwrap_or_default()
            }
            Body::Table(table) => table.rows(cols, focus.is_some()),
            Body::Picture(picture) => {
                ctx.draw(uid, picture, Some(cols));
                let mut rows = picture.rows(cols, focus.is_some(), ctx.compact);
                if let Some(caption) = picture.block["caption"].as_str().filter(|c| !c.is_empty() && picture.protocol().is_some()) {
                    rows.extend(wrapped(caption, cols, dim()));
                }
                rows
            }
            Body::Diff(diff) => diff.rows(cols, focus),
            Body::Log(log) => log.rows(cols, focus.is_some()),
            Body::Choice(choice) => choice.rows(cols, focus),
            Body::Confirm(confirm) => confirm.rows(cols, focus),
            Body::TextDrawing(asked, lines) => {
                if *asked != Some(cols) {
                    *asked = Some(cols);
                    let spec = self.spec.clone();
                    ctx.spawn(uid, move || Payload::TextDrawing(cols, crate::text_drawing::render(&spec, cols)));
                }
                lines.iter().flatten().flat_map(|line| fitted(line, cols)).collect()
            }
            Body::Unknown(kind) => vec![Row::text(format!("{kind}: unknown block type"), dim())],
            Body::Note(text) => wrapped(text, cols, dim()),
        }
    }

    pub fn targets(&self) -> Vec<Target> {
        match &self.body {
            Body::Table(_) => vec![Target::Table],
            Body::Diff(diff) => diff.targets(),
            Body::Log(log) => log.targets(),
            Body::Picture(picture) => picture.targets(),
            Body::Choice(choice) => choice.targets(),
            Body::Confirm(confirm) => confirm.targets(),
            _ => vec![],
        }
    }

    /// Starts the background work that is due (a repo diff to read, a command to run, a picture to draw again) and
    /// steps a GIF; true when the screen changed.
    pub fn tick(&mut self, ctx: &Ctx) -> bool {
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
            Body::TextDrawing(Some(cols), _) if !self.seen.is_empty() => {
                let now = ci_caches();
                if now != self.seen {
                    self.seen = now;
                    let (spec, cols) = (self.spec.clone(), *cols);
                    ctx.spawn(uid, move || Payload::TextDrawing(cols, crate::text_drawing::render(&spec, cols)));
                }
            }
            Body::Picture(picture) => {
                let cols = picture.asked();
                ctx.draw(uid, picture, cols);
                return picture.advance();
            }
            Body::Diff(diff) => {
                if !diff.due() {
                    return false;
                }
                let shown = diff.shown();
                let work = match &diff.source {
                    Source::Patch(patch) => Ok(patch.clone()),
                    Source::Repo { repo, base } => Err((repo.clone(), base.clone())),
                };
                ctx.spawn(uid, move || {
                    let patch = match work {
                        Ok(patch) => patch,
                        Err((repo, base)) => match git_diff(&repo, base.as_deref()) {
                            Ok(patch) => patch,
                            Err(error) => return Payload::Diff(0, None, Some(error)),
                        },
                    };
                    let hash = hash(&patch);
                    let files = (shown != Some(hash)).then(|| prepare(&patch));
                    crate::raster::give_back();
                    Payload::Diff(hash, files, None)
                });
            }
            Body::Log(log) => {
                if !log.due() {
                    return false;
                }
                let (command, cwd) = (log.command.clone(), ctx.cwd.clone());
                ctx.spawn(uid, move || {
                    let (output, code) = run(&command, &cwd);
                    Payload::Log(output, code)
                });
            }
            _ => {}
        }
        false
    }

    /// True while a chart's values glide, for the loop to tick at their pace.
    pub fn moving(&self) -> bool {
        matches!(&self.body, Body::Picture(p) if p.moving())
    }

    /// True while a picture waits for a draw, for tests to wait for it.
    pub fn pending(&self) -> bool {
        matches!(&self.body, Body::Picture(p) if p.pending())
    }

    /// How long until the block's GIF shows its next frame.
    pub fn next_due(&self) -> Option<Duration> {
        if let Body::Picture(p) = &self.body { p.next_due() } else { None }
    }

    /// Whether the block's picture was on screen at the last frame.
    pub fn set_visible(&mut self, visible: bool) {
        if let Body::Picture(p) = &mut self.body {
            p.visible = visible;
        }
    }

    /// The mouse over a cell of the block's picture (its column and image row), or away from it; true when that
    /// changed what it shows.
    pub fn hover(&mut self, at: Option<(usize, u16)>, font: ratatui_image::FontSize) -> bool {
        if let Body::Picture(p) = &mut self.body { p.hover(at, font) } else { false }
    }

    /// The mouse pressed or dragged at a column of the block's picture: a compare's split follows it.
    pub fn drag(&mut self, col: usize) -> bool {
        match &mut self.body {
            Body::Picture(p) if p.draw == Draw::Compare => p.drag(col),
            _ => false,
        }
    }

    /// A background result; false when it changed nothing on screen.
    pub fn arrived(&mut self, payload: Payload) -> bool {
        if let Payload::Fed(output, code) = payload {
            return self.fed(&output, code);
        }
        match (&mut self.body, payload) {
            (Body::Picture(picture), Payload::Picture(drawn)) => picture.arrived(drawn),
            (Body::TextDrawing(asked, lines), Payload::TextDrawing(cols, drawn)) if *asked == Some(cols) => {
                *lines = Some(drawn);
                true
            }
            (Body::Diff(diff), Payload::Diff(hash, files, error)) => diff.arrived(hash, files, error),
            (Body::Log(log), Payload::Log(output, code)) => log.arrived(&output, code),
            _ => false,
        }
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
                let old = std::mem::replace(&mut self.body, body_for(&block));
                if let (Body::Picture(new), Body::Picture(old)) = (&mut self.body, old) {
                    new.inherit(old);
                }
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
            Body::Diff(diff) => return diff.key(key),
            Body::Log(log) => log.key(key),
            Body::Picture(picture) => picture.key(key),
            Body::Choice(choice) => return choice.key(target, key),
            Body::Confirm(confirm) => return confirm.key(target, key),
            _ => false,
        };
        if used { Key::Used } else { Key::Unused }
    }
}
