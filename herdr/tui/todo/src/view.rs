//! The frame: the header, the filter, the list (the adder, the open items, the folded done ones), the selection's panel
//! (or, compact, its buttons on the row itself), the footer, and the toasts, palette and dialog over them.

use cockpit_core::acts::{self, Act};
use cockpit_core::files::Density;
use cockpit_core::text::{fit, width, wrap};
use cockpit_core::theme::{FG, GREEN, LINE, ORANGE, PANEL, base, bold, dim, fg};
use cockpit_core::widgets;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

use crate::app::{App, Cmd, FOLD_FX, Sel};
use crate::doc::Item;
use crate::links;
use crate::pinned::{ICON_ADD, ICON_DONE, ICON_EDIT, ICON_FOLD, ICON_OPEN, ICON_TODO, ICON_UNFOLD};

/// Mark, space, glyph and two spaces before an item's text.
const LEAD: usize = 5;
const PANEL_MAX: usize = 10;

/// One line of the list: what it shows, what selecting it stands for, and what a flash on it is keyed by.
struct Row {
    line: Line<'static>,
    sel: Option<Sel>,
    fx: Option<String>,
    /// A token's place in the line and the token, for clicks.
    tokens: Vec<(usize, usize, String)>,
    item: Option<Item>,
}

impl Row {
    fn plain(line: Line<'static>) -> Self {
        Self { line, sel: None, fx: None, tokens: Vec::new(), item: None }
    }
}

/// The item's text cut to `room` cells, its tokens underlined in the accent colour; their places, for clicks.
fn text_spans(text: &str, style: Style, room: usize, tokens: &regex::Regex) -> (Vec<Span<'static>>, Vec<(usize, usize, String)>) {
    let mut spans = Vec::new();
    let mut places = Vec::new();
    let mut used = 0;
    let cut = width(text) > room;
    for (part, is_token) in links::split(text, tokens) {
        let left = room.saturating_sub(used + usize::from(cut));
        if left == 0 {
            break;
        }
        let shown: String = if width(part) <= left { part.to_string() } else { fit(part, left).trim_end_matches(['…', ' ']).to_string() };
        let w = width(&shown);
        if is_token {
            spans.push(Span::styled(shown, style.fg(ORANGE).add_modifier(Modifier::UNDERLINED)));
            places.push((used, w, part.to_string()));
        } else {
            spans.push(Span::styled(shown, style));
        }
        used += w;
    }
    if cut {
        spans.push(Span::styled("…", style));
    }
    (spans, places)
}

