//! The inbox's caches as rows of a tree: the header and its error lines, then Today, Alerts, Review, Work and Other PRs.
//! A source the hub reads says how it fails in its header chip, so its errors are no line of their own.

use std::collections::{HashMap, HashSet};

use cockpit_core::theme::{DIM, GREEN, ORANGE, RED, bold, dim, fg};
use cockpit_core::{header, sources};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::{Value, json};

use crate::facts::{self, Config, OLD_REVIEW_DAYS, PREVIEW_WARN_HOURS, RED_ALERT_HOURS, age_days, ago, first_name, flag, list, reference, s};
use crate::icons::icon;
use crate::inputs::Inputs;
use crate::row::{Cells, Kind, Row, RowAct, call, open};
use crate::sessions::Local;
use crate::work;

/// Team requests the compact layout shows before folding the rest away.
const TEAM_SHOWN: usize = 5;

/// Which review requests the Review section lists: every one, or only those asked of you, asked of your team, or made by
/// bots. The heading's counts always cover all of them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    #[default]
    All,
    Me,
    Team,
    Bots,
}

impl Scope {
    /// The one after this, all of them again after the last.
    pub fn next(self) -> Self {
        match self {
            Scope::All => Scope::Me,
            Scope::Me => Scope::Team,
            Scope::Team => Scope::Bots,
            Scope::Bots => Scope::All,
        }
    }

    /// How the heading and the toasts say it.
    pub fn word(self) -> &'static str {
        match self {
            Scope::All => "all",
            Scope::Me => "yours",
            Scope::Team => "team",
            Scope::Bots => "bots",
        }
    }
}

/// How a rebuild lays the rows out, beside what it reads.
pub struct Ctx<'a> {
    pub config: &'a Config,
    pub by_status: bool,
    /// The review requests listed, set with `f` in the inbox.
    pub review: Scope,
    pub compact: bool,
    /// Each ticket's cost history, oldest first, for its sparkline.
    pub series: &'a HashMap<String, Vec<f64>>,
}

pub const INBOX_VERSION: i64 = 1;

/// What the inbox says of a cache another version of cockpit wrote, which this one does not read.
pub const OTHER_VERSION: &str = "Written by another version of cockpit · this one reads it once its own hub writes it.";

pub fn heading(name: &str, title: &str, extra: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{}  ", icon(name)), fg(ORANGE)),
        Span::styled(title.to_string(), bold(ORANGE)),
        Span::styled(format!("   {extra}"), dim()),
    ])
}

pub fn strong() -> Style {
    Style::new().add_modifier(Modifier::BOLD)
}

/// The inbox cache when it is there and of this version.
pub fn usable(inputs: &Inputs) -> Option<&Value> {
    inputs.data.as_ref().filter(|d| d["version"].as_i64() == Some(INBOX_VERSION))
}

/// The header with its error lines, and the tree's top rows.
pub fn model(inputs: &Inputs, ctx: &Ctx, local: &mut Local) -> (Vec<Line<'static>>, Vec<Row>) {
    let Some(data) = &inputs.data else {
        // Nothing wired waits for ever, which reads the same as waiting a minute unless the line says which.
        let waiting = sources::empty_line(
            &[sources::Source::Github, sources::Source::Tickets, sources::Source::Ci],
            "No data yet",
            "the hub fills this within a minute.",
            "nothing is connected · cockpit setup wires your PRs, tickets and CI.",
        );
        return (vec![Line::styled(waiting, dim())], Vec::new());
    };
    if usable(inputs).is_none() {
        return (vec![Line::styled(OTHER_VERSION, dim())], Vec::new());
    }
    let ci = if inputs.ci.is_object() { inputs.ci.clone() } else { json!({"runs": []}) };
    let mut head = vec![heading("inbox", "Inbox", &format!("updated {}", facts::clock(&data["at"])))];
    let told = header::sources(&[data, &ci]);
    // The hub's error lines a chip tells; a source no chip shows (the config dropped it) keeps its line.
    let hubs: Vec<Value> = list(&data["hub"], "errors").iter().filter(|e| e.as_str().is_some_and(|line| header::covers(&told, line))).cloned().collect();
    let ci_error = if header::has(&told, "ci") { &Value::Null } else { &ci["error"] };
    let errors = list(data, "errors").iter().filter(|e| !hubs.contains(e)).chain([ci_error]).filter(|e| e.is_string());
    head.extend(errors.map(|e| Line::styled(e.as_str().unwrap_or_default().to_string(), fg(RED))));

    let mut top = Vec::new();
    if let Some(today) = today(inputs.brief.as_ref(), inputs.routines.as_ref()) {
        top.push(today);
    }
    let mut alerts = alerts(data, &ci, ctx.config, local);
    for row in start_rows(&inputs.starts) {
        alerts.add(row);
    }
    if !alerts.children.is_empty() {
        top.push(alerts);
    }
    top.push(review(data, &inputs.muted, ctx.compact, ctx.review));
    let (work, other) = work::rows(data, &ci, ctx, &inputs.costs, &inputs.secs);
    top.push(work);
    top.push(other);
    let merged: HashSet<(String, String)> = list(data, "merged").iter().map(|m| (s(m, "repo").into(), s(m, "branch").into())).collect();
    let cleanup = cleanup(&inputs.worktrees, &merged, list(data, "mine"), local);
    if !cleanup.children.is_empty() {
        top.push(cleanup);
    }
    (head, top)
}

