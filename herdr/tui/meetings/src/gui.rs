//! The meetings for the Herdr app's cockpit mod: the week as the mod draws it (the list grouped by day, the now-or-next
//! banner, each meeting's times in words, brief and actions, and the calendar grid's blocks), built from the week the
//! hub keeps and the briefs on disk. The mod asks again each minute, so the "in N min" texts stay true.

use std::path::PathBuf;

use chrono::{Datelike, NaiveDate, NaiveDateTime, Timelike, Weekday};
use cockpit_core::files;
use serde_json::{Value, json};

use crate::timeline;
use crate::week::{self, Clock, Meeting, Week, day_label, short_day, until_text};

/// The folder the view reads: the briefs, and their pending marks.
pub fn dirs() -> Vec<PathBuf> {
    vec![week::briefs_dir()]
}

pub fn view(_args: &Value, doc: &dyn Fn(&str) -> Value) -> Value {
    let data = doc("meetings");
    let data = if data.is_null() { files::load(&week::cache_path()) } else { Some(data) };
    let enabled = files::config()["meetingBrief"]["enabled"].as_bool().unwrap_or(true);
    model(data.as_ref(), &Clock::system(), enabled, &|id| week::read_brief(id).map(|l| l.join("\n")), &week::is_preparing)
}

fn hours(t: NaiveDateTime) -> f64 {
    t.hour() as f64 + t.minute() as f64 / 60.0
}

/// The week as the mod draws it; `brief` and `preparing` read a meeting's brief and whether one is on its way.
pub fn model(data: Option<&Value>, clock: &Clock, brief_enabled: bool, brief: &dyn Fn(&str) -> Option<String>, preparing: &dyn Fn(&str) -> bool) -> Value {
    let Some(data) = data else {
        return json!({"waiting": true, "meetings": [], "days": [], "now": clock.now.timestamp_millis()});
    };
    let week = Week::parse(data);
    let updated = week.at.and_then(chrono::DateTime::from_timestamp_millis).map(|at| clock.local(at).format("%H:%M").to_string());
    let shown = week.shown(clock);
    let mut days: Vec<Value> = Vec::new();
    let mut day: Option<NaiveDate> = None;
    for m in &shown {
        let date = clock.local(m.start).date();
        if day != Some(date) {
            day = Some(date);
            days.push(
                json!({"date": date.to_string(), "label": day_label(date, clock), "short": short_day(date, clock), "today": date == clock.today(), "ids": []}),
            );
        }
        if let Some(ids) = days.last_mut().and_then(|d| d["ids"].as_array_mut()) {
            ids.push(Value::from(m.id.clone()));
        }
    }
    let meetings: Vec<Value> = week.meetings.iter().map(|m| meeting_json(m, clock, brief_enabled, brief(&m.id), preparing(&m.id))).collect();
    json!({
        "waiting": false,
        "updated": updated,
        "error": week.error,
        "now": clock.now.timestamp_millis(),
        "today": clock.today().to_string(),
        "banner": banner(&week, clock),
        "days": days,
        "shown": shown.iter().map(|m| m.id.clone()).collect::<Vec<_>>(),
        "meetings": meetings,
        "grid": grid(&week, clock),
        "brief_enabled": brief_enabled,
    })
}

/// The running meeting, else the next one.
fn banner(week: &Week, clock: &Clock) -> Value {
    let Some(first) = week.upcoming(clock).into_iter().next() else { return json!({"state": "none"}) };
    if first.start <= clock.now {
        let total = (first.end - first.start).num_seconds().max(1) as f64;
        let done = (clock.now - first.start).num_seconds() as f64;
        return json!({"id": first.id, "state": "now", "title": first.title, "until": clock.local(first.end).format("%H:%M").to_string(),
                      "progress": (done / total).clamp(0.0, 1.0), "join": first.join.is_some()});
    }
    let local = clock.local(first.start);
    let when =
        if local.date() == clock.today() { until_text(first.start, clock) } else { format!("{} {}", day_label(local.date(), clock), local.format("%H:%M")) };
    json!({"id": first.id, "state": "next", "title": first.title, "when": when, "soon": first.is_soon(clock), "join": first.join.is_some()})
}

fn meeting_json(m: &Meeting, clock: &Clock, brief_enabled: bool, brief: Option<String>, preparing: bool) -> Value {
    let local = clock.local(m.start);
    let state = if m.is_now(clock) {
        "now"
    } else if m.is_past(clock) {
        "past"
    } else if m.is_soon(clock) {
        "soon"
    } else {
        "upcoming"
    };
    let when = if m.is_now(clock) {
        format!("now, until {}", clock.local(m.end).format("%H:%M"))
    } else if !m.is_past(clock) && local.date() == clock.today() {
        format!("{} {}", m.span_text(clock), until_text(m.start, clock))
    } else {
        format!("{} {}", day_label(local.date(), clock), m.span_text(clock))
    };
    let mut acts = Vec::new();
    if brief.is_some() {
        acts.push(json!({"id": "brief", "label": "Brief", "key": "b", "main": false}));
    }
    if m.join.is_some() {
        acts.push(json!({"id": "join", "label": "Join", "key": "m", "main": true}));
    }
    acts.push(json!({"id": "open", "label": "Open", "key": "o", "main": m.join.is_none()}));
    if brief.is_some() {
        acts.push(json!({"id": "prepare", "label": "Redo brief", "key": "r", "main": false}));
    } else if m.briefable && !m.is_past(clock) && brief_enabled {
        acts.push(json!({"id": "prepare", "label": "Prepare brief", "key": "p", "main": false}));
    }
    json!({
        "id": m.id,
        "title": m.title,
        "day": local.date().to_string(),
        "start": m.start.timestamp_millis(),
        "end": m.end.timestamp_millis(),
        "span": m.span_text(clock),
        "time": local.format("%H:%M").to_string(),
        "duration": m.duration_text(),
        "until": until_text(m.start, clock),
        "when": when,
        "state": state,
        "response": m.response,
        "attendees": m.attendees,
        "people": m.people.iter().map(|p| p.split('@').next().unwrap_or(p).to_string()).collect::<Vec<_>>(),
        "url": m.url,
        "join": m.join,
        "briefable": m.briefable,
        "brief": brief,
        "preparing": preparing,
        "raw": m.raw,
        "acts": acts,
    })
}

