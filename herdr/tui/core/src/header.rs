//! The header every space draws: its title line, then a chip per source it reads saying how fresh that is ("GitHub · 2
//! min ago", "Linear · sign in again"), with the key that fixes a failing one. One space runs per process, so the chips
//! drawn last are the ones `L` and the `?` overlay act on.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use chrono::TimeZone;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::text::{Line, Span};
use serde_json::{Value, json};

use crate::fresh::{self, Freshness, Kind, State};
use crate::text::width;
use crate::theme::{ORANGE, RED, bold, dim, fg};

/// The key that signs in again, the same in every space.
pub const SIGN_IN: char = 'L';
/// The key that reads again: each space's own refresh.
pub const REFRESH: char = 'R';

/// The order sources are drawn in, whichever document holds them.
const ORDER: [&str; 7] = ["github", "tickets", "ci", "platform", "calendar", "mail", "monitor"];
const GAP: &str = "   ";
/// How long the outcome of a press stays on the header.
const NOTE_FOR: Duration = Duration::from_secs(8);

/// A source as a document's hub record (`hub.fresh.<source>`) holds it: its freshness, its name, how it signs in again.
#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    pub fresh: Freshness,
    pub label: String,
    /// The command that signs in again, as the person would type it.
    pub login: Option<String>,
    /// When the document's data last came in another way (a Claude session's plugin), in ms.
    pub elsewhere: Option<u64>,
}

/// The sources these documents' hub records name, in the order every space draws them.
pub fn sources(docs: &[&Value]) -> Vec<Source> {
    let mut out: Vec<Source> = Vec::new();
    for doc in docs {
        // When the plugin brought this source in: the inbox says it per source, a document of one source by its writer.
        let written = doc["at"].as_u64().filter(|_| doc["by"].as_str().is_some_and(|by| by != "hub"));
        for (id, value) in doc["hub"]["fresh"].as_object().into_iter().flatten() {
            let elsewhere = doc["modFetched"][id.as_str()].as_u64().or(written);
            let mut fresh = Freshness::from_json(value);
            if fresh.source.is_empty() {
                fresh.source = id.clone();
            }
            let label = value["label"].as_str().filter(|l| !l.is_empty()).unwrap_or(id).to_string();
            out.push(Source { fresh, label, login: value["login"].as_str().map(str::to_string), elsewhere });
        }
    }
    out.sort_by_key(|s| ORDER.iter().position(|o| *o == s.fresh.source).unwrap_or(ORDER.len()));
    out
}

/// Whether these sources hold this one: its errors are told by its chip, not by a line of their own.
pub fn has(sources: &[Source], id: &str) -> bool {
    sources.iter().any(|s| s.fresh.source == id)
}

/// Whether a chip tells the hub's error line (`GitHub: …`, `Linear: …`, `CI: …`): the line of a source no chip shows
/// stays.
pub fn covers(sources: &[Source], line: &str) -> bool {
    let named = line.split(':').next().unwrap_or_default().trim().to_ascii_lowercase();
    let source = match named.as_str() {
        "github" | "code" => "github",
        "linear" | "tickets" => "tickets",
        "ci" | "circleci" => "ci",
        "platform" => "platform",
        _ => return false,
    };
    has(sources, source)
}

/// The view models' `sources`: each one's freshness as `Freshness::json` gives it, with its label and login.
pub fn json(sources: &[Source], now: u64) -> Value {
    Value::Array(
        sources
            .iter()
            .map(|s| {
                let mut v = s.fresh.json(now);
                v["label"] = json!(s.label);
                v["login"] = json!(s.login);
                v["elsewhere"] = json!(s.elsewhere);
                // The build's own time, so a desk on another machine says ages by this one's clock.
                v["now"] = json!(now);
                v
            })
            .collect(),
    )
}

/// How loud a chip is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Quiet,
    /// Old data: drawn dim, the word "stale" says it.
    Warn,
    Bad,
}

