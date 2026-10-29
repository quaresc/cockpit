//! The frame: the header with what changed since you last looked, the routines as a list with their runs under them,
//! the selected row's panel (its settings, prompt and last output) or, compact, its buttons on the row itself, the
//! footer, and the form, dialog, toasts and palette over them.

use cockpit_core::acts::{self, Act};
use cockpit_core::files::Density;
use cockpit_core::seen;
use cockpit_core::text::{fit, width, wrap};
use cockpit_core::theme::{DIM, FG, GREEN, LINE, ORANGE, PANEL, RED, base, bold, dim, fg};
use cockpit_core::widgets;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Wrap};

use crate::app::{App, Cmd};
use crate::data::{Doc, Routine, Run, State, dollars, duration, estimate};
use crate::pinned::ICON;
use crate::rows::{Item, Row};

const NAME_MIN: usize = 12;
const NAME_MAX: usize = 28;
const WHEN_MAX: usize = 26;
const LAST: usize = 24;
const COST: usize = 13;
const NEXT: usize = 16;
const PANEL_MAX: usize = 16;
const SPINNER: [&str; 8] = ["⣾", "⣽", "⣻", "⢿", "⡿", "⣟", "⣯", "⣷"];

impl App {
    pub fn draw(&mut self, frame: &mut Frame) {
        self.take_replies();
        let area = frame.area();
        frame.render_widget(Block::new().style(base()), area);
        self.hits = Default::default();
        let inner_w = area.width.saturating_sub(2) as usize;
        let acts = fitting(&self.acts(), inner_w);
        let comfortable = self.density == Density::Comfortable;
        let room = (area.height as usize).saturating_sub(8).min(PANEL_MAX);
        let info = if comfortable { self.info(inner_w, room.saturating_sub(3)) } else { Vec::new() };
        let info_h: usize = info.iter().map(|l| wrap(&plain(l), inner_w).len().max(1)).sum();
        let panel_h = if comfortable && info_h > 0 { (1 + info_h + 2 * usize::from(!acts.is_empty())).min(room.max(4)) as u16 } else { 0 };
        let head = self.head_lines(inner_w);
        let [head_area, list, panel, footer] =
            Layout::vertical([Constraint::Length(head.len() as u16 + 1), Constraint::Min(1), Constraint::Length(panel_h), Constraint::Length(1)]).areas(area);
        frame.render_widget(Paragraph::new(head), inset(head_area));
        self.draw_list(frame, inset(list), &acts);
        if panel_h > 0 {
            self.draw_panel(frame, panel, info, &acts);
        }
        self.draw_footer(frame, footer);
        self.toasts.render(frame, Rect { height: area.height.saturating_sub(1), ..area });
        if let Some(form) = &self.form {
            form.render(frame, area, &self.clock);
        }
        if let Some((dialog, _, _)) = &self.dialog {
            dialog.render(frame, area);
        }
        if self.palette.is_open {
            self.palette.render(frame, area);
        }
    }

