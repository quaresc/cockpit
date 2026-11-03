//! My meetings from the calendar's secret iCal address (`calendar.ics`), so meetings.json stays fresh with no Claude
//! session: every five minutes, its events from yesterday to a week ahead, recurrences expanded, mapped as the mod maps
//! the Google Calendar connector's answer (same ids, so briefs and notifications carry over), marked `by: "hub"`.
//!
//! Me, for my answer to each invite: `calendar.email` (one address or a list), else the calendar id a Google address
//! names, else the calendar's X-WR-CALNAME when it is an address, else git's `user.email`.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use chrono::{DateTime, Duration, Local, SecondsFormat, TimeZone, Utc};
use regex::Regex;
use rrule::{RRule, RRuleSet, Tz, Unvalidated};
use serde_json::{Map, Value, json};

use super::http::{self, Failure, Fetched};
use super::ics::{self, Calendar, Prop, Props, When};
use crate::config::Config;
use crate::value::list;

/// How often the address is fetched: Google refreshes what it serves there every few minutes at best.
pub const EVERY: u64 = 5 * 60;
const MAX_INSTANCES: u16 = 500;
const MAX_DESCRIPTION: usize = 2000;
const MAX_PEOPLE: usize = 15;

/// The address as the config gives it; a webcal:// link is the same over HTTPS.
pub fn address(raw: &Value) -> Option<String> {
    let url = raw.as_str().map(str::trim).filter(|u| !u.is_empty())?;
    Some(match url.strip_prefix("webcal://") {
        Some(rest) => format!("https://{rest}"),
        None => url.to_string(),
    })
}

/// The meetings of the coming week from the address, or why not; the address never shows in an error.
pub fn fetch(url: &str, config: &Config) -> Fetched<Vec<Value>> {
    let source = http::text(url, &[("Accept", "text/calendar")]).map_err(|failure| match failure {
        Failure::Other(message) => Failure::Other(message.replace(url, "the calendar address")),
        limited => limited,
    })?;
    if !source.trim_start().starts_with("BEGIN:VCALENDAR") {
        return Err(Failure::Other("the calendar address answered with no calendar".into()));
    }
    let calendar = ics::parse(&source);
    let mut emails = emails(&config.0["calendar"]["email"]);
    let calendar_id = google_calendar_id(url).or_else(|| calendar.props.text("X-WR-CALNAME").filter(|n| is_address(n)));
    emails.extend(calendar_id.clone());
    if emails.is_empty() {
        emails.extend(git_email());
    }
    let today = Local::now().date_naive();
    let midnight = |days: i64| {
        let day = (today + Duration::days(days)).and_hms_opt(0, 0, 0).unwrap_or_default();
        Local.from_local_datetime(&day).earliest().map(|t| t.with_timezone(&Utc))
    };
    let (Some(from), Some(to)) = (midnight(-1), midnight(8)) else {
        return Err(Failure::Other("no local midnight today".into()));
    };
    let skip: Vec<String> = list(&config.0["meetingBrief"]["skip"]).iter().filter_map(Value::as_str).map(str::to_string).collect();
    let reading = Reading { emails: &emails, calendar_id: calendar_id.as_deref(), from, to, zone: Tz::LOCAL, skip: &skip };
    Ok(meetings(&calendar, &reading))
}

/// What reading a calendar needs besides the file: who I am, the window, the zone times are written in.
pub struct Reading<'a> {
    pub emails: &'a [String],
    pub calendar_id: Option<&'a str>,
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub zone: Tz,
    /// meetingBrief.skip: titles that get no brief.
    pub skip: &'a [String],
}

/// The calendar's timed events in the window, as meetings.json lists them: by start, without the cancelled, free and
/// declined ones; all-day events have no start time and drop out, as with the connector.
pub fn meetings(calendar: &Calendar, reading: &Reading) -> Vec<Value> {
    let floating = calendar.props.get("X-WR-TIMEZONE").and_then(|p| ics::zone(&p.value)).unwrap_or(reading.zone);
    let mut found: Vec<(DateTime<Utc>, Value)> =
        occurrences(calendar, floating, reading.from, reading.to).iter().filter_map(|o| meeting(o, reading).map(|m| (o.start, m))).collect();
    found.sort_by_key(|(start, _)| *start);
    let mut meetings: Vec<Value> = found.into_iter().map(|(_, m)| m).collect();
    mark_briefable(&mut meetings, reading.skip);
    meetings
}