impl App {
    pub fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        frame.render_widget(Block::new().style(base()), area);
        self.hits = Default::default();
        let inner_w = area.width.saturating_sub(2) as usize;
        let acts = fitting(&self.acts(), inner_w);
        let info = if self.density == Density::Comfortable { self.info(inner_w) } else { Vec::new() };
        let panel_h = if self.density == Density::Comfortable { (2 + info.len() + usize::from(!acts.is_empty())).min(PANEL_MAX) as u16 } else { 0 };
        let filter_h = u16::from(self.filtering || !self.filter.text.is_empty());
        let [head, _, filter, list, panel, footer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(filter_h),
            Constraint::Min(1),
            Constraint::Length(panel_h),
            Constraint::Length(1),
        ])
        .areas(area);
        self.draw_head(frame, inset(head));
        if filter_h > 0 {
            let matched = self.shown().0.len() + self.shown().1.len();
            frame.render_widget(Paragraph::new(widgets::filter_line(&self.filter, self.filtering, matched)), inset(filter));
        }
        self.draw_list(frame, inset(list), &acts);
        if panel_h > 0 {
            self.draw_panel(frame, panel, info, &acts);
        }
        self.draw_footer(frame, footer);
        self.toasts.render(frame, Rect { height: area.height.saturating_sub(1), ..area });
        if self.palette.is_open {
            self.palette.render(frame, area);
        }
        if let Some((dialog, _)) = &self.dialog {
            dialog.render(frame, area);
        }
    }

    fn draw_head(&self, frame: &mut Frame, area: Rect) {
        let mut spans = vec![
            Span::styled(format!("{ICON_TODO}  "), fg(ORANGE)),
            Span::styled("To-do", bold(ORANGE)),
            Span::styled(format!("   {} open", self.open_count()), dim()),
        ];
        if !self.marks.is_empty() {
            spans.push(Span::styled(format!(" · {} marked", self.marks.len()), fg(ORANGE)));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    /// The list's lines, as many as the text has cells for.
    fn rows(&self, cells: usize) -> Vec<Row> {
        let (open, done) = self.shown();
        let mut rows = Vec::new();
        let adding = self.editing.as_ref().filter(|e| e.item.is_none());
        rows.push(Row {
            line: match adding {
                Some(editing) => editing.input.line(&format!("  {ICON_ADD}  ")),
                None => Line::from(vec![Span::styled(format!("  {ICON_ADD}  add a to-do"), dim())]),
            },
            sel: Some(Sel::Adder),
            ..Row::plain(Line::raw(""))
        });
        rows.push(Row::plain(Line::raw("")));
        for item in &open {
            rows.push(self.item_row(item, cells));
        }
        if open.is_empty() {
            let text = if self.filter.text.is_empty() { "  nothing left, nice" } else { "  no open to-do matches" };
            rows.push(Row::plain(Line::styled(text, dim())));
        }
        if !done.is_empty() {
            rows.push(Row::plain(Line::raw("")));
            let unfolded = self.done_unfolded();
            let label = format!("  {} {} done", if unfolded { ICON_UNFOLD } else { ICON_FOLD }, done.len());
            rows.push(Row { sel: Some(Sel::Fold), fx: Some(FOLD_FX.into()), ..Row::plain(Line::styled(label, dim())) });
            if unfolded {
                for item in &done {
                    rows.push(self.item_row(item, cells));
                }
            }
        }
        rows
    }

    fn item_row(&self, item: &Item, cells: usize) -> Row {
        let sel = Sel::Item(item.text.clone());
        let row = |line| Row { line, sel: Some(sel.clone()), fx: Some(item.text.clone()), tokens: Vec::new(), item: Some(item.clone()) };
        if let Some(editing) = self.editing.as_ref().filter(|e| e.item.as_deref() == Some(item.text.as_str())) {
            return row(editing.input.line(&format!("  {ICON_EDIT}  ")));
        }
        let marked = self.marks.contains(&item.text);
        let (glyph, glyph_style) = if item.done { (ICON_DONE, fg(GREEN)) } else { (ICON_OPEN, fg(ORANGE)) };
        let style = if item.done { dim().add_modifier(Modifier::CROSSED_OUT) } else { fg(FG) };
        let mut spans = vec![Span::styled(if marked { "▍ " } else { "  " }, bold(ORANGE)), Span::styled(glyph, glyph_style), Span::raw("  ")];
        let (text, tokens) = text_spans(&item.text, style, cells.saturating_sub(LEAD), &self.tokens);
        spans.extend(text);
        Row { tokens: tokens.into_iter().map(|(x, w, t)| (x + LEAD, w, t)).collect(), ..row(Line::from(spans)) }
    }

    fn draw_list(&mut self, frame: &mut Frame, area: Rect, acts: &[(Act, Cmd)]) {
        self.hits.list = area;
        let compact = self.density == Density::Compact;
        // Compact, the buttons share the selected row with its text.
        let acts = &fitting(acts, (area.width as usize).saturating_sub(LEAD + 16));
        let buttons = if compact && self.editing.is_none() { button_line(acts) } else { Line::default() };
        let buttons_w = width(&plain(&buttons));
        let height = area.height as usize;
        let mut cells = area.width as usize;
        let mut rows = self.rows(cells);
        // A list longer than the pane gives its last column to the scrollbar.
        if rows.len() > height {
            cells = cells.saturating_sub(1);
            rows = self.rows(cells);
        }
        // Compact, the selected row's text leaves room for its buttons.
        if buttons_w > 0
            && let Some(row) = rows.iter_mut().find(|r| r.sel.as_ref() == Some(&self.sel) && r.item.is_some())
        {
            *row = self.item_row(row.item.as_ref().unwrap(), cells.saturating_sub(buttons_w + 1));
        }
        let selected = rows.iter().position(|r| r.sel.as_ref() == Some(&self.sel)).unwrap_or(0);
        if self.follow {
            // The selected row in view, and the adder's gap above the first item kept when it fits.
            if selected < self.scroll {
                self.scroll = selected;
            } else if selected >= self.scroll + height {
                self.scroll = selected + 1 - height;
            }
        }
        self.scroll = self.scroll.min(rows.len().saturating_sub(height));
        let overflow = rows.len() > height;
        let text_w = if overflow { area.width.saturating_sub(1) } else { area.width };
        for (i, row) in rows.iter().enumerate().skip(self.scroll).take(height) {
            let rect = Rect { x: area.x, y: area.y + (i - self.scroll) as u16, width: text_w, height: 1 };
            let is_selected = i == selected && row.sel.is_some();
            let line = row.line.clone();
            let editing_here = self.editing.as_ref().is_some_and(|e| e.item.clone().map_or(Sel::Adder, Sel::Item) == *row.sel.as_ref().unwrap_or(&Sel::Fold));
            let shift = if editing_here { cursor_shift(&line, rect.width as usize) } else { 0 };
            frame.render_widget(Paragraph::new(line).scroll((0, shift)), rect);
            if is_selected {
                frame.buffer_mut().set_style(rect, Style::new().bg(PANEL));
            }
            if let Some(sel) = &row.sel {
                self.hits.rows.push((rect, sel.clone()));
            }
            if let Some(item) = &row.item {
                self.hits.glyphs.push((Rect { x: rect.x + 2, width: 1, ..rect }, item.text.clone()));
            }
            for (x, w, token) in &row.tokens {
                self.hits.tokens.push((Rect { x: rect.x + *x as u16, width: *w as u16, ..rect }, token.clone()));
            }
            if is_selected && buttons_w > 0 && buttons_w < rect.width as usize {
                let x = rect.x + rect.width - buttons_w as u16;
                let bar = Rect { x, width: buttons_w as u16, ..rect };
                frame.render_widget(Paragraph::new(buttons.clone()), bar);
                self.hits.buttons.extend(button_hits(acts, bar));
            }
            if let Some(key) = &row.fx {
                self.fx.paint(key, frame.buffer_mut(), rect);
            }
        }
        if overflow {
            widgets::scrollbar(frame, area, self.scroll, rows.len(), height);
        }
    }

    /// The selection's details: an item's whole text with its links and its state, or how the row works.
    fn info(&self, cells: usize) -> Vec<Line<'static>> {
        if !self.marks.is_empty() {
            let n = self.marks.len();
            return vec![Line::styled(format!("{n} marked: x toggles {}, D deletes {}, esc unmarks", them(n), them(n)), fg(FG))];
        }
        match &self.sel {
            Sel::Adder => vec![Line::styled("a new to-do goes on top; Claude's /todo adds to the same list", dim())],
            Sel::Fold => vec![Line::styled(format!("{} done, the newest first", self.items().iter().filter(|i| i.done).count()), dim())],
            Sel::Item(text) => {
                let Some(item) = self.item(text) else { return Vec::new() };
                let style = if item.done { dim() } else { bold(FG) };
                let mut lines: Vec<Line<'static>> = wrap(&item.text, cells)
                    .into_iter()
                    .map(|part| Line::from(text_spans(&part, style, usize::MAX, &self.tokens).0))
                    .collect();
                let links = links::split(&item.text, &self.tokens).iter().filter(|p| p.1).count();
                let state = if item.done { "done" } else { "open" };
                let more = match links {
                    0 => String::new(),
                    1 => " · o opens its link".into(),
                    n => format!(" · {n} links, ^p finds each"),
                };
                lines.push(Line::styled(format!("{state}{more}"), dim()));
                lines
            }
        }
    }

    fn draw_panel(&mut self, frame: &mut Frame, area: Rect, info: Vec<Line<'static>>, acts: &[(Act, Cmd)]) {
        frame.render_widget(Paragraph::new("─".repeat(area.width as usize)).style(fg(PANEL)), Rect { height: 1, ..area });
        let body = inset(Rect { y: area.y + 1, height: area.height.saturating_sub(1), ..area });
        let has_buttons = !acts.is_empty() && self.editing.is_none();
        let text_h = body.height.saturating_sub(if has_buttons { 2 } else { 0 });
        frame.render_widget(Paragraph::new(info), Rect { height: text_h, ..body });
        if has_buttons && body.height >= 1 {
            let bar = Rect { y: body.y + body.height - 1, height: 1, ..body };
            frame.render_widget(Paragraph::new(button_line(acts)), bar);
            self.hits.buttons.extend(button_hits(acts, bar));
        }
    }

    fn draw_footer(&self, frame: &mut Frame, area: Rect) {
        let keys: &[(&str, &str)] = if self.editing.is_some() {
            &[("enter", "save"), ("esc", "cancel")]
        } else if self.filtering {
            &[("enter", "keep"), ("esc", "clear")]
        } else {
            &[("a", "add"), ("x", "done"), ("e", "edit"), ("/", "filter"), ("v", "mark"), ("u", "undo"), ("D", "delete"), ("q", "quit")]
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

fn them(n: usize) -> &'static str {
    if n == 1 { "it" } else { "them" }
}

fn inset(area: Rect) -> Rect {
    Rect { x: area.x + 1, width: area.width.saturating_sub(2), ..area }
}

fn plain(line: &Line) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

/// The actions that fit in `cells`, the least used ones left out first: they keep their keys and palette entries.
fn fitting(acts: &[(Act, Cmd)], cells: usize) -> Vec<(Act, Cmd)> {
    const DROPPED_FIRST: [&str; 7] = ["unmark", "mark", "down", "up", "edit", "open", "delete"];
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

fn button_line(acts: &[(Act, Cmd)]) -> Line<'static> {
    let acts: Vec<Act> = acts.iter().map(|(a, _)| a.clone()).collect();
    let mut line = acts::buttons(&acts);
    line.spans.pop();
    line
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

/// How far a text field's line scrolls left so its cursor shows.
fn cursor_shift(line: &Line, cells: usize) -> u16 {
    let before: usize = line.spans.iter().take(2).map(|s| width(&s.content)).sum();
    (before + 2).saturating_sub(cells) as u16
}