    fn head_lines(&self, cells: usize) -> Vec<Line<'static>> {
        let mut spans = vec![Span::styled(format!("{ICON}  "), fg(ORANGE)), Span::styled("Routines", bold(ORANGE))];
        let Some(doc) = &self.doc else {
            spans.push(Span::styled("   waiting for the hub", dim()));
            return vec![Line::from(spans)];
        };
        let mut facts = vec![plural(doc.routines.len(), "routine")];
        if !doc.running.is_empty() {
            facts.push(format!("{} running", doc.running.len()));
        }
        if !doc.queued.is_empty() {
            facts.push(format!("{} waiting", doc.queued.len()));
        }
        let today = doc.runs.values().flatten().filter(|r| self.clock.local(r.started).date() == self.clock.local(self.clock.now).date()).count();
        facts.push(format!("{} today", plural(today, "run")));
        let cost: f64 = doc.runs.values().flatten().filter(|r| self.clock.now.saturating_sub(r.started) < 7 * 86_400_000).filter_map(|r| r.cost).sum();
        if cost > 0.0 {
            facts.push(format!("{} this week", estimate(cost)));
        }
        while facts.len() > 1 && 13 + width(&facts.join(" · ")) > cells {
            facts.pop();
        }
        spans.push(Span::styled(format!("   {}", facts.join(" · ")), dim()));
        let mut lines = vec![Line::from(spans)];
        lines.extend(self.looked.head(cells, |ms| self.clock.when(ms)));
        lines
    }

    fn name_w(&self) -> usize {
        let longest = self.doc.as_ref().map(|d| d.routines.iter().map(|r| width(&r.name)).max().unwrap_or(0)).unwrap_or(0);
        (longest + 2).clamp(NAME_MIN, NAME_MAX)
    }

    fn draw_list(&mut self, frame: &mut Frame, area: Rect, acts: &[(Act, Cmd)]) {
        self.hits.list = area;
        let rows = self.rows();
        if rows.is_empty() {
            let text = if self.doc.is_some() {
                "No routine yet. a adds one, or ask Claude: /routine every weekday at 9, sum up my open PRs."
            } else {
                "No routine yet. a adds one; the hub runs them on their schedule or when something happens."
            };
            frame.render_widget(Paragraph::new(Line::styled(text, dim())).wrap(Wrap { trim: false }), area);
            return;
        }
        let height = area.height as usize;
        let overflow = rows.len() > height;
        let row_w = if overflow { area.width.saturating_sub(1) } else { area.width } as usize;
        let selected = rows.iter().position(|r| Some(&r.key) == self.selected.as_ref());
        if let (true, Some(at)) = (self.follow, selected) {
            if at < self.scroll {
                self.scroll = at;
            } else if at >= self.scroll + height {
                self.scroll = at + 1 - height;
            }
        }
        self.scroll = self.scroll.min(rows.len().saturating_sub(height));
        let compact = self.density == Density::Compact;
        let buttons = if compact { fitting(acts, row_w / 2) } else { Vec::new() };
        let buttons_w = width(&plain(&button_line(&buttons)));
        for (i, row) in rows.iter().enumerate().skip(self.scroll).take(height) {
            let rect = Rect { x: area.x, y: area.y + (i - self.scroll) as u16, width: row_w as u16, height: 1 };
            let is_selected = Some(i) == selected;
            let room = if is_selected && buttons_w > 0 { row_w.saturating_sub(buttons_w + 1) } else { row_w };
            let line = self.row_line(row, room);
            frame.render_widget(Paragraph::new(line), Rect { width: room as u16, ..rect });
            if is_selected {
                frame.buffer_mut().set_style(rect, Style::new().bg(PANEL));
            }
            let glyph = Rect { x: rect.x + (row.depth * 2) as u16, width: 2, ..rect };
            self.hits.rows.push((rect, row.key.clone(), glyph));
            if is_selected && buttons_w > 0 {
                let bar = Rect { x: rect.x + rect.width - buttons_w as u16, width: buttons_w as u16, ..rect };
                frame.render_widget(Paragraph::new(button_line(&buttons)), bar);
                self.hits.buttons.extend(button_hits(&buttons, bar));
            }
        }
        if overflow {
            widgets::scrollbar(frame, area, self.scroll, rows.len(), height);
        }
    }

    fn row_line(&self, row: &Row, room: usize) -> Line<'static> {
        let Some(doc) = &self.doc else { return Line::default() };
        let Some(r) = doc.routine(row.item.slug()) else { return Line::default() };
        let fold = match row.fold {
            Some(true) => "▼ ",
            Some(false) => "▶ ",
            None => "  ",
        };
        let mut spans = vec![Span::styled(if row.depth == 0 { fold.to_string() } else { format!("  {}", row.guide) }, fg(LINE))];
        if self.is_marked(row) {
            spans.push(seen::mark());
        }
        let room = room.saturating_sub(width(&plain(&Line::from(spans.clone()))));
        match &row.item {
            Item::Routine(_) => spans.extend(self.routine_cells(doc, r, room)),
            Item::Run(slug, id) => {
                let Some(run) = doc.run(slug, id) else { return Line::from(spans) };
                spans.extend(self.run_cells(run, room));
            }
        }
        Line::from(spans)
    }

    /// The glyph of a routine's state, moving while it runs.
    fn glyph(&self, state: State) -> (String, Color) {
        match state {
            State::Running => (SPINNER[(self.clock.now / 125) as usize % SPINNER.len()].to_string(), ORANGE),
            State::Waiting => ("\u{f252}".into(), ORANGE),
            State::Broken => ("\u{f071}".into(), RED),
            State::Over => ("\u{f155}".into(), ORANGE),
            State::Paused => ("\u{f04c}".into(), DIM),
            State::Failed => ("\u{f00d}".into(), RED),
            State::Ok => ("\u{f00c}".into(), GREEN),
            State::Never => ("\u{f10c}".into(), DIM),
        }
    }

    /// A routine's cells: its glyph and name, when it runs, then as room allows its last run and its next one.
    fn routine_cells(&self, doc: &Doc, r: &Routine, room: usize) -> Vec<Span<'static>> {
        let state = doc.state(r);
        let (glyph, color) = self.glyph(state);
        let name_w = self.name_w();
        let when_w = WHEN_MAX.min(room.saturating_sub(2 + name_w));
        let name_style = if r.paused { dim() } else { bold(FG) };
        let mut spans = vec![Span::styled(format!("{glyph} "), fg(color)), Span::styled(cell(&r.name, name_w), name_style)];
        let when_style = if r.error.is_some() { fg(RED) } else { dim() };
        let when = if r.error.is_some() { "when?".to_string() } else { r.when.clone() };
        spans.push(Span::styled(cell(&when, when_w), when_style));
        let mut left = room.saturating_sub(2 + name_w + when_w);
        if left >= COST + LAST {
            let cost = r.last_cost.map(|c| format!("{} last", estimate(c))).unwrap_or_default();
            spans.push(Span::styled(cell(&cost, COST), if r.over { fg(ORANGE) } else { dim() }));
            left -= COST;
        }
        let (last, last_style) = match (doc.running.get(&r.slug), doc.last(&r.slug)) {
            (None, _) if r.over && !r.paused => ("over its weekly budget".into(), fg(ORANGE)),
            (Some(run), _) => (format!("running {}", duration(self.clock.now.saturating_sub(run.started))), fg(ORANGE)),
            (None, _) if doc.queued.contains(&r.slug) => ("waiting its turn".into(), fg(ORANGE)),
            (None, Some(run)) => (format!("{} {}", if run.ok { "ran" } else { "failed" }, self.clock.ago(run.started)), if run.ok { dim() } else { fg(RED) }),
            (None, None) => ("never ran".into(), dim()),
        };
        if left >= LAST {
            spans.push(Span::styled(cell(&last, LAST), last_style));
            left -= LAST;
        }
        let next = match (r.paused, r.next, r.trigger.as_str()) {
            (true, _, _) => "paused".to_string(),
            (_, Some(at), _) => format!("next {}", self.clock.until(at)),
            (_, None, "event") => "on its event".into(),
            _ => String::new(),
        };
        if left >= NEXT {
            spans.push(Span::styled(cell(&next, NEXT), dim()));
            left -= NEXT;
        }
        if let Some(run) = doc.last(&r.slug).filter(|_| left > 8 && !doc.running.contains_key(&r.slug)) {
            spans.push(Span::styled(fit(&run.summary, left), if run.ok { fg(FG) } else { fg(RED) }));
        }
        spans
    }

    /// A run's cells: how it ended, when, how long, its cost, why it ran and what it said.
    fn run_cells(&self, run: &Run, room: usize) -> Vec<Span<'static>> {
        let (glyph, color) = if run.ok { ("\u{f00c}", GREEN) } else if run.stopped { ("\u{f04d}", DIM) } else { ("\u{f00d}", RED) };
        let when = self.clock.when(run.started);
        let cost = run.cost.map(estimate).unwrap_or_default();
        let facts = format!("{:<12}{:<8}{:<9}", when, run.took(), cost);
        let used = 2 + width(&facts);
        let rest = room.saturating_sub(used);
        let text = if run.why.is_empty() { run.summary.clone() } else { format!("{} · {}", run.summary, run.why) };
        vec![Span::styled(format!("{glyph} "), fg(color)), Span::styled(facts, dim()), Span::styled(fit(&text, rest), fg(FG))]
    }

    /// The selected row's details for the panel: settings, prompt and the output of the run in question, `room` lines
    /// at most.
    fn info(&mut self, cells: usize, room: usize) -> Vec<Line<'static>> {
        let Some(row) = self.selected_row() else { return Vec::new() };
        let Some(doc) = self.doc.clone() else { return Vec::new() };
        let Some(r) = doc.routine(row.item.slug()).cloned() else { return Vec::new() };
        let mut lines = Vec::new();
        let run = match &row.item {
            Item::Routine(_) => {
                let mut title = vec![Span::styled(r.name.clone(), bold(FG)), Span::styled(format!("  ·  {}", r.when), dim())];
                if let Some(at) = r.next.filter(|_| !r.paused) {
                    title.push(Span::styled(format!("  ·  next {}", self.clock.until(at)), dim()));
                }
                if r.paused {
                    title.push(Span::styled("  ·  paused", fg(ORANGE)));
                }
                lines.push(Line::from(title));
                if let Some(err) = &r.error {
                    lines.push(Line::styled(format!("when \"{}\": {err}", r.when_text), fg(RED)));
                }
                lines.push(Line::styled(settings(&r), dim()));
                let first = r.prompt.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default().to_string();
                lines.push(Line::styled(fit(&first, cells), fg(FG)));
                if let Some(running) = doc.running.get(&r.slug) {
                    let how = format!("running for {} · {}", duration(self.clock.now.saturating_sub(running.started)), running.why);
                    lines.push(Line::styled(how, fg(ORANGE)));
                    None
                } else {
                    doc.last(&r.slug).cloned()
                }
            }
            Item::Run(slug, id) => doc.run(slug, id).cloned(),
        };
        if let Some(run) = run {
            let (word, color) = if run.ok { ("ran", GREEN) } else if run.stopped { ("stopped", DIM) } else { ("failed", RED) };
            let mut facts = vec![format!("{word} {}", self.clock.ago(run.started)), format!("took {}", run.took())];
            if let Some(cost) = run.cost {
                facts.push(estimate(cost));
            }
            if let Some(code) = run.code.filter(|c| *c != 0) {
                facts.push(format!("exit {code}"));
            }
            facts.push(run.why.clone());
            lines.push(Line::styled(facts.join(" · "), fg(color)));
            let left = room.saturating_sub(lines.len());
            let output = self.output(&r.slug, &run.id);
            let mut shown = Vec::new();
            for line in output.iter().filter(|l| !l.trim().is_empty()) {
                for part in wrap(line, cells) {
                    shown.push(part);
                }
            }
            if shown.is_empty() {
                shown.push(run.summary.clone());
            }
            let more = shown.len() > left;
            for part in shown.iter().take(if more { left.saturating_sub(1) } else { left }) {
                lines.push(Line::styled(part.clone(), fg(FG)));
            }
            if more {
                lines.push(Line::styled(format!("… {} more lines · o opens it all", shown.len() + 1 - left), dim()));
            }
        }
        lines.truncate(room.max(1));
        lines
    }

    fn draw_panel(&mut self, frame: &mut Frame, area: Rect, info: Vec<Line<'static>>, acts: &[(Act, Cmd)]) {
        frame.render_widget(Paragraph::new("─".repeat(area.width as usize)).style(fg(PANEL)), Rect { height: 1, ..area });
        let body = inset(Rect { y: area.y + 1, height: area.height.saturating_sub(1), ..area });
        let text_h = body.height.saturating_sub(if acts.is_empty() { 0 } else { 2 });
        frame.render_widget(Paragraph::new(info).wrap(Wrap { trim: false }), Rect { height: text_h, ..body });
        if !acts.is_empty() && body.height >= 1 {
            let bar = Rect { y: body.y + body.height - 1, height: 1, ..body };
            frame.render_widget(Paragraph::new(button_line(acts)), bar);
            self.hits.buttons.extend(button_hits(acts, bar));
        }
    }

    fn draw_footer(&self, frame: &mut Frame, area: Rect) {
        let keys = [("enter", "main"), ("a", "add"), ("space", "runs"), ("R", "refresh"), ("q", "quit")];
        let mut spans = Vec::new();
        for (key, what) in keys {
            spans.push(Span::styled(format!(" {key}"), bold(ORANGE)));
            spans.push(Span::styled(format!(" {what} "), fg(FG)));
        }
        let palette = vec![Span::styled("▏", fg(LINE)), Span::styled("^p", bold(ORANGE)), Span::styled(" palette ", fg(FG))];
        let used = |spans: &[Span]| -> usize { spans.iter().chain(&palette).map(|s| width(&s.content)).sum() };
        while !spans.is_empty() && used(&spans) > area.width as usize {
            spans.truncate(spans.len() - 2);
        }
        spans.push(Span::raw(" ".repeat((area.width as usize).saturating_sub(used(&spans)))));
        spans.extend(palette);
        frame.render_widget(Paragraph::new(Line::from(spans)).style(Style::new().bg(PANEL)), area);
    }
}

