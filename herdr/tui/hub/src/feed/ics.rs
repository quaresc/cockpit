//! An iCalendar file read as the calendar's own properties and its VEVENTs' (RFC 5545): folded lines joined, parameters
//! split with their quotes, text values unescaped on demand, dates and date-times read in their TZID's zone.

use std::str::FromStr;

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, TimeZone, Utc};
use rrule::Tz;

#[derive(Clone, Debug, PartialEq)]
pub struct Prop {
    pub name: String,
    pub params: Vec<(String, String)>,
    pub value: String,
}

impl Prop {
    pub fn param(&self, name: &str) -> Option<&str> {
        self.params.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    /// The value as text: \n, \, \; and \\ unescaped.
    pub fn text(&self) -> String {
        let mut out = String::with_capacity(self.value.len());
        let mut chars = self.value.chars();
        while let Some(c) = chars.next() {
            if c != '\\' {
                out.push(c);
                continue;
            }
            match chars.next() {
                Some('n' | 'N') => out.push('\n'),
                Some(other) => out.push(other),
                None => out.push('\\'),
            }
        }
        out
    }
}

/// One component's properties, in file order; nested components (alarms) left out.
#[derive(Clone, Debug, Default)]
pub struct Props(pub Vec<Prop>);

impl Props {
    pub fn get(&self, name: &str) -> Option<&Prop> {
        self.0.iter().find(|p| p.name == name)
    }

    pub fn all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Prop> + 'a {
        self.0.iter().filter(move |p| p.name == name)
    }

    pub fn text(&self, name: &str) -> Option<String> {
        self.get(name).map(Prop::text)
    }
}

#[derive(Debug, Default)]
pub struct Calendar {
    pub props: Props,
    pub events: Vec<Props>,
}

/// The calendar's properties and events; whatever does not parse is skipped.
pub fn parse(source: &str) -> Calendar {
    let mut lines: Vec<String> = Vec::new();
    for raw in source.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        match (line.strip_prefix([' ', '\t']), lines.last_mut()) {
            (Some(rest), Some(last)) => last.push_str(rest),
            _ => lines.push(line.to_string()),
        }
    }
    let mut calendar = Calendar::default();
    let mut stack: Vec<String> = Vec::new();
    let mut event = Props::default();
    for line in lines.iter().filter(|l| !l.is_empty()) {
        let Some(prop) = property(line) else { continue };
        match prop.name.as_str() {
            "BEGIN" => {
                if prop.value.eq_ignore_ascii_case("VEVENT") && stack.last().is_some_and(|s| s == "VCALENDAR") {
                    event = Props::default();
                }
                stack.push(prop.value.to_ascii_uppercase());
            }
            "END" => {
                if stack.pop().is_some_and(|s| s == "VEVENT") && stack.last().is_some_and(|s| s == "VCALENDAR") {
                    calendar.events.push(std::mem::take(&mut event));
                }
            }
            _ => match stack.last().map(String::as_str) {
                Some("VEVENT") => event.0.push(prop),
                Some("VCALENDAR") => calendar.props.0.push(prop),
                _ => {}
            },
        }
    }
    calendar
}

/// NAME;PARAM=value;PARAM="quoted:value":value, the name and parameter names upper-cased.
fn property(line: &str) -> Option<Prop> {
    let end = line.find([';', ':'])?;
    let name = line[..end].to_ascii_uppercase();
    let mut rest = &line[end..];
    let mut params = Vec::new();
    while let Some(after) = rest.strip_prefix(';') {
        let eq = after.find('=')?;
        let key = after[..eq].to_ascii_uppercase();
        let tail = &after[eq + 1..];
        let (value, next) = match tail.strip_prefix('"') {
            Some(quoted) => {
                let close = quoted.find('"')?;
                (quoted[..close].to_string(), &quoted[close + 1..])
            }
            None => {
                let stop = tail.find([';', ':']).unwrap_or(tail.len());
                (tail[..stop].to_string(), &tail[stop..])
            }
        };
        params.push((key, value));
        rest = next;
    }
    let value = rest.strip_prefix(':')?.to_string();
    Some(Prop { name, params, value })
}

/// A DTSTART, DTEND, RECURRENCE-ID, EXDATE or RDATE value: a whole day, or an instant in its zone.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum When {
    Day(NaiveDate),
    At(DateTime<Tz>),
}

impl When {
    pub fn utc(&self) -> Option<DateTime<Utc>> {
        match self {
            When::At(at) => Some(at.with_timezone(&Utc)),
            When::Day(_) => None,
        }
    }
}

/// An IANA zone name as Google and most calendars write TZID; None for anything else (Windows names).
pub fn zone(name: &str) -> Option<Tz> {
    let name = name.trim().trim_matches('"');
    let name = name.strip_prefix("/mozilla.org/20050126_1/").or_else(|| name.strip_prefix("/freeassociation.sourceforge.net/")).unwrap_or(name);
    if name.eq_ignore_ascii_case("UTC") || name.eq_ignore_ascii_case("Etc/UTC") || name == "Z" {
        return Some(Tz::UTC);
    }
    chrono_tz::Tz::from_str(name).ok().map(Tz::Tz)
}

