//! The numbers behind the release monitor and the show panes' metrics: Datadog through its own tools, or any metrics and
//! errors tool through an `observability` mapping (Grafana, Prometheus, New Relic, Sentry…).

use serde_json::{Value, json};

use crate::mapping::{list_at, pick, text};
use crate::monitor::{self, DAY_MS, Health};
use crate::tools::Tools;

const BINS: usize = 20;
const POINTS: usize = 60;

/// Where the numbers come from: Datadog's tools, or a server mapped by the config's `observability` section.
pub enum Signals<'a, T: Tools> {
    Datadog(&'a T),
    Mapped { tools: &'a T, section: &'a Value },
}

/// The config's observability section with its defaults: no tools, and an empty status map.
pub fn mapping(section: &Value) -> Value {
    let mut merged = json!({"name": "Observability", "server": "", "tools": {}, "fields": {}, "statuses": {}, "queries": {}});
    for (key, value) in section.as_object().into_iter().flatten() {
        merged[key] = value.clone();
    }
    merged
}

/// The server behind the monitor and metrics panes, from the raw config: `observability.server`'s entry with that
/// section, else `datadog.server`'s with Datadog's own tools; only once it can be called.
pub fn server(raw: &Value) -> Option<(crate::Server, Value)> {
    let given = raw["observability"]["server"].as_str().filter(|s| !s.is_empty());
    let (name, section) = match given {
        Some(name) => (name, mapping(&raw["observability"])),
        None => (raw["datadog"]["server"].as_str().filter(|s| !s.is_empty())?, json!({"name": "Datadog", "tools": {}})),
    };
    let server = crate::Server::from_entry(&raw["servers"][name]).filter(crate::Server::is_ready)?;
    let mut section = section;
    section["server"] = name.into();
    Some((server, section))
}

/// Whether a section reads Datadog through its own tools: Datadog's server with nothing mapped.
pub fn is_datadog(section: &Value, server: &str) -> bool {
    let unmapped = section["tools"].as_object().is_none_or(|t| t.is_empty());
    let named = format!("{} {server}", section["name"].as_str().unwrap_or("")).to_lowercase();
    unmapped && named.contains("datadog")
}

/// One point's time in milliseconds: epoch seconds or milliseconds (as a number or text), or an ISO time.
fn time_ms(value: Option<&Value>) -> Option<i64> {
    let value = value?;
    let n = value.as_f64().or_else(|| value.as_str().and_then(|s| s.trim().parse::<f64>().ok()));
    match n {
        Some(n) if n > 1e12 => Some(n as i64),
        Some(n) => Some((n * 1000.0) as i64),
        None => value.as_str().and_then(monitor::parse_ms),
    }
}

fn number(value: Option<&Value>) -> Option<f64> {
    let value = value?;
    value.as_f64().or_else(|| value.as_str().and_then(|s| s.trim().parse::<f64>().ok())).filter(|v| v.is_finite())
}

/// A start such as now-1h, now-30m, now-2d or an ISO time, in milliseconds.
pub fn start_of(from: &str, now: i64) -> i64 {
    let Some(rest) = from.trim().strip_prefix("now-") else { return monitor::parse_ms(from).unwrap_or(now - 3_600_000) };
    let (n, unit) = rest.split_at(rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len()));
    let n: i64 = n.parse().unwrap_or(1);
    let unit_ms = match unit {
        "s" => 1000,
        "m" => 60_000,
        "d" => DAY_MS,
        "w" => 7 * DAY_MS,
        _ => 3_600_000,
    };
    now - n * unit_ms
}