/// "Claude · sonnet · acceptEdits · in ~/work · Read, Grep · ≤ $0.50", "shell · in ~/notes".
fn settings(r: &Routine) -> String {
    let home = cockpit_core::files::home().to_string_lossy().into_owned();
    let cwd = match (r.cwd.is_empty(), r.run == "shell") {
        (false, _) => format!("in {}", r.cwd.replacen(&home, "~", 1)),
        (true, true) => "in the repos folder".to_string(),
        (true, false) => "in a folder of its own".to_string(),
    };
    let mut parts = vec![if r.run == "shell" { "shell".to_string() } else { "Claude".to_string() }];
    if r.run != "shell" {
        let model = match r.model.to_lowercase().as_str() {
            "" => "default model".to_string(),
            "lightest" | "light" => "lightest (haiku, nothing but its tools)".to_string(),
            other => other.to_string(),
        };
        parts.extend([model, r.mode.clone()].into_iter().filter(|p| !p.is_empty()));
    }
    parts.push(cwd);
    if !r.tools.is_empty() {
        parts.push(r.tools.join(", "));
    }
    match (r.budget, r.spent_week > 0.0) {
        (Some(budget), _) => parts.push(format!("{} of its {} a week", estimate(r.spent_week), dollars(budget))),
        (None, true) => parts.push(format!("{} this week", estimate(r.spent_week))),
        _ => {}
    }
    if r.notify != "always" && !r.notify.is_empty() {
        parts.push(format!("notifies {}", if r.notify == "failed" { "on failure" } else { "never" }));
    }
    parts.join(" · ")
}

