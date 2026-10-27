//! inbox.py's build as rows of a tree: the header and its error lines, then Today, Alerts, Review, Work and Other PRs.

use std::collections::HashSet;

use cockpit_core::files;
use cockpit_core::theme::{DIM, GREEN, ORANGE, RED, bold, dim, fg};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::{Value, json};

use crate::facts::{self, Config, OLD_REVIEW_DAYS, PREVIEW_WARN_HOURS, RED_ALERT_HOURS, age_days, ago, first_name, flag, list, reference, s};
use crate::icons::icon;
use crate::row::{Cells, Row, RowAct, call, open};
use crate::sessions::Local;
use crate::work;

pub const INBOX_VERSION: i64 = 1;

pub fn heading(name: &str, title: &str, extra: &str) -> Line<'static> {
    Line::from(vec![Span::styled(format!("{}  ", icon(name)), fg(ORANGE)), Span::styled(title.to_string(), bold(ORANGE)), Span::styled(format!("   {extra}"), dim())])
}

pub fn strong() -> Style {
    Style::new().add_modifier(Modifier::BOLD)
}

fn load(name: &str) -> Option<Value> {
    files::load(&files::cache().join(name))
}

pub fn muted() -> Vec<String> {
    files::load(&files::config_dir().join("muted.json")).and_then(|v| v.as_array().map(|a| a.iter().filter_map(|r| r.as_str().map(str::to_string)).collect())).unwrap_or_default()
}

/// The header with its error lines, and the tree's top rows, from the caches.
pub fn model(by_status: bool, config: &Config, local: &mut Local) -> (Vec<Line<'static>>, Vec<Row>) {
    let Some(data) = load("inbox.json") else {
        return (vec![Line::styled("No data yet: the cockpit mod in a Claude session fills ~/.cache/cockpit/inbox.json.", dim())], Vec::new());
    };
    if data["version"].as_i64() != Some(INBOX_VERSION) {
        return (vec![Line::styled("Refreshing: the inbox cache comes from another cockpit version.", dim())], Vec::new());
    }
    let ci = load("ci.json").unwrap_or_else(|| json!({"runs": []}));
    let mut head = vec![heading("inbox", "Inbox", &format!("updated {}", facts::clock(&data["at"])))];
    let errors = list(&data, "errors").iter().chain(std::iter::once(&ci["error"]).filter(|e| e.is_string()));
    head.extend(errors.map(|e| Line::styled(e.as_str().unwrap_or_default().to_string(), fg(RED))));

    let mut top = Vec::new();
    if let Some(today) = today() {
        top.push(today);
    }
    let alerts = alerts(&data, &ci, config, local);
    if !alerts.children.is_empty() {
        top.push(alerts);
    }
    top.push(review(&data));
    let (work, other) = work::rows(&data, &ci, config, by_status);
    top.push(work);
    top.push(other);
    (head, top)
}

/// The morning brief, when it is today's and not dismissed.
fn today() -> Option<Row> {
    let brief = load("brief.json")?;
    if s(&brief, "date") != facts::today() || flag(&brief, "dismissed") || list(&brief, "lines").is_empty() {
        return None;
    }
    let written = facts::clock(&brief["at"]);
    let mut today = Row::text("today", heading("today", "Today", &format!("morning brief, {written}"))).acts(vec![RowAct::new("dismiss", 'x', call("dismiss_brief", vec![])).main()]);
    for (n, entry) in list(&brief, "lines").iter().enumerate() {
        let style = match s(entry, "tone") {
            "bad" => fg(RED),
            "warn" => fg(ORANGE),
            "good" => fg(GREEN),
            _ => Style::new(),
        };
        let glyph_style = if style == Style::new() { dim() } else { style };
        let text = s(entry, "text").to_string();
        let line = Line::from(vec![Span::styled(format!("{}  ", icon(s(entry, "icon"))), glyph_style), Span::styled(text.clone(), style)]);
        let mut row = Row::text(format!("brief:{n}"), line).info(vec![Line::styled(text.clone(), style)]).name(text).section("Today");
        if !s(entry, "url").is_empty() {
            row = row.acts(vec![RowAct::new("open", 'o', open(s(entry, "url"))).main()]).primary(open(s(entry, "url")));
        }
        today.add(row);
    }
    Some(today)
}

fn alert(key: String, text: &str, style: Style) -> Row {
    Row::text(key, Line::styled(format!(" {}  {text}", icon("alert")), style)).name(text).section("Alerts")
}

