//! The monitor space's document: every app of the watched environment from the platform connector, and for those with a
//! release in the last day (or pinned) their deploys, alerting monitors, open errors and a verdict on each release from
//! the numbers before and after it (Datadog's, or any observability tool's through its mapping).

use serde_json::{Value, json};

use crate::mapping::{pick, text};
use crate::observability::Signals;
use crate::tools::Tools;

pub const DAY_MS: i64 = 86_400_000;
const BINS: usize = 20;
/// Monitors come in pages well under what an MCP answer may hold.
const MONITOR_PAGES: usize = 8;
const MONITOR_PAGE_TOKENS: u32 = 8000;
const INTENT: &str = "cockpit monitor space: health of recent releases";

/// What the document is built from besides the tools: the mapped platform section, the monitor settings, and who I am.
pub struct Context<'a> {
    /// The platform section as `platform::mapping` gives it.
    pub platform: &'a Value,
    /// The config's monitor section as `settings` gives it.
    pub monitor: &'a Value,
    /// The config's `deployWatch.afterMinutes`: how long after a release its numbers are compared.
    pub after_minutes: i64,
    /// My GitHub login, empty when unknown.
    pub login: &'a str,
    /// The config's `github.org`, for PR links.
    pub org: &'a str,
    /// The inbox document, for the tickets and PRs each release ships.
    pub inbox: Option<&'a Value>,
    /// The app ids pinned in the monitor space.
    pub pinned: &'a [String],
    /// The app ids muted in the monitor space: never watched, whatever they release.
    pub muted: &'a [String],
    /// Now, in milliseconds.
    pub now: i64,
    /// The local time's offset from UTC, for the times a summary says.
    pub utc_offset_minutes: i32,
}

/// The service as Datadog names it: the monitor section's `serviceReplace` substrings, longest key first.
fn datadog_service(monitor: &Value, service: &str) -> String {
    let mut rules: Vec<(&str, &str)> = monitor["serviceReplace"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(from, to)| Some((from.as_str(), to.as_str()?)))
        .filter(|(from, _)| !from.is_empty())
        .collect();
    rules.sort_by(|(a, _), (b, _)| b.len().cmp(&a.len()).then(a.cmp(b)));
    rules.iter().fold(service.to_string(), |s, (from, to)| s.replace(from, to))
}

/// The app's Datadog link under the name Datadog knows, the platform's service name swapped for the mapped one.
fn datadog_url(url: &str, from: &str, to: &str) -> String {
    if from.is_empty() || from == to { url.to_string() } else { url.replace(from, to) }
}

/// Errors, hits and p95 (seconds) of a service over a window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Health {
    pub errors: f64,
    pub hits: f64,
    pub p95: Option<f64>,
}

impl Health {
    fn to_value(self) -> Value {
        json!({"errors": num(self.errors), "hits": num(self.hits), "p95": self.p95.map(num)})
    }
}

/// When a release counts as moved: the error rate up by `error_points`, or the p95 up by both the ratio and the
/// milliseconds, with at least `min_hits` requests after it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Moved {
    pub error_points: f64,
    pub p95_ratio: f64,
    pub p95_ms: f64,
    pub min_hits: f64,
}

impl Moved {
    pub fn of(monitor: &Value) -> Moved {
        let m = &monitor["moved"];
        let get = |key: &str, default: f64| m[key].as_f64().unwrap_or(default);
        Moved { error_points: get("errorPoints", 0.5), p95_ratio: get("p95Ratio", 1.5), p95_ms: get("p95Ms", 100.0), min_hits: get("minHits", 50.0) }
    }
}

/// A notification a refresh calls for: a verdict that turned bad, or a release of mine that came out fine.
#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    pub title: String,
    pub body: String,
    /// What clicking it opens: `monitor:<app id>`.
    pub action: String,
}

/// The config's monitor section with defaults for missing keys, as the plugin's loadConfig merges it.
pub fn settings(monitor: &Value) -> Value {
    let mut merged = json!({
        "env": "production",
        "queries": {
            "errors": "sum:trace.http.server.request.errors{service:{service},env:{env}}.as_count()",
            "hits": "sum:trace.http.server.request.hits{service:{service},env:{env}}.as_count()",
            "p95": "p95:trace.http.server.request{service:{service},env:{env}}"
        },
        "moved": {"errorPoints": 0.5, "p95Ratio": 1.5, "p95Ms": 100, "minHits": 50},
        "notify": "mine"
    });
    for (key, value) in monitor.as_object().into_iter().flatten() {
        match (key.as_str(), value.as_object(), merged[key].as_object_mut()) {
            ("queries" | "moved", Some(over), Some(base)) => base.extend(over.clone()),
            _ => merged[key] = value.clone(),
        }
    }
    merged
}

// ---------------------------------------------------------------------------
// Datadog's answers

/// The JSON a Datadog tool answers, inside its `<JSON_DATA>` block or as the whole text; an empty block is `[]`.
pub fn data_of(body: &str) -> Result<Value, String> {
    let inner = body.split_once("<JSON_DATA>").and_then(|(_, rest)| rest.split_once("</JSON_DATA>")).map_or(body, |(inner, _)| inner);
    if inner.trim().is_empty() {
        return Ok(json!([]));
    }
    serde_json::from_str(inner.trim()).map_err(|_| format!("Datadog answered no JSON: {}", body.chars().take(160).collect::<String>()))
}

/// A YAML scalar's text: quotes taken off, `''` as one quote.
fn unquote(value: &str) -> String {
    let v = value.trim();
    if v.starts_with('\'') && v.ends_with('\'') {
        return v.get(1..v.len().saturating_sub(1)).unwrap_or("").replace("''", "'");
    }
    if v.starts_with('"') && v.ends_with('"') {
        return serde_json::from_str(v).unwrap_or_else(|_| v.to_string());
    }
    v.to_string()
}

/// A `- key: value` or `  key: value` line: whether it starts an item, its key and its value.
fn field_line(line: &str) -> Option<(bool, &str, &str)> {
    let (is_item, rest) = match line.strip_prefix("- ") {
        Some(rest) => (true, rest),
        None => (false, line.strip_prefix("  ")?),
    };
    let key_len = rest.bytes().take_while(|b| b.is_ascii_lowercase() || *b == b'_').count();
    let after = rest.get(key_len..)?.strip_prefix(':').filter(|_| key_len > 0)?;
    let value = match after.chars().next() {
        Some(c) if c.is_whitespace() => &after[c.len_utf8()..],
        _ => after,
    };
    (!value.contains(['\r', '\u{2028}', '\u{2029}'])).then_some((is_item, &rest[..key_len], value))
}

