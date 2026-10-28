//! The frame: the header, the apps (or the releases) as a list, the selected row's panel (or, compact, its buttons on
//! the row itself), the footer, and the dialog, toasts and palette over them.

use cockpit_core::acts::{self, Act};
use cockpit_core::files::Density;
use cockpit_core::text::{fit, width, wrap};
use cockpit_core::theme::{BLUE, DIM, FG, GREEN, LINE, ORANGE, PANEL, RED, base, bold, dim, fg};
use cockpit_core::widgets;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Wrap};

use crate::app::{App, Cmd};
use crate::data::{self, State, count, duration, id_text};
use crate::pinned::ICON;
use crate::rows::{Item, Row};

/// The name column: as wide as the longest name, within these bounds.
const NAME_MIN: usize = 10;
const NAME_MAX: usize = 24;
const TAG: usize = 15;
const WHEN: usize = 18;
const MONITORS: usize = 17;
/// A sparkline's value after it: "14 err", "161 ms".
const VALUE: usize = 8;
const SPARK_MIN: usize = 8;
const SPARK_MAX: usize = 20;
const TIME: usize = 6;
const WHO: usize = 8;
const PANEL_MAX: usize = 12;

const OK_GLYPH: &str = "\u{f42e}";
const WAIT_GLYPH: &str = "\u{f43a}";
const ALERT_GLYPH: &str = "\u{f421}";
const UNKNOWN_GLYPH: &str = "\u{f420}";
const BELL: &str = "\u{f0f3}";
const BUG: &str = "\u{f188}";

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
        let info_h: usize = info.iter().map(|l| wrap(&plain(l), inner_w).len()).sum();
        let panel_h = if comfortable && info_h > 0 { (1 + info_h + 2 * usize::from(!acts.is_empty())).min(PANEL_MAX) as u16 } else { 0 };
        let head = self.head_lines();
        let [head_area, list, panel, footer] =
            Layout::vertical([Constraint::Length(head.len() as u16 + 1), Constraint::Min(1), Constraint::Length(panel_h), Constraint::Length(1)]).areas(area);
        frame.render_widget(Paragraph::new(head), inset(head_area));
        self.draw_list(frame, inset(list), &acts);
        if panel_h > 0 {
            self.draw_panel(frame, panel, info, &acts);
        }
        self.draw_footer(frame, footer);
        self.toasts.render(frame, Rect { height: area.height.saturating_sub(1), ..area });
        if let Some((dialog, _, _)) = &self.dialog {
            dialog.render(frame, area);
        }
        if self.palette.is_open {
            self.palette.render(frame, area);
        }
    }

    fn head_lines(&self) -> Vec<Line<'static>> {
        let mut spans = vec![Span::styled(format!("{ICON}  "), fg(ORANGE)), Span::styled("Monitor", bold(ORANGE))];
        let Some(doc) = &self.doc else {
            spans.push(Span::styled("   waiting for the mod", dim()));
            return vec![Line::from(spans)];
        };
        let today = doc.apps.iter().flat_map(|a| &a.releases).filter(|r| r.started_at.is_some_and(|t| self.clock.is_today(t))).count();
        let mut facts = vec![plural(doc.apps.len(), "app"), format!("{} today", plural(today, "release"))];
        if let Some(at) = doc.at.and_then(chrono::DateTime::from_timestamp_millis) {
            facts.push(format!("updated {}", self.clock.hm(at)));
        }
        spans.push(Span::styled(format!("   {}", facts.join(" · ")), dim()));
        if self.timeline {
            spans.push(Span::styled("   releases, newest first", fg(ORANGE)));
        }
        let mut lines = vec![Line::from(spans)];
        // One line for what failed to fetch, however much did: the first, and how many more.
        let errors = doc.error_lines();
        if let Some(first) = errors.first() {
            let more = if errors.len() > 1 { format!("  (+{} more)", errors.len() - 1) } else { String::new() };
            lines.push(Line::styled(format!("{first}{more}"), fg(RED)));
        }
        lines
    }

    /// The name column's width: the longest app name, within bounds.
    fn name_w(&self) -> usize {
        let longest = self.doc.as_ref().map(|d| d.apps.iter().map(|a| width(&a.name)).max().unwrap_or(0)).unwrap_or(0);
        (longest + 1).clamp(NAME_MIN, NAME_MAX)
    }

    fn draw_list(&mut self, frame: &mut Frame, area: Rect, acts: &[(Act, Cmd)]) {
        self.hits.list = area;
        let rows = self.rows();
        if rows.is_empty() {
            let text = if self.doc.is_some() { "No app in the monitor." } else { "A Claude session with the Backstage and Datadog connectors fills this." };
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
        // Compact, the buttons cover the end of the selected row.
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

    /// A row's line, `room` cells wide.
    fn row_line(&self, row: &Row, room: usize) -> Line<'static> {
        let Some(app) = self.doc.as_ref().and_then(|d| d.app(row.item.app())) else { return Line::default() };
        let fold = match row.fold {
            Some(true) => "▼ ",
            Some(false) => "▶ ",
            None => "  ",
        };
        let mut spans = vec![Span::styled(if row.depth == 0 { fold.to_string() } else { format!("  {}", row.guide) }, fg(LINE))];
        let room = room.saturating_sub(width(&plain(&Line::from(spans.clone()))));
        match &row.item {
            Item::App(_) => spans.extend(self.app_cells(app, room)),
            Item::Monitor(_, id) => {
                let Some(m) = app.alerting().iter().find(|m| &id_text(&m.id) == id) else { return Line::from(spans) };
                let color = if m.status == "Alert" { RED } else { ORANGE };
                let status = format!("  {}", m.status);
                spans.push(Span::styled(format!("{BELL} "), fg(color)));
                spans.push(Span::styled(fit(&m.name, room.saturating_sub(2 + width(&status))), fg(FG)));
                spans.push(Span::styled(status, fg(color)));
            }
            Item::Issue(_, id) => {
                let Some(issue) = app.issues.iter().find(|i| &i.id == id) else { return Line::from(spans) };
                let since = issue.first_seen.map(|t| format!("since {}", self.clock.hm(t))).unwrap_or_default();
                let tail = format!("  {}×  {since}", issue.count);
                spans.push(Span::styled(format!("{BUG} "), fg(RED)));
                spans.push(Span::styled("new ", bold(ORANGE)));
                spans.push(Span::styled(fit(&format!("{}: {}", issue.kind, issue.message), room.saturating_sub(6 + width(&tail))), fg(FG)));
                spans.push(Span::styled(tail, dim()));
            }
            Item::Release(_, id) => {
                let Some(r) = app.release(id) else { return Line::from(spans) };
                spans.extend(self.release_cells(app, r, room));
            }
        }
        Line::from(spans)
    }

    /// An app's cells: its glyph, name, tag and when, then as room allows its monitor counts and sparklines.
    fn app_cells(&self, app: &data::App, room: usize) -> Vec<Span<'static>> {
        let (glyph, color) = app_glyph(app);
        let name_w = self.name_w();
        let fixed = 2 + name_w + TAG;
        let monitors = if room >= fixed + MONITORS + 8 { MONITORS } else { 0 };
        let when_w = WHEN.min(room.saturating_sub(fixed + monitors));
        let mut spans = vec![
            Span::styled(format!("{glyph} "), fg(color)),
            Span::styled(cell(&app.name, name_w), bold(FG)),
            Span::styled(cell(&app.tag, TAG), fg(FG)),
            Span::styled(cell(&self.when(app), when_w), dim()),
        ];
        if monitors > 0 {
            let (text, style) = match (&app.monitors, app.counts()) {
                (Some(m), Some(counts)) => (
                    counts,
                    if m.alert > 0 {
                        fg(RED)
                    } else if m.warn > 0 {
                        fg(ORANGE)
                    } else {
                        dim()
                    },
                ),
                _ if app.watched => ("monitors ?".into(), dim()),
                _ => ("not watched".into(), dim()),
            };
            spans.push(Span::styled(cell(&text, monitors), style));
        }
        let Some(series) = &app.series else { return spans };
        let left = room.saturating_sub(fixed + monitors + when_w);
        let one = SPARK_MIN + 1 + VALUE + 2;
        let sparks = if left >= 2 * one { 2 } else { usize::from(left >= one) };
        if sparks == 0 {
            return spans;
        }
        let spark_w = (left / sparks).saturating_sub(VALUE + 3).min(SPARK_MAX);
        let errors_color = if app.is_troubled() { RED } else { DIM };
        let last = |v: &[f64]| v.last().copied().unwrap_or_default();
        spans.extend(spark(&series.errors, spark_w, &format!("{} err", count(last(&series.errors))), Some(VALUE), errors_color));
        if sparks == 2 {
            spans.extend(spark(&series.p95, spark_w, &duration(latest(&series.p95)), Some(VALUE), BLUE));
        }
        spans
    }

    /// "2 h ago by sam", "1 h ago by you"; an app's deploy alone when it has no release in view.
    fn when(&self, app: &data::App) -> String {
        let ago = app.deployed_at.map(|t| self.clock.ago(t)).unwrap_or_default();
        match app.releases.first().filter(|r| r.tag == app.tag) {
            Some(r) => format!("{ago} by {}", r.who()),
            None => ago,
        }
    }

    /// A release's cells: its verdict, tag (with its app in the timeline), time, who and tickets, then its summary.
    fn release_cells(&self, app: &data::App, r: &data::Release, room: usize) -> Vec<Span<'static>> {
        let state = r.verdict.state;
        let accent = if r.is_mine { fg(ORANGE) } else { fg(FG) };
        let mut spans = vec![Span::raw(format!("{} ", state.icon()))];
        let mut used = 3;
        if self.timeline {
            let name_w = self.name_w();
            spans.push(Span::styled(cell(&app.name, name_w), bold(FG)));
            used += name_w;
        }
        let time =
            r.at().map(|t| if self.clock.is_today(t) { self.clock.hm(t) } else { self.clock.local(t).format("%a %H:%M").to_string() }).unwrap_or_default();
        let time_w = TIME.max(width(&time) + 1);
        let tickets = r.tickets.iter().map(|t| t.id.clone()).collect::<Vec<_>>().join(" ");
        let tickets = if tickets.is_empty() { String::new() } else { format!("{tickets}  ") };
        spans.push(Span::styled(cell(&r.tag, TAG), if r.is_mine { bold(ORANGE) } else { fg(FG) }));
        spans.push(Span::styled(cell(&time, time_w), dim()));
        spans.push(Span::styled(cell(&r.who(), WHO), accent));
        spans.push(Span::styled(tickets.clone(), accent));
        used += TAG + time_w + WHO + width(&tickets);
        let color = match state {
            State::Moved => ORANGE,
            State::Failed => RED,
            State::Ok => GREEN,
            _ => DIM,
        };
        spans.push(Span::styled(fit(&r.verdict.summary, room.saturating_sub(used)), fg(color)));
        spans
    }

    /// The selected row's details, for the panel under the list.
    fn info(&self, cells: usize) -> Vec<Line<'static>> {
        let Some(row) = self.selected_row() else { return Vec::new() };
        let Some(app) = self.doc.as_ref().and_then(|d| d.app(row.item.app())) else { return Vec::new() };
        match &row.item {
            Item::App(_) => self.app_info(app, cells),
            Item::Monitor(_, id) => {
                let Some(m) = app.alerting().iter().find(|m| &id_text(&m.id) == id) else { return Vec::new() };
                vec![
                    Line::styled(m.name.clone(), bold(FG)),
                    Line::styled(format!("{} · monitor {id} of {}", m.status, app.name), dim()),
                    Line::styled(m.url.clone(), dim()),
                ]
            }
            Item::Issue(_, id) => {
                let Some(issue) = app.issues.iter().find(|i| &i.id == id) else { return Vec::new() };
                let mut lines = vec![Line::styled(issue.kind.clone(), bold(FG)), Line::styled(issue.message.clone(), fg(FG))];
                let since = issue.first_seen.map(|t| format!(" · first seen {}", self.clock.ago(t))).unwrap_or_default();
                lines.push(Line::styled(format!("{} events in the last day{since}, in {}", issue.count, issue.first_version), dim()));
                lines.push(Line::styled(issue.url.clone(), dim()));
                lines
            }
            Item::Release(_, id) => app.release(id).map(|r| self.release_info(app, r)).unwrap_or_default(),
        }
    }

    fn app_info(&self, app: &data::App, cells: usize) -> Vec<Line<'static>> {
        let mut title = format!("{}  ·  service {} in {}", app.name, app.service, self.doc.as_ref().map(|d| d.env.clone()).unwrap_or_default());
        if app.pinned {
            title.push_str("  ·  pinned");
        }
        let mut facts = vec![app.health.clone(), format!("{} deployed {}", app.tag, self.when(app))];
        if app.behind > 0 {
            facts.push(format!("{} behind", plural(app.behind as usize, "commit")));
        }
        let mut lines = vec![Line::styled(title, bold(FG)), Line::styled(facts.join(" · "), dim())];
        if !app.watched {
            lines.push(Line::styled("not watched: no release in the last 24 h; pin it to follow it anyway", dim()));
            return lines;
        }
        if let Some(m) = &app.monitors {
            lines.push(Line::styled(format!("monitors: {} ok · {} warn · {} alert · {} no data", m.ok, m.warn, m.alert, m.nodata), dim()));
        }
        if let Some(series) = &app.series {
            let w = SPARK_MAX.min(cells.saturating_sub(24));
            let total: f64 = series.errors.iter().sum();
            let mut errors = vec![Span::styled("errors 24 h  ", dim())];
            errors.extend(spark(
                &series.errors,
                w,
                &format!("{} in all, {} in the last bin", count(total), count(series.errors.last().copied().unwrap_or_default())),
                None,
                if app.is_troubled() { RED } else { DIM },
            ));
            lines.push(Line::from(errors));
            let mut p95 = vec![Span::styled("p95    24 h  ", dim())];
            p95.extend(spark(&series.p95, w, &format!("{} lately", duration(latest(&series.p95))), None, BLUE));
            lines.push(Line::from(p95));
        }
        let new = app.new_issues().count();
        if !app.issues.is_empty() {
            lines.push(Line::styled(format!("{} in Error Tracking over the last day, {new} new", plural(app.issues.len(), "open error")), dim()));
        }
        lines
    }

    fn release_info(&self, app: &data::App, r: &data::Release) -> Vec<Line<'static>> {
        let v = &r.verdict;
        let mut lines = vec![Line::from(vec![
            Span::raw(format!("{} ", v.state.icon())),
            Span::styled(format!("{} {}", app.name, r.tag), bold(if r.is_mine { ORANGE } else { FG })),
            Span::styled(format!("  by {}", r.who()), dim()),
        ])];
        let mut facts = vec![r.phase.clone()];
        if let Some(start) = r.started_at {
            facts.push(match r.finished_at {
                Some(end) => format!("{} → {}", self.clock.hm(start), self.clock.hm(end)),
                None => format!("since {}", self.clock.hm(start)),
            });
        }
        if let Some(until) = v.until {
            let watched = if until > self.clock.now { "watched until" } else { "watched to" };
            facts.push(format!("{watched} {}", self.clock.hm(until)));
        }
        lines.push(Line::styled(facts.join(" · "), dim()));
        let color = match v.state {
            State::Moved => ORANGE,
            State::Failed => RED,
            State::Ok => GREEN,
            _ => FG,
        };
        lines.push(Line::styled(v.summary.clone(), fg(color)));
        if let (Some(before), Some(after)) = (v.before, v.after) {
            let side = |w: data::Window| format!("{} errors / {} hits, p95 {}", count(w.errors), count(w.hits), duration(w.p95));
            lines.push(Line::styled(format!("before {}   after {}", side(before), side(after)), dim()));
        }
        let mut refs: Vec<String> = r.tickets.iter().map(|t| t.id.clone()).collect();
        refs.extend(r.prs.iter().map(|p| format!("{}#{}", p.repo, p.number)));
        if !refs.is_empty() {
            lines.push(Line::styled(refs.join("  "), if r.is_mine { fg(ORANGE) } else { dim() }));
        }
        for id in &v.new_issues {
            if let Some(issue) = app.issues.iter().find(|i| &i.id == id) {
                lines.push(Line::styled(format!("new: {}: {} ({}×)", issue.kind, issue.message, issue.count), fg(RED)));
            }
        }
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
        let view = if self.timeline { "by app" } else { "releases" };
        let keys = [("enter", "main"), ("space", "fold"), ("w", view), ("R", "refresh"), ("q", "quit")];
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