/// The calendar: the coming seven days (a weekend left out when it has nothing), the hours they span, each meeting a
/// block as fractions of that span, side by side in lanes when they overlap, and where now is.
fn grid(week: &Week, clock: &Clock) -> Value {
    let today = clock.today();
    let on = |day: NaiveDate| week.meetings.iter().filter(move |m| clock.local(m.start).date() == day);
    let days: Vec<NaiveDate> = (0..7u64)
        .filter_map(|n| today.checked_add_days(chrono::Days::new(n)))
        .filter(|d| !matches!(d.weekday(), Weekday::Sat | Weekday::Sun) || on(*d).next().is_some())
        .collect();
    let end_of = |m: &Meeting| {
        let (start, end) = (clock.local(m.start), clock.local(m.end));
        if end.date() > start.date() { 24.0 } else { hours(end) }
    };
    let (mut from, mut to) = (8.0_f64, 19.0_f64);
    for m in days.iter().flat_map(|d| on(*d)) {
        from = from.min(clock.local(m.start).hour() as f64);
        to = to.max(end_of(m).ceil());
    }
    let span = (to - from).max(1.0);
    let mut blocks = Vec::new();
    for (i, day) in days.iter().enumerate() {
        let mut meetings: Vec<&Meeting> = on(*day).collect();
        meetings.sort_by_key(|m| m.start);
        // A block is drawn at least a quarter hour tall: overlaps are reckoned on that.
        let spans: Vec<(f64, f64)> = meetings
            .iter()
            .map(|m| {
                let top = (hours(clock.local(m.start)) - from) / span;
                (top, ((end_of(m) - from) / span).max(top + 0.25 / span))
            })
            .collect();
        for ((m, (top, bottom)), (lane, lanes)) in meetings.iter().zip(&spans).zip(timeline::lanes(&spans)) {
            blocks.push(json!({"id": m.id, "day": i, "top": top, "bottom": bottom, "lane": lane, "lanes": lanes}));
        }
    }
    let now = days.iter().position(|d| *d == today).map(|i| (i, (hours(clock.now_local()) - from) / span)).filter(|(_, at)| (0.0..=1.0).contains(at));
    json!({
        "days": days.iter().map(|d| json!({"date": d.to_string(), "short": short_day(*d, clock), "label": day_label(*d, clock), "today": *d == today})).collect::<Vec<_>>(),
        "from": from,
        "to": to,
        "blocks": blocks,
        "now": now.map(|(day, at)| json!({"day": day, "at": at})),
    })
}

/// The mod's own actions: none, the hub runs `prepare`, `open_url` and `join`.
pub fn act(_action: &str, _args: &[Value]) -> Option<Result<String, String>> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};

    #[test]
    fn the_week_as_the_mod_draws_it() {
        let data: Value =
            serde_json::from_str(&std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/meetings.json")).unwrap()).unwrap();
        let first = &data["meetings"][0];
        let start = DateTime::parse_from_rfc3339(first["start"].as_str().unwrap()).unwrap();
        // Five minutes into the first meeting.
        let clock = Clock { now: (start + chrono::TimeDelta::minutes(5)).with_timezone(&Utc), zone: Some(*start.offset()) };
        let id = first["id"].as_str().unwrap().to_string();
        let view = model(Some(&data), &clock, true, &|m| (m == id).then(|| "## Bring\n- notes".to_string()), &|_| false);
        assert_eq!((view["banner"]["state"].as_str(), view["banner"]["id"].as_str()), (Some("now"), Some(id.as_str())));
        assert!(view["banner"]["progress"].as_f64().unwrap() > 0.3);
        assert_eq!(view["days"][0]["label"], "Today");
        let m = view["meetings"].as_array().unwrap().iter().find(|m| m["id"] == id.as_str()).unwrap();
        assert_eq!((m["state"].as_str(), m["people"][0].as_str()), (Some("now"), Some("alice")));
        let acts: Vec<&str> = m["acts"].as_array().unwrap().iter().map(|a| a["id"].as_str().unwrap()).collect();
        assert_eq!(acts, ["brief", "join", "open", "prepare"]);
        let block = view["grid"]["blocks"].as_array().unwrap().iter().find(|b| b["id"] == id.as_str()).unwrap();
        assert!(block["top"].as_f64().unwrap() < block["bottom"].as_f64().unwrap());
        assert_eq!(view["grid"]["now"]["day"], 0);
        assert_eq!(model(None, &clock, true, &|_| None, &|_| false)["waiting"], true);
    }
}
