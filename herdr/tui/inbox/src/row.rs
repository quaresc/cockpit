//! A tree row: aligned cells (lead glyphs, ref, title, notes, age) or one free text, with its actions, the call its
//! enter makes and the details the panel shows.

use cockpit_core::acts::Act;
use cockpit_core::text::{fit, width};
use cockpit_core::theme::{DIM, GREEN, ORANGE, RED, fg};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;

use crate::icons::icon;
use crate::requests::Requests;

pub const LEAD: usize = 4;
const REF: usize = 24;
const NOTES: usize = 16;
const AGE: usize = 5;

/// One of inbox.py's functions, called through the bridge with these arguments.
#[derive(Clone, Debug)]
pub struct Call {
    pub function: &'static str,
    pub args: Vec<Value>,
}

pub fn call(function: &'static str, args: Vec<Value>) -> Call {
    Call { function, args }
}

pub fn open(url: &str) -> Call {
    call("open_url", vec![Value::from(url)])
}

/// An action: its button, and what it calls, or the calls to pick from when it asks where first.
pub struct RowAct {
    pub act: Act,
    pub call: Option<Call>,
    pub picks: Vec<Call>,
}

impl RowAct {
    pub fn new(label: &str, key: char, call: Call) -> Self {
        Self { act: Act::new(label, key, call.function), call: Some(call), picks: Vec::new() }
    }

    pub fn pick(label: &str, key: char, choices: Vec<(&str, Call)>) -> Self {
        let labels: Vec<&str> = choices.iter().map(|(l, _)| *l).collect();
        Self { act: Act::new(label, key, label).choices(&labels), call: None, picks: choices.into_iter().map(|(_, c)| c).collect() }
    }

    pub fn main(self) -> Self {
        self.main_if(true)
    }

    pub fn main_if(mut self, is_main: bool) -> Self {
        self.act.main = is_main;
        self
    }

    pub fn confirm(mut self, key: String) -> Self {
        self.act.confirm = Some(key);
        self
    }
}

/// What a row stands for: rows of one kind can be selected together and share their actions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// Headings, groups and detail lines: never selected.
    Plain,
    Pr,
    Review,
    Ticket,
    Alert,
    Brief,
    Muted,
}

impl Kind {
    /// How a count of these reads: "1 PR", "4 PRs".
    pub fn count(self, n: usize) -> String {
        let (one, many) = match self {
            Kind::Pr => ("PR", "PRs"),
            Kind::Review => ("review request", "review requests"),
            Kind::Ticket => ("ticket", "tickets"),
            Kind::Alert => ("alert", "alerts"),
            Kind::Brief => ("brief line", "brief lines"),
            Kind::Muted => ("muted repo", "muted repos"),
            Kind::Plain => ("row", "rows"),
        };
        format!("{n} {}", if n == 1 { one } else { many })
    }
}

/// How a filter's matched characters show: underlined and bold, which survives the cursor's colours.
fn hit_style() -> Style {
    Style::new().fg(ORANGE).add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
}

/// Text split into spans so the characters at `hits` (counted from `from` in the row's haystack) stand out.
fn marked(text: &str, original: &str, from: usize, hits: &[usize], style: Style) -> Vec<Span<'static>> {
    if hits.is_empty() {
        return vec![Span::styled(text.to_string(), style)];
    }
    let original: Vec<char> = original.chars().collect();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut run = String::new();
    let mut run_hit = false;
    for (i, c) in text.chars().enumerate() {
        let hit = original.get(i) == Some(&c) && hits.binary_search(&(from + i)).is_ok();
        if hit != run_hit && !run.is_empty() {
            spans.push(Span::styled(std::mem::take(&mut run), if run_hit { style.patch(hit_style()) } else { style }));
        }
        run_hit = hit;
        run.push(c);
    }
    if !run.is_empty() {
        spans.push(Span::styled(run, if run_hit { style.patch(hit_style()) } else { style }));
    }
    spans
}

pub struct Cells {
    pub lead: Vec<(String, Style)>,
    pub reference: (String, Style),
    pub title: (String, Style),
    pub notes: (String, Style),
    pub age: String,
}

pub enum Body {
    Text(Line<'static>),
    Cells(Cells),
}

pub struct Row {
    pub key: String,
    pub body: Body,
    pub acts: Vec<RowAct>,
    pub primary: Option<Call>,
    pub info: Vec<Line<'static>>,
    /// The key its requests in flight go by, to show their progress on it.
    pub status: Option<String>,
    pub expand: bool,
    pub name: String,
    pub section: String,
    pub kind: Kind,
    /// The PR a PR or review request row stands for, for its details on demand.
    pub item: Option<Value>,
    pub children: Vec<Row>,
}

impl Row {
    pub fn text(key: impl Into<String>, line: Line<'static>) -> Self {
        let name = line.spans.iter().map(|s| s.content.as_ref()).collect::<String>().trim().to_string();
        Self::with(key.into(), Body::Text(line), name)
    }

    pub fn cells(key: impl Into<String>, cells: Cells, name: String) -> Self {
        Self::with(key.into(), Body::Cells(cells), name)
    }