/// The values of a property, a list for EXDATE and RDATE: Z is UTC, a TZID its zone when known, else `floating`.
pub fn whens(prop: &Prop, floating: Tz) -> Vec<When> {
    let tz = prop.param("TZID").and_then(zone).unwrap_or(floating);
    prop.value.split(',').filter_map(|v| when(v.trim(), tz)).collect()
}

pub fn when_of(prop: &Prop, floating: Tz) -> Option<When> {
    whens(prop, floating).into_iter().next()
}

fn when(value: &str, tz: Tz) -> Option<When> {
    if value.len() == 8 {
        return NaiveDate::parse_from_str(value, "%Y%m%d").ok().map(When::Day);
    }
    let (value, is_utc) = match value.strip_suffix(['Z', 'z']) {
        Some(v) => (v, true),
        None => (value, false),
    };
    let naive = NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S").ok()?;
    if is_utc {
        return Some(When::At(Utc.from_utc_datetime(&naive).with_timezone(&Tz::UTC)));
    }
    // A time skipped by a clock change is read an hour later; a repeated one as its first occurrence.
    let at = tz.from_local_datetime(&naive).earliest().or_else(|| tz.from_local_datetime(&(naive + Duration::hours(1))).earliest())?;
    Some(When::At(at))
}

/// A DURATION value (P1D, PT1H30M, -PT15M, P2W).
pub fn duration(value: &str) -> Option<Duration> {
    let (sign, rest) = match value.strip_prefix('-') {
        Some(r) => (-1, r),
        None => (1, value.strip_prefix('+').unwrap_or(value)),
    };
    let rest = rest.strip_prefix('P')?;
    let (mut total, mut number, mut is_time) = (0i64, String::new(), false);
    for c in rest.chars() {
        match c {
            '0'..='9' => number.push(c),
            'T' => is_time = true,
            unit => {
                let n: i64 = std::mem::take(&mut number).parse().ok()?;
                total += n * match (unit, is_time) {
                    ('W', false) => 7 * 86_400,
                    ('D', false) => 86_400,
                    ('H', true) => 3_600,
                    ('M', true) => 60,
                    ('S', true) => 1,
                    _ => return None,
                };
            }
        }
    }
    Some(Duration::seconds(sign * total))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folded_lines_quoted_params_and_escapes() {
        let calendar = parse(
            "BEGIN:VCALENDAR\r\nX-WR-CALNAME:me@acme.example\r\nBEGIN:VEVENT\r\nSUMMARY:Design\\, review\\; v2\r\nDESCRIPTION:line one\\nline\r\n  two\r\n\
             ATTENDEE;CN=\"Ortiz, Sam\";PARTSTAT=ACCEPTED:mailto:sam@acme.example\r\n\
             BEGIN:VALARM\r\nSUMMARY:alarm\r\nEND:VALARM\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
        );
        assert_eq!(calendar.props.text("X-WR-CALNAME").as_deref(), Some("me@acme.example"));
        let event = &calendar.events[0];
        assert_eq!(event.text("SUMMARY").as_deref(), Some("Design, review; v2"));
        assert_eq!(event.text("DESCRIPTION").as_deref(), Some("line one\nline two"));
        let attendee = event.get("ATTENDEE").unwrap();
        assert_eq!((attendee.param("cn"), attendee.param("PARTSTAT")), (Some("Ortiz, Sam"), Some("ACCEPTED")));
        assert_eq!(attendee.value, "mailto:sam@acme.example");
        assert_eq!(event.all("SUMMARY").count(), 1, "the alarm's properties stay out");
    }

    #[test]
    fn times_in_their_zone_days_and_durations() {
        let paris = zone("Europe/Paris").unwrap();
        let prop = property("DTSTART;TZID=Europe/Paris:20261103T100000").unwrap();
        assert_eq!(when_of(&prop, Tz::UTC).unwrap().utc().unwrap().to_rfc3339(), "2026-11-03T09:00:00+00:00");
        let utc = property("DTSTART:20261103T100000Z").unwrap();
        assert_eq!(when_of(&utc, paris).unwrap().utc().unwrap().to_rfc3339(), "2026-11-03T10:00:00+00:00");
        let day = property("DTSTART;VALUE=DATE:20261104").unwrap();
        assert_eq!(when_of(&day, paris), Some(When::Day(NaiveDate::from_ymd_opt(2026, 11, 4).unwrap())));
        let list = property("EXDATE;TZID=Europe/Paris:20261110T100000,20261124T100000").unwrap();
        assert_eq!(whens(&list, Tz::UTC).len(), 2);
        assert_eq!(zone("Romance Standard Time"), None);
        assert_eq!(duration("PT1H30M"), Some(Duration::minutes(90)));
        assert_eq!(duration("P1DT2H"), Some(Duration::hours(26)));
        assert_eq!(duration("-PT15M"), Some(Duration::minutes(-15)));
        assert_eq!(duration("1H"), None);
    }
}