/// One time an event happens: its start and end, and for a recurring one the start it has in the series.
struct Occurrence<'a> {
    event: &'a Props,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    instance: Option<DateTime<Utc>>,
}

fn overlaps(start: DateTime<Utc>, end: DateTime<Utc>, from: DateTime<Utc>, to: DateTime<Utc>) -> bool {
    start < to && (end > from || start >= from)
}

fn occurrences(calendar: &Calendar, floating: Tz, from: DateTime<Utc>, to: DateTime<Utc>) -> Vec<Occurrence<'_>> {
    let uid = |event: &Props| event.get("UID").map(|p| p.value.clone()).unwrap_or_default();
    let instant = |event: &Props, name: &str| event.get(name).and_then(|p| ics::when_of(p, floating)).and_then(|w| w.utc());
    // The instances an override replaces, by series and the start it had there.
    let overridden: HashSet<(String, i64)> =
        calendar.events.iter().filter_map(|e| Some((uid(e), instant(e, "RECURRENCE-ID")?.timestamp()))).collect();
    let mut out = Vec::new();
    for event in &calendar.events {
        let Some(When::At(start)) = event.get("DTSTART").and_then(|p| ics::when_of(p, floating)) else { continue };
        let end = match event.get("DTEND").and_then(|p| ics::when_of(p, floating)) {
            Some(When::At(end)) => end,
            _ => start + event.get("DURATION").and_then(|p| ics::duration(&p.value)).unwrap_or_default(),
        };
        let length = (end - start).max(Duration::zero());
        let utc = start.with_timezone(&Utc);
        if event.get("RECURRENCE-ID").is_some() {
            if overlaps(utc, utc + length, from, to) {
                out.push(Occurrence { event, start: utc, end: utc + length, instance: instant(event, "RECURRENCE-ID") });
            }
            continue;
        }
        let starts = if event.get("RRULE").is_some() || event.get("RDATE").is_some() { expand(event, start, length, from, to) } else { None };
        match starts {
            Some(starts) => {
                let series = uid(event);
                for at in starts.into_iter().map(|s| s.with_timezone(&Utc)) {
                    if !overridden.contains(&(series.clone(), at.timestamp())) {
                        out.push(Occurrence { event, start: at, end: at + length, instance: Some(at) });
                    }
                }
            }
            // A one-off, or a rule that does not read: the event once.
            None if overlaps(utc, utc + length, from, to) => out.push(Occurrence { event, start: utc, end: utc + length, instance: None }),
            None => {}
        }
    }
    out
}

/// The starts of a recurring event that overlap the window: its RRULE and RDATEs less its EXDATEs.
fn expand(event: &Props, start: DateTime<Tz>, length: Duration, from: DateTime<Utc>, to: DateTime<Utc>) -> Option<Vec<DateTime<Tz>>> {
    let tz = start.timezone();
    let mut set = match event.get("RRULE") {
        Some(rule) => until_in_utc(&rule.value, tz).parse::<RRule<Unvalidated>>().ok()?.build(start).ok()?,
        None => RRuleSet::new(start).rdate(start),
    };
    let at = |p: &Prop| ics::whens(p, tz).into_iter().filter_map(|w| w.utc()).map(move |t| t.with_timezone(&tz)).collect::<Vec<_>>();
    for date in event.all("RDATE").flat_map(at) {
        set = set.rdate(date);
    }
    for date in event.all("EXDATE").flat_map(at) {
        set = set.exdate(date);
    }
    let result = set.after((from - length).with_timezone(&tz)).before(to.with_timezone(&tz)).all(MAX_INSTANCES);
    Some(result.dates.into_iter().filter(|s| *s >= (from - length).with_timezone(&tz) && s.with_timezone(&Utc) < to).collect())
}