    fn with(key: String, body: Body, name: String) -> Self {
        Self { key, body, acts: Vec::new(), primary: None, info: Vec::new(), status: None, expand: true, name, section: String::new(), kind: Kind::Plain, item: None, children: Vec::new() }
    }

    pub fn acts(mut self, acts: Vec<RowAct>) -> Self {
        self.acts = acts;
        self
    }

    pub fn primary(mut self, call: Call) -> Self {
        self.primary = Some(call);
        self
    }

    pub fn info(mut self, info: Vec<Line<'static>>) -> Self {
        self.info = info;
        self
    }

    pub fn status(mut self, status: impl Into<String>) -> Self {
        self.status = Some(status.into());
        self
    }

    pub fn folded(mut self) -> Self {
        self.expand = false;
        self
    }

    pub fn expand(mut self, expand: bool) -> Self {
        self.expand = expand;
        self
    }

    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    pub fn section(mut self, section: &str) -> Self {
        self.section = section.into();
        self
    }

    pub fn kind(mut self, kind: Kind) -> Self {
        self.kind = kind;
        self
    }

    pub fn item(mut self, item: &Value) -> Self {
        self.item = Some(item.clone());
        self
    }

    pub fn add(&mut self, row: Row) -> &mut Row {
        self.children.push(row);
        self.children.last_mut().expect("just pushed")
    }

    /// The details the panel shows: its info lines, else its own text, else its name.
    pub fn details(&self) -> Vec<Line<'static>> {
        match (&self.info, &self.body) {
            (info, _) if !info.is_empty() => info.clone(),
            (_, Body::Text(line)) => vec![line.clone()],
            _ => vec![Line::from(self.name.clone())],
        }
    }

    /// What a filter matches against: the text a row shows, its cells joined by single spaces.
    pub fn haystack(&self) -> String {
        match &self.body {
            Body::Text(line) => line.spans.iter().map(|s| s.content.as_ref()).collect(),
            Body::Cells(cells) => format!("{} {} {}", cells.reference.0, cells.title.0, cells.notes.0),
        }
    }

    /// The row in `room` cells: narrow panes and deep rows give up ref cells first, so the title keeps some room and
    /// the age stays in view; a request in flight, or its outcome, takes the notes' place. `hits` are the haystack's
    /// characters a filter matched, sorted.
    pub fn line(&self, room: usize, requests: &Requests, hits: &[usize]) -> Line<'static> {
        let cells = match &self.body {
            Body::Text(line) => {
                let mut spans = Vec::new();
                let mut at = 0;
                for span in &line.spans {
                    spans.extend(marked(&span.content, &span.content, at, hits, span.style));
                    at += span.content.chars().count();
                }
                // An alert's extend or stop has no notes cell: its progress follows the text.
                if let Some((text, style)) = self.progress(requests) {
                    spans.push(Span::styled(format!("   {text}"), style));
                }
                return Line::from(spans).style(line.style);
            }
            Body::Cells(cells) => cells,
        };
        let mut spans: Vec<Span<'static>> = cells.lead.iter().map(|(g, st)| Span::styled(g.clone(), *st)).collect();
        let lead: usize = cells.lead.iter().map(|(g, _)| width(g)).sum();
        spans.push(Span::raw(" ".repeat(LEAD.saturating_sub(lead))));
        let avail = room as i64 - (LEAD + NOTES + AGE + 1) as i64;
        let ref_room = (REF as i64).min(10.max(avail.div_euclid(2))) as usize;
        let (reference, title) = (&cells.reference.0, &cells.title.0);
        let title_at = reference.chars().count() + 1;
        spans.extend(marked(&format!("{} ", fit(reference, ref_room - 1)), reference, 0, hits, cells.reference.1));
        let (mut notes, mut age) = (cells.notes.clone(), cells.age.clone());
        let mut notes_hits = hits;
        if let Some(progress) = self.progress(requests) {
            (notes, age, notes_hits) = (progress, String::new(), &[]);
        }
        let title_room = 4.max(avail - ref_room as i64) as usize;
        spans.extend(marked(&format!("{} ", fit(title, title_room)), title, title_at, hits, cells.title.1));
        let notes_at = title_at + title.chars().count() + 1;
        let notes_text = fit(&notes.0, NOTES + if age.is_empty() { AGE } else { 0 });
        spans.extend(marked(&notes_text, &notes.0, notes_at, notes_hits, notes.1));
        if !age.is_empty() {
            spans.push(Span::styled(fit(&format!("{age:>AGE$}"), AGE), fg(DIM)));
        }
        Line::from(spans)
    }

    /// The working text of a request this row filed, or its outcome for a few seconds after.
    fn progress(&self, requests: &Requests) -> Option<(String, Style)> {
        let status = self.status.as_deref()?;
        if let Some(job) = requests.working(status) {
            return Some((format!("{} {}", icon("refresh"), job.working), fg(ORANGE)));
        }
        let outcome = requests.outcome.get(status)?;
        Some((outcome.message.clone(), fg(if outcome.ok { GREEN } else { RED })))
    }
}