/// The morning brief when it is today's and not dismissed, then the routine runs that need a look.
fn today(brief: Option<&Value>, routines: Option<&Value>) -> Option<Row> {
    let brief = brief.filter(|b| s(b, "date") == facts::today() && !flag(b, "dismissed") && !list(b, "lines").is_empty());
    let look = routines.map(cockpit_core::routines::needing_look).unwrap_or_default();
    let mut today = match brief {
        Some(brief) => Row::text("today", heading("today", "Today", &format!("morning brief, {}", facts::clock(&brief["at"]))))
            .acts(vec![RowAct::new("dismiss", 'x', call("dismiss_brief", vec![])).main()]),
        None if look.is_empty() => return None,
        None if look.len() == 1 => Row::text("today", heading("today", "Today", "1 routine needs a look")),
        None => Row::text("today", heading("today", "Today", &format!("{} routines need a look", look.len()))),
    };
    for (n, entry) in brief.map(|b| list(b, "lines")).unwrap_or_default().iter().enumerate() {
        let style = match s(entry, "tone") {
            "bad" => fg(RED),
            "warn" => fg(ORANGE),
            "good" => fg(GREEN),
            _ => Style::new(),
        };
        let glyph_style = if style == Style::new() { dim() } else { style };
        let text = s(entry, "text").to_string();
        let line = Line::from(vec![Span::styled(format!("{}  ", icon(s(entry, "icon"))), glyph_style), Span::styled(text.clone(), style)]);
        let mut row = Row::text(format!("brief:{n}"), line).info(vec![Line::styled(text.clone(), style)]).name(text).section("Today").kind(Kind::Brief);
        if !s(entry, "url").is_empty() {
            row = row.acts(vec![RowAct::new("open", 'o', open(s(entry, "url"))).main()]).primary(open(s(entry, "url")));
        }
        today.add(row);
    }
    for row in look.iter().map(routine_row) {
        today.add(row);
    }
    Some(today)
}

/// A routine run that needs a look, or a routine whose file is broken: open takes it into the routines space, dismiss
/// marks the run read.
fn routine_row(item: &Value) -> Row {
    let (glyph, style) = match s(item, "word") {
        "missed" => ("\u{f05e}", fg(ORANGE)),
        "broken" => ("\u{f071}", fg(RED)),
        _ => ("\u{f00d}", fg(RED)),
    };
    let at = if item["at"].as_u64().unwrap_or(0) > 0 { format!(" {}", facts::day_clock(&item["at"])) } else { String::new() };
    let word = if s(item, "word") == "broken" { "cannot run" } else { s(item, "word") };
    // A sentence that starts with how it went ("Missed Sun 09:00: …") says it once.
    let says_it = s(item, "reason").to_lowercase().starts_with(word);
    let text =
        if says_it { format!("{} · {}", s(item, "name"), s(item, "reason")) } else { format!("{} · {word}{at} · {}", s(item, "name"), s(item, "reason")) };
    let cockpit = std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| "cockpit".into());
    let go = call("run", vec![json!([cockpit, "focus", format!("routine:{}", s(item, "key"))])]);
    let mut info = vec![Line::styled(s(item, "reason").to_string(), style)];
    if s(item, "summary") != s(item, "reason") {
        info.push(Line::raw(s(item, "summary").to_string()));
    }
    Row::text(format!("routine:{}", s(item, "key")), Line::from(vec![Span::styled(format!("{glyph}  "), style), Span::raw(text.clone())]))
        .info(info)
        .name(text)
        .section("Today")
        .kind(Kind::Alert)
        .acts(if item["id"].is_string() {
            vec![
                RowAct::new("open", 'o', go.clone()).main(),
                RowAct::new("dismiss", 'x', call("routines:read", vec![item["slug"].clone(), item["id"].clone()])),
            ]
        } else {
            vec![RowAct::new("open", 'o', go.clone()).main()]
        })
        .primary(go)
}

