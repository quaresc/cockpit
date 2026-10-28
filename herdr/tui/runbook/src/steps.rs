//! The step list: the rows the filter lets through, scrolled; the cursor's row on the panel colour, picked rows tinted,
//! status animations painted over their rows, and in compact density the cursor row's actions at its end.

use cockpit_core::acts;
use cockpit_core::anim::Kind;
use cockpit_core::files::Density;
use cockpit_core::text::width;
use cockpit_core::theme::{BG, LINE, PANEL, dim, fg};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState};

use crate::anim::Phase;
use crate::app::{App, Cmd, key};
use crate::book::Book;
use crate::rows::{self, Marks, Widths};

/// The columns a long list gives up on its right: the bar and a gap, as wide as Textual's, for the same column widths.
const SCROLLBAR: u16 = 2;
/// Behind a picked step's row.
const PICKED: Color = Color::Rgb(38, 32, 26);

/// A row's place in the list: its step's index, its first line and how many lines it takes.
struct Placed {
    step: usize,
    top: usize,
    height: usize,
}

impl App {
    /// The rows as lines at this width, and where each one is.
    fn layout(&self, book: &Book, cells: usize) -> (Vec<Line<'static>>, Vec<Placed>) {
        let compact = self.density == Density::Compact;
        let now = self.clock.now();
        let widths = Widths::new(book, cells);
        let mut lines = vec![rows::header(&widths)];
        if !compact {
            lines.push(Line::raw(""));
        }
        let mut placed = Vec::new();
        for i in self.shown() {
            let step = &book.steps[i];
            let k = key(&book.slug, &step.n);
            let marks = Marks { phase: self.fx.get(&k).map_or(Phase::Still, |fx| fx.phase(now)), picked: self.picked.contains(&k), hit: self.hit(&book.slug, &step.n) };
            let row = rows::row(book, step, &widths, marks, &self.handoffs, &self.inbox);
            // Comfortable rows keep a blank line under them, which their highlight covers.
            placed.push(Placed { step: i, top: lines.len(), height: row.len() + usize::from(!compact) });
            lines.extend(row);
            if !compact {
                lines.push(Line::raw(""));
            }
        }
        (lines, placed)
    }

    pub fn draw_steps(&mut self, frame: &mut Frame, area: Rect, book: &Book) {
        self.hits.list = area;
        if self.found.is_some() && self.shown().is_empty() {
            let line = Line::styled(format!("no step of {} matches “{}”", book.slug.to_uppercase(), self.filter.query()), dim());
            frame.render_widget(Paragraph::new(line), area);
            return;
        }
        let (mut lines, mut placed) = self.layout(book, area.width as usize);
        let overflow = lines.len() > area.height as usize;
        if overflow {
            (lines, placed) = self.layout(book, area.width.saturating_sub(SCROLLBAR) as usize);
        }
        let (total, height) = (lines.len() as f32, area.height as f32);
        let scroll = self.scroll.entry(book.slug.clone()).or_insert_with(|| cockpit_core::anim::Glide::new(0.0, 0.25));
        let find = |step: usize| placed.iter().find(|p| p.step == step);
        let reveal = self.reveal.take().and_then(|(first, last)| Some((find(first)?, find(last)?)));
        if let Some((first, last)) = reveal {
            let (top, h, now) = (first.top as f32, (last.top + last.height).saturating_sub(first.top) as f32, scroll.target());
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
        let now = self.clock.now();
        let mut cursor = None;
        for p in &placed {
            let Some(rect) = visible(content, p.top, p.height, offset) else { continue };
            let k = key(&book.slug, &book.steps[p.step].n);
            self.hits.rows.push((rect, p.step));
            let buf = frame.buffer_mut();
            if p.step == self.selected {
                buf.set_style(rect, Style::new().bg(PANEL));
                cursor = Some(rect);
            } else if self.picked.contains(&k) {
                buf.set_style(rect, Style::new().bg(PICKED));
            }
            if self.fx.get(&k).is_some_and(|fx| fx.wants_flash(now)) {
                self.flash(&k, Kind::GOOD);
                if let Some(fx) = self.fx.get_mut(&k) {
                    fx.flashed = true;
                }
            }
            self.effects.paint(&k, frame.buffer_mut(), rect);
        }
        if let (Density::Compact, Some(rect)) = (self.density, cursor) {
            let widths = Widths::new(book, content.width as usize);
            let keep = (widths.n + widths.status) as u16;
            self.draw_row_buttons(frame, Rect { height: 1, ..rect }, keep);
        }
        if overflow {
            let mut state = ScrollbarState::new((total - height).max(0.0) as usize + 1).position(offset).viewport_content_length(area.height as usize);
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

    /// The cursor row's actions over the end of its first line, as the light runbook shows them; its first `keep`
    /// columns, number and status, stay in sight, the actions that do not fit left out from the last.
    fn draw_row_buttons(&mut self, frame: &mut Frame, row: Rect, keep: u16) {
        let mut acts: Vec<acts::Act> = self.step_cmds().into_iter().filter_map(|c| if let Cmd::Act { act, .. } = c { Some(act) } else { None }).collect();
        while acts.len() > 1 && acts::buttons(&acts).width() as u16 > row.width.saturating_sub(keep) {
            acts.pop();
        }
        if acts.is_empty() {
            return;
        }
        let line = acts::buttons(&acts);
        let w = (line.width() as u16).min(row.width);
        let x = row.x + row.width - w;
        // A cell of the row's colour before the first button, so the text it covers ends cleanly.
        let gap = Rect { x: x.saturating_sub(1).max(row.x), width: x - x.saturating_sub(1).max(row.x), ..row };
        frame.buffer_mut().set_string(gap.x, gap.y, " ".repeat(gap.width as usize), Style::new().bg(PANEL));
        frame.render_widget(Paragraph::new(line).style(Style::new().bg(PANEL)), Rect { x, width: w, ..row });
        self.hits.buttons = button_hits(&acts, Rect { x, width: w, ..row });
    }
}

/// Where each of the buttons `acts::buttons` draws lands in `row`, those it cuts left out.
pub fn button_hits(acts: &[acts::Act], row: Rect) -> Vec<(Rect, usize)> {
    let mut hits = Vec::new();
    let mut at = row.x;
    for (i, act) in acts.iter().enumerate() {
        let key = act.key.map(|k| format!("  {k}")).unwrap_or_default();
        let w = width(&format!("  {}{key}  ", act.label)) as u16;
        if at + w <= row.x + row.width {
            hits.push((Rect { x: at, y: row.y, width: w, height: 1 }, i));
        }
        at += w + 1;
    }
    hits
}

/// The part of a row, `top` lines into the list, that shows with the list scrolled by `offset`.
fn visible(area: Rect, top: usize, height: usize, offset: usize) -> Option<Rect> {
    let start = top.max(offset);
    let end = (top + height).min(offset + area.height as usize);
    (start < end).then(|| Rect { x: area.x, y: area.y + (start - offset) as u16, width: area.width, height: (end - start) as u16 })
}
