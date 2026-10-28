//! The frame: the header, a tab per runbook with its progress (or its matches while filtering every runbook), the steps
//! and notes, the selected step's panel unless compact, the filter bar, the footer, and the editor over them.

use cockpit_core::acts;
use cockpit_core::anim::Glide;
use cockpit_core::files::Density;
use cockpit_core::text::{width, wrap};
use cockpit_core::theme::{DIM, FG, GREEN, LINE, ORANGE, PANEL, base, bold, dim, fg};
use cockpit_core::widgets::filter_line;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Wrap};
use tachyonfx::ColorSpace;

use crate::app::{App, Cmd};
use crate::pinned::ICON;
use crate::steps::button_hits;

const BAR: usize = 32;
const NOTES_MAX: usize = 6;
/// Behind a note's `code`, the tint Textual's Markdown gives it.
const CODE_BG: ratatui::style::Color = ratatui::style::Color::Rgb(35, 30, 26);

impl App {
    pub fn draw(&mut self, frame: &mut Frame) {
        self.take_replies();
        let area = frame.area();
        frame.render_widget(Block::new().style(base()), area);
        self.hits = Default::default();
        let (info, cmds) = if self.density == Density::Compact { (Vec::new(), Vec::new()) } else { self.panel(area.width.saturating_sub(2) as usize) };
        let panel_h = if self.density == Density::Compact { 0 } else { (1 + info.len() + if cmds.is_empty() { 1 } else { 2 }).min(12) as u16 };
        let bar_h = u16::from(self.filter.is_on());
        let [head, tabs, pane, panel, bar, footer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Min(3),
            Constraint::Length(panel_h),
            Constraint::Length(bar_h),
            Constraint::Length(1),
        ])
        .areas(area);
        self.draw_head(frame, head);
        self.draw_tabs(frame, tabs);
        self.draw_pane(frame, pane.inner(ratatui::layout::Margin::new(1, 0)));
        self.draw_panel(frame, panel, info, &cmds);
        self.draw_filter(frame, bar);
        self.draw_footer(frame, footer);
        if let Some(editor) = &self.editor {
            editor.render(frame, Rect { height: area.height.saturating_sub(1), ..area });
        }
        self.toasts.render(frame, Rect { height: area.height.saturating_sub(1), ..area });
        if self.palette.is_open {
            self.palette.render(frame, area);
        }
        if let Some((dialog, _)) = &self.dialog {
            dialog.render(frame, area);
        }
    }

    fn draw_head(&self, frame: &mut Frame, area: Rect) {
        let left: usize = self.books.iter().map(|b| b.todo()).sum();
        let mut spans = vec![
            Span::styled(format!(" {ICON}  "), fg(ORANGE)),
            Span::styled("Runbooks", bold(ORANGE)),
            Span::styled(format!("   {} · {left} steps left", self.books.len()), dim()),
        ];
        if !self.picked.is_empty() {
            spans.push(Span::styled(format!("   ● {} picked", self.picked.len()), bold(ORANGE)));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    /// The tab labels, scrolled so the active one shows, and under them a track whose highlight glides to it.
    fn draw_tabs(&mut self, frame: &mut Frame, area: Rect) {
        if area.height < 2 {
            return;
        }
        let mut x = 0;
        let mut spans = Vec::new();
        let mut tabs = Vec::new();
        let matches = self.found.as_ref().filter(|_| self.filter.global);
        for (i, book) in self.books.iter().enumerate() {
            let todo = book.todo();
            // While every runbook is filtered, each tab counts its matches instead of its steps to do.
            let found = matches.map(|m| m.get(&book.slug).map_or(0, |h| h.len()));
            let count = match found {
                Some(0) => Span::styled(" /0", dim()),
                Some(found) => Span::styled(format!(" /{found}"), bold(ORANGE)),
                None if todo > 0 => Span::styled(format!(" {todo}"), fg(ORANGE)),
                None => Span::styled(" ✓", fg(GREEN)),
            };
            let name = book.slug.to_uppercase();
            let label = if i == self.active { bold(FG) } else if found == Some(0) { Style::new().fg(LINE) } else { dim() };
            let w = width(&name) + width(&count.content) + 2;
            spans.extend([Span::raw(" "), Span::styled(name, label), count, Span::raw(" ")]);
            tabs.push((x, w));
            x += w;
        }
        let (start, w) = tabs.get(self.active).copied().unwrap_or_default();
        let shift = (start + w).saturating_sub(area.width as usize).min(start);
        for (i, &(start, w)) in tabs.iter().enumerate() {
            if let Some(x) = start.checked_sub(shift).filter(|&x| x < area.width as usize) {
                let w = w.min(area.width as usize - x) as u16;
                self.hits.tabs.push((Rect { x: area.x + x as u16, y: area.y, width: w, height: 2 }, i));
            }
        }
        // Under the label, its padding left out.
        let active = ((start + 1 - shift) as f32, (start + w).saturating_sub(shift + 1) as f32);
        frame.render_widget(Paragraph::new(Line::from(spans)).scroll((0, shift as u16)), Rect { height: 1, ..area });
        let (start, end) = &mut self.underline;
        if end.target() == 0.0 {
            start.set(active.0);
            end.set(active.1);
        }
        start.go(active.0);
        end.go(active.1);
        let (from, to) = (start.now().round() as u16, end.now().round() as u16);
        let buf = frame.buffer_mut();
        let any = !self.books.is_empty();
        for dx in 0..area.width {
            let lit = any && dx >= from && dx < to;
            // Half cells on either side of the highlight, as Textual's tabs end it.
            let symbol = if any && dx + 1 == from {
                "╸"
            } else if any && dx == to {
                "╺"
            } else {
                "━"
            };
            buf[(area.x + dx, area.y + 1)].set_symbol(symbol).set_fg(if lit { ORANGE } else { LINE });
        }
    }

    fn draw_pane(&mut self, frame: &mut Frame, area: Rect) {
        let Some(book) = self.book().cloned() else { return };
        let todo = book.todo();
        let notes: Vec<Line> = book
            .notes
            .iter()
            .flat_map(|note| {
                note_lines(note, (area.width as usize).saturating_sub(2)).into_iter().enumerate().map(|(i, mut spans)| {
                    spans.insert(0, Span::styled(if i == 0 { "• " } else { "  " }, fg(ORANGE)));
                    Line::from(spans)
                })
            })
            .take(NOTES_MAX)
            .collect();
        let notes_h = if notes.is_empty() { 0 } else { notes.len() as u16 + 1 };
        let [sub, gauge, _, list, notes_area] =
            Layout::vertical([Constraint::Length(1), Constraint::Length(1), Constraint::Length(1), Constraint::Min(1), Constraint::Length(notes_h)]).areas(area);

        let refreshing = self.refreshing.get(&book.slug).map(|(_, at)| at.elapsed().as_secs());
        let title = if book.title.is_empty() { book.slug.clone() } else { book.title.clone() };
        let mut spans = vec![
            Span::styled(title, bold(if todo > 0 { ORANGE } else { GREEN })),
            Span::styled(format!("   updated {}", book.updated), dim()),
            match refreshing {
                Some(secs) => Span::styled(format!("   refreshing… {secs}s"), fg(ORANGE)),
                None => Span::styled("   R refreshes", dim()),
            },
        ];
        if refreshing.is_some() {
            spans.extend(throbber(self.started.elapsed().as_secs_f32()));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), sub);

        let total = book.steps.len().max(1) as f32;
        let progress = self.gauges.get(&book.slug).map_or(0.0, Glide::now) / total;
        frame.render_widget(Paragraph::new(gauge_line(progress)), gauge);
        self.draw_steps(frame, list, &book);
        if !notes.is_empty() {
            frame.render_widget(Paragraph::new(notes), Rect { y: notes_area.y + 1, height: notes_h - 1, ..notes_area });
        }
    }

    /// The selected step's lines and its actions; when there is none, a word on how to get one.
    fn panel(&self, cells: usize) -> (Vec<Line<'static>>, Vec<Cmd>) {
        let Some(step) = self.book().and_then(|b| b.steps.get(self.selected)).filter(|_| self.shown().contains(&self.selected)) else {
            let text = if self.books.is_empty() { "No runbook yet: run /runbook in a Claude session working on a ticket." } else { "" };
            return (vec![Line::styled(text, dim())], Vec::new());
        };
        let mut lines = Vec::new();
        let head = format!("#{}  ", step.n);
        for (i, part) in wrap(&format!("{head}{}", step.step), cells).into_iter().enumerate() {
            lines.push(if i == 0 {
                let rest = part.get(head.len()..).unwrap_or_default().to_string();
                Line::from(vec![Span::styled(head.clone(), dim()), Span::styled(rest, bold(FG))])
            } else {
                Line::styled(part, bold(FG))
            });
        }
        for part in wrap(&format!("status {}", step.status), cells) {
            lines.push(match part.strip_prefix("status ") {
                Some(rest) => Line::from(vec![Span::styled("status ", dim()), Span::styled(rest.to_string(), fg(FG))]),
                None => Line::styled(part, fg(FG)),
            });
        }
        if !step.command.is_empty() {
            for part in wrap(&format!("$ {}", step.command), cells) {
                lines.push(match part.strip_prefix("$ ") {
                    Some(rest) => Line::from(vec![Span::styled("$ ", dim()), Span::styled(rest.to_string(), fg(ORANGE))]),
                    None => Line::styled(part, fg(ORANGE)),
                });
            }
        }
        (lines, self.step_cmds())
    }

    fn draw_panel(&mut self, frame: &mut Frame, area: Rect, info: Vec<Line<'static>>, cmds: &[Cmd]) {
        if area.height == 0 {
            return;
        }
        let buf = frame.buffer_mut();
        for dx in 0..area.width {
            buf[(area.x + dx, area.y)].set_symbol("─").set_fg(PANEL);
        }
        let inner = Rect { x: area.x + 1, y: area.y + 1, width: area.width.saturating_sub(2), height: area.height.saturating_sub(1) };
        let info_h = (info.len() as u16).min(inner.height);
        frame.render_widget(Paragraph::new(info).wrap(Wrap { trim: false }), Rect { height: info_h, ..inner });
        let acts: Vec<acts::Act> = cmds.iter().filter_map(|c| if let Cmd::Act { act, .. } = c { Some(act.clone()) } else { None }).collect();
        if acts.is_empty() || inner.height < info_h + 2 {
            return;
        }
        let row = Rect { y: inner.y + info_h + 1, height: 1, ..inner };
        frame.render_widget(Paragraph::new(acts::buttons(&acts)), row);
        self.hits.buttons = button_hits(&acts, row);
    }

    /// The filter bar: what is typed, how many steps match, and whether every runbook is searched.
    fn draw_filter(&self, frame: &mut Frame, area: Rect) {
        if area.height == 0 {
            return;
        }
        let count = |slug: &str| self.found.as_ref().and_then(|f| f.get(slug)).map_or(0, |h| h.len());
        let matched = match (&self.found, self.filter.global) {
            (None, _) => self.book().map_or(0, |b| b.steps.len()),
            (Some(found), true) => found.values().map(|h| h.len()).sum(),
            (Some(_), false) => self.book().map_or(0, |b| count(&b.slug)),
        };
        let mut line = filter_line(&self.filter.input, self.filter.editing, matched);
        line.spans.push(match (self.filter.global, self.filter.editing) {
            (true, true) => Span::styled("   in every runbook · / this one only", fg(ORANGE)),
            (true, false) => Span::styled("   in every runbook", fg(ORANGE)),
            (false, true) => Span::styled("   / every runbook · enter keeps", dim()),
            (false, false) => Span::raw(""),
        });
        frame.render_widget(Paragraph::new(line), Rect { x: area.x + 1, width: area.width.saturating_sub(1), ..area });
    }

    /// The keys on the left, as many as fit, and the palette's on the right; the batch's keys while steps are picked.
    fn draw_footer(&self, frame: &mut Frame, area: Rect) {
        let keys: &[(&str, &str)] = if self.picked.is_empty() {
            &[("q", "quit"), ("/", "filter"), ("v", "pick"), ("e", "edit"), ("a", "add"), ("R", "refresh"), ("t", "talk"), ("p", "prev runbook"), ("n", "next runbook")]
        } else {
            &[("d", "mark done"), ("c", "copy commands"), ("v", "pick"), ("V", "all open"), ("esc", "unpick")]
        };
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

/// A note wrapped to `cells`, its `code` without the backticks and on a tint, as the Markdown of the Textual app shows it.
fn note_lines(note: &str, cells: usize) -> Vec<Vec<Span<'static>>> {
    let mut chars = Vec::new();
    let ticks = note.matches('`').count();
    for (i, part) in note.split('`').enumerate() {
        // A backtick nothing closes stays as it is written.
        let is_code = i % 2 == 1 && ticks > i;
        if i % 2 == 1 && !is_code {
            chars.push(('`', false));
        }
        chars.extend(part.chars().map(|c| (c, is_code)));
    }
    let plain: String = chars.iter().map(|(c, _)| c).collect();
    let mut at = 0;
    wrap(&plain, cells)
        .into_iter()
        .map(|line| {
            let mut spans: Vec<Span<'static>> = Vec::new();
            for c in line.chars() {
                // wrap only drops the spaces where it breaks a line.
                while chars.get(at).is_some_and(|&(p, _)| p != c && p == ' ') {
                    at += 1;
                }
                let is_code = chars.get(at).is_some_and(|&(_, code)| code);
                at += 1;
                let style = if is_code { Style::new().fg(ORANGE).bg(CODE_BG) } else { fg(FG) };
                match spans.last_mut() {
                    Some(last) if last.style == style => last.content.to_mut().push(c),
                    _ => spans.push(Span::styled(c.to_string(), style)),
                }
            }
            spans
        })
        .collect()
}

/// A bar of BAR cells, half cells included and rounded down as Textual's bar is, then the percentage.
fn gauge_line(progress: f32) -> Line<'static> {
    let halves = (progress.clamp(0.0, 1.0) * (BAR * 2) as f32 + 1e-3) as usize;
    let (full, half) = (halves / 2, halves % 2 == 1);
    let mut spans = vec![Span::styled("━".repeat(full), fg(GREEN))];
    if half {
        spans.push(Span::styled("╸", fg(GREEN)));
    }
    let rest = BAR - full - usize::from(half);
    if rest > 0 {
        let lead = if half { "" } else { "╺" };
        spans.push(Span::styled(format!("{lead}{}", "━".repeat(rest - usize::from(!half))), fg(LINE)));
    }
    spans.push(Span::styled(format!("{:>5}", format!("{}%", (progress * 100.0).round() as u32)), fg(FG)));
    Line::from(spans)
}

/// Five dots a light runs through, while a background refresh works.
fn throbber(t: f32) -> Vec<Span<'static>> {
    let mut spans = vec![Span::raw("  ")];
    for i in 0..5 {
        let wave = ((t * 4.0 - i as f32 * 0.7).sin() + 1.0) / 2.0;
        spans.push(Span::styled("●", fg(ColorSpace::Rgb.lerp(&DIM, &ORANGE, wave))));
    }
    spans
}