/// Start work's runs that still run or failed: open one in the form, retry it, cancel it (asked first, since that
/// removes what it made) or dismiss it.
fn start_rows(starts: &[Value]) -> Vec<Row> {
    let mut rows = Vec::new();
    for run in starts.iter().filter(|r| matches!(s(r, "state"), "running" | "failed")) {
        let id = s(run, "id");
        let title = if s(run, "title").is_empty() { s(run, "label") } else { s(run, "title") };
        let failed = s(run, "state") == "failed";
        let (words, style) = if failed {
            let step = s(&run["error"], "step");
            (format!("start {title}: {} failed", if step.is_empty() { "a step" } else { step }), fg(RED))
        } else {
            (format!("start {title}: running"), fg(ORANGE))
        };
        let open = call("start_open", vec![json!(id)]);
        let mut acts = vec![RowAct::new("open", 'o', open.clone()).main()];
        if failed {
            acts.push(RowAct::new("rerun", 'r', call("start_retry", vec![json!(id)])));
        }
        if flag(run, "cancel") {
            acts.push(RowAct::new("cancel", 'c', call("start_cancel", vec![json!(id)])).confirm(format!("cancel-start-{id}")));
        }
        if failed {
            acts.push(RowAct::new("dismiss", 'x', call("start_dismiss", vec![json!(id)])));
        }
        rows.push(alert(format!("start:{id}"), &words, style).acts(acts).primary(open));
    }
    rows
}

fn alert(key: String, text: &str, style: Style) -> Row {
    Row::text(key, Line::styled(format!(" {}  {text}", icon("alert")), style)).name(text).section("Alerts").kind(Kind::Alert)
}

/// Previews expiring, unhealthy or running a merged branch, red CI of the last hours and sessions left on merged
/// branches.
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
            let extend = RowAct::new("extend 2 days", '+', call("extend", vec![json!(preview)])).main().confirm(format!("extend-{preview}"));
            alerts.add(with_url(row, if config.claude() { vec![extend] } else { vec![] }));
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
            let stop = RowAct::new("stop", 'S', call("stop_preview", vec![json!(id)])).main().confirm(format!("stop-{id}"));
            let mut acts = if config.claude() { vec![stop] } else { vec![] };
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
        let verb = if tab.is_some() {
            "close tab"
        } else if is_worktree {
            "remove worktree"
        } else {
            "close space"
        };
        let herdr = std::env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".into());
        let go = match tab {
            Some(tab) => json!([herdr, "tab", "focus", tab]),
            None => json!([herdr, "workspace", "focus", space]),
        };
        let text = format!("a session still works on {label}");
        let row = Row::text(format!("alert:session:{space}"), Line::styled(format!(" {}  {text}", icon("alert")), dim()))
            .name(format!("session on {label}"))
            .section("Alerts")
            .kind(Kind::Alert);
        // Removing a worktree is on the one key every removal is on.
        let key = if is_worktree && tab.is_none() { 'D' } else { 'c' };
        let close = RowAct::new(verb, key, call("close_space", vec![json!(space), json!(label), json!(is_worktree)])).main().confirm(format!("close-{space}"));
        alerts.add(row.acts(vec![close, RowAct::new("go", 'o', call("run", vec![go]))]));
    }
    if !alerts.children.is_empty()
        && let crate::row::Body::Text(line) = &mut alerts.body
    {
        line.spans.push(Span::styled(alerts.children.len().to_string(), dim()));
    }
    alerts
}

