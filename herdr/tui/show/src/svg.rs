//! graphics.py in Rust: line charts, bar charts and flow charts as SVG, every size in pixels of the pane so a chart
//! drawn at the terminal's cell size fills whole cells; `font` is the label size.

use std::collections::{HashMap, HashSet};
use std::fmt::Write;

use serde_json::Value;

use crate::value::{at, axes, cell, compact, number, rows, text};

const BG: &str = "#101010";
const PANEL: &str = "#1E1E22";
const GRID: &str = "#2A2A2E";
const AXIS: &str = "#6E6E74";
const TEXT: &str = "#9A9AA0";
const FG: &str = "#D2D2D2";
const SERIES: [&str; 5] = ["#FFC799", "#99FFE4", "#A0A0FF", "#FF99CC", "#FF8080"];
const SANS: &str = "Helvetica Neue, Helvetica, Arial, sans-serif";

/// A label's width, estimated: resvg lays text out itself, so this only has to leave enough room.
fn text_width(text: &str, font: f64) -> f64 {
    text.chars().map(|c| if c.is_uppercase() || c.is_ascii_digit() { 0.62 } else { 0.5 }).sum::<f64>() * font
}

fn esc(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#x27;")
}

/// Round tick values covering lo..hi: 0, 200, 400… rather than 0, 213.3, 426.7…
fn ticks(lo: f64, hi: f64, count: f64) -> Vec<f64> {
    let span = if hi - lo == 0.0 { 1.0 } else { hi - lo };
    let mut step = 10f64.powf((span / count).log10().floor());
    step *= [1.0, 2.0, 2.5, 5.0, 10.0].into_iter().find(|m| span / (step * m) <= count).unwrap_or(10.0);
    let (start, end) = ((lo / step).floor() * step, (hi / step).ceil() * step);
    (0..=((end - start) / step).round() as usize).map(|i| start + i as f64 * step).collect()
}

fn svg(width: f64, height: f64, body: &str, defs: &str) -> String {
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width:.0}" height="{height:.0}" viewBox="0 0 {width:.0} {height:.0}" font-family="{SANS}"><defs>{defs}</defs>{body}</svg>"#
    )
}

fn legend(names: &[String], mut x: f64, y: f64, font: f64) -> String {
    let mut out = String::new();
    for (n, name) in names.iter().enumerate() {
        let color = SERIES[n % SERIES.len()];
        let _ = write!(
            out,
            r#"<circle cx="{:.1}" cy="{:.1}" r="{:.1}" fill="{color}"/><text x="{:.1}" y="{y:.1}" font-size="{font:.1}" fill="{TEXT}">{}</text>"#,
            x + font * 0.35,
            y - font * 0.32,
            font * 0.3,
            x + font * 0.9,
            esc(name)
        );
        x += font * 2.0 + text_width(name, font);
    }
    out
}

fn values(block: &Value, ys: &[usize]) -> Vec<f64> {
    rows(block).iter().flat_map(|r| ys.iter().filter_map(|&i| number(at(r, i))).collect::<Vec<_>>()).collect()
}

