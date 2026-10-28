//! Text measured in terminal cells.

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub fn width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

/// Cuts or pads text to exactly `cells` terminal cells, an ellipsis marking a cut.
pub fn fit(text: &str, cells: usize) -> String {
    if cells == 0 {
        return String::new();
    }
    if width(text) <= cells {
        return format!("{text}{}", " ".repeat(cells - width(text)));
    }
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        if used + w > cells - 1 {
            break;
        }
        out.push(c);
        used += w;
    }
    format!("{out}…{}", " ".repeat(cells - 1 - used))
}

/// Words wrapped to `cells`, a word longer than a line split across lines.
pub fn wrap(text: &str, cells: usize) -> Vec<String> {
    let cells = cells.max(4);
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        for word in paragraph.split(' ') {
            let mut word = word.to_string();
            while width(&word) > cells {
                if !line.is_empty() {
                    lines.push(std::mem::take(&mut line));
                }
                let mut head = String::new();
                let mut rest = String::new();
                for c in word.chars() {
                    if width(&head) + c.width().unwrap_or(0) <= cells && rest.is_empty() {
                        head.push(c);
                    } else {
                        rest.push(c);
                    }
                }
                lines.push(head);
                word = rest;
            }
            let extra = if line.is_empty() { 0 } else { 1 };
            if width(&line) + extra + width(&word) > cells {
                lines.push(std::mem::take(&mut line));
            } else if extra == 1 {
                line.push(' ');
            }
            line.push_str(&word);
        }
        lines.push(line);
    }
    lines
}