/// Previews expiring, unhealthy or running a merged branch, red CI of the last hours, sessions left on merged branches.
fn alerts(data: &Value, ci: &Value, config: &Config, local: &mut Local) -> Row {
    let mut alerts = Row::text("alerts", heading("alert", "Alerts", ""));
    let previews = list(data, "previews");
    let mut ids: Vec<&str> = previews.iter().map(|b| s(b, "id")).collect::<HashSet<_>>().into_iter().collect();
    ids.sort();
    for preview in ids {
        let bx = previews.iter().find(|b| s(b, "id") == preview).expect("listed above");
        let Some(days) = age_days(s(bx, "expiresAt")) else { continue };
        let (hours, url) = (-days * 24.0, config.dashboard_url(preview));
        let with_url = |row: Row, acts: Vec<RowAct>| match &url {
            Some(url) => row.acts(acts.into_iter().chain([RowAct::new("open", 'o', open(url))]).collect()).primary(open(url)),
            None => row.acts(acts),
        };
        if hours <= PREVIEW_WARN_HOURS {
            let text = if hours <= 0.0 { format!("preview {preview} has expired") } else { format!("preview {preview} expires in {hours:.1} h") };
            let row = alert(format!("alert:expire:{preview}"), &text, fg(if hours <= 0.0 { RED } else { ORANGE })).status(format!("preview-{preview}"));
            let extend = RowAct::new("extend 2 days", 'a', call("extend", vec![json!(preview)])).main().confirm(format!("extend-{preview}"));
            alerts.add(with_url(row, vec![extend]));
        }
        if !matches!(s(bx, "health"), "Healthy" | "Progressing") && hours > 0.0 {
            let text = format!("preview {preview} is {}", s(bx, "health").to_lowercase());
            let mut row = with_url(alert(format!("alert:health:{preview}"), &text, fg(ORANGE)), vec![]);
            if let Some(open) = row.acts.first_mut() {
                open.act.main = true;
            }
            alerts.add(row);
        }
    }
    for run in list(ci, "runs") {
        let days = age_days(s(run, "createdAt"));
        if !flag(run, "isRunning") && s(run, "outcome") == "failed" && days.is_some_and(|d| d * 24.0 <= RED_ALERT_HOURS) {
            let job = if run["failed"].is_object() { format!(" · {}", s(&run["failed"], "job")) } else { String::new() };
            let text = format!("ci failed on {} {}{job}", s(run, "repo"), s(run, "branch"));
            alerts.add(alert(format!("alert:ci:{}:{}", s(run, "repo"), s(run, "branch")), &text, fg(RED)));
        }
    }
    let merged: HashSet<(String, String)> = list(data, "merged").iter().map(|m| (s(m, "repo").into(), s(m, "branch").into())).collect();
    for bx in previews {
        if flag(bx, "activated") && merged.contains(&(s(bx, "app").into(), s(bx, "branch").into())) {
            let id = s(bx, "id");
            let text = format!("preview {id} runs merged {} {}", s(bx, "app"), s(bx, "branch"));
            let mut acts = vec![RowAct::new("stop", 'a', call("stop_preview", vec![json!(id)])).main().confirm(format!("stop-{id}"))];
            let mut row = alert(format!("alert:merged:{id}"), &text, fg(ORANGE)).status(format!("preview-{id}"));
            if let Some(url) = config.dashboard_url(id) {
                acts.push(RowAct::new("open", 'o', open(&url)));
                row = row.primary(open(&url));
            }
            alerts.add(row.acts(acts));
        }
    }
    for (space, (repo, branch)) in local.merged_sessions(&merged, list(data, "mine"), config) {
        let label = format!("merged {repo} {branch}");
        let is_worktree = local.spaces().get(&space).is_some_and(|w| flag(&w["worktree"], "is_linked_worktree"));
        let tab = space.strip_prefix("tab:");
        let verb = if tab.is_some() { "close tab" } else if is_worktree { "remove worktree" } else { "close space" };
        let herdr = std::env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".into());
        let go = match tab {
            Some(tab) => json!([herdr, "tab", "focus", tab]),
            None => json!([herdr, "workspace", "focus", space]),
        };
        let text = format!("a session still works on {label}");
        let row = Row::text(format!("alert:session:{space}"), Line::styled(format!(" {}  {text}", icon("alert")), dim())).name(format!("session on {label}")).section("Alerts");
        let close = RowAct::new(verb, 'a', call("close_space", vec![json!(space), json!(label), json!(is_worktree)])).main().confirm(format!("close-{space}"));
        alerts.add(row.acts(vec![close, RowAct::new("go", 'o', call("run", vec![go]))]));
    }
    if !alerts.children.is_empty()
        && let crate::row::Body::Text(line) = &mut alerts.body
    {
        line.spans.push(Span::styled(alerts.children.len().to_string(), dim()));
    }
    alerts
}

