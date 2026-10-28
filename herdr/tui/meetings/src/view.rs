//! The frame: the header, the now-or-next banner, the week as a list of days (or as a picture), the selected meeting's
//! panel (or, compact, its buttons on the row itself), the footer, and the toasts and palette over them.

use cockpit_core::acts::{self, Act};
use cockpit_core::files::Density;
use cockpit_core::text::{fit, width, wrap};
use cockpit_core::theme::{FG, GREEN, LINE, ORANGE, PANEL, RED, base, bold, dim, fg};
use cockpit_core::widgets;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui_image::Image;

use crate::app::{App, Cmd};
use crate::picture::Key;
use crate::pinned::{ICON_CALENDAR, ICON_JOIN, ICON_NOW, ICON_PEOPLE, response_icon};
use crate::week::{Meeting, day_label, until_text};

const TIME: usize = 14;
const DURATION: usize = 8;
const PEOPLE: usize = 6;
/// Before the time: a space, the glyph and a space.
const LEAD: usize = 3;
const BRIEF_INDENT: usize = 6;
const PANEL_MAX: usize = 9;
const NOW_BG: Color = Color::Rgb(48, 38, 30);

/// One line of the list, and the meeting it belongs to when it is that meeting's row.
struct Row {
    line: Line<'static>,
    meeting: Option<String>,
    bg: Option<Color>,
}

fn plain_row(line: Line<'static>) -> Row {
    Row { line, meeting: None, bg: None }
}

impl App {
    pub fn draw(&mut self, frame: &mut Frame) {
        self.take_replies();
        let area = frame.area();
        frame.render_widget(Block::new().style(base()), area);
        self.hits = Default::default();
        let inner_w = area.width.saturating_sub(2) as usize;
        let acts = fitting(&self.acts(), inner_w);
        let comfortable = self.density == Density::Comfortable;
        let info = if comfortable { self.info(inner_w) } else { Vec::new() };
        let panel_h = if comfortable && !info.is_empty() { (2 + info.len() + usize::from(!acts.is_empty())).min(PANEL_MAX) as u16 } else { 0 };
        let head = self.head_lines();
        let banner = self.banner();
        let [head_area, banner_area, list, panel, footer] = Layout::vertical([
            Constraint::Length(head.len() as u16 + 1),
            Constraint::Length(if banner.is_some() { 2 } else { 0 }),
            Constraint::Min(1),
            Constraint::Length(panel_h),
            Constraint::Length(1),
        ])
        .areas(area);
        frame.render_widget(Paragraph::new(head), inset(head_area));
        if let Some((line, join)) = banner {
            self.draw_banner(frame, inset(Rect { height: 1, ..banner_area }), line, join);
        }
        if self.week_view {
            self.draw_week(frame, inset(list));
        } else {
            self.draw_list(frame, inset(list), &acts);
        }
        if panel_h > 0 {
            self.draw_panel(frame, panel, info, &acts);
        }
        self.draw_footer(frame, footer);
        self.toasts.render(frame, Rect { height: area.height.saturating_sub(1), ..area });
        if self.palette.is_open {
            self.palette.render(frame, area);
        }
    }

