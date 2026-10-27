//! The Work and Other PRs sections: tickets by feature or by status, each with its sub-tickets, deploys and PRs, a PR
//! based on another PR's branch hanging under it; then the PRs no assigned ticket claims, grouped by what they need.

use std::collections::{HashMap, HashSet};

use cockpit_core::theme::{DIM, GREEN, ORANGE, RED, dim, fg};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use serde_json::{Value, json};

use crate::facts::{self, Config, STALE_PR_DAYS, age_days, ago, ci_glyph, dashboard_name, flag, has_ticket, is_deploying, is_failing, list, needs_me, reference, review_glyph, s, text};
use crate::icons::icon;
use crate::model::{heading, strong};
use crate::row::{Cells, Row, RowAct, call, open};
use crate::sessions::ticket_costs;

type Key = (String, String);

fn pr_key(pr: &Value) -> Key {
    (s(pr, "repo").into(), s(pr, "branch").into())
}

const STEPS: [&str; 5] = ["progress", "review", "merge", "test", "deploy"];
const LABELS: [&str; 9] = ["in progress", "in review", "to merge", "to test", "to deploy", "todo", "backlog", "triage", "other"];

/// In progress, review, merge, test, deploy, then todo, backlog, triage.
fn status_order(ticket: &Value) -> usize {
    let name = s(ticket, "status").to_lowercase();
    match s(ticket, "statusType") {
        "started" => STEPS.iter().position(|step| name.contains(step)).unwrap_or(0),
        "unstarted" => 5,
        "backlog" => 6,
        "triage" => 7,
        _ => 8,
    }
}

struct Build<'a> {
    data: &'a Value,
    config: &'a Config,
    mine: &'a [Value],
    runs: HashMap<Key, &'a Value>,
    pr_children: HashMap<Key, Vec<&'a Value>>,
    pr_parent: HashMap<Key, Key>,
    t_children: HashMap<String, Vec<&'a Value>>,
    /// Each ticket's PRs, matched once: sorting and grouping ask again and again.
    linked: HashMap<String, Vec<&'a Value>>,
    costs: HashMap<String, f64>,
}

impl<'a> Build<'a> {
    fn new(data: &'a Value, ci: &'a Value, config: &'a Config) -> Self {
        let mine = list(data, "mine");
        let by_key: HashSet<Key> = mine.iter().map(pr_key).collect();
        let mut pr_children: HashMap<Key, Vec<&Value>> = HashMap::new();
        for pr in mine {
            let parent = (s(pr, "repo").to_string(), s(pr, "base").to_string());
            if by_key.contains(&parent) && parent != pr_key(pr) {
                pr_children.entry(parent).or_default().push(pr);
            }
        }
        let pr_parent = pr_children.iter().flat_map(|(parent, kids)| kids.iter().map(|kid| (pr_key(kid), parent.clone()))).collect();
        let tickets = list(data, "tickets");
        let ids: HashSet<&str> = tickets.iter().map(|t| s(t, "id")).collect();
        let mut t_children: HashMap<String, Vec<&Value>> = HashMap::new();
        for t in tickets {
            if ids.contains(s(t, "parentId")) {
                t_children.entry(s(t, "parentId").to_string()).or_default().push(t);
            }
        }
        let runs = list(ci, "runs").iter().map(|r| (pr_key(r), r)).collect();
        let linked = tickets.iter().map(|t| (s(t, "id").to_string(), mine.iter().filter(|pr| has_ticket(s(pr, "branch"), s(t, "id"))).collect())).collect();
        Self { data, config, mine, runs, pr_children, pr_parent, t_children, linked, costs: ticket_costs() }
    }

    fn kids(&self, pr: &Value) -> &[&'a Value] {
        self.pr_children.get(&pr_key(pr)).map(Vec::as_slice).unwrap_or(&[])
    }

    /// The PR and its stack, depth first; a base cycle stops where it comes back.
    fn pr_subtree(&self, pr: &'a Value) -> Vec<&'a Value> {
        let (mut out, mut stack, mut seen) = (Vec::new(), vec![pr], HashSet::new());
        while let Some(pr) = stack.pop() {
            if seen.insert(pr_key(pr)) {
                out.push(pr);
                stack.extend(self.kids(pr).iter().rev());
            }
        }
        out
    }

    /// A PR with no parent PR, or the first of a base cycle, which would otherwise have no top and show nowhere.
    fn is_top(&self, pr: &Value) -> bool {
        let key = pr_key(pr);
        let mut chain = vec![key.clone()];
        while let Some(parent) = self.pr_parent.get(chain.last().expect("never empty")) {
            if *parent == key {
                return chain.iter().all(|k| *k >= key);
            }
            if chain.contains(parent) {
                return false;
            }
            chain.push(parent.clone());
        }
        chain.len() == 1
    }

    fn previews(&self) -> &'a [Value] {
        list(self.data, "previews")
    }

