//! What an agent works with, as links named after their service (Linear, GitHub, CircleCI, Argo, Slack…): its ticket,
//! PR, CI and deploy dashboards, and the pages its conversation cites, read from its transcript's tail whatever the agent.
//! Its documents are the views it showed beside itself.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::SystemTime;

use cockpit_core::sites::{self, Link};
use cockpit_core::{files, transcripts};
use serde_json::{Value, json};

/// How much of a transcript's end is read for the pages it cites.
const TAIL: u64 = 256 * 1024;
/// A view older than this is gone for good.
const KEEP_MS: f64 = 7.0 * 86_400_000.0;

type Stamp = (u64, Option<SystemTime>);

/// Each transcript's cited pages, by the stamp they were read at.
type Cited = HashMap<PathBuf, (Stamp, Vec<Link>)>;
/// Each view's brief (terminal, title, time, placement), by the modification time it was read at.
type Briefs = HashMap<String, (Option<SystemTime>, Value)>;

static CITED: LazyLock<Mutex<Cited>> = LazyLock::new(Mutex::default);
static VIEWS: LazyLock<Mutex<Briefs>> = LazyLock::new(Mutex::default);

fn stamp(path: &Path) -> Option<Stamp> {
    std::fs::metadata(path).ok().map(|m| (m.len(), m.modified().ok()))
}

/// The pages the transcript's tail cites, read again only when the file changed.
fn cited(transcript: &Path) -> Vec<Link> {
    let Some(now) = stamp(transcript) else { return Vec::new() };
    let mut cache = CITED.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((at, links)) = cache.get(transcript)
        && *at == now
    {
        return links.clone();
    }
    let text = transcripts::tail(transcript, TAIL).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    let links = sites::cited(&text);
    cache.insert(transcript.to_path_buf(), (now, links.clone()));
    links
}

fn link(kind: &str, name: &str, icon: &str, detail: &str, url: &str) -> Value {
    json!({"kind": kind, "name": name, "icon": icon, "detail": detail, "url": url})
}

fn text(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

/// Everything the row links to, always in this order: PR, ticket, CI, deploy dashboard, deploy run, then the cited pages
/// in the sites table's order; a name two links share carries its detail.
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
    let mut out = Vec::new();
    let pr_url = text(&pr["url"]);
    if !pr_url.is_empty() {
        let detail = format!("#{} · {}", pr["number"], text(&pr["title"]));
        out.push(link("pr", sites::host_name(pr_url).unwrap_or("PR"), "git-pull-request", &detail, pr_url));
    }
    if let Some(t) = ticket.filter(|t| !text(&t["url"]).is_empty()) {
        let name = Some(text(&config["tickets"]["name"])).filter(|n| !n.is_empty()).or_else(|| sites::host_name(text(&t["url"]))).unwrap_or("Ticket");
        let detail = [text(&t["key"]), text(&t["title"])].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
        out.push(link("ticket", name, "ticket", &detail, text(&t["url"])));
    }
    if let Some(state) = ci["state"].as_str().filter(|_| !pr_url.is_empty()) {
        let name = Some(text(&config["ci"]["name"])).filter(|n| !n.is_empty()).unwrap_or("CI");
        let mut ci = link("ci", name, "circle-dot", text(&ci["label"]), &format!("{pr_url}/checks"));
        ci["state"] = json!(state);
        out.push(ci);
    }
    let preview = inbox["previews"].as_array().into_iter().flatten().find(|e| text(&e["app"]) == repo && branch.is_some_and(|b| text(&e["branch"]) == b));
    let preview_dashboard = preview.and_then(|e| {
        let template = text(&config["platform"]["previewDashboardUrl"]);
        (!template.is_empty()).then(|| (text(&e["id"]).to_string(), template.replace("{preview}", text(&e["id"]))))
    });
    let deploy = &inbox["deployLinks"][repo];
    match preview_dashboard {
        Some((id, url)) => out.push(link("dashboard", sites::dashboard_name(&url), "rocket", &format!("preview {id}"), &url)),
        None if !text(&deploy["dashboard"]).is_empty() => {
            let url = text(&deploy["dashboard"]);
            out.push(link("dashboard", sites::dashboard_name(url), "rocket", "production", url));
        }
        None => {}
    }
    if !text(&deploy["workflow"]["url"]).is_empty() {
        let url = text(&deploy["workflow"]["url"]);
        out.push(link("workflow", sites::dashboard_name(url), "workflow", &format!("deploy {}", text(&deploy["workflow"]["label"])), url));
    }
    let known: Vec<String> = out.iter().map(|l| text(&l["url"]).trim_end_matches('/').to_string()).collect();
    let ticket_key = ticket.map(|t| text(&t["key"])).unwrap_or_default();
    for c in transcript.map(cited).unwrap_or_default() {
        let same = known.iter().any(|u| c.url.trim_end_matches('/').starts_with(u.as_str()) && !u.is_empty());
        if same || (!ticket_key.is_empty() && c.detail == ticket_key) {
            continue;
        }
        out.push(link(c.kind, &c.name, c.icon, &c.detail, &c.url));
    }
    for i in 0..out.len() {
        let name = text(&out[i]["name"]).to_string();
        let shared = out.iter().filter(|l| text(&l["name"]) == name).count() > 1;
        let detail = text(&out[i]["detail"]).split(" · ").next().unwrap_or_default().to_string();
        // CI from the code host (GitHub's checks) is just CI beside its PR; other shared names carry their detail.
        out[i]["label"] = json!(match (shared, out[i]["kind"].as_str()) {
            (true, Some("ci")) => "CI".to_string(),
            (true, Some("pr")) => name,
            (true, _) if !detail.is_empty() => format!("{name} {detail}"),
            _ => name,
        });
    }
    out
}

