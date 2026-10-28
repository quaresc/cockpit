//! The week view as an SVG: a column per day, the hours down the side, each meeting a block (side by side when they
//! overlap), the running one filled, the selected one outlined, and a red line at the time it is now. Sizes are pixels of
//! the pane, so the picture fills whole cells; the blocks' places come back too, for clicks.

use std::fmt::Write;

use chrono::{Datelike, NaiveDate, NaiveDateTime, Timelike, Weekday};

use crate::week::{Clock, Meeting, Week, short_day};

const BG: &str = "#101010";
const TODAY_BG: &str = "#18181A";
const GRID: &str = "#2C2C30";
const LABEL: &str = "#909094";
const FG: &str = "#D2D2D2";
const ORANGE: &str = "#FFC799";
const UPCOMING: &str = "#3A2E24";
const PAST: &str = "#26262A";
const MAYBE: &str = "#1E1E22";
const RED: &str = "#FF8080";
const SELECTED: &str = "#F5F5F5";
const SANS: &str = "Helvetica Neue, Helvetica, Arial, sans-serif";
const DAYS: i64 = 7;

/// A meeting's block, in pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub id: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Where everything goes in a picture this big, with this font's cell size.
pub struct Layout {
    pub days: Vec<NaiveDate>,
    pub blocks: Vec<Block>,
    gutter: f64,
    header: f64,
    column: f64,
    from: f64,
    to: f64,
    height: f64,
}

fn hours(t: NaiveDateTime) -> f64 {
    t.hour() as f64 + t.minute() as f64 / 60.0
}

impl Layout {
    pub fn new(week: &Week, clock: &Clock, width: f64, height: f64, cell: (f64, f64)) -> Self {
        let today = clock.today();
        let on = |day: NaiveDate| week.meetings.iter().filter(move |m| clock.local(m.start).date() == day);
        // The coming seven days, a weekend left out when it has nothing.
        let days: Vec<NaiveDate> = (0..DAYS)
            .filter_map(|n| today.checked_add_days(chrono::Days::new(n as u64)))
            .filter(|d| !matches!(d.weekday(), Weekday::Sat | Weekday::Sun) || on(*d).next().is_some())
            .collect();
        let shown: Vec<&Meeting> = days.iter().flat_map(|d| on(*d)).collect();
        let (mut from, mut to) = (8.0_f64, 19.0_f64);
        for m in &shown {
            let (start, end) = (clock.local(m.start), clock.local(m.end));
            from = from.min(start.hour() as f64);
            to = to.max(if end.date() > start.date() { 24.0 } else { hours(end).ceil() });
        }
        let (gutter, header) = (cell.0 * 6.0, cell.1 * 2.0);
        let column = (width - gutter - cell.0).max(1.0) / days.len().max(1) as f64;
        let mut layout = Self { days, blocks: Vec::new(), gutter, header, column, from, to, height };
        // A block is at least a line of text tall; overlaps are reckoned on what is drawn.
        let least = cell.1 * 0.95;
        for (i, day) in layout.days.clone().into_iter().enumerate() {
            let x = gutter + i as f64 * column;
            let mut meetings: Vec<&Meeting> = on(day).collect();
            meetings.sort_by_key(|m| m.start);
            let spans: Vec<(f64, f64)> = meetings
                .iter()
                .map(|m| {
                    let top = layout.y(hours(clock.local(m.start)));
                    (top, layout.y(end_hours(m, clock)).max(top + least))
                })
                .collect();
            for ((m, (top, bottom)), (lane, count)) in meetings.iter().zip(&spans).zip(lanes(&spans)) {
                let w = column / count as f64;
                layout.blocks.push(Block { id: m.id.clone(), x: x + lane as f64 * w + 2.0, y: top + 1.0, w: w - 4.0, h: bottom - top - 2.0 });
            }
        }
        layout
    }

    fn y(&self, hours: f64) -> f64 {
        let span = (self.to - self.from).max(1.0);
        self.header + (hours - self.from) / span * (self.height - self.header - 6.0)
    }

    /// The meeting whose block holds this pixel.
    pub fn at(&self, x: f64, y: f64) -> Option<&str> {
        self.blocks.iter().find(|b| x >= b.x && x < b.x + b.w && y >= b.y && y < b.y + b.h).map(|b| b.id.as_str())
    }
}

/// A meeting that runs past midnight ends at the bottom of its day.
fn end_hours(m: &Meeting, clock: &Clock) -> f64 {
    let (start, end) = (clock.local(m.start), clock.local(m.end));
    if end.date() > start.date() { 24.0 } else { hours(end) }
}

/// Spans (top, bottom) by start, each with its lane and how many lanes its overlapping group takes.
fn lanes(spans: &[(f64, f64)]) -> Vec<(usize, usize)> {
    let mut out = vec![(0, 1); spans.len()];
    let mut group: Vec<usize> = Vec::new();
    let mut ends: Vec<f64> = Vec::new();
    let mut group_end = f64::MIN;
    for (n, &(top, bottom)) in spans.iter().enumerate() {
        if top >= group_end {
            for &k in &group {
                out[k].1 = ends.len().max(1);
            }
            group.clear();
            ends.clear();
        }
        let lane = ends.iter().position(|end| *end <= top).unwrap_or(ends.len());
        if lane == ends.len() {
            ends.push(bottom);
        } else {
            ends[lane] = bottom;
        }
        out[n].0 = lane;
        group.push(n);
        group_end = if group.len() == 1 { bottom } else { group_end.max(bottom) };
    }
    for &k in &group {
        out[k].1 = ends.len().max(1);
    }
    out
}

