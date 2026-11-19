//! `cockpit browser`: the Chromium side browser's own commands. It runs on macOS and shows in the Herdr app.

pub mod agent;
pub mod hub;
pub mod install;
pub mod ports;
pub mod side;
pub mod track;

use std::time::Duration;

use cockpit_core::browser;
use serde_json::{Value, json};

use crate::proc::fail;

const USAGE: &str = "The Chromium side browser: cockpit's own Chromium window, docked in the Herdr app's side panel (macOS).

    cockpit browser install                                 downloads Electron and agent-browser at their pinned versions, checked
    cockpit browser status [--json]                         what is installed, whether it runs, what it shows
    cockpit browser quit                                    ends it; the next page opened starts it again
    cockpit browser refresh                                 after an update: installs what the new pins ask for and ends the running one
    cockpit browser profiles                                the profiles, as JSON
    cockpit browser import-cookies <profile> --from <browser>[:<its profile>] | --file <path>
                                                            copies another browser's cookies into a profile; you confirm it
    cockpit browser track --host <pid> [--once]             follows a window on screen, a reading per line, for the docked browser

Turn it on with `cockpit options set sideBrowser chromium`; `cockpit browse <url>` then opens pages in it.";

pub fn main(args: &[String]) {
    if !cockpit_core::options::CHROMIUM_HERE {
        eprintln!("{}", browser::MAC_ONLY);
        std::process::exit(2);
    }
    let rest: Vec<&str> = args.iter().map(String::as_str).collect();
    match rest.as_slice() {
        ["install"] => {
            let pins = install::pins().unwrap_or_else(|why| fail(&format!("browser install: {why}")));
            match install::install(&pins, &mut |line| println!("{line}")) {
                Ok(()) => println!("the Chromium side browser is installed (Electron {}, agent-browser {})", pins.electron.version, pins.agent.version),
                Err(why) => fail(&format!("browser install: {why}")),
            }
        }
        ["status"] | ["status", "--json"] => {
            let status = status();
            if rest.len() == 2 {
                println!("{status}");
            } else {
                println!("{}", status_lines(&status).join("\n"));
            }
        }
        ["quit"] => {
            if browser::connect().is_some() {
                browser::request(&json!({"op": "quit"}), Duration::from_secs(10)).unwrap_or_else(|why| fail(&format!("browser quit: {why}")));
            }
            agent::prune();
        }
        // update.sh runs this: nothing to do unless Chromium is the side browser here.
        ["refresh"] => {
            if cockpit_core::files::config()["sideBrowser"] != cockpit_core::options::CHROMIUM {
                return;
            }
            let pins = install::pins().unwrap_or_else(|why| fail(&format!("browser refresh: {why}")));
            if install::installed(&pins) != (true, true) {
                install::install(&pins, &mut |line| println!("{line}")).unwrap_or_else(|why| fail(&format!("browser refresh: {why}")));
            }
            // The running shell is the version before: its tabs come back, asleep, with the next page opened.
            if browser::connect().is_some() {
                let _ = browser::request(&json!({"op": "quit"}), Duration::from_secs(10));
            }
            agent::prune();
        }
        ["profiles"] => match browser::request(&json!({"op": "profiles.list"}), Duration::from_secs(10)) {
            Ok(answer) => println!("{answer}"),
            Err(why) => fail(&format!("browser profiles: {why}")),
        },
        ["import-cookies", profile, "--from", source] => {
            let (source, from) = source.split_once(':').map_or((*source, None), |(s, p)| (s, Some(p)));
            person("import_cookies", json!({"profile": profile, "source": source, "from": from}));
        }
        ["import-cookies", profile, "--file", file] => {
            let file = std::path::absolute(file).unwrap_or_else(|_| file.into());
            person("import_cookies", json!({"profile": profile, "file": file}));
        }
        ["track", "--host", pid] | ["track", "--host", pid, "--once"] => {
            let pid = pid.parse().unwrap_or_else(|_| fail("browser track: --host takes a process id"));
            track::run(pid, rest.len() == 4);
        }
        _ => fail(USAGE),
    }
}

