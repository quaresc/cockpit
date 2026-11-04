//! The app's state and what changes it: the mail the hub pushes, the selected message's thread read again when its file
//! changes, focus requests (a notification's click), and the actions sent through the bridge.

use std::path::PathBuf;
use std::time::{Instant, SystemTime};

use chrono::Utc;
use cockpit_core::acts::Act;
use cockpit_core::bridge::Bridge;
use cockpit_core::files;
use cockpit_core::hub::{self, Docs};
use cockpit_core::toast::{Kind as Toast, Toasts};
use ratatui::layout::Rect;
use ratatui::text::Line;
use serde_json::Value;

use crate::model::{Mail, Message};
use crate::pinned;
use crate::thread::{self, Thread};

/// What a key or a button runs, on a message's id.
#[derive(Clone, Debug, PartialEq)]
pub enum Cmd {
    See(String),
    Open(String),
    Reply(String),
    Archive(String),
    Read(String),
    Html(String),
    Refresh,
}

/// Where the last frame drew what a click can land on.
#[derive(Default)]
pub struct Hits {
    pub list: Rect,
    pub body: Rect,
    pub rows: Vec<(Rect, String)>,
    pub buttons: Vec<(Rect, Cmd)>,
}

pub struct App {
    pub mail: Mail,
    pub config: Value,
    pub selected: Option<String>,
    /// The selected message's thread, and the file times it was read at.
    pub thread: Thread,
    thread_of: Option<(String, Option<SystemTime>, Option<SystemTime>)>,
    /// The thread drawn as lines for a width, kept until either changes.
    pub body: Option<(String, usize, Vec<Line<'static>>)>,
    pub body_scroll: usize,
    pub scroll: usize,
    pub follow: bool,
    /// A message whose page opens in the side browser once its thread is there.
    pub html_wanted: Option<String>,
    pub toasts: Toasts,
    toasting: bool,
    pub bridge: Bridge,
    calls: usize,
    focus_seen: SystemTime,
    pinned: bool,
    reported: Option<String>,
    pub hits: Hits,
    pub last_click: Option<(Instant, String)>,
    docs: Option<Docs>,
    revision: u64,
}

impl App {
    pub fn start() -> Self {
        let mut app = Self::with(files::config(), pinned::is_pinned());
        app.docs = Some(hub::subscribe(&["mail"]));
        app.reload();
        app
    }

    /// The app on this config, with no mail yet: what tests start from.
    pub fn with(config: Value, pinned: bool) -> Self {
        Self {
            mail: Mail::default(),
            config,
            selected: None,
            thread: Thread::Unread,
            thread_of: None,
            body: None,
            body_scroll: 0,
            scroll: 0,
            follow: true,
            html_wanted: None,
            toasts: Toasts::default(),
            toasting: false,
            bridge: Bridge::default(),
            calls: 0,
            focus_seen: SystemTime::now(),
            pinned,
            reported: None,
            hits: Hits::default(),
            last_click: None,
            docs: None,
            revision: 0,
        }
    }

    /// A mail document given whole: what tests draw.
    pub fn set_mail(&mut self, doc: &Value) {
        self.mail = Mail::parse(Some(doc));
        self.select_default();
    }

    pub fn selected_message(&self) -> Option<&Message> {
        self.mail.message(self.selected.as_deref()?)
    }

    fn select_default(&mut self) {
        if self.selected.as_ref().is_some_and(|id| self.mail.message(id).is_some()) {
            return;
        }
        // A message that left (archived) gives its place to the first.
        self.selected = self.mail.messages.first().map(|m| m.id.clone());
        self.body_scroll = 0;
    }

    pub fn select(&mut self, id: &str) {
        if self.selected.as_deref() != Some(id) {
            self.selected = Some(id.to_string());
            self.body_scroll = 0;
            self.follow = true;
        }
    }

    pub fn move_sel(&mut self, by: isize) {
        let ids: Vec<String> = self.mail.messages.iter().map(|m| m.id.clone()).collect();
        if ids.is_empty() {
            return;
        }
        let at = self.selected.as_ref().and_then(|id| ids.iter().position(|i| i == id)).unwrap_or(0) as isize;
        let next = ids[(at + by).clamp(0, ids.len() as isize - 1) as usize].clone();
        self.select(&next);
    }

    // -----------------------------------------------------------------------------------------------------------------
    // Reading again