/// The views (documents) a terminal's session showed this week, newest first: name and title. A view names its terminal
/// by any of `ids` (the Herdr app's address or the row's handle).
pub fn documents(ids: &[&str]) -> Vec<Value> {
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
    let now = cockpit_core::seen::now_ms() as f64;
    let mut mine: Vec<(&String, &Value)> = cache
        .iter()
        .map(|(n, (_, s))| (n, s))
        .filter(|(_, s)| {
            s["terminal"].as_str().is_some_and(|t| ids.contains(&t)) && s["placement"] != "browser" && now - s["at"].as_f64().unwrap_or(0.0) <= KEEP_MS
        })
        .collect();
    mine.sort_by(|a, b| b.1["at"].as_f64().unwrap_or(0.0).total_cmp(&a.1["at"].as_f64().unwrap_or(0.0)));
    mine.into_iter().map(|(n, s)| json!({"name": n, "title": s["title"]})).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_are_named_after_their_service() {
        let dir = tempfile::tempdir().unwrap();
        let transcript = dir.path().join("t.jsonl");
        std::fs::write(&transcript, "{\"text\":\"https://acme.slack.com/archives/C1/p1 and https://linear.app/acme/issue/ENG-4/x\"}\n").unwrap();
        let ticket = json!({"key": "ENG-4", "title": "Quotas", "url": "https://linear.app/acme/issue/ENG-4/quotas"});
        let pr = json!({"url": "https://github.com/acme/api/pull/12", "number": 12, "title": "Quotas"});
        let inbox = json!({"deployLinks": {"api": {"dashboard": "https://argocd.acme.internal/applications/api", "workflow": {"label": "v1", "url": "https://argo-workflows.acme.internal/w/1"}}}});
        let config = json!({"tickets": {"name": "Linear"}, "ci": {"name": "CircleCI"}});
        let links = of(Some(&ticket), &pr, &json!({"state": "pass", "label": "CI green"}), "api", Some("eng-4"), &inbox, &config, Some(&transcript));
        let labels: Vec<&str> = links.iter().map(|l| l["label"].as_str().unwrap()).collect();
        assert_eq!(labels, ["GitHub", "Linear", "CircleCI", "Argo", "Argo Workflows", "Slack"], "the ticket cited again is not repeated");
        assert_eq!((links[1]["detail"].as_str(), links[2]["state"].as_str()), (Some("ENG-4 · Quotas"), Some("pass")));
        let github_ci = json!({"tickets": {"name": "Linear"}, "ci": {"name": "GitHub"}});
        let links = of(Some(&ticket), &pr, &json!({"state": "pass", "label": "CI green"}), "api", None, &json!({}), &github_ci, None);
        let labels: Vec<&str> = links.iter().map(|l| l["label"].as_str().unwrap()).collect();
        assert_eq!(labels, ["GitHub", "Linear", "CI"], "the code host's own checks are just CI beside its PR");
    }
}
