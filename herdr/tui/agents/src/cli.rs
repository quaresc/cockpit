//! `cockpit agents`: the agents view from a shell or a herdr key, through the hub. What the Herdr app draws, in text: who
//! needs you and what they ask, answered by number or words; a recap; snooze, seen, dismiss, fork, the week's time per
//! ticket.

use std::time::{Duration, Instant};

use cockpit_core::hub;
use serde_json::{Value, json};

const USAGE: &str = "Every agent terminal, through the hub.

    cockpit agents [--json]                      who needs you, then the rest, by whose move
    cockpit agents answer <terminal> <n|n,m|text> [--asked <fingerprint>]
                                                 a choice, ticked choices (then Submit), or words; a permission
                                                 asks you to type its choice's label, or takes the fingerprint
                                                 `--json` showed, and is refused once its screen moved on
    cockpit agents recap <terminal>              the agent's recap of its session (waits for it)
    cockpit agents snooze <terminal> <1h|18:00|tomorrow|done|off>
    cockpit agents seen <terminal>
    cockpit agents dismiss <terminal> [--undo]   out of Needs you until it asks something new; --undo brings it back
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

/// The handle a word names: a handle as is, else the one row whose ticket or tab or space it names.
fn resolve(model: &Value, word: &str) -> String {
    resolve_in(model, word).unwrap_or_else(|e| fail(&e))
}

fn resolve_in(model: &Value, word: &str) -> Result<String, String> {
    let rows = model["agents"].as_array().cloned().unwrap_or_default();
    if let Some(r) = rows.iter().find(|r| r["terminal"] == word || r["address"] == word) {
        return Ok(r["terminal"].as_str().unwrap_or_default().to_string());
    }
    let low = word.to_lowercase();
    let named: Vec<&str> = rows
        .iter()
        .filter(|r| [&r["ticket"]["key"], &r["tab"]["name"], &r["space"]["name"]].iter().any(|v| v.as_str().is_some_and(|s| s.to_lowercase() == low)))
        .filter_map(|r| r["terminal"].as_str())
        .collect();
    match named.as_slice() {
        [one] => Ok(one.to_string()),
        [] => Err(format!("no agent {word:?}: `cockpit agents` lists them")),
        many => Err(format!("{word:?} names {} agents ({}): name one by its handle", many.len(), many.join(", "))),
    }
}

fn try_act(action: &str, arg: Value) -> Result<String, String> {
    act_on("agents", action, arg)
}

/// One of the hub's actions on one of its views: its note, or the refusal.
fn act_on(app: &str, action: &str, arg: Value) -> Result<String, String> {
    let answer = hub::request(&json!({"op": "act", "app": app, "action": action, "args": [arg]}), Duration::from_secs(60))
        .map_err(|e| format!("the cockpit hub: {e}"))?;
    answer["error"].as_str().filter(|e| !e.is_empty()).map_or_else(|| Ok(answer["note"].as_str().unwrap_or_default().to_string()), |e| Err(e.to_string()))
}

/// Ends a session cockpit drives: `confirm` says the person agreed to stop the turn it is in.
fn end(key: &str, confirm: bool) -> Result<String, String> {
    act_on("session", "stop", json!({"key": key, "confirm": confirm}))
}

fn act(action: &str, arg: Value) -> String {
    try_act(action, arg).unwrap_or_else(|e| fail(&e))
}

/// The answer act's argument for what was typed at a row, naming the question the row showed: a driven session's
/// `ask`, a terminal's `asked`. A permission takes a number and nothing else.
pub fn answer_arg(row: &Value, words: &str, source: &str) -> Result<Value, String> {
    let q = &row["question"];
    let terminal = row["terminal"].clone();
    // Numbers as the screen can show them: one past u32 is words, never a choice.
    let numbers: Option<Vec<u32>> = words.split(',').map(|n| n.trim().parse().ok()).collect();
    let mut arg = match numbers {
        Some(ns) if q["kind"] == "multi" => json!({"terminal": terminal, "choices": ns}),
        Some(ns) if ns.len() == 1 => json!({"terminal": terminal, "choice": ns[0]}),
        _ if q["permission"] == true => return Err(numbers_for(q)),
        _ => json!({"terminal": terminal, "text": words}),
    };
    for key in ["ask", "asked"] {
        if let Some(v) = q[key].as_str() {
            arg[key] = json!(v);
        }
    }
    arg["source"] = json!(source);
    Ok(arg)
}