/// The Error Tracking issues of a search, which answers a YAML list: the fields cockpit shows, a message to its first
/// line, new when first seen since `since` (ms).
pub fn parse_issues(body: &str, since: i64) -> Vec<Value> {
    let mut items: Vec<serde_json::Map<String, Value>> = Vec::new();
    let mut block: Option<(String, Vec<String>)> = None;
    let close = |items: &mut Vec<serde_json::Map<String, Value>>, block: Option<(String, Vec<String>)>| {
        if let (Some((key, lines)), Some(last)) = (block, items.last_mut()) {
            last.insert(key, Value::String(lines.join("\n")));
        }
    };
    for line in body.split('\n') {
        if let Some((_, lines)) = block.as_mut().filter(|_| line.starts_with("    ") || line.trim().is_empty()) {
            lines.push(line.trim().to_string());
            continue;
        }
        close(&mut items, block.take());
        let Some((is_item, key, value)) = field_line(line) else { continue };
        if is_item {
            items.push(serde_json::Map::new());
        }
        if value == "|" || value == "|-" {
            block = Some((key.to_string(), Vec::new()));
        } else if let Some(last) = items.last_mut() {
            last.insert(key.to_string(), Value::String(unquote(value)));
        }
    }
    close(&mut items, block);
    items
        .iter()
        .filter(|i| i.get("issue_id").and_then(Value::as_str).is_some_and(|id| !id.is_empty()))
        .map(|i| {
            let get = |key: &str| i.get(key).and_then(Value::as_str).unwrap_or("");
            let error_type = get("error_type");
            let first_seen = get("first_seen");
            json!({
                "id": get("issue_id"),
                "type": error_type.strip_prefix("Elixir.").unwrap_or(error_type),
                "message": get("error_message").split('\n').find(|l| !l.trim().is_empty()).unwrap_or(""),
                "count": number(i.get("total_count").and_then(Value::as_str)),
                "firstSeen": first_seen,
                "firstVersion": get("first_seen_version"),
                "url": get("issue_url"),
                "isNew": parse_ms(first_seen).is_some_and(|t| t >= since),
            })
        })
        .collect()
}

/// A timeseries answer as `n` values over [from, to): a bin's sum for a count, its average otherwise.
pub fn bins(series: Option<&Value>, from: i64, to: i64, is_count: bool, n: usize) -> Vec<f64> {
    let mut out = vec![0.0; n];
    if n == 0 {
        return out;
    }
    let width = (to - from) as f64 / n as f64;
    for b in series.and_then(|s| s["binned"].as_array()).into_iter().flatten() {
        let Some(start) = b["start_time"].as_str().and_then(parse_ms) else { continue };
        let i = (((start - from) as f64 / width).floor()).clamp(0.0, (n - 1) as f64) as usize;
        let avg = b["avg"].as_f64().unwrap_or(0.0);
        let add = if is_count { avg * b["count"].as_f64().unwrap_or(0.0) } else { avg };
        out[i] = ((out[i] + add) * 1e6 + 0.5).floor() / 1e6;
    }
    out
}

fn rate(h: &Health) -> f64 {
    if h.hits > 0.0 { 100.0 * h.errors / h.hits } else { 0.0 }
}

fn pct(n: f64) -> String {
    if n < 1.0 { format!("{n:.2}%") } else { format!("{n:.1}%") }
}

fn millis(seconds: Option<f64>) -> String {
    seconds.map_or("?".to_string(), |s| format!("{}", (s * 1000.0 + 0.5).floor() as i64))
}

/// A release's verdict from its numbers: failed when its deploy did, watching until the window ends, else moved when
/// the error rate, the p95 or a new error says so, ok, or unknown without traffic.
#[allow(clippy::too_many_arguments)]
pub fn verdict_of(
    phase: &str,
    before: Option<Health>,
    after: Option<Health>,
    new_issues: &[String],
    until: &str,
    now: i64,
    moved: &Moved,
    utc_offset_minutes: i32,
) -> Value {
    let verdict = |state: &str, summary: String| {
        json!({
            "state": state, "summary": summary, "before": before.map(Health::to_value), "after": after.map(Health::to_value),
            "newIssues": new_issues, "until": until
        })
    };
    let lower = phase.to_lowercase();
    if lower.contains("fail") || lower.contains("error") {
        return verdict("failed", format!("deploy {lower}"));
    }
    let mut parts = Vec::new();
    if let (Some(b), Some(a)) = (&before, &after) {
        parts.push(format!("errors {} → {}", pct(rate(b)), pct(rate(a))));
        if b.p95.is_some() || a.p95.is_some() {
            parts.push(format!("p95 {} → {} ms", millis(b.p95), millis(a.p95)));
        }
    }
    if !new_issues.is_empty() {
        parts.push(format!("{} new error{}", new_issues.len(), if new_issues.len() > 1 { "s" } else { "" }));
    }
    if parse_ms(until).is_some_and(|u| now < u) {
        let head = format!("watching until {}", hhmm(until, utc_offset_minutes));
        return verdict("watching", std::iter::once(head).chain(parts).collect::<Vec<_>>().join(" · "));
    }
    let (Some(b), Some(a)) = (before, after) else {
        return verdict("unknown", if parts.is_empty() { "no traffic to compare".into() } else { parts.join(" · ") });
    };
    if b.hits == 0.0 && a.hits == 0.0 {
        return verdict("unknown", if parts.is_empty() { "no traffic to compare".into() } else { parts.join(" · ") });
    }
    let has_traffic = a.hits >= moved.min_hits;
    let errors_up = has_traffic && rate(&a) - rate(&b) >= moved.error_points;
    let slower = has_traffic
        && match (b.p95, a.p95) {
            (Some(bp), Some(ap)) => bp > 0.0 && ap / bp >= moved.p95_ratio && (ap - bp) * 1000.0 >= moved.p95_ms,
            _ => false,
        };
    let state = if errors_up || slower || !new_issues.is_empty() { "moved" } else { "ok" };
    verdict(state, parts.join(" · "))
}

// ---------------------------------------------------------------------------
// Fetching

/// One Datadog tool's answer text.
async fn datadog(tools: &impl Tools, tool: &str, mut args: Value) -> Result<String, String> {
    args["telemetry"] = json!({"intent": INTENT});
    tools.text(tool, &args).await.map_err(|e| format!("Datadog {e}"))
}

fn fill(query: &str, service: &str, env: &str) -> String {
    query.replace("{service}", service).replace("{env}", env)
}

fn query(monitor: &Value, name: &str, service: &str) -> String {
    fill(monitor["queries"][name].as_str().unwrap_or(""), service, monitor["env"].as_str().unwrap_or(""))
}

