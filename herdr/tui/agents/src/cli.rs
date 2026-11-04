//! `cockpit agents`: the agents view from a shell or a herdr key, through the hub. What the Herdr app draws, in text: who
//! needs you and what they ask, answered by number or words; a recap; snooze, seen, fork, the week's time per ticket.

use std::time::{Duration, Instant};

use cockpit_core::hub;
use serde_json::{Value, json};

const USAGE: &str = "Every agent terminal, through the hub.

    cockpit agents [--json]                      who needs you, then the rest, by whose move
    cockpit agents answer <terminal> <n|n,m|text>  a choice, ticked choices (then Submit), or words
    cockpit agents recap <terminal>              the agent's recap of its session (waits for it)
    cockpit agents snooze <terminal> <1h|18:00|tomorrow|done|off>
    cockpit agents seen <terminal>
    cockpit agents fork <terminal> [tab|right|down]
    cockpit agents week                          agent time per ticket, Monday to Sunday
    cockpit agents pick                          the list, answered from a prompt (herdr's agents popup)

<terminal> is a handle from the list (app:t3, w9:p1), or its ticket key or tab name.";

fn fail(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(1)
}

fn model() -> Value {
    hub::request(&json!({"op": "view", "name": "agents"}), Duration::from_secs(10))
        .map(|v| v["value"].clone())
        .unwrap_or_else(|e| fail(&format!("the cockpit hub: {e}")))
}

/// The handle a word names: a handle as is, else the row whose ticket or tab or space it names.
fn resolve(model: &Value, word: &str) -> String {
    let rows = model["agents"].as_array().cloned().unwrap_or_default();
    let low = word.to_lowercase();
    let found = rows.iter().find(|r| r["terminal"] == word || r["address"] == word).or_else(|| {
        rows.iter().find(|r| [&r["ticket"]["key"], &r["tab"]["name"], &r["space"]["name"]].iter().any(|v| v.as_str().is_some_and(|s| s.to_lowercase() == low)))
    });
    found.and_then(|r| r["terminal"].as_str()).map(str::to_string).unwrap_or_else(|| fail(&format!("no agent {word:?}: `cockpit agents` lists them")))
}

fn try_act(action: &str, arg: Value) -> Result<String, String> {
    let answer = hub::request(&json!({"op": "act", "app": "agents", "action": action, "args": [arg]}), Duration::from_secs(60))
        .map_err(|e| format!("the cockpit hub: {e}"))?;
    match answer["error"].as_str() {
        Some(e) => Err(e.to_string()),
        None => Ok(answer["note"].as_str().unwrap_or_default().to_string()),
    }
}

fn act(action: &str, arg: Value) -> String {
    try_act(action, arg).unwrap_or_else(|e| fail(&e))
}

fn ago(ms: u64) -> String {
    let m = cockpit_core::seen::now_ms().saturating_sub(ms) / 60_000;
    match m {
        0 => "<1m".into(),
        1..60 => format!("{m}m"),
        _ if m < 48 * 60 => format!("{}h", m / 60),
        _ => format!("{}d", m / 1440),
    }
}

fn hours(secs: u64) -> String {
    let m = secs / 60;
    if m < 60 { format!("{m}m") } else { format!("{}h{:02}", m / 60, m % 60) }
}

/// The list as text: stage, title, place, facts; a waiting agent's question and its numbered choices under it.
pub fn render(model: &Value) -> String {
    let mut out = String::new();
    let mut last = String::new();
    for r in model["agents"].as_array().into_iter().flatten() {
        let stage = r["stage"].as_str().unwrap_or_default();
        if stage != last {
            let label = crate::gui::STAGES.iter().find(|(id, _)| *id == stage).map(|(_, l)| *l).unwrap_or(stage);
            out.push_str(&format!("{}{label}\n", if last.is_empty() { "" } else { "\n" }));
            last = stage.to_string();
        }
        let mut facts: Vec<String> = Vec::new();
        if let Some(k) = r["ticket"]["key"].as_str() {
            facts.push(k.to_string());
        }
        if let Some(n) = r["pr"]["number"].as_u64() {
            facts.push(format!("#{n}"));
        }
        if let Some(c) = r["ci"]["label"].as_str() {
            facts.push(c.to_string());
        }
        let when = if stage == "snoozed" { r["snoozed"]["label"].as_str().unwrap_or("snoozed").to_string() } else { ago(r["since"].as_u64().unwrap_or(0)) };
        out.push_str(&format!(
            "  {:<10} {}  ({} · {})  {}  {when}\n",
            r["terminal"].as_str().unwrap_or_default(),
            r["title"].as_str().unwrap_or_default(),
            r["space"]["name"].as_str().unwrap_or_default(),
            r["tab"]["name"].as_str().unwrap_or_default(),
            facts.join(" · ")
        ));
        if let Some(q) = r["question"].as_object() {
            out.push_str(&format!("      ? {}\n", q["text"].as_str().unwrap_or_default().replace('\n', " ")));
            for c in q["choices"].as_array().into_iter().flatten() {
                let mark = match c["checked"].as_bool() {
                    Some(true) => "[x] ",
                    Some(false) => "[ ] ",
                    None => "",
                };
                out.push_str(&format!("        {}. {mark}{}\n", c["n"], c["label"].as_str().unwrap_or_default()));
            }
        } else if let Some(t) = r["recap"]["text"].as_str() {
            out.push_str(&format!("      {t}\n"));
        }
    }
    if out.is_empty() {
        out.push_str("No agent reports its status.\n");
    }
    out
}

/// The list in a loop at a prompt: `<n> <choice|choices|words>` answers the n-th waiting agent, `r <n>` recaps it, `g
/// <n>` goes to it, `s <n> <when>` snoozes it, Enter refreshes, `q` leaves.
fn pick() {
    use std::io::{BufRead, Write};
    let stdin = std::io::stdin();
    loop {
        let m = model();
        let waiting: Vec<String> = m["agents"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|r| r["stage"] == "needs_you")
            .filter_map(|r| r["terminal"].as_str().map(str::to_string))
            .collect();
        print!("\x1b[2J\x1b[H{}", render(&m));
        for (i, t) in waiting.iter().enumerate() {
            println!("  [{}] {t}", i + 1);
        }
        print!("\n<n> <choice|1,3|words> answers · r <n> recap · g <n> go · s <n> 1h|18:00|done · q quit\n> ");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        if stdin.lock().read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        let words: Vec<&str> = line.split_whitespace().collect();
        let nth = |w: Option<&&str>| w.and_then(|n| n.parse::<usize>().ok()).and_then(|n| waiting.get(n.wrapping_sub(1)).cloned());
        let note: Option<Result<String, String>> = match words.as_slice() {
            [] => continue,
            ["q" | "quit"] => return,
            ["r", ..] => nth(words.get(1)).map(|t| try_act("recap", json!({"terminal": t, "force": true}))),
            ["g", ..] => nth(words.get(1)).map(|t| try_act("focus", json!({"terminal": t}))),
            ["s", _, when, ..] => nth(words.get(1)).map(|t| try_act("snooze", json!({"terminal": t, "until": when}))),
            [n, rest @ ..] if !rest.is_empty() => nth(Some(n)).map(|t| {
                let text = rest.join(" ");
                let multi = m["agents"].as_array().into_iter().flatten().any(|r| r["terminal"] == t.as_str() && r["question"]["kind"] == "multi");
                let numbers: Option<Vec<u64>> = text.split(',').map(|x| x.trim().parse().ok()).collect();
                let arg = match numbers {
                    Some(ns) if multi => json!({"terminal": t, "choices": ns}),
                    Some(ns) if ns.len() == 1 => json!({"terminal": t, "choice": ns[0]}),
                    _ => json!({"terminal": t, "text": text}),
                };
                try_act("answer", arg)
            }),
            _ => None,
        };
        if let Some(note) = note.map(|n| n.unwrap_or_else(|e| e)).filter(|n| !n.is_empty()) {
            println!("{note}");
            std::thread::sleep(Duration::from_millis(900));
        }
    }
}

pub fn main(args: &[String]) {
    let word = |i: usize| args.get(i).map(String::as_str).unwrap_or_default();
    match args.first().map(String::as_str) {
        None => print!("{}", render(&model())),
        Some("--json") => println!("{}", model()),
        Some("answer") if args.len() >= 3 => {
            let m = model();
            let terminal = resolve(&m, word(1));
            let rest = args[2..].join(" ");
            let numbers: Option<Vec<u64>> = rest.split(',').map(|n| n.trim().parse().ok()).collect();
            let multi =
                m["agents"].as_array().into_iter().flatten().find(|r| r["terminal"] == terminal.as_str()).is_some_and(|r| r["question"]["kind"] == "multi");
            let arg = match numbers {
                Some(ns) if multi => json!({"terminal": terminal, "choices": ns}),
                Some(ns) if ns.len() == 1 => json!({"terminal": terminal, "choice": ns[0]}),
                _ => json!({"terminal": terminal, "text": rest}),
            };
            println!("{}", act("answer", arg));
        }
        Some("recap") if args.len() >= 2 => {
            let terminal = resolve(&model(), word(1));
            act("recap", json!({"terminal": terminal, "force": true}));
            let started = Instant::now();
            while started.elapsed() < Duration::from_secs(200) {
                std::thread::sleep(Duration::from_secs(2));
                let m = model();
                let r = m["agents"].as_array().into_iter().flatten().find(|r| r["terminal"] == terminal.as_str()).cloned().unwrap_or(Value::Null);
                if r["recap"]["running"] != true {
                    match (r["recap"]["text"].as_str(), r["recap"]["error"].as_str()) {
                        (_, Some(e)) => fail(e),
                        (Some(t), _) => println!("{t}"),
                        _ => fail("no recap came"),
                    }
                    return;
                }
            }
            fail("the recap took too long");
        }
        Some("snooze") if args.len() >= 3 => {
            let terminal = resolve(&model(), word(1));
            let until = if matches!(word(2), "off" | "wake") { Value::Null } else { json!(word(2)) };
            println!("{}", act("snooze", json!({"terminal": terminal, "until": until})));
        }
        Some("seen") if args.len() >= 2 => {
            act("seen", json!({"terminal": resolve(&model(), word(1))}));
        }
        Some("fork") if args.len() >= 2 => {
            let terminal = resolve(&model(), word(1));
            println!("{}", act("fork", json!({"terminal": terminal, "place": args.get(2).map(String::as_str).unwrap_or("tab")})));
        }
        Some("pick") => pick(),
        Some("week") => {
            let m = model();
            let days = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
            println!("{:<12} {}  Week", "Ticket", days.map(|d| format!("{d:>6}")).join(""));
            for r in m["week"]["rows"].as_array().into_iter().flatten() {
                let secs: Vec<String> = r["secs"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|s| s.as_u64().unwrap_or(0))
                    .map(|s| format!("{:>6}", if s == 0 { "·".into() } else { hours(s) }))
                    .collect();
                println!("{:<12} {}  {}", r["key"].as_str().unwrap_or_default(), secs.join(""), hours(r["total"].as_u64().unwrap_or(0)));
            }
        }
        _ => fail(USAGE),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_reads_as_text_with_the_question_under_its_agent() {
        let m = json!({"agents": [
            {"terminal": "app:t1", "stage": "needs_you", "title": "Quotas", "space": {"name": "api"}, "tab": {"name": "q"}, "since": 0, "ticket": {"key": "ENG-1"},
             "question": {"text": "Which?", "choices": [{"n": 1, "label": "A", "checked": null}, {"n": 2, "label": "B", "checked": null}]}},
            {"terminal": "app:t2", "stage": "working", "title": "CSV", "space": {"name": "billing"}, "tab": {"name": "c"}, "since": 0, "recap": {"text": "On it."}}
        ]});
        let text = render(&m);
        assert!(text.starts_with("Needs you\n  app:t1"));
        assert!(text.contains("      ? Which?\n        1. A\n        2. B\n"));
        assert!(text.contains("\nWorking\n  app:t2"));
        assert_eq!(resolve(&m, "eng-1"), "app:t1");
    }
}