/// What a permission answered with words is told: a driven session's two answers, else the numbers its screen shows.
fn numbers_for(q: &Value) -> String {
    if q["ask"].is_string() {
        return "a permission takes its number: 1 allows, 2 denies".into();
    }
    let each: Vec<String> = q["choices"].as_array().into_iter().flatten().map(|c| format!("{} {}", c["n"], c["label"].as_str().unwrap_or_default())).collect();
    format!("a permission takes one of its numbers: {}", each.join(" · "))
}

/// A choice's label as the person types it to confirm: its key hint, case and spacing left out.
fn plain_label(label: &str) -> String {
    static HINT: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"\s*\([^)]*\)\s*$").unwrap());
    HINT.replace(label, "").split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// A permission answered from a shell: the question, what it allows and its choices are printed, and the person types
/// the label of the choice they named, so a number aimed at another prompt does not pass.
fn confirm_permission(row: &Value, n: u64) {
    let q = &row["question"];
    let label = q["choices"].as_array().into_iter().flatten().find(|c| c["n"] == n).and_then(|c| c["label"].as_str()).unwrap_or_else(|| fail(&numbers_for(q)));
    println!("{} asks: {}", row["title"].as_str().unwrap_or_default(), q["text"].as_str().unwrap_or_default());
    for line in q["preview"].as_str().unwrap_or_default().lines() {
        println!("  │ {line}");
    }

    for c in q["choices"].as_array().into_iter().flatten() {
        println!("  {}. {}", c["n"], c["label"].as_str().unwrap_or_default());
    }
    print!("Type “{}” to answer {n}: ", plain_label(label));
    let _ = std::io::Write::flush(&mut std::io::stdout());
    let mut typed = String::new();
    let _ = std::io::stdin().read_line(&mut typed);
    if plain_label(&typed) != plain_label(label) {
        fail("not answered: what was typed is not that choice's label");
    }
}

/// Whether a terminal is on the input: a speed bump for permission answers, which a script with no terminal does not
/// pass. The hub refuses answers from an agent's process tree, not a same-user process pressing keys in a terminal.
pub fn at_a_terminal() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal()
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

/// The word that stands for a row's agent where the desktop shows its logo (`claude`, `codex`), empty when none is known.
fn mark_of(r: &Value, table: &[cockpit_core::agent::Agent]) -> String {
    let id = r["agent"].as_str().filter(|a| !a.is_empty()).or_else(|| r["program"].as_str()).unwrap_or_default();
    if id.is_empty() { String::new() } else { cockpit_core::agent::mark_in(table, id) }
}

