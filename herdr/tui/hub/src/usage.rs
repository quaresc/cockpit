//! Every agent's usage kept current in the hub: the sources read again every 30 s and the transcripts' tokens counted
//! again each minute, the views that draw them rebuilt when what they draw moved, and the herdr sidebar's `$limits`
//! token written, the hub its only writer. Claude Code's plugin only keeps limits.json, which the hub reads.

use std::sync::Mutex;
use std::time::Duration;

use cockpit_core::seen::now_ms;
use cockpit_core::usage;
use serde_json::{Value, json};

/// How often the sources are read again: a new reading reaches the views and the token within this.
const EVERY: Duration = Duration::from_secs(30);
/// How often the transcripts are counted again, in ms.
const RECOUNT: u64 = 60_000;
/// An unchanged token is sent again this often, in ms, so whatever else wrote it gives way.
const RESEND: u64 = 5 * 60_000;
/// herdr drops the token this long after its last send, in ms: a hub that dies leaves no reading behind for long.
const TTL: u64 = 2 * RESEND + 60_000;
/// The source herdr's token goes by: the same as the inbox space's other counts.
const SOURCE: &str = "cockpit";

pub fn start() {
    std::thread::spawn(|| {
        let (mut drawn, mut spent, mut shown, mut counted) = (String::new(), Value::Null, String::new(), 0u64);
        loop {
            let now = now_ms();
            usage::refresh(now);
            let view = usage::view(now);
            // The token goes out first: counting the transcripts can take a while the first time.
            report(view["token"].as_str(), now);
            if now.saturating_sub(counted) >= RECOUNT {
                usage::rescan(now);
                counted = now;
            }
            let (windows, spend) = (windows_of(&view).to_string(), usage::spend());
            if windows != drawn {
                drawn = windows;
                crate::views::touch("agents");
            }
            if spend != spent {
                spent = spend;
                crate::views::touch("home");
            }
            // The sheet's words move with the clock ("resets in"), so it is rebuilt whenever any of them does.
            let mut whole = usage::view(now_ms());
            whole["at"] = Value::Null;
            let whole = whole.to_string();
            if whole != shown {
                shown = whole;
                crate::views::touch("usage");
            }
            std::thread::sleep(EVERY);
        }
    });
}

/// The files read and the tokens counted again now, and the views that draw them rebuilt: a usage chip's refresh.
pub fn read_now() {
    let now = now_ms();
    usage::refresh(now);
    usage::rescan(now);
    for view in ["usage", "agents", "home"] {
        crate::views::touch(view);
    }
}

/// What the bars and the token draw of the view, without what moves with the clock alone (the pace tick, "resets in").
fn windows_of(view: &Value) -> Value {
    let windows: Vec<Value> = view["windows"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|w| json!([w["label"], w["percent"], w["status"], w["resets_at"], w["as_of"], w["state"]]))
        .collect();
    let providers: Vec<Value> = view["providers"].as_array().into_iter().flatten().map(|p| json!([p["agent"], p["state"], p["identity"]])).collect();
    json!([windows, providers, view["token"]])
}

/// The token last sent and when; None before the first send.
fn last() -> std::sync::MutexGuard<'static, Option<(Option<String>, u64)>> {
    static LAST: Mutex<Option<(Option<String>, u64)>> = Mutex::new(None);
    LAST.lock().unwrap_or_else(|e| e.into_inner())
}

/// The herdr TUI's `$limits` token on the inbox space's row: sent on the first call whatever it is (clearing one an
/// earlier hub left), then when it changes and now and then. None in the Herdr app alone, or with COCKPIT_HUB_LIMITS=off.
fn report(text: Option<&str>, now: u64) {
    let app_only = cockpit_core::files::config()["frontend"] == "app" || std::env::var("COCKPIT_FRONTEND").is_ok_and(|f| f == "app");
    if app_only || std::env::var("COCKPIT_HUB_LIMITS").is_ok_and(|v| v == "off") {
        return;
    }
    let due = match last().as_ref() {
        None => true,
        Some((sent, at)) => sent.as_deref() != text || (text.is_some() && now.saturating_sub(*at) >= RESEND),
    };
    if due && send(text) {
        *last() = Some((text.map(str::to_string), now));
    }
}

/// The token cleared as the hub leaves, so no reading outlives the hub that kept it current: only while hub.json still
/// names this hub (a newer one owns it otherwise), and with herdr given two seconds a call.
pub fn stand_down() {
    let ours = cockpit_core::files::load(&crate::tcp_file()).is_some_and(|v| v["pid"] == std::process::id());
    if ours && last().as_ref().is_some_and(|(sent, _)| sent.is_some()) {
        send_within(None, Duration::from_secs(2));
    }
}

/// herdr keeps a token per name: the last write wins whatever its source, and a clear from any source clears it. Each
/// write lasts `TTL`; true when herdr took it.
fn send(text: Option<&str>) -> bool {
    send_within(text, Duration::from_secs(15))
}

fn send_within(text: Option<&str>, limit: Duration) -> bool {
    let spaces = crate::herdr::spaces_within(limit);
    let Some(id) = spaces.iter().find(|w| w["label"].as_str().is_some_and(|l| l.ends_with("  inbox"))).and_then(|w| w["workspace_id"].as_str()) else {
        return false;
    };
    let (token, ttl) = (text.map(|t| format!("limits={t}")), TTL.to_string());
    let mut args = vec!["workspace", "report-metadata", id, "--source", SOURCE];
    match &token {
        Some(t) => args.extend(["--token", t.as_str(), "--ttl-ms", ttl.as_str()]),
        None => args.extend(["--clear-token", "limits"]),
    }
    crate::herdr::run_within(&args, limit).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bars_change_only_when_a_reading_does() {
        let view = |elapsed: f64, words: &str| {
            json!({"windows": [{"label": "Claude 5h", "percent": 64, "resets_at": 9, "as_of": 1, "state": "fresh", "elapsed": elapsed, "resets_words": words}],
                   "providers": [{"agent": "claude", "state": "fresh"}], "token": "C 64%·5h", "tokens": {"at": 5}})
        };
        assert_eq!(windows_of(&view(0.2, "resets in 2 h")), windows_of(&view(0.3, "resets in 1 h")));
        let mut counted = view(0.2, "");
        counted["tokens"]["at"] = json!(6);
        assert_eq!(windows_of(&counted), windows_of(&view(0.2, "")), "a new count is not a new bar");
        let mut moved = view(0.2, "");
        moved["windows"][0]["percent"] = json!(65);
        assert_ne!(windows_of(&moved), windows_of(&view(0.2, "")));
    }
}