/// The rule with its UNTIL in UTC, as the rule reader wants beside a zoned start; a local or whole-day UNTIL is read
/// in the start's zone.
fn until_in_utc(rule: &str, tz: Tz) -> String {
    rule.split(';')
        .map(|part| {
            let Some(value) = part.strip_prefix("UNTIL=") else { return part.to_string() };
            if value.ends_with(['Z', 'z']) {
                return part.to_string();
            }
            let local = if value.len() == 8 { format!("{value}T235959") } else { value.to_string() };
            let prop = Prop { name: "UNTIL".into(), params: vec![], value: local };
            match ics::when_of(&prop, tz).and_then(|w| w.utc()) {
                Some(utc) => format!("UNTIL={}", utc.format("%Y%m%dT%H%M%SZ")),
                None => part.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join(";")
}

/// One occurrence as the mod writes a meeting; None for one left out (cancelled, free, declined).
fn meeting(o: &Occurrence, reading: &Reading) -> Option<Value> {
    let e = o.event;
    let is = |name: &str, value: &str| e.get(name).is_some_and(|p| p.value.trim().eq_ignore_ascii_case(value));
    if is("STATUS", "CANCELLED") || is("TRANSP", "TRANSPARENT") {
        return None;
    }
    let uid = e.get("UID").map(|p| p.value.trim().to_string()).unwrap_or_default();
    let base = uid.strip_suffix("@google.com").unwrap_or(&uid).to_string();
    let id = match o.instance {
        Some(at) => format!("{base}_{}", at.format("%Y%m%dT%H%M%SZ")),
        None => base.clone(),
    };
    let attendees: Vec<&Prop> = e.all("ATTENDEE").filter(|a| !is_resource(a)).collect();
    let is_me = |a: &Prop| reading.emails.iter().any(|m| m.eq_ignore_ascii_case(&address_of(a)));
    let me = attendees.iter().find(|a| is_me(a));
    let response = me.map(|a| response(a.param("PARTSTAT").unwrap_or("NEEDS-ACTION"))).unwrap_or_else(|| "accepted".into());
    if response == "declined" {
        return None;
    }
    let people: Vec<String> = attendees
        .iter()
        .filter(|a| !is_me(a) && !a.param("PARTSTAT").is_some_and(|p| p.eq_ignore_ascii_case("DECLINED")))
        .map(|a| a.param("CN").map(str::to_string).filter(|n| !n.is_empty()).unwrap_or_else(|| address_of(a)))
        .take(MAX_PEOPLE)
        .collect();
    let raw_description = e.text("DESCRIPTION").unwrap_or_default();
    let location = e.text("LOCATION").map(|l| l.trim().to_string()).unwrap_or_default();
    let join = e
        .text("X-GOOGLE-CONFERENCE")
        .map(|j| j.trim().to_string())
        .filter(|j| j.starts_with("http"))
        .or_else(|| location.starts_with("http").then(|| location.clone()))
        .or_else(|| CALL.find(&raw_description).map(|m| m.as_str().to_string()));
    let url = e.text("URL").map(|u| u.trim().to_string()).filter(|u| u.starts_with("http")).unwrap_or_else(|| match reading.calendar_id {
        Some(calendar) if uid.ends_with("@google.com") => format!("https://www.google.com/calendar/event?eid={}", base64(&format!("{id} {calendar}"))),
        _ => String::new(),
    });
    let mut m = Map::new();
    m.insert("id".into(), json!(id));
    m.insert("title".into(), json!(e.text("SUMMARY").map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).unwrap_or_else(|| "(no title)".into())));
    m.insert("start".into(), json!(iso(o.start, reading.zone)));
    m.insert("end".into(), json!(iso(o.end, reading.zone)));
    m.insert("url".into(), json!(url));
    if let Some(join) = join {
        m.insert("join".into(), json!(join));
    }
    m.insert("response".into(), json!(response));
    m.insert("attendees".into(), json!(attendees.len()));
    let description = clean_description(&raw_description);
    if !description.is_empty() {
        m.insert("description".into(), json!(description.chars().take(MAX_DESCRIPTION).collect::<String>()));
    }
    if o.instance.is_some() {
        m.insert("series".into(), json!(base));
    }
    m.insert("people".into(), json!(people));
    let attachments: Vec<Value> = e
        .all("ATTACH")
        .filter(|a| a.value.starts_with("http"))
        .map(|a| json!({"title": a.param("FILENAME").unwrap_or("file"), "url": a.value.trim()}))
        .collect();
    m.insert("attachments".into(), json!(attachments));
    Some(Value::Object(m))
}

/// A video call's link, as Google, Zoom, Teams, Webex and Whereby write them into a description.
static CALL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"https://(meet\.google\.com|([\w-]+\.)*zoom\.us|teams\.microsoft\.com|teams\.live\.com|([\w-]+\.)*webex\.com|whereby\.com)/[^\s<>"')\]]+"#)
        .expect("a valid pattern")
});

