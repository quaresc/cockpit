//! A Claude Code transcript as the session index reads it: its folder, title, first and last times, and the words of
//! its prompts and replies; tool calls, their output and Claude Code's own messages are left out.

use std::io::{BufRead, BufReader};
use std::path::Path;

use serde::Deserialize;
use serde::de::{Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};

/// The most text kept per session: enough to find it by what was said, little enough to keep the index small.
pub const TEXT_CAP: usize = 200 * 1024;
const TITLE_CAP: usize = 100;
/// Longer lines hold tool output or pictures: skipped without being read into memory.
const LINE_CAP: usize = 4 << 20;

#[derive(Debug, Default, PartialEq)]
pub struct Session {
    pub id: String,
    pub cwd: String,
    pub title: String,
    pub first: String,
    pub last: String,
    pub text: String,
}

/// One line of the transcript, its other fields skipped unread.
#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct Line {
    #[serde(rename = "type")]
    kind: String,
    timestamp: Option<String>,
    cwd: Option<String>,
    summary: Option<String>,
    custom_title: Option<String>,
    ai_title: Option<String>,
    is_meta: Option<bool>,
    is_sidechain: Option<bool>,
    is_compact_summary: Option<bool>,
    message: Option<Message>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Message {
    #[serde(deserialize_with = "texts")]
    content: Vec<String>,
}

/// A message's text: the content when it is a string, else its `text` blocks (tool calls, results and thinking dropped).
fn texts<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    struct Texts;
    struct Block(Option<String>);
    impl<'de> Deserialize<'de> for Block {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            struct Fields;
            impl<'de> Visitor<'de> for Fields {
                type Value = Block;
                fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                    f.write_str("a content block")
                }
                fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Block, A::Error> {
                    let (mut kind, mut text) = (None::<String>, None::<String>);
                    while let Some(key) = map.next_key::<String>()? {
                        match key.as_str() {
                            "type" => kind = Some(map.next_value()?),
                            "text" => text = map.next_value()?,
                            _ => {
                                map.next_value::<IgnoredAny>()?;
                            }
                        }
                    }
                    Ok(Block(text.filter(|_| kind.as_deref() == Some("text"))))
                }
            }
            d.deserialize_any(Fields)
        }
    }
    impl<'de> Visitor<'de> for Texts {
        type Value = Vec<String>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a string or a list of content blocks")
        }
        fn visit_str<E>(self, s: &str) -> Result<Vec<String>, E> {
            Ok(vec![s.to_string()])
        }
        fn visit_unit<E>(self) -> Result<Vec<String>, E> {
            Ok(Vec::new())
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<String>, A::Error> {
            let mut out = Vec::new();
            while let Some(Block(text)) = seq.next_element::<Block>()? {
                out.extend(text);
            }
            Ok(out)
        }
    }
    d.deserialize_any(Texts)
}

/// Claude Code's own messages in the user's turn: command wrappers, hook output, reminders, interruptions.
fn is_typed(text: &str) -> bool {
    let text = text.trim_start();
    let tag = text.strip_prefix('<').map(|rest| rest.split(['>', ' ']).next().unwrap_or_default());
    !text.is_empty() && !tag.is_some_and(|t| t.contains('-')) && !text.starts_with("[Request interrupted")
}

/// The session in this transcript; None when it holds nothing the user typed, or cannot be read.
pub fn read(path: &Path) -> Option<Session> {
    let id = path.file_stem()?.to_str()?.to_string();
    let file = std::fs::File::open(path).ok()?;
    let mut session = Session { id, ..Session::default() };
    let (mut prompt, mut summary, mut named, mut ai_title) = (None::<String>, None, None, None);
    let mut lines = Lines { reader: BufReader::new(file), line: Vec::new() };
    while let Some(line) = lines.next() {
        let Ok(line) = serde_json::from_slice::<Line>(line) else { continue };
        if let Some(at) = line.timestamp.filter(|t| !t.is_empty()) {
            if session.first.is_empty() {
                session.first = at.clone();
            }
            session.last = at;
        }
        if session.cwd.is_empty()
            && let Some(cwd) = line.cwd
        {
            session.cwd = cwd;
        }
        match line.kind.as_str() {
            "summary" => summary = line.summary.or(summary),
            "custom-title" => named = line.custom_title.or(named),
            "ai-title" => ai_title = line.ai_title.or(ai_title),
            "user" | "assistant" if ![line.is_sidechain, line.is_compact_summary, line.is_meta].contains(&Some(true)) => {
                let is_user = line.kind == "user";
                for text in line.message.map(|m| m.content).unwrap_or_default() {
                    if is_user && !is_typed(&text) {
                        continue;
                    }
                    if is_user && prompt.is_none() {
                        prompt = Some(text.clone());
                    }
                    keep(&mut session.text, &text);
                }
            }
            _ => {}
        }
    }
    let prompt = prompt?;
    let title = named.or(summary).or(ai_title).unwrap_or(prompt);
    session.title = title.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(TITLE_CAP).collect();
    Some(session)
}