/// The app's glyph: red for a failed release, a degraded app or an alert, orange for a moved release, a warning or a
/// deploy under way, teal when healthy.
fn app_glyph(app: &data::App) -> (&'static str, Color) {
    let states: Vec<State> = app.releases.iter().map(|r| r.verdict.state).collect();
    let m = app.monitors.as_ref();
    if states.contains(&State::Failed) || app.health == "Degraded" || m.is_some_and(|m| m.alert > 0) {
        (ALERT_GLYPH, RED)
    } else if states.contains(&State::Moved) || m.is_some_and(|m| m.warn > 0) {
        (ALERT_GLYPH, ORANGE)
    } else if app.health == "Progressing" || states.contains(&State::Watching) {
        (WAIT_GLYPH, ORANGE)
    } else if app.health == "Healthy" {
        (OK_GLYPH, GREEN)
    } else {
        (UNKNOWN_GLYPH, DIM)
    }
}

/// A sparkline of the whole series `cells` wide, each cell the highest of the values it covers, and its value after it,
/// fitted to `value_w` cells when given.
fn spark(values: &[f64], cells: usize, value: &str, value_w: Option<usize>, color: Color) -> Vec<Span<'static>> {
    let cells = cells.max(1);
    let squeezed: Vec<f64> = if values.len() <= cells {
        values.to_vec()
    } else {
        (0..cells).map(|i| values[i * values.len() / cells..(i + 1) * values.len() / cells].iter().copied().fold(f64::MIN, f64::max)).collect()
    };
    vec![
        Span::raw("  "),
        Span::styled(widgets::sparkline(&squeezed, cells), fg(color)),
        Span::styled(format!(" {}", value_w.map_or(value.to_string(), |w| fit(value, w))), dim()),
    ]
}

/// Text cut or padded to `cells`, its last cell a gap before the next.
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
    const DROPPED_FIRST: [&str; 5] = ["refresh", "pin", "argocd", "bug", "talk"];
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

/// A gauge's latest value: its last bin with data, a bin without any being 0.
fn latest(values: &[f64]) -> f64 {
    values.iter().rev().copied().find(|v| *v > 0.0).unwrap_or_default()
}