/// The block Google appends to a description for its call ("-::~:~::~…::-" lines around it) is the join link, not prose.
static GOOGLE_BLOCK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)-::~[:~]*::-.*?(-::~[:~]*::-|$)").expect("a valid pattern"));

fn clean_description(text: &str) -> String {
    GOOGLE_BLOCK.replace_all(text, "").trim().to_string()
}

fn iso(at: DateTime<Utc>, zone: Tz) -> String {
    at.with_timezone(&zone).fixed_offset().to_rfc3339_opts(SecondsFormat::Secs, false)
}

fn address_of(attendee: &Prop) -> String {
    let value = attendee.value.trim();
    value.strip_prefix("mailto:").or_else(|| value.strip_prefix("MAILTO:")).unwrap_or(value).to_lowercase()
}

fn is_resource(attendee: &Prop) -> bool {
    attendee.param("CUTYPE").is_some_and(|c| c.eq_ignore_ascii_case("RESOURCE") || c.eq_ignore_ascii_case("ROOM"))
        || address_of(attendee).ends_with("@resource.calendar.google.com")
}

/// PARTSTAT as the connector answers: accepted, tentative, declined, needsAction.
fn response(partstat: &str) -> String {
    match partstat.to_ascii_uppercase().as_str() {
        "NEEDS-ACTION" => "needsAction".into(),
        other => other.to_lowercase(),
    }
}

/// A series seen on three days or more of the week is a daily: it gets no brief (the mod's markBriefable).
pub fn mark_briefable(meetings: &mut [Value], skip: &[String]) {
    let mut days: HashMap<String, HashSet<String>> = HashMap::new();
    for m in meetings.iter() {
        if let Some(series) = m["series"].as_str() {
            days.entry(series.to_string()).or_default().insert(m["start"].as_str().unwrap_or_default().chars().take(10).collect());
        }
    }
    for m in meetings.iter_mut() {
        let is_daily = m["series"].as_str().is_some_and(|s| days.get(s).map_or(0, HashSet::len) >= 3);
        let title = m["title"].as_str().unwrap_or_default().to_lowercase();
        let is_skipped = skip.iter().any(|w| title.contains(&w.to_lowercase()));
        let has_people = m["people"].as_array().is_some_and(|p| !p.is_empty());
        m["briefable"] = json!(!is_daily && !is_skipped && has_people && m["response"] != "declined");
    }
}

/// meetings.json with the hub's fetch: the meetings it read, the notifications already sent for those still listed;
/// after a failure, the meetings it had beside the error.
pub fn merge(doc: &Value, fetched: Result<Vec<Value>, String>, now: u64) -> Value {
    match fetched {
        Ok(meetings) => {
            let live: HashSet<&str> = meetings.iter().filter_map(|m| m["id"].as_str()).collect();
            let notified: Vec<&Value> = list(&doc["notified"]).iter().filter(|id| id.as_str().is_some_and(|id| live.contains(id))).collect();
            json!({"at": now, "meetings": meetings, "notified": notified, "by": "hub"})
        }
        Err(error) => {
            let mut doc = if doc.is_object() { doc.clone() } else { json!({"at": 0, "meetings": [], "notified": []}) };
            doc["error"] = json!(error);
            doc
        }
    }
}

fn emails(value: &Value) -> Vec<String> {
    let all: Vec<&Value> = if value.is_array() { list(value).iter().collect() } else { vec![value] };
    all.into_iter().filter_map(Value::as_str).map(|e| e.trim().to_lowercase()).filter(|e| is_address(e)).collect()
}