impl<T: Tools> Signals<'_, T> {
    fn query(section: &Value, monitor: &Value, name: &str, service: &str) -> String {
        let env = monitor["env"].as_str().unwrap_or("");
        section["queries"][name].as_str().unwrap_or("").replace("{service}", service).replace("{env}", env)
    }

    /// One metric query's series over [from, to], each a name and its points (time in ms, value).
    async fn series(&self, section: &Value, query: &str, from: i64, to: i64) -> Result<Vec<(String, Vec<(i64, f64)>)>, String> {
        let Self::Mapped { tools, .. } = self else { return Ok(Vec::new()) };
        let step = ((to - from) / 1000 / POINTS as i64).max(1);
        // A query may size its own windows by the step, as Prometheus' `[{step}s]`.
        let query = query.replace("{step}", &step.to_string());
        let values = json!({
            "query": query, "from": monitor::iso(from), "to": monitor::iso(to), "fromSeconds": from / 1000, "toSeconds": to / 1000,
            "fromMs": from, "toMs": to, "step": step
        });
        let answer = tools.call("metric", &values).await?;
        let f = |name: &str| section["fields"][name].as_str().unwrap_or("").to_string();
        Ok(list_at(&answer, &f("series"))
            .into_iter()
            .map(|s| {
                let name = text(pick(s, &f("seriesName"))).unwrap_or_else(|| query.clone());
                let points = list_at(s, &f("points"))
                    .into_iter()
                    .filter_map(|p| Some((time_ms(pick(p, &f("pointTime")))?, number(pick(p, &f("pointValue")))?)))
                    .collect();
                (name, points)
            })
            .collect())
    }

    /// Errors, hits and p95 (seconds) of a service over [from, to]; none without errors nor hits.
    pub async fn health(&self, monitor: &Value, service: &str, from: i64, to: i64) -> Result<Option<Health>, String> {
        let section = match self {
            Self::Datadog(tools) => return monitor::health(*tools, monitor, service, from, to).await,
            Self::Mapped { section, .. } => *section,
        };
        let sum = |series: &[(String, Vec<(i64, f64)>)]| -> Option<f64> {
            let points: Vec<f64> = series.iter().flat_map(|(_, p)| p.iter().map(|(_, v)| *v)).collect();
            (!points.is_empty()).then(|| points.iter().sum())
        };
        let errors = sum(&self.series(section, &Self::query(section, monitor, "errors", service), from, to).await?);
        let hits = sum(&self.series(section, &Self::query(section, monitor, "hits", service), from, to).await?);
        if errors.is_none() && hits.is_none() {
            return Ok(None);
        }
        let p95: Vec<f64> = self
            .series(section, &Self::query(section, monitor, "p95", service), from, to)
            .await?
            .into_iter()
            .flat_map(|(_, p)| p.into_iter().map(|(_, v)| v))
            .collect();
        let p95 = (!p95.is_empty()).then(|| (p95.iter().sum::<f64>() / p95.len() as f64 * 1e6).round() / 1e6);
        Ok(Some(Health { errors: errors.unwrap_or(0.0), hits: hits.unwrap_or(0.0), p95 }))
    }

    /// A day of a service's errors (summed) and p95 (averaged), in `BINS` values each.
    pub async fn day(&self, monitor: &Value, service: &str, now: i64) -> Result<Value, String> {
        let section = match self {
            Self::Datadog(tools) => return monitor::series_of(*tools, monitor, service, now).await,
            Self::Mapped { section, .. } => *section,
        };
        let from = now - DAY_MS;
        let binned = |series: Vec<(String, Vec<(i64, f64)>)>, is_sum: bool| {
            let mut sums = [0.0; BINS];
            let mut counts = [0usize; BINS];
            for (t, v) in series.into_iter().flat_map(|(_, p)| p) {
                let i = (((t - from) as f64 / (DAY_MS as f64 / BINS as f64)).floor()).clamp(0.0, (BINS - 1) as f64) as usize;
                sums[i] += v;
                counts[i] += 1;
            }
            (0..BINS).map(|i| if is_sum || counts[i] == 0 { round(sums[i]) } else { round(sums[i] / counts[i] as f64) }).collect::<Vec<_>>()
        };
        let errors = self.series(section, &Self::query(section, monitor, "errors", service), from, now).await?;
        let p95 = self.series(section, &Self::query(section, monitor, "p95", service), from, now).await?;
        Ok(json!({"errors": binned(errors, true), "p95": binned(p95, false)}))
    }

    /// A service's monitors by status, and the alerting ones with their links.
    pub async fn monitors(&self, monitor: &Value, service: &str, site: &str) -> Result<Value, String> {
        let (tools, section) = match self {
            Self::Datadog(tools) => return monitor::monitors_of(*tools, service, site).await,
            Self::Mapped { tools, section } => (*tools, *section),
        };
        if !tools.has("monitors") {
            return Ok(Value::Null);
        }
        let env = monitor["env"].as_str().unwrap_or("");
        let answer = tools.call("monitors", &json!({"service": service, "env": env})).await?;
        let f = |name: &str| section["fields"][name].as_str().unwrap_or("").to_string();
        let status = |m: &Value| {
            let raw = text(pick(m, &f("monitorStatus"))).unwrap_or_default();
            section["statuses"][&raw].as_str().map(str::to_string).unwrap_or(raw).to_lowercase()
        };
        let found = list_at(&answer, &f("monitors"));
        let is = |s: &str| found.iter().filter(|m| status(m) == s).count();
        let alerting: Vec<Value> = found
            .iter()
            .filter(|m| ["alert", "warn"].contains(&status(m).as_str()))
            .map(|m| json!({"id": text(pick(m, &f("monitorId"))), "name": text(pick(m, &f("monitorName"))), "status": status(m), "url": text(pick(m, &f("monitorUrl")))}))
            .collect();
        Ok(json!({"ok": is("ok"), "warn": is("warn"), "alert": is("alert"), "nodata": is("no data"), "alerting": alerting}))
    }

    /// A service's open errors of the last day, most frequent first, at most eight.
    pub async fn issues(&self, monitor: &Value, service: &str, now: i64) -> Result<Vec<Value>, String> {
        let (tools, section) = match self {
            Self::Datadog(tools) => return monitor::issues_of(*tools, monitor, service, now).await,
            Self::Mapped { tools, section } => (*tools, *section),
        };
        if !tools.has("issues") {
            return Ok(Vec::new());
        }
        let from = now - DAY_MS;
        let env = monitor["env"].as_str().unwrap_or("");
        let values = json!({"service": service, "env": env, "from": monitor::iso(from), "fromSeconds": from / 1000, "fromMs": from});
        let answer = tools.call("issues", &values).await?;
        let f = |name: &str| section["fields"][name].as_str().unwrap_or("").to_string();
        let mut issues: Vec<Value> = list_at(&answer, &f("issues"))
            .into_iter()
            .map(|i| {
                let get = |name: &str| text(pick(i, &f(name))).unwrap_or_default();
                let first_seen = pick(i, &f("issueFirstSeen")).and_then(|v| time_ms(Some(v)));
                json!({
                    "id": get("issueId"),
                    "type": get("issueType"),
                    "message": get("issueMessage").lines().find(|l| !l.trim().is_empty()).unwrap_or("").to_string(),
                    "count": number(pick(i, &f("issueCount"))).unwrap_or(0.0),
                    "firstSeen": first_seen.map(monitor::iso).unwrap_or_default(),
                    "firstVersion": get("issueFirstVersion"),
                    "url": get("issueUrl"),
                    "isNew": first_seen.is_some_and(|t| t >= from),
                })
            })
            .filter(|i| i["id"].as_str().is_some_and(|id| !id.is_empty()))
            .collect();
        issues.sort_by(|a, b| b["count"].as_f64().unwrap_or(0.0).total_cmp(&a["count"].as_f64().unwrap_or(0.0)));
        issues.truncate(8);
        Ok(issues)
    }

    /// A show pane's metric queries over [from, now] as `{columns, rows}`: a local time (`offset` minutes from UTC), then
    /// one column per series.
    pub async fn rows(&self, queries: &[String], from: &str, now: i64, offset: i64) -> Result<Value, String> {
        let section = match self {
            Self::Datadog(tools) => {
                let source = json!({"datadog": {"queries": queries, "from": from}});
                return crate::sources::rows(*tools, &source, &Value::Null).await;
            }
            Self::Mapped { section, .. } => *section,
        };
        let start = start_of(from, now);
        let mut series = Vec::new();
        for query in queries {
            let found = self.series(section, query, start, now).await?;
            let many = found.len() > 1;
            series.extend(found.into_iter().map(|(name, points)| (if many { name } else { query.clone() }, points)));
        }
        if series.iter().all(|(_, p)| p.is_empty()) {
            return Err("no data for these queries in this window".into());
        }
        let width = ((now - start) / POINTS as i64).max(1);
        let mut columns = vec![json!("time")];
        columns.extend(series.iter().map(|(name, _)| json!(name)));
        let rows: Vec<Value> = (0..POINTS as i64)
            .map(|b| {
                let (lo, hi) = (start + b * width, start + (b + 1) * width);
                let mut row = vec![json!(clock(lo + offset * 60_000))];
                for (_, points) in &series {
                    let found: Vec<f64> = points.iter().filter(|(t, _)| *t >= lo && *t < hi).map(|(_, v)| *v).collect();
                    row.push(if found.is_empty() { Value::Null } else { json!(round(found.iter().sum::<f64>() / found.len() as f64)) });
                }
                Value::Array(row)
            })
            .filter(|row| row.as_array().is_some_and(|r| r[1..].iter().any(|v| !v.is_null())))
            .collect();
        Ok(json!({"columns": columns, "rows": rows}))
    }
}

