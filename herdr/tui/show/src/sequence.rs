//! Sequence charts: one lifeline per actor, each message an arrow between two of them, a reply dashed, a message
//! to oneself looping back, a note beside its actor; drawn on a canvas whose line cells join into box-drawing corners.

use std::collections::HashMap;

use ratatui::style::{Modifier, Style};
use serde_json::Value;

use cockpit_core::text::width;
use cockpit_core::theme::{ORANGE, dim, fg};

const GAP: i64 = 4;
/// A line cell's directions: up, down, left, right.
const U: u8 = 1;
const D: u8 = 2;
const L: u8 = 4;
const R: u8 = 8;

/// The box-drawing character joining a cell's directions.
fn joint(bits: u8) -> char {
    const JOINTS: [char; 16] = [' ', '│', '│', '│', '─', '╯', '╮', '┤', '─', '╰', '╭', '├', '─', '┴', '┬', '┼'];
    JOINTS[usize::from(bits & 15)]
}

/// Cells of line directions, merged into box-drawing joins, under cells of text.
#[derive(Default)]
struct Canvas {
    lines: HashMap<(i64, i64), u8>,
    text: HashMap<(i64, i64), (char, Style)>,
}

impl Canvas {
    /// A straight line between two (row, col) cells, both included.
    fn segment(&mut self, (r0, c0): (i64, i64), (r1, c1): (i64, i64)) {
        if r0 == r1 {
            for c in c0.min(c1)..c0.max(c1) {
                *self.lines.entry((r0, c)).or_default() |= R;
                *self.lines.entry((r0, c + 1)).or_default() |= L;
            }
        } else {
            for r in r0.min(r1)..r0.max(r1) {
                *self.lines.entry((r, c0)).or_default() |= D;
                *self.lines.entry((r + 1, c0)).or_default() |= U;
            }
        }
    }

    fn put(&mut self, row: i64, mut col: i64, text: &str, style: Style) {
        for c in text.chars() {
            self.text.insert((row, col), (c, style));
            col += width(&c.to_string()) as i64;
        }
    }

    /// The canvas as rows of styled runs, each row's trailing spaces cut.
    fn rows(&self) -> Vec<Vec<(String, Style)>> {
        let cells = self.lines.keys().chain(self.text.keys());
        let (Some(height), Some(wide)) = (cells.clone().map(|c| c.0).max(), cells.map(|c| c.1).max()) else { return Vec::new() };
        (0..=height)
            .map(|r| {
                let mut runs: Vec<(String, Style)> = Vec::new();
                let mut c = 0;
                while c <= wide {
                    let (ch, style) = match (self.text.get(&(r, c)), self.lines.get(&(r, c))) {
                        (Some(&(ch, style)), _) => (ch, style),
                        (None, Some(&bits)) => (joint(bits), dim()),
                        _ => (' ', Style::new()),
                    };
                    match runs.last_mut() {
                        Some(last) if last.1 == style => last.0.push(ch),
                        _ => runs.push((ch.to_string(), style)),
                    }
                    c += width(&ch.to_string()).max(1) as i64;
                }
                if let Some(last) = runs.last_mut() {
                    last.0 = last.0.trim_end().to_string();
                }
                runs
            })
            .collect()
    }
}

/// Words filled greedily into lines of `room` cells, long words broken, nothing for an empty text.
fn wrap(text: &str, room: usize) -> Vec<String> {
    let options = textwrap::Options::new(room.max(1))
        .word_separator(textwrap::WordSeparator::AsciiSpace)
        .wrap_algorithm(textwrap::WrapAlgorithm::FirstFit);
    let lines: Vec<String> = textwrap::wrap(text.trim(), options).into_iter().map(|l| l.into_owned()).collect();
    if lines.iter().all(String::is_empty) { vec![String::new()] } else { lines }
}

struct Arrow {
    start: (i64, i64),
    end: (i64, i64),
    turn: Option<((i64, i64), (i64, i64))>,
    head: &'static str,
    reply: bool,
}

