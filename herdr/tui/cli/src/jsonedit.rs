//! One member of a JSON (or JSONC) object set or removed in place, in the text itself: every other byte stays as the
//! person wrote it, comments, order and spacing included.

use serde_json::Value;

/// A member of an object: its key, where its key starts and where its value sits.
struct Member {
    key: String,
    start: usize,
    value: (usize, usize),
}

/// An object's members, and where its braces are.
struct Object {
    open: usize,
    close: usize,
    members: Vec<Member>,
}

struct Scan<'a> {
    text: &'a [u8],
    at: usize,
}

impl<'a> Scan<'a> {
    fn new(text: &'a str, at: usize) -> Self {
        Scan { text: text.as_bytes(), at }
    }

    /// Skips whitespace and comments.
    fn blank(&mut self) {
        loop {
            while self.at < self.text.len() && self.text[self.at].is_ascii_whitespace() {
                self.at += 1;
            }
            let rest = &self.text[self.at.min(self.text.len())..];
            if rest.starts_with(b"//") {
                while self.at < self.text.len() && self.text[self.at] != b'\n' {
                    self.at += 1;
                }
            } else if rest.starts_with(b"/*") {
                self.at = rest[2..].windows(2).position(|w| w == b"*/").map_or(self.text.len(), |p| self.at + 2 + p + 2);
            } else {
                return;
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.text.get(self.at).copied()
    }

    fn string(&mut self) -> Result<String, String> {
        let start = self.at;
        self.at += 1;
        while self.at < self.text.len() {
            match self.text[self.at] {
                b'\\' => self.at += 2,
                b'"' => {
                    self.at += 1;
                    let raw = std::str::from_utf8(&self.text[start..self.at]).map_err(|e| e.to_string())?;
                    return serde_json::from_str(raw).map_err(|e| e.to_string());
                }
                _ => self.at += 1,
            }
        }
        Err("a string never ends".into())
    }

    /// Steps over one value of any kind.
    fn value(&mut self) -> Result<(), String> {
        self.blank();
        match self.peek() {
            Some(b'"') => self.string().map(|_| ()),
            Some(open @ (b'{' | b'[')) => {
                let close = if open == b'{' { b'}' } else { b']' };
                self.at += 1;
                loop {
                    self.blank();
                    match self.peek() {
                        Some(c) if c == close => {
                            self.at += 1;
                            return Ok(());
                        }
                        Some(b',' | b':') => self.at += 1,
                        Some(_) => self.value()?,
                        None => return Err("an object or a list never ends".into()),
                    }
                }
            }
            Some(_) => {
                let start = self.at;
                while self.at < self.text.len() && !b",}]/ \t\r\n".contains(&self.text[self.at]) {
                    self.at += 1;
                }
                if self.at == start { Err(format!("unexpected text at byte {start}")) } else { Ok(()) }
            }
            None => Err("a value is missing".into()),
        }
    }

    fn object(&mut self) -> Result<Object, String> {
        self.blank();
        if self.peek() != Some(b'{') {
            return Err("not a JSON object".into());
        }
        let open = self.at;
        self.at += 1;
        let mut members = Vec::new();
        loop {
            self.blank();
            match self.peek() {
                Some(b'}') => return Ok(Object { open, close: self.at, members }),
                Some(b',') => self.at += 1,
                Some(b'"') => {
                    let start = self.at;
                    let key = self.string()?;
                    self.blank();
                    if self.peek() != Some(b':') {
                        return Err(format!("no colon after {key:?}"));
                    }
                    self.at += 1;
                    self.blank();
                    let from = self.at;
                    self.value()?;
                    members.push(Member { key, start, value: (from, self.at) });
                }
                _ => return Err(format!("unexpected text at byte {}", self.at)),
            }
        }
    }
}

/// The object at `path` from the root, or the depth reached and the object there when a key is missing.
fn walk(text: &str, path: &[&str]) -> Result<(Object, usize), String> {
    let mut object = Scan::new(text, 0).object()?;
    for (n, key) in path.iter().enumerate() {
        let Some(m) = object.members.iter().find(|m| m.key == *key) else { return Ok((object, n)) };
        object = Scan::new(text, m.value.0).object().map_err(|e| format!("{}: {e}", path[..=n].join(".")))?;
    }
    Ok((object, path.len()))
}

/// The leading whitespace of the line holding byte `at`.
fn line_indent(text: &str, at: usize) -> &str {
    let line = text[..at].rfind('\n').map_or(0, |n| n + 1);
    let lead = &text[line..at];
    &lead[..lead.len() - lead.trim_start_matches([' ', '\t']).len()]
}

/// One level of the file's indentation: its first indented line's, else two spaces.
fn unit(text: &str) -> String {
    text.lines().map(|l| &l[..l.len() - l.trim_start_matches([' ', '\t']).len()]).find(|w| !w.is_empty()).unwrap_or("  ").to_string()
}

/// A value as pretty JSON in the file's indentation, its later lines under `indent`.
fn pretty(value: &Value, indent: &str, unit: &str) -> String {
    let text = serde_json::to_string_pretty(value).unwrap_or_default();
    let lines: Vec<String> = text
        .lines()
        .enumerate()
        .map(|(i, l)| {
            let depth = (l.len() - l.trim_start().len()) / 2;
            format!("{}{}{}", if i == 0 { "" } else { indent }, unit.repeat(depth), l.trim_start())
        })
        .collect();
    lines.join("\n")
}

/// The text with the object at `path` (made when missing) holding `name: value`, replacing a member of that name.
pub fn set(text: &str, path: &[&str], name: &str, value: &Value) -> Result<String, String> {
    let text = if text.trim().is_empty() { "{}\n" } else { text };
    let (object, reached) = walk(text, path)?;
    // What is missing of the path is made as one nested value.
    let (key, value) = if reached < path.len() {
        let mut nested = serde_json::json!({ name: value });
        for k in path[reached + 1..].iter().rev() {
            nested = serde_json::json!({ *k: nested });
        }
        (path[reached], nested)
    } else {
        (name, value.clone())
    };
    let unit = unit(text);
    if let Some(m) = object.members.iter().find(|m| m.key == key) {
        let indent = line_indent(text, m.start);
        return Ok(format!("{}{}{}", &text[..m.value.0], pretty(&value, indent, &unit), &text[m.value.1..]));
    }
    let outer = line_indent(text, object.open);
    let multiline = text[object.open..object.close].contains('\n') || object.members.is_empty();
    let indent = object.members.first().map(|m| line_indent(text, m.start).to_string()).filter(|_| multiline).unwrap_or_else(|| format!("{outer}{unit}"));
    let member = format!("{}: {}", Value::from(key), pretty(&value, &indent, &unit));
    let Some(last) = object.members.last() else {
        return Ok(format!("{}\n{indent}{member}\n{outer}{}", &text[..=object.open], &text[object.close..]));
    };
    let mut after = Scan::new(text, last.value.1);
    after.blank();
    if after.peek() == Some(b',') {
        // A trailing comma (JSONC): the new member goes after it and keeps one of its own.
        let at = after.at + 1;
        let gap = if multiline { format!("\n{indent}") } else { " ".into() };
        return Ok(format!("{}{gap}{member},{}", &text[..at], &text[at..]));
    }
    let gap = if multiline { format!("\n{indent}") } else { " ".into() };
    Ok(format!("{},{gap}{member}{}", &text[..last.value.1], &text[last.value.1..]))
}

/// The text without `name` in the object at `path`; None when it holds no such member.
pub fn remove(text: &str, path: &[&str], name: &str) -> Result<Option<String>, String> {
    if text.trim().is_empty() {
        return Ok(None);
    }
    let (object, reached) = walk(text, path)?;
    if reached < path.len() {
        return Ok(None);
    }
    let Some(i) = object.members.iter().position(|m| m.key == name) else { return Ok(None) };
    let m = &object.members[i];
    let mut after = Scan::new(text, m.value.1);
    after.blank();
    let comma_after = after.peek() == Some(b',');
    let out = if i > 0 {
        // From the end of the previous value: this member's comma before it goes, and its own after it when it was last.
        let from = object.members[i - 1].value.1;
        let to = if comma_after && i == object.members.len() - 1 { after.at + 1 } else { m.value.1 };
        let kept_comma = comma_after && i == object.members.len() - 1;
        format!("{}{}{}", &text[..from], if kept_comma { "," } else { "" }, &text[to..])
    } else {
        // The first member goes with its comma and the blank up to the next member.
        let to = if comma_after {
            let mut next = Scan::new(text, after.at + 1);
            next.blank();
            if object.members.len() > 1 { next.at } else { after.at + 1 }
        } else {
            m.value.1
        };
        let from = if object.members.len() > 1 { m.start } else { object.open + 1 };
        let to = if object.members.len() > 1 { to } else { object.close };
        format!("{}{}", &text[..from], &text[to..])
    };
    Ok(Some(out))
}

/// The value of `name` in the object at `path`, parsed; None when missing.
pub fn get(text: &str, path: &[&str], name: &str) -> Result<Option<Value>, String> {
    if text.trim().is_empty() {
        return Ok(None);
    }
    let (object, reached) = walk(text, path)?;
    if reached < path.len() {
        return Ok(None);
    }
    object.members.iter().find(|m| m.key == name).map(|m| plain(&text[m.value.0..m.value.1])).transpose()
}

/// JSONC as JSON: comments and trailing commas dropped.
pub fn plain(raw: &str) -> Result<Value, String> {
    let bytes = raw.as_bytes();
    let mut out = String::with_capacity(raw.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                let mut scan = Scan::new(raw, i);
                scan.string()?;
                out.push_str(&raw[i..scan.at]);
                i = scan.at;
            }
            b'/' if matches!(bytes.get(i + 1), Some(b'/' | b'*')) => {
                let mut scan = Scan::new(raw, i);
                scan.blank();
                i = scan.at;
            }
            b',' => {
                let mut scan = Scan::new(raw, i + 1);
                scan.blank();
                if !matches!(scan.peek(), Some(b'}' | b']')) {
                    out.push(',');
                }
                i += 1;
            }
            _ => {
                let next = raw[i..].chars().next().map_or(1, char::len_utf8);
                out.push_str(&raw[i..i + next]);
                i += next;
            }
        }
    }
    serde_json::from_str(&out).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SERVER: &str = "cockpit";

