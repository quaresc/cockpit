//! The live sources of a show pane's chart or table, the hub's twin of the plugin's `fetchSource` (sources.ts): a
//! Metabase question or query, or Datadog metric queries, as columns and rows.

use serde_json::{Value, json};

use crate::tools::Tools;

/// About this many points across a Datadog window: wider series are averaged down to it.
const POINTS: usize = 60;
const MS_PER_DAY: i64 = 86_400_000;

/// The server a source reads, from the config's `metabase` and `datadog` blocks or their defaults.
pub fn server(source: &Value, servers: &Value) -> String {
    let (kind, default) = if is_metabase(source) { ("metabase", "metabase") } else { ("datadog", "claude.ai Datadog") };
    servers[kind]["server"].as_str().unwrap_or(default).to_string()
}

/// A source's `{columns, rows}`: `source` holds `metabase` or `datadog`, `servers` is the config that names their
/// servers (errors carry the name, as the plugin's do).
pub async fn rows(tools: &impl Tools, source: &Value, servers: &Value) -> Result<Value, String> {
    let server = server(source, servers);
    let call = |tool: &'static str, args: Value| {
        let server = server.clone();
        async move { tools.text(tool, &args).await.map_err(|e| format!("{server} {e}")) }
    };
    if is_metabase(source) {
        let source = &source["metabase"];
        let text = match source["question"].as_f64() {
            Some(_) => call("run_saved_question", json!({"id": source["question"], "parameters": null, "row_limit": 2000})).await?,
            None => {
                let args = json!({"query": source["query"], "query_handle": null, "cursor": null, "prompt": null, "validate_only": null, "row_limit": 2000});
                call("execute_query", args).await?
            }
        };
        return metabase(&text);
    }
    let source = &source["datadog"];
    let mut args = json!({
        "from": if source["from"].is_null() { json!("now-1h") } else { source["from"].clone() },
        "raw_data": true,
        "max_tokens": 20000,
        "telemetry": {"intent": "A live chart in a cockpit show pane beside a coding agent"}
    });
    if !source["queries"].is_null() {
        args["queries"] = source["queries"].clone();
    }
    datadog(&call("get_datadog_metric", args).await?)
}

fn is_metabase(source: &Value) -> bool {
    !source["metabase"].is_null() && source["metabase"] != false && source["metabase"] != 0 && source["metabase"] != ""
}

/// A Metabase answer's columns, by display name, and rows as they are; the answer may wrap them in `data`, and text may
/// follow it (how many rows came back).
fn metabase(text: &str) -> Result<Value, String> {
    let answer: Value =
        serde_json::Deserializer::from_str(text).into_iter().next().ok_or("Metabase: an empty answer")?.map_err(|e| format!("Metabase: {e}"))?;
    let data = if answer["data"].is_object() { &answer["data"] } else { &answer };
    if answer["status"] == "failed" || data["cols"].is_null() {
        return Err(format!("Metabase: {}", answer["error"].as_str().unwrap_or("no data")));
    }
    let columns: Vec<Value> = data["cols"]
        .as_array()
        .map(|cols| cols.iter().map(|c| c["display_name"].as_str().or(c["name"].as_str()).unwrap_or("?").into()).collect())
        .unwrap_or_default();
    let rows = if data["rows"].is_null() { json!([]) } else { data["rows"].clone() };
    Ok(json!({"columns": columns, "rows": rows}))
}

