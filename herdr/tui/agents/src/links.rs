//! What an agent works with, as links grouped by kind and named after their service (Linear, GitHub, CircleCI, Argo,
//! Slack…): its ticket, PR, CI and deploy dashboards, and the pages its conversation cites, read from its transcript's
//! tail whatever the agent. Its documents are the views it showed beside itself.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::SystemTime;

use cockpit_core::sites::{self, Link};
use cockpit_core::{files, transcripts};
use regex::Regex;
use serde_json::{Value, json};

/// How much of a transcript's end is read for the pages and tickets it cites.
const TAIL: u64 = 256 * 1024;
/// A view older than this is gone for good.
const KEEP_MS: f64 = 7.0 * 86_400_000.0;
/// The kinds of cited page that are pull requests.
const PR_KINDS: [&str; 4] = ["pr", "mr", "bitbucket", "azure-pr"];

type Stamp = (u64, Option<SystemTime>);

/// Each transcript's cited pages and ticket keys, by the stamp they were read at and the ticket pattern used.
type Cited = HashMap<PathBuf, (Stamp, Option<String>, Vec<Link>, Vec<String>)>;
/// Each view's brief (terminal, title, time, placement), by the modification time it was read at.
type Briefs = HashMap<String, (Option<SystemTime>, Value)>;

static CITED: LazyLock<Mutex<Cited>> = LazyLock::new(Mutex::default);
static VIEWS: LazyLock<Mutex<Briefs>> = LazyLock::new(Mutex::default);

fn stamp(path: &Path) -> Option<Stamp> {
    std::fs::metadata(path).ok().map(|m| (m.len(), m.modified().ok()))
}

/// The transcript's tail, read again only when the file changed or another ticket pattern is given: the pages it cites
/// and, with a pattern, the ticket keys it names.
fn read(transcript: &Path, pattern: Option<&Regex>) -> (Vec<Link>, Vec<String>) {
    let Some(now) = stamp(transcript) else { return (Vec::new(), Vec::new()) };
    let source = pattern.map(|p| p.as_str().to_string());
    let mut cache = CITED.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((at, with, links, tickets)) = cache.get(transcript)
        && *at == now
        && (source.is_none() || *with == source)
    {
        return (links.clone(), if source.is_none() { Vec::new() } else { tickets.clone() });
    }
    let text = transcripts::tail(transcript, TAIL).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    let links = sites::cited(&text);
    let tickets = pattern.map(|p| keys_in(&text, p)).unwrap_or_default();
    cache.insert(transcript.to_path_buf(), (now, source, links.clone(), tickets.clone()));
    (links, tickets)
}