    #[test]
    fn a_member_joins_an_object_as_its_last_line() {
        let text = "{\n    \"theme\": \"dark\", // mine\n    \"mcpServers\": {\n        \"other\": {\"command\": \"x\"}\n    }\n}\n";
        let done = set(text, &["mcpServers"], SERVER, &json!({"command": "/c", "args": ["mcp"]})).unwrap();
        assert_eq!(
            done,
            "{\n    \"theme\": \"dark\", // mine\n    \"mcpServers\": {\n        \"other\": {\"command\": \"x\"},\n        \"cockpit\": {\n            \"args\": [\n                \"mcp\"\n            ],\n            \"command\": \"/c\"\n        }\n    }\n}\n"
        );
        assert_eq!(plain(&done).unwrap()["mcpServers"]["other"], json!({"command": "x"}));
        // Again: the same text, the member replaced in place.
        assert_eq!(set(&done, &["mcpServers"], SERVER, &json!({"command": "/c", "args": ["mcp"]})).unwrap(), done);
        assert_eq!(remove(&done, &["mcpServers"], SERVER).unwrap().unwrap(), text);
    }

    #[test]
    fn a_missing_object_is_made() {
        let text = "{\n  \"model\": \"o3\"\n}\n";
        let done = set(text, &["mcp"], SERVER, &json!({"type": "local"})).unwrap();
        assert_eq!(done, "{\n  \"model\": \"o3\",\n  \"mcp\": {\n    \"cockpit\": {\n      \"type\": \"local\"\n    }\n  }\n}\n");
        assert_eq!(set("", &["mcpServers"], SERVER, &json!(1)).unwrap(), "{\n  \"mcpServers\": {\n    \"cockpit\": 1\n  }\n}\n");
        assert_eq!(set("{}", &["mcpServers"], SERVER, &json!(1)).unwrap(), "{\n  \"mcpServers\": {\n    \"cockpit\": 1\n  }\n}");
        assert_eq!(get(&done, &["mcp"], SERVER).unwrap(), Some(json!({"type": "local"})));
        assert_eq!(get(text, &["mcp"], SERVER).unwrap(), None);
    }

