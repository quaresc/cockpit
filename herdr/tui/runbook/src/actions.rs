//! What the keys and the palette run: a step's or a runbook's action through the bridge, and in the app itself the
//! steps marked done, commands copied and cells edited, each written straight into the runbook file.

use cockpit_core::acts::Act;
use cockpit_core::anim::Kind;
use cockpit_core::dialog::Dialog;
use cockpit_core::input::Input;
use cockpit_core::toast;
use serde_json::{Value, json};

use crate::app::{App, Cmd, key};
use crate::book::Book;
use crate::edit::Editor;
use crate::handoff;
use crate::table::{self, Column};

impl App {
    /// The runbook as the hub resolves it: by its slug, or whole when the app reads another folder than the hub's.
    fn book_arg(&self, book: &Book) -> Value {
        if std::env::var("COCKPIT_RUNBOOKS_DIR").is_ok() { book.json() } else { json!({"$book": book.slug}) }
    }

    pub fn refresh(&mut self, slug: &str) {
        if self.asking.contains(slug) || self.refreshing.get(slug).is_some_and(|(pid, _)| crate::app::is_alive(*pid)) {
            self.toasts.push(format!("{slug} is already refreshing"), toast::Kind::Good);
            return;
        }
        self.newly.remove(slug);
        if let Some(book) = self.books.iter().find(|b| b.slug == slug) {
            let arg = self.book_arg(book);
            self.asking.insert(slug.to_string());
            self.call(&format!("refresh:{slug}"), "update", vec![arg], None);
        }
    }

    fn call(&mut self, tag: &str, function: &str, args: Vec<Value>, armed: Option<String>) {
        self.calls += 1;
        self.bridge.call(tag, "runbook", function, args, armed);
    }

    /// Runs an action; `armed` is the confirm key, given once its dialog said yes.
    pub fn perform(&mut self, slug: &str, n: Option<&str>, act: &Act, armed: Option<String>) {
        let Some(book) = self.books.iter().find(|b| b.slug == slug).cloned() else { return };
        let step = n.and_then(|n| book.steps.iter().find(|s| s.n == n)).cloned();
        let arg = self.book_arg(&book);
        match (act.id.as_str(), step) {
            ("refresh", _) => self.refresh(slug),
            ("talk", _) => self.call("talk", "talk", vec![arg], None),
            ("delete", _) => self.call("delete", "delete", vec![arg], armed),
            ("run", Some(step)) => self.call("run", "run_step", vec![arg, step.json()], armed),
            ("done", Some(step)) => self.mark_done(vec![(slug.to_string(), step.n)]),
            ("edit", Some(step)) => self.edit(slug, &step.n, Column::Command),
            ("session", Some(step)) => {
                let id = crate::acts::handoff_id(&book, &step);
                self.call("session", "focus_session", vec![json!({"$handoff": id})], None);
            }
            ("open", Some(step)) => {
                if let Some((repo, number)) = crate::acts::pr_ref(&step) {
                    self.call("open", "open_pr", vec![repo.into(), number.into()], None);
                }
            }
            ("copy", Some(step)) => self.copy(&[step.command]),
            _ => {}
        }
    }