/// Errors, hits and p95 of a service over [from, to]; none when Datadog has neither errors nor hits.
pub async fn health(datadog_tools: &impl Tools, monitor: &Value, service: &str, from: i64, to: i64) -> Result<Option<Health>, String> {
    let queries = [("errors", "sum"), ("hits", "sum"), ("p95", "avg")].map(|(name, aggregator)| (name, query(monitor, name, service), aggregator));
    let args = json!({
        "queries": queries.iter().map(|(name, q, aggregator)| json!({"name": name, "query": q, "aggregator": aggregator})).collect::<Vec<_>>(),
        "response_format": "scalar", "from": iso(from), "to": iso(to)
    });
    let data = data_of(&datadog(datadog_tools, "get_datadog_metric", args).await?)?;
    let values = data.as_array().and_then(|a| a.first()).map(|v| &v["values"]).unwrap_or(&Value::Null);
    let value = |i: usize| values[&queries[i].1].as_f64().or_else(|| values[queries[i].0].as_f64());
    if value(1).is_none() && value(0).is_none() {
        return Ok(None);
    }
    Ok(Some(Health { errors: value(0).unwrap_or(0.0), hits: value(1).unwrap_or(0.0), p95: value(2) }))
}

/// A service's monitors by status, and the alerting ones with their link on `site`.
pub async fn monitors_of(datadog_tools: &impl Tools, service: &str, site: &str) -> Result<Value, String> {
    let mut found: Vec<Value> = Vec::new();
    for _ in 0..MONITOR_PAGES {
        let args = json!({"query": format!("service:{service}"), "start_at": found.len(), "max_tokens": MONITOR_PAGE_TOKENS});
        let body = datadog(datadog_tools, "search_datadog_monitors", args).await?;
        let batch = data_of(&body)?.as_array().cloned().unwrap_or_default();
        let is_last = batch.is_empty() || !body.contains("<is_truncated>true</is_truncated>");
        found.extend(batch);
        if is_last {
            break;
        }
    }
    let status = |m: &Value| m["status"].as_str().unwrap_or("").to_lowercase();
    let is = |s: &str| found.iter().filter(|m| status(m) == s).count();
    let alerting: Vec<Value> = found
        .iter()
        .filter(|m| ["alert", "warn"].contains(&status(m).as_str()))
        .map(|m| {
            let id = text(Some(&m["id"])).unwrap_or_default();
            json!({"id": m["id"], "name": m["name"], "status": m["status"], "url": format!("{site}/monitors/{id}")})
        })
        .collect();
    Ok(json!({"ok": is("ok"), "warn": is("warn"), "alert": is("alert"), "nodata": is("no data"), "alerting": alerting}))
}

/// A day of a service's errors and p95, in `BINS` values each.
pub async fn series_of(datadog_tools: &impl Tools, monitor: &Value, service: &str, now: i64) -> Result<Value, String> {
    let from = now - DAY_MS;
    let args = json!({"queries": [query(monitor, "errors", service), query(monitor, "p95", service)], "from": iso(from), "to": iso(now)});
    let data = data_of(&datadog(datadog_tools, "get_datadog_metric", args).await?)?;
    let series = data.as_array().cloned().unwrap_or_default();
    let values = |v: Vec<f64>| v.into_iter().map(num).collect::<Vec<_>>();
    Ok(json!({
        "errors": values(bins(series.first(), from, now, true, BINS)),
        "p95": values(bins(series.get(1), from, now, false, BINS)),
    }))
}

/// A service's open Error Tracking issues of the last day, most frequent first, at most eight.
pub async fn issues_of(datadog_tools: &impl Tools, monitor: &Value, service: &str, now: i64) -> Result<Vec<Value>, String> {
    let env = monitor["env"].as_str().unwrap_or("");
    let args = json!({
        "query": format!("service:{service} env:{env}"), "from": "now-1d", "states": ["FOR_REVIEW"],
        "includes": ["timing", "total_count", "issue_url"], "order_by": "TOTAL_COUNT", "max_tokens": 4000
    });
    let body = datadog(datadog_tools, "search_datadog_error_tracking_issues", args).await?;
    Ok(parse_issues(&body, now - DAY_MS).into_iter().take(8).collect())
}

/// The repo's name from its URL: https://github.com/acme/api(.git) is api.
fn repo_name(url: &str) -> String {
    url.strip_suffix(".git").unwrap_or(url).rsplit('/').next().unwrap_or("").to_string()
}

fn platform_field<'a>(ctx: &'a Context, name: &str, default: &'a str) -> &'a str {
    ctx.platform["fields"][name].as_str().unwrap_or(default)
}

fn list<'a>(value: &'a Value, path: &str) -> Vec<&'a Value> {
    pick(value, path).and_then(Value::as_array).map(|a| a.iter().collect()).unwrap_or_default()
}

/// An app's recent deploys, newest first, as releases: mine when I started it or it ships one of my tickets.
pub async fn releases_of(platform: &impl Tools, ctx: &Context<'_>, app_id: &str, repo: &str) -> Result<Vec<Value>, String> {
    let f = |name: &str, default: &'static str| platform_field(ctx, name, default);
    let answer = platform.call("deployHistory", &json!({"appId": app_id})).await?;
    let deploys = ctx.inbox.and_then(|i| i["deploys"].as_object());
    let inbox_tickets = ctx.inbox.and_then(|i| i["tickets"].as_array());
    let releases = list(&answer, f("deployments", ""))
        .into_iter()
        .map(|d| {
            let tag = text(pick(d, f("deployTag", ""))).unwrap_or_default();
            let actor_path = text(pick(d, f("deployActor", "actor"))).unwrap_or_default();
            let actor = actor_path.rsplit('/').next().unwrap_or("").to_string();
            let ships = |p: &Value| p["repo"] == repo && p["tag"] == tag.as_str();
            let shipped: Vec<(&String, &Vec<Value>)> =
                deploys.into_iter().flatten().filter_map(|(id, prs)| Some((id, prs.as_array()?))).filter(|(_, prs)| prs.iter().any(ships)).collect();
            let tickets: Vec<Value> = shipped
                .iter()
                .map(|(id, _)| {
                    let url = inbox_tickets.into_iter().flatten().find(|t| t["id"] == id.as_str()).and_then(|t| t["url"].as_str()).unwrap_or("");
                    json!({"id": id, "url": url})
                })
                .collect();
            let prs: Vec<Value> = shipped
                .iter()
                .flat_map(|(_, prs)| prs.iter().filter(|p| ships(p)))
                .map(|p| {
                    let number = text(Some(&p["number"])).unwrap_or_default();
                    let url = if ctx.org.is_empty() {
                        String::new()
                    } else {
                        format!("https://github.com/{}/{}/pull/{number}", ctx.org, p["repo"].as_str().unwrap_or(""))
                    };
                    json!({"repo": p["repo"], "number": p["number"], "url": url})
                })
                .collect();
            json!({
                "id": number(text(pick(d, f("deployId", "id"))).as_deref()),
                "tag": tag,
                "isMine": (!ctx.login.is_empty() && actor == ctx.login) || !tickets.is_empty(),
                "actor": actor,
                "phase": text(pick(d, f("deployPhase", "phase"))).unwrap_or_default(),
                "startedAt": text(pick(d, f("deployStarted", "startedAt"))).unwrap_or_default(),
                "finishedAt": text(pick(d, f("deployFinished", "finishedAt"))),
                "tickets": tickets,
                "prs": prs,
                "verdict": null,
            })
        })
        .enumerate()
        .filter(|(i, r)| *i == 0 || r["startedAt"].as_str().and_then(parse_ms).is_some_and(|t| ctx.now - t < DAY_MS))
        .map(|(_, r)| r)
        .take(5)
        .collect();
    Ok(releases)
}