/// The list as text: stage, then each agent's mark, title, place and facts; a waiting agent's question and its numbered
/// choices under it.
pub fn render(model: &Value) -> String {
    let mut out = String::new();
    let mut last = String::new();
    let rows: Vec<&Value> = model["agents"].as_array().into_iter().flatten().collect();
    let table = cockpit_core::agent::all();
    let marks: Vec<String> = rows.iter().map(|r| mark_of(r, &table)).collect();
    let mark_w = marks.iter().map(|m| m.chars().count()).max().unwrap_or(0);
    for (r, mark) in rows.into_iter().zip(&marks) {
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
        if r["interrupted"] == true {
            facts.push("stopped by you".into());
        }
        if r["paused"] == true {
            facts.push("paused · picks up when you write".into());
        }
        let when = match stage {
            "snoozed" => r["snoozed"]["label"].as_str().unwrap_or("snoozed").to_string(),
            "dismissed" => format!("dismissed · {}", r["status"].as_str().unwrap_or_default()),
            _ => ago(r["since"].as_u64().unwrap_or(0)),
        };
        let mark = if mark_w == 0 { String::new() } else { format!("{mark:<mark_w$}  ") };
        out.push_str(&format!(
            "  {:<10} {mark}{}  ({} · {})  {}  {when}\n",
            r["terminal"].as_str().unwrap_or_default(),
            r["title"].as_str().unwrap_or_default(),
            r["space"]["name"].as_str().unwrap_or_default(),
            r["tab"]["name"].as_str().unwrap_or_default(),
            facts.join(" · ")
        ));
        if let Some(q) = r["question"].as_object() {
            out.push_str(&format!("      ? {}\n", q["text"].as_str().unwrap_or_default().replace('\n', " ")));
            // What a permission would allow, in full: the command, the diff, the plan.
            if q.get("permission").and_then(Value::as_bool) == Some(true) {
                if let Some(why) = q.get("terminal_only").and_then(Value::as_str) {
                    out.push_str(&format!("        │ {why}\n"));
                }
                for line in q.get("preview").and_then(Value::as_str).unwrap_or_default().lines().filter(|l| !l.trim().is_empty()) {
                    out.push_str(&format!("        │ {line}\n"));
                }
            }
            for c in q["choices"].as_array().into_iter().flatten() {
                let mark = match c["checked"].as_bool() {
                    Some(true) => "[x] ",
                    Some(false) => "[ ] ",
                    None => "",
                };
                out.push_str(&format!("        {}. {mark}{}\n", c["n"], c["label"].as_str().unwrap_or_default()));
            }
        } else if let Some(waits) = r["waits"]["text"].as_str() {
            // Blocked with nothing to answer here: what it waits on, and that its terminal is where to go.
            out.push_str(&format!("      ⏸ {waits}\n"));
        } else if let Some(e) = r["error"].as_str().filter(|_| r["stage"] == "failed") {
            out.push_str(&format!("      ✗ {e}\n"));
        } else if let Some(t) = r["recap"]["text"].as_str() {
            out.push_str(&format!("      {t}\n"));
        }
    }
    if out.is_empty() {
        out.push_str("No agent reports its status.\n");
    }
    out
}

/// What a line typed at the popup's prompt asks, numbers as the list prints them (from 1).
#[derive(Debug, PartialEq)]
enum Asked {
    Answer(usize, String),
    Recap(usize),
    Go(usize),
    Snooze(usize, String),
    /// The n-th waiting agent set aside until it asks something new.
    Dismiss(usize),
    /// The n-th dismissed agent back in Needs you (`x d1`).
    Restore(usize),
    /// The popup's keys.
    Help,
    /// Of the n-th working agent.
    Stop(usize),
    /// Of the n-th session cockpit drives.
    Hand(usize),
    End(usize),
    Quit,
}

/// A line of the popup's prompt read as what it asks; None for anything else. Each word leads to one verb: stop is `s`,
/// as in the review and routines apps (`s 1`, `s s1` and `s1` alike), and `x` is dismiss, as in every app (`x d1`
/// brings a dismissed one back).
fn asked(line: &str) -> Option<Asked> {
    let words: Vec<&str> = line.split_whitespace().collect();
    // A number as the list prints it, its mark (`s1`, `o1`) or not.
    let n = |w: &str, mark: char| w.strip_prefix(mark).unwrap_or(w).parse::<usize>().ok().filter(|n| *n > 0);
    Some(match words.as_slice() {
        ["q" | "quit"] => Asked::Quit,
        ["?" | "help"] => Asked::Help,
        ["x", w] if w.starts_with('d') => Asked::Restore(n(w, 'd')?),
        ["x", w] => Asked::Dismiss(n(w, ' ')?),
        ["r", w, ..] => Asked::Recap(n(w, ' ')?),
        ["g", w, ..] => Asked::Go(n(w, ' ')?),
        ["z", w, when, ..] => Asked::Snooze(n(w, ' ')?, when.to_string()),
        ["s", w] => Asked::Stop(n(w, 's')?),
        [w] if w.starts_with('s') => Asked::Stop(n(w, 's')?),
        ["end", w] => Asked::End(n(w, 'o')?),
        ["o", w] => Asked::Hand(n(w, 'o')?),
        [w, rest @ ..] if !rest.is_empty() => Asked::Answer(n(w, ' ')?, rest.join(" ")),
        _ => return None,
    })
}

