//! The Herdr app half: the views the hub serves to its cockpit mod (registered here, since the hub does not depend on
//! the app crates), `cockpit hub start` the mod runs, and `cockpit app install` that puts the layout, the mod and its
//! keys into the Herdr app's home.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use cockpit_core::{files, frontend};
use cockpit_hub::views::{self, Def};
use serde_json::{Map, Value, json};

use crate::proc::fail;

/// Every app's view; an app crate adds its own `Def` here.
pub fn register() {
    views::register(Def {
        name: "agents",
        docs: &["inbox", "ci", "agents"],
        dirs: cockpit_agents::gui::dirs,
        build: cockpit_agents::gui::view,
        act: cockpit_agents::gui::act,
    });
    views::register(Def {
        name: "session",
        docs: &[],
        dirs: cockpit_agents::session::dirs,
        build: cockpit_agents::session::view,
        act: cockpit_agents::session::act,
    });
    // Who sent a request is placed by the agents' process trees: what only a person may do is refused to the rest.
    cockpit_core::peer::on_place(cockpit_agents::gui::place);
    // Words for an agent go through one door: a session cockpit drives takes them, and none goes into a question.
    cockpit_core::deliver::on_driven(cockpit_agents::session::deliver_to);
    cockpit_core::deliver::on_waiting(cockpit_agents::gui::waiting);
    // A session that moves can change what the agents view and the chat show: a new ask, an answered one.
    cockpit_agents::session::on_change(
        || views::touch("session"),
        || {
            views::touch("agents");
            views::touch("chat");
        },
    );
    // The hub stays, and an update waits, while a session it drives is in a turn; one at rest comes back paused in the
    // hub that starts next.
    cockpit_hub::keep_alive(cockpit_agents::session::driving);
    cockpit_hub::on_start(cockpit_agents::session::restore);
    cockpit_agents::session::on_hand_over(cockpit_hub::store::handed_over);
    // A session goes to a terminal only with nothing left in its queue.
    cockpit_agents::session::on_queued(crate::queue::waiting);
    views::register(Def {
        name: "chat",
        docs: &["inbox", "ci", "agents"],
        dirs: cockpit_agents::chat::dirs,
        build: cockpit_agents::chat::view,
        act: cockpit_agents::chat::act,
    });
    // A long turn writes its transcript without its status moving: the chat is built again as the transcript grows.
    cockpit_agents::chat::on_growth(|| views::touch("chat"));
    cockpit_agents::gui::start(|| {
        views::touch("agents");
        views::touch("chat");
    });
    // A failed or stopped turn holds the agent's queue; an agent at rest gets its next prompt.
    cockpit_agents::gui::on_turn_end(crate::queue::turn_ended);
    cockpit_agents::gui::on_rest(crate::queue::rested);
    views::register(Def {
        name: "inbox",
        docs: &["inbox", "ci", "brief"],
        dirs: cockpit_inbox::gui::dirs,
        build: cockpit_inbox::gui::view,
        act: cockpit_inbox::gui::act,
    });
    views::register(Def { name: "todo", docs: &["inbox"], dirs: cockpit_todo::gui::dirs, build: cockpit_todo::gui::view, act: cockpit_todo::gui::act });
    views::register(Def { name: "palette", docs: &[], dirs: cockpit_palette::gui::dirs, build: cockpit_palette::gui::view, act: cockpit_palette::gui::act });
    // Start work's runs, step by step; their actions are the hub's own (`act start work|cancel|retry|dismiss`).
    views::register(Def { name: "start", docs: &[], dirs: Vec::new, build: cockpit_hub::start_view, act: |_, _| None });
    views::register(Def {
        name: "runbook",
        docs: &["inbox"],
        dirs: cockpit_runbook::gui::dirs,
        build: cockpit_runbook::gui::view,
        act: cockpit_runbook::gui::act,
    });
    views::register(Def {
        name: "meetings",
        docs: &["meetings"],
        dirs: cockpit_meetings::gui::dirs,
        build: cockpit_meetings::gui::view,
        act: cockpit_meetings::gui::act,
    });
    views::register(Def { name: "mail", docs: &["mail"], dirs: cockpit_mail::gui::dirs, build: cockpit_mail::gui::view, act: cockpit_mail::gui::act });
    views::register(Def { name: "mail-thread", docs: &[], dirs: cockpit_mail::gui::dirs, build: cockpit_mail::gui::thread_view, act: cockpit_mail::gui::act });
    views::register(Def {
        name: "monitor",
        docs: &["monitor"],
        dirs: cockpit_monitor::gui::dirs,
        build: cockpit_monitor::gui::view,
        act: cockpit_monitor::gui::act,
    });
    views::register(Def {
        name: "routines",
        docs: &["routines"],
        dirs: cockpit_routines::gui::dirs,
        build: cockpit_routines::gui::view,
        act: cockpit_routines::gui::act,
    });
    views::register(Def { name: "routines-when", docs: &[], dirs: Vec::new, build: cockpit_routines::gui::when_view, act: cockpit_routines::gui::no_act });
    views::register(Def { name: "show", docs: &[], dirs: cockpit_show::gui::dirs, build: cockpit_show::gui::view, act: cockpit_show::gui::act });
    views::register(Def { name: "show-block", docs: &[], dirs: Vec::new, build: cockpit_show::gui::block_data, act: |_, _| None });
    views::register(Def { name: "review", docs: &[], dirs: Vec::new, build: cockpit_show::review::view, act: cockpit_show::review::act });
    views::register(Def { name: "home", docs: &["inbox", "ci"], dirs: crate::home::dirs, build: crate::home::view, act: crate::home::act });
    views::register(Def { name: "queue", docs: &["queue"], dirs: crate::queue::dirs, build: crate::queue::view, act: crate::queue::act });
    views::register(Def { name: "options", docs: &[], dirs: Vec::new, build: crate::options::view, act: crate::options::act });
    views::register(Def {
        name: "usage",
        docs: &[],
        dirs: cockpit_agents::gui::dirs,
        build: |_, _| cockpit_core::usage::view(cockpit_core::seen::now_ms()),
        act: |_, _| None,
    });
    crate::browser::hub::register();
    cockpit_core::usage::stream_from(|| cockpit_agents::session::keys().iter().filter_map(|k| cockpit_agents::session::summary(k)).collect());
}

