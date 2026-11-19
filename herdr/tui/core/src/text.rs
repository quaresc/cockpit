//! Text measured in terminal cells, and untrusted text cleaned before a person sees it.

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

/// How many screen lines these lines take once wrapped to `cells`.
pub fn wrapped(lines: &[ratatui::text::Line], cells: u16) -> usize {
    lines
        .iter()
        .map(|line| {
            let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            wrap(&text, cells as usize).len().max(1)
        })
        .sum()
}

/// Text without its terminal CSI (`ESC [ … final`) and OSC (`ESC ] … BEL`, or ending `ESC \`) sequences, all else kept.
pub fn without_sequences(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, chars.peek()) {
            ('\u{1b}', Some('[')) => {
                chars.next();
                while chars.next().is_some_and(|p| !('@'..='~').contains(&p)) {}
            }
            ('\u{1b}', Some(']')) => {
                chars.next();
                while let Some(p) = chars.next() {
                    if p == '\u{7}' || (p == '\u{1b}' && chars.next_if_eq(&'\\').is_some()) {
                        break;
                    }
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// Characters with no use in shown text but to hide something: Unicode's format characters (zero widths, bidi
/// controls, the soft hyphen), private-use and non-characters, variation selectors and the letters that draw blank.
fn unseen(c: char) -> bool {
    let n = c as u32;
    c.is_control()
        || matches!(n, 0x00AD | 0x034F | 0x061C | 0x06DD | 0x070F | 0x115F | 0x1160 | 0x180E | 0x3164 | 0xFEFF | 0xFFA0)
        || matches!(n, 0x0600..=0x0605 | 0x0890..=0x0891 | 0x180B..=0x180F | 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x206F)
        || matches!(n, 0xFE00..=0xFE0F | 0xFFF9..=0xFFFB | 0xFDD0..=0xFDEF | 0xE000..=0xF8FF | 0x1BCA0..=0x1BCA3 | 0x1D173..=0x1D17A)
        || matches!(n, 0xE0100..=0xE01EF | 0xF0000..=0x10FFFF)
        || (n & 0xFFFE) == 0xFFFE
}

/// A combining mark, in the blocks that hold them: more than three in a row only pile up over a letter.
fn combining(c: char) -> bool {
    matches!(c as u32, 0x0300..=0x036F | 0x0483..=0x0489 | 0x0591..=0x05BD | 0x0610..=0x061A | 0x064B..=0x065F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF | 0xFE20..=0xFE2F)
}

/// What a person sees of untrusted text, at most `max` characters: escape sequences and unseen characters out, whitespace
/// folded to single spaces, at most three combining marks in a row. Text holding Unicode tag characters is refused whole.
pub fn clean(text: &str, max: usize) -> String {
    if text.chars().any(|c| (0xE0000..=0xE007F).contains(&(c as u32))) {
        return String::new();
    }
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    let mut marks = 0;
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            match chars.next() {
                // CSI: parameters and intermediates, then one final byte.
                Some('[') => {
                    while let Some(&p) = chars.peek() {
                        chars.next();
                        if ('@'..='~').contains(&p) {
                            break;
                        }
                    }
                }
                // OSC: up to BEL or ESC \.
                Some(']') => {
                    while let Some(p) = chars.next() {
                        if p == '\u{7}' || (p == '\u{1b}' && chars.peek() == Some(&'\\')) {
                            chars.next_if_eq(&'\\');
                            break;
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        if c.is_whitespace() {
            if !out.ends_with(' ') {
                out.push(' ');
            }
            marks = 0;
            continue;
        }
        if unseen(c) {
            continue;
        }
        if combining(c) {
            marks += 1;
            if marks > 3 {
                continue;
            }
        } else {
            marks = 0;
        }
        out.push(c);
    }
    let out = out.trim();
    if out.chars().count() <= max {
        return out.to_string();
    }
    format!("{}…", out.chars().take(max.saturating_sub(1)).collect::<String>().trim_end())
}