fn esc(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The week drawn `width` by `height` pixels, `cell` the font's cell size in pixels.
pub fn svg(week: &Week, clock: &Clock, selected: Option<&str>, width: f64, height: f64, cell: (f64, f64)) -> String {
    let layout = Layout::new(week, clock, width, height, cell);
    let font = (cell.1 * 0.62).max(8.0);
    let mut out = String::new();
    let mut defs = String::new();
    let _ = write!(out, r#"<rect width="{width:.0}" height="{height:.0}" fill="{BG}"/>"#);
    let today = clock.today();
    for (i, day) in layout.days.iter().enumerate() {
        let x = layout.gutter + i as f64 * layout.column;
        let is_today = *day == today;
        if is_today {
            let _ = write!(out, r#"<rect x="{x:.1}" y="0" width="{:.1}" height="{height:.0}" fill="{TODAY_BG}"/>"#, layout.column);
        }
        let (color, weight) = if is_today { (ORANGE, "bold") } else { (FG, "normal") };
        let _ = write!(
            out,
            r#"<text x="{:.1}" y="{:.1}" font-size="{font:.1}" fill="{color}" font-weight="{weight}">{}</text>"#,
            x + 4.0,
            cell.1 * 1.2,
            esc(&short_day(*day, clock))
        );
        let _ = write!(out, r#"<line x1="{x:.1}" y1="{:.1}" x2="{x:.1}" y2="{height:.0}" stroke="{GRID}"/>"#, layout.header - 4.0);
    }
    let mut hour = layout.from.ceil() as i64;
    while (hour as f64) <= layout.to {
        let y = layout.y(hour as f64);
        let _ = write!(out, r#"<line x1="{:.1}" y1="{y:.1}" x2="{width:.0}" y2="{y:.1}" stroke="{GRID}"/>"#, layout.gutter - 4.0);
        let _ = write!(out, r#"<text x="{:.1}" y="{:.1}" font-size="{:.1}" fill="{LABEL}">{hour:02}:00</text>"#, cell.0 * 0.5, y + font * 0.35, font * 0.9);
        hour += 1;
    }
    for (n, block) in layout.blocks.iter().enumerate() {
        let Some(m) = week.meetings.iter().find(|m| m.id == block.id) else { continue };
        let (fill, stroke, text, dash) = if m.is_now(clock) {
            (ORANGE, ORANGE, BG, "")
        } else if m.is_past(clock) {
            (PAST, PAST, LABEL, "")
        } else if m.response == "accepted" {
            (UPCOMING, ORANGE, FG, "")
        } else {
            (MAYBE, LABEL, LABEL, r#" stroke-dasharray="4 3""#)
        };
        let (stroke, stroke_w) = if selected == Some(m.id.as_str()) { (SELECTED, 2.5) } else { (stroke, 1.0) };
        let Block { x, y, w, h, .. } = *block;
        let _ = write!(
            out,
            r#"<rect x="{x:.1}" y="{y:.1}" width="{w:.1}" height="{h:.1}" rx="3" fill="{fill}" stroke="{stroke}" stroke-width="{stroke_w}"{dash}/>"#
        );
        let _ = write!(defs, r#"<clipPath id="b{n}"><rect x="{x:.1}" y="{y:.1}" width="{:.1}" height="{h:.1}"/></clipPath>"#, (w - 3.0).max(0.0));
        let line = font * 1.15;
        if h >= font * 0.8 {
            let ty = y + font.min(h - 1.0).max(font * 0.8);
            let _ = write!(out, r#"<g clip-path="url(#b{n})"><text x="{:.1}" y="{ty:.1}" font-size="{font:.1}" fill="{text}">{}</text>"#, x + 4.0, esc(&m.title));
            if h >= line * 2.0 {
                let _ = write!(out, r#"<text x="{:.1}" y="{:.1}" font-size="{:.1}" fill="{text}" opacity="0.75">{}</text>"#, x + 4.0, ty + line, font * 0.85, m.span_text(clock));
            }
            out.push_str("</g>");
        }
    }
    if let Some(i) = layout.days.iter().position(|d| *d == today) {
        let now = hours(clock.now_local());
        if (layout.from..=layout.to).contains(&now) {
            let (x, y) = (layout.gutter + i as f64 * layout.column, layout.y(now));
            let _ = write!(out, r#"<line x1="{x:.1}" y1="{y:.1}" x2="{:.1}" y2="{y:.1}" stroke="{RED}" stroke-width="2"/>"#, x + layout.column);
            let _ = write!(out, r#"<circle cx="{x:.1}" cy="{y:.1}" r="4" fill="{RED}"/>"#);
        }
    }
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width:.0}" height="{height:.0}" viewBox="0 0 {width:.0} {height:.0}" font-family="{SANS}"><defs>{defs}</defs>{out}</svg>"#
    )
}