    #[test]
    fn jsonc_keeps_its_comments_and_trailing_commas() {
        let text = "// global\n{\n  \"mcp\": {\n    /* a */ \"a\": {\"enabled\": true},\n  },\n}\n";
        let done = set(text, &["mcp"], SERVER, &json!(true)).unwrap();
        assert_eq!(done, "// global\n{\n  \"mcp\": {\n    /* a */ \"a\": {\"enabled\": true},\n    \"cockpit\": true,\n  },\n}\n");
        assert_eq!(plain(&done).unwrap(), json!({"mcp": {"a": {"enabled": true}, "cockpit": true}}));
        assert_eq!(remove(&done, &["mcp"], SERVER).unwrap().unwrap(), text);
    }

    #[test]
    fn removing_the_first_or_only_member_leaves_valid_json() {
        let text = "{\"mcpServers\": {\"cockpit\": {}, \"b\": 2}}";
        assert_eq!(remove(text, &["mcpServers"], SERVER).unwrap().unwrap(), "{\"mcpServers\": {\"b\": 2}}");
        let only = "{\"mcpServers\": {\n  \"cockpit\": {}\n}}";
        assert_eq!(remove(only, &["mcpServers"], SERVER).unwrap().unwrap(), "{\"mcpServers\": {}}");
        assert_eq!(remove("{\"a\": 1}", &["mcpServers"], SERVER).unwrap(), None);
        assert!(set("[1]", &["mcpServers"], SERVER, &json!(1)).is_err());
        assert!(set("{\"a\": }", &["mcpServers"], SERVER, &json!(1)).is_err());
    }
}
