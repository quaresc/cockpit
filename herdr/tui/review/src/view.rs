//! The frame: the header (whose worktree, the comments' recipient, when it was read), the scopes as tabs with what the
//! scope holds, the body (the diff, the walkthrough over it, or a state saying why there is nothing to show and how to
//! get something), the footer, and the toasts, picker and palette over them.

use cockpit_core::text::{fit, width, wrap};
use cockpit_core::theme::{BG, FG, LINE, ORANGE, PANEL, base, bold, dim, fg};
use cockpit_core::widgets;
use cockpit_show::row::{Row, Target};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

use crate::app::{App, Mode, Scope};
use crate::pinned::ICON;
use crate::walk;

impl App {
    pub fn draw(&mut self, frame: &mut Frame) {
        self.take_replies();
        let area = frame.area();
        frame.render_widget(Block::new().style(base()), area);
        let [head, tabs, _, body, footer] =
            Layout::vertical([Constraint::Length(1), Constraint::Length(1), Constraint::Length(1), Constraint::Min(1), Constraint::Length(1)]).areas(area);
        // The shared header: the space reads no feed source, so it draws no chip.
        let cells = head.width.saturating_sub(2) as usize;
        let title = cockpit_core::header::lines(self.head_line(cells), &[], cells).into_iter().next().unwrap_or_default();
        frame.render_widget(Paragraph::new(title), inset(head));
        self.draw_tabs(frame, inset(tabs));
        self.draw_body(frame, inset(body));
        self.draw_footer(frame, footer);
        self.toasts.render(frame, Rect { height: area.height.saturating_sub(1), ..area });
        if let Some((dialog, _)) = &self.dialog {
            dialog.render(frame, area);
        }
        if self.palette.is_open {
            self.palette.render(frame, area);
        }
    }

