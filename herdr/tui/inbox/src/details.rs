//! A highlighted PR's details on demand, fetched in the background through the hub and kept per PR: its unresolved
//! review threads and, when its CI is red, the failing tests and the last lines of the failing job's log.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use cockpit_core::hub;
use cockpit_core::theme::{DIM, FG, ORANGE, RED, dim, fg};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::{Value, json};

use crate::facts::{list, s, text};

const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
/// Details fetched stay this long before a highlight asks again; a failure, less.
const KEPT: Duration = Duration::from_secs(300);
const RETRY: Duration = Duration::from_secs(60);
const THREADS: usize = 4;
const TESTS: usize = 3;
const LOG: usize = 4;

#[derive(Clone, Debug, Default)]
pub struct Fetched {
    /// "path:line", author, the comment's first line.
    pub threads: Vec<(String, String, String)>,
    pub tests: Vec<String>,
    pub log: Vec<String>,
    pub error: Option<String>,
}

enum Entry {
    Loading(Instant),
    Ready(Instant, Fetched),
}

pub struct Details {
    enabled: bool,
    cache: HashMap<String, Entry>,
    tx: Sender<(String, Fetched)>,
    rx: Receiver<(String, Fetched)>,
}

impl Details {
    /// `enabled` false for tests: nothing is fetched, only what `set` gives shows.
    pub fn new(enabled: bool) -> Self {
        let (tx, rx) = channel();
        Self { enabled, cache: HashMap::new(), tx, rx }
    }

    /// Fetches the PR's details unless they are here, or on their way, and fresh.
    pub fn want(&mut self, url: &str, pr: &Value, run: Option<&Value>, failing: bool) {
        let fresh = match self.cache.get(url) {
            Some(Entry::Loading(_)) => true,
            Some(Entry::Ready(at, fetched)) => at.elapsed() < if fetched.error.is_some() { RETRY } else { KEPT },
            None => false,
        };
        if !self.enabled || url.is_empty() || fresh {
            return;
        }
        self.cache.insert(url.to_string(), Entry::Loading(Instant::now()));
        let args = json!({"url": url, "repo": s(pr, "repo"), "number": pr["number"], "branch": s(pr, "branch"), "run": run.cloned().unwrap_or(Value::Null)});
        let (tx, url) = (self.tx.clone(), url.to_string());
        std::thread::spawn(move || {
            let _ = tx.send((url, fetch(&args, failing)));
        });
    }

    /// Takes what arrived; true when something did.
    pub fn pump(&mut self) -> bool {
        let mut any = false;
        while let Ok((url, fetched)) = self.rx.try_recv() {
            self.cache.insert(url, Entry::Ready(Instant::now(), fetched));
            any = true;
        }
        any
    }

    pub fn set(&mut self, url: &str, fetched: Fetched) {
        self.cache.insert(url.to_string(), Entry::Ready(Instant::now(), fetched));
    }

    /// Drops what is kept for a PR whose state changed, so the next highlight fetches it again.
    pub fn forget(&mut self, url: &str) {
        if matches!(self.cache.get(url), Some(Entry::Ready(..))) {
            self.cache.remove(url);
        }
    }

    pub fn loading(&self, url: &str) -> bool {
        matches!(self.cache.get(url), Some(Entry::Loading(_)))
    }

    /// The panel's lines for the PR: a spinner while loading, then each part found, nothing when there is none.
    pub fn lines(&self, url: &str) -> Vec<Line<'static>> {
        match self.cache.get(url) {
            None => Vec::new(),
            Some(Entry::Loading(since)) => {
                let frame = SPINNER[(since.elapsed().as_millis() / 80) as usize % SPINNER.len()];
                vec![Line::styled(format!("{frame} fetching review threads and CI details…"), fg(ORANGE))]
            }
            Some(Entry::Ready(_, fetched)) => render(fetched),
        }
    }
}

fn render(fetched: &Fetched) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let title = |text: String, color| Line::styled(text, Style::new().fg(color).add_modifier(Modifier::BOLD));
    if !fetched.threads.is_empty() {
        lines.push(title(format!("{} unresolved review thread{}", fetched.threads.len(), if fetched.threads.len() == 1 { "" } else { "s" }), FG));
        for (place, author, body) in fetched.threads.iter().take(THREADS) {
            let mut spans = vec![Span::styled(format!("  {place}  "), dim()), Span::styled(format!("{author}  "), fg(ORANGE))];
            // The comment's code, emphasis and links marked as GitHub shows them.
            spans.extend(cockpit_show::markdown_inline(body).into_iter().map(|span| Span::styled(span.content, fg(FG).patch(span.style))));
            lines.push(Line::from(spans));
        }
    }
    if !fetched.tests.is_empty() {
        lines.push(title(format!("{} failing test{}", fetched.tests.len(), if fetched.tests.len() == 1 { "" } else { "s" }), RED));
        lines.extend(fetched.tests.iter().take(TESTS).map(|t| Line::styled(format!("  {t}"), fg(RED))));
    }
    if !fetched.log.is_empty() {
        lines.push(title("failing job's log".into(), DIM));
        lines.extend(fetched.log.iter().map(|l| Line::styled(format!("  {l}"), dim())));
    }
    if let Some(error) = &fetched.error {
        lines.push(Line::styled(format!("details unavailable: {error}"), dim()));
    }
    lines
}