/// The list in a loop at a prompt: `<n> <choice|choices|words>` answers the n-th waiting agent, `r <n>` recaps it, `g
/// <n>` goes to it, `z <n> <when>` snoozes it, `s <n>` stops the n-th working one, `o <n>` hands the n-th session cockpit
/// drives over to a terminal of its own and `end <n>` ends it (asking first while it works), Enter refreshes, `q` leaves.
fn pick() {
    use std::io::{BufRead, Write};
    let stdin = std::io::stdin();
    loop {
        let m = model();
        let waiting = numbered(&m);
        let dismissed: Vec<String> = m["agents"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|r| r["stage"] == "dismissed")
            .filter_map(|r| r["terminal"].as_str().map(str::to_string))
            .collect();
        let stoppable: Vec<String> = m["agents"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|r| r["stoppable"] == true)
            .filter_map(|r| r["terminal"].as_str().map(str::to_string))
            .collect();
        // The sessions cockpit drives, and whether ending one would stop a turn.
        let driven: Vec<String> =
            m["agents"].as_array().into_iter().flatten().filter(|r| r["driven"] == true).filter_map(|r| r["terminal"].as_str().map(str::to_string)).collect();
        let busy = |key: &str| {
            m["agents"].as_array().into_iter().flatten().any(|r| r["terminal"] == key && matches!(r["status"].as_str(), Some("working" | "blocked")))
        };
        print!("\x1b[2J\x1b[H{}", render(&m));
        for (i, t) in waiting.iter().enumerate() {
            let failed = m["agents"].as_array().into_iter().flatten().any(|r| r["terminal"] == t.as_str() && r["stage"] == "failed");
            println!("  [{}] {t}{}", i + 1, if failed { " stopped with an error" } else { "" });
        }
        for (i, t) in stoppable.iter().enumerate() {
            println!("  [s{}] {t} works", i + 1);
        }
        for (i, t) in dismissed.iter().enumerate() {
            println!("  [d{}] {t} dismissed · x d{} brings it back", i + 1, i + 1);
        }
        for (i, t) in driven.iter().enumerate() {
            let row = m["agents"].as_array().into_iter().flatten().find(|r| r["terminal"] == t.as_str());
            let title = row.and_then(|r| r["title"].as_str()).unwrap_or(t);
            let asks = row.and_then(|r| r["question"]["text"].as_str()).map(|q| format!(" · asks: {q}")).unwrap_or_default();
            println!("  [o{}] {title}{asks}, in chat", i + 1);
        }
        let stop = if stoppable.is_empty() { "" } else { " · s <n> stop" };
        let open = if driven.is_empty() { "" } else { " · o <n> go on in a terminal · end <n> end" };
        print!("\n{}\n> ", footer(stop, open));
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        if stdin.lock().read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        if line.trim().is_empty() {
            continue;
        }
        let nth = |n: usize| waiting.get(n - 1).cloned();
        let note: Option<Result<String, String>> = match asked(&line) {
            None => None,
            Some(Asked::Quit) => return,
            Some(Asked::Recap(n)) => nth(n).map(|t| try_act("recap", json!({"terminal": t, "force": true}))),
            Some(Asked::Go(n)) => nth(n).map(|t| try_act("focus", json!({"terminal": t}))),
            Some(Asked::Snooze(n, when)) => nth(n).map(|t| try_act("snooze", json!({"terminal": t, "until": when}))),
            // The person typed it at the popup's prompt: dismissed until it asks something new, `x d<n>` its undo.
            Some(Asked::Dismiss(n)) => nth(n).map(|t| {
                // The wait this list printed: the hub refuses it once the agent waits on something else.
                let wait = m["agents"].as_array().into_iter().flatten().find(|r| r["terminal"] == t.as_str()).map(|r| r["wait"].clone()).unwrap_or(Value::Null);
                try_act("dismiss", json!({"terminal": t, "wait": wait})).map(|_| format!("dismissed {t} · it is listed as [d…], x d<n> brings it back"))
            }),
            Some(Asked::Restore(n)) => dismissed.get(n - 1).map(|t| try_act("dismiss", json!({"terminal": t, "undo": true}))),
            Some(Asked::Help) => {
                print!("{HELP}\npress Enter ");
                let _ = std::io::stdout().flush();
                let _ = stdin.lock().read_line(&mut String::new());
                None
            }
            Some(Asked::Stop(n)) => stoppable.get(n - 1).map(|t| try_act("interrupt", json!({"terminal": t}))),
            Some(Asked::End(n)) => driven.get(n - 1).map(|key| {
                let working = busy(key);
                if working {
                    print!("It is working: end it anyway, its turn too? (y/N) ");
                    let _ = std::io::stdout().flush();
                    let mut yes = String::new();
                    let _ = stdin.lock().read_line(&mut yes);
                    if !yes.trim().eq_ignore_ascii_case("y") {
                        return Ok("kept".into());
                    }
                }
                end(key, working)
            }),
            // `o <n>` is the hand-over itself: the session leaves cockpit for a terminal of this herdr.
            Some(Asked::Hand(n)) => driven.get(n - 1).map(|key| act_on("session", "terminal", json!({"key": key, "confirm": true, "frontend": "tui"}))),
            Some(Asked::Answer(n, words)) => nth(n).map(|t| {
                // The row as this list printed it: its question is the one the answer names.
                let row = m["agents"].as_array().into_iter().flatten().find(|r| r["terminal"] == t.as_str()).cloned().unwrap_or(Value::Null);
                if let Some(why) = row["question"]["terminal_only"].as_str() {
                    return Err(why.to_string());
                }
                if let Some(waits) = row["waits"]["text"].as_str() {
                    return Err(format!("{waits}: g {n} goes to it"));
                }
                if row["question"]["permission"] == true && !at_a_terminal() {
                    return Err("a permission is answered by a person, at a terminal".to_string());
                }
                answer_arg(&row, &words, "popup:key").and_then(|arg| try_act("answer", arg))
            }),
        };
        if let Some(note) = note.map(|n| n.unwrap_or_else(|e| e)).filter(|n| !n.is_empty()) {
            println!("{note}");
            std::thread::sleep(Duration::from_millis(900));
        }
    }
}