/// `cockpit hub`: the hub on the person's environment, serving every app's view.
pub fn hub() {
    crate::hub_env::settle();
    // Before its first look at what is published, which rewrites the version update.json was written by.
    cockpit_core::options::note_run(env!("CARGO_PKG_VERSION"));
    register();
    cockpit_hub::run()
}

// ---------------------------------------------------------------------------------------------------------------------
// cockpit hub start

/// Starts the hub when none answers, waits for its TCP side and prints hub.json: {port, token, pid}. A hub holding the
/// socket without a TCP side (an older one) is stopped and replaced once.
pub fn hub_start() {
    // A person starts it: whatever `cockpit hub stop` said, a hub may start, and the windows may start one again.
    let _ = std::fs::remove_file(cockpit_core::hub::stopped_file());
    let path = files::cache().join("hub.json");
    for attempt in 0..2 {
        let _ = cockpit_core::hub::request(&json!({"op": "ping"}), Duration::from_secs(3));
        // A hub whose socket takes a connection is there, only slow to answer or to write hub.json: waited on, never
        // stopped.
        let there = std::os::unix::net::UnixStream::connect(cockpit_core::hub::socket()).is_ok();
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(if there { 15 } else { 3 }) {
            if let Some(info) = files::load(&path).filter(|i| alive(i["pid"].as_u64().unwrap_or(0))) {
                println!("{info}");
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        if attempt == 0 && !there {
            stop(&hubs());
        }
    }
    fail("the cockpit hub runs without its TCP side and did not stop (cockpit hub stop, then try again)");
}

fn alive(pid: u64) -> bool {
    // SAFETY: kill with signal 0 only checks the process exists.
    pid > 0 && unsafe { libc::kill(pid as i32, 0) } == 0
}

/// The hubs of this home: the one hub.json names and whoever holds the unix socket, which can be another one.
fn hubs() -> Vec<u64> {
    let named = files::load(&files::cache().join("hub.json")).and_then(|i| i["pid"].as_u64());
    let held = std::process::Command::new("lsof")
        .arg("-t")
        .arg(cockpit_core::hub::socket())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let mut pids: Vec<u64> = named.into_iter().chain(held.lines().filter_map(|l| l.trim().parse().ok())).collect();
    pids.sort_unstable();
    pids.dedup();
    pids.retain(|&pid| pid != u64::from(std::process::id()) && alive(pid) && is_hub(pid));
    pids
}

fn is_hub(pid: u64) -> bool {
    let command = std::process::Command::new("ps")
        .args(["-o", "command=", "-p", &pid.to_string()])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    command.trim_end().ends_with("cockpit hub")
}

/// SIGTERM to each, then waits a few seconds for them to go.
fn stop(pids: &[u64]) {
    for &pid in pids {
        // SAFETY: a plain SIGTERM to a hub's own pid.
        unsafe { libc::kill(pid as i32, libc::SIGTERM) };
    }
    let started = Instant::now();
    while pids.iter().any(|&p| alive(p)) && started.elapsed() < Duration::from_secs(3) {
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Stops the running hubs of this home and says so to the windows, which wait for Start it or the next command that
/// starts one rather than starting it again themselves.
pub fn hub_stop() {
    let _ = std::fs::write(cockpit_core::hub::stopped_file(), b"");
    stop(&hubs());
}

/// Prints the args of every drawn copy of a view, as JSON: `[]` when none is, or no hub runs.
pub fn hub_shown(view: &str) {
    let answer = cockpit_core::hub::request(&json!({"op": "shown", "view": view}), Duration::from_secs(5)).unwrap_or_default();
    println!("{}", answer.get("args").cloned().unwrap_or(json!([])));
}

// ---------------------------------------------------------------------------------------------------------------------
// cockpit app install

const USAGE: &str = "The Herdr app half of cockpit.

    cockpit app install [--watch] [--from <dir>]  the cockpit layouts, mod, themes and keys into the Herdr app's home
                                                  (HERDR_APP_HOME, else ~/.config/herdr-app); --watch keeps copying
                                                  what changes in the checkout, for working on them (Herdr loads no
                                                  mod through a symlink: its icons would not show)
    cockpit app uninstall                         takes the layouts, the mod and cockpit.json out again (keys and themes stay)";

pub fn main(args: &[String]) {
    match args.first().map(String::as_str) {
        Some("install") => {
            let from = args.iter().position(|a| a == "--from").and_then(|i| args.get(i + 1)).map(PathBuf::from).unwrap_or_else(source);
            let from = std::fs::canonicalize(&from).unwrap_or(from);
            let home = frontend::app_home();
            let first = !home.join("layouts/cockpit.json").exists();
            match install(&from, &home, false) {
                Ok(lines) => lines.iter().for_each(|l| println!("{l}")),
                Err(err) => fail(&format!("cockpit app install: {err}")),
            }
            // The first install shows the cockpit layout once; Herdr's own stays a cmd-shift-l away.
            if first && frontend::app_do("layout.switch", &json!({"name": "cockpit"})).is_some() {
                println!("switched the Herdr window to the cockpit layout (cmd-shift-l switches layouts)");
            }
            if args.iter().any(|a| a == "--watch") {
                watch(&from, &home);
            }
        }
        Some("uninstall") => {
            let home = frontend::app_home();
            for name in ["cockpit.json", "cockpit.svg", "cockpit-chat.json", "cockpit-chat.svg"] {
                let _ = std::fs::remove_file(home.join("layouts").join(name));
            }
            let _ = remove(&home.join("mods/cockpit"));
            let _ = std::fs::remove_file(home.join("cockpit.json"));
            println!("removed the cockpit layout and mod from {}", home.display());
        }
        _ => fail(USAGE),
    }
}

/// Copies each file that changes under the checkout's app folder to its place in the Herdr home, until killed.
fn watch(from: &Path, home: &Path) {
    use notify::{RecursiveMode, Watcher};
    let (tx, rx) = std::sync::mpsc::channel();
    let Ok(mut watcher) = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if let Ok(event) = event {
            event.paths.into_iter().for_each(|p| _ = tx.send(p));
        }
    }) else {
        fail("cockpit app install --watch: cannot watch the checkout")
    };
    if watcher.watch(from, RecursiveMode::Recursive).is_err() {
        fail("cockpit app install --watch: cannot watch the checkout");
    }
    println!("watching {}", from.display());
    while let Ok(first) = rx.recv() {
        std::thread::sleep(Duration::from_millis(150));
        let mut paths: Vec<PathBuf> = std::iter::once(first).chain(rx.try_iter()).collect();
        paths.sort();
        paths.dedup();
        for path in paths {
            let Ok(rel) = path.strip_prefix(from) else { continue };
            let to = match rel.iter().next().and_then(|c| c.to_str()) {
                Some("mods" | "layouts" | "themes") => home.join(rel),
                _ => continue,
            };
            if path.is_file() {
                let _ = std::fs::create_dir_all(to.parent().unwrap_or(home));
                if std::fs::copy(&path, &to).is_ok() {
                    println!("copied {}", rel.display());
                }
            } else if !path.exists() {
                let _ = std::fs::remove_file(&to);
            }
        }
    }
}

/// The checkout's app/ folder beside the herdr plugin folder, else the one the release packs beside this binary.
fn source() -> PathBuf {
    // Through the ~/.config/cockpit/herdr link, the checkout's app/ is beside the folder the link names.
    let herdr = files::herdr_dir();
    let herdr = std::fs::canonicalize(&herdr).unwrap_or(herdr);
    let exe = std::env::current_exe().map(|e| std::fs::canonicalize(&e).unwrap_or(e)).unwrap_or_default();
    let packed = exe.parent().map(|p| p.join("app")).unwrap_or_default();
    [herdr.join("app"), herdr.parent().map(|p| p.join("app")).unwrap_or_default(), packed]
        .into_iter()
        .find(|p| p.join("mods/cockpit/main.js").exists())
        .unwrap_or_default()
}

fn remove(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(_) => Ok(()),
    }
}

/// A file copied, or linked to its source.
fn place(from: &Path, to: &Path, link: bool) -> std::io::Result<()> {
    remove(to)?;
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if link { std::os::unix::fs::symlink(from, to) } else { std::fs::copy(from, to).map(|_| ()) }
}

/// Every file under a folder, folders followed, as paths relative to it.
fn tree(dir: &Path, under: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        let rel = under.join(entry.file_name());
        if path.is_dir() { tree(&path, &rel, out) } else { out.push(rel) }
    }
}

