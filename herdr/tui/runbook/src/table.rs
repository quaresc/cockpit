//! The step table as text: a line's cells between its unescaped pipes, one cell rewritten in place, a step inserted and
//! the numbers after it shifted, everything else in the file left byte for byte as it was.

use std::ops::Range;

/// A step row's columns, in the file's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Column {
    N = 0,
    Step = 1,
    Where = 2,
    Status = 3,
    Command = 4,
}

/// The status a step added by hand starts with.
pub const NEW_STATUS: &str = "⬜ to do";

/// The byte ranges of a table line's cells: the outer pipes left out, a `\|` kept inside its cell.
pub fn cells(line: &str) -> Vec<Range<usize>> {
    let bytes = line.as_bytes();
    let mut start = line.len() - line.trim_start().len();
    let mut end = line.trim_end().len().max(start);
    while start < end && bytes[start] == b'|' {
        start += 1;
    }
    while end > start && bytes[end - 1] == b'|' && !(end >= 2 && bytes[end - 2] == b'\\') {
        end -= 1;
    }
    let mut out = Vec::new();
    let mut from = start;
    for i in start..end {
        if bytes[i] == b'|' && i > start && bytes[i - 1] != b'\\' {
            out.push(from..i);
            from = i + 1;
        }
    }
    out.push(from..end);
    out
}

/// A cell's value as the file means it: trimmed, its escaped pipes unescaped.
pub fn value(raw: &str) -> String {
    raw.trim().replace("\\|", "|")
}

/// The values of a line that is a step row, as `runbook.parse` tells one: a table line of four cells or more that is
/// neither the header nor its rule.
pub fn step_cells(line: &str) -> Option<Vec<String>> {
    if !line.trim_start().starts_with('|') {
        return None;
    }
    let values: Vec<String> = cells(line).into_iter().map(|r| value(&line[r])).collect();
    let rule = values.concat().chars().all(|c| "-: ".contains(c));
    (values.len() >= 4 && !rule && values[0] != "#").then_some(values)
}

/// What a typed value becomes in a cell: one line, trimmed, and a command without the backticks the file wraps it in.
pub fn clean(column: Column, value: &str) -> String {
    let line: String = value.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    match column {
        Column::Command => line.trim_matches(|c: char| c == '`' || c.is_whitespace()).to_string(),
        _ => line.trim().to_string(),
    }
}

/// The value as written between the pipes: escaped, a command in backticks.
fn render(column: Column, value: &str) -> String {
    let value = clean(column, value).replace('|', "\\|");
    if column == Column::Command && !value.is_empty() { format!("`{value}`") } else { value }
}

/// The line with one cell replaced, its padding kept (a single space where it had none).
fn replace(line: &str, range: Range<usize>, text: &str) -> String {
    let raw = &line[range.clone()];
    let lead = &raw[..raw.len() - raw.trim_start().len()];
    let trail = if raw.trim().is_empty() { "" } else { &raw[raw.trim_end().len()..] };
    let (lead, trail) = (if lead.is_empty() { " " } else { lead }, if trail.is_empty() { " " } else { trail });
    format!("{}{lead}{text}{trail}{}", &line[..range.start], &line[range.end..])
}

/// The line with this column set; a command column added when the row has none.
fn set_in_line(line: &str, column: Column, value: &str) -> String {
    let ranges = cells(line);
    let text = render(column, value);
    match ranges.get(column as usize) {
        Some(range) => replace(line, range.clone(), &text),
        None => {
            let end = ranges.last().map_or(line.len(), |r| r.end);
            let missing = "|  ".repeat(column as usize + 1 - ranges.len());
            format!("{}{}{text} {}", &line[..end], &missing[..missing.len() - 1], &line[end..])
        }
    }
}

/// The index of the first line that is the step numbered `n`.
fn find(lines: &[&str], n: &str) -> Option<usize> {
    lines.iter().position(|line| step_cells(line).is_some_and(|cells| cells[0] == n))
}

/// The file with step `n`'s cell in this column set to `value`; None when no row is numbered `n`.
pub fn set_cell(text: &str, n: &str, column: Column, value: &str) -> Option<String> {
    let mut lines: Vec<String> = text.split('\n').map(str::to_string).collect();
    let i = find(&lines.iter().map(String::as_str).collect::<Vec<_>>(), n)?;
    lines[i] = set_in_line(&lines[i], column, value);
    Some(lines.join("\n"))
}

/// The file with each of these steps' status set to done on `date`, as `runbook.mark_done` writes it; the steps found.
pub fn mark_done(text: &str, ns: &[String], date: &str) -> (String, Vec<String>) {
    let mut text = text.to_string();
    let mut found = Vec::new();
    for n in ns {
        if let Some(next) = set_cell(&text, n, Column::Status, &format!("✅ done {date}")) {
            text = next;
            found.push(n.clone());
        }
    }
    (text, found)
}

/// A step number split into its leading digits and what follows them: `4b` is (4, "b").
fn split_n(n: &str) -> Option<(u64, &str)> {
    let digits = n.len() - n.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    Some((n[..digits].parse().ok()?, &n[digits..]))
}

/// How a later step's number moves to make room for an inserted one; None when it stays.
type Shift = Box<dyn Fn(&str) -> Option<String>>;

