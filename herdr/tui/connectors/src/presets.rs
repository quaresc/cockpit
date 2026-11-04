//! The MCP servers cockpit knows: their public address, the source they feed and, for most, how to read them; one
//! without a mapping is mapped by the person's agent at setup.

use serde_json::{Value, json};

use crate::client::Server;

pub struct Preset {
    /// The name its `servers` entry takes.
    pub id: &'static str,
    pub name: &'static str,
    /// The config sections it feeds.
    pub sources: &'static [&'static str],
    /// Its remote server.
    pub url: &'static str,
    /// A command printing the bearer token it takes, when it has no OAuth login (GitHub's takes gh's).
    pub bearer_command: &'static [&'static str],
    /// The scopes it asks for when its login needs the person's own app (Google's), else empty.
    pub app_scopes: &'static [&'static str],
    /// Text that tells it in a URL or a command.
    pub marks: &'static [&'static str],
    /// The section's mapping (`tools`, `fields`…), when cockpit has one.
    pub mapping: Option<fn() -> Value>,
}

impl Preset {
    /// Whether its login needs the person's own OAuth app (a client id from Google Cloud).
    pub fn needs_app(&self) -> bool {
        !self.app_scopes.is_empty()
    }

    /// Its `servers` entry.
    pub fn entry(&self) -> Value {
        if self.bearer_command.is_empty() { json!({"url": self.url}) } else { json!({"url": self.url, "bearerCommand": self.bearer_command}) }
    }
}

pub const PRESETS: &[Preset] = &[
    Preset {
        id: "gmail",
        name: "Gmail",
        sources: &["mail"],
        url: "https://gmailmcp.googleapis.com/mcp/v1",
        bearer_command: &[],
        app_scopes: &["https://www.googleapis.com/auth/gmail.modify"],
        marks: &["gmailmcp.googleapis.com"],
        mapping: None,
    },
    Preset {
        id: "google-calendar",
        name: "Google Calendar",
        sources: &["calendar"],
        url: "https://calendarmcp.googleapis.com/mcp/v1",
        bearer_command: &[],
        app_scopes: &["https://www.googleapis.com/auth/calendar.readonly"],
        marks: &["calendarmcp.googleapis.com"],
        mapping: None,
    },
    Preset {
        id: "linear",
        name: "Linear",
        sources: &["tickets"],
        url: "https://mcp.linear.app/mcp",
        bearer_command: &[],
        app_scopes: &[],
        marks: &["mcp.linear.app", "linear-mcp", "@linear/", "mcp-linear"],
        mapping: Some(crate::tickets::linear),
    },
    Preset {
        id: "circleci",
        name: "CircleCI",
        sources: &["ci"],
        url: "https://mcp.circleci.com/v1/mcp",
        bearer_command: &[],
        app_scopes: &[],
        marks: &["mcp.circleci.com", "mcp-server-circleci", "circleci-mcp"],
        mapping: Some(crate::ci::circleci),
    },
    Preset {
        id: "github",
        name: "GitHub Issues",
        sources: &["tickets"],
        url: "https://api.githubcopilot.com/mcp/",
        bearer_command: &["gh", "auth", "token"],
        app_scopes: &[],
        marks: &["api.githubcopilot.com/mcp", "github-mcp-server", "server-github"],
        mapping: None,
    },
    Preset {
        id: "gitlab",
        name: "GitLab",
        sources: &["tickets", "ci", "code"],
        url: "https://gitlab.com/api/v4/mcp",
        bearer_command: &[],
        app_scopes: &[],
        marks: &["/api/v4/mcp", "gitlab-mcp", "server-gitlab"],
        mapping: None,
    },
    Preset {
        id: "atlassian",
        name: "Jira",
        sources: &["tickets"],
        url: "https://mcp.atlassian.com/v1/mcp",
        bearer_command: &[],
        app_scopes: &[],
        marks: &["mcp.atlassian.com", "mcp-atlassian"],
        mapping: None,
    },
    Preset {
        id: "datadog",
        name: "Datadog",
        sources: &["observability"],
        url: "https://mcp.datadoghq.com/v1/mcp",
        bearer_command: &[],
        app_scopes: &[],
        marks: &["mcp.datadoghq.", "datadog-mcp"],
        mapping: None,
    },
    Preset {
        id: "sentry",
        name: "Sentry",
        sources: &["observability"],
        url: "https://mcp.sentry.dev/mcp",
        bearer_command: &[],
        app_scopes: &[],
        marks: &["mcp.sentry.dev", "sentry-mcp"],
        mapping: None,
    },
];

/// The preset a server is, by its address or its command.
pub fn of(server: &Server) -> Option<&'static Preset> {
    let text = server.describe();
    PRESETS.iter().find(|p| p.marks.iter().any(|m| text.contains(m)))
}

pub fn for_source(source: &str) -> impl Iterator<Item = &'static Preset> {
    PRESETS.iter().filter(move |p| p.sources.contains(&source))
}

/// Words that tell a server of a source in its name or address, for one cockpit has no preset for.
pub fn hints(source: &str) -> &'static [&'static str] {
    match source {
        "tickets" => &["jira", "linear", "asana", "trello", "youtrack", "shortcut", "clickup", "monday", "plane", "issue", "ticket", "github", "gitlab"],
        "ci" => &["circle", "gitlab", "buildkite", "jenkins", "travis", "drone", "teamcity", "azure", "bitrise", "pipeline", "github"],
        "mail" => &["gmail", "outlook", "mail", "fastmail", "proton", "imap"],
        "calendar" => &["calendar", "outlook", "gcal", "cal.com", "fastmail", "ical", "agenda"],
        "platform" => &["deploy", "platform", "backstage", "argo", "vercel", "render", "fly", "heroku", "railway", "preview", "kube"],
        "code" => &["gitlab", "bitbucket", "gitea", "forgejo", "azure", "codeberg", "sourcehut"],
        "observability" => {
            &["datadog", "grafana", "prometheus", "sentry", "newrelic", "new-relic", "honeycomb", "signoz", "elastic", "loki", "dynatrace", "metrics"]
        }
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_server_is_known_by_its_address_or_command() {
        assert_eq!(of(&Server::parse("https://mcp.linear.app/mcp")).map(|p| p.id), Some("linear"));
        assert_eq!(of(&Server::parse("npx -y mcp-remote https://mcp.linear.app/sse")).map(|p| p.id), Some("linear"));
        assert_eq!(of(&Server::parse("https://gitlab.example.com/api/v4/mcp")).map(|p| p.id), Some("gitlab"));
        assert!(of(&Server::parse("https://example.com/mcp")).is_none());
        assert_eq!(for_source("ci").map(|p| p.id).collect::<Vec<_>>(), ["circleci", "gitlab"]);
        assert_eq!(for_source("code").map(|p| p.id).collect::<Vec<_>>(), ["gitlab"]);
        assert_eq!(
            PRESETS.iter().find(|p| p.id == "github").unwrap().entry(),
            json!({"url": "https://api.githubcopilot.com/mcp/", "bearerCommand": ["gh", "auth", "token"]})
        );
    }
}
