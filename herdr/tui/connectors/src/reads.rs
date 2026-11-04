//! What any agent may read through cockpit (its `cockpit_read` and `cockpit_health` tools): a source's read verbs on the
//! server the config names for it, and a service's numbers from the observability server.

use serde_json::{Value, json};

use crate::observability::{self, Signals};
use crate::tools::Mapped;
use crate::{Server, block_on, connect};

/// The read verbs of each source; the others change things and stay with cockpit's own actions.
pub const READS: &[(&str, &[&str])] = &[
    ("tickets", &["listMine", "get", "comments", "listStates", "details"]),
    ("ci", &["listRuns", "listWorkflows", "listJobs", "getJob", "listTests", "logs"]),
    ("calendar", &["listEvents", "getEvent"]),
    ("platform", &["deployStatus", "deployHistory", "listPreviews", "previewStatus"]),
    ("observability", &["metric", "monitors", "issues"]),
    ("mail", &["listInbox", "get"]),
    ("code", &["listMine", "listReviews", "listMerged", "details"]),
];

/// An answer cut to what an agent's context takes.
const MAX_CHARS: usize = 20_000;

/// A section's mapping over its defaults, as the hub reads it.
fn mapping(source: &str, section: &Value) -> Value {
    match source {
        "tickets" => crate::tickets::mapping(section),
        "ci" => crate::ci::mapping(section),
        "calendar" => crate::calendar::mapping(section),
        "platform" => crate::platform::mapping(section),
        "observability" => observability::mapping(section),
        "code" => crate::code::mapping(section),
        "mail" => crate::mail::mapping(section),
        _ => section.clone(),
    }
}

/// The server and mapped section of a source, from the raw config, or why cockpit cannot reach it.
pub fn reach(raw: &Value, source: &str) -> Result<(Server, Value), String> {
    if source == "observability" {
        return observability::server(raw).ok_or_else(|| "no metrics or errors server of cockpit's own (cockpit setup)".into());
    }
    let section = &raw[source];
    let name = section["server"].as_str().filter(|s| !s.is_empty()).ok_or_else(|| format!("no {source} server in cockpit's config"))?;
    let server = Server::from_entry(&raw["servers"][name])
        .ok_or_else(|| format!("{name} is not a server cockpit calls itself (it is an agent connector): use your own tools for it"))?;
    if !server.is_ready() {
        return Err(format!("cockpit is not logged in to {name}: cockpit connector login {name}"));
    }
    Ok((server, mapping(source, section)))
}

/// One read verb's answer, as the server gives it.
pub fn read(raw: &Value, source: &str, verb: &str, values: &Value) -> Result<String, String> {
    let verbs = READS
        .iter()
        .find(|(s, _)| *s == source)
        .map(|(_, v)| *v)
        .ok_or_else(|| format!("source: one of {}", READS.iter().map(|(s, _)| *s).collect::<Vec<_>>().join(", ")))?;
    if !verbs.contains(&verb) {
        return Err(format!("verb: {source} reads with {}", verbs.join(", ")));
    }
    let (server, section) = reach(raw, source)?;
    let answer = block_on(async {
        let c = connect(&server).await?;
        let out = crate::call_mapped(&c, &section, verb, values).await;
        c.close().await;
        out
    })?;
    let text = answer.to_string();
    Ok(if text.chars().count() > MAX_CHARS { format!("{}… (cut)", text.chars().take(MAX_CHARS).collect::<String>()) } else { text })
}

/// A service's errors, hits and p95 over [from, to], its alerting monitors and its open errors of the day.
pub fn health(raw: &Value, service: &str, from: i64, to: i64, now: i64) -> Result<Value, String> {
    let (server, section) = reach(raw, "observability")?;
    let monitor = crate::monitor::settings(&raw["monitor"]);
    block_on(async {
        let c = connect(&server).await?;
        let tools = Mapped { connection: &c, section: &section };
        let signals = if observability::is_datadog(&section, section["server"].as_str().unwrap_or("")) {
            Signals::Datadog(&tools)
        } else {
            Signals::Mapped { tools: &tools, section: &section }
        };
        let numbers = signals.health(&monitor, service, from, to).await;
        let monitors = signals.monitors(&monitor, service, "").await.unwrap_or(Value::Null);
        let issues = signals.issues(&monitor, service, now).await.unwrap_or_default();
        c.close().await;
        let numbers = numbers?.map(|h| json!({"errors": h.errors, "hits": h.hits, "p95Seconds": h.p95}));
        Ok(
            json!({"service": service, "from": crate::monitor::iso(from), "to": crate::monitor::iso(to), "numbers": numbers, "monitors": monitors, "issues": issues}),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_read_verbs_and_reachable_servers() {
        let raw = json!({"tickets": {"server": "claude.ai Linear"}, "servers": {}});
        assert!(read(&raw, "tickets", "setState", &json!({})).unwrap_err().starts_with("verb: tickets reads with"));
        assert!(read(&raw, "nope", "x", &json!({})).unwrap_err().starts_with("source: one of"));
        assert!(read(&raw, "tickets", "get", &json!({})).unwrap_err().contains("agent connector"));
        assert!(reach(&json!({}), "observability").is_err());
    }
}
