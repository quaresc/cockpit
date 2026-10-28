//! The list's rows: by app (troubled apps first and unfolded, each with its alerting monitors, new issues and releases
//! under it) or as a timeline of every release, newest first; and each row's actions.

use cockpit_core::acts::Act;
use serde_json::json;

use crate::app::{App, Cmd};
use crate::data::{self, Doc, id_text};

/// What a row stands for, by the ids the document gives.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    App(String),
    Monitor(String, String),
    Issue(String, String),
    Release(String, String),
}

impl Item {
    pub fn app(&self) -> &str {
        match self {
            Item::App(app) | Item::Monitor(app, _) | Item::Issue(app, _) | Item::Release(app, _) => app,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Row {
    pub key: String,
    pub item: Item,
    pub depth: usize,
    /// The guide drawn before a child row: "├ " or "└ ".
    pub guide: &'static str,
    /// An app with rows under it, and whether they show.
    pub fold: Option<bool>,
}

/// The apps in the order they show: those in trouble first, each group in the document's order.
pub fn ordered(doc: &Doc) -> Vec<&data::App> {
    let (troubled, rest): (Vec<_>, Vec<_>) = doc.apps.iter().partition(|a| a.is_troubled());
    troubled.into_iter().chain(rest).collect()
}

impl App {
    pub fn rows(&self) -> Vec<Row> {
        let Some(doc) = &self.doc else { return Vec::new() };
        if self.timeline {
            return timeline(doc);
        }
        let mut rows = Vec::new();
        for app in ordered(doc) {
            let mut kids: Vec<(String, Item)> = Vec::new();
            for m in app.alerting() {
                let id = id_text(&m.id);
                kids.push((format!("{}/monitor/{id}", app.id), Item::Monitor(app.id.clone(), id)));
            }
            for issue in app.new_issues() {
                kids.push((format!("{}/issue/{}", app.id, issue.id), Item::Issue(app.id.clone(), issue.id.clone())));
            }
            for release in &app.releases {
                let id = id_text(&release.id);
                kids.push((format!("{}/release/{id}", app.id), Item::Release(app.id.clone(), id)));
            }
            let open = self.is_open(&app.id);
            let fold = (!kids.is_empty()).then_some(open);
            rows.push(Row { key: app.id.clone(), item: Item::App(app.id.clone()), depth: 0, guide: "", fold });
            if open {
                let last = kids.len().saturating_sub(1);
                for (n, (key, item)) in kids.into_iter().enumerate() {
                    rows.push(Row { key, item, depth: 1, guide: if n == last { "└ " } else { "├ " }, fold: None });
                }
            }
        }
        rows
    }

    pub fn selected_row(&self) -> Option<Row> {
        let rows = self.rows();
        let key = self.selected.as_ref()?;
        rows.into_iter().find(|r| &r.key == key)
    }

    pub fn move_sel(&mut self, by: isize) {
        let rows = self.rows();
        if rows.is_empty() {
            return;
        }
        let at = self.selected.as_ref().and_then(|k| rows.iter().position(|r| &r.key == k)).unwrap_or(0) as isize;
        self.selected = Some(rows[(at + by).clamp(0, rows.len() as isize - 1) as usize].key.clone());
        self.follow = true;
    }

    pub fn select_edge(&mut self, last: bool) {
        let rows = self.rows();
        let edge = if last { rows.last() } else { rows.first() };
        self.selected = edge.map(|r| r.key.clone());
        self.follow = true;
    }

    /// The previous or next app's row, by app; the previous or next release in the timeline.
    pub fn move_group(&mut self, by: isize) {
        let rows = self.rows();
        let at = self.selected.as_ref().and_then(|k| rows.iter().position(|r| &r.key == k)).unwrap_or(0);
        let heads: Vec<usize> = rows.iter().enumerate().filter(|(_, r)| r.depth == 0).map(|(i, _)| i).collect();
        let next = if by > 0 { heads.iter().find(|&&i| i > at) } else { heads.iter().rev().find(|&&i| i < at) };
        if let Some(&i) = next {
            self.selected = Some(rows[i].key.clone());
            self.follow = true;
        }
    }

    /// The row's actions in order, the main one marked; empty for a row whose data is gone.
    pub fn acts_for(&self, item: &Item) -> Vec<(Act, Cmd)> {
        let Some(app) = self.doc.as_ref().and_then(|d| d.app(item.app())) else { return Vec::new() };
        let id = app.id.clone();
        let mut acts = Vec::new();
        match item {
            Item::App(_) => {
                let wrong = app.is_troubled();
                let investigate = (Act::new("investigate", 'i', "investigate"), Cmd::Investigate(id.clone(), json!({})));
                let mut open = (Act::new("open", 'o', "open"), Cmd::OpenUrl(app.datadog_url.clone()));
                open.0.main = !wrong;
                if wrong {
                    acts.push((investigate.0.main(), investigate.1));
                    acts.push(open);
                } else {
                    acts.push(open);
                    acts.push(investigate);
                }
                if !app.argo_url.is_empty() {
                    acts.push((Act::new("ArgoCD", 'A', "argocd"), Cmd::OpenUrl(app.argo_url.clone())));
                }
                acts.push((Act::new(if app.pinned { "unpin" } else { "pin" }, 'p', "pin"), Cmd::Pin(id.clone(), !app.pinned)));
                acts.push((Act::new("refresh", 'R', "refresh"), Cmd::Refresh));
            }
            Item::Monitor(_, monitor) => {
                let url = app.alerting().iter().find(|m| &id_text(&m.id) == monitor).map(|m| m.url.clone()).unwrap_or_default();
                acts.push((Act::new("open", 'o', "open").main(), Cmd::OpenUrl(url)));
                acts.push((Act::new("investigate", 'i', "investigate"), Cmd::Investigate(id, json!({"monitor": monitor}))));
            }
            Item::Issue(_, issue) => {
                let url = app.issues.iter().find(|i| &i.id == issue).map(|i| i.url.clone()).unwrap_or_default();
                acts.push((Act::new("fix", 'f', "fix").main(), Cmd::Fix(id.clone(), issue.clone())));
                acts.push((Act::new("open", 'o', "open"), Cmd::OpenUrl(url)));
                acts.push((Act::new("investigate", 'i', "investigate"), Cmd::Investigate(id.clone(), json!({"issue": issue}))));
                acts.push((Act::new("bug", 'b', "bug"), Cmd::Bug(id, json!({"issue": issue}))));
            }
            Item::Release(_, release) => {
                let Some(r) = app.release(release) else { return Vec::new() };
                let bad = r.verdict.state.is_bad();
                let investigate = (Act::new("investigate", 'i', "investigate"), Cmd::Investigate(id.clone(), json!({"release": r.id})));
                let mut open = (Act::new("open", 'o', "open"), Cmd::OpenUrl(window_url(app, r)));
                open.0.main = !bad;
                if bad {
                    acts.push((investigate.0.main(), investigate.1));
                    acts.push(open);
                } else {
                    acts.push(open);
                    acts.push(investigate);
                }
                if let Some(previous) = app.previous_tag(release) {
                    let act = Act::new("roll back", 'u', "roll_back").confirm(&format!("rollback-{id}-{}", r.tag));
                    acts.push((act, Cmd::RollBack(id.clone(), release.clone(), previous)));
                }
                if r.is_mine && !r.tickets.is_empty() {
                    acts.push((Act::new("talk", 't', "talk"), Cmd::Talk(id.clone(), release.clone())));
                }
                if bad {
                    acts.push((Act::new("bug", 'b', "bug"), Cmd::Bug(id, json!({"release": r.id}))));
                }
            }
        }
        acts
    }

    pub fn acts(&self) -> Vec<(Act, Cmd)> {
        self.selected_row().map(|r| self.acts_for(&r.item)).unwrap_or_default()
    }
}

/// Every release of every app, newest first.
fn timeline(doc: &Doc) -> Vec<Row> {
    let mut releases: Vec<(&data::App, &data::Release)> = doc.apps.iter().flat_map(|a| a.releases.iter().map(move |r| (a, r))).collect();
    releases.sort_by_key(|(_, r)| std::cmp::Reverse(r.started_at.or(r.finished_at)));
    releases
        .into_iter()
        .map(|(app, r)| {
            let id = id_text(&r.id);
            Row { key: format!("{}/release/{id}", app.id), item: Item::Release(app.id.clone(), id), depth: 0, guide: "", fold: None }
        })
        .collect()
}

/// The app's Datadog page over the release's window: half an hour before it started to the end of its watch.
pub fn window_url(app: &data::App, release: &data::Release) -> String {
    let (Some(start), Some(end)) = (release.started_at, release.verdict.until.or(release.at())) else { return app.datadog_url.clone() };
    let start = (start - chrono::TimeDelta::minutes(30)).timestamp_millis();
    let end = end.timestamp_millis();
    let glue = if app.datadog_url.contains('?') { '&' } else { '?' };
    format!("{}{glue}start={start}&end={end}&paused=true", app.datadog_url)
}