fn is_address(text: &str) -> bool {
    text.contains('@') && !text.contains(char::is_whitespace)
}

/// The calendar a Google secret address names: …/calendar/ical/<calendar id>/private-…/basic.ics.
fn google_calendar_id(url: &str) -> Option<String> {
    let (_, rest) = url.split_once("/calendar/ical/")?;
    let id = percent_decode(rest.split('/').next()?);
    is_address(&id).then(|| id.to_lowercase())
}

fn git_email() -> Option<String> {
    let out = crate::proc::run(&crate::proc::strings(&["git", "config", "--global", "--get", "user.email"]), std::time::Duration::from_secs(5), None);
    let email = out.stdout.trim().to_lowercase();
    is_address(&email).then_some(email)
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = (bytes[i] == b'%').then(|| text.get(i + 1..i + 3)).flatten().and_then(|h| u8::from_str_radix(h, 16).ok());
        match hex {
            Some(byte) => {
                out.push(byte);
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Standard base64 without its padding, as Google's event links carry "<event id> <calendar id>".
fn base64(text: &str) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in text.as_bytes().chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, b)| n | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..=chunk.len() {
            out.push(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paris() -> Tz {
        ics::zone("Europe/Paris").unwrap()
    }

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text).unwrap().with_timezone(&Utc)
    }

    /// The fixture's two weeks, read as me.
    fn week() -> Vec<Value> {
        let calendar = ics::parse(include_str!("../../tests/fixtures/feed/calendar.ics"));
        let emails = ["me@acme.example".to_string()];
        let skip = ["lunch".to_string()];
        let reading = Reading {
            emails: &emails,
            calendar_id: Some("me@acme.example"),
            from: at("2026-11-02T00:00:00+01:00"),
            to: at("2026-11-21T00:00:00+01:00"),
            zone: paris(),
            skip: &skip,
        };
        meetings(&calendar, &reading)
    }

    fn by_title<'a>(meetings: &'a [Value], title: &str) -> Vec<&'a Value> {
        meetings.iter().filter(|m| m["title"] == title).collect()
    }

    #[test]
    fn a_weekly_series_less_its_exdate_with_its_moved_instance() {
        let week = week();
        let syncs: Vec<(&str, &str)> = week
            .iter()
            .filter(|m| m["series"] == "7kq2sync")
            .map(|m| (m["id"].as_str().unwrap(), m["start"].as_str().unwrap()))
            .collect();
        assert_eq!(
            syncs,
            [
                ("7kq2sync_20261103T090000Z", "2026-11-03T10:00:00+01:00"),
                // The 10th is an EXDATE; the 17th moved to the afternoon and keeps the id of its place in the series.
                ("7kq2sync_20261117T090000Z", "2026-11-17T15:00:00+01:00"),
            ]
        );
        let moved = by_title(&week, "Platform sync (moved)")[0];
        assert_eq!(moved["end"], "2026-11-17T15:30:00+01:00");
        let first = by_title(&week, "Platform sync")[0];
        assert_eq!(first["join"], "https://meet.google.com/abc-defg-hij");
        assert_eq!(first["response"], "accepted");
        assert_eq!(first["attendees"], 4, "everyone invited but the room");
        assert_eq!(first["people"], json!(["Maya Lopez", "sam@acme.example"]), "the one who declined left out");
        assert_eq!(first["description"], "Weekly platform sync.\nAgenda in the doc.", "Google's call block is the join link, not prose");
        assert_eq!(first["attachments"], json!([{"title": "Sync notes", "url": "https://docs.google.com/document/d/acme-sync-notes"}]));
        assert_eq!(first["url"], format!("https://www.google.com/calendar/event?eid={}", base64("7kq2sync_20261103T090000Z me@acme.example")));
    }

    #[test]
    fn all_day_cancelled_free_and_declined_events_stay_out() {
        let week = week();
        let titles: Vec<&str> = week.iter().map(|m| m["title"].as_str().unwrap()).collect();
        for gone in ["Acme offsite", "Vendor demo", "Focus block", "Billing retro"] {
            assert!(!titles.contains(&gone), "{gone} in {titles:?}");
        }
        assert_eq!(
            titles,
            [
                "Platform sync",
                "1:1 Maya / me",
                "Ledger standup",
                "Ledger standup",
                "Ledger standup",
                "Lunch with Priya",
                "Ledger standup",
                "Platform sync (moved)"
            ]
        );
    }

    #[test]
    fn a_zoned_event_in_my_zone_and_its_tentative_answer() {
        let week = week();
        let one = by_title(&week, "1:1 Maya / me")[0];
        // 09:00 in New York is 15:00 in Paris that day.
        assert_eq!((one["start"].as_str(), one["end"].as_str()), (Some("2026-11-04T15:00:00+01:00"), Some("2026-11-04T15:30:00+01:00")));
        assert_eq!(one["response"], "tentative");
        assert_eq!(one["join"], "https://acme.zoom.us/j/123456789", "a call link in the location");
        assert_eq!(one["id"], "oneonone-maya@acme.example", "a UID not Google's stays whole");
        assert_eq!(one["url"], "https://calendar.acme.example/e/1", "its own URL");
        assert!(one.get("series").is_none());
        assert_eq!(one["briefable"], true);
    }

    #[test]
    fn a_daily_and_the_skipped_titles_get_no_brief() {
        let week = week();
        let standups = by_title(&week, "Ledger standup");
        assert_eq!(standups.len(), 4, "COUNT=4, over four days");
        assert!(standups.iter().all(|m| m["briefable"] == false && m["response"] == "needsAction"));
        assert_eq!(standups[0]["join"], "https://meet.google.com/xyz-uvwx-rst", "the Meet link of the description");
        assert_eq!(by_title(&week, "Lunch with Priya")[0]["briefable"], false);
        assert_eq!(by_title(&week, "Platform sync")[0]["briefable"], true);
    }

    #[test]
    fn rules_and_addresses() {
        assert_eq!(until_in_utc("FREQ=WEEKLY;UNTIL=20261201T100000;BYDAY=TU", paris()), "FREQ=WEEKLY;UNTIL=20261201T090000Z;BYDAY=TU");
        assert_eq!(until_in_utc("FREQ=DAILY;UNTIL=20261201T085959Z", paris()), "FREQ=DAILY;UNTIL=20261201T085959Z");
        assert_eq!(
            google_calendar_id("https://calendar.google.com/calendar/ical/me%40acme.example/private-0123abcd/basic.ics").as_deref(),
            Some("me@acme.example")
        );
        assert_eq!(google_calendar_id("https://calendar.acme.example/feed.ics"), None);
        assert_eq!(address(&json!(" webcal://calendar.acme.example/feed.ics ")).as_deref(), Some("https://calendar.acme.example/feed.ics"));
        assert_eq!(address(&json!("")), None);
        assert_eq!(emails(&json!(["Me@Acme.example", "not an address"])), ["me@acme.example"]);
        assert_eq!(emails(&json!("me@acme.example")), ["me@acme.example"]);
        assert_eq!(base64("ab"), "YWI");
        assert_eq!(base64("abc d"), "YWJjIGQ");
    }

    #[test]
    fn a_fetch_keeps_the_notifications_of_meetings_still_listed_and_a_failure_keeps_the_meetings() {
        let before = json!({"at": 5, "meetings": [{"id": "a"}], "notified": ["a", "gone"], "error": "Calendar: old"});
        let after = merge(&before, Ok(vec![json!({"id": "a"}), json!({"id": "b"})]), 9);
        assert_eq!(after, json!({"at": 9, "meetings": [{"id": "a"}, {"id": "b"}], "notified": ["a"], "by": "hub"}));
        let failed = merge(&after, Err("Calendar: HTTP 404".into()), 12);
        assert_eq!((&failed["at"], &failed["meetings"][1]["id"]), (&json!(9), &json!("b")));
        assert_eq!((&failed["error"], &failed["by"]), (&json!("Calendar: HTTP 404"), &json!("hub")));
        assert_eq!(merge(&Value::Null, Err("Calendar: x".into()), 1)["meetings"], json!([]));
    }
}
