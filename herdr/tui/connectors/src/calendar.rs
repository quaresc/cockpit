//! My coming meetings through a calendar connector's mapping, the hub's twin of the plugin's `fetchMeetings`
//! (meetings.ts): the list meetings.json holds, before the hub marks which ones get a brief.

use std::collections::HashSet;

use serde_json::{Map, Value, json};

use crate::mapping::{list_at, pick, text};
use crate::tools::Tools;

/// Small pages, so no answer outgrows a tool's size limit; eight of them cover a busy week.
const MAX_PAGES: usize = 8;
const MAX_DESCRIPTION: usize = 2000;
const MAX_PEOPLE: usize = 15;

/// The Google Calendar connector's mapping, the default for a config that names no other (config.ts `GOOGLE_CALENDAR`).
pub fn google() -> Value {
    json!({
        "enabled": false,
        "name": "Google Calendar",
        "server": "claude.ai Google Calendar",
        "tools": {
            "listEvents": {"tool": "list_events", "args": {"calendarId": "{calendar}", "orderBy": "startTime", "pageSize": 12, "pageToken": "{pageToken}"}},
            "getEvent": {"tool": "get_event", "args": {"calendarId": "{calendar}", "eventId": "{event}"}}
        },
        "fields": {
            "events": "events", "nextPage": "nextPageToken", "id": "id", "title": "summary", "start": "start.dateTime", "end": "end.dateTime",
            "url": "htmlLink", "join": "conferenceUrl|hangoutLink", "location": "location", "description": "description", "series": "recurringEventId",
            "attendees": "attendees", "attendeeSelf": "self", "attendeeResponse": "responseStatus", "attendeeResource": "resource",
            "attendeeName": "displayName|email", "attachments": "attachments", "attachmentTitle": "title", "attachmentUrl": "fileUrl"
        },
        "skip": {"status": ["cancelled"], "transparency": ["transparent"]},
        "only": {"eventType": ["", "DEFAULT"]},
        "responses": {}
    })
}

/// The config's calendar block over the Google default, key by key as the plugin's loadConfig spreads it: a block
/// that gives `fields` replaces all of them.
pub fn mapping(section: &Value) -> Value {
    let mut merged = google();
    if let (Some(base), Some(over)) = (merged.as_object_mut(), section.as_object()) {
        base.extend(over.clone());
    }
    merged
}

/// My coming timed events, in the plugin's `Meeting` shape, without the declined ones and those the mapping leaves out
/// (cancelled, free, out of office); an all-day marker has no start time and drops out too. The config's `calendars`
/// are read in turn, an event on several of them kept from the first, and each meeting names the calendar it came from.
pub async fn meetings(tools: &impl Tools, calendar: &Value) -> Result<Vec<Value>, String> {
    let named: Vec<&str> = calendar["calendars"].as_array().into_iter().flatten().filter_map(Value::as_str).filter(|c| !c.is_empty()).collect();
    if named.is_empty() {
        return from_calendar(tools, calendar, None).await;
    }
    let mut seen = HashSet::new();
    let mut all = Vec::new();
    for id in &named {
        let meetings = from_calendar(tools, calendar, Some(id)).await.map_err(|e| format!("calendar {id}: {e}"))?;
        for mut m in meetings {
            let key = m["id"].as_str().unwrap_or_default().to_string();
            if key.is_empty() || seen.insert(key) {
                m["calendar"] = json!(id);
                all.push(m);
            }
        }
    }
    // Calendars answer in their own time zones, so the merge sorts by instant; a start that does not parse goes last.
    let instant = |m: &Value| chrono::DateTime::parse_from_rfc3339(m["start"].as_str().unwrap_or_default()).map_or(i64::MAX, |t| t.timestamp());
    all.sort_by_key(instant);
    Ok(all)
}

/// The meetings of one calendar, or of the server's default one when `id` is none.
async fn from_calendar(tools: &impl Tools, calendar: &Value, id: Option<&str>) -> Result<Vec<Value>, String> {
    let f = |name: &str| calendar["fields"][name].as_str().unwrap_or("").to_string();
    let mut answers = Vec::new();
    let mut page_token: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let mut values = Map::new();
        if let Some(id) = id {
            values.insert("calendar".into(), json!(id));
        }
        if let Some(token) = &page_token {
            values.insert("pageToken".into(), json!(token));
        }
        let body = tools.call("listEvents", &Value::Object(values)).await?;
        page_token = text(pick(&body, &f("nextPage"))).filter(|t| !t.is_empty());
        answers.push(body);
        if page_token.is_none() {
            break;
        }
    }
    let present = |e: &Value, name: &str| text(pick(e, &f(name))).filter(|t| !t.is_empty()).is_some();
    Ok(answers
        .iter()
        .flat_map(|body| list_at(body, &f("events")))
        .filter(|e| present(e, "start") && present(e, "end") && !is_skipped(e, calendar))
        .map(|e| meeting(e, calendar))
        .filter(|m| m["response"] != "declined")
        .collect())
}