/// The diagram's rows for actors (names or {id, label}) and messages ({from, to, label, reply} or {note, over}), laid
/// out to fit `room` cells where it can.
pub fn draw(actors: &[Value], messages: &[Value], room: i64) -> Vec<Vec<(String, Style)>> {
    let id = |a: &Value| a.as_str().map_or_else(|| a["id"].as_str().unwrap_or_default().to_string(), str::to_string);
    let mut names: Vec<String> = actors.iter().map(id).collect();
    let mut labels: HashMap<String, String> = actors
        .iter()
        .filter(|a| a.is_object())
        .map(|a| (id(a), a["label"].as_str().filter(|l| !l.is_empty()).map_or_else(|| id(a), str::to_string)))
        .collect();
    for m in messages {
        for end in ["from", "to", "over"] {
            if let Some(end) = m[end].as_str().filter(|e| !e.is_empty())
                && !names.iter().any(|n| n == end)
            {
                names.push(end.to_string());
            }
        }
    }
    for name in &names {
        labels.entry(name.clone()).or_insert_with(|| name.clone());
    }
    let index = |n: &str| names.iter().position(|x| x == n);
    let boxes: HashMap<&str, i64> = names.iter().map(|a| (a.as_str(), width(&labels[a]) as i64 + 4)).collect();
    let mut spacing = vec![0i64; names.len().saturating_sub(1)];
    let label_of = |m: &Value| m["label"].as_str().unwrap_or_default().to_string();
    for m in messages {
        let (from, to) = (m["from"].as_str().and_then(index), m["to"].as_str().and_then(index));
        match (from, to) {
            (Some(i), Some(j)) if i != j => {
                let (i, j) = (i.min(j), i.max(j));
                let need = width(&label_of(m)) as i64 + 4;
                let per = (need + (j - i) as i64 - 1) / (j - i) as i64;
                for s in &mut spacing[i..j] {
                    *s = (*s).max(per);
                }
            }
            (Some(i), Some(_)) if i < spacing.len() => spacing[i] = spacing[i].max(width(&label_of(m)) as i64 + 4),
            _ => {}
        }
    }
    // Too wide for the pane: every gap shrinks to its share of the room, and the labels wrap to fit.
    let half_boxes = boxes.values().sum::<i64>() / 2;
    let least = half_boxes + GAP * spacing.len() as i64;
    if !spacing.is_empty() && least + spacing.iter().sum::<i64>() > room {
        let share = 14.max((room - half_boxes).div_euclid(spacing.len() as i64));
        spacing.iter_mut().for_each(|s| *s = (*s).min(share));
    }
    let mut centres: Vec<i64> = Vec::new();
    for (k, a) in names.iter().enumerate() {
        let at = match centres.last() {
            Some(&last) => (last + boxes[names[k - 1].as_str()] / 2 + boxes[a.as_str()] / 2 + GAP).max(last + spacing[k - 1] + 1),
            None => boxes[a.as_str()] / 2,
        };
        centres.push(at);
    }
    let centre: HashMap<&str, i64> = names.iter().map(String::as_str).zip(centres.iter().copied()).collect();
    let mut canvas = Canvas::default();
    let heads = |canvas: &mut Canvas, row: i64| {
        for a in &names {
            let (w, left) = (boxes[a.as_str()], centre[a.as_str()] - boxes[a.as_str()] / 2);
            let rule = "─".repeat((w - 2).max(0) as usize);
            canvas.put(row, left, &format!("┌{rule}┐"), fg(ORANGE));
            canvas.put(row + 1, left, "│", fg(ORANGE));
            canvas.put(row + 1, left + 2, &labels[a], Style::new().add_modifier(Modifier::BOLD));
            canvas.put(row + 1, left + w - 1, "│", fg(ORANGE));
            canvas.put(row + 2, left, &format!("└{rule}┘"), fg(ORANGE));
        }
    };
    heads(&mut canvas, 0);
    let mut row = 4;
    let mut arrows = Vec::new();
    for m in messages {
        if let Some(note) = m.get("note").filter(|n| !n.is_null()) {
            let at = m["over"].as_str().and_then(|o| centre.get(o)).copied().unwrap_or(0);
            canvas.put(row, at + 2, &format!("▏{}", note.as_str().map_or_else(|| note.to_string(), str::to_string)), dim());
            row += 2;
            continue;
        }
        let (Some(&ca), Some(&cb)) = (m["from"].as_str().and_then(|a| centre.get(a)), m["to"].as_str().and_then(|b| centre.get(b))) else { continue };
        let own = m["from"] == m["to"];
        let room_for_label = 8.max(if own { room - ca } else { (cb - ca).abs() } - 3);
        for line in wrap(&label_of(m), room_for_label as usize) {
            canvas.put(row, if own { ca } else { ca.min(cb) } + 2, &line, Style::new());
            row += 1;
        }
        row -= 1;
        let reply = m["reply"].as_bool().unwrap_or(false);
        if own {
            arrows.push(Arrow { start: (row + 1, ca), end: (row + 1, ca + 3), turn: Some(((row + 2, ca + 3), (row + 2, ca + 1))), head: "◀", reply });
            row += 4;
        } else {
            let head = if cb > ca { "▶" } else { "◀" };
            arrows.push(Arrow { start: (row + 1, ca), end: (row + 1, cb + if cb > ca { -2 } else { 2 }), turn: None, head, reply });
            row += 3;
        }
    }
    let bottom = row;
    for &c in &centres {
        canvas.segment((3, c), (bottom, c));
    }
    for arrow in &arrows {
        canvas.segment(arrow.start, arrow.end);
        let tip = match arrow.turn {
            Some((turn, back)) => {
                canvas.segment(arrow.end, turn);
                canvas.segment(turn, back);
                back
            }
            None => (arrow.end.0, arrow.end.1 + if arrow.head == "▶" { 1 } else { -1 }),
        };
        canvas.put(tip.0, tip.1, arrow.head, fg(ORANGE));
        if arrow.reply {
            let (r, c0, c1) = (arrow.start.0, arrow.start.1, arrow.end.1);
            for c in c0.min(c1)..=c0.max(c1) {
                if !centres.contains(&c) {
                    canvas.put(r, c, "╌", dim());
                }
            }
        }
    }
    heads(&mut canvas, bottom + 1);
    canvas.rows()
}