/// What a chip's key does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fix {
    /// `cockpit connector login` (or `gh auth login`) in a tab of the actions space.
    SignIn,
    /// The space's refresh: every source is read again at once, a rate-limited one at its reset.
    Refresh,
}

impl Fix {
    pub fn key(self) -> char {
        match self {
            Fix::SignIn => SIGN_IN,
            Fix::Refresh => REFRESH,
        }
    }
}

/// One source's chip.
#[derive(Clone, Debug, PartialEq)]
pub struct Chip {
    pub source: String,
    pub label: String,
    /// "GitHub · 2 min ago", "Linear · sign in again", "CI · offline since 12:04".
    pub text: String,
    pub tone: Tone,
    pub fix: Option<Fix>,
    /// The exact error and its age, or how old the data is: the `?` overlay's words.
    pub detail: String,
}

/// The chip of a source at `now`, its times written by `clock`.
pub fn chip(source: &Source, now: u64, clock: &dyn Fn(u64) -> String) -> Chip {
    let f = &source.fresh;
    let read = f.at.map(|at| format!("last read {}", fresh::ago(at, now))).unwrap_or_else(|| "never read".into());
    let (words, tone, fix, detail) = match (f.state(now), &f.error) {
        // The plugin brought the data in after the hub's own read failed: it is not old, only read another way.
        (State::Failing, Some((_, message, since))) if source.elsewhere.is_some_and(|t| t > *since) => {
            let t = source.elsewhere.unwrap_or_default();
            (fresh::ago(t, now), Tone::Quiet, None, format!("came in through a Claude session at {}; the hub's own read fails: {message}", clock(t)))
        }
        (State::Failing, Some((kind, message, since))) => {
            let since = clock(*since);
            let (words, tone, fix) = match kind {
                Kind::Auth => ("sign in again".to_string(), Tone::Bad, Fix::SignIn),
                Kind::Rate => (f.next.map(|n| format!("limited until {}", clock(n))).unwrap_or_else(|| "rate limited".into()), Tone::Bad, Fix::Refresh),
                Kind::Network => (format!("offline since {since}"), Tone::Bad, Fix::Refresh),
                Kind::NotFound => (format!("not found since {since}"), Tone::Bad, Fix::Refresh),
                Kind::Server => (format!("server error since {since}"), Tone::Bad, Fix::Refresh),
                Kind::Other => (format!("failing since {since}"), Tone::Bad, Fix::Refresh),
            };
            (words, tone, Some(fix), format!("{message} · since {since} · {read}"))
        }
        (State::Stale, _) => {
            let ago = f.at.map(|at| fresh::ago(at, now)).unwrap_or_default();
            let every = f.every.map(|e| format!(", read every {}", span(e))).unwrap_or_default();
            (format!("stale, {ago}"), Tone::Warn, Some(Fix::Refresh), format!("not read again when due{every} · {read}"))
        }
        (State::Never, _) => ("not read yet".to_string(), Tone::Quiet, None, read),
        _ => (f.at.map(|at| fresh::ago(at, now)).unwrap_or_default(), Tone::Quiet, None, read),
    };
    Chip { source: f.source.clone(), label: source.label.clone(), text: format!("{} · {words}", source.label), tone, fix, detail }
}

