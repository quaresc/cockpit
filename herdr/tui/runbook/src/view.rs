//! The frame: the header, a tab per runbook with its progress, steps and notes, the selected step's panel, the footer.

use cockpit_core::acts;
use cockpit_core::text::{width, wrap};
use cockpit_core::theme::{BG, DIM, FG, GREEN, LINE, ORANGE, PANEL, base, bold, dim, fg};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap};
use tachyonfx::ColorSpace;

use crate::anim::{Glide, Phase};
use crate::app::{App, Cmd};
use crate::pinned::ICON;
use crate::rows::{self, Widths};

const BAR: usize = 32;
const NOTES_MAX: usize = 6;
/// The columns a long list gives up on its right: the bar and a gap, as wide as Textual's, for the same column widths.
const SCROLLBAR: u16 = 2;
/// Behind a note's `code`, the tint Textual's Markdown gives it.
const CODE_BG: ratatui::style::Color = ratatui::style::Color::Rgb(35, 30, 26);

impl App {
    pub fn draw(&mut self, frame: &mut Frame) {
        self.take_replies();
        let area = frame.area();
        frame.render_widget(Block::new().style(base()), area);
        self.hits = Default::default();
        let (info, cmds) = self.panel(area.width.saturating_sub(2) as usize);
        let panel_h = (1 + info.len() + if cmds.is_empty() { 1 } else { 2 }).min(12) as u16;
        let [head, tabs, pane, panel, footer] =
            Layout::vertical([Constraint::Length(1), Constraint::Length(2), Constraint::Min(3), Constraint::Length(panel_h), Constraint::Length(1)]).areas(area);
        self.draw_head(frame, head);
        self.draw_tabs(frame, tabs);
        self.draw_pane(frame, pane.inner(ratatui::layout::Margin::new(1, 0)));
        self.draw_panel(frame, panel, info, &cmds);
        draw_footer(frame, footer);
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
        let line = Line::from(vec![
            Span::styled(format!(" {ICON}  "), fg(ORANGE)),
            Span::styled("Runbooks", bold(ORANGE)),
            Span::styled(format!("   {} · {left} steps left", self.books.len()), dim()),
        ]);
        frame.render_widget(Paragraph::new(line), area);
    }

    /// The tab labels, scrolled so the active one shows, and under them a track whose highlight glides to it.
    fn draw_tabs(&mut self, frame: &mut Frame, area: Rect) {
        if area.height < 2 {
            return;
        }
        let mut x = 0;
        let mut spans = Vec::new();
        let mut tabs = Vec::new();
        for (i, book) in self.books.iter().enumerate() {
            let todo = book.todo();
            let count = if todo > 0 { Span::styled(format!(" {todo}"), fg(ORANGE)) } else { Span::styled(" ✓", fg(GREEN)) };
            let name = book.slug.to_uppercase();
            let label = if i == self.active { bold(FG) } else { dim() };
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

    /// The steps, scrolled; the selected row on the panel colour, the rows turning done painted by their animation.
    fn draw_steps(&mut self, frame: &mut Frame, area: Rect, book: &crate::book::Book) {
        self.hits.list = area;
        let build = |app: &App, w: usize| {
            let widths = Widths::new(book, w);
            let mut lines = vec![rows::header(&widths), Line::raw("")];
            let mut spans = Vec::new();
            for step in &book.steps {
                let phase = app.fx.get(&(book.slug.clone(), step.n.clone())).map_or(Phase::Still, |fx| fx.phase());
                let row = rows::row(book, step, &widths, phase, &app.handoffs, &app.inbox);
                spans.push((lines.len(), row.len() + 1));
                lines.extend(row);
                lines.push(Line::raw(""));
            }
            (lines, spans)
        };
        let (mut lines, mut spans) = build(self, area.width as usize);
        let overflow = lines.len() > area.height as usize;
        if overflow {
            (lines, spans) = build(self, area.width.saturating_sub(SCROLLBAR) as usize);
        }
        let (total, height) = (lines.len() as f32, area.height as f32);
        let scroll = self.scroll.entry(book.slug.clone()).or_insert_with(|| Glide::new(0.0, 0.25));
        let reveal = self.reveal.take().and_then(|(first, last)| Some((*spans.get(first)?, *spans.get(last)?)));
        if let Some(((top, _), (last, last_h))) = reveal {
            let (top, h, now) = (top as f32, (last + last_h).saturating_sub(top) as f32, scroll.target());
            if top < now || h > height {
                scroll.go(top);
            } else if top + h > now + height {
                scroll.go(top + h - height);
            }
        }
        let most = (total - height).max(0.0);
        if scroll.target() > most || scroll.target() < 0.0 {
            scroll.go(scroll.target().clamp(0.0, most));
        }
        let offset = scroll.now().round().clamp(0.0, most) as usize;
        let content = Rect { width: if overflow { area.width.saturating_sub(SCROLLBAR) } else { area.width }, ..area };
        frame.render_widget(Paragraph::new(lines).scroll((offset as u16, 0)), content);
        let buf = frame.buffer_mut();
        for (i, &(top, h)) in spans.iter().enumerate() {
            let rect = visible(content, top, h, offset);
            if let Some(rect) = rect {
                self.hits.rows.push((rect, i));
                if i == self.selected {
                    buf.set_style(rect, Style::new().bg(PANEL));
                }
            }
            if let Some(fx) = self.fx.get_mut(&(book.slug.clone(), book.steps[i].n.clone())) {
                fx.paint(buf, rect.unwrap_or_default());
            }
        }
        if overflow {
            let mut state = ScrollbarState::new(lines_len(total, height)).position(offset).viewport_content_length(area.height as usize);
            let bar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .track_symbol(Some(" "))
                .thumb_symbol("▐")
                .track_style(Style::new().bg(BG))
                .thumb_style(fg(LINE));
            frame.render_stateful_widget(bar, area, &mut state);
        }
    }

    /// The selected step's lines and its actions; when there is none, a word on how to get one.
    fn panel(&self, cells: usize) -> (Vec<Line<'static>>, Vec<Cmd>) {
        let Some(step) = self.book().and_then(|b| b.steps.get(self.selected)) else {
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
        let mut x = row.x;
        for (i, act) in acts.iter().enumerate() {
            let key = act.key.map(|k| format!("  {k}")).unwrap_or_default();
            let w = width(&format!(" {}{key} ", act.label)) as u16;
            self.hits.buttons.push((Rect { x, y: row.y, width: w, height: 1 }, i));
            x += w + 1;
        }
    }
}

fn lines_len(total: f32, height: f32) -> usize {
    (total - height).max(0.0) as usize + 1
}

/// The part of a row, `top` lines into the list, that shows with the list scrolled by `offset`.
fn visible(area: Rect, top: usize, height: usize, offset: usize) -> Option<Rect> {
    let start = top.max(offset);
    let end = (top + height).min(offset + area.height as usize);
    (start < end).then(|| Rect { x: area.x, y: area.y + (start - offset) as u16, width: area.width, height: (end - start) as u16 })
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

/// The keys on the left, as many as fit, and the palette's on the right.
fn draw_footer(frame: &mut Frame, area: Rect) {
    let keys = [("q", "quit"), ("R", "refresh"), ("t", "talk"), ("p", "prev runbook"), ("n", "next runbook")];
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

