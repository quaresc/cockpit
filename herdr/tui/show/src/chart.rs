//! What makes a chart more than a picture: the window of rows a zoom shows, the hovered point's values as a line under
//! it, and values that glide from the old rows to the new ones when a refresh changes them.

use ratatui::style::Style;
use serde_json::{Value, json};

use cockpit_core::theme::{FG, SERIES, bold, dim, fg};

use crate::row::Row;
use crate::value::{at, axes, cell, compact, number, rows};

/// The fewest points a zoom keeps.
const CLOSEST: usize = 3;

/// The block with only the rows of the window (start, length) when zoomed.
pub fn windowed(block: &Value, zoom: Option<(usize, usize)>) -> Value {
    let Some((start, len)) = zoom else { return block.clone() };
    let mut out = block.clone();
    out["rows"] = json!(rows(block).into_iter().skip(start).take(len).collect::<Vec<_>>());
    out
}

/// The window after a zoom in (`closer`) or out, centred on `centre` (else the window's middle); None shows them all.
pub fn zoom(window: Option<(usize, usize)>, total: usize, centre: Option<usize>, closer: bool) -> Option<(usize, usize)> {
    let (start, len) = window.unwrap_or((0, total));
    let centre = centre.unwrap_or(start + len / 2);
    let len = if closer { (len / 2).max(CLOSEST.min(total)) } else { len.saturating_mul(2) };
    if len >= total {
        return None;
    }
    Some((centre.saturating_sub(len / 2).min(total - len), len))
}

/// The hovered row's x and each series' value, in the series' colours.
pub fn tooltip(block: &Value, k: usize) -> Row {
    let rows = rows(block);
    let Some(r) = rows.get(k) else { return Row::new() };
    let (xi, ys, names) = axes(block);
    let mut row = Row::text("◆ ", fg(SERIES[0])).add(cell(at(r, xi)), bold(FG));
    for (n, (&i, name)) in ys.iter().zip(&names).enumerate() {
        let value = number(at(r, i)).map_or_else(|| "—".to_string(), compact);
        row = row.add("   ● ", fg(SERIES[n % SERIES.len()])).add(format!("{name} "), dim()).add(value, Style::new().fg(FG));
    }
    row
}

/// The new rows part way (`t` from 0 to 1) from the old ones: each value from the old row with the same x, or from
/// nothing (zero for bars, its own value for lines) for an x that is new.
pub fn between(from: &Value, to: &Value, t: f64, bars: bool) -> Value {
    let (xi, ys, _) = axes(to);
    let old = rows(from);
    let mut out = to.clone();
    out["rows"] = json!(
        rows(to)
            .into_iter()
            .map(|mut r| {
                let x = at(&r, xi).clone();
                let before = old.iter().find(|o| *at(o, xi) == x);
                for &i in &ys {
                    let Some(end) = number(at(&r, i)) else { continue };
                    let start = before.and_then(|o| number(at(o, i))).unwrap_or(if bars { 0.0 } else { end });
                    if let Some(slot) = r.get_mut(i) {
                        *slot = json!(start + (end - start) * t);
                    }
                }
                r
            })
            .collect::<Vec<_>>()
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_halves_around_the_point_and_back() {
        assert_eq!(zoom(None, 40, Some(30), true), Some((20, 20)));
        assert_eq!(zoom(Some((20, 20)), 40, Some(39), true), Some((30, 10)));
        assert_eq!(zoom(Some((30, 10)), 40, None, false), Some((20, 20)));
        assert_eq!(zoom(Some((20, 20)), 40, None, false), None);
        assert_eq!(zoom(Some((0, 3)), 40, None, true), Some((0, 3)));
        assert_eq!(zoom(None, 21, None, true), Some((5, 10)));
    }

    #[test]
    fn values_glide_by_x() {
        let from = json!({"columns": ["d", "v"], "rows": [["a", 10], ["b", 20]]});
        let to = json!({"columns": ["d", "v"], "rows": [["b", 40], ["c", 8]]});
        assert_eq!(between(&from, &to, 0.5, true)["rows"], json!([["b", 30.0], ["c", 4.0]]));
        assert_eq!(between(&from, &to, 0.5, false)["rows"], json!([["b", 30.0], ["c", 8.0]]));
    }
}