fn review_row(pr: &Value) -> Row {
    let is_direct = flag(pr, "isDirect");
    let mut acts = vec![RowAct::new("review", 'a', call("review", vec![pr.clone()])).main(), RowAct::new("open", 'o', open(s(pr, "url")))];
    if !is_direct {
        acts.push(RowAct::new("mute repo", 'm', call("set_muted", vec![json!(s(pr, "repo")), json!(true)])));
    }
    let by = if s(pr, "authorName").is_empty() { s(pr, "author") } else { s(pr, "authorName") };
    let asked = if is_direct { "asked of you" } else { "asked of your team" };
    let info = vec![
        Line::from(vec![Span::styled(format!("{}  ", reference(pr)), dim()), Span::styled(s(pr, "title").to_string(), strong())]),
        Line::styled(format!("{asked} · by {by} · updated {} ago", ago(s(pr, "updatedAt"))), dim()),
        Line::styled(s(pr, "url").to_string(), dim()),
    ];
    let cells = Cells {
        lead: vec![if is_direct { (icon("user").into(), fg(ORANGE)) } else { (icon("team").into(), dim()) }],
        reference: (reference(pr), dim()),
        title: (s(pr, "title").into(), Style::new()),
        notes: (first_name(pr), dim()),
        age: ago(s(pr, "updatedAt")),
    };
    Row::cells(s(pr, "url"), cells, format!("{} {}", reference(pr), s(pr, "title")))
        .acts(acts)
        .primary(call("details", vec![pr.clone()]))
        .info(info)
        .status(s(pr, "url"))
        .section("Review")
}

/// Review requests: direct ones first, then team ones; old ones and muted repos folded away.
fn review(data: &Value) -> Row {
    let muted = muted();
    let is_muted = |repo: &str| muted.iter().any(|m| m == repo);
    let is_old = |pr: &&Value| age_days(s(pr, "updatedAt")).unwrap_or(0.0) > OLD_REVIEW_DAYS;
    let shown: Vec<&Value> = list(data, "team").iter().filter(|pr| !is_muted(s(pr, "repo"))).collect();
    let direct: Vec<&Value> = list(data, "direct").iter().filter(|pr| !is_old(pr)).collect();
    let team: Vec<&Value> = shown.iter().copied().filter(|pr| !is_old(pr)).collect();
    let old: Vec<&Value> = list(data, "direct").iter().chain(shown.iter().copied()).filter(is_old).collect();
    let bot_repos = list(data, "botRepos");
    let bots = bot_repos.iter().filter(|r| !is_muted(r.as_str().unwrap_or(""))).count();
    let hidden = list(data, "team").len() - shown.len() + bot_repos.len() - bots;
    let extra = format!("{} for you · {} team · {bots} bots", direct.len(), team.len()) + &if hidden > 0 { format!(" · {hidden} muted") } else { String::new() };
    let mut reviews = Row::text("review", heading("review", "Review", &extra));
    for pr in direct.iter().chain(&team) {
        reviews.add(review_row(pr));
    }
    if !old.is_empty() {
        let group = reviews.add(Row::text("review:old", Line::styled(format!("{} older than {OLD_REVIEW_DAYS} days", old.len()), dim())).folded());
        for pr in &old {
            group.add(review_row(pr));
        }
    }
    if !muted.is_empty() {
        let group = reviews.add(Row::text("review:muted", Line::styled(format!("{} {} muted repos", icon("mute"), muted.len()), dim())).folded());
        let mut repos = muted.clone();
        repos.sort();
        for repo in repos {
            let unmute = RowAct::new("unmute", 'a', call("set_muted", vec![json!(repo), json!(false)])).main();
            group.add(Row::text(format!("muted:{repo}"), Line::styled(repo.clone(), fg(DIM))).acts(vec![unmute]).name(repo).section("Muted"));
        }
    }
    reviews
}