/// Puts the layout and the mod in place (copied, or the folder linked to the checkout), writes cockpit.json for the mod
/// and merges the keys; the lines say what it did.
/// The COCKPIT_* variables set for the install (a sandbox's test hooks), which go to every cockpit command the mod runs.
/// Keys from outside the window are never one: the mod reads COCKPIT_TEST_KEYS from its own process alone.
fn install_env(vars: impl Iterator<Item = (String, String)>) -> Map<String, Value> {
    vars.filter(|(k, _)| k.starts_with("COCKPIT_") && k != "COCKPIT_FRONTEND" && k != "COCKPIT_TEST_KEYS").map(|(k, v)| (k, Value::from(v))).collect()
}

pub fn install(from: &Path, home: &Path, link: bool) -> Result<Vec<String>, String> {
    if !from.join("mods/cockpit/main.js").exists() {
        return Err(format!("no cockpit mod in {}: pass --from <the checkout's app folder>", from.display()));
    }
    let mut lines = Vec::new();
    let err = |e: std::io::Error| e.to_string();
    let mut layouts = Vec::new();
    tree(&from.join("layouts"), Path::new(""), &mut layouts);
    for rel in &layouts {
        place(&from.join("layouts").join(rel), &home.join("layouts").join(rel), link).map_err(err)?;
    }
    // A layout's own theme is the person's to edit afterwards, so one already there is left alone.
    let mut themes = Vec::new();
    tree(&from.join("themes"), Path::new(""), &mut themes);
    for rel in &themes {
        let to = home.join("themes").join(rel);
        if !to.exists() {
            place(&from.join("themes").join(rel), &to, link).map_err(err)?;
        }
    }
    let mod_dir = home.join("mods/cockpit");
    let mut files_in = Vec::new();
    tree(&from.join("mods/cockpit"), Path::new(""), &mut files_in);
    if link {
        // Herdr refuses a module whose file resolves outside the mod's folder: the folder itself is the link.
        place(&from.join("mods/cockpit"), &mod_dir, true).map_err(err)?;
    } else {
        // Filled beside it and swapped in by renames, so neither Herdr reloading nor a watcher copying sees half a mod.
        let (new, old) = (home.join(".cockpit-mod.new"), home.join(".cockpit-mod.old"));
        remove(&new).and_then(|_| remove(&old)).map_err(err)?;
        for rel in &files_in {
            place(&from.join("mods/cockpit").join(rel), &new.join(rel), false).map_err(err)?;
        }
        if std::fs::symlink_metadata(&mod_dir).is_ok() {
            std::fs::rename(&mod_dir, &old).map_err(err)?;
        }
        std::fs::create_dir_all(home.join("mods")).map_err(err)?;
        std::fs::rename(&new, &mod_dir).map_err(err)?;
        let _ = remove(&old);
    }
    lines.push(format!(
        "{} {} cockpit layouts and the mod ({} files) in {}",
        if link { "linked" } else { "copied" },
        layouts.iter().filter(|r| r.extension().is_some_and(|e| e == "json")).count(),
        files_in.len(),
        home.display()
    ));
    let exe = std::env::current_exe().map_err(err)?;
    let env = install_env(std::env::vars());
    let info = json!({"bin": exe, "cache": files::cache(), "home": files::home(), "version": env!("CARGO_PKG_VERSION"), "env": env});
    files::write(&home.join("cockpit.json"), &info).map_err(err)?;
    let (keys, skipped) = merge_keys(&files::load(&home.join("keys.json")).unwrap_or(json!({})), &files::load(&from.join("keys.json")).unwrap_or(json!({})));
    files::write(&home.join("keys.json"), &keys).map_err(err)?;
    for (section, key, theirs) in skipped {
        lines.push(format!("kept your {key} in {section} ({theirs}): bind a cockpit key there yourself if you want one"));
    }
    Ok(lines)
}