/// A transcript's lines through one buffer, those over LINE_CAP skipped.
struct Lines<R> {
    reader: R,
    line: Vec<u8>,
}

impl<R: BufRead> Lines<R> {
    fn next(&mut self) -> Option<&[u8]> {
        self.line.clear();
        let mut too_long = false;
        loop {
            let buf = match self.reader.fill_buf() {
                Ok(buf) if !buf.is_empty() => buf,
                _ => return (!too_long && !self.line.is_empty()).then_some(self.line.as_slice()),
            };
            let end = buf.iter().position(|&b| b == b'\n');
            let chunk = &buf[..end.unwrap_or(buf.len())];
            too_long |= self.line.len() + chunk.len() > LINE_CAP;
            if !too_long {
                self.line.extend_from_slice(chunk);
            }
            let used = chunk.len() + usize::from(end.is_some());
            self.reader.consume(used);
            if end.is_some() {
                if too_long {
                    self.line.clear();
                    too_long = false;
                    continue;
                }
                return Some(self.line.as_slice());
            }
        }
    }
}

/// Adds the text, up to the cap, cut on a character boundary.
fn keep(out: &mut String, text: &str) {
    let room = TEXT_CAP.saturating_sub(out.len() + 1);
    if room == 0 {
        return;
    }
    let mut end = text.len().min(room);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(&text[..end]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn projects() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/box/home/.claude/projects")
    }

    #[test]
    fn the_summary_is_the_title() {
        let login = read(&projects().join("-box-repos-api/s-login.jsonl")).unwrap();
        assert_eq!(login.id, "s-login");
        assert_eq!(login.title, "Google sign-in for ENG-401");
        assert_eq!(login.cwd, "@BOX@/repos/api");
        assert_eq!((login.first.as_str(), login.last.as_str()), ("2026-10-27T15:58:02Z", "2026-10-27T16:05:00Z"));
    }

    #[test]
    fn prompts_and_replies_are_kept_tool_output_is_not() {
        let login = read(&projects().join("-box-repos-api/s-login.jsonl")).unwrap();
        assert!(login.text.contains("the callback should check the OAuth state"), "{}", login.text);
        assert!(login.text.contains("The callback now compares the state"), "{}", login.text);
        assert!(!login.text.contains("zebrafish"), "tool output was indexed: {}", login.text);
        assert!(!login.text.contains("thinking it over"), "thinking was indexed: {}", login.text);
        assert!(!login.text.contains("command-name"), "a command wrapper was indexed: {}", login.text);
    }

    #[test]
    fn without_a_summary_the_first_prompt_is_the_title() {
        let dark = read(&projects().join("-box-repos-web/s-dark.jsonl")).unwrap();
        assert_eq!(dark.title, "Add dark mode tokens to the sign-in page");
        assert_eq!(dark.cwd, "@BOX@/repos/web");
    }

    #[test]
    fn a_session_with_nothing_typed_is_left_out() {
        assert_eq!(read(&projects().join("-box-repos-api/s-empty.jsonl")), None);
        assert_eq!(read(&projects().join("-box-repos-api/missing.jsonl")), None);
    }

    #[test]
    fn a_line_too_long_is_skipped_and_the_next_one_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s-big.jsonl");
        let big = serde_json::json!({"type": "user", "message": {"content": "x".repeat(LINE_CAP)}}).to_string();
        let small = serde_json::json!({"type": "user", "message": {"content": "after the picture"}}).to_string();
        std::fs::write(&path, format!("{big}\n{small}")).unwrap();
        assert_eq!(read(&path).unwrap().text, "after the picture");
    }

    #[test]
    fn the_text_stops_at_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s-long.jsonl");
        let line = serde_json::json!({"type": "user", "message": {"role": "user", "content": "é".repeat(60_000)}}).to_string();
        std::fs::write(&path, [line.as_str(); 4].join("\n")).unwrap();
        let long = read(&path).unwrap();
        assert!(long.text.len() <= TEXT_CAP && long.text.len() > TEXT_CAP - 8, "{}", long.text.len());
        assert_eq!(long.title.chars().count(), TITLE_CAP);
    }
}
