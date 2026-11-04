//! The tools a conversation links to, each named after its service: the pages worth a link in the agent's links, found
//! in any text (a transcript's tail, a ticket's description). The Claude plugin keeps the same table in `sites.ts`.

use std::sync::LazyLock;

use regex::Regex;

/// A page of a tool: the service's name, the icon of its kind, what the page is (an issue number, a doc kind) and its URL.
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub kind: &'static str,
    pub name: String,
    pub icon: &'static str,
    pub detail: String,
    pub url: String,
}

struct Site {
    kind: &'static str,
    name: &'static str,
    icon: &'static str,
    host: &'static str,
    path: &'static str,
}

const SITES: [Site; 32] = [
    // Code review and issues
    Site { kind: "mr", name: "GitLab", icon: "git-pull-request", host: r"gitlab\.[\w.-]+", path: r"/[^\s?#]+?/-/merge_requests/(?<ref>\d+)" },
    Site { kind: "bitbucket", name: "Bitbucket", icon: "git-pull-request", host: r"bitbucket\.org", path: r"/[\w.-]+/[\w.-]+/pull-requests/(?<ref>\d+)" },
    Site {
        kind: "azure-pr",
        name: "Azure DevOps",
        icon: "git-pull-request",
        host: r"dev\.azure\.com|[\w-]+\.visualstudio\.com",
        path: r"/[^\s?#]+?/_git/[^\s?#]+?/pullrequest/(?<ref>\d+)",
    },
    Site { kind: "issue", name: "GitHub", icon: "circle-dot", host: r"github\.com", path: r"/[\w.-]+/[\w.-]+/issues/(?<ref>\d+)" },
    Site { kind: "gitlab-issue", name: "GitLab", icon: "circle-dot", host: r"gitlab\.[\w.-]+", path: r"/[^\s?#]+?/-/(?:issues|work_items)/(?<ref>\d+)" },
    // Tickets
    Site { kind: "linear", name: "Linear", icon: "ticket", host: r"linear\.app", path: r"/[\w-]+/issue/(?<ref>[A-Z][A-Z0-9]+-\d+)" },
    Site {
        kind: "jira",
        name: "Jira",
        icon: "ticket",
        host: r"[\w-]+\.atlassian\.net|jira\.[\w.-]+",
        path: r#"/(?:browse/|[^\s"'`]*?selectedIssue=)(?<ref>[A-Z][A-Z0-9]+-\d+)"#,
    },
    Site {
        kind: "azure-item",
        name: "Azure DevOps",
        icon: "ticket",
        host: r"dev\.azure\.com|[\w-]+\.visualstudio\.com",
        path: r"/[^\s?#]+?/_workitems/edit/(?<ref>\d+)",
    },
    Site { kind: "trello", name: "Trello", icon: "ticket", host: r"trello\.com", path: r"/[cb]/" },
    Site { kind: "asana", name: "Asana", icon: "ticket", host: r"app\.asana\.com", path: r"/\d+/" },
    Site { kind: "clickup", name: "ClickUp", icon: "ticket", host: r"app\.clickup\.com", path: r"/t/" },
    Site { kind: "shortcut", name: "Shortcut", icon: "ticket", host: r"app\.shortcut\.com", path: r"/[\w-]+/story/(?<ref>\d+)" },
    // Design
    Site { kind: "figma", name: "Figma", icon: "palette", host: r"(?:www\.)?figma\.com", path: r"/(?:design|file|proto|board|slides|make)/" },
    Site { kind: "miro", name: "Miro", icon: "palette", host: r"miro\.com", path: r"/app/board/" },
    Site { kind: "excalidraw", name: "Excalidraw", icon: "palette", host: r"(?:app\.|link\.)?excalidraw\.com", path: r"/(?:#json=|#room=|l/|s/)" },
    // Docs
    Site { kind: "notion", name: "Notion", icon: "file-text", host: r"(?:[\w-]+\.)?notion\.(?:so|site)|app\.notion\.com", path: r"/[\w-]" },
    Site { kind: "confluence", name: "Confluence", icon: "file-text", host: r"[\w-]+\.atlassian\.net|confluence\.[\w.-]+", path: r"/(?:wiki|display|pages)/" },
    Site { kind: "google", name: "Google", icon: "file-text", host: r"docs\.google\.com", path: r"/(?<ref>document|spreadsheets|presentation|forms)/d/" },
    Site { kind: "coda", name: "Coda", icon: "file-text", host: r"coda\.io", path: r"/d/" },
    Site { kind: "loom", name: "Loom", icon: "video", host: r"(?:www\.)?loom\.com", path: r"/(?:share|embed)/" },
    // Chat
    Site { kind: "slack", name: "Slack", icon: "message-square", host: r"[a-z0-9-]+\.slack\.com", path: r"/archives/" },
    Site { kind: "teams", name: "Teams", icon: "message-square", host: r"teams\.microsoft\.com|teams\.live\.com|teams\.cloud\.microsoft", path: r"/l/" },
    Site { kind: "discord", name: "Discord", icon: "message-square", host: r"(?:ptb\.|canary\.)?discord(?:app)?\.com", path: r"/channels/" },
    // Monitoring and incidents
    Site { kind: "sentry", name: "Sentry", icon: "bug", host: r"(?:[\w-]+\.)?sentry\.io|sentry\.[\w.-]+", path: r#"/[^\s"'`]*?issues/\d+"# },
    Site { kind: "datadog", name: "Datadog", icon: "activity", host: r"(?:[\w-]+\.)?datadoghq\.(?:com|eu)|(?:[\w-]+\.)?ddog-gov\.com", path: r"/[\w-]" },
    Site { kind: "grafana", name: "Grafana", icon: "activity", host: r"[\w-]+\.grafana\.net|grafana\.[\w.-]+", path: r"/(?:d/|explore|alerting/)" },
    Site { kind: "pagerduty", name: "PagerDuty", icon: "triangle-alert", host: r"[\w-]+\.pagerduty\.com", path: r"/incidents/" },
    Site { kind: "incident", name: "incident.io", icon: "triangle-alert", host: r"app\.incident\.io", path: r#"/[^\s"'`]*?incidents/"# },
    // Product analytics
    Site { kind: "amplitude", name: "Amplitude", icon: "chart-column", host: r"(?:[\w-]+\.)*amplitude\.com", path: r"/analytics/" },
    Site { kind: "mixpanel", name: "Mixpanel", icon: "chart-column", host: r"(?:eu\.)?mixpanel\.com", path: r"/project/" },
    Site { kind: "posthog", name: "PostHog", icon: "chart-column", host: r"(?:us|eu|app)\.posthog\.com", path: r"/project/" },
    Site {
        kind: "metabase",
        name: "Metabase",
        icon: "chart-column",
        host: r"metabase\.[\w.-]+|[\w-]+\.metabaseapp\.com",
        path: r"/(?:question|dashboard)/\d+",
    },
];

/// The rest of a URL: up to a space, the punctuation around it, or the backslash of a JSON escape.
const REST: &str = r#"[^\s)>\]"'`\\]*"#;

static COMPILED: LazyLock<Vec<(&'static Site, Regex)>> =
    LazyLock::new(|| SITES.iter().filter_map(|s| Regex::new(&format!("https://(?:{}){}{REST}", s.host, s.path)).ok().map(|r| (s, r))).collect());

/// The URL without the punctuation that closes the sentence.
fn clean(url: &str) -> &str {
    url.trim_end_matches(['.', ',', ';', ':', '!', '?'])
}

/// The page each kind of site gets in this text, the last one cited winning, in the table's order.
pub fn cited(text: &str) -> Vec<Link> {
    COMPILED
        .iter()
        .filter_map(|(site, re)| {
            let m = re.captures_iter(text).last()?;
            let url = clean(m.get(0)?.as_str()).to_string();
            let reference = m.name("ref").map(|r| r.as_str()).unwrap_or_default();
            let (name, detail) = match (site.kind, reference) {
                ("google", "document") => ("Google Docs".to_string(), String::new()),
                ("google", "spreadsheets") => ("Google Sheets".to_string(), String::new()),
                ("google", "presentation") => ("Google Slides".to_string(), String::new()),
                ("google", "forms") => ("Google Forms".to_string(), String::new()),
                ("mr" | "azure-pr", r) if !r.is_empty() => (site.name.to_string(), format!("!{r}")),
                ("issue" | "gitlab-issue" | "bitbucket" | "azure-item", r) if !r.is_empty() => (site.name.to_string(), format!("#{r}")),
                ("shortcut", r) if !r.is_empty() => (site.name.to_string(), format!("sc-{r}")),
                (_, r) => (site.name.to_string(), r.to_string()),
            };
            Some(Link { kind: site.kind, name, icon: site.icon, detail, url })
        })
        .collect()
}

/// A dashboard's service, by its host.
pub fn dashboard_name(url: &str) -> &'static str {
    let host = url.split("://").nth(1).unwrap_or(url).split('/').next().unwrap_or_default();
    let argo = host.split(['.', '-']).any(|part| part == "argocd" || part == "argo");
    match host {
        _ if argo && host.contains("workflows") => "Argo Workflows",
        _ if argo => "Argo",
        h if h.ends_with("vercel.com") => "Vercel",
        h if h.ends_with("netlify.app") || h.ends_with("netlify.com") => "Netlify",
        h if h.ends_with("render.com") => "Render",
        h if h.ends_with("railway.app") => "Railway",
        h if h.ends_with("fly.io") => "Fly",
        _ => "Dashboard",
    }
}

/// A code host or tracker, by its URL's host.
pub fn host_name(url: &str) -> Option<&'static str> {
    let host = url.split("://").nth(1).unwrap_or(url).split('/').next().unwrap_or_default();
    Some(match host {
        "github.com" => "GitHub",
        "linear.app" => "Linear",
        "app.shortcut.com" => "Shortcut",
        h if h.starts_with("gitlab.") => "GitLab",
        "bitbucket.org" => "Bitbucket",
        h if h.ends_with(".atlassian.net") || h.starts_with("jira.") => "Jira",
        h if h == "dev.azure.com" || h.ends_with(".visualstudio.com") => "Azure DevOps",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_site_keeps_its_last_page_named_after_its_service() {
        let text = r#"see https://acme.slack.com/archives/C1/p1. Then {"url":"https://acme.slack.com/archives/C2/p2\"} and
            https://docs.google.com/spreadsheets/d/abc/edit, https://github.com/acme/api/issues/42 and
            https://linear.app/acme/issue/ENG-7/quota"#;
        let links = cited(text);
        let pick = |kind: &str| links.iter().find(|l| l.kind == kind).cloned().unwrap();
        assert_eq!(pick("slack").url, "https://acme.slack.com/archives/C2/p2");
        assert_eq!((pick("google").name.as_str(), pick("issue").name.as_str(), pick("issue").detail.as_str()), ("Google Sheets", "GitHub", "#42"));
        assert_eq!((pick("linear").name.as_str(), pick("linear").detail.as_str()), ("Linear", "ENG-7"));
        assert_eq!(links.iter().map(|l| l.kind).collect::<Vec<_>>(), ["issue", "linear", "google", "slack"]);
    }

    #[test]
    fn dashboards_and_hosts_by_name() {
        assert_eq!(dashboard_name("https://argocd.production.acme.internal/applications/api"), "Argo");
        assert_eq!(dashboard_name("https://argo-workflows.production.acme.internal/workflows/x"), "Argo Workflows");
        assert_eq!((host_name("https://linear.app/acme/issue/ENG-1"), host_name("https://example.com")), (Some("Linear"), None));
    }
}