fn round(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

/// A time as HH:MM, in the offset it is given.
fn clock(ms: i64) -> String {
    let iso = monitor::iso(ms);
    iso.get(11..16).unwrap_or("").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::Fake;

    /// A Prometheus-shaped mapping, as Grafana's MCP answers a range query.
    fn prometheus() -> Value {
        mapping(&json!({
            "name": "Grafana",
            "tools": {
                "metric": {"tool": "query_prometheus", "args": {"expr": "{query}", "startTime": "{from}", "endTime": "{to}", "stepSeconds": "{step}", "queryType": "range"}},
                "monitors": {"tool": "list_alert_rules", "args": {"label_selectors": [{"name": "service", "value": "{service}"}]}},
                "issues": {"tool": "sentry_issues", "args": {"query": "service:{service} environment:{env}"}}
            },
            "fields": {
                "series": "data.result", "seriesName": "metric.__name__", "points": "values", "pointTime": "0", "pointValue": "1",
                "monitors": "rules", "monitorName": "title", "monitorStatus": "state", "monitorId": "uid", "monitorUrl": "url",
                "issues": "", "issueId": "shortId", "issueType": "metadata.type", "issueMessage": "title", "issueCount": "count",
                "issueFirstSeen": "firstSeen", "issueFirstVersion": "firstRelease.version", "issueUrl": "permalink"
            },
            "statuses": {"firing": "alert", "pending": "warn", "inactive": "ok", "normal": "ok"},
            "queries": {"errors": "sum(increase(http_errors{service=\"{service}\"}[5m]))", "hits": "sum(increase(http_hits{service=\"{service}\"}[5m]))",
                "p95": "histogram_quantile(0.95, rate(http_seconds_bucket{service=\"{service}\"}[5m]))"}
        }))
    }

    const NOW: i64 = 1_793_800_000_000;

    fn range(values: &[(i64, &str)]) -> Value {
        json!({"data": {"result": [{"metric": {"__name__": "x"}, "values": values.iter().map(|(t, v)| json!([t, v])).collect::<Vec<_>>()}]}})
    }

    #[test]
    fn health_sums_counts_and_averages_the_p95() {
        let t = NOW / 1000 - 600;
        let section = prometheus();
        let fake = Fake::default()
            .with("metric", range(&[(t, "2"), (t + 60, "1")]))
            .with("metric", range(&[(t, "100"), (t + 60, "300")]))
            .with("metric", range(&[(t, "0.2"), (t + 60, "0.4")]));
        let signals = Signals::Mapped { tools: &fake, section: &section };
        let monitor = json!({"env": "production"});
        let health = crate::block_on(signals.health(&monitor, "api", NOW - 3_600_000, NOW)).unwrap().unwrap();
        assert_eq!((health.errors, health.hits), (3.0, 400.0));
        assert!((health.p95.unwrap() - 0.3).abs() < 1e-9);
        let first = &fake.calls.borrow()[0].1;
        assert_eq!(first["query"], "sum(increase(http_errors{service=\"api\"}[5m]))");
        assert_eq!(first["toSeconds"], NOW / 1000);
        let empty = Fake::default().with("metric", json!({"data": {"result": []}}));
        let signals = Signals::Mapped { tools: &empty, section: &section };
        assert_eq!(crate::block_on(signals.health(&monitor, "api", NOW - 3_600_000, NOW)).unwrap(), None);
    }

    #[test]
    fn a_day_bins_monitors_map_statuses_and_issues_sort_by_count() {
        let section = prometheus();
        let t = (NOW - DAY_MS) / 1000;
        let fake = Fake::default()
            .with("metric", range(&[(t + 10, "1"), (t + 20, "2"), (NOW / 1000 - 10, "5")]))
            .with("metric", range(&[(t + 10, "0.1"), (t + 20, "0.3")]))
            .with("monitors", json!({"rules": [{"title": "5xx", "state": "firing", "uid": "a", "url": "https://g/a"}, {"title": "lat", "state": "normal"}]}))
            .with("issues", json!([
                {"shortId": "API-1", "title": "Boom\nat x", "count": "3", "firstSeen": "2026-11-04T14:00:00Z", "permalink": "https://s/1", "metadata": {"type": "KeyError"}},
                {"shortId": "API-2", "title": "Bang", "count": 9, "firstSeen": "2025-01-01T00:00:00Z"}
            ]));
        let signals = Signals::Mapped { tools: &fake, section: &section };
        let monitor = json!({"env": "production"});
        let day = crate::block_on(signals.day(&monitor, "api", NOW)).unwrap();
        assert_eq!((day["errors"][0].as_f64(), day["errors"][19].as_f64(), day["p95"][0].as_f64()), (Some(3.0), Some(5.0), Some(0.2)));
        let monitors = crate::block_on(signals.monitors(&monitor, "api", "")).unwrap();
        assert_eq!((&monitors["alert"], &monitors["ok"]), (&json!(1), &json!(1)));
        assert_eq!(monitors["alerting"][0], json!({"id": "a", "name": "5xx", "status": "alert", "url": "https://g/a"}));
        let issues = crate::block_on(signals.issues(&monitor, "api", NOW)).unwrap();
        assert_eq!(issues.iter().map(|i| i["id"].as_str().unwrap()).collect::<Vec<_>>(), ["API-2", "API-1"]);
        assert_eq!((&issues[1]["message"], &issues[1]["type"], &issues[1]["isNew"]), (&json!("Boom"), &json!("KeyError"), &json!(true)));
    }

    #[test]
    fn show_rows_put_each_series_in_a_column() {
        let section = prometheus();
        let t = NOW / 1000 - 1800;
        let fake = Fake::default().with("metric", range(&[(t, "4"), (t + 1, "6")]));
        let signals = Signals::Mapped { tools: &fake, section: &section };
        let rows = crate::block_on(signals.rows(&["rate(x[5m])".into()], "now-1h", NOW, 0)).unwrap();
        assert_eq!(rows["columns"], json!(["time", "rate(x[5m])"]));
        assert_eq!(rows["rows"].as_array().unwrap().len(), 1);
        assert_eq!(rows["rows"][0][1], json!(5.0));
        assert_eq!(start_of("now-30m", NOW), NOW - 1_800_000);
        assert!(is_datadog(&json!({"name": "Datadog"}), "") && !is_datadog(&section, "grafana"));
    }
}