/// The worktrees to clean up: those of merged branches with no open PR, and those holding work, each saying what keeps
/// it (`cockpit_core::cleanup`'s words). One a live session works on is the Alerts' to say; removing is refused while
/// anything keeps it.
fn cleanup(worktrees: &[Value], merged: &HashSet<(String, String)>, mine: &[Value], local: &mut Local) -> Row {
    let mut section = Row::text("cleanup", heading("worktree", "To clean up", ""));
    let open: HashSet<(String, String)> = mine.iter().map(|pr| (s(pr, "repo").into(), s(pr, "branch").into())).collect();
    let live: HashSet<(String, String)> = local.agents().into_iter().map(|(repo, branch, _)| (repo, branch)).collect();
    for tree in worktrees {
        let key = (s(tree, "repo").to_string(), s(tree, "branch").to_string());
        let is_merged = (merged.contains(&key) || flag(tree, "merged")) && !open.contains(&key);
        if live.contains(&key) || !(is_merged || tree["changes"].as_u64().unwrap_or(0) > 0) {
            continue;
        }
        let path = s(tree, "path");
        let blockers = list(tree, "blockers").to_vec();
        let words: Vec<String> = blockers.iter().map(|b| s(b, "words").to_string()).collect();
        let place = format!("{} {}", key.0, key.1);
        let (text, style) = if words.is_empty() {
            (format!("{place}: merged, nothing keeps it"), dim())
        } else {
            (format!("{place}: kept by {}", words.join(", ")), fg(ORANGE))
        };
        let remove = RowAct::new("remove worktree", 'D', call("remove_worktree", vec![json!(path)])).confirm(format!("remove-{path}"));
        let go = RowAct::new("go", 'o', call("open_worktree", vec![json!(path)]));
        let acts = if words.is_empty() { vec![remove.main(), go] } else { vec![go.main(), remove] };
        let mut info = vec![Line::styled(text.clone(), style), Line::styled(path.to_string(), dim())];
        if words.is_empty() {
            info.push(Line::styled("Nothing keeps it: removing it deletes its folder; its branch too once git counts it merged.", dim()));
        } else {
            info.push(Line::styled("Kept by:", dim()));
            info.extend(words.iter().map(|w| Line::styled(format!("  · {w}"), fg(ORANGE))));
        }
        let item = tree.clone();
        let line = Line::styled(format!(" {}  {text}", icon("worktree")), style);
        section.add(Row::text(format!("worktree:{path}"), line).name(place).info(info).acts(acts).section("To clean up").kind(Kind::Worktree).item(&item));
    }
    if !section.children.is_empty()
        && let crate::row::Body::Text(line) = &mut section.body
    {
        line.spans.push(Span::styled(section.children.len().to_string(), dim()));
    }
    section
}

fn review_row(pr: &Value) -> Row {
    let is_direct = flag(pr, "isDirect");
    let mut acts = vec![RowAct::new("review", 'r', call("review", vec![pr.clone()])).main(), RowAct::new("open", 'o', open(s(pr, "url")))];
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
        .kind(Kind::Review)
        .item(pr)
}