/// The ticket keys a text names, uppercased, the last named first, each once.
fn keys_in(text: &str, pattern: &Regex) -> Vec<String> {
    let mut keys = Vec::new();
    for caps in pattern.captures_iter(text).collect::<Vec<_>>().into_iter().rev() {
        let key = format!("{}-{}", caps[1].to_uppercase(), &caps[2]);
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    keys
}

/// The pages the transcript's tail cites.
fn cited(transcript: &Path) -> Vec<Link> {
    read(transcript, None).0
}

/// The ticket keys the transcript's tail names, the last named first.
pub fn cited_tickets(transcript: &Path, pattern: Option<&Regex>) -> Vec<String> {
    read(transcript, pattern).1
}

/// The pull request URLs the transcript's tail cites, the last cited first.
pub fn cited_prs(transcript: &Path) -> Vec<String> {
    cited(transcript).into_iter().filter(|l| PR_KINDS.contains(&l.kind)).map(|l| l.url).collect()
}

fn text(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

/// A group of links of one kind and service, its items in order.
struct Group {
    kind: &'static str,
    name: String,
    icon: &'static str,
    items: Vec<Value>,
}

impl Group {
    fn new(kind: &'static str, name: &str, icon: &'static str) -> Group {
        Group { kind, name: name.to_string(), icon, items: Vec::new() }
    }

    /// Adds an item unless its URL is already in the group.
    fn push(&mut self, item: Value) {
        let url = text(&item["url"]).trim_end_matches('/').to_string();
        if !self.items.iter().any(|i| text(&i["url"]).trim_end_matches('/') == url) {
            self.items.push(item);
        }
    }

    fn json(&self, label: String) -> Value {
        json!({"kind": self.kind, "name": self.name, "icon": self.icon, "label": label, "items": self.items})
    }
}

fn item(detail: &str, url: &str) -> Value {
    json!({"detail": detail, "url": url})
}

/// A cited pull request with the title and state of the inbox's PR at its URL, when one is.
fn pr_item(c: &Link, inbox: &Value) -> Value {
    let url = c.url.trim_end_matches('/');
    let found = |list: &str| inbox[list].as_array().into_iter().flatten().find(|p| text(&p["url"]).trim_end_matches('/') == url).cloned();
    match (found("mine"), found("merged")) {
        (Some(p), _) => json!({"detail": c.detail, "url": c.url, "title": p["title"], "state": "open"}),
        (None, Some(p)) => json!({"detail": c.detail, "url": c.url, "title": p["title"], "state": "merged"}),
        (None, None) => item(&c.detail, &c.url),
    }
}

/// A cited ticket with the title and status of the inbox's ticket at its URL or key, when one is.
fn ticket_item(c: &Link, inbox: &Value) -> Value {
    let url = c.url.trim_end_matches('/');
    let found = inbox["tickets"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|t| text(&t["url"]).trim_end_matches('/') == url || (!c.detail.is_empty() && t["id"] == c.detail.as_str()));
    match found {
        Some(t) => json!({"detail": c.detail, "url": c.url, "title": t["title"], "state": t["status"]}),
        None => item(&c.detail, &c.url),
    }
}

/// Everything the row links to, grouped by kind and always in this order: its PRs, its tickets, CI, deploy dashboard,
/// deploy run, then the cited pages kind by kind in the sites table's order; a name two groups share carries its first
/// item's detail in its label.
#[allow(clippy::too_many_arguments)]
pub fn of(
    ticket: Option<&Value>,
    pr: &Value,
    ci: &Value,
    repo: &str,
    branch: Option<&str>,
    inbox: &Value,
    config: &Value,
    transcript: Option<&Path>,
) -> Vec<Value> {
    let mut groups: Vec<Group> = Vec::new();
    let pr_url = text(&pr["url"]);
    let pr_name = sites::host_name(pr_url).unwrap_or("PR");
    if !pr_url.is_empty() {
        let mut g = Group::new("pr", pr_name, "git-pull-request");
        let detail = [format!("#{}", pr["number"]), text(&pr["title"]).to_string()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
        g.push(json!({"detail": detail, "url": pr_url, "title": pr["title"], "state": pr["state"]}));
        groups.push(g);
    }
    let ticket = ticket.filter(|t| !text(&t["url"]).is_empty());
    let ticket_name = Some(text(&config["tickets"]["name"])).filter(|n| !n.is_empty()).or_else(|| sites::host_name(text(&ticket?["url"]))).unwrap_or("Ticket");
    if let Some(t) = ticket {
        let mut g = Group::new("ticket", ticket_name, "ticket");
        let detail = [text(&t["key"]), text(&t["title"])].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
        g.push(json!({"detail": detail, "url": t["url"], "title": t["title"], "state": t["status"]}));
        groups.push(g);
    }
    if let Some(state) = ci["state"].as_str().filter(|_| !pr_url.is_empty()) {
        let name = Some(text(&config["ci"]["name"])).filter(|n| !n.is_empty()).unwrap_or("CI");
        let mut g = Group::new("ci", name, "circle-dot");
        g.push(json!({"detail": ci["label"], "url": format!("{pr_url}/checks"), "state": state}));
        groups.push(g);
    }
    let preview = inbox["previews"].as_array().into_iter().flatten().find(|e| text(&e["app"]) == repo && branch.is_some_and(|b| text(&e["branch"]) == b));
    let preview_dashboard = preview.and_then(|e| {
        let template = text(&config["platform"]["previewDashboardUrl"]);
        (!template.is_empty()).then(|| (text(&e["id"]).to_string(), template.replace("{preview}", text(&e["id"]))))
    });
    let deploy = &inbox["deployLinks"][repo];
    match preview_dashboard {
        Some((id, url)) => {
            let mut g = Group::new("dashboard", sites::dashboard_name(&url), "rocket");
            g.push(item(&format!("preview {id}"), &url));
            groups.push(g);
        }
        None if !text(&deploy["dashboard"]).is_empty() => {
            let url = text(&deploy["dashboard"]);
            let mut g = Group::new("dashboard", sites::dashboard_name(url), "rocket");
            g.push(item("production", url));
            groups.push(g);
        }
        None => {}
    }
    if !text(&deploy["workflow"]["url"]).is_empty() {
        let url = text(&deploy["workflow"]["url"]);
        let mut g = Group::new("workflow", sites::dashboard_name(url), "workflow");
        g.push(item(&format!("deploy {}", text(&deploy["workflow"]["label"])), url));
        groups.push(g);
    }
    // The cited pages: a PR of the row's code host joins its PRs, a ticket of its tracker its tickets, the rest group by kind.
    let known: Vec<String> = groups.iter().flat_map(|g| g.items.iter().map(|i| text(&i["url"]).trim_end_matches('/').to_string())).collect();
    let ticket_key = ticket.map(|t| text(&t["key"])).unwrap_or_default();
    let mut rest: Vec<Group> = Vec::new();
    for c in transcript.map(cited).unwrap_or_default() {
        let same = known.iter().any(|u| !u.is_empty() && c.url.trim_end_matches('/').starts_with(u.as_str()));
        if same || (!ticket_key.is_empty() && c.detail == ticket_key) {
            continue;
        }
        let is_pr = PR_KINDS.contains(&c.kind);
        let is_ticket = ticket.is_some() && matches!(c.icon, "ticket" | "circle-dot") && c.name == ticket_name;
        let (kind, item) = match () {
            _ if is_pr => ("pr", pr_item(&c, inbox)),
            _ if is_ticket => ("ticket", ticket_item(&c, inbox)),
            _ => (c.kind, item(&c.detail, &c.url)),
        };
        if let Some(g) = groups.iter_mut().chain(rest.iter_mut()).find(|g| g.kind == kind && g.name == c.name) {
            g.push(item);
            continue;
        }
        let mut g = Group::new(kind, &c.name, c.icon);
        g.push(item);
        // A pull request of another host sits with the PRs, ahead of the pages.
        if kind == "pr" { groups.insert(groups.iter().position(|g| g.kind != "pr").unwrap_or(groups.len()), g) } else { rest.push(g) }
    }
    groups.extend(rest);
    let labels: Vec<String> = groups
        .iter()
        .map(|g| {
            let shared = groups.iter().filter(|o| o.name == g.name).count() > 1;
            let detail = text(&g.items[0]["detail"]).split(" · ").next().unwrap_or_default().to_string();
            // CI from the code host (GitHub's checks) is just CI beside its PR; other shared names carry their detail.
            match (shared, g.kind) {
                (true, "ci") => "CI".to_string(),
                (true, "pr") => g.name.clone(),
                (true, _) if !detail.is_empty() => format!("{} {detail}", g.name),
                _ => g.name.clone(),
            }
        })
        .collect();
    groups.iter().zip(labels).map(|(g, label)| g.json(label)).collect()
}

/// The views (documents) `show` keeps, each `{name, terminal, title, at, placement}`: the folder is listed once per call,
/// and a file is read again only once it changed.
pub fn shown() -> Vec<Value> {
    let dir = files::cache().join("show");
    let mut cache = VIEWS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut seen = Vec::new();
    for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(name) = name.strip_suffix(".json").map(str::to_string) else { continue };
        let modified = entry.metadata().ok().and_then(|m| m.modified().ok());
        if cache.get(&name).is_none_or(|(at, _)| *at != modified) {
            let spec = files::load(&entry.path()).unwrap_or(Value::Null);
            let brief = json!({"terminal": spec["terminal"], "title": spec["title"], "at": spec["at"], "placement": spec["placement"]});
            cache.insert(name.clone(), (modified, brief));
        }
        seen.push(name);
    }
    cache.retain(|name, _| seen.contains(name));
    cache.iter().map(|(n, (_, s))| json!({"name": n, "terminal": s["terminal"], "title": s["title"], "at": s["at"], "placement": s["placement"]})).collect()
}

/// The views of `shown` a terminal's session showed this week, newest first: name and title. A view names its terminal
/// by any of `ids` (the Herdr app's address or the row's handle).
pub fn documents(shown: &[Value], ids: &[&str]) -> Vec<Value> {
    let now = cockpit_core::seen::now_ms() as f64;
    let mut mine: Vec<&Value> = shown
        .iter()
        .filter(|s| s["terminal"].as_str().is_some_and(|t| ids.contains(&t)) && s["placement"] != "browser" && now - s["at"].as_f64().unwrap_or(0.0) <= KEEP_MS)
        .collect();
    mine.sort_by(|a, b| b["at"].as_f64().unwrap_or(0.0).total_cmp(&a["at"].as_f64().unwrap_or(0.0)));
    mine.into_iter().map(|s| json!({"name": s["name"], "title": s["title"]})).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_terminals_documents_are_its_own_of_the_week_newest_first_never_a_browser_page() {
        let now = cockpit_core::seen::now_ms() as f64;
        let shown = [
            json!({"name": "quota-chart", "terminal": "app:t1", "title": "Quota use", "at": now - 60_000.0, "placement": "right"}),
            json!({"name": "plan", "terminal": "w1:p1", "title": "Plan", "at": now - 1_000.0, "placement": "right"}),
            json!({"name": "page", "terminal": "app:t1", "title": "Page", "at": now, "placement": "browser"}),
            json!({"name": "old", "terminal": "app:t1", "title": "Old", "at": now - 8.0 * 86_400_000.0, "placement": "right"}),
            json!({"name": "theirs", "terminal": "app:t2", "title": "Theirs", "at": now, "placement": "right"}),
        ];
        let mine = documents(&shown, &["w1:p1", "app:t1"]);
        assert_eq!(mine, [json!({"name": "plan", "title": "Plan"}), json!({"name": "quota-chart", "title": "Quota use"})]);
    }

    #[test]
    fn links_are_named_after_their_service_and_grouped_by_kind() {
        let dir = tempfile::tempdir().unwrap();
        let transcript = dir.path().join("t.jsonl");
        std::fs::write(
            &transcript,
            "{\"text\":\"https://acme.slack.com/archives/C1/p1 and https://linear.app/acme/issue/ENG-4/x then https://github.com/acme/api/pull/13 and https://linear.app/acme/issue/ENG-7/limits\"}\n",
        )
        .unwrap();
        let ticket = json!({"key": "ENG-4", "title": "Quotas", "url": "https://linear.app/acme/issue/ENG-4/quotas"});
        let pr = json!({"url": "https://github.com/acme/api/pull/12", "number": 12, "title": "Quotas", "state": "open"});
        let inbox = json!({
            "mine": [{"url": "https://github.com/acme/api/pull/13", "number": 13, "title": "Quota limits", "branch": "me/eng-7"}],
            "tickets": [{"id": "ENG-7", "title": "Limits", "url": "https://linear.app/acme/issue/ENG-7/limits", "status": "Todo"}],
            "deployLinks": {"api": {"dashboard": "https://argocd.acme.internal/applications/api", "workflow": {"label": "v1", "url": "https://argo-workflows.acme.internal/w/1"}}}
        });
        let config = json!({"tickets": {"name": "Linear"}, "ci": {"name": "CircleCI"}});
        let links = of(Some(&ticket), &pr, &json!({"state": "pass", "label": "CI green"}), "api", Some("eng-4"), &inbox, &config, Some(&transcript));
        let labels: Vec<&str> = links.iter().map(|l| l["label"].as_str().unwrap()).collect();
        assert_eq!(labels, ["GitHub", "Linear", "CircleCI", "Argo", "Argo Workflows", "Slack"], "the ticket cited again is not repeated");
        let counts: Vec<usize> = links.iter().map(|l| l["items"].as_array().unwrap().len()).collect();
        assert_eq!(counts, [2, 2, 1, 1, 1, 1]);
        assert_eq!(links[0]["items"][0]["detail"], "#12 · Quotas");
        assert_eq!(
            (&links[0]["items"][1]["detail"], &links[0]["items"][1]["title"], &links[0]["items"][1]["state"]),
            (&json!("#13"), &json!("Quota limits"), &json!("open"))
        );
        assert_eq!(
            (&links[1]["items"][0]["detail"], &links[1]["items"][1]["title"], &links[1]["items"][1]["state"]),
            (&json!("ENG-4 · Quotas"), &json!("Limits"), &json!("Todo"))
        );
        assert_eq!(
            (links[2]["items"][0]["state"].as_str(), links[2]["items"][0]["url"].as_str()),
            (Some("pass"), Some("https://github.com/acme/api/pull/12/checks"))
        );
        let github_ci = json!({"tickets": {"name": "Linear"}, "ci": {"name": "GitHub"}});
        let links = of(Some(&ticket), &pr, &json!({"state": "pass", "label": "CI green"}), "api", None, &json!({}), &github_ci, None);
        let labels: Vec<&str> = links.iter().map(|l| l["label"].as_str().unwrap()).collect();
        assert_eq!(labels, ["GitHub", "Linear", "CI"], "the code host's own checks are just CI beside its PR");
    }

    #[test]
    fn the_tickets_and_prs_a_transcript_names_come_last_first() {
        let dir = tempfile::tempdir().unwrap();
        let transcript = dir.path().join("t.jsonl");
        std::fs::write(&transcript, "see ENG-412 and eng-7, then https://github.com/acme/api/pull/2 after https://gitlab.acme.io/g/p/-/merge_requests/9\n")
            .unwrap();
        let pattern = Regex::new(r"(?i)(?:^|[^a-z])(ENG)-(\d+)").unwrap();
        assert_eq!(cited_tickets(&transcript, Some(&pattern)), ["ENG-7", "ENG-412"]);
        assert_eq!(cited_prs(&transcript), ["https://github.com/acme/api/pull/2", "https://gitlab.acme.io/g/p/-/merge_requests/9"]);
        assert!(cited_tickets(&transcript, None).is_empty());
    }
}