/// "30 s", "5 min", "2 h": an interval in words.
fn span(ms: u64) -> String {
    match ms / 1000 {
        s @ 0..=59 => format!("{s} s"),
        s @ 60..=3599 => format!("{} min", s / 60),
        s => format!("{} h", s / 3600),
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// The chips this space drew last, for its keys and its `?` overlay

static SHOWN: Mutex<Vec<Chip>> = Mutex::new(Vec::new());
static NOTE: Mutex<Option<(String, bool, Instant)>> = Mutex::new(None);
static FROZEN: AtomicU64 = AtomicU64::new(0);

/// Draws every header at this time, in UTC, whatever the clock says: for snapshot tests.
pub fn freeze(now_ms: u64) {
    FROZEN.store(now_ms, Ordering::Relaxed);
}

/// Now, in ms: the frozen time in snapshot tests.
pub fn now() -> u64 {
    match FROZEN.load(Ordering::Relaxed) {
        0 => std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0),
        frozen => frozen,
    }
}

/// HH:MM, local (UTC once frozen).
pub fn clock(ms: u64) -> String {
    let shown = if FROZEN.load(Ordering::Relaxed) == 0 {
        chrono::Local.timestamp_millis_opt(ms as i64).single().map(|t| t.format("%H:%M").to_string())
    } else {
        chrono::Utc.timestamp_millis_opt(ms as i64).single().map(|t| t.format("%H:%M").to_string())
    };
    shown.unwrap_or_default()
}

fn shown() -> std::sync::MutexGuard<'static, Vec<Chip>> {
    SHOWN.lock().unwrap_or_else(|e| e.into_inner())
}

/// A note on the header for `NOTE_FOR`, then a frame drawn without it.
fn note(text: String, bad: bool) {
    *NOTE.lock().unwrap_or_else(|e| e.into_inner()) = Some((text, bad, Instant::now()));
    std::thread::spawn(|| {
        std::thread::sleep(NOTE_FOR + Duration::from_millis(50));
        crate::term::redraw();
    });
}

/// The `?` overlay's rows for the chips that need something: each key with what it does and the exact error.
pub fn help_rows() -> Vec<(String, String)> {
    shown()
        .iter()
        .filter_map(|c| {
            let fix = c.fix?;
            let what = match fix {
                Fix::SignIn => format!("sign in again to {}", c.label),
                Fix::Refresh if c.text.contains("limited until") => format!("read {} again (it waits for its limit)", c.label),
                Fix::Refresh => format!("read {} again", c.label),
            };
            Some((fix.key().to_string(), format!("{what} · {}", c.detail)))
        })
        .collect()
}

/// `L` signs in again to the first source whose login was refused, through the hub's `sources sign_in`, which opens a
/// tab of the actions space; true when the key was the header's.
pub fn key(key: &KeyEvent) -> bool {
    if key.code != KeyCode::Char(SIGN_IN) || key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
        return false;
    }
    let Some(chip) = shown().iter().find(|c| c.fix == Some(Fix::SignIn)).cloned() else { return false };
    note(format!("signing in to {}: a tab of the actions space asks", chip.label), false);
    let env = json!({"HERDR_PANE_ID": std::env::var("HERDR_PANE_ID").unwrap_or_default()});
    let request = json!({"op": "act", "app": "sources", "action": "sign_in", "args": [chip.source], "env": env});
    std::thread::spawn(move || {
        match crate::hub::request(&request, Duration::from_secs(60)) {
            Ok(answer) if answer["error"].is_string() => note(answer["error"].as_str().unwrap_or_default().to_string(), true),
            Ok(answer) if answer["unknown"] == true => note("this hub cannot sign in: update cockpit".into(), true),
            Ok(answer) => note(answer["note"].as_str().unwrap_or_default().to_string(), false),
            Err(error) => note(error, true),
        }
        crate::term::redraw();
    });
    true
}

// ---------------------------------------------------------------------------------------------------------------------
// Drawing

fn chip_spans(chip: &Chip) -> Vec<Span<'static>> {
    // Orange is for keys: a failing chip is red, a stale one dim with its word.
    let style = match chip.tone {
        Tone::Quiet | Tone::Warn => dim(),
        Tone::Bad => fg(RED),
    };
    let mut spans = vec![Span::styled(chip.text.clone(), style)];
    if let Some(fix) = chip.fix {
        spans.push(Span::styled(format!(" {}", fix.key()), bold(ORANGE)));
    }
    spans
}

fn spans_width(spans: &[Span]) -> usize {
    spans.iter().map(|s| width(&s.content)).sum()
}