/// Lines over a soft area, round y ticks, the last value of each series at its end.
pub fn line_chart(block: &Value, width: f64, height: f64, font: f64) -> Option<String> {
    let rows = rows(block);
    let (xi, ys, names) = axes(block);
    let values = values(block, &ys);
    if rows.is_empty() || values.is_empty() {
        return None;
    }
    let marks = ticks(values.iter().copied().fold(0.0, f64::min), values.iter().copied().fold(f64::MIN, f64::max), 4.0);
    let (lo, hi) = (marks[0], marks[marks.len() - 1]);
    let s = font / 16.0;
    let left = marks.iter().map(|m| text_width(&compact(*m), font)).fold(0.0, f64::max) + font * 1.2;
    let right = text_width(&compact(values[values.len() - 1]), font) + font * 1.4;
    let (top, bottom) = (font * if ys.len() > 1 { 2.4 } else { 1.2 }, font * 2.2);
    let (pw, ph) = (width - left - right, height - top - bottom);
    let last_k = rows.len().saturating_sub(1);
    let x_of = |k: usize| left + pw * k as f64 / last_k.max(1) as f64;
    let y_of = |v: f64| top + ph * (1.0 - (v - lo) / if hi - lo == 0.0 { 1.0 } else { hi - lo });
    let mut body = format!(r#"<rect width="{width}" height="{height}" fill="{BG}"/>"#);
    let mut defs = String::new();
    for &m in &marks {
        let y = y_of(m);
        let dash = if m == lo { String::new() } else { format!(r#" stroke-dasharray="{:.1} {:.1}""#, 4.0 * s, 4.0 * s) };
        let _ = write!(body, r#"<line x1="{left:.1}" x2="{:.1}" y1="{y:.1}" y2="{y:.1}" stroke="{GRID}" stroke-width="{s:.2}"{dash}/>"#, left + pw);
        let _ = write!(
            body,
            r#"<text x="{:.1}" y="{:.1}" font-size="{font:.1}" fill="{AXIS}" text-anchor="end">{}</text>"#,
            left - font * 0.6,
            y + font * 0.35,
            esc(&compact(m))
        );
    }
    let shown = (rows.len().min((pw / (font * 6.0)).floor().max(0.0) as usize)).max(2);
    let picked: std::collections::BTreeSet<usize> = (0..shown).map(|i| (i as f64 * last_k as f64 / (shown - 1) as f64).round() as usize).collect();
    for k in picked {
        let anchor = if k == 0 { "start" } else if k == last_k { "end" } else { "middle" };
        let _ = write!(
            body,
            r#"<text x="{:.1}" y="{:.1}" font-size="{font:.1}" fill="{AXIS}" text-anchor="{anchor}">{}</text>"#,
            x_of(k),
            height - font * 0.6,
            esc(&cell(at(&rows[k], xi)))
        );
    }
    for (n, &i) in ys.iter().enumerate() {
        let color = SERIES[n % SERIES.len()];
        let points: Vec<(f64, f64, f64)> = rows.iter().enumerate().filter_map(|(k, r)| number(at(r, i)).map(|v| (x_of(k), y_of(v), v))).collect();
        let Some(&(lx, ly, last)) = points.last() else { continue };
        let line: Vec<String> = points.iter().enumerate().map(|(j, (x, y, _))| format!("{}{x:.1},{y:.1}", if j == 0 { 'M' } else { 'L' })).collect();
        let line = line.join(" ");
        if n == 0 {
            let _ = write!(
                defs,
                r#"<linearGradient id="area" x1="0" x2="0" y1="0" y2="1"><stop offset="0" stop-color="{color}" stop-opacity="0.28"/><stop offset="1" stop-color="{color}" stop-opacity="0"/></linearGradient>"#
            );
            let _ = write!(body, r#"<path d="{line} L{lx:.1},{:.1} L{:.1},{:.1} Z" fill="url(#area)"/>"#, y_of(lo), points[0].0, y_of(lo));
        }
        let _ = write!(body, r#"<path d="{line}" fill="none" stroke="{color}" stroke-width="{:.2}" stroke-linejoin="round" stroke-linecap="round"/>"#, 2.4 * s);
        let _ = write!(
            body,
            r#"<circle cx="{lx:.1}" cy="{ly:.1}" r="{:.1}" fill="{BG}" stroke="{color}" stroke-width="{:.2}"/><text x="{:.1}" y="{:.1}" font-size="{font:.1}" fill="{color}" font-weight="600">{}</text>"#,
            5.0 * s,
            2.4 * s,
            lx + font * 0.6,
            ly + font * 0.35,
            esc(&compact(last))
        );
    }
    if ys.len() > 1 {
        body.push_str(&legend(&names, left, font * 1.2, font));
    }
    Some(svg(width, height, &body, &defs))
}

/// Horizontal bars on a faint track, the value at each bar's end; returns the SVG and its height.
pub fn bar_chart(block: &Value, width: f64, font: f64) -> Option<(String, f64)> {
    let rows = rows(block);
    let (xi, ys, names) = axes(block);
    let values = values(block, &ys);
    if rows.is_empty() || values.is_empty() {
        return None;
    }
    let top_value = Some(values.iter().copied().fold(0.0, f64::max)).filter(|v| *v != 0.0).unwrap_or(1.0);
    let (bar, gap) = (font * 1.15, font * 0.55);
    let widest = rows.iter().map(|r| text_width(&cell(at(r, xi)), font)).fold(0.0, f64::max);
    let label = text_width(&"M".repeat(22), font).min(widest) + font;
    let value_room = values.iter().map(|v| text_width(&compact(*v), font)).fold(0.0, f64::max) + font * 1.2;
    let top = if ys.len() > 1 { font * 2.4 } else { font * 0.4 };
    let track = width - label - value_room;
    let mut body = String::new();
    let mut y = top;
    for r in &rows {
        let _ = write!(
            body,
            r#"<text x="{:.1}" y="{:.1}" font-size="{font:.1}" fill="{FG}" text-anchor="end">{}</text>"#,
            label - font * 0.6,
            y + bar * 0.5 * ys.len() as f64 + font * 0.35,
            esc(&cell(at(r, xi)))
        );
        for (n, &i) in ys.iter().enumerate() {
            let value = number(at(r, i));
            let color = SERIES[n % SERIES.len()];
            let length = (bar * 0.4).max(track * value.unwrap_or(0.0).max(0.0) / top_value);
            let shown = value.map_or_else(|| "—".to_string(), |v| esc(&compact(v)));
            let _ = write!(
                body,
                r#"<rect x="{label:.1}" y="{y:.1}" width="{track:.1}" height="{h:.1}" rx="{rx:.1}" fill="{PANEL}"/><rect x="{label:.1}" y="{y:.1}" width="{length:.1}" height="{h:.1}" rx="{rx:.1}" fill="{color}" fill-opacity="0.9"/><text x="{:.1}" y="{:.1}" font-size="{font:.1}" fill="{TEXT}">{shown}</text>"#,
                label + length + font * 0.5,
                y + bar * 0.41 + font * 0.35,
                h = bar * 0.82,
                rx = bar * 0.3,
            );
            y += bar;
        }
        y += gap;
    }
    if ys.len() > 1 {
        body.insert_str(0, &legend(&names, label, font * 1.3, font));
    }
    let height = y + font * 0.2;
    Some((svg(width, height, &format!(r#"<rect width="{width}" height="{height:.0}" fill="{BG}"/>{body}"#), ""), height))
}

/// Each node's layer, the longest path to it over forward edges; the edges that close a cycle point backwards.
fn layers_of(ids: &[String], edges: &[(String, String)]) -> (HashMap<String, usize>, HashSet<(String, String)>) {
    let mut after: HashMap<&str, Vec<&str>> = ids.iter().map(|i| (i.as_str(), Vec::new())).collect();
    for (a, b) in edges {
        after.entry(a).or_default().push(b);
    }
    fn visit<'a>(n: &'a str, after: &HashMap<&'a str, Vec<&'a str>>, state: &mut HashMap<&'a str, bool>, back: &mut HashSet<(String, String)>) {
        state.insert(n, false);
        for &m in after.get(n).into_iter().flatten() {
            match state.get(m) {
                Some(false) => {
                    back.insert((n.to_string(), m.to_string()));
                }
                None => visit(m, after, state, back),
                Some(true) => {}
            }
        }
        state.insert(n, true);
    }
    let (mut state, mut back) = (HashMap::new(), HashSet::new());
    for n in ids {
        if !state.contains_key(n.as_str()) {
            visit(n, &after, &mut state, &mut back);
        }
    }
    let mut layer: HashMap<String, usize> = ids.iter().map(|i| (i.clone(), 0)).collect();
    for _ in ids {
        for (a, b) in edges {
            if !back.contains(&(a.clone(), b.clone())) && a != b {
                let next = layer[a] + 1;
                let to = layer.get_mut(b).unwrap();
                *to = (*to).max(next);
            }
        }
    }
    (layer, back)
}

/// Boxes in layers joined by curves with arrowheads; returns the SVG and its natural width and height.
pub fn flow_chart(block: &Value, font: f64) -> (String, f64, f64) {
    let nodes: Vec<(String, String, String)> = block["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|n| (text(&n["id"]), n["label"].as_str().map_or_else(|| text(&n["id"]), str::to_string), n["kind"].as_str().unwrap_or("box").to_string()))
        .collect();
    let ids: Vec<String> = nodes.iter().map(|n| n.0.clone()).collect();
    let edges: Vec<(String, String, Option<String>)> = block["edges"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|e| (text(&e["from"]), text(&e["to"]), e["label"].as_str().filter(|l| !l.is_empty()).map(str::to_string)))
        .filter(|(a, b, _)| ids.contains(a) && ids.contains(b))
        .collect();
    let down = block["direction"].as_str() != Some("right");
    let pairs: Vec<(String, String)> = edges.iter().map(|(a, b, _)| (a.clone(), b.clone())).collect();
    let (layer, back) = layers_of(&ids, &pairs);
    let (pad, s) = (font * 0.9, font / 16.0);
    let size: HashMap<&str, (f64, f64)> = nodes
        .iter()
        .map(|(id, label, kind)| (id.as_str(), (text_width(label, font) + pad * 2.4 + if kind == "decision" { font * 1.6 } else { 0.0 }, font * 2.6)))
        .collect();
    let mut order: Vec<Vec<&str>> = vec![Vec::new(); layer.values().copied().max().map_or(0, |m| m + 1)];
    for (id, _, _) in &nodes {
        order[layer[id]].push(id);
    }
    let (gap_main, gap_cross) = (font * if down { 3.2 } else { 5.5 }, font * 1.4);
    let main_of = |i: &str| if down { size[i].1 } else { size[i].0 };
    let cross_of = |i: &str| if down { size[i].0 } else { size[i].1 };
    let mains: Vec<f64> = order.iter().map(|ids| ids.iter().map(|i| main_of(i)).fold(0.0, f64::max)).collect();
    let crosses: Vec<f64> = order.iter().map(|ids| ids.iter().map(|i| cross_of(i)).sum::<f64>() + gap_cross * (ids.len() as f64 - 1.0)).collect();
    let widest = crosses.iter().copied().fold(0.0, f64::max);
    let margin = font * 1.2;
    let mut centre: HashMap<&str, (f64, f64)> = HashMap::new();
    let mut position = margin;
    for (l, ids) in order.iter().enumerate() {
        let mut c = margin + (widest - crosses[l]) / 2.0;
        for &i in ids {
            let (mid_main, mid_cross) = (position + mains[l] / 2.0, c + cross_of(i) / 2.0);
            centre.insert(i, if down { (mid_cross, mid_main) } else { (mid_main, mid_cross) });
            c += cross_of(i) + gap_cross;
        }
        position += mains[l] + gap_main;
    }
    let total_main = position - gap_main + margin;
    let (width, height) = if down { (widest + 2.0 * margin, total_main) } else { (total_main, widest + 2.0 * margin) };
    let defs = format!(
        r#"<marker id="arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="6.0" markerHeight="6.0" orient="auto-start-reverse"><path d="M0,0 L10,5 L0,10 z" fill="{AXIS}"/></marker>"#
    );
    let mut body = format!(r#"<rect width="{width:.0}" height="{height:.0}" fill="{BG}"/>"#);
    let anchor = |i: &str, outgoing: bool| {
        let ((x, y), (w, h)) = (centre[i], size[i]);
        let sign = if outgoing { 1.0 } else { -1.0 };
        if down { (x, y + sign * h / 2.0) } else { (x + sign * w / 2.0, y) }
    };
    for (a, b, label) in &edges {
        let is_back = back.contains(&(a.clone(), b.clone())) || a == b;
        let ((x1, y1), (x2, y2)) = (anchor(a, !is_back), anchor(b, is_back));
        let path = if down {
            let bend = (font * 1.5).max((y2 - y1).abs() / 2.0);
            format!("M{x1:.1},{y1:.1} C{x1:.1},{:.1} {x2:.1},{:.1} {x2:.1},{y2:.1}", y1 + bend, y2 - bend)
        } else {
            let bend = (font * 1.5).max((x2 - x1).abs() / 2.0);
            format!("M{x1:.1},{y1:.1} C{:.1},{y1:.1} {:.1},{y2:.1} {x2:.1},{y2:.1}", x1 + bend, x2 - bend)
        };
        let dash = if is_back { format!(r#" stroke-dasharray="{:.1} {:.1}""#, 5.0 * s, 4.0 * s) } else { String::new() };
        let _ = write!(body, r#"<path d="{path}" fill="none" stroke="{AXIS}" stroke-width="{:.2}" marker-end="url(#arrow)"{dash}/>"#, 1.8 * s);
        if let Some(label) = label {
            let (mx, my) = ((x1 + x2) / 2.0, (y1 + y2) / 2.0);
            let w = text_width(label, font * 0.85) + font * 0.8;
            let _ = write!(
                body,
                r#"<rect x="{:.1}" y="{:.1}" width="{w:.1}" height="{:.1}" rx="{:.1}" fill="{BG}"/><text x="{mx:.1}" y="{:.1}" font-size="{:.1}" fill="{TEXT}" text-anchor="middle">{}</text>"#,
                mx - w / 2.0,
                my - font * 0.7,
                font * 1.3,
                font * 0.6,
                my + font * 0.3,
                font * 0.85,
                esc(label)
            );
        }
    }
    for (id, label, kind) in &nodes {
        let ((x, y), (w, h)) = (centre[id.as_str()], size[id.as_str()]);
        if kind == "decision" {
            let k = font * 0.8;
            let (l, r, t, b) = (x - w / 2.0, x + w / 2.0, y - h / 2.0, y + h / 2.0);
            let _ = write!(
                body,
                r#"<polygon points="{l:.1},{y:.1} {:.1},{t:.1} {:.1},{t:.1} {r:.1},{y:.1} {:.1},{b:.1} {:.1},{b:.1}" fill="{PANEL}" stroke="{}" stroke-width="{:.2}"/>"#,
                l + k,
                r - k,
                r - k,
                l + k,
                SERIES[1],
                1.8 * s
            );
        } else {
            let (radius, stroke) = if kind == "round" { (h / 2.0, SERIES[0]) } else { (6.0 * s, "#3A3A40") };
            let _ = write!(
                body,
                r#"<rect x="{:.1}" y="{:.1}" width="{w:.1}" height="{h:.1}" rx="{radius:.1}" fill="{PANEL}" stroke="{stroke}" stroke-width="{:.2}"/>"#,
                x - w / 2.0,
                y - h / 2.0,
                1.8 * s
            );
        }
        let _ = write!(body, r#"<text x="{x:.1}" y="{:.1}" font-size="{font:.1}" fill="{FG}" text-anchor="middle">{}</text>"#, y + font * 0.35, esc(label));
    }
    (svg(width, height, &body, &defs), width, height)
}

/// An SVG file's own width and height, from its width/height or its viewBox.
pub fn svg_size(text: &str) -> (f64, f64) {
    let head: String = text.chars().take(2000).collect();
    let find = |re: &str| regex::Regex::new(re).ok().and_then(|r| r.captures(&head)).map(|c| (c[1].parse().unwrap_or(0.0), c.get(2).map_or(0.0, |m| m.as_str().parse().unwrap_or(0.0))));
    if let (Some((w, _)), Some((h, _))) = (find(r#"\swidth="([\d.]+)"#), find(r#"\sheight="([\d.]+)"#)) {
        return (w, h);
    }
    find(r#"viewBox="[\d.\s-]*?([\d.]+)\s+([\d.]+)""#).unwrap_or((300.0, 150.0))
}