/// The agents the popup numbers `[1]`, `[2]`: those asking and those stopped with an error, which `x <n>` dismisses too.
fn numbered(m: &Value) -> Vec<String> {
    let rows = m["agents"].as_array().into_iter().flatten();
    rows.filter(|r| matches!(r["stage"].as_str(), Some("needs_you" | "failed"))).filter_map(|r| r["terminal"].as_str().map(str::to_string)).collect()
}

/// The popup's keys, as `?` prints them.
const HELP: &str = "The agents popup
  <n> <choice|1,3|words>  answer the n-th agent waiting on you (or stopped with an error)
  r <n>                   recap it
  g <n>                   go to its terminal
  z <n> <1h|18:00|done>   snooze it until then
  x <n>                   dismiss it: out of Needs you until it asks something new
  x d<n>                  bring the n-th dismissed agent back
  s <n>                   stop the n-th working agent's turn
  o <n> · end <n>         hand the n-th driven session to a terminal · end it
  Enter                   look again
  ?                       these keys
  q                       quit";

/// The popup's footer: its verbs, stop and the driven sessions' only when there are some.
fn footer(stop: &str, open: &str) -> String {
    format!("<n> <choice|1,3|words> answers · r <n> recap · g <n> go · z <n> 1h|18:00|done snooze · x <n> dismiss{stop}{open} · ? keys · q quit")
}