/// The chips as one run of spans, the quiet ones left out when `loud_only`.
fn run(chips: &[Chip], loud_only: bool) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for chip in chips.iter().filter(|c| !loud_only || c.tone != Tone::Quiet) {
        if !spans.is_empty() {
            spans.push(Span::raw(GAP));
        }
        spans.extend(chip_spans(chip));
    }
    spans
}

/// The header's lines for `cells` columns: the title line with the chips at its right when they fit, else the chips on
/// a line of their own, the quiet ones dropped first, then all but the first loud one ("+2 more · ? lists them").
pub fn lines(title: Line<'static>, sources: &[Source], cells: usize) -> Vec<Line<'static>> {
    let now = now();
    let chips: Vec<Chip> = sources.iter().map(|s| chip(s, now, &clock)).collect();
    *shown() = chips.clone();
    let mut lines = layout(title, &chips, cells);
    let note = NOTE.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if let Some((text, bad, _)) = note.filter(|(_, _, at)| at.elapsed() < NOTE_FOR) {
        lines.push(Line::styled(crate::text::fit(&text, cells), if bad { fg(RED) } else { dim() }));
    }
    lines
}

/// A chip followed by how many more there are, in the longest words that fit.
fn with_more(mut spans: Vec<Span<'static>>, more: usize, cells: usize) -> Vec<Span<'static>> {
    if more == 0 {
        return spans;
    }
    let used = spans_width(&spans);
    let words = [format!("   +{more} more · ? lists them"), format!("   +{more} more"), format!("  +{more}")];
    let fits = words.iter().find(|w| used + width(w) <= cells).unwrap_or(&words[2]);
    spans.push(Span::styled(fits.clone(), dim()));
    spans
}

/// `lines` without the shared state: what a header looks like for these chips.
pub fn layout(title: Line<'static>, chips: &[Chip], cells: usize) -> Vec<Line<'static>> {
    if chips.is_empty() {
        return vec![title];
    }
    let used = spans_width(&title.spans);
    let all = run(chips, false);
    if used + GAP.len() + spans_width(&all) <= cells {
        let mut line = title;
        line.spans.push(Span::raw(" ".repeat(cells - used - spans_width(&all))));
        line.spans.extend(all);
        return vec![line];
    }
    let loud = run(chips, true);
    let loud_chips: Vec<&Chip> = chips.iter().filter(|c| c.tone != Tone::Quiet).collect();
    let mut below = if spans_width(&all) <= cells {
        all
    } else if loud_chips.is_empty() {
        // Only quiet chips, too many for the pane: the first, and how many more.
        with_more(chip_spans(&chips[0]), chips.len() - 1, cells)
    } else if spans_width(&loud) <= cells {
        loud
    } else {
        with_more(chip_spans(loud_chips[0]), loud_chips.len() - 1, cells)
    };
    if spans_width(&below) > cells {
        let text: String = below.iter().map(|s| s.content.as_ref()).collect();
        let style = below.first().map(|s| s.style).unwrap_or_default();
        below = vec![Span::styled(crate::text::fit(&text, cells), style)];
    }
    vec![title, Line::from(below)]
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: u64 = 60_000;
    const NOW: u64 = 1_793_628_000_000; // 2026-11-02 14:00 UTC

    fn utc(ms: u64) -> String {
        chrono::Utc.timestamp_millis_opt(ms as i64).single().map(|t| t.format("%H:%M").to_string()).unwrap_or_default()
    }

    fn source(id: &str, label: &str, at: Option<u64>, every: u64, error: Option<(Kind, &str, u64)>, next: Option<u64>) -> Source {
        let fresh = Freshness { source: id.into(), at, next, every: Some(every), error: error.map(|(k, m, s)| (k, m.to_string(), s)) };
        Source { fresh, label: label.into(), login: None, elsewhere: None }
    }

    fn text(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn acme() -> Vec<Source> {
        vec![
            source("github", "GitHub", Some(NOW - 2 * MIN), MIN, None, None),
            source("tickets", "Linear", Some(NOW - 30 * MIN), MIN, Some((Kind::Auth, "HTTP 401: token refused", NOW - 25 * MIN)), None),
            source("ci", "CI", Some(NOW - 60 * MIN), MIN / 2, Some((Kind::Network, "unreachable: connection refused", NOW - 56 * MIN)), None),
            source("calendar", "Calendar", Some(NOW - 120 * MIN), 5 * MIN, None, None),
            source("mail", "Mail", Some(NOW - 3 * MIN), MIN, Some((Kind::Rate, "rate limited", NOW - 2 * MIN)), Some(NOW + 30 * MIN)),
        ]
    }

    #[test]
    fn each_state_reads_as_its_own_chip() {
        let chips: Vec<Chip> = acme().iter().map(|s| chip(s, NOW, &utc)).collect();
        let texts: Vec<&str> = chips.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(
            texts,
            ["GitHub · 2 min ago", "Linear · sign in again", "CI · offline since 13:04", "Calendar · stale, 2 h ago", "Mail · limited until 14:30"]
        );
        let fixes: Vec<Option<Fix>> = chips.iter().map(|c| c.fix).collect();
        assert_eq!(fixes, [None, Some(Fix::SignIn), Some(Fix::Refresh), Some(Fix::Refresh), Some(Fix::Refresh)]);
        let tones: Vec<Tone> = chips.iter().map(|c| c.tone).collect();
        assert_eq!(tones, [Tone::Quiet, Tone::Bad, Tone::Bad, Tone::Warn, Tone::Bad]);
        assert_eq!(chips[1].detail, "HTTP 401: token refused · since 13:35 · last read 30 min ago");
        assert!(chips[3].detail.starts_with("not read again when due, read every 5 min"), "{}", chips[3].detail);
        let never = chip(&source("monitor", "Monitor", None, MIN, None, None), NOW, &utc);
        assert_eq!((never.text.as_str(), never.tone), ("Monitor · not read yet", Tone::Quiet));
    }

    #[test]
    fn data_the_plugin_brought_in_after_a_failed_read_is_not_old() {
        let mut linear = acme().remove(1);
        linear.elsewhere = Some(NOW - 2 * MIN);
        let c = chip(&linear, NOW, &utc);
        assert_eq!((c.text.as_str(), c.tone, c.fix), ("Linear · 2 min ago", Tone::Quiet, None));
        assert!(c.detail.starts_with("came in through a Claude session at 13:58; the hub's own read fails: HTTP 401"), "{}", c.detail);
        linear.elsewhere = Some(NOW - 40 * MIN);
        assert_eq!(chip(&linear, NOW, &utc).tone, Tone::Bad, "older than the failure, it says nothing");
        let failing = json!({"source": "tickets", "error": {"kind": "auth", "message": "x", "since": 5}});
        let doc = json!({"modAt": NOW, "modFetched": {"tickets": NOW - MIN}, "hub": {"fresh": {"tickets": failing, "github": {"source": "github"}}}});
        let found = sources(&[&doc]);
        let of = |id: &str| found.iter().find(|s| s.fresh.source == id).unwrap().elsewhere;
        assert_eq!((of("tickets"), of("github")), (Some(NOW - MIN), None), "only the source the plugin brought, and when it did");
        let ci = json!({"at": 7, "by": "hub", "hub": {"fresh": {"ci": {"source": "ci"}}}});
        assert_eq!(sources(&[&ci])[0].elsewhere, None, "the hub's own write is no other way");
        let theirs = json!({"at": 7, "by": "plugin", "hub": {"fresh": {"ci": {"source": "ci"}}}});
        assert_eq!(sources(&[&theirs])[0].elsewhere, Some(7));
    }

    #[test]
    fn a_chip_tells_only_its_own_sources_error_line() {
        let shown = vec![source("github", "GitHub", Some(NOW), MIN, None, None)];
        assert!(covers(&shown, "GitHub: HTTP 502"));
        assert!(!covers(&shown, "Linear: HTTP 401: token refused"), "a source the config stopped reading keeps its line");
        assert!(!covers(&shown, "Previews failed"));
    }

    #[test]
    fn a_recovered_source_is_quiet_again() {
        let mut ci = acme().remove(2);
        assert_eq!(chip(&ci, NOW, &utc).tone, Tone::Bad);
        ci.fresh.read(NOW - 5_000);
        let back = chip(&ci, NOW, &utc);
        assert_eq!((back.text.as_str(), back.tone, back.fix), ("CI · just now", Tone::Quiet, None));
    }

    #[test]
    fn chips_sit_beside_the_title_then_below_it_then_collapse() {
        let title = Line::from(vec![Span::raw("  Inbox"), Span::raw("   updated 14:00")]);
        let chips: Vec<Chip> = acme().iter().map(|s| chip(s, NOW, &utc)).collect();
        let wide = layout(title.clone(), &chips, 200);
        assert_eq!(wide.len(), 1);
        assert!(text(&wide[0]).ends_with("Mail · limited until 14:30 R"), "{}", text(&wide[0]));
        let below = layout(title.clone(), &chips, 140);
        assert_eq!(
            text(&below[1]),
            "GitHub · 2 min ago   Linear · sign in again L   CI · offline since 13:04 R   Calendar · stale, 2 h ago R   Mail · limited until 14:30 R"
        );
        let loud = layout(title.clone(), &chips, 120);
        assert_eq!(text(&loud[1]), "Linear · sign in again L   CI · offline since 13:04 R   Calendar · stale, 2 h ago R   Mail · limited until 14:30 R");
        let narrow = layout(title.clone(), &chips, 60);
        assert_eq!(text(&narrow[1]), "Linear · sign in again L   +3 more · ? lists them");
        assert!(layout(title.clone(), &[], 60).len() == 1, "no source, no chip line");
        let tiny = layout(title, &chips, 20);
        assert!(width(&text(&tiny[1])) <= 20, "{}", text(&tiny[1]));
    }

    #[test]
    fn documents_hold_their_sources_in_one_order() {
        let ci = json!({"hub": {"fresh": {"ci": {"source": "ci", "label": "CI", "at": 5, "every": 30000, "error": null}}}});
        let inbox = json!({"hub": {"fresh": {
            "tickets": {"source": "tickets", "label": "Linear", "at": 5, "every": 60000, "login": "cockpit connector login Linear",
                        "error": {"kind": "auth", "message": "HTTP 401", "since": 4}},
            "github": {"source": "github", "label": "GitHub", "at": 5, "every": 30000, "error": null}}}});
        let found = sources(&[&inbox, &ci, &Value::Null]);
        let ids: Vec<&str> = found.iter().map(|s| s.fresh.source.as_str()).collect();
        assert_eq!(ids, ["github", "tickets", "ci"]);
        assert_eq!(found[1].login.as_deref(), Some("cockpit connector login Linear"));
        assert!(has(&found, "ci") && !has(&found, "mail"));
        let served = json(&found, 10);
        assert_eq!((&served[1]["state"], &served[1]["error"]["kind"], &served[1]["label"]), (&json!("failing"), &json!("auth"), &json!("Linear")));
    }

    #[test]
    fn the_overlay_lists_what_each_key_fixes() {
        *shown() = acme().iter().map(|s| chip(s, NOW, &utc)).collect();
        let rows = help_rows();
        assert_eq!(rows.len(), 4, "{rows:?}");
        assert_eq!(rows[0].0, "L");
        assert!(rows[0].1.starts_with("sign in again to Linear · HTTP 401"), "{}", rows[0].1);
        assert!(rows[3].1.starts_with("read Mail again (it waits for its limit)"), "{}", rows[3].1);
        shown().clear();
    }
}