    fn links(&self, repo: &str) -> &'a Value {
        &self.data["deployLinks"][repo]
    }

    /// The PR's row with its stack under it; `above` holds the PRs over it, so a base cycle stops where it comes back.
    fn pr_row(&self, pr: &'a Value, section: &str, above: &[Key]) -> Row {
        let run = self.runs.get(&pr_key(pr)).copied();
        let bx = self.previews().iter().find(|b| s(b, "app") == s(pr, "repo") && s(b, "branch") == s(pr, "branch"));
        let image_ready = run.is_some_and(|r| s(r, "dockerTag") == "succeeded");
        let running = run.filter(|r| flag(r, "isRunning"));
        let mut notes = Vec::new();
        if let Some(r) = running {
            notes.push(format!("{} {}/{}", icon("ci"), text(r, "done"), text(r, "total")));
        }
        if image_ready {
            notes.push(format!("{} image", icon("docker")));
        }
        if bx.is_some() {
            notes.push(format!("{} preview", icon("preview")));
        }
        let (ci_g, ci_s) = ci_glyph(pr);
        let (rv_g, rv_s) = review_glyph(pr);
        let mut acts = Vec::new();
        let run_arg = run.cloned().unwrap_or(Value::Null);
        if is_failing(pr) {
            let here = call("fix_ci", vec![pr.clone(), run_arg.clone(), json!("here")]);
            let new = call("fix_ci", vec![pr.clone(), run_arg.clone(), json!("new")]);
            acts.push(RowAct::pick("fix ci", 'a', vec![("on this PR's branch", here), ("on a new PR off main", new)]).main());
        }
        if let Some(r) = run
            && s(r, "outcome") == "failed"
            && !s(r, "workflowId").is_empty()
            && running.is_none()
        {
            acts.push(RowAct::new("rerun", 'u', call("rerun", vec![pr.clone(), r.clone()])).confirm(format!("rerun-{}", s(r, "workflowId"))));
        }
        if let Some(bx) = bx
            && image_ready
        {
            let redeploy = call("redeploy", vec![bx.clone(), json!(s(pr, "url"))]);
            acts.push(RowAct::new("redeploy", 'd', redeploy).main_if(acts.is_empty()).confirm(format!("redeploy-{}-{}", s(bx, "id"), s(bx, "app"))));
        }
        if bx.is_none() && image_ready && self.config.can_preview() {
            let preview = call("create_preview", vec![pr.clone()]);
            acts.push(RowAct::new("preview", 's', preview).main_if(acts.is_empty()).confirm(format!("preview-{}-{}", s(pr, "repo"), s(pr, "branch"))));
        }
        let links = self.links(s(pr, "repo"));
        let dashboard = bx.and_then(|b| self.config.dashboard_url(s(b, "id"))).or_else(|| links["dashboard"].as_str().map(str::to_string));
        if let Some(dashboard) = dashboard {
            acts.push(RowAct::new(dashboard_name(&dashboard), 'A', open(&dashboard)));
        }
        if let Some(url) = links["workflow"]["url"].as_str() {
            acts.push(RowAct::new("deploy", 'W', open(url)));
        }
        acts.push(RowAct::new("open", 'o', open(s(pr, "url"))).main_if(acts.is_empty()));
        let review = if s(pr, "review").is_empty() { "no review".to_string() } else { s(pr, "review").to_lowercase().replace('_', " ") };
        let ci_state = match running {
            Some(r) => format!("running {}/{}", text(r, "done"), text(r, "total")),
            None if s(pr, "ci").is_empty() => "none".into(),
            None => s(pr, "ci").to_lowercase(),
        };
        let draft = if flag(pr, "isDraft") { " · draft" } else { "" };
        let preview = bx.map(|b| format!("   preview {} · {}", s(b, "id"), s(b, "health").to_lowercase())).unwrap_or_default();
        let mut info = vec![
            Line::from(vec![Span::styled(format!("{}  ", reference(pr)), dim()), Span::styled(s(pr, "title").to_string(), strong())]),
            Line::styled(format!("{} → {}{draft} · updated {} ago", s(pr, "branch"), s(pr, "base"), ago(s(pr, "updatedAt"))), dim()),
            Line::from(vec![Span::styled("review ", dim()), Span::styled(review, rv_s), Span::styled("   ci ", dim()), Span::styled(ci_state, ci_s), Span::styled(preview, dim())]),
        ];
        let mut kids = Vec::new();
        if let Some(r) = run.filter(|r| r["failed"].is_object() && !flag(r, "isRunning")) {
            let failed = &r["failed"];
            let tests = list(failed, "tests");
            let why = if tests.is_empty() { s(failed, "step").to_string() } else { format!("{} failing: {}", tests.len(), join(tests)) };
            let line = format!("{} {} · {why}", icon("ci"), s(failed, "job"));
            kids.push(Row::text(format!("{}:ci", s(pr, "url")), Line::styled(line.clone(), fg(RED))));
            info.push(Line::styled(line, fg(RED)));
        }
        let shown_job = run.map(|r| s(&r["failed"], "job")).unwrap_or("");
        let mirror = self.config.mirror();
        let checks: Vec<&str> = list(pr, "failedChecks")
            .iter()
            .filter_map(Value::as_str)
            .filter(|c| *c != shown_job && !mirror.as_ref().is_some_and(|m| c.to_lowercase().starts_with(&format!("ci/{m}")) || c.to_lowercase().starts_with(m.as_str())))
            .collect();
        if !checks.is_empty() {
            let line = format!("{} {}", icon("github"), checks.join(", "));
            kids.push(Row::text(format!("{}:checks", s(pr, "url")), Line::styled(line.clone(), fg(RED))));
            info.push(Line::styled(line, fg(RED)));
        }
        let cells = Cells {
            lead: vec![(ci_g.into(), ci_s), (" ".into(), Style::new()), (rv_g.into(), rv_s)],
            reference: (reference(pr), dim()),
            title: (s(pr, "title").into(), if flag(pr, "isDraft") { dim() } else { Style::new() }),
            notes: (notes.join("  "), fg(ORANGE)),
            age: ago(s(pr, "updatedAt")),
        };
        let mut row = Row::cells(s(pr, "url"), cells, format!("{} {}", reference(pr), s(pr, "title")))
            .acts(acts)
            .primary(call("details", vec![pr.clone()]))
            .info(info)
            .status(s(pr, "url"))
            .section(section);
        row.children = kids;
        let path: Vec<Key> = above.iter().cloned().chain([pr_key(pr)]).collect();
        for kid in self.kids(pr).iter().filter(|kid| !path.contains(&pr_key(kid))) {
            row.add(self.pr_row(kid, section, &path));
        }
        row
    }

    fn linked(&self, ticket: &Value) -> Vec<&'a Value> {
        self.linked.get(s(ticket, "id")).cloned().unwrap_or_default()
    }

    /// Linked PRs whose parent PR is not linked to the same ticket; their stacks follow them.
    fn linked_roots(&self, ticket: &Value) -> Vec<&'a Value> {
        let prs = self.linked(ticket);
        let keys: HashSet<Key> = prs.iter().map(|pr| pr_key(pr)).collect();
        prs.into_iter().filter(|pr| !self.pr_parent.get(&pr_key(pr)).is_some_and(|p| keys.contains(p))).collect()
    }

    fn tree_tickets(&self, ticket: &'a Value) -> Vec<&'a Value> {
        let mut out = vec![ticket];
        for kid in self.t_children.get(s(ticket, "id")).into_iter().flatten() {
            out.extend(self.tree_tickets(kid));
        }
        out
    }

    fn tree_needs_me(&self, ticket: &'a Value) -> bool {
        self.tree_tickets(ticket).iter().any(|t| self.linked_roots(t).into_iter().any(|root| self.pr_subtree(root).iter().any(|pr| needs_me(pr))))
    }

    /// A tree sorts by its most urgent ticket: one whose PR needs me, then the least advanced status.
    fn tree_order(&self, ticket: &'a Value) -> (u8, usize, String) {
        let least = self.tree_tickets(ticket).iter().map(|t| status_order(t)).min().unwrap_or(8);
        (if self.tree_needs_me(ticket) { 0 } else { 1 }, least, s(ticket, "id").to_string())
    }

    fn ticket_row(&self, ticket: &'a Value, section: &str) -> Row {
        let id = s(ticket, "id");
        let prs = self.linked(ticket);
        let usd = self.costs.get(id).copied().filter(|u| *u != 0.0);
        let deploying = is_deploying(ticket);
        let started = s(ticket, "statusType") == "started";
        let (glyph, style) = if deploying {
            (icon("rocket"), dim())
        } else if started {
            (icon("progress"), fg(ORANGE))
        } else {
            (icon("backlog"), dim())
        };
        let deployed = list(&self.data["deploys"], id);
        let mut acts = Vec::new();
        if deploying {
            if !deployed.is_empty() && deployed.iter().all(|d| flag(d, "isLive")) {
                acts.push(RowAct::new("done", 'a', call("mark_done", vec![ticket.clone()])).main().confirm(format!("done-{id}")));
            }
        } else if started {
            acts.push(RowAct::new("continue", 'a', call("continue_ticket", vec![ticket.clone(), json!(prs)])).main());
        } else {
            acts.push(RowAct::new("plan", 'a', call("plan", vec![ticket.clone()])).main());
        }
        if deploying && let Some(first) = deployed.first() {
            let links = self.links(s(first, "repo"));
            if let Some(dashboard) = links["dashboard"].as_str() {
                acts.push(RowAct::new(dashboard_name(dashboard), 'A', open(dashboard)));
            }
            if let Some(url) = links["workflow"]["url"].as_str() {
                acts.push(RowAct::new("deploy", 'W', open(url)));
            }
        }
        acts.push(RowAct::new("talk", 't', call("talk", vec![ticket.clone(), json!(prs), json!({"$inbox": true})])));
        acts.push(RowAct::new("open", 'o', open(s(ticket, "url"))));
        let project = if s(ticket, "project").is_empty() { "no project" } else { s(ticket, "project") };
        let spent = usd.map(|u| format!(" · ${u:.2} in Claude usage")).unwrap_or_default();
        let mut info = vec![
            Line::from(vec![Span::styled(format!("{id}  "), dim()), Span::styled(s(ticket, "title").to_string(), strong())]),
            Line::styled(format!("{} · {project} · {} PRs{spent}", s(ticket, "status"), prs.len()), dim()),
        ];
        let mut kids = Vec::new();
        for (n, done) in deployed.iter().enumerate() {
            let is_live = flag(done, "isLive");
            let live = if is_live { format!("{} live in production", icon("rocket")) } else { format!("{} not in production yet, prod runs {}", icon("waiting"), s(done, "tag")) };
            let line = format!("{}  {live}", reference(done));
            let style = if is_live { fg(GREEN) } else { fg(DIM) };
            kids.push(Row::text(format!("{id}:deploy:{n}"), Line::styled(line.clone(), style)));
            info.push(Line::styled(line, style));
        }
        let cells = Cells {
            lead: vec![(glyph.into(), style)],
            reference: (format!("{id:<10}{}", facts::short_status(ticket)), dim()),
            title: (s(ticket, "title").into(), Style::new()),
            notes: (usd.map(|u| format!("${u:.2}")).unwrap_or_default(), dim()),
            age: String::new(),
        };
        let mut row = Row::cells(id, cells, format!("{id} {}", s(ticket, "title")))
            .acts(acts)
            .primary(open(s(ticket, "url")))
            .info(info)
            .status(id)
            .section(section);
        row.children = kids;
        for kid in self.t_children.get(id).into_iter().flatten() {
            row.add(self.ticket_row(kid, section));
        }
        for root in self.linked_roots(ticket) {
            row.add(self.pr_row(root, section, &[]));
        }
        row
    }

    fn work(&self, by_status: bool, claimed: usize) -> Row {
        let tickets = list(self.data, "tickets");
        let ids: HashSet<&str> = tickets.iter().map(|t| s(t, "id")).collect();
        let roots: Vec<&Value> = tickets.iter().filter(|t| !ids.contains(s(t, "parentId"))).collect();
        let in_progress = tickets.iter().filter(|t| s(t, "statusType") == "started" && !is_deploying(t)).count();
        let mode = if by_status { "status" } else { "feature" };
        let extra = format!("{} tickets · {in_progress} in progress · {claimed} PRs · by {mode} (g)", tickets.len());
        let mut work = Row::text("work", heading("work", "Work", &extra));
        let order = |roots: &[&'a Value]| roots.iter().map(|r| self.tree_order(r)).min().unwrap_or((1, 8, String::new()));
        let mut groups: Vec<(String, Vec<&Value>)> = Vec::new();
        for root in roots {
            let name = if by_status {
                let least = self.tree_tickets(root).iter().map(|t| status_order(t)).min().unwrap_or(8);
                if self.tree_needs_me(root) { "needs you".to_string() } else { LABELS[least.min(LABELS.len() - 1)].to_string() }
            } else if s(root, "project").is_empty() {
                "No project".to_string()
            } else {
                s(root, "project").to_string()
            };
            match groups.iter_mut().find(|(n, _)| *n == name) {
                Some((_, members)) => members.push(root),
                None => groups.push((name, vec![root])),
            }
        }
        if by_status {
            groups.sort_by_key(|(_, roots)| order(roots));
        } else {
            groups.sort_by_key(|(name, roots)| (name == "No project", order(roots)));
        }
        for (name, mut roots) in groups {
            roots.sort_by_key(|r| self.tree_order(r));
            let group = if by_status {
                let style = if name == "needs you" { fg(RED) } else { strong() };
                Row::text(format!("status:{name}"), Line::styled(name.clone(), style)).expand(name != "to deploy")
            } else {
                let count: usize = roots.iter().map(|r| self.tree_tickets(r).len()).sum();
                let marker = if roots.iter().any(|r| self.tree_needs_me(r)) { "● " } else { "" };
                let line = Line::from(vec![Span::styled(marker, fg(RED)), Span::styled(name.clone(), strong()), Span::styled(format!("  {count}"), dim())]);
                Row::text(format!("project:{name}"), line)
            };
            let group = work.add(group);
            for root in roots {
                group.add(self.ticket_row(root, &name));
            }
        }
        work
    }

    fn claimed(&self) -> HashSet<Key> {
        let mut claimed = HashSet::new();
        for t in list(self.data, "tickets") {
            for root in self.linked_roots(t) {
                claimed.extend(self.pr_subtree(root).iter().map(|pr| pr_key(pr)));
            }
        }
        claimed
    }

    fn other(&self, claimed: &HashSet<Key>) -> Row {
        let others: Vec<&Value> = self.mine.iter().filter(|pr| !claimed.contains(&pr_key(pr)) && !self.pr_parent.get(&pr_key(pr)).is_some_and(|p| claimed.contains(p))).collect();
        let mut groups: [Vec<&Value>; 4] = Default::default();
        for root in others.iter().copied().filter(|pr| self.is_top(pr)) {
            let tree = self.pr_subtree(root);
            let rank = if tree.iter().all(|p| age_days(s(p, "updatedAt")).unwrap_or(0.0) > STALE_PR_DAYS) {
                3
            } else if tree.iter().any(|p| needs_me(p)) {
                0
            } else if tree.iter().all(|p| s(p, "review") == "APPROVED") {
                2
            } else {
                1
            };
            groups[rank].push(root);
        }
        let mut other = Row::text("other", heading("pr", "Other PRs", &format!("{} without an assigned ticket", others.len())));
        let stale = format!("stale, no update in {STALE_PR_DAYS} days");
        let labels = [("needs you", fg(RED), true), ("waiting for review", dim(), true), ("ready to merge", dim(), false), (stale.as_str(), dim(), false)];
        for (rank, (label, style, is_open)) in labels.into_iter().enumerate() {
            if groups[rank].is_empty() {
                continue;
            }
            let count: usize = groups[rank].iter().map(|r| self.pr_subtree(r).len()).sum();
            let line = Line::from(vec![Span::styled(label.to_string(), style), Span::styled(format!("  {count}"), dim())]);
            let group = other.add(Row::text(format!("other:{rank}"), line).expand(is_open));
            for root in &groups[rank] {
                group.add(self.pr_row(root, "Other PRs", &[]));
            }
        }
        other
    }
}

fn join(values: &[Value]) -> String {
    values.iter().map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())).collect::<Vec<_>>().join(", ")
}

/// The Work section and the Other PRs section.
pub fn rows(data: &Value, ci: &Value, config: &Config, by_status: bool) -> (Row, Row) {
    let build = Build::new(data, ci, config);
    let claimed = build.claimed();
    (build.work(by_status, claimed.len()), build.other(&claimed))
}
