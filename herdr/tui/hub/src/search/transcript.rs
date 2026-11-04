//! A past session as the session index reads it, from any agent's transcript (`cockpit_core::transcripts`): its agent,
//! folder, title, times, and the words typed and answered, without tool calls, their output or the agent's reasoning.

use std::path::Path;

use cockpit_core::transcripts::{self, Transcripts};

/// The most text kept per session: enough to find it by what was said, little enough to keep the index small.
pub const TEXT_CAP: usize = 200 * 1024;

#[derive(Debug, Default, PartialEq)]
pub struct Session {
    /// The agent that wrote it (`claude`, `codex`), to resume it with.
    pub agent: &'static str,
    pub id: String,
    pub cwd: String,
    pub title: String,
    pub first: String,
    pub last: String,
    pub text: String,
}

/// The session in this transcript; None when it holds nothing the user typed, or cannot be read.
pub fn read(kind: Transcripts, path: &Path) -> Option<Session> {
    let t = transcripts::read(kind, path)?;
    Some(Session { agent: kind.agent(), text: t.text(TEXT_CAP), id: t.id, cwd: t.cwd, title: t.title, first: t.first, last: t.last })
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
        let login = read(Transcripts::Claude, &projects().join("-box-repos-api/s-login.jsonl")).unwrap();
        assert_eq!(login.id, "s-login");
        assert_eq!(login.title, "Google sign-in for ENG-401");
        assert_eq!(login.cwd, "@BOX@/repos/api");
        assert_eq!((login.first.as_str(), login.last.as_str()), ("2026-10-27T15:58:02Z", "2026-10-27T16:05:00Z"));
    }

    #[test]
    fn prompts_and_replies_are_kept_tool_output_is_not() {
        let login = read(Transcripts::Claude, &projects().join("-box-repos-api/s-login.jsonl")).unwrap();
        assert!(login.text.contains("the callback should check the OAuth state"), "{}", login.text);
        assert!(login.text.contains("The callback now compares the state"), "{}", login.text);
        assert!(!login.text.contains("zebrafish"), "tool output was indexed: {}", login.text);
        assert!(!login.text.contains("thinking it over"), "thinking was indexed: {}", login.text);
        assert!(!login.text.contains("command-name"), "a command wrapper was indexed: {}", login.text);
    }

    #[test]
    fn without_a_summary_the_first_prompt_is_the_title() {
        let dark = read(Transcripts::Claude, &projects().join("-box-repos-web/s-dark.jsonl")).unwrap();
        assert_eq!(dark.title, "Add dark mode tokens to the sign-in page");
        assert_eq!(dark.cwd, "@BOX@/repos/web");
    }

    #[test]
    fn a_codex_session_reads_like_a_claude_one() {
        let sessions = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/box/home/.codex/sessions");
        let quotas = read(Transcripts::Codex, &sessions.join("2026/10/26/rollout-2026-10-26T14-20-00-0c0dec5e-0412-7a11-9e00-ac3e00000412.jsonl")).unwrap();
        assert_eq!((quotas.agent, quotas.id.as_str(), quotas.cwd.as_str()), ("codex", "0c0dec5e-0412-7a11-9e00-ac3e00000412", "@BOX@/repos/api"));
        assert_eq!(quotas.title, "Store the daily quotas for ENG-412 and reject requests over the limit");
        assert!(quotas.text.contains("answers 429 past the limit"), "{}", quotas.text);
        assert!(!quotas.text.contains("narwhal") && !quotas.text.contains("environment_context"), "{}", quotas.text);
    }

    #[test]
    fn a_session_with_nothing_typed_is_left_out() {
        assert_eq!(read(Transcripts::Claude, &projects().join("-box-repos-api/s-empty.jsonl")), None);
        assert_eq!(read(Transcripts::Claude, &projects().join("-box-repos-api/missing.jsonl")), None);
    }

    #[test]
    fn the_text_stops_at_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s-long.jsonl");
        let line = serde_json::json!({"type": "user", "message": {"role": "user", "content": "é".repeat(60_000)}}).to_string();
        std::fs::write(&path, [line.as_str(); 20].join("\n")).unwrap();
        let long = read(Transcripts::Claude, &path).unwrap();
        assert!(long.text.len() <= TEXT_CAP && long.text.len() > TEXT_CAP - 8, "{}", long.text.len());
        assert_eq!(long.title.chars().count(), 100);
    }
}