    fn head_lines(&self) -> Vec<Line<'static>> {
        let mut spans = vec![Span::styled(format!("{ICON_CALENDAR}  "), fg(ORANGE)), Span::styled("Meetings", bold(ORANGE))];
        let at = self.week.as_ref().and_then(|w| w.at).and_then(chrono::DateTime::from_timestamp_millis);
        match at {
            Some(at) => spans.push(Span::styled(format!("   updated {}", self.clock.local(at).format("%H:%M")), dim())),
            None => spans.push(Span::styled("   waiting for a Claude session with the Google Calendar connector", dim())),
        }
        if self.week_view {
            spans.push(Span::styled("   week view", fg(ORANGE)));
        }
        let mut lines = vec![Line::from(spans)];
        if let Some(error) = self.week.as_ref().and_then(|w| w.error.clone()) {
            lines.push(Line::styled(error, fg(RED)));
        }
        lines
    }

    /// The running meeting, else the next one, and whether it has a call to join.
    fn banner(&self) -> Option<(Line<'static>, Option<String>)> {
        let week = self.week.as_ref()?;
        let Some(first) = week.upcoming(&self.clock).into_iter().next() else {
            return Some((Line::styled("  Nothing left this week.", dim()), None));
        };
        let clock = &self.clock;
        let line = if first.start <= clock.now {
            Line::from(vec![
                Span::styled(format!(" {ICON_NOW} now  "), bold(ORANGE)),
                Span::styled(first.title.clone(), bold(FG)),
                Span::styled(format!("   until {}", clock.local(first.end).format("%H:%M")), dim()),
            ])
        } else {
            let local = clock.local(first.start);
            let when = if local.date() == clock.today() { until_text(first.start, clock) } else { local.format("%A %H:%M").to_string() };
            Line::from(vec![
                Span::styled(" next  ", dim()),
                Span::styled(when, bold(ORANGE)),
                Span::raw("  "),
                Span::styled(first.title.clone(), bold(FG)),
            ])
        };
        Some((line, first.join.as_ref().map(|_| first.id.clone())))
    }

    fn draw_banner(&mut self, frame: &mut Frame, area: Rect, line: Line<'static>, join: Option<String>) {
        let Some(id) = join else {
            frame.render_widget(Paragraph::new(line), area);
            return;
        };
        let pill = button_line(&[(Act::new(&format!("{ICON_JOIN} join"), 'm', "join").main(), Cmd::Join(id.clone()))]);
        let w = width(&plain(&pill)) as u16;
        let text = Rect { width: area.width.saturating_sub(w + 1), ..area };
        frame.render_widget(Paragraph::new(line), text);
        if area.width > w {
            let bar = Rect { x: area.x + area.width - w, width: w, ..area };
            frame.render_widget(Paragraph::new(pill), bar);
            self.hits.buttons.push((bar, Cmd::Join(id)));
        }
    }

    /// A meeting's row: its glyph, time, length, title (clickable) and people, `room` cells wide.
    fn meeting_line(&self, m: &Meeting, room: usize) -> (Line<'static>, usize) {
        let clock = &self.clock;
        let (is_past, is_now) = (m.is_past(clock), m.is_now(clock));
        let glyph = if is_now { ICON_NOW } else { response_icon(&m.response) };
        let glyph_style = if is_now {
            fg(ORANGE)
        } else if m.response == "accepted" {
            fg(GREEN)
        } else {
            dim()
        };
        let title_room = room.saturating_sub(LEAD + TIME + DURATION + PEOPLE).max(8);
        let title_style = if is_past {
            dim()
        } else if is_now {
            bold(FG)
        } else {
            fg(FG)
        };
        let people = if m.attendees > 1 { format!("{ICON_PEOPLE} {}", m.attendees) } else { String::new() };
        let line = Line::from(vec![
            Span::raw(" "),
            Span::styled(format!("{glyph} "), glyph_style),
            Span::styled(fit(&m.span_text(clock), TIME), if is_past { dim() } else { fg(FG) }),
            Span::styled(fit(&m.duration_text(), DURATION), dim()),
            Span::styled(fit(&m.title, title_room), title_style),
            Span::styled(fit(&people, PEOPLE), dim()),
        ]);
        (line, title_room)
    }

    fn rows(&self, cells: usize, compact_room: Option<usize>) -> Vec<Row> {
        let mut rows: Vec<Row> = Vec::new();
        let mut day = None;
        for m in self.shown() {
            let local = self.clock.local(m.start);
            if Some(local.date()) != day {
                day = Some(local.date());
                if !rows.is_empty() {
                    rows.push(plain_row(Line::raw("")));
                }
                rows.push(plain_row(Line::styled(format!(" {}", day_label(local.date(), &self.clock)), bold(FG))));
            }
            let is_selected = self.selected.as_deref() == Some(m.id.as_str());
            let room = if is_selected { compact_room.unwrap_or(cells) } else { cells };
            let (line, _) = self.meeting_line(m, room);
            let bg = m.is_now(&self.clock).then_some(NOW_BG);
            rows.push(Row { line, meeting: Some(m.id.clone()), bg });
            if self.preparing.contains(&m.id) {
                rows.push(plain_row(Line::from(vec![Span::raw(" ".repeat(BRIEF_INDENT)), Span::styled("preparing the brief…", fg(ORANGE).add_modifier(Modifier::DIM))])));
            }
            if let (Some(brief), true) = (self.briefs.get(&m.id), self.open.contains(&m.id)) {
                // The brief's markdown drawn as the show view draws it, kept tight: no blank line between its blocks.
                for line in cockpit_show::markdown_lines(&brief.join("\n"), cells.saturating_sub(BRIEF_INDENT)) {
                    if line.width() > 0 {
                        let indent = Span::raw(" ".repeat(BRIEF_INDENT));
                        rows.push(plain_row(Line::from(std::iter::once(indent).chain(line.spans).collect::<Vec<_>>()).style(fg(FG))));
                    }
                }
                rows.push(plain_row(Line::raw("")));
            }
        }
        if rows.is_empty() && self.week.is_some() {
            rows.push(plain_row(Line::styled(" No meeting in the calendar this week.", dim())));
        }
        rows
    }

    fn draw_list(&mut self, frame: &mut Frame, area: Rect, acts: &[(Act, Cmd)]) {
        self.hits.list = area;
        let compact = self.density == Density::Compact;
        // Compact, the buttons share the selected row with its time and the start of its title.
        let acts = fitting(acts, (area.width as usize).saturating_sub(LEAD + TIME + DURATION + 10));
        let buttons = if compact { button_line(&acts) } else { Line::default() };
        let buttons_w = width(&plain(&buttons));
        let compact_room = (buttons_w > 0).then(|| (area.width as usize).saturating_sub(buttons_w + 1));
        let height = area.height as usize;
        let mut rows = self.rows(area.width as usize, compact_room);
        // A list longer than the pane gives its last column to the scrollbar.
        if rows.len() > height {
            rows = self.rows((area.width as usize).saturating_sub(1), compact_room.map(|r| r.saturating_sub(1)));
        }
        let selected = rows.iter().position(|r| r.meeting.is_some() && r.meeting == self.selected);
        if let Some(id) = self.reveal.take()
            && let Some(at) = rows.iter().position(|r| r.meeting.as_deref() == Some(id.as_str()))
        {
            self.scroll = at.saturating_sub(2);
            self.follow = false;
        }
        if let (true, Some(selected)) = (self.follow, selected) {
            // The day's heading stays in view with its first meeting.
            let top = if selected > 0 && rows[selected - 1].meeting.is_none() { selected - 1 } else { selected };
            if top < self.scroll {
                self.scroll = top;
            } else if selected >= self.scroll + height {
                self.scroll = selected + 1 - height;
            }
        }
        self.scroll = self.scroll.min(rows.len().saturating_sub(height));
        let overflow = rows.len() > height;
        let text_w = if overflow { area.width.saturating_sub(1) } else { area.width };
        for (i, row) in rows.iter().enumerate().skip(self.scroll).take(height) {
            let rect = Rect { x: area.x, y: area.y + (i - self.scroll) as u16, width: text_w, height: 1 };
            frame.render_widget(Paragraph::new(row.line.clone()), rect);
            let is_selected = Some(i) == selected;
            if let Some(bg) = if is_selected { Some(PANEL) } else { row.bg } {
                frame.buffer_mut().set_style(rect, Style::new().bg(bg));
            }
            let Some(id) = &row.meeting else { continue };
            self.hits.rows.push((rect, id.clone()));
            let room = if is_selected { compact_room.unwrap_or(rect.width as usize) } else { rect.width as usize };
            let title_x = (LEAD + TIME + DURATION) as u16;
            let title_w = room.saturating_sub(LEAD + TIME + DURATION + PEOPLE).max(8) as u16;
            self.hits.titles.push((Rect { x: rect.x + title_x, width: title_w.min(rect.width.saturating_sub(title_x)), ..rect }, id.clone()));
            if is_selected && buttons_w > 0 && buttons_w < rect.width as usize {
                let bar = Rect { x: rect.x + rect.width - buttons_w as u16, width: buttons_w as u16, ..rect };
                frame.render_widget(Paragraph::new(buttons.clone()), bar);
                self.hits.buttons.extend(button_hits(&acts, bar));
            }
            self.fx.paint(id, frame.buffer_mut(), rect);
        }
        if overflow {
            widgets::scrollbar(frame, area, self.scroll, rows.len(), height);
        }
    }

    /// The week as a picture filling the list's place; a word while it is drawn or when it cannot be.
    fn draw_week(&mut self, frame: &mut Frame, area: Rect) {
        self.hits.picture = area;
        let Some(week) = &self.week else {
            frame.render_widget(Paragraph::new(Line::styled("No week to draw yet.", dim())), area);
            return;
        };
        let key = Key { cols: area.width, rows: area.height, minute: self.clock.minute(), version: self.version, selected: self.selected.clone() };
        self.picture.want(key, week, &self.clock);
        match (&self.picture.shown, &self.picture.error) {
            (_, Some(error)) => frame.render_widget(Paragraph::new(Line::styled(format!("cannot draw the week: {error}"), fg(RED))), area),
            (Some((key, protocol)), _) if key.cols <= area.width && key.rows <= area.height => {
                frame.render_widget(Image::new(protocol), Rect { width: key.cols, height: key.rows, ..area });
            }
            _ => frame.render_widget(Paragraph::new(Line::styled("drawing the week…", dim())), area),
        }
    }

    /// The selected meeting's details: its title, when, who, and its brief's state.
    fn info(&self, cells: usize) -> Vec<Line<'static>> {
        let Some(m) = self.selected_meeting() else { return Vec::new() };
        let clock = &self.clock;
        let mut lines: Vec<Line<'static>> = wrap(&m.title, cells).into_iter().map(|l| Line::styled(l, bold(FG))).collect();
        let local = clock.local(m.start);
        let when = if m.is_now(clock) {
            format!("now, until {}", clock.local(m.end).format("%H:%M"))
        } else if !m.is_past(clock) && local.date() == clock.today() {
            format!("{} {}", m.span_text(clock), until_text(m.start, clock))
        } else {
            format!("{} {}", day_label(local.date(), clock), m.span_text(clock))
        };
        let mut facts = vec![when, m.duration_text()];
        if m.attendees > 1 {
            facts.push(format!("{} people", m.attendees));
        }
        facts.push(m.response.clone());
        lines.push(Line::styled(facts.join(" · "), dim()));
        if !m.people.is_empty() {
            let names: Vec<&str> = m.people.iter().map(|p| p.split('@').next().unwrap_or(p)).collect();
            lines.extend(wrap(&names.join(", "), cells).into_iter().take(2).map(|l| Line::styled(l, dim())));
        }
        if self.preparing.contains(&m.id) {
            lines.push(Line::styled("preparing the brief…", fg(ORANGE)));
        }
        lines
    }

    fn draw_panel(&mut self, frame: &mut Frame, area: Rect, info: Vec<Line<'static>>, acts: &[(Act, Cmd)]) {
        frame.render_widget(Paragraph::new("─".repeat(area.width as usize)).style(fg(PANEL)), Rect { height: 1, ..area });
        let body = inset(Rect { y: area.y + 1, height: area.height.saturating_sub(1), ..area });
        let text_h = body.height.saturating_sub(if acts.is_empty() { 0 } else { 2 });
        frame.render_widget(Paragraph::new(info), Rect { height: text_h, ..body });
        if !acts.is_empty() && body.height >= 1 {
            let bar = Rect { y: body.y + body.height - 1, height: 1, ..body };
            frame.render_widget(Paragraph::new(button_line(acts)), bar);
            self.hits.buttons.extend(button_hits(acts, bar));
        }
    }

    fn draw_footer(&self, frame: &mut Frame, area: Rect) {
        let view = if self.week_view { "list" } else { "week" };
        let keys = [("o", "open"), ("m", "join"), ("b", "brief"), ("p", "prepare"), ("w", view), ("q", "quit")];
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
    const DROPPED_FIRST: [&str; 4] = ["redo", "open", "brief", "prepare"];
    let fits = |acts: &[(Act, Cmd)]| width(&plain(&button_line(acts))) <= cells;
    let mut kept = acts.to_vec();
    for id in DROPPED_FIRST {
        if fits(&kept) {
            break;
        }
        kept.retain(|(a, _)| a.id != id);
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