/// Bindings cockpit replaced, as (section, key, command): an install takes out exactly these, so their key takes what
/// cockpit binds there now, and a key the person bound themselves stays theirs.
const RETIRED: &[(&str, &str, &str)] = &[("layout:cockpit-chat", "escape", "cockpit.stop"), ("layout:cockpit-chat", "shift-tab", "cockpit.mode")];

/// The person's keys with cockpit's added; a key they already bind to something else stays theirs: (section, key,
/// their action) for each. A binding cockpit replaced (`RETIRED`) goes first.
pub fn merge_keys(theirs: &Value, ours: &Value) -> (Value, Vec<(String, String, String)>) {
    let mut merged = theirs.as_object().cloned().unwrap_or_default();
    let mut skipped = Vec::new();
    for (section, keys) in ours.as_object().into_iter().flatten() {
        let into = merged.entry(section.clone()).or_insert_with(|| Value::Object(Map::new()));
        let Some(into) = into.as_object_mut() else { continue };
        into.retain(|key, action| !RETIRED.iter().any(|(s, k, retired)| s == section && k == key && action.as_str() == Some(retired)));
        for (key, action) in keys.as_object().into_iter().flatten() {
            match into.get(key) {
                Some(existing) if existing != action => skipped.push((section.clone(), key.clone(), existing.as_str().unwrap_or_default().to_string())),
                _ => _ = into.insert(key.clone(), action.clone()),
            }
        }
    }
    (Value::Object(merged), skipped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_merge_without_taking_the_persons() {
        let theirs = json!({"global": {"cmd-k": "app.palette", "ctrl-shift-t": "tab.new"}});
        let ours = json!({"global": {"cmd-k": "cockpit.palette", "cmd-1": "cockpit.inbox"}, "mod:cockpit": {"/": "cockpit.filter"}});
        let (merged, skipped) = merge_keys(&theirs, &ours);
        assert_eq!(merged["global"]["cmd-k"], "app.palette");
        assert_eq!(merged["global"]["cmd-1"], "cockpit.inbox");
        assert_eq!(merged["mod:cockpit"]["/"], "cockpit.filter");
        assert_eq!(skipped, [("global".to_string(), "cmd-k".to_string(), "app.palette".to_string())]);
        let (again, none) = merge_keys(&merged, &ours);
        assert_eq!((again, none.len()), (merged, 1));
    }

    #[test]
    fn the_bindings_cockpit_replaced_go_and_the_persons_own_stay() {
        let theirs = json!({"layout:cockpit-chat": {
            "escape": "cockpit.stop", "shift-tab": "cockpit.mode", "ctrl-.": "cockpit.stop",
            "cmd-u": "cockpit.go:mail", "ctrl-x": "app.palette", "cmd-shift-p": "cockpit.palette"},
            "layout:cockpit": {"escape": "cockpit.stop"}});
        let ours = json!({"layout:cockpit-chat": {"escape": "cockpit.chat:stop", "shift-tab": "cockpit.chat:mode", "cmd-shift-p": "cockpit.palette"}});
        let (merged, skipped) = merge_keys(&theirs, &ours);
        let chat = &merged["layout:cockpit-chat"];
        assert_eq!((chat["escape"].as_str(), chat["shift-tab"].as_str()), (Some("cockpit.chat:stop"), Some("cockpit.chat:mode")));
        assert_eq!(chat["ctrl-."], "cockpit.stop", "a key the person bound themselves stays, whatever it runs");
        assert_eq!((chat["cmd-u"].as_str(), chat["ctrl-x"].as_str()), (Some("cockpit.go:mail"), Some("app.palette")), "the person's own stay");
        assert_eq!(merged["layout:cockpit"]["escape"], "cockpit.stop", "another section is left as it is");
        assert!(skipped.is_empty(), "{skipped:?}");
    }

    #[test]
    fn the_install_never_turns_on_keys_from_outside_the_window() {
        let vars = [("COCKPIT_HUB_FEED", "off"), ("COCKPIT_TEST_KEYS", "1"), ("COCKPIT_FRONTEND", "app"), ("HOME", "/h")];
        let env = install_env(vars.iter().map(|(k, v)| (k.to_string(), v.to_string())));
        assert_eq!(Value::Object(env), json!({"COCKPIT_HUB_FEED": "off"}));
    }

    #[test]
    fn installs_the_layout_and_mod_and_says_where_cockpit_is() {
        let from = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        for (path, text) in [
            ("mods/cockpit/main.js", "x"),
            ("mods/cockpit/ui.js", "y"),
            ("layouts/cockpit.json", "{}"),
            ("layouts/cockpit.svg", "<svg/>"),
            ("layouts/cockpit-chat.json", "{}"),
            ("themes/cockpit-chat.json", "{}"),
            ("keys.json", "{\"global\": {\"cmd-k\": \"cockpit.palette\"}}"),
        ] {
            let p = from.path().join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        std::fs::write(home.path().join("themes-kept.json"), "mine").unwrap();
        let lines = install(from.path(), home.path(), true).unwrap();
        assert!(lines[0].starts_with("linked 2 cockpit layouts and the mod (2 files)"), "{}", lines[0]);
        assert!(home.path().join("layouts/cockpit-chat.json").exists());
        assert!(home.path().join("themes/cockpit-chat.json").exists());
        assert!(std::fs::symlink_metadata(home.path().join("mods/cockpit")).unwrap().file_type().is_symlink());
        assert!(files::load(&home.path().join("cockpit.json")).unwrap()["bin"].is_string());
        assert_eq!(files::load(&home.path().join("keys.json")).unwrap()["global"]["cmd-k"], "cockpit.palette");
        // A theme the person edited is theirs: the next install leaves it alone.
        std::fs::write(home.path().join("themes/cockpit-chat.json"), "mine").unwrap();
        install(from.path(), home.path(), false).unwrap();
        assert!(!std::fs::symlink_metadata(home.path().join("mods/cockpit/ui.js")).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_to_string(home.path().join("themes/cockpit-chat.json")).unwrap(), "mine");
    }
}
