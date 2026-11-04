//! The apps' requests (a CI rerun, a preview's redeploy, creation, stop or extension, a ticket's release) carried out by
//! the hub when it reaches that source's server itself; the others stay for a Claude session to take.

use std::path::Path;

use cockpit_connectors::tools::Mapped;
use cockpit_connectors::{Server, ci, platform, tickets};
use serde_json::{Value, json};

use super::{mcp, news, now_ms};
use crate::config::Config;

/// A request nobody took in this time was given up on by the app.
const TTL_MS: u64 = 10 * 60_000;

/// The fields each request carries, and their kind.
const FIELDS: &[(&str, &[(&str, &str)])] = &[
    ("rerun", &[("workflowId", "string")]),
    ("redeploy", &[("preview", "string"), ("app", "string")]),
    ("release", &[("ticket", "string")]),
    ("extend", &[("preview", "string"), ("days", "number")]),
    ("create-preview", &[("app", "string"), ("branch", "string")]),
    ("stop-preview", &[("preview", "string")]),
    ("mail-archive", &[("message", "string")]),
    ("mail-read", &[("message", "string")]),
];

/// The section whose server carries a request out.
fn section_of(kind: &str) -> &'static str {
    match kind {
        "rerun" => "ci",
        "release" => "tickets",
        "mail-archive" | "mail-read" => "mail",
        _ => "platform",
    }
}

/// What is wrong with a request, as the plugin's checkRequest says it.
pub fn refusal(request: &Value) -> Option<String> {
    let kind = request["type"].as_str().unwrap_or_default();
    let Some((_, fields)) = FIELDS.iter().find(|(k, _)| *k == kind) else {
        let allowed: Vec<&str> = FIELDS.iter().map(|(k, _)| *k).collect();
        return Some(format!("unknown request type {}; allowed: {}", request["type"], allowed.join(", ")));
    };
    let all: Vec<(&str, &str)> = std::iter::once(("label", "string")).chain(fields.iter().copied()).collect();
    for (name, kind_of) in &all {
        let value = &request[*name];
        let fits = if *kind_of == "number" { value.is_number() } else { value.as_str().is_some_and(|s| !s.is_empty()) };
        if !fits {
            let wanted: Vec<String> = all.iter().map(|(n, k)| format!("{n} ({k})")).collect();
            return Some(format!("{kind} needs {name} as a {kind_of}, got {value}; it takes {}", wanted.join(", ")));
        }
    }
    None
}

/// Takes the requests the hub's servers can carry out, once each, and writes each outcome where the app waits for it.
pub fn take(cache: &Path, config: &Config, raw: &Value) {
    let dir = cache.join("requests");
    let Ok(entries) = std::fs::read_dir(&dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".json") || name.starts_with("claimed-") {
            continue;
        }
        let path = entry.path();
        let Some(request) = cockpit_core::files::load(&path) else { continue };
        let kind = request["type"].as_str().unwrap_or_default();
        let section = section_of(kind);
        let Some(server) = mcp::server(raw, &config.0, section) else { continue };
        // A rename is atomic: whoever moves the file carries it out.
        let claimed = dir.join(format!("claimed-{name}"));
        if std::fs::rename(&path, &claimed).is_err() {
            continue;
        }
        let age = entry.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok()).map_or(0, |d| d.as_millis() as u64);
        let _ = std::fs::remove_file(&claimed);
        let results = dir.join("results");
        let _ = std::fs::create_dir_all(&results);
        let result = results.join(&name);
        let write = |state: &str, message: &str| {
            let outcome = json!({"ok": state == "done", "state": state, "message": message, "at": now_ms()});
            let _ = cockpit_core::files::write(&result, &outcome);
            let _ = std::fs::remove_file(results.join(format!("{name}.claimed")));
        };
        if age > TTL_MS {
            write("refused", "expired before anyone took it");
            continue;
        }
        if let Some(why) = refusal(&request) {
            write("refused", &why);
            continue;
        }
        let _ = std::fs::write(results.join(format!("{name}.claimed")), now_ms().to_string());
        let label = request["label"].as_str().unwrap_or(kind).to_string();
        match carry_out(&server, config, raw, &request) {
            Ok((message, title, body, action)) => {
                write("done", &message);
                news::notify(config, &title, &body, &action, &[]);
                let _ = std::fs::write(cache.join("refresh-request"), "");
            }
            Err(e) => {
                write("failed", &e.chars().take(200).collect::<String>());
                news::notify(config, "⚠️ Cockpit", &format!("{label}: {e}"), "space:inbox", &[]);
            }
        }
    }
}