fn failed(errors: &mut Vec<String>, app: &str, label: &str, error: &str) {
    errors.push(format!("{app} {label}: {}", error.chars().take(120).collect::<String>()));
}

/// One app's details: its releases with their verdicts (a settled one kept from the last round), monitors, errors and a
/// day of numbers. A source that fails leaves its part as it was and says so in `errors`.
pub async fn watch<D: Tools>(
    platform: &impl Tools,
    signals: &Signals<'_, D>,
    app: &mut Value,
    previous: Option<&Value>,
    ctx: &Context<'_>,
    errors: &mut Vec<String>,
) {
    let name = app["name"].as_str().unwrap_or("").to_string();
    let site = app["datadogUrl"]
        .as_str()
        .and_then(|u| u.strip_prefix("https://"))
        .and_then(|rest| rest.split('/').next())
        .filter(|host| !host.is_empty())
        .map_or("https://app.datadoghq.com".to_string(), |host| format!("https://{host}"));
    let window = ctx.after_minutes * 60_000;
    let kept = |key: &str, fallback: Value| previous.map(|p| p[key].clone()).filter(|v| !v.is_null()).unwrap_or(fallback);
    let (id, repo) = (app["id"].as_str().unwrap_or("").to_string(), app["repo"].as_str().unwrap_or("").to_string());
    app["releases"] = match releases_of(platform, ctx, &id, &repo).await {
        Ok(releases) => Value::Array(releases),
        Err(e) => {
            failed(errors, &name, "deploys", &e);
            kept("releases", json!([]))
        }
    };
    let service = app["service"].as_str().unwrap_or("").to_string();
    if service.is_empty() {
        return;
    }
    app["monitors"] = match signals.monitors(ctx.monitor, &service, &site).await {
        Ok(monitors) => monitors,
        Err(e) => {
            failed(errors, &name, "monitors", &e);
            kept("monitors", Value::Null)
        }
    };
    app["series"] = match signals.day(ctx.monitor, &service, ctx.now).await {
        Ok(series) => series,
        Err(e) => {
            failed(errors, &name, "series", &e);
            kept("series", Value::Null)
        }
    };
    app["issues"] = match signals.issues(ctx.monitor, &service, ctx.now).await {
        Ok(issues) => Value::Array(issues),
        Err(e) => {
            failed(errors, &name, "errors", &e);
            kept("issues", json!([]))
        }
    };
    let issues = app["issues"].as_array().cloned().unwrap_or_default();
    let moved = Moved::of(ctx.monitor);
    let Some(releases) = app["releases"].as_array_mut() else { return };
    for release in releases {
        let settled = previous
            .and_then(|p| p["releases"].as_array())
            .and_then(|rs| rs.iter().find(|r| r["id"] == release["id"]))
            .map(|r| &r["verdict"])
            .filter(|v| v.is_object() && v["state"] != "watching");
        if let Some(settled) = settled {
            release["verdict"] = settled.clone();
            continue;
        }
        let Some(start) = release["startedAt"].as_str().and_then(parse_ms) else { continue };
        let end = release["finishedAt"].as_str().and_then(parse_ms).filter(|&t| t != 0).unwrap_or(start);
        let until = iso(end + window);
        let before = signals.health(ctx.monitor, &service, start - window, start).await.ok().flatten();
        let after = if ctx.now > end { signals.health(ctx.monitor, &service, end, ctx.now.min(end + window)).await.ok().flatten() } else { None };
        let tag = release["tag"].as_str().unwrap_or("");
        let fresh: Vec<String> = issues
            .iter()
            .filter(|i| !tag.is_empty() && i["firstVersion"].as_str().is_some_and(|v| v.ends_with(tag)))
            .filter_map(|i| i["id"].as_str().map(str::to_string))
            .collect();
        let phase = release["phase"].as_str().unwrap_or("").to_string();
        release["verdict"] = verdict_of(&phase, before, after, &fresh, &until, ctx.now, &moved, ctx.utc_offset_minutes);
    }
}