    /// The mail as pushed, read again when it moved; true when it did.
    fn reload(&mut self) -> bool {
        let Some(docs) = &mut self.docs else { return false };
        docs.pump();
        if docs.revision == self.revision && self.revision > 0 {
            return false;
        }
        self.revision = docs.revision.max(1);
        let doc = docs.get("mail").cloned();
        self.mail = Mail::parse(doc.as_ref());
        self.select_default();
        true
    }

    /// The selected message's thread, read again when it is another message or its files changed; true when it was.
    pub fn reload_thread(&mut self) -> bool {
        let Some(id) = self.selected.clone() else {
            let changed = self.thread != Thread::Unread;
            self.thread = Thread::Unread;
            self.thread_of = None;
            return changed;
        };
        let path = thread::path(&id);
        let at = |p: PathBuf| std::fs::metadata(p).and_then(|m| m.modified()).ok();
        let stamp = (id.clone(), at(path.clone()), at(path.with_extension("asked")));
        if self.thread_of.as_ref() == Some(&stamp) {
            return false;
        }
        self.thread_of = Some(stamp);
        self.thread = thread::read(&id);
        self.body = None;
        if let (Thread::Ready(_), Some(wanted)) = (&self.thread, self.html_wanted.clone())
            && wanted == id
        {
            self.html_wanted = None;
            self.open_page(&id);
        }
        true
    }

    pub fn report(&mut self) {
        if self.docs.as_ref().is_some_and(|d| d.get("mail").is_none()) {
            return;
        }
        let text = cockpit_connectors::mail::row_text(&serde_json::json!({"messages": self.mail.messages.iter().map(|m| m.raw.clone()).collect::<Vec<_>>()}));
        let Some(text) = text else { return };
        if self.pinned && self.reported.as_ref() != Some(&text) {
            pinned::report(text.clone());
        }
        self.reported = Some(text);
    }

    /// A notification click or a palette jump: that message selected and read whole.
    fn take_focus(&mut self) -> bool {
        let Some(id) = cockpit_core::focus::take("mail", &mut self.focus_seen).and_then(|t| t["key"].as_str().map(str::to_string)) else { return false };
        self.focus(&id);
        true
    }

    pub fn focus(&mut self, id: &str) {
        if self.mail.message(id).is_none() {
            self.toasts.push("that message is not in the inbox any more", Toast::Note);
            return;
        }
        self.select(id);
        self.run(Cmd::See(id.to_string()));
    }

    pub fn is_busy(&self) -> bool {
        self.calls > 0 || matches!(self.thread, Thread::Loading(_)) || self.html_wanted.is_some()
    }

    pub fn tick(&mut self) -> bool {
        let mut changed = self.take_replies();
        if self.reload() {
            self.report();
            changed = true;
        }
        changed |= self.reload_thread();
        changed |= self.take_focus();
        let toasting = self.toasts.any();
        changed |= toasting || self.toasting;
        self.toasting = toasting;
        // The ages move with the clock.
        changed | (Utc::now().timestamp() % 60 == 0)
    }

    // -----------------------------------------------------------------------------------------------------------------
    // Actions

    pub fn take_replies(&mut self) -> bool {
        let mut any = false;
        while let Ok(reply) = self.bridge.replies.try_recv() {
            any = true;
            self.calls = self.calls.saturating_sub(1);
            if let Some(error) = reply.error {
                self.toasts.push(error, Toast::Bad);
            } else if reply.pending.as_object().is_some_and(|p| !p.is_empty()) && reply.tag != "see" {
                self.toasts.push("asked a Claude Code session to do it", Toast::Note);
            }
            if !reply.note.is_empty() && !reply.note.starts_with("opened ") {
                self.toasts.push(reply.note, Toast::Good);
            }
        }
        any
    }

    fn call(&mut self, tag: &str, action: &str, message: &Message) {
        self.calls += 1;
        self.bridge.call(tag, "mail", action, vec![message.raw.clone()], None);
    }