/// Datadog series as rows of a time and one value per series, each the average of its points in that step.
fn datadog(text: &str) -> Result<Value, String> {
    let Some(inner) = json_data(text) else {
        return Err(if text.contains("NO_DATA") {
            "Datadog: no data for these queries in this window".into()
        } else {
            format!("Datadog: {}", text.chars().take(160).collect::<String>())
        });
    };
    let series: Vec<Value> = serde_json::from_str(inner).map_err(|e| format!("Datadog: {e}"))?;
    let values: Vec<Vec<Option<f64>>> = series
        .iter()
        .map(|s| s["values"].as_str().map(|v| v.split(',').map(number).collect()).ok_or_else(|| "Datadog: a series without values".to_string()))
        .collect::<Result<_, _>>()?;
    let first = series.first().ok_or("Datadog: no data for these queries in this window")?;
    let length = values.iter().map(Vec::len).max().unwrap_or(0);
    let step = length.div_ceil(POINTS).max(1);
    let start = first["start_time"].as_str().and_then(time_of_day);
    let interval = first["interval_ms"].as_i64().unwrap_or(0);
    let rows: Vec<Value> = (0..length)
        .step_by(step)
        .map(|i| {
            let time = start.map_or_else(|| "NaN:NaN".to_string(), |ms| clock(ms + i as i64 * interval));
            let mut row = vec![json!(time)];
            row.extend(values.iter().map(|points| average(points.get(i..(i + step).min(points.len())).unwrap_or_default())));
            Value::Array(row)
        })
        .collect();
    let label = |s: &Value| match s["scope"].as_str().filter(|scope| series.len() > 1 && !scope.is_empty()) {
        Some(scope) => json!(scope),
        None => s["expression"].clone(),
    };
    let mut columns = vec![json!("time")];
    columns.extend(series.iter().map(label));
    Ok(json!({"columns": columns, "rows": rows}))
}

/// The text inside a Datadog answer's `<JSON_DATA>` block, None when it has none or it is empty.
fn json_data(text: &str) -> Option<&str> {
    let (_, rest) = text.split_once("<JSON_DATA>")?;
    let (inner, _) = rest.split_once("</JSON_DATA>")?;
    Some(inner.trim()).filter(|i| !i.is_empty())
}

/// A point as JavaScript's `Number` reads it: blank is zero, anything else unreadable is no point.
fn number(raw: &str) -> Option<f64> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Some(0.0);
    }
    raw.parse::<f64>().ok().filter(|v| !v.is_nan())
}

/// The mean of the points there are, to two decimals, or null without any.
fn average(points: &[Option<f64>]) -> Value {
    let found: Vec<f64> = points.iter().flatten().copied().collect();
    if found.is_empty() {
        return Value::Null;
    }
    let mean = ((found.iter().sum::<f64>() / found.len() as f64) * 100.0 + 0.5).floor() / 100.0;
    if mean.fract() == 0.0 && mean.abs() < 1e15 { json!(mean as i64) } else { json!(mean) }
}

/// Milliseconds into the day of an ISO time, in the offset it is written with.
fn time_of_day(iso: &str) -> Option<i64> {
    let time = iso.split_once(['T', ' '])?.1;
    let digits = |s: &str| s.parse::<i64>().ok();
    let (h, m, s) = (digits(time.get(0..2)?)?, digits(time.get(3..5)?)?, digits(time.get(6..8).unwrap_or("00")).unwrap_or(0));
    let ms = time.get(8..).and_then(|r| r.strip_prefix('.')).map_or(0, |r| {
        let frac: String = r.chars().take_while(char::is_ascii_digit).take(3).collect();
        format!("{frac:0<3}").parse::<i64>().unwrap_or(0)
    });
    Some(((h * 60 + m) * 60 + s) * 1000 + ms)
}