/// Review requests: direct ones first, then team ones; old ones, bots' and muted repos folded away, and in the compact
/// layout the team requests past the first few. With a scope, only that kind is listed (the bots' unfolded), and the
/// heading says so.
fn review(data: &Value, muted: &[String], compact: bool, scope: Scope) -> Row {
    let is_muted = |repo: &str| muted.iter().any(|m| m == repo);
    let wants = |kind: Scope| scope == Scope::All || scope == kind;
    let is_old = |pr: &&Value| age_days(s(pr, "updatedAt")).unwrap_or(0.0) > OLD_REVIEW_DAYS;
    let shown: Vec<&Value> = list(data, "team").iter().filter(|pr| !is_muted(s(pr, "repo"))).collect();
    let direct: Vec<&Value> = list(data, "direct").iter().filter(|pr| !is_old(pr)).collect();
    let team: Vec<&Value> = shown.iter().copied().filter(|pr| !is_old(pr)).collect();
    let old: Vec<&Value> =
        list(data, "direct").iter().filter(|_| wants(Scope::Me)).chain(shown.iter().copied().filter(|_| wants(Scope::Team))).filter(is_old).collect();
    let bot_repos = list(data, "botRepos");
    // The hub keeps the bots' PRs (`botPrs`) to list them; another source only says which repos they are in.
    let bot_prs: Vec<&Value> = list(data, "botPrs").iter().filter(|pr| !is_muted(s(pr, "repo"))).collect();
    let bots = if list(data, "botPrs").is_empty() { bot_repos.iter().filter(|r| !is_muted(r.as_str().unwrap_or(""))).count() } else { bot_prs.len() };
    let hidden = (list(data, "team").len() + bot_repos.len()).saturating_sub(shown.len() + bots);
    // The counts always cover every request; the kind `f` picked is bold, orange and underlined (the underline stays when
    // a terminal shows no colour), the others dim.
    let count = |kind: Scope, text: String| Span::styled(text, if scope == kind { bold(ORANGE).add_modifier(Modifier::UNDERLINED) } else { dim() });
    let dot = || Span::styled(" · ", dim());
    let mut spans = vec![
        Span::styled(format!("{}  ", icon("review")), fg(ORANGE)),
        Span::styled("Review", bold(ORANGE)),
        Span::styled("   ", dim()),
        count(Scope::Me, format!("{} for you", direct.len())),
        dot(),
        count(Scope::Team, format!("{} team", team.len())),
        dot(),
        count(Scope::Bots, format!("{bots} bots")),
    ];
    if hidden > 0 {
        spans.push(Span::styled(format!(" · {hidden} muted"), dim()));
    }
    let mut reviews = Row::text("review", Line::from(spans));
    let cap = if compact { TEAM_SHOWN } else { usize::MAX };
    for pr in direct.iter().filter(|_| wants(Scope::Me)).chain(team.iter().take(cap).filter(|_| wants(Scope::Team))) {
        reviews.add(review_row(pr));
    }
    if wants(Scope::Team) && team.len() > cap {
        let more = Row::text("review:more", Line::styled(format!("{} {} more team requests", icon("fold"), team.len() - cap), dim())).folded();
        let group = reviews.add(more);
        for pr in &team[cap..] {
            group.add(review_row(pr));
        }
    }
    if !old.is_empty() {
        let group = reviews.add(Row::text("review:old", Line::styled(format!("{} older than {OLD_REVIEW_DAYS} days", old.len()), dim())).folded());
        for pr in &old {
            group.add(review_row(pr));
        }
    }
    if scope == Scope::Bots {
        for pr in &bot_prs {
            reviews.add(review_row(pr));
        }
    } else if scope == Scope::All && !bot_prs.is_empty() {
        let group = reviews.add(Row::text("review:bots", Line::styled(format!("{} {} requests by bots", icon("fold"), bot_prs.len()), dim())).folded());
        for pr in &bot_prs {
            group.add(review_row(pr));
        }
    }
    if scope == Scope::All && !muted.is_empty() {
        let group = reviews.add(Row::text("review:muted", Line::styled(format!("{} {} muted repos", icon("mute"), muted.len()), dim())).folded());
        let mut repos = muted.to_vec();
        repos.sort();
        for repo in repos {
            let unmute = RowAct::new("unmute", 'm', call("set_muted", vec![json!(repo), json!(false)])).main();
            group.add(Row::text(format!("muted:{repo}"), Line::styled(repo.clone(), fg(DIM))).acts(vec![unmute]).name(repo).section("Muted").kind(Kind::Muted));
        }
    }
    reviews
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn today_lists_the_routine_runs_that_need_a_look_each_opening_its_run() {
        let routines = json!({"version": 1, "inboxFrom": 0, "routines": [{"slug": "sweep", "name": "Morning sweep"}, {"slug": "x", "name": "X", "error": "when?"}],
            "runs": {"sweep": [{"id": "7", "ok": false, "ended": 5, "summary": "Did not start: the folder /x does not exist.", "reason": "Did not start: the folder /x does not exist."}]}});
        let section = today(None, Some(&routines)).expect("a Today section");
        let keys: Vec<&str> = section.children.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(keys, ["routine:sweep/run/7", "routine:x"]);
        let call = section.children[0].acts[0].call.as_ref().expect("open runs a command");
        let argv = call.args[0].as_array().expect("an argv");
        assert_eq!((call.function, &argv[1], &argv[2]), ("run", &json!("focus"), &json!("routine:sweep/run/7")));
        assert!(section.children[0].name.contains("Morning sweep · failed"), "{}", section.children[0].name);
        let dismiss = section.children[0].acts[1].call.as_ref().expect("dismiss marks it read");
        assert_eq!((dismiss.function, &dismiss.args), ("routines:read", &vec![json!("sweep"), json!("7")]));
        assert_eq!(section.children[1].acts.len(), 1, "a routine that cannot run has no run to mark read");
        assert!(today(None, Some(&json!({"version": 1, "routines": [], "runs": {}}))).is_none(), "nothing to look at: no Today");
    }

    fn pr(repo: &str, number: u64, author: &str) -> Value {
        json!({"repo": repo, "number": number, "title": "chore(deps): update", "url": format!("https://github.com/acme/{repo}/pull/{number}"),
            "author": author, "authorName": "", "updatedAt": "2026-11-26T10:00:00Z", "isDraft": false, "isDirect": false})
    }

    #[test]
    fn the_requests_of_bots_fold_under_review_where_they_can_be_opened() {
        let data = json!({"direct": [], "team": [pr("api", 1, "maya")], "botRepos": ["web", "web", "ui"],
            "botPrs": [pr("web", 7, "app-bot"), pr("web", 8, "app-bot"), pr("ui", 9, "app-bot")]});
        let section = review(&data, &[], false, Scope::All);
        let keys: Vec<&str> = section.children.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(keys, ["https://github.com/acme/api/pull/1", "review:bots"], "a person's request stays a row of its own");
        let group = &section.children[1];
        assert!(!group.expand, "the bots start folded");
        let listed: Vec<&str> = group.children.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(listed, ["https://github.com/acme/web/pull/7", "https://github.com/acme/web/pull/8", "https://github.com/acme/ui/pull/9"]);
        let muted = review(&data, &["web".to_string()], false, Scope::All);
        let group = muted.children.iter().find(|r| r.key == "review:bots").expect("the other repo's bot is still listed");
        assert_eq!(group.children.len(), 1, "a muted repo's bots are left out");
    }

    #[test]
    fn without_the_bots_prs_they_are_only_counted() {
        let data = json!({"direct": [], "team": [pr("api", 1, "maya")], "botRepos": ["web", "web"]});
        let section = review(&data, &[], false, Scope::All);
        assert!(section.children.iter().all(|r| r.key != "review:bots"), "a source that gives repos only has nothing to list");
    }

    fn keys_of(section: &Row) -> Vec<String> {
        section.children.iter().map(|r| r.key.rsplit('/').next().unwrap_or_default().to_string()).collect()
    }

    fn mixed() -> Value {
        let mut direct = pr("api", 1, "maya");
        direct["isDirect"] = json!(true);
        json!({"direct": [direct], "team": [pr("api", 2, "lin"), pr("web", 3, "lin")], "botRepos": ["web", "ui"],
            "botPrs": [pr("web", 7, "app-bot"), pr("ui", 9, "app-bot")]})
    }

    #[test]
    fn a_review_scope_lists_only_that_kind_and_the_heading_still_counts_them_all() {
        let data = mixed();
        assert_eq!(keys_of(&review(&data, &[], false, Scope::Me)), ["1"], "asked of you");
        assert_eq!(keys_of(&review(&data, &[], false, Scope::Team)), ["2", "3"], "asked of your team");
        let bots = review(&data, &[], false, Scope::Bots);
        assert_eq!(keys_of(&bots), ["7", "9"], "the bots' requests, unfolded, with no fold row to open first");
        let all = review(&data, &[], false, Scope::All);
        assert_eq!(keys_of(&all), ["1", "2", "3", "review:bots"]);
        // The heading's counts cover every request, and only the kind shown is highlighted.
        let lit = |row: &Row| -> (String, Vec<String>) {
            let crate::row::Body::Text(line) = &row.body else { return Default::default() };
            let text = line.spans.iter().map(|s| s.content.as_ref()).collect::<String>();
            let marked = line.spans.iter().filter(|s| s.style.add_modifier.contains(Modifier::UNDERLINED)).map(|s| s.content.to_string()).collect();
            (text, marked)
        };
        let (text, marked) = lit(&bots);
        assert!(text.contains("1 for you · 2 team · 2 bots"), "{text}");
        assert_eq!(marked, ["2 bots"]);
        assert_eq!(lit(&review(&data, &[], false, Scope::Me)).1, ["1 for you"]);
        assert_eq!(lit(&review(&data, &[], false, Scope::Team)).1, ["2 team"]);
        assert!(lit(&all).1.is_empty(), "all of them highlights nothing");
    }

    #[test]
    fn a_review_scope_goes_round_and_leaves_the_muted_repos_to_all() {
        let seen: Vec<Scope> = std::iter::successors(Some(Scope::All), |s| Some(s.next())).take(5).collect();
        assert_eq!(seen, [Scope::All, Scope::Me, Scope::Team, Scope::Bots, Scope::All]);
        let data = mixed();
        let muted = ["web".to_string()];
        assert!(review(&data, &muted, false, Scope::All).children.iter().any(|r| r.key == "review:muted"));
        assert!(review(&data, &muted, false, Scope::Team).children.iter().all(|r| r.key != "review:muted"), "the mutes are managed from all");
        assert_eq!(keys_of(&review(&data, &muted, false, Scope::Team)), ["2"], "a muted repo's request stays out of a scope too");
    }
}