/// The new monitor document (`MonitorState`): every app the platform deploys, the watched ones in detail, most recent
/// deploy first. None when the platform maps no `deployStatus` or `deployHistory`.
pub async fn refresh<D: Tools>(platform: &impl Tools, signals: &Signals<'_, D>, ctx: &Context<'_>, previous: Option<&Value>) -> Result<Option<Value>, String> {
    if !platform.has("deployStatus") || !platform.has("deployHistory") {
        return Ok(None);
    }
    let f = |name: &str, default: &'static str| platform_field(ctx, name, default);
    let skip: Vec<&str> = ctx.platform["fields"]["skipKinds"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    let status = platform.call("deployStatus", &json!({})).await?;
    let mut apps: Vec<Value> = list(&status, f("apps", ""))
        .into_iter()
        .filter(|a| !skip.contains(&text(pick(a, f("appKind", ""))).unwrap_or_default().as_str()))
        .map(|a| {
            let get = |name: &str, default: &'static str| text(pick(a, f(name, default)));
            let id = get("appId", "").unwrap_or_default();
            let deployed_at = get("appDeployedAt", "lastDeployedAt").unwrap_or_default();
            let pinned = ctx.pinned.contains(&id);
            let muted = ctx.muted.contains(&id);
            let name = get("appName", "name").unwrap_or_else(|| id.clone());
            let platform_service = get("appService", "service").unwrap_or_default();
            let service = datadog_service(ctx.monitor, &platform_service);
            let datadog_link = datadog_url(&get("appDatadog", "datadogUrl").unwrap_or_default(), &platform_service, &service);
            json!({
                "id": id,
                "name": name,
                "service": service,
                "repo": repo_name(&get("appRepo", "").unwrap_or_default()),
                "health": get("appHealth", "health").unwrap_or_default(),
                "tag": get("appTag", "").or_else(|| get("appSha", "")).unwrap_or_default(),
                "deployedAt": deployed_at,
                "behind": number(get("appBehind", "commitsBehindHead").as_deref()),
                "argoUrl": get("appLink", "").unwrap_or_default(),
                "datadogUrl": datadog_link,
                "watched": !muted && (pinned || parse_ms(&deployed_at).is_some_and(|t| ctx.now - t < DAY_MS)),
                "pinned": pinned,
                "muted": muted,
                "monitors": null,
                "series": null,
                "issues": [],
                "releases": [],
            })
        })
        .collect();
    let mut errors = Vec::new();
    for app in apps.iter_mut().filter(|a| a["watched"] == true) {
        let before = previous.and_then(|p| p["apps"].as_array()).and_then(|ps| ps.iter().find(|p| p["id"] == app["id"]));
        watch(platform, signals, app, before, ctx, &mut errors).await;
    }
    apps.sort_by_key(|a| std::cmp::Reverse(a["deployedAt"].as_str().and_then(parse_ms).unwrap_or(0)));
    Ok(Some(json!({"version": 1, "at": ctx.now, "env": ctx.monitor["env"], "errors": errors, "apps": apps})))
}

/// What a refresh announces: each verdict that turned moved or failed, and each release of mine that turned ok, as
/// `notify` (mine, all or none) allows; `before` is the last document, null when there is none.
pub fn changes(before: &Value, after: &Value, notify: &str) -> Vec<Notice> {
    let mut out = Vec::new();
    for app in after["apps"].as_array().into_iter().flatten() {
        let was_app = before["apps"].as_array().and_then(|apps| apps.iter().find(|a| a["id"] == app["id"]));
        for release in app["releases"].as_array().into_iter().flatten() {
            let was = was_app.and_then(|a| a["releases"].as_array()).and_then(|rs| rs.iter().find(|r| r["id"] == release["id"]));
            let (Some(was), Some(state)) = (was.and_then(|r| r["verdict"]["state"].as_str()), release["verdict"]["state"].as_str()) else { continue };
            if state == was {
                continue;
            }
            let is_mine = release["isMine"] == true;
            let is_bad = state == "moved" || state == "failed";
            let wanted = notify == "all" || (notify == "mine" && is_mine);
            if !wanted || (!is_bad && !(state == "ok" && is_mine)) {
                continue;
            }
            let summary = release["verdict"]["summary"].as_str().filter(|s| !s.is_empty()).unwrap_or(state);
            out.push(Notice {
                title: format!("{} {} {}", if is_bad { "⚠️" } else { "✅" }, app["name"].as_str().unwrap_or(""), release["tag"].as_str().unwrap_or("")),
                body: summary.to_string(),
                action: format!("monitor:{}", app["id"].as_str().unwrap_or("")),
            });
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Numbers and times

/// A number as JSON writes it: whole numbers without a fraction, NaN as null.
fn num(x: f64) -> Value {
    if x.is_finite() && x.fract() == 0.0 && x.abs() < 9e15 { json!(x as i64) } else { serde_json::Number::from_f64(x).map_or(Value::Null, Value::Number) }
}

/// A text as a number, as JavaScript's `Number`: none or blank is 0, anything else not a number is null.
fn number(text: Option<&str>) -> Value {
    match text.map(str::trim) {
        None | Some("") => json!(0),
        Some(t) => num(t.parse::<f64>().unwrap_or(f64::NAN)),
    }
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// An ISO 8601 date or date-time in milliseconds since the epoch, UTC when it names no offset.
pub fn parse_ms(s: &str) -> Option<i64> {
    let s = s.trim();
    if !s.is_ascii() || s.len() < 10 {
        return None;
    }
    let digits = |range: std::ops::Range<usize>| -> Option<i64> {
        let part = s.get(range)?;
        part.bytes().all(|b| b.is_ascii_digit()).then(|| part.parse().ok())?
    };
    let (b, len) = (s.as_bytes(), s.len());
    if b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let (y, mo, d) = (digits(0..4)?, digits(5..7)?, digits(8..10)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    let mut ms = days_from_civil(y, mo, d) * DAY_MS;
    if len == 10 {
        return Some(ms);
    }
    if !matches!(b[10], b'T' | b't' | b' ') || len < 16 || b[13] != b':' {
        return None;
    }
    let (h, mi) = (digits(11..13)?, digits(14..16)?);
    let mut at = 16;
    let mut sec = 0;
    if b.get(at) == Some(&b':') {
        sec = digits(at + 1..at + 3)?;
        at += 3;
    }
    let mut frac = 0;
    if b.get(at) == Some(&b'.') {
        let n = b[at + 1..].iter().take_while(|c| c.is_ascii_digit()).count();
        let millis = format!("{:0<3}", &s[at + 1..at + 1 + n.min(3)]);
        frac = millis.parse::<i64>().ok()?;
        at += 1 + n;
    }
    if h > 24 || mi > 59 || sec > 59 {
        return None;
    }
    ms += ((h * 60 + mi) * 60 + sec) * 1000 + frac;
    let offset = match &s[at..] {
        "" | "Z" | "z" => 0,
        zone => {
            let sign = match zone.as_bytes()[0] {
                b'+' => 1,
                b'-' => -1,
                _ => return None,
            };
            let rest = zone[1..].replace(':', "");
            if (rest.len() != 2 && rest.len() != 4) || !rest.bytes().all(|c| c.is_ascii_digit()) {
                return None;
            }
            let minutes = rest[..2].parse::<i64>().ok()? * 60 + rest.get(2..).filter(|m| !m.is_empty()).map_or(Some(0), |m| m.parse().ok())?;
            sign * minutes
        }
    };
    Some(ms - offset * 60_000)
}

/// Milliseconds since the epoch as JavaScript's `toISOString` writes them.
pub fn iso(ms: i64) -> String {
    let (days, rest) = (ms.div_euclid(DAY_MS), ms.rem_euclid(DAY_MS));
    let (y, m, d) = civil_from_days(days);
    let (h, mi, s, milli) = (rest / 3_600_000, rest / 60_000 % 60, rest / 1000 % 60, rest % 1000);
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{milli:03}Z")
}

/// A date-time's local hour and minute.
fn hhmm(at: &str, utc_offset_minutes: i32) -> String {
    let Some(ms) = parse_ms(at) else { return String::new() };
    iso(ms + i64::from(utc_offset_minutes) * 60_000)[11..16].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_on;
    use crate::tools::Fake;

    const MOVED: Moved = Moved { error_points: 0.5, p95_ratio: 1.5, p95_ms: 100.0, min_hits: 50.0 };

    fn at(s: &str) -> i64 {
        parse_ms(s).unwrap()
    }

    fn h(errors: f64, hits: f64, p95: Option<f64>) -> Option<Health> {
        Some(Health { errors, hits, p95 })
    }

    #[test]
    fn times_read_and_write_as_javascript_does() {
        assert_eq!(at("1970-01-01T00:00:00Z"), 0);
        assert_eq!(at("2026-11-04T12:00:00.250Z"), 1_793_793_600_250);
        assert_eq!(at("2026-11-04T13:00:00+01:00"), at("2026-11-04T12:00:00Z"));
        assert_eq!(at("2026-11-04"), at("2026-11-04T00:00:00Z"));
        assert_eq!(iso(1_793_793_600_250), "2026-11-04T12:00:00.250Z");
        assert_eq!(iso(at("2024-02-29T23:59:59Z")), "2024-02-29T23:59:59.000Z");
        assert_eq!(parse_ms("yesterday"), None);
        assert_eq!(hhmm("2026-11-04T09:35:00.000Z", 60), "10:35");
    }

    #[test]
    fn data_is_the_json_data_block_or_the_whole_text() {
        assert_eq!(data_of("Metrics:\n<JSON_DATA>\n[{\"a\": 1}]\n</JSON_DATA>\n<is_truncated>false</is_truncated>").unwrap(), json!([{"a": 1}]));
        assert_eq!(data_of("<JSON_DATA>\n</JSON_DATA>").unwrap(), json!([]));
        assert_eq!(data_of("{\"b\": 2}").unwrap(), json!({"b": 2}));
        assert!(data_of("<JSON_DATA>oops</JSON_DATA>").is_err());
    }

    #[test]
    fn bins_sum_counts_and_average_the_rest() {
        let from = at("2026-11-04T00:00:00Z");
        let to = from + 20 * 3_600_000;
        let series = json!({"binned": [
            {"start_time": "2026-11-04T00:10:00Z", "count": 3, "avg": 2.0},
            {"start_time": "2026-11-04T00:40:00Z", "count": 1, "avg": 1.5},
            {"start_time": "2026-11-04T05:00:00Z", "count": 2, "avg": 0.25},
            {"start_time": "2026-11-05T09:00:00Z", "count": 1, "avg": 9.0},
            {"start_time": "garbage", "count": 1, "avg": 100.0}
        ]});
        let counts = bins(Some(&series), from, to, true, 20);
        assert_eq!((counts[0], counts[5], counts[19]), (7.5, 0.5, 9.0));
        assert_eq!(counts.iter().sum::<f64>(), 17.0);
        let averages = bins(Some(&series), from, to, false, 20);
        assert_eq!((averages[0], averages[5]), (3.5, 0.25));
        assert_eq!(bins(None, from, to, true, 20), vec![0.0; 20]);
    }

    const ISSUES: &str = "\
- issue_id: 3f1c2a9e-0b7d-11f0-9a2b-da7ad0900002
  error_type: Elixir.DBConnection.ConnectionError
  error_message: |
    connection not available and request was dropped from queue after 2000ms

    second line
  total_count: 42
  first_seen: '2026-11-04T09:12:00Z'
  first_seen_version: sourcing@v1.5.0
  issue_url: https://app.datadoghq.eu/error-tracking/issue/3f1c2a9e
- issue_id: 77aa
  error_type: \"Elixir.KeyError\"
  error_message: 'key :id not found in: %{name: ''x''}'
  total_count: 3
  first_seen: '2026-10-01T00:00:00Z'
  first_seen_version: sourcing@v1.2.0
  issue_url: https://app.datadoghq.eu/error-tracking/issue/77aa
- error_type: NoId
";

    #[test]
    fn issues_read_from_the_yaml_list() {
        let issues = parse_issues(ISSUES, at("2026-11-03T12:00:00Z"));
        assert_eq!(issues.len(), 2);
        assert_eq!(
            issues[0],
            json!({
                "id": "3f1c2a9e-0b7d-11f0-9a2b-da7ad0900002", "type": "DBConnection.ConnectionError",
                "message": "connection not available and request was dropped from queue after 2000ms", "count": 42,
                "firstSeen": "2026-11-04T09:12:00Z", "firstVersion": "sourcing@v1.5.0",
                "url": "https://app.datadoghq.eu/error-tracking/issue/3f1c2a9e", "isNew": true
            })
        );
        assert_eq!(issues[1]["type"], "KeyError");
        assert_eq!(issues[1]["message"], "key :id not found in: %{name: 'x'}");
        assert_eq!(issues[1]["isNew"], false);
    }

    #[test]
    fn verdicts() {
        let now = at("2026-11-04T12:00:00Z");
        let until = "2026-11-04T09:35:00.000Z";
        let failed = verdict_of("Failed", h(1.0, 100.0, None), None, &[], until, now, &MOVED, 0);
        assert_eq!((failed["state"].as_str(), failed["summary"].as_str()), (Some("failed"), Some("deploy failed")));

        let ok = verdict_of("Succeeded", h(2.0, 1000.0, Some(0.2)), h(3.0, 1000.0, Some(0.22)), &[], until, now, &MOVED, 0);
        assert_eq!(ok["state"], "ok");
        assert_eq!(ok["summary"], "errors 0.20% → 0.30% · p95 200 → 220 ms");
        assert_eq!(ok["before"], json!({"errors": 2, "hits": 1000, "p95": 0.2}));

        let errors_up = verdict_of("Succeeded", h(2.0, 1000.0, Some(0.2)), h(30.0, 1000.0, Some(0.21)), &[], until, now, &MOVED, 0);
        assert_eq!((errors_up["state"].as_str(), errors_up["summary"].as_str()), (Some("moved"), Some("errors 0.20% → 3.0% · p95 200 → 210 ms")));
        let slower = verdict_of("Succeeded", h(0.0, 1000.0, Some(0.2)), h(0.0, 1000.0, Some(0.35)), &[], until, now, &MOVED, 0);
        assert_eq!(slower["state"], "moved");
        let quiet = verdict_of("Succeeded", h(0.0, 10.0, Some(0.2)), h(5.0, 10.0, Some(0.9)), &[], until, now, &MOVED, 0);
        assert_eq!(quiet["state"], "ok", "too few hits after to say");
        let new_error = verdict_of("Succeeded", h(0.0, 10.0, None), h(0.0, 10.0, None), &["a".into()], until, now, &MOVED, 0);
        assert_eq!((new_error["state"].as_str(), new_error["summary"].as_str()), (Some("moved"), Some("errors 0.00% → 0.00% · 1 new error")));

        let watching = verdict_of("Running", h(2.0, 1000.0, None), None, &[], until, at("2026-11-04T09:20:00Z"), &MOVED, 60);
        assert_eq!((watching["state"].as_str(), watching["summary"].as_str()), (Some("watching"), Some("watching until 10:35")));
        let unknown = verdict_of("Succeeded", None, None, &[], until, now, &MOVED, 0);
        assert_eq!((unknown["state"].as_str(), unknown["summary"].as_str(), &unknown["after"]), (Some("unknown"), Some("no traffic to compare"), &Value::Null));
    }

    #[test]
    fn settings_fill_defaults_and_merge_nested_blocks() {
        let s = settings(&json!({"env": "staging", "moved": {"minHits": 10}, "notify": "all"}));
        assert_eq!((s["env"].as_str(), s["notify"].as_str()), (Some("staging"), Some("all")));
        assert_eq!(Moved::of(&s), Moved { min_hits: 10.0, ..MOVED });
        assert!(s["queries"]["p95"].as_str().unwrap().starts_with("p95:"));
    }

    fn metric(values: Value) -> String {
        format!("Metric values:\n<JSON_DATA>\n{}\n</JSON_DATA>", json!([{"values": values}]))
    }

    #[test]
    fn builds_the_monitor_document() {
        let platform_section = crate::platform::mapping(&json!({
            "tools": {"deployStatus": {"tool": "ship_get-deploy-status"}, "deployHistory": {"tool": "ship_list-deployments", "args": {"app": "{appId}"}}},
            "fields": {"apps": "apps", "appId": "id", "appRepo": "repository", "appLink": "argoUrl", "appTag": "tag", "appKind": "kind",
                       "skipKinds": ["cronjob"], "deployments": "deployments", "deployTag": "tag", "deployActor": "triggeredBy"}
        }));
        let status = json!({"apps": [
            {"id": "old", "name": "Old", "repository": "https://github.com/acme/old", "tag": "v9", "lastDeployedAt": "2026-11-01T10:00:00Z", "kind": "web"},
            {"id": "sourcing", "name": "Sourcing", "service": "sourcing", "repository": "https://github.com/acme/sourcing.git", "tag": "v1.5.0",
             "lastDeployedAt": "2026-11-04T09:05:00Z", "commitsBehindHead": 2, "health": "Healthy", "argoUrl": "https://argocd.acme.io/applications/sourcing",
             "datadogUrl": "https://app.datadoghq.eu/apm/entity/service:sourcing", "kind": "web"},
            {"id": "docs", "repository": "https://github.com/acme/docs", "lastDeployedAt": "2026-10-20T10:00:00Z", "kind": "web"},
            {"id": "cron", "repository": "https://github.com/acme/sourcing", "lastDeployedAt": "2026-11-04T11:00:00Z", "kind": "cronjob"}
        ]});
        let sourcing_history = json!({"deployments": [
            {"id": 812, "tag": "v1.5.0", "triggeredBy": "github/quaresc", "phase": "Succeeded", "startedAt": "2026-11-04T09:00:00Z", "finishedAt": "2026-11-04T09:05:00Z"},
            {"id": "811", "tag": "v1.4.0", "triggeredBy": "github/someone", "phase": "Succeeded", "startedAt": "2026-11-03T15:00:00Z", "finishedAt": "2026-11-03T15:04:00Z"},
            {"id": 790, "tag": "v1.3.0", "triggeredBy": "someone", "phase": "Succeeded", "startedAt": "2026-10-30T15:00:00Z"}
        ]});
        let docs_history = json!({"deployments": [{"id": 5, "tag": "v2", "phase": "Succeeded", "startedAt": "2026-10-20T10:00:00Z"}]});
        let platform = Fake::default().with("deployStatus", status).with("deployHistory", sourcing_history).with("deployHistory", docs_history);

        let monitors = "<JSON_DATA>\n[{\"id\": 101, \"name\": \"High error rate on sourcing\", \"status\": \"Alert\"}, {\"id\": 102, \"name\": \"Latency\", \"status\": \"OK\"}, \
                        {\"id\": 103, \"name\": \"Apdex\", \"status\": \"No Data\"}]\n</JSON_DATA>\n<is_truncated>false</is_truncated>";
        let series = json!([
            {"binned": [{"start_time": "2026-11-04T09:00:00Z", "count": 10, "avg": 3.0}]},
            {"binned": [{"start_time": "2026-11-04T09:00:00Z", "count": 10, "avg": 0.21}]}
        ]);
        let q = settings(&json!({}));
        let errors_q = query(&q, "errors", "sourcing");
        let datadog = Fake::default()
            .with("search_datadog_monitors", json!(monitors))
            .with("get_datadog_metric", json!(format!("<JSON_DATA>{series}</JSON_DATA>")))
            .with("get_datadog_metric", json!(metric(json!({(errors_q.clone()): 2, "hits": 1000, "p95": 0.2}))))
            .with("get_datadog_metric", json!(metric(json!({(errors_q): 30, "hits": 1000, "p95": 0.21}))))
            .with("search_datadog_error_tracking_issues", json!(ISSUES));

        let inbox = json!({
            "tickets": [{"id": "SOU-2909", "url": "https://linear.app/acme/issue/SOU-2909"}],
            "deploys": {"SOU-2909": [{"repo": "sourcing", "number": 12, "isLive": true, "tag": "v1.4.0"}]}
        });
        let previous = json!({"apps": [{"id": "sourcing", "releases": [
            {"id": 812, "verdict": {"state": "watching", "summary": "watching until 09:35"}},
            {"id": 811, "verdict": {"state": "ok", "summary": "kept"}}
        ]}]});
        let pinned = ["docs".to_string()];
        let ctx = Context {
            platform: &platform_section,
            monitor: &q,
            after_minutes: 30,
            login: "quaresc",
            org: "acme",
            inbox: Some(&inbox),
            pinned: &pinned,
            muted: &[],
            now: at("2026-11-04T12:00:00Z"),
            utc_offset_minutes: 60,
        };
        let doc = block_on(refresh(&platform, &Signals::Datadog(&datadog), &ctx, Some(&previous))).unwrap().unwrap();

        assert_eq!((doc["version"].as_i64(), doc["env"].as_str(), doc["errors"].as_array().map(Vec::len)), (Some(1), Some("production"), Some(0)));
        let ids: Vec<&str> = doc["apps"].as_array().unwrap().iter().map(|a| a["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["sourcing", "old", "docs"], "most recent deploy first, cronjobs skipped");
        let old = &doc["apps"][1];
        assert_eq!((old["watched"].as_bool(), &old["monitors"], old["releases"].as_array().map(Vec::len)), (Some(false), &Value::Null, Some(0)));
        let docs = &doc["apps"][2];
        assert_eq!((docs["pinned"].as_bool(), docs["name"].as_str()), (Some(true), Some("docs")));
        assert_eq!(docs["releases"].as_array().map(Vec::len), Some(1), "the latest release is kept however old");
        assert_eq!(docs["monitors"], Value::Null, "no service, no Datadog");
        let muted = ["sourcing".to_string()];
        let quiet = block_on(refresh(&platform, &Signals::Datadog(&datadog), &Context { muted: &muted, ..ctx }, Some(&previous))).unwrap().unwrap();
        let sourcing = &quiet["apps"][0];
        assert_eq!(
            (sourcing["muted"].as_bool(), sourcing["watched"].as_bool(), sourcing["releases"].as_array().map(Vec::len)),
            (Some(true), Some(false), Some(0))
        );

        let app = &doc["apps"][0];
        assert_eq!((app["repo"].as_str(), app["behind"].as_i64(), app["watched"].as_bool()), (Some("sourcing"), Some(2), Some(true)));
        assert_eq!(app["monitors"]["alert"], 1);
        assert_eq!(app["monitors"]["nodata"], 1);
        assert_eq!(
            app["monitors"]["alerting"],
            json!([{"id": 101, "name": "High error rate on sourcing", "status": "Alert", "url": "https://app.datadoghq.eu/monitors/101"}])
        );
        assert_eq!(app["series"]["errors"][17], 30, "09:00 is the 18th of twenty bins since 12:00 yesterday");
        assert_eq!(app["series"]["p95"][17], 0.21);
        assert_eq!(app["issues"].as_array().map(Vec::len), Some(2));

        let releases = app["releases"].as_array().unwrap();
        assert_eq!(releases.len(), 2, "a release older than a day is left out");
        let latest = &releases[0];
        assert_eq!((latest["actor"].as_str(), latest["isMine"].as_bool(), &latest["tickets"]), (Some("quaresc"), Some(true), &json!([])));
        assert_eq!(latest["verdict"]["state"], "moved");
        assert_eq!(latest["verdict"]["summary"], "errors 0.20% → 3.0% · p95 200 → 210 ms · 1 new error");
        assert_eq!(latest["verdict"]["newIssues"], json!(["3f1c2a9e-0b7d-11f0-9a2b-da7ad0900002"]));
        assert_eq!(latest["verdict"]["until"], "2026-11-04T09:35:00.000Z");
        let shipped = &releases[1];
        assert_eq!(shipped["id"], 811);
        assert_eq!(shipped["isMine"], true, "it ships one of my tickets");
        assert_eq!(shipped["tickets"], json!([{"id": "SOU-2909", "url": "https://linear.app/acme/issue/SOU-2909"}]));
        assert_eq!(shipped["prs"], json!([{"repo": "sourcing", "number": 12, "url": "https://github.com/acme/sourcing/pull/12"}]));
        assert_eq!(shipped["verdict"]["summary"], "kept", "a settled verdict stays");

        let calls = datadog.calls.borrow();
        let before = &calls.iter().filter(|(tool, _)| tool == "get_datadog_metric").nth(1).unwrap().1;
        assert_eq!((before["from"].as_str(), before["to"].as_str()), (Some("2026-11-04T08:30:00.000Z"), Some("2026-11-04T09:00:00.000Z")));
        assert_eq!(before["telemetry"]["intent"], INTENT);
        drop(calls);

        let notices = changes(&previous, &doc, "mine");
        assert_eq!(
            notices,
            [Notice {
                title: "⚠️ Sourcing v1.5.0".into(),
                body: "errors 0.20% → 3.0% · p95 200 → 210 ms · 1 new error".into(),
                action: "monitor:sourcing".into()
            }]
        );
        assert!(changes(&previous, &doc, "none").is_empty());
        assert!(changes(&Value::Null, &doc, "all").is_empty(), "nothing to compare with");
    }

    #[test]
    fn a_service_is_asked_of_datadog_under_the_name_service_replace_gives_it() {
        let platform_section = crate::platform::mapping(&json!({"tools": {"deployStatus": {"tool": "s"}, "deployHistory": {"tool": "h"}},
            "fields": {"apps": "apps", "appId": "id", "deployments": "deployments"}}));
        let platform = Fake::default()
            .with("deployStatus", json!({"apps": [{"id": "api", "name": "API", "service": "api-gateway", "lastDeployedAt": "2026-11-04T11:00:00Z"}]}))
            .with("deployHistory", json!({"deployments": []}));
        let datadog = Fake::default().with("search_datadog_monitors", json!("<JSON_DATA>[]</JSON_DATA>"));
        let q = settings(&json!({"serviceReplace": {"-": "_"}}));
        let ctx = Context {
            platform: &platform_section,
            monitor: &q,
            after_minutes: 30,
            login: "",
            org: "",
            inbox: None,
            pinned: &[],
            muted: &[],
            now: at("2026-11-04T12:00:00Z"),
            utc_offset_minutes: 0,
        };
        let doc = block_on(refresh(&platform, &Signals::Datadog(&datadog), &ctx, None)).unwrap().unwrap();
        assert_eq!(doc["apps"][0]["service"], "api_gateway");
        assert_eq!(datadog.calls.borrow()[0].1["query"], "service:api_gateway");
    }

    #[test]
    fn the_longest_service_replace_key_wins_and_the_datadog_link_follows() {
        let platform_section = crate::platform::mapping(&json!({"tools": {"deployStatus": {"tool": "s"}, "deployHistory": {"tool": "h"}},
            "fields": {"apps": "apps", "appId": "id", "deployments": "deployments"}}));
        let platform = Fake::default()
            .with(
                "deployStatus",
                json!({"apps": [{"id": "api", "name": "API", "service": "api-gateway",
                "datadogUrl": "https://app.datadoghq.eu/apm/entity/service%3Aapi-gateway", "lastDeployedAt": "2026-11-04T11:00:00Z"}]}),
            )
            .with("deployHistory", json!({"deployments": []}));
        let datadog = Fake::default().with("search_datadog_monitors", json!("<JSON_DATA>[]</JSON_DATA>"));
        let q = settings(&json!({"serviceReplace": {"-": "_", "-gateway": "_gw"}}));
        let ctx = Context {
            platform: &platform_section,
            monitor: &q,
            after_minutes: 30,
            login: "",
            org: "",
            inbox: None,
            pinned: &[],
            muted: &[],
            now: at("2026-11-04T12:00:00Z"),
            utc_offset_minutes: 0,
        };
        let doc = block_on(refresh(&platform, &Signals::Datadog(&datadog), &ctx, None)).unwrap().unwrap();
        assert_eq!(doc["apps"][0]["service"], "api_gw");
        assert_eq!(doc["apps"][0]["datadogUrl"], "https://app.datadoghq.eu/apm/entity/service%3Aapi_gw");
    }

    #[test]
    fn a_failing_source_keeps_its_part_and_says_so() {
        let platform_section = crate::platform::mapping(&json!({"tools": {"deployStatus": {"tool": "s"}, "deployHistory": {"tool": "h"}},
            "fields": {"apps": "apps", "appId": "id", "deployments": "deployments"}}));
        let platform = Fake::default()
            .with("deployStatus", json!({"apps": [{"id": "api", "name": "API", "service": "api", "lastDeployedAt": "2026-11-04T11:00:00Z"}]}))
            .with("deployHistory", json!({"deployments": []}));
        let datadog = Fake::default();
        let q = settings(&json!({}));
        let previous = json!({"apps": [{"id": "api", "monitors": {"ok": 3, "warn": 0, "alert": 0, "nodata": 0, "alerting": []}}]});
        let ctx = Context {
            platform: &platform_section,
            monitor: &q,
            after_minutes: 30,
            login: "",
            org: "",
            inbox: None,
            pinned: &[],
            muted: &[],
            now: at("2026-11-04T12:00:00Z"),
            utc_offset_minutes: 0,
        };
        let doc = block_on(refresh(&platform, &Signals::Datadog(&datadog), &ctx, Some(&previous))).unwrap().unwrap();
        assert_eq!(doc["apps"][0]["monitors"]["ok"], 3);
        assert_eq!(doc["apps"][0]["series"], Value::Null);
        let errors: Vec<&str> = doc["errors"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
        assert_eq!(errors.len(), 3);
        assert!(errors[0].starts_with("API monitors: Datadog no tool mapped"), "{errors:?}");
        assert!(block_on(refresh(&Fake::default(), &Signals::Datadog(&datadog), &ctx, None)).unwrap().is_none(), "no platform tools");
    }
}