pub fn main(args: &[String]) {
    let word = |i: usize| args.get(i).map(String::as_str).unwrap_or_default();
    match args.first().map(String::as_str) {
        None => print!("{}", render(&model())),
        Some("--json") => println!("{}", model()),
        Some("answer") if args.len() >= 3 => {
            let mut words: Vec<String> = args[2..].to_vec();
            let asked = words.iter().position(|w| w == "--asked").map(|i| {
                let fingerprint = words.get(i + 1).cloned().unwrap_or_else(|| fail("--asked takes the fingerprint `cockpit agents --json` shows"));
                words.drain(i..i + 2);
                fingerprint
            });
            let m = model();
            let terminal = resolve(&m, word(1));
            let mut row = m["agents"].as_array().into_iter().flatten().find(|r| r["terminal"] == terminal.as_str()).cloned().unwrap_or(Value::Null);
            let words = words.join(" ");
            // The question the person looked at, by its fingerprint: the hub refuses it once the screen moved on.
            if let Some(fingerprint) = &asked {
                let key = if row["question"]["ask"].is_string() { "ask" } else { "asked" };
                row["question"][key] = json!(fingerprint);
            }
            let mut confirmed = false;
            // A permission cockpit cannot show in full is answered in the agent's terminal, from here as from a card.
            if let Some(why) = row["question"]["terminal_only"].as_str() {
                fail(why);
            }
            if row["question"]["permission"] == true {
                if !at_a_terminal() {
                    fail("a permission is answered by a person: run this at a terminal, or answer it in the window");
                }
                if asked.is_none() {
                    let n: u64 = words.trim().parse().unwrap_or_else(|_| fail(&numbers_for(&row["question"])));
                    confirm_permission(&row, n);
                    // Its label typed in full is the confirm a choice that runs every tool unasked needs.
                    confirmed = row["question"]["unasked"].as_array().into_iter().flatten().any(|c| *c == n);
                }
            }
            let mut arg = answer_arg(&row, &words, "cli:command").unwrap_or_else(|e| fail(&e));
            if confirmed {
                arg["confirm"] = json!(true);
            }
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
        Some("stop") if args.len() >= 2 => {
            println!("{}", act("interrupt", json!({"terminal": resolve(&model(), word(1))})));
        }
        Some("seen") if args.len() >= 2 => {
            act("seen", json!({"terminal": resolve(&model(), word(1))}));
        }
        Some("dismiss") if args.len() >= 2 => {
            let undo = args.iter().any(|a| a == "--undo");
            let m = model();
            let terminal = resolve(&m, word(1));
            // The wait as the model read now shows it, so one that moves on before the act is refused.
            let wait =
                m["agents"].as_array().into_iter().flatten().find(|r| r["terminal"] == terminal.as_str()).map(|r| r["wait"].clone()).unwrap_or(Value::Null);
            println!("{}", act("dismiss", json!({"terminal": terminal, "undo": undo, "wait": if undo { Value::Null } else { wait }})));
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
    fn the_popup_stops_on_s_and_keeps_one_verb_per_key() {
        assert_eq!(asked("s 1"), Some(Asked::Stop(1)));
        assert_eq!(asked("s s2"), Some(Asked::Stop(2)));
        assert_eq!(asked("s2"), Some(Asked::Stop(2)), "as the list prints it");
        assert_eq!(asked("x 1"), Some(Asked::Dismiss(1)), "x is the apps' done, reopen and dismiss, never a stop");
        assert_eq!((asked("x d2"), asked("?"), asked("x")), (Some(Asked::Restore(2)), Some(Asked::Help), None));
        assert_eq!(cockpit_core::keys::key_of("dismiss"), Some('x'), "the popup's dismiss is the shared verb's key");
        assert!(HELP.contains("x <n>                   dismiss it") && HELP.contains("x d<n>") && HELP.contains("z <n>"), "{HELP}");
        let model = json!({"agents": [{"terminal": "app:t41", "stage": "needs_you"}, {"terminal": "app:t48", "stage": "failed"}, {"terminal": "app:t52", "stage": "dismissed"}, {"terminal": "app:t44", "stage": "working"}]});
        assert_eq!(numbered(&model), ["app:t41", "app:t48"], "x <n> reaches one stopped with an error too");
        let shown = footer(" · s <n> stop", "");
        assert!(shown.contains("x <n> dismiss") && shown.contains("? keys") && shown.starts_with("<n> <choice") && shown.ends_with("q quit"), "{shown}");
        assert_eq!(asked("z 1 18:00"), Some(Asked::Snooze(1, "18:00".into())));
        assert_eq!(asked("1 sure, go on"), Some(Asked::Answer(1, "sure, go on".into())));
        assert_eq!((asked("o o1"), asked("end 1"), asked("sure")), (Some(Asked::Hand(1)), Some(Asked::End(1)), None));
        assert_eq!(cockpit_core::keys::key_of("done"), Some('x'), "the verb x keeps in every app");
        assert_eq!(cockpit_core::keys::key_of("stop"), None, "stop is the apps' own verb, on s in review and routines");
    }

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
        let marked = json!({"agents": [
            {"terminal": "app:t1", "stage": "working", "title": "Quotas", "agent": "claude", "program": "claude", "space": {"name": "api"}, "tab": {"name": "q"}, "since": 0},
            {"terminal": "app:t2", "stage": "working", "title": "Search", "agent": "", "program": "gemini", "space": {"name": "web"}, "tab": {"name": "s"}, "since": 0}
        ]});
        let marked = render(&marked);
        assert!(marked.contains("  app:t1     claude  Quotas  (api · q)"), "{marked}");
        assert!(marked.contains("  app:t2     gemini  Search  (web · s)"), "a kind Herdr reports and the table lacks: {marked}");
        let asks = json!({"agents": [{"terminal": "claude:1", "stage": "needs_you", "title": "Quotas", "space": {"name": "In chat"}, "tab": {"name": "q"}, "since": 0,
            "question": {"text": "May it run Bash: mix test?", "permission": true, "preview": "mix test test/api/quota_test.exs\nRun the quota tests",
                         "choices": [{"n": 1, "label": "Allow once"}, {"n": 2, "label": "Deny"}]}}]});
        let failed = json!({"agents": [
            {"terminal": "claude:2", "stage": "failed", "title": "Exports", "space": {"name": "In chat"}, "tab": {"name": "e"}, "since": 0, "error": "credit balance too low"},
            {"terminal": "app:t9", "stage": "done", "title": "Proration", "space": {"name": "billing"}, "tab": {"name": "p"}, "since": 0, "interrupted": true}]});
        // Blocked with nothing to answer: what it waits on, never a line of its screen's furniture.
        let waits = json!({"agents": [{"terminal": "app:t52", "stage": "needs_you", "title": "Search", "space": {"name": "api"}, "tab": {"name": "s"}, "since": 0,
            "question": null, "waits": {"kind": "limit", "text": "Reached its usage limit · resets 08:00", "resets": "08:00"}, "recap": {"text": "Two edge cases left."}}]});
        assert!(render(&waits).contains("      ⏸ Reached its usage limit · resets 08:00\n"), "{}", render(&waits));
        let set_aside = json!({"agents": [{"terminal": "app:t52", "stage": "dismissed", "status": "blocked", "title": "Search", "space": {"name": "api"}, "tab": {"name": "s"},
            "since": 0, "question": null, "waits": {"kind": "limit", "text": "Reached its usage limit"}, "dismissed": {"at": 1}}]});
        let quiet = render(&set_aside);
        assert!(quiet.starts_with("Dismissed\n  app:t52") && quiet.contains("dismissed · blocked"), "listed with its real status: {quiet}");
        let shown = render(&failed);
        assert!(shown.starts_with("Stopped with an error\n  claude:2"), "{shown}");
        assert!(shown.contains("      ✗ credit balance too low\n"));
        assert!(shown.contains("\nDone\n  app:t9") && shown.contains("stopped by you"));
        assert!(
            render(&asks).contains(
                "      ? May it run Bash: mix test?\n        │ mix test test/api/quota_test.exs\n        │ Run the quota tests\n        1. Allow once"
            )
        );
        assert_eq!(resolve(&m, "eng-1"), "app:t1");
    }

    #[test]
    fn an_answer_from_the_shell_names_the_question_it_saw() {
        let driven = json!({"terminal": "claude:1", "question": {"kind": "single", "permission": true, "ask": "toolu_a", "choices": []}});
        assert_eq!(answer_arg(&driven, "1", "popup:key").unwrap(), json!({"terminal": "claude:1", "choice": 1, "ask": "toolu_a", "source": "popup:key"}));
        assert!(answer_arg(&driven, "yes", "popup:key").is_err(), "words are not an allow");
        let multi = json!({"terminal": "app:t2", "question": {"kind": "multi", "asked": "00ff"}});
        assert_eq!(
            answer_arg(&multi, "1,3", "cli:command").unwrap(),
            json!({"terminal": "app:t2", "choices": [1, 3], "asked": "00ff", "source": "cli:command"})
        );
        let text = json!({"terminal": "app:t3", "question": {"kind": "text", "asked": "00aa"}});
        assert_eq!(answer_arg(&text, "after the backfill", "cli:command").unwrap()["text"], "after the backfill");
        assert_eq!(answer_arg(&multi, "4294967298", "cli:command").unwrap()["text"], "4294967298", "past u32 it is words, never choice 2");
        let gone = json!({"terminal": "app:t4", "question": null});
        assert!(answer_arg(&gone, "1", "cli:command").unwrap().get("asked").is_none(), "nothing asked: the hub refuses it");
        // Words for a permission are told its own numbers: on a terminal, 2 is Claude's "don't ask again", not a deny.
        assert_eq!(answer_arg(&driven, "no", "cli:command").unwrap_err(), "a permission takes its number: 1 allows, 2 denies");
        let terminal = json!({"terminal": "app:t5", "question": {"kind": "single", "permission": true, "asked": "00bb", "choices": [
            {"n": 1, "label": "Yes"}, {"n": 2, "label": "Yes, and don't ask again for rm commands"}, {"n": 3, "label": "No, and tell Claude what to do differently (esc)"}]}});
        assert_eq!(
            answer_arg(&terminal, "no", "cli:command").unwrap_err(),
            "a permission takes one of its numbers: 1 Yes · 2 Yes, and don't ask again for rm commands · 3 No, and tell Claude what to do differently (esc)"
        );
    }

    #[test]
    fn a_name_two_agents_share_answers_neither() {
        let m = json!({"agents": [
            {"terminal": "app:t1", "address": "t1", "ticket": {"key": "ENG-412"}, "tab": {"name": "quotas"}, "space": {"name": "api"}},
            {"terminal": "app:t2", "address": "t2", "ticket": {"key": "ENG-413"}, "tab": {"name": "banner"}, "space": {"name": "api"}}]});
        assert_eq!(resolve_in(&m, "quotas").unwrap(), "app:t1");
        assert_eq!(resolve_in(&m, "t2").unwrap(), "app:t2");
        assert_eq!(resolve_in(&m, "api").unwrap_err(), "\"api\" names 2 agents (app:t1, app:t2): name one by its handle");
        assert!(resolve_in(&m, "billing").is_err());
    }

    #[test]
    fn a_permission_from_the_shell_is_confirmed_by_its_label() {
        assert_eq!(plain_label("No, and tell Claude what to do differently (esc)"), "no, and tell claude what to do differently");
        assert_eq!(plain_label("  yes,  and don't ask again for rm commands\n"), plain_label("Yes, and don't ask again for rm commands"));
        assert_ne!(plain_label("Yes"), plain_label("Yes, and don't ask again for rm commands"), "1's label does not confirm 2");
    }
}