fn clock(ms: i64) -> String {
    let minutes = ms.rem_euclid(MS_PER_DAY) / 60_000;
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::Fake;

    fn servers() -> Value {
        json!({"metabase": {"server": "metabase"}, "datadog": {"server": "claude.ai Datadog"}})
    }

    #[test]
    fn a_saved_question_reads_as_its_columns_and_rows() {
        // Metabase's MCP answers {cols, rows} at the top, then a line on how many rows came back.
        let answer = json!({
            "returned": 2, "truncated": false,
            "cols": [{"name": "inserted_at", "display_name": "Inserted At: Day"}, {"name": "count"}], "rows": [["2026-11-03", 12], ["2026-11-04", 7]]
        });
        let fake = Fake::default().with("run_saved_question", json!(format!("{answer}\nreturned 2 rows")));
        let source = json!({"metabase": {"question": 4210}});
        let rows = crate::block_on(rows(&fake, &source, &servers())).unwrap();
        assert_eq!(rows, json!({"columns": ["Inserted At: Day", "count"], "rows": [["2026-11-03", 12], ["2026-11-04", 7]]}));
        assert_eq!(fake.calls.borrow()[0], ("run_saved_question".to_string(), json!({"id": 4210, "parameters": null, "row_limit": 2000})));
    }

    #[test]
    fn an_mbql_query_an_older_answer_and_a_failed_one() {
        let older = json!({"status": "completed", "data": {"cols": [{"name": "count"}], "rows": [[3]]}});
        let fake = Fake::default().with("execute_query", json!(older.to_string()));
        let query = json!({"lib/type": "mbql/query", "stages": [{"source-table": 12, "aggregation": [["count", {}]]}]});
        let source = json!({"metabase": {"query": query}});
        assert_eq!(crate::block_on(rows(&fake, &source, &servers())).unwrap(), json!({"columns": ["count"], "rows": [[3]]}));
        let args = json!({"query": query, "query_handle": null, "cursor": null, "prompt": null, "validate_only": null, "row_limit": 2000});
        assert_eq!(fake.calls.borrow()[0].1, args);
        let failed = json!({"status": "failed", "error": "Table 12 not found"});
        let fake = Fake::default().with("execute_query", json!(failed.to_string()));
        assert_eq!(crate::block_on(rows(&fake, &source, &servers())).unwrap_err(), "Metabase: Table 12 not found");
    }

    #[test]
    fn datadog_series_average_down_to_sixty_points() {
        let values: Vec<String> = (0..120).map(|i| if i == 3 { "null".into() } else { i.to_string() }).collect();
        let series = json!([
            {"expression": "sum:trace.http.server.request.hits{service:api}", "scope": "service:api", "values": values.join(","), "start_time": "2026-11-04T09:58:00Z", "interval_ms": 30000},
            {"expression": "sum:trace.http.server.request.hits{service:web}", "scope": "service:web", "values": "1.004,2", "start_time": "2026-11-04T09:58:00Z", "interval_ms": 30000}
        ]);
        let answer = format!("Metric query results.\n<JSON_DATA>\n{series}\n</JSON_DATA>\n<is_truncated>false</is_truncated>");
        let fake = Fake::default().with("get_datadog_metric", json!(answer));
        let source = json!({"datadog": {"queries": ["sum:trace.http.server.request.hits{service:api}", "sum:trace.http.server.request.hits{service:web}"], "from": "now-1h"}});
        let rows = crate::block_on(rows(&fake, &source, &servers())).unwrap();
        assert_eq!(rows["columns"], json!(["time", "service:api", "service:web"]));
        let all = rows["rows"].as_array().unwrap();
        assert_eq!(all.len(), 60);
        assert_eq!(all[0], json!(["09:58", 0.5, 1.5]));
        assert_eq!(all[1], json!(["09:59", 2, null]), "a null point is left out of its step's mean");
        assert_eq!(all[59], json!(["10:57", 118.5, null]));
        let (tool, args) = fake.calls.borrow()[0].clone();
        assert_eq!((tool.as_str(), &args["from"], &args["raw_data"]), ("get_datadog_metric", &json!("now-1h"), &json!(true)));
    }

    #[test]
    fn one_datadog_series_is_named_by_its_query_and_no_data_says_so() {
        let series = json!([{"expression": "avg:system.load.1{env:production}", "scope": "env:production", "values": "1.5,2.5", "start_time": "2026-11-04T23:59:30+01:00", "interval_ms": 60000}]);
        let fake = Fake::default().with("get_datadog_metric", json!(format!("<JSON_DATA>{series}</JSON_DATA>")));
        let source = json!({"datadog": {"queries": ["avg:system.load.1{env:production}"]}});
        let rows = crate::block_on(rows(&fake, &source, &servers())).unwrap();
        assert_eq!(rows, json!({"columns": ["time", "avg:system.load.1{env:production}"], "rows": [["23:59", 1.5], ["00:00", 2.5]]}));

        let empty = Fake::default().with("get_datadog_metric", json!("NO_DATA: the queries matched no series.\n<JSON_DATA></JSON_DATA>"));
        let error = crate::block_on(super::rows(&empty, &source, &json!({}))).unwrap_err();
        assert_eq!(error, "Datadog: no data for these queries in this window");
        assert_eq!(server(&source, &json!({})), "claude.ai Datadog");
        assert_eq!(server(&json!({"metabase": {"question": 1}}), &json!({"metabase": {"server": "mb"}})), "mb");
    }
}