/// One request through the server: the outcome's message, and the notification's title, body and click.
fn carry_out(server: &Server, config: &Config, raw: &Value, request: &Value) -> Result<(String, String, String, String), String> {
    let s = |key: &str| request[key].as_str().unwrap_or_default().to_string();
    let section = &raw[section_of(request["type"].as_str().unwrap_or_default())];
    let dashboard = |preview: &str| {
        let url = config.0["platform"]["previewDashboardUrl"].as_str().unwrap_or_default().replace("{preview}", preview);
        if url.is_empty() { "space:inbox".to_string() } else { url }
    };
    match request["type"].as_str().unwrap_or_default() {
        "rerun" => {
            let mapping = ci::mapping(section);
            mcp::with(server, async |c| ci::rerun(&Mapped { connection: c, section: &mapping }, &s("workflowId")).await)?;
            let name = mapping["name"].as_str().unwrap_or("CI").to_string();
            Ok(("rerun started".into(), format!("🔁 {name}"), format!("Rerunning failed jobs: {}", s("label")), "space:inbox".into()))
        }
        "release" => {
            let mapping = tickets::mapping(section);
            let ticket = s("ticket");
            let state = mcp::with(server, async |c| tickets::complete(&Mapped { connection: c, section: &mapping }, &mapping, &ticket).await)?;
            crate::store::global().update("inbox", |doc| {
                let mut doc = doc.clone();
                if let Some(list) = doc["tickets"].as_array_mut() {
                    list.retain(|t| t["id"] != ticket.as_str());
                }
                doc
            });
            let name = section["name"].as_str().unwrap_or("Tickets").to_string();
            Ok((format!("moved to {state}"), format!("🎫 {name}"), format!("{ticket} moved to {state}"), "space:inbox".into()))
        }
        "redeploy" => {
            let mapping = platform::mapping(section);
            let (preview, app) = (s("preview"), s("app"));
            mcp::with(server, async |c| platform::redeploy_preview(&Mapped { connection: c, section: &mapping }, &preview, &app).await)?;
            Ok(("redeploying".into(), "🧪 Preview".into(), format!("Redeploying {app} on {preview}"), dashboard(&preview)))
        }
        "create-preview" => {
            let mapping = platform::mapping(section);
            let (app, branch) = (s("app"), s("branch"));
            let preview = mcp::with(server, async |c| platform::create_preview(&Mapped { connection: c, section: &mapping }, &mapping, &app, &branch).await)?;
            let message = if preview.is_empty() { "creating".to_string() } else { format!("creating {preview}") };
            let body = format!("Creating {} for {app} {branch}", if preview.is_empty() { "a preview" } else { &preview });
            Ok((message, "🧪 Preview".into(), body, dashboard(&preview)))
        }
        "stop-preview" => {
            let mapping = platform::mapping(section);
            let preview = s("preview");
            mcp::with(server, async |c| platform::stop_preview(&Mapped { connection: c, section: &mapping }, &preview).await)?;
            Ok(("stopped".into(), "🧪 Preview".into(), format!("{preview} stopped; it keeps its setup and extend switches it back on"), "space:inbox".into()))
        }
        "extend" => {
            let mapping = platform::mapping(section);
            let preview = s("preview");
            let days = request["days"].as_u64().unwrap_or(1) as u32;
            mcp::with(server, async |c| platform::extend_preview(&Mapped { connection: c, section: &mapping }, &mapping, &preview, days).await)?;
            Ok((format!("extended by {days} days"), "🧪 Preview".into(), format!("{preview} extended by {days} days and redeploying"), dashboard(&preview)))
        }
        kind @ ("mail-archive" | "mail-read") => {
            let mapping = cockpit_connectors::mail::mapping(section);
            let message = serde_json::json!({"id": s("message")});
            if kind == "mail-archive" {
                mcp::with(server, async |c| cockpit_connectors::mail::archive(&Mapped { connection: c, section: &mapping }, &message).await)?;
            } else {
                mcp::with(server, async |c| cockpit_connectors::mail::mark_read(&Mapped { connection: c, section: &mapping }, &message).await)?;
            }
            Ok(("done".into(), "✉️ Mail".into(), format!("{} {}", if kind == "mail-archive" { "Archived" } else { "Read" }, s("label")), "space:inbox".into()))
        }
        other => Err(format!("no such request {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_are_checked_as_the_plugin_checks_them() {
        assert_eq!(refusal(&json!({"type": "rerun", "label": "api main", "workflowId": "w1"})), None);
        assert!(refusal(&json!({"type": "extend", "label": "x", "preview": "p", "days": "2"})).unwrap().starts_with("extend needs days as a number"));
        assert!(refusal(&json!({"type": "nuke", "label": "x"})).unwrap().starts_with("unknown request type"));
        assert_eq!(section_of("release"), "tickets");
    }
}