fn cell(text: &str, cells: usize) -> String {
    if cells == 0 { String::new() } else { format!("{} ", fit(text, cells - 1)) }
}

fn plural(n: usize, one: &str) -> String {
    format!("{n} {one}{}", if n == 1 { "" } else { "s" })
}

fn inset(area: Rect) -> Rect {
    Rect { x: area.x + 1, width: area.width.saturating_sub(2), ..area }
}

fn plain(line: &Line) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

fn button_line(acts: &[(Act, Cmd)]) -> Line<'static> {
    let acts: Vec<Act> = acts.iter().map(|(a, _)| a.clone()).collect();
    let mut line = acts::buttons(&acts);
    line.spans.pop();
    line
}

/// The actions that fit in `cells`, the least used ones left out first: they keep their keys and palette entries.
fn fitting(acts: &[(Act, Cmd)], cells: usize) -> Vec<(Act, Cmd)> {
    const DROPPED_FIRST: [&str; 5] = ["delete", "model", "pause", "talk", "edit"];
    let fits = |acts: &[(Act, Cmd)]| width(&plain(&button_line(acts))) <= cells;
    let mut kept = acts.to_vec();
    for id in DROPPED_FIRST {
        if fits(&kept) {
            break;
        }
        kept.retain(|(a, _)| a.id != id);
    }
    while !fits(&kept) && kept.len() > 1 {
        kept.pop();
    }
    if fits(&kept) { kept } else { Vec::new() }
}

/// Each button's cells in this bar, with what it runs.
fn button_hits(acts: &[(Act, Cmd)], bar: Rect) -> Vec<(Rect, Cmd)> {
    let mut x = bar.x;
    acts.iter()
        .map(|(act, cmd)| {
            let key = act.key.map(|k| format!("  {k}")).unwrap_or_default();
            let w = width(&format!("  {}{key}  ", act.label)) as u16;
            let rect = Rect { x, width: w, ..bar };
            x += w + 1;
            (rect, cmd.clone())
        })
        .collect()
}