    /// An action, asked about first when it changes something hard to undo.
    pub fn act(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Act { slug, n, act, question } if act.confirm.is_some() => {
                let ask = Dialog::Confirm { question: format!("{question}?") };
                self.dialog = Some((ask, Cmd::Act { slug, n, act, question }));
            }
            Cmd::Act { slug, n, act, .. } => self.perform(&slug, n.as_deref(), &act, None),
            Cmd::GoTo { slug, n } => self.focus(&key(&slug, &n)),
            Cmd::Done(steps) => self.mark_done(steps),
        }
    }

    /// What a dialog said yes to.
    pub fn confirmed(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Act { slug, n, act, .. } => self.perform(&slug, n.as_deref(), &act, act.confirm.clone()),
            other => self.act(other),
        }
    }

    /// The picked steps in table order, as (slug, number); the step under the cursor when none is picked.
    pub fn targets(&self) -> Vec<(String, String)> {
        if self.picked.is_empty() {
            return self.book().and_then(|b| b.steps.get(self.selected).map(|s| vec![(b.slug.clone(), s.n.clone())])).unwrap_or_default();
        }
        self.books.iter().flat_map(|b| b.steps.iter().filter(|s| self.picked.contains(&key(&b.slug, &s.n))).map(|s| (b.slug.clone(), s.n.clone()))).collect()
    }

    /// The picked steps marked done together, after one question.
    pub fn ask_done(&mut self) {
        let steps: Vec<(String, String)> = self.targets().into_iter().filter(|(slug, n)| self.step(slug, n).is_some_and(|s| s.is_open())).collect();
        if steps.is_empty() {
            self.toasts.push("the picked steps are done already", toast::Kind::Note);
            return;
        }
        let mut by_book: Vec<(String, Vec<String>)> = Vec::new();
        for (slug, n) in &steps {
            match by_book.last_mut() {
                Some((last, ns)) if last == slug => ns.push(n.clone()),
                _ => by_book.push((slug.clone(), vec![n.clone()])),
            }
        }
        let which: Vec<String> = by_book.iter().map(|(slug, ns)| format!("{} {}", slug.to_uppercase(), ns.join(", "))).collect();
        let s = if steps.len() == 1 { "" } else { "s" };
        let question = format!("Mark {} step{s} done: {}?", steps.len(), which.join(" · "));
        self.dialog = Some((Dialog::Confirm { question }, Cmd::Done(steps)));
    }

    fn step(&self, slug: &str, n: &str) -> Option<&crate::book::Step> {
        self.books.iter().find(|b| b.slug == slug)?.steps.iter().find(|s| s.n == n)
    }

    /// Writes the steps' status as done, one write per runbook; the reload that follows plays them as a cascade.
    pub fn mark_done(&mut self, steps: Vec<(String, String)>) {
        let day = self.clock.today();
        let mut slugs: Vec<&String> = steps.iter().map(|(slug, _)| slug).collect();
        slugs.dedup();
        for slug in slugs {
            let Some(book) = self.books.iter().find(|b| &b.slug == slug).cloned() else { continue };
            let ns: Vec<String> = steps.iter().filter(|(s, _)| s == slug).map(|(_, n)| n.clone()).collect();
            let written = std::fs::read_to_string(&book.path).and_then(|text| {
                let (text, found) = table::mark_done(&text, &ns, &day);
                table::write(&book.path, &text).map(|_| found)
            });
            match written {
                Ok(found) => {
                    for step in book.steps.iter().filter(|s| found.contains(&s.n)) {
                        if let Some(h) = self.handoffs.get(&crate::acts::handoff_id(&book, step)) {
                            handoff::close(&self.paths.handoffs, h);
                        }
                    }
                }
                Err(err) => self.toasts.push(format!("{}: {err}", book.path.display()), toast::Kind::Bad),
            }
        }
        self.picked.clear();
        self.reread();
    }

    /// The commands of these steps on the clipboard, one per line.
    pub fn copy(&mut self, commands: &[String]) {
        let commands: Vec<&str> = commands.iter().map(String::as_str).filter(|c| !c.is_empty()).collect();
        if commands.is_empty() {
            self.toasts.push("no command to copy", toast::Kind::Note);
            return;
        }
        let note = if !cockpit_core::files::copy(&commands.join("\n")) {
            "no clipboard tool: install wl-copy or xclip".to_string()
        } else if commands.len() == 1 {
            format!("copied: {}", commands[0])
        } else {
            format!("copied {} commands", commands.len())
        };
        self.toasts.push(note, toast::Kind::Good);
    }

    /// The picked steps' commands copied, in table order.
    pub fn copy_picked(&mut self) {
        let commands: Vec<String> = self.targets().iter().filter_map(|(slug, n)| self.step(slug, n).map(|s| s.command.clone())).collect();
        self.copy(&commands);
    }

    /// Opens the text field on a cell of step `n`, holding its value.
    pub fn edit(&mut self, slug: &str, n: &str, column: Column) {
        let Some(step) = self.step(slug, n) else { return };
        let value = if column == Column::Command { &step.command } else { &step.status };
        self.editor = Some(Editor::Cell { slug: slug.into(), n: n.into(), column, input: Input::new(value) });
    }

    /// Opens the form for a step under the cursor.
    pub fn add(&mut self) {
        let Some(book) = self.book() else { return };
        let after = book.steps.get(self.selected).map(|s| s.n.clone());
        self.editor = Some(Editor::Add { slug: book.slug.clone(), after, fields: Default::default(), at: 0 });
    }

    /// Writes what the editor holds into its runbook; the editor stays open when there is nothing to write yet.
    pub fn save(&mut self) {
        let Some(editor) = self.editor.take() else { return };
        let (slug, path) = match &editor {
            Editor::Cell { slug, .. } | Editor::Add { slug, .. } => (slug.clone(), self.books.iter().find(|b| &b.slug == slug).map(|b| b.path.clone())),
        };
        let Some(path) = path else { return };
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) => return self.toasts.push(format!("{}: {err}", path.display()), toast::Kind::Bad),
        };
        let (text, n, note) = match &editor {
            Editor::Cell { n, column, input, .. } => {
                let Some(text) = table::set_cell(&text, n, *column, &input.text) else {
                    return self.toasts.push(format!("step {n} is no longer in {}", slug.to_uppercase()), toast::Kind::Bad);
                };
                let what = if *column == Column::Command { "command" } else { "status" };
                (text, n.clone(), format!("{} step {n}: {what} saved", slug.to_uppercase()))
            }
            Editor::Add { after, fields, .. } => {
                if fields[0].text.trim().is_empty() {
                    self.toasts.push("a step needs its text", toast::Kind::Note);
                    self.editor = Some(editor);
                    return;
                }
                let (text, n) = table::insert_step(&text, after.as_deref(), &fields[0].text, &fields[1].text, &fields[2].text);
                let note = format!("{} step {n} added", slug.to_uppercase());
                (text, n, note)
            }
        };
        if let Err(err) = table::write(&path, &text) {
            return self.toasts.push(format!("{}: {err}", path.display()), toast::Kind::Bad);
        }
        self.reread();
        if let Some(i) = self.books.iter().position(|b| b.slug == slug) {
            self.go_tab(i);
            if let Some(at) = self.books[i].steps.iter().position(|s| s.n == n) {
                self.selected = at;
                self.reveal = Some((at, at));
            }
        }
        self.keep_cursor();
        let kind = if matches!(editor, Editor::Add { .. }) { Kind::NEW } else { Kind::GOOD };
        self.flash(&key(&slug, &n), kind);
        self.toasts.push(note, toast::Kind::Good);
    }
}
