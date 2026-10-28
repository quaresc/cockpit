//! `cockpit search` and `cockpit events`: the palette's search and the week's events as the hub's JSON, for a Claude
//! session's tools. Starts the hub when none runs.

use std::time::Duration;

use serde_json::{Value, json};

use crate::proc::fail;

const SEARCH: &str = "cockpit search <query>: what the palette finds (PRs, tickets, runbooks, views, to-dos, meetings, releases, the week's events, past sessions)";
const EVENTS: &str = "cockpit events [--since 30m|2h|1d|<epoch ms>] [--app inbox|monitor]: what changed in the shared data, newest first";

fn ask(what: &str, request: Value, timeout: Duration) -> Value {
    cockpit_core::hub::request(&request, timeout).unwrap_or_else(|error| fail(&format!("cockpit {what}: {error}")))
}

pub fn search(args: &[String]) {
    let query = args.join(" ");
    if query.trim().is_empty() {
        fail(SEARCH);
    }
    let answer = ask("search", json!({"op": "search", "query": query.trim()}), Duration::from_secs(10));
    println!("{answer}");
}

pub fn events(args: &[String]) {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    let (since, app) = events_args(args, now).unwrap_or_else(|| fail(EVENTS));
    let mut answer = ask("events", json!({"op": "events", "since": since, "app": app, "limit": 500}), Duration::from_secs(5));
    // Each event's local time beside its epoch milliseconds, so a reader needs no conversion.
    for event in answer["events"].as_array_mut().into_iter().flatten() {
        if let Some(at) = event["at"].as_i64().and_then(chrono::DateTime::from_timestamp_millis) {
            event["when"] = Value::from(at.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string());
        }
    }
    println!("{answer}");
}

/// Since when (epoch milliseconds) and which app, from `--since` and `--app`; None when an argument is wrong.
pub fn events_args(args: &[String], now_ms: u64) -> Option<(u64, String)> {
    let (mut since, mut app) = (now_ms.saturating_sub(86_400_000), String::new());
    let mut args = args.iter();
    while let Some(flag) = args.next() {
        let value = args.next()?;
        match flag.as_str() {
            "--since" => since = since_ms(value, now_ms)?,
            "--app" if ["inbox", "monitor"].contains(&value.as_str()) => app = value.clone(),
            _ => return None,
        }
    }
    Some((since, app))
}

/// "30m", "2h", "1d" back from now, or epoch milliseconds as is.
fn since_ms(value: &str, now_ms: u64) -> Option<u64> {
    let unit = match value.chars().last()? {
        'm' => 60_000,
        'h' => 3_600_000,
        'd' => 86_400_000,
        _ => return value.parse::<u64>().ok().filter(|ms| *ms >= 1_000_000_000_000),
    };
    let count: u64 = value[..value.len() - 1].parse().ok()?;
    Some(now_ms.saturating_sub(count.checked_mul(unit)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn since_and_app() {
        let now = 1_761_660_000_000;
        assert_eq!(events_args(&[], now), Some((now - 86_400_000, String::new())));
        assert_eq!(events_args(&args(&["--since", "30m", "--app", "monitor"]), now), Some((now - 1_800_000, "monitor".into())));
        assert_eq!(events_args(&args(&["--since", "2h"]), now), Some((now - 7_200_000, String::new())));
        assert_eq!(events_args(&args(&["--since", "1761000000000"]), now), Some((1_761_000_000_000, String::new())));
        for wrong in [&["--since"][..], &["--since", "soon"], &["--since", "12"], &["--app", "todo"], &["--when", "1d"]] {
            assert_eq!(events_args(&args(wrong), now), None, "{wrong:?}");
        }
    }
}