/// The number a step inserted after `after` takes, and how a later step's number shifts to make room for it.
fn numbering(after: Option<&str>) -> (String, Shift) {
    match after.and_then(split_n) {
        // After 4b comes 4c, and the 4c, 4d… that were there move one letter on.
        Some((d, suffix)) if suffix.len() == 1 && suffix.as_bytes()[0].is_ascii_lowercase() && suffix != "z" => {
            let next = (suffix.as_bytes()[0] + 1) as char;
            let shift = move |n: &str| {
                let (nd, ns) = split_n(n)?;
                let c = *ns.as_bytes().first()?;
                (nd == d && ns.len() == 1 && c.is_ascii_lowercase() && c >= next as u8 && c < b'z').then(|| format!("{nd}{}", (c + 1) as char))
            };
            (format!("{d}{next}"), Box::new(shift))
        }
        found => {
            let new = found.map_or(1, |(d, _)| d + 1);
            let shift = move |n: &str| {
                let (nd, ns) = split_n(n)?;
                (nd >= new).then(|| format!("{}{ns}", nd + 1))
            };
            (new.to_string(), Box::new(shift))
        }
    }
}

/// The file with a step added under step `after` (at the end of the table when None), the steps below it renumbered;
/// and the new step's number.
pub fn insert_step(text: &str, after: Option<&str>, step: &str, where_: &str, command: &str) -> (String, String) {
    let mut lines: Vec<String> = text.split('\n').map(str::to_string).collect();
    let rows: Vec<usize> = (0..lines.len()).filter(|&i| step_cells(&lines[i]).is_some()).collect();
    let at = after.and_then(|n| find(&lines.iter().map(String::as_str).collect::<Vec<_>>(), n)).or(rows.last().copied());
    let after = at.and_then(|i| step_cells(&lines[i])).map(|cells| cells[0].clone());
    let (n, shift) = numbering(after.as_deref());
    for &i in rows.iter().filter(|&&i| at.is_some_and(|at| i > at)) {
        if let Some(next) = step_cells(&lines[i]).and_then(|cells| shift(&cells[0])) {
            lines[i] = set_in_line(&lines[i], Column::N, &next);
        }
    }
    let indent = at.map_or("", |i| &lines[i][..lines[i].len() - lines[i].trim_start().len()]).to_string();
    let cells = [n.clone(), render(Column::Step, step), render(Column::Where, where_), NEW_STATUS.to_string(), render(Column::Command, command)];
    let row = format!("{indent}| {} |", cells.join(" | "));
    match at {
        Some(i) => lines.insert(i + 1, row),
        None => {
            // No step yet: under the table's rule, or a new table at the end.
            match lines.iter().position(|l| l.trim_start().starts_with('|') && step_cells(l).is_none() && l.contains("--")) {
                Some(rule) => lines.insert(rule + 1, row),
                None => {
                    while lines.last().is_some_and(|l| l.trim().is_empty()) {
                        lines.pop();
                    }
                    lines.extend(["".into(), "| # | Step | Where | Status | Command |".into(), "|---|---|---|---|---|".into(), row, "".into()]);
                }
            }
        }
    }
    (lines.join("\n"), n)
}

/// Writes through a temporary file, so the watcher never reads half a runbook.
pub fn write(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("md.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = "# T\n\n| # | Step | Where | Status | Command |\n|---|---|---|---|---|\n| 1 | a | x | ⬜ to do | |\n| 2 | b | y#2 | ✅ done | `ls \\| wc` |\n| 3 | c | z | ⬜ to do |\n";

    #[test]
    fn a_cell_changes_and_nothing_else() {
        let out = set_cell(TABLE, "1", Column::Command, "git grep 'a|b'").unwrap();
        assert_eq!(out, TABLE.replace("| 1 | a | x | ⬜ to do | |", "| 1 | a | x | ⬜ to do | `git grep 'a\\|b'` |"));
        assert_eq!(step_cells(out.lines().nth(4).unwrap()).unwrap()[4], "`git grep 'a|b'`");
    }

    #[test]
    fn a_row_without_a_command_gets_one() {
        let out = set_cell(TABLE, "3", Column::Command, "ls").unwrap();
        assert!(out.contains("| 3 | c | z | ⬜ to do | `ls` |"), "{out}");
    }

    #[test]
    fn done_writes_the_status_like_python() {
        let (out, found) = mark_done(TABLE, &["1".into(), "9".into()], "28 Oct");
        assert_eq!(found, ["1"]);
        assert!(out.contains("| 1 | a | x | ✅ done 28 Oct | |"));
    }

    #[test]
    fn inserting_renumbers_what_follows() {
        let (out, n) = insert_step(TABLE, Some("1"), "new", "here", "");
        assert_eq!(n, "2");
        let ns: Vec<String> = out.lines().filter_map(step_cells).map(|c| c[0].clone()).collect();
        assert_eq!(ns, ["1", "2", "3", "4"]);
        assert!(out.contains("| 2 | new | here | ⬜ to do |  |"), "{out}");
    }

    #[test]
    fn inserting_after_a_lettered_step_takes_the_next_letter() {
        let text = "| 4a | a | x | ⬜ |\n| 4b | b | x | ⬜ |\n| 4c | c | x | ⬜ |\n| 5 | d | x | ⬜ |";
        let (out, n) = insert_step(text, Some("4a"), "new", "", "");
        assert_eq!(n, "4b");
        let ns: Vec<String> = out.lines().filter_map(step_cells).map(|c| c[0].clone()).collect();
        assert_eq!(ns, ["4a", "4b", "4c", "4d", "5"]);
    }

    #[test]
    fn a_runbook_without_a_table_gets_one() {
        let (out, n) = insert_step("# T\nupdated: now\n", None, "first", "", "ls");
        assert_eq!(n, "1");
        assert!(out.ends_with("| 1 | first |  | ⬜ to do | `ls` |\n"), "{out}");
    }
}