    /// The selected message's actions, the main one first: read it whole, reply, archive, mark read, the page beside, the
    /// mail client.
    pub fn acts(&self) -> Vec<(Act, Cmd)> {
        let Some(m) = self.selected_message() else { return Vec::new() };
        let id = m.id.clone();
        let maps = |verb: &str| self.config["mail"]["tools"][verb]["tool"].as_str().is_some_and(|t| !t.is_empty());
        let mut acts = Vec::new();
        if !matches!(self.thread, Thread::Ready(_)) {
            acts.push((Act::new("read", '⏎', "see").main(), Cmd::See(id.clone())));
        }
        acts.push((Act::new("reply", 'c', "reply"), Cmd::Reply(id.clone())));
        if maps("archive") {
            acts.push((Act::new("archive", 'b', "archive"), Cmd::Archive(id.clone())));
        }
        if m.unread && maps("markRead") {
            acts.push((Act::new("mark read", 'm', "read"), Cmd::Read(id.clone())));
        }
        acts.push((Act::new("html", 'h', "html"), Cmd::Html(id.clone())));
        if !m.url.is_empty() {
            acts.push((Act::new("open", 'o', "open"), Cmd::Open(id)));
        }
        acts
    }

    pub fn run(&mut self, cmd: Cmd) {
        let message = |id: &str| self.mail.message(id).cloned();
        match cmd {
            Cmd::See(id) => {
                let Some(m) = message(&id) else { return };
                self.select(&id);
                self.thread_of = None;
                self.reload_thread();
                if !matches!(self.thread, Thread::Ready(_)) {
                    self.call("see", "mail_thread", &m);
                }
            }
            Cmd::Open(id) => {
                if let Some(m) = message(&id) {
                    self.call("open", "mail_open", &m);
                }
            }
            Cmd::Reply(id) => {
                if let Some(m) = message(&id) {
                    self.call("reply", "mail_reply", &m);
                    self.toasts.push(format!("an agent drafts a reply to {}", m.name), Toast::Note);
                }
            }
            Cmd::Archive(id) => {
                if let Some(m) = message(&id) {
                    self.call("archive", "mail_archive", &m);
                }
            }
            Cmd::Read(id) => {
                if let Some(m) = message(&id) {
                    self.call("read", "mail_read", &m);
                }
            }
            Cmd::Html(id) => {
                if self.selected.as_deref() == Some(id.as_str()) && matches!(self.thread, Thread::Ready(_)) {
                    self.open_page(&id);
                } else {
                    self.html_wanted = Some(id.clone());
                    self.run(Cmd::See(id));
                    self.toasts.push("reading the thread first", Toast::Note);
                }
            }
            Cmd::Refresh => {
                self.calls += 1;
                self.bridge.call("refresh", "mail", "request_refresh", Vec::new(), None);
            }
        }
    }

    /// The thread as a page in ~/.cache/cockpit/mail/<id>.html, opened in terminal-browser beside this pane.
    fn open_page(&mut self, id: &str) {
        let (Thread::Ready(messages), Some(m)) = (&self.thread, self.mail.message(id)) else { return };
        let path = thread::page_path(id);
        if let Err(error) = std::fs::create_dir_all(thread::dir()).and_then(|_| std::fs::write(&path, thread::page(m.title(), messages))) {
            self.toasts.push(format!("could not write the page: {error}"), Toast::Bad);
            return;
        }
        let Ok(own) = std::env::current_exe() else { return };
        std::thread::spawn(move || {
            let _ = std::process::Command::new(own).args(["browse", &format!("file://{}", path.display())]).output();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_selection_follows_the_list_and_the_acts_the_mapping() {
        let mut app = App::with(json!({"mail": {"tools": {"archive": {"tool": "archive_thread"}}}}), false);
        app.set_mail(&json!({"messages": [
            {"id": "a", "from": "noreply@ci.example.com", "subject": "Build", "date": "2026-11-04T10:00:00Z", "unread": true, "url": "https://mail/a"},
            {"id": "b", "from": "maya@example.com", "fromName": "Maya", "subject": "Lunch?", "date": "2026-11-04T08:00:00Z", "unread": true}
        ]}));
        assert_eq!(app.selected.as_deref(), Some("b"), "the first that needs me");
        let labels = |app: &App| app.acts().iter().map(|(a, _)| a.label.clone()).collect::<Vec<_>>();
        assert_eq!(labels(&app), ["read", "reply", "archive", "html"]);
        app.move_sel(1);
        assert_eq!(app.selected.as_deref(), Some("a"));
        assert_eq!(labels(&app), ["read", "reply", "archive", "html", "open"]);
        app.set_mail(&json!({"messages": [{"id": "b", "from": "maya@example.com", "unread": false}]}));
        assert_eq!(app.selected.as_deref(), Some("b"), "an archived message's place goes to the first");
    }
}