/// Whether the mapping's `skip` names one of the event's values, or its `only` misses one.
fn is_skipped(event: &Value, calendar: &Value) -> bool {
    let value = |path: &str| text(pick(event, path)).unwrap_or_default();
    let names = |values: &Value, v: &str| values.as_array().is_some_and(|list| list.iter().any(|x| x.as_str() == Some(v)));
    let entries = |key: &str| calendar[key].as_object().map(|m| m.iter().map(|(p, v)| (p.clone(), v.clone())).collect::<Vec<_>>()).unwrap_or_default();
    entries("skip").iter().any(|(path, values)| names(values, &value(path))) || entries("only").iter().any(|(path, values)| !names(values, &value(path)))
}

/// An answer as cockpit's responses: the mapping's `responses` renames it, and no answer is an accepted one.
fn response(calendar: &Value, value: Option<&Value>) -> String {
    let raw = text(value).unwrap_or_else(|| "accepted".into());
    calendar["responses"][&raw].as_str().map(str::to_string).unwrap_or(raw)
}

fn meeting(e: &Value, calendar: &Value) -> Value {
    let f = |name: &str| calendar["fields"][name].as_str().unwrap_or("").to_string();
    let get = |node: &Value, name: &str| text(pick(node, &f(name)));
    let attendees = list_at(e, &f("attendees"));
    let me = attendees.iter().copied().find(|a| pick(a, &f("attendeeSelf")) == Some(&Value::Bool(true)));
    let others: Vec<&Value> = attendees.iter().copied().filter(|a| pick(a, &f("attendeeResource")) != Some(&Value::Bool(true))).collect();
    let location = get(e, "location");
    let join = get(e, "join").or_else(|| location.filter(|l| l.starts_with("http")));
    let my_answer = pick(e, &f("myResponse")).or_else(|| me.and_then(|m| pick(m, &f("attendeeResponse"))));
    let people: Vec<String> = others
        .iter()
        .filter(|a| !me.is_some_and(|m| std::ptr::eq(m, **a)) && response(calendar, pick(a, &f("attendeeResponse"))) != "declined")
        .map(|a| get(a, "attendeeName").unwrap_or_else(|| "?".into()))
        .take(MAX_PEOPLE)
        .collect();
    let attachments: Vec<Value> = list_at(e, &f("attachments"))
        .into_iter()
        .map(|a| (get(a, "attachmentTitle").unwrap_or_else(|| "file".into()), get(a, "attachmentUrl").unwrap_or_default()))
        .filter(|(_, url)| !url.is_empty())
        .map(|(title, url)| json!({"title": title, "url": url}))
        .collect();
    let mut m = Map::new();
    m.insert("id".into(), json!(get(e, "id").unwrap_or_default()));
    m.insert("title".into(), json!(get(e, "title").unwrap_or_else(|| "(no title)".into()).trim()));
    m.insert("start".into(), json!(get(e, "start").unwrap_or_default()));
    m.insert("end".into(), json!(get(e, "end").unwrap_or_default()));
    m.insert("url".into(), json!(get(e, "url").unwrap_or_default()));
    if let Some(join) = join {
        m.insert("join".into(), json!(join));
    }
    m.insert("response".into(), json!(response(calendar, my_answer)));
    m.insert("attendees".into(), json!(others.len()));
    if let Some(description) = get(e, "description") {
        m.insert("description".into(), json!(description.chars().take(MAX_DESCRIPTION).collect::<String>()));
    }
    if let Some(series) = get(e, "series") {
        m.insert("series".into(), json!(series));
    }
    m.insert("people".into(), json!(people));
    m.insert("attachments".into(), json!(attachments));
    Value::Object(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::Fake;

    fn person(email: &str, name: Option<&str>, response: &str) -> Value {
        let mut a = json!({"email": email, "responseStatus": response});
        if let Some(name) = name {
            a["displayName"] = json!(name);
        }
        a
    }

    fn event(id: &str, title: &str, start: &str, end: &str) -> Value {
        json!({
            "id": id, "status": "confirmed", "summary": title, "eventType": "DEFAULT",
            "htmlLink": format!("https://www.google.com/calendar/event?eid={id}"),
            "start": {"dateTime": start, "timeZone": "Europe/Paris"}, "end": {"dateTime": end, "timeZone": "Europe/Paris"}
        })
    }

    /// Two pages of a week, as the Google Calendar connector answers `list_events`.
    fn fake() -> Fake {
        let mut sync = event("7kq2sync_20261103T090000Z", " Platform sync ", "2026-11-03T10:00:00+01:00", "2026-11-03T10:30:00+01:00");
        sync["recurringEventId"] = json!("7kq2sync");
        sync["hangoutLink"] = json!("https://meet.google.com/abc-defg-hij");
        sync["description"] = json!("x".repeat(2500));
        sync["attendees"] = json!([
            {"email": "me@acme.example", "self": true, "responseStatus": "accepted"},
            person("maya@acme.example", Some("Maya Lopez"), "accepted"),
            person("sam@acme.example", None, "needsAction"),
            person("lee@acme.example", Some("Lee"), "declined"),
            {"email": "c_room@resource.calendar.google.com", "displayName": "Room 4", "resource": true, "responseStatus": "accepted"}
        ]);
        sync["attachments"] = json!([
            {"title": "Sync notes", "fileUrl": "https://docs.google.com/document/d/acme-sync-notes", "mimeType": "application/vnd.google-apps.document"},
            {"title": "No link"}
        ]);
        let mut one = event("oneonone", "1:1 Maya / me", "2026-11-04T15:00:00+01:00", "2026-11-04T15:30:00+01:00");
        one["location"] = json!("https://acme.zoom.us/j/123456789");
        one["attendees"] =
            json!([{"email": "me@acme.example", "self": true, "responseStatus": "tentative"}, person("maya@acme.example", Some("Maya Lopez"), "accepted")]);
        let mut cancelled = event("gone", "Vendor demo", "2026-11-04T11:00:00+01:00", "2026-11-04T12:00:00+01:00");
        cancelled["status"] = json!("cancelled");
        let mut free = event("focus", "Focus block", "2026-11-04T13:00:00+01:00", "2026-11-04T14:00:00+01:00");
        free["transparency"] = json!("transparent");
        let mut away = event("ooo", "Out of office", "2026-11-05T09:00:00+01:00", "2026-11-05T18:00:00+01:00");
        away["eventType"] = json!("OUT_OF_OFFICE");
        let all_day = json!({"id": "offsite", "summary": "Acme offsite", "start": {"date": "2026-11-06"}, "end": {"date": "2026-11-07"}});
        let mut declined = event("retro", "Billing retro", "2026-11-05T16:00:00+01:00", "2026-11-05T17:00:00+01:00");
        declined["attendees"] =
            json!([{"email": "me@acme.example", "self": true, "responseStatus": "declined"}, person("maya@acme.example", None, "accepted")]);
        let mut room = event("hall", "Town hall", "2026-11-06T17:00:00+01:00", "2026-11-06T18:00:00+01:00");
        room["location"] = json!("Paris HQ, floor 3");
        Fake::default()
            .with("listEvents", json!({"kind": "calendar#events", "events": [sync, one, cancelled, free], "nextPageToken": "page-2"}))
            .with("listEvents", json!({"kind": "calendar#events", "events": [away, all_day, declined, room]}))
    }

    #[test]
    fn pages_until_no_next_token_and_leaves_out_skipped_and_declined_events() {
        let fake = fake();
        let meetings = crate::block_on(meetings(&fake, &mapping(&json!({"enabled": true})))).unwrap();
        let titles: Vec<&str> = meetings.iter().map(|m| m["title"].as_str().unwrap()).collect();
        assert_eq!(titles, ["Platform sync", "1:1 Maya / me", "Town hall"]);
        let calls = fake.calls.borrow();
        assert_eq!(calls.iter().map(|(_, v)| v.clone()).collect::<Vec<_>>(), [json!({}), json!({"pageToken": "page-2"})]);
    }

    #[test]
    fn a_meeting_carries_its_people_attachments_and_call() {
        let meetings = crate::block_on(meetings(&fake(), &mapping(&json!({})))).unwrap();
        let sync = &meetings[0];
        assert_eq!(sync["id"], "7kq2sync_20261103T090000Z");
        assert_eq!((&sync["start"], &sync["end"]), (&json!("2026-11-03T10:00:00+01:00"), &json!("2026-11-03T10:30:00+01:00")));
        assert_eq!(sync["url"], "https://www.google.com/calendar/event?eid=7kq2sync_20261103T090000Z");
        assert_eq!(sync["join"], "https://meet.google.com/abc-defg-hij");
        assert_eq!(sync["response"], "accepted");
        assert_eq!(sync["attendees"], 4, "everyone invited but the room");
        assert_eq!(sync["people"], json!(["Maya Lopez", "sam@acme.example"]), "not me, not the one who declined");
        assert_eq!(sync["attachments"], json!([{"title": "Sync notes", "url": "https://docs.google.com/document/d/acme-sync-notes"}]));
        assert_eq!(sync["series"], "7kq2sync");
        assert_eq!(sync["description"].as_str().unwrap().len(), 2000);

        let one = &meetings[1];
        assert_eq!(one["join"], "https://acme.zoom.us/j/123456789", "a call link in the location");
        assert_eq!(one["response"], "tentative");
        assert!(one.get("series").is_none() && one.get("description").is_none());

        let hall = &meetings[2];
        assert!(hall.get("join").is_none(), "a place is no call");
        assert_eq!((&hall["response"], &hall["attendees"], &hall["people"]), (&json!("accepted"), &json!(0), &json!([])));
    }

    #[test]
    fn a_config_block_replaces_whole_keys_and_renames_answers() {
        let section = json!({"enabled": true, "only": {}, "responses": {"needsAction": "declined"}});
        let merged = mapping(&section);
        assert_eq!((&merged["enabled"], &merged["fields"]["title"]), (&json!(true), &json!("summary")));
        let meetings = crate::block_on(meetings(&fake(), &merged)).unwrap();
        let titles: Vec<&str> = meetings.iter().map(|m| m["title"].as_str().unwrap()).collect();
        assert_eq!(titles, ["Platform sync", "1:1 Maya / me", "Out of office", "Town hall"], "no `only`: out of office stays");
        assert_eq!(meetings[0]["people"], json!(["Maya Lopez"]), "sam's answer reads as declined");
    }

    #[test]
    fn reads_each_calendar_named_and_merges_them_by_start() {
        let mut shared = event("oneonone", "1:1 Maya / me", "2026-11-04T15:00:00+01:00", "2026-11-04T15:30:00+01:00");
        shared["attendees"] = json!([{"email": "me@acme.example", "self": true, "responseStatus": "accepted"}]);
        let primary = json!({"events": [event("standup", "Standup", "2026-11-04T09:00:00+01:00", "2026-11-04T09:15:00+01:00"), shared.clone()]});
        let mut early = event("review", "Design review", "2026-11-04T08:30:00Z", "2026-11-04T09:00:00Z");
        early["attendees"] = json!([{"email": "me@acme.example", "self": true, "responseStatus": "needsAction"}]);
        shared["attendees"][0]["responseStatus"] = json!("declined");
        let fake = Fake::default().with("listEvents", primary).with("listEvents", json!({"events": [shared, early]}));
        let section = json!({"calendars": ["primary", "team@acme.example"]});
        let meetings = crate::block_on(meetings(&fake, &mapping(&section))).unwrap();
        let read: Vec<(&str, &str)> = meetings.iter().map(|m| (m["title"].as_str().unwrap(), m["calendar"].as_str().unwrap())).collect();
        assert_eq!(
            read,
            [("Standup", "primary"), ("Design review", "team@acme.example"), ("1:1 Maya / me", "primary")],
            "09:30 Paris comes after 09:00 Paris, and the 1:1 stays as primary has it"
        );
        let calls: Vec<Value> = fake.calls.borrow().iter().map(|(_, v)| v.clone()).collect();
        assert_eq!(calls, [json!({"calendar": "primary"}), json!({"calendar": "team@acme.example"})]);
    }

    #[test]
    fn without_calendars_the_server_reads_its_default_one() {
        let fake = fake();
        let meetings = crate::block_on(meetings(&fake, &google())).unwrap();
        assert!(meetings.iter().all(|m| m.get("calendar").is_none()));
        let args = crate::mapping::fill_args(&google()["tools"]["listEvents"]["args"], &Map::new()).unwrap();
        assert!(args.get("calendarId").is_none(), "no calendarId argument at all");
    }

    #[test]
    fn a_calendar_that_fails_is_named() {
        let fake = Fake::default().with("listEvents", json!({"events": []})).with("listEvents", json!({"_error": "not found"}));
        let error = crate::block_on(meetings(&fake, &mapping(&json!({"calendars": ["primary", "typo@acme.example"]})))).unwrap_err();
        assert_eq!(error, "calendar typo@acme.example: not found");
    }

    #[test]
    fn stops_after_eight_pages() {
        let fake = Fake::default().with("listEvents", json!({"events": [], "nextPageToken": "more"}));
        assert!(crate::block_on(meetings(&fake, &google())).unwrap().is_empty());
        assert_eq!(fake.calls.borrow().len(), MAX_PAGES);
    }
}