    /// "  Review   To Daily quotas · api · me/eng-412   updated 10:42": the space, who the comments go to, the read.
    fn head_line(&self, cells: usize) -> Line<'static> {
        let mut spans = vec![Span::styled(format!("{ICON}  "), fg(ORANGE)), Span::styled("Review", bold(ORANGE))];
        let right = self.here().read_at.map(|ms| format!("updated {}", (self.clock)(ms))).unwrap_or_default();
        let room = cells.saturating_sub(8 + width(&right) + 3);
        match self.subject() {
            Some(agent) => {
                let branch = self.here().about["branch"].as_str().filter(|b| !b.is_empty()).map(str::to_string).unwrap_or_else(|| agent.branch.clone());
                let text = [agent.repo_name().to_string(), branch].into_iter().filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" · ");
                let name = self.recipient().map_or_else(|| agent.name.clone(), |r| r.name());
                let to = fit(&format!("To {name}"), room.saturating_sub(width(&text) + 3).max(room / 2)).trim_end().to_string();
                let rest = fit(&format!(" · {text}"), room.saturating_sub(width(&to))).trim_end().to_string();
                spans.push(Span::styled(format!("   {to}"), fg(FG)));
                spans.push(Span::styled(rest, dim()));
            }
            None if self.agents.is_none() && self.hub_error.is_none() => spans.push(Span::styled("   looking for your agents…", dim())),
            None => {}
        }
        let used: usize = spans.iter().map(|s| width(&s.content)).sum();
        if !right.is_empty() && used + width(&right) + 2 <= cells {
            spans.push(Span::raw(" ".repeat(cells - used - width(&right))));
            spans.push(Span::styled(right, dim()));
        }
        Line::from(spans)
    }

    /// The scopes as tabs, the mode, and where the changes come from: "→ main · 3 commits · #2198".
    fn draw_tabs(&mut self, frame: &mut Frame, area: Rect) {
        self.hits.tabs.clear();
        if self.subject().is_none() {
            return;
        }
        let mut x = area.x;
        let mut spans = Vec::new();
        for (i, scope) in Scope::ALL.into_iter().enumerate() {
            let on = scope == self.scope;
            let text = format!(" {} {} ", i + 1, scope.label());
            let style = if on { Style::new().fg(BG).bg(ORANGE).add_modifier(Modifier::BOLD) } else { Style::new().fg(FG).bg(LINE) };
            let w = width(&text) as u16;
            self.hits.tabs.push((Rect { x, y: area.y, width: w, height: 1 }, scope));
            spans.push(Span::styled(text, style));
            spans.push(Span::raw(" "));
            x += w + 1;
        }
        let here = self.here();
        if self.mode == Mode::Walk {
            spans.push(Span::styled(" walkthrough ", Style::new().fg(ORANGE).bg(PANEL).add_modifier(Modifier::BOLD)));
        }
        let mut facts = Vec::new();
        let about = &here.about;
        let mut more = Vec::new();
        if self.scope != Scope::Uncommitted
            && let Some(base) = about["base"].as_str().filter(|b| !b.is_empty())
        {
            more.push(format!("→ {base}"));
        }
        if self.scope == Scope::Branch
            && let Some(n) = about["commits"].as_u64()
        {
            more.push(format!("{n} commit{}", if n == 1 { "" } else { "s" }));
        }
        if let Some(n) = about["pr"]["number"].as_u64() {
            more.push(format!("#{n}{}", if about["pr"]["isDraft"] == true { " draft" } else { "" }));
        }
        if !more.is_empty() {
            facts.push(Span::styled(format!("  {}", more.join(" · ")), dim()));
        }
        let used: usize = spans.iter().map(|s| width(&s.content)).sum();
        let facts_w: usize = facts.iter().map(|s| width(&s.content)).sum();
        if used + facts_w <= area.width as usize {
            spans.extend(facts);
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    /// What the body shows: a state when there is nothing to review, else the walkthrough and the diff.
    fn body_rows(&self, cols: usize) -> Vec<Row> {
        let Some(agent) = self.subject() else { return self.no_subject(cols) };
        let here = self.here();
        if let Some(error) = &here.error {
            return self.error_rows(error, cols);
        }
        if !here.diff.has_read() {
            return vec![Row::text("Reading the changes…", dim())];
        }
        let branch = here.about["branch"].as_str().filter(|b| !b.is_empty()).unwrap_or(&agent.branch).to_string();
        if here.diff.files().is_empty() {
            let mut hint = match self.scope {
                Scope::Uncommitted => "The working tree is clean. 2 shows what the branch adds.".to_string(),
                Scope::Branch => format!("It adds nothing to {} yet.", here.about["base"].as_str().unwrap_or("its base")),
                Scope::Pr => "The pull request changes nothing.".to_string(),
            };
            if self.mode == Mode::Walk {
                hint += " w goes back to the diff.";
            }
            let mut rows = vec![Row::text(format!("Nothing to review on {branch}"), bold(FG))];
            rows.extend(text_rows(&hint, cols, dim()));
            return rows;
        }
        let mut rows = Vec::new();
        let status = walk::status(&here.walkthrough);
        if self.mode == Mode::Walk {
            rows.extend(walk::rows(&here.walkthrough, here.step, self.model, cols, self.tick));
            if status != "done" {
                return rows;
            }
            rows.push(Row::new());
        }
        rows.extend(here.diff.rows(cols, Some(&Target::Diff)));
        rows
    }

    fn no_subject(&self, cols: usize) -> Vec<Row> {
        match (&self.agents, &self.hub_error) {
            (None, None) => vec![Row::text("Looking for your agents…", dim())],
            (None, Some(error)) => {
                let mut rows = vec![Row::text("The cockpit hub did not answer", bold(FG))];
                rows.extend(text_rows(&format!("{error}. R asks the hub again, starting it when it is not running."), cols, dim()));
                rows
            }
            _ => {
                let mut rows = vec![Row::text("No agent to review", bold(FG))];
                let text = "Start an agent in a space at a git repository: its changes show here, to comment line by line and send back to it. W picks among the agents' worktrees once there are several.";
                rows.extend(text_rows(text, cols, dim()));
                rows
            }
        }
    }

    fn error_rows(&self, error: &str, cols: usize) -> Vec<Row> {
        let branch = self.here().about["branch"].as_str().unwrap_or("this branch").to_string();
        let (title, text) = match self.scope {
            Scope::Pr if error.starts_with("gh did not start") => {
                ("No pull request to show".to_string(), "gh, GitHub's command line, is not installed: brew install gh, then gh auth login.".to_string())
            }
            Scope::Pr if error.contains("gh auth login") => {
                ("No pull request to show".to_string(), "gh is not signed in to GitHub: run gh auth login, then R.".to_string())
            }
            Scope::Pr if error.contains("no pull requests found") => (format!("No pull request for {branch} yet"), "2 shows what the branch adds.".to_string()),
            Scope::Pr => ("No pull request to show".to_string(), format!("{error}. R asks again.")),
            _ => ("Could not read the changes".to_string(), format!("{error}. R reads them again.")),
        };
        let mut rows = vec![Row::text(title, bold(FG))];
        rows.extend(text_rows(&text, cols, dim()));
        rows
    }

    fn draw_body(&mut self, frame: &mut Frame, area: Rect) {
        self.hits.body = area;
        self.hits.rows.clear();
        self.height = area.height as usize;
        let cols = (area.width as usize).saturating_sub(1);
        // Built again only when something they show changed: a large diff costs a frame's worth of time to lay out.
        if std::mem::take(&mut self.dirty) || self.rows.as_ref().is_none_or(|(w, _)| *w != cols) {
            self.rows = Some((cols, self.body_rows(cols)));
        }
        let rows = self.rows.take().map(|(_, rows)| rows).unwrap_or_default();
        let height = area.height as usize;
        if std::mem::take(&mut self.reveal)
            && let Some(at) = rows.iter().position(|r| r.here)
        {
            let last = rows.iter().rposition(|r| r.here).unwrap_or(at);
            if last >= self.scroll + height {
                self.scroll = (last + 1).saturating_sub(height);
            }
            self.scroll = self.scroll.min(at);
        }
        self.scroll = self.scroll.min(rows.len().saturating_sub(height));
        for (k, row) in rows.iter().skip(self.scroll).take(height).enumerate() {
            let y = area.y + k as u16;
            frame.render_widget(Paragraph::new(row.line.clone()), Rect { y, height: 1, width: cols as u16, ..area });
            if !row.clicks.is_empty() {
                self.hits.rows.push((y, area.x, row.clicks.clone()));
            }
        }
        widgets::scrollbar(frame, area, self.scroll, rows.len(), height);
        self.rows = Some((cols, rows));
    }

    fn draw_footer(&self, frame: &mut Frame, area: Rect) {
        let here = self.here();
        let notes = here.diff.notes().len();
        let walking = self.mode == Mode::Walk && !here.diff.files().is_empty() && here.error.is_none();
        // With no agent to send it to, S starts one: it says so.
        let verb = if matches!(self.recipient(), Some(crate::app::Recipient::New)) { "start an agent" } else { "send" };
        let send = if notes > 0 { format!("{verb} {notes}") } else { verb.to_string() };
        // q first: the footer drops hints from the end when the pane is narrow, and how to leave must survive.
        let keys: Vec<(&str, &str)> = if here.diff.is_typing() {
            vec![("enter", "add"), ("esc", "cancel")]
        } else if self.subject().is_none() {
            vec![("q", "quit"), ("R", "refresh"), ("?", "keys")]
        } else if here.error.is_some() || here.diff.files().is_empty() {
            let back = if self.mode == Mode::Walk { vec![("w", "diff")] } else { vec![] };
            [vec![("q", "quit"), ("1-3", "scope")], back, vec![("R", "refresh"), ("W", "worktree"), ("?", "keys")]].concat()
        } else if walking && walk::status(&here.walkthrough) != "done" {
            match walk::status(&here.walkthrough) {
                "running" => vec![("q", "quit"), ("s", "stop"), ("w", "diff"), ("W", "worktree"), ("?", "keys")],
                _ => vec![("q", "quit"), ("r", "write it"), ("m", "model"), ("w", "diff"), ("W", "worktree"), ("?", "keys")],
            }
        } else if walking {
            vec![("q", "quit"), ("h/l", "step"), ("n/p", "hunk"), ("c", "comment"), ("S", send.as_str()), ("w", "diff"), ("?", "keys")]
        } else {
            vec![
                ("q", "quit"),
                ("j/k", "line"),
                ("n/p", "hunk"),
                ("c", "comment"),
                ("S", send.as_str()),
                ("T", "to"),
                ("1-3", "scope"),
                ("w", "walkthrough"),
                ("W", "worktree"),
                ("?", "keys"),
            ]
        };
        let mut spans = Vec::new();
        for (key, what) in keys {
            spans.push(Span::styled(format!(" {key}"), bold(ORANGE)));
            spans.push(Span::styled(format!(" {what} "), fg(FG)));
        }
        let palette = vec![Span::styled("▏", fg(LINE)), Span::styled("^p", bold(ORANGE)), Span::styled(" palette ", fg(FG))];
        let used = |spans: &[Span]| -> usize { spans.iter().chain(&palette).map(|s| width(&s.content)).sum() };
        while !spans.is_empty() && used(&spans) > area.width as usize {
            spans.truncate(spans.len() - 2);
        }
        spans.push(Span::raw(" ".repeat((area.width as usize).saturating_sub(used(&spans)))));
        spans.extend(palette);
        frame.render_widget(Paragraph::new(Line::from(spans)).style(Style::new().bg(PANEL)), area);
    }
}

fn text_rows(text: &str, cols: usize, style: Style) -> Vec<Row> {
    wrap(text, cols.max(10)).into_iter().map(|l| Row::text(l, style)).collect()
}

fn inset(area: Rect) -> Rect {
    Rect { x: area.x + 1, width: area.width.saturating_sub(2), ..area }
}
