//! JSON values as show.py writes them in cells, on axes and in CSV.

use serde_json::Value;

pub fn number(value: &Value) -> Option<f64> {
    value.as_f64().filter(|_| value.is_number())
}

/// Thousands separators on the integer part, as Python's `{:,}`.
fn grouped(digits: &str) -> String {
    let (sign, digits) = digits.strip_prefix('-').map_or(("", digits), |d| ("-", d));
    let (int, frac) = digits.split_once('.').map_or((digits, None), |(i, f)| (i, Some(f)));
    let mut out = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    match frac {
        Some(f) => format!("{sign}{out}.{f}"),
        None => format!("{sign}{out}"),
    }
}

fn trimmed(text: String) -> String {
    if text.contains('.') { text.trim_end_matches('0').trim_end_matches('.').to_string() } else { text }
}

/// A table cell: 1,234 · 12.5 · yes · —.
pub fn cell(value: &Value) -> String {
    match value {
        Value::Null => "—".into(),
        Value::Bool(b) => if *b { "yes" } else { "no" }.into(),
        Value::Number(n) if n.is_f64() => trimmed(grouped(&format!("{:.2}", n.as_f64().unwrap_or(0.0)))),
        Value::Number(n) => grouped(&n.to_string()),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// A number in at most five or six characters: 1234 → 1.2k, 3400000 → 3.4M.
pub fn compact(value: f64) -> String {
    for (limit, suffix) in [(1e9, "B"), (1e6, "M"), (1e3, "k")] {
        if value.abs() >= limit {
            return trimmed(format!("{:.1}", value / limit)) + suffix;
        }
    }
    trimmed(format!("{value:.2}"))
}

/// The x column and the y columns: as given, else the first column against every other one.
pub fn axes(block: &Value) -> (usize, Vec<usize>, Vec<String>) {
    let columns: Vec<String> = block["columns"].as_array().into_iter().flatten().map(|c| c.as_str().map_or_else(|| c.to_string(), str::to_string)).collect();
    let x = block["x"].as_str().filter(|x| columns.iter().any(|c| c == x)).map_or_else(|| columns.first().cloned().unwrap_or_default(), str::to_string);
    let mut ys: Vec<String> = block["y"].as_array().into_iter().flatten().filter_map(|y| y.as_str()).filter(|y| columns.iter().any(|c| c == y)).map(str::to_string).collect();
    if ys.is_empty() {
        ys = columns.iter().filter(|c| **c != x).cloned().collect();
    }
    let index = |name: &str| columns.iter().position(|c| c == name).unwrap_or(0);
    (index(&x), ys.iter().map(|y| index(y)).collect(), ys)
}

pub fn rows(block: &Value) -> Vec<Vec<Value>> {
    block["rows"].as_array().into_iter().flatten().map(|r| r.as_array().cloned().unwrap_or_default()).collect()
}

pub fn at(row: &[Value], i: usize) -> &Value {
    row.get(i).unwrap_or(&Value::Null)
}

/// A column header or a string cell as text.
pub fn text(value: &Value) -> String {
    value.as_str().map_or_else(|| value.to_string(), str::to_string)
}

/// Rows as Python's csv module writes them: quoted only when needed, None empty, booleans True/False.
pub fn csv(columns: &[Value], rows: &[Vec<Value>]) -> String {
    let field = |v: &Value| {
        let text = match v {
            Value::Null => String::new(),
            Value::Bool(b) => if *b { "True" } else { "False" }.into(),
            other => text(other),
        };
        if text.contains([',', '"', '\n', '\r']) { format!("\"{}\"", text.replace('"', "\"\"")) } else { text }
    };
    std::iter::once(columns.to_vec()).chain(rows.iter().cloned()).map(|r| r.iter().map(field).collect::<Vec<_>>().join(",") + "\r\n").collect()
}