/// What only a person asks, through the hub, which checks who is asking.
fn person(action: &str, arg: Value) {
    let request = json!({"op": "act", "app": "browser", "action": action, "args": [arg]});
    match cockpit_core::hub::request(&request, Duration::from_secs(120)) {
        Ok(answer) if answer["error"].is_null() => println!("{}", answer["note"].as_str().unwrap_or_default()),
        Ok(answer) => fail(&format!("browser: {}", answer["error"].as_str().unwrap_or_default())),
        Err(why) => fail(&format!("browser: {why}")),
    }
}

/// What is installed and what runs: `{enabled, installed: {electron, agentBrowser}, versions, running, shell}`.
pub fn status() -> Value {
    let (engine, why) = browser::engine_for(&cockpit_core::files::config(), cockpit_core::frontend::current(), true);
    let pins = install::pins();
    let installed = pins.as_ref().map(install::installed).unwrap_or((false, false));
    let shell = browser::request(&json!({"op": "status"}), Duration::from_secs(5)).ok();
    json!({
        "enabled": engine == browser::Engine::Chromium,
        "why": why,
        "installed": {"electron": installed.0, "agentBrowser": installed.1},
        "versions": pins.as_ref().map_or(Value::Null, |p| json!({"electron": p.electron.version, "agentBrowser": p.agent.version})),
        "pins": pins.as_ref().err(),
        "running": shell.is_some(),
        "shell": shell,
    })
}

fn status_lines(status: &Value) -> Vec<String> {
    let yes = |v: &Value, yes: &str, no: &str| if v == true { yes.to_string() } else { no.to_string() };
    let mut lines = vec![
        format!(
            "side browser: {}",
            yes(&status["enabled"], "Chromium", status["why"].as_str().unwrap_or("terminal-browser (cockpit options set sideBrowser chromium)"))
        ),
        format!(
            "Electron {}: {}",
            status["versions"]["electron"].as_str().unwrap_or("?"),
            yes(&status["installed"]["electron"], "installed", "missing (cockpit browser install)")
        ),
        format!(
            "agent-browser {}: {}",
            status["versions"]["agentBrowser"].as_str().unwrap_or("?"),
            yes(&status["installed"]["agentBrowser"], "installed", "missing (cockpit browser install)")
        ),
    ];
    match status["shell"].as_object() {
        Some(shell) => {
            let tabs: u64 = shell["browsers"].as_array().into_iter().flatten().map(|w| w["tabs"].as_u64().unwrap_or(0)).sum();
            lines.push(format!(
                "running: pid {}, {}, {tabs} tabs for {} sessions",
                shell["pid"],
                yes(&shell["docked"], "docked", "a window of its own"),
                shell["browsers"].as_array().map_or(0, Vec::len)
            ));
        }
        None => lines.push("not running: the next page opened starts it".into()),
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_status_reads_in_lines() {
        let status = json!({
            "enabled": true, "installed": {"electron": true, "agentBrowser": false}, "versions": {"electron": "44.5.1", "agentBrowser": "0.38.1"},
            "shell": {"pid": 4242, "docked": true, "browsers": [{"tabs": 2}, {"tabs": 1}]},
        });
        assert_eq!(
            status_lines(&status),
            [
                "side browser: Chromium",
                "Electron 44.5.1: installed",
                "agent-browser 0.38.1: missing (cockpit browser install)",
                "running: pid 4242, docked, 3 tabs for 2 sessions"
            ]
        );
        let off = json!({"enabled": false, "why": browser::APP_ONLY, "installed": {}, "versions": {}});
        assert_eq!(status_lines(&off)[0], format!("side browser: {}", browser::APP_ONLY));
        assert_eq!(status_lines(&off)[3], "not running: the next page opened starts it");
    }
}