/// Everything the panel shows of a PR, asked of the hub in turn; the first failure to reach it stops the rest.
fn fetch(args: &Value, failing: bool) -> Fetched {
    let mut fetched = Fetched::default();
    let kinds: &[&str] = if failing { &["review_threads", "failed_tests", "ci_log"] } else { &["review_threads"] };
    for kind in kinds {
        match hub::fetch(kind, args.clone()) {
            Ok(answer) => fetched.absorb(kind, &answer),
            Err(error) => {
                fetched.error = Some(error);
                break;
            }
        }
    }
    fetched
}

/// An array answered as is, or under one of these keys.
fn items<'a>(answer: &'a Value, keys: &[&str]) -> &'a [Value] {
    if let Some(all) = answer.as_array() {
        return all;
    }
    keys.iter().map(|k| list(answer, k)).find(|l| !l.is_empty()).unwrap_or(&[])
}

fn first_line(text: &str) -> String {
    text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").to_string()
}

impl Fetched {
    /// One hub answer read into its part, leniently: the hub's fields may come as strings, numbers or objects.
    pub fn absorb(&mut self, kind: &str, answer: &Value) {
        match kind {
            "review_threads" => {
                for thread in items(answer, &["threads", "items"]) {
                    if thread["isResolved"].as_bool() == Some(true) || thread["resolved"].as_bool() == Some(true) {
                        continue;
                    }
                    let first = list(thread, "comments").first().cloned().unwrap_or(Value::Null);
                    let author = [s(thread, "author"), s(&thread["author"], "login"), s(&first["author"], "login"), s(&first, "author")].into_iter().find(|a| !a.is_empty()).unwrap_or("");
                    let body = if s(thread, "body").is_empty() { s(&first, "body") } else { s(thread, "body") };
                    let line = text(thread, "line");
                    let place = if line.is_empty() { s(thread, "path").to_string() } else { format!("{}:{line}", s(thread, "path")) };
                    self.threads.push((place, author.to_string(), first_line(body)));
                }
            }
            "failed_tests" => {
                for test in items(answer, &["tests", "items"]) {
                    let name = test.as_str().map(str::to_string).unwrap_or_else(|| [s(test, "name"), s(test, "title"), s(test, "file")].into_iter().find(|n| !n.is_empty()).unwrap_or("").to_string());
                    if !name.is_empty() {
                        self.tests.push(name);
                    }
                }
            }
            "ci_log" => {
                let lines: Vec<String> = match answer["log"].as_str() {
                    Some(log) => log.lines().map(str::to_string).collect(),
                    None => items(answer, &["lines"]).iter().filter_map(|l| l.as_str().map(str::to_string)).collect(),
                };
                let kept: Vec<String> = lines.into_iter().map(|l| l.trim_end().to_string()).filter(|l| !l.trim().is_empty()).collect();
                self.log = kept[kept.len().saturating_sub(LOG)..].to_vec();
                // Another CI's logs need a session: the hub says so instead.
                if self.log.is_empty() && !s(answer, "message").is_empty() {
                    self.log.push(s(answer, "message").to_string());
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_hubs_answers_leniently() {
        let mut fetched = Fetched::default();
        let threads = json!({"threads": [
            {"path": "src/a.rs", "line": 12, "author": "ana", "body": "\nCould this be a constant?\nAnd more"},
            {"path": "src/b.rs", "isResolved": true, "author": "bo", "body": "done"},
            {"path": "README.md", "comments": [{"author": {"login": "cy"}, "body": "typo"}]}
        ]});
        fetched.absorb("review_threads", &threads);
        fetched.absorb("failed_tests", &json!(["test_login", {"name": "test_logout"}]));
        fetched.absorb("ci_log", &json!({"log": "one\ntwo\n\nthree\nfour\nfive\n"}));
        assert_eq!(fetched.threads, vec![("src/a.rs:12".into(), "ana".into(), "Could this be a constant?".into()), ("README.md".into(), "cy".into(), "typo".into())]);
        assert_eq!(fetched.tests, vec!["test_login", "test_logout"]);
        assert_eq!(fetched.log, vec!["two", "three", "four", "five"]);
    }

    #[test]
    fn a_comments_code_is_marked() {
        let fetched = Fetched { threads: vec![("src/a.rs:3".into(), "ana".into(), "Use `quota()` here".into())], ..Fetched::default() };
        let line = &render(&fetched)[1];
        let code = line.spans.iter().find(|s| s.content == "quota()").map(|s| s.style.fg);
        assert_eq!(code, Some(Some(ORANGE)));
        assert_eq!(line.spans.last().map(|s| (s.content.as_ref(), s.style.fg)), Some((" here", Some(FG))));
    }
}
