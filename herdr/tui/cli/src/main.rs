//! cockpit, the plugin's one binary: the space apps, the show view and the hub run as its subcommands, beside what the
//! plugin's scripts and herdr's events call.

mod act;
mod actions;
mod browse;
mod doctor;
mod find;
mod focus;
mod gui;
mod home;
mod proc;
mod queue;
mod routine;
mod shots;
mod space;
mod store;
mod update;

/// mimalloc hands freed pages back to the system soon after: the system allocator kept the tens of megabytes a show
/// picture's draw frees, so an idle pane held them.
#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

const USAGE: &str = "cockpit, the plugin's command.

    cockpit inbox|runbook|todo|meetings|monitor|routines
                                               a space's app (cockpit space adds cockpit-space, which update.sh looks for)
    cockpit space <app>                        keeps that app running in its space; ctrl+c twice in a row leaves it
    cockpit show <name>                        the view a Claude session shows beside itself
    cockpit palette                            the launcher herdr opens as a popup (prefix+space); PALETTE_PANE is the pane below
    cockpit agents [answer|recap|snooze|seen|fork|week]  every agent terminal: who needs you, answered from here
    cockpit search <query>                     what the palette finds, as the hub's JSON: items, the week's events, past sessions
    cockpit events [--since 30m|2h|1d|<epoch ms>] [--app inbox|monitor|routines]
                                               what changed in the shared data, newest first, as the hub's JSON (a day by default)
    cockpit hub                                the background process the apps share; the first app that needs it starts it
    cockpit hub ping                           exits 0 when the hub answers within 2 seconds
    cockpit hub start                          starts the hub when none runs, prints {port, token, pid} for the Herdr app
    cockpit hub stop                           stops the hub; the next app that needs it starts a new one
    cockpit hub shown <view>                   the args of every drawn copy of a Herdr app view, as JSON
    cockpit app install|uninstall              the cockpit layout, mod and keys in the Herdr app (cockpit app for more)
    cockpit update-apps [herdr folder]         installs the release's binary for the plugin's version
    cockpit actions tab <cwd> <label> [key]   a tab in the actions space, its pane id printed
    cockpit actions sidebar                    the actions space's sessions listed under its sidebar row
    cockpit act <review|ci|rerun|plan|routine> <ref>
                                               what a notification's button does
    cockpit notify [--system] <title> <body> [action] [<label>=<action> ...]
                                               a notification (the Herdr app's toast when it follows the hub, else
                                               the system's; --system: the system's); a click or a button does its action
    cockpit focus <target>                     what a notification's click does (cockpit focus for more)
    cockpit doctor                             checks every part of cockpit, one ok, warn or fail line each
    cockpit browse ...                         a page in terminal-browser beside a pane (cockpit browse for more)
    cockpit shots ...                          screenshots at phone, tablet and desktop sizes (cockpit shots for more)
    cockpit store ...                          the documents the hub keeps and its leases, for the mod (cockpit store for more)
    cockpit routine run <slug>                 a routine run now and waited for: how it went, what it called, its output
    cockpit resume [look|all|<terminal>|dismiss <terminal>]
                                               the agents a restart stopped, listed, or resumed where they were
                                               (look: notes what runs now first, as the hub does every minute)
    cockpit queue [list|add <terminal> <text>|send <terminal> [all|<id>]|clear <terminal>|pick [terminal]]
                                               an agent's prompt queue (terminal: app:t3 or a herdr pane id)";

fn main() {
    cockpit_core::answer_version("cockpit");
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((command, rest)) = args.split_first() else { proc::fail(USAGE) };
    match command.as_str() {
        "inbox" => app(cockpit_inbox::run()),
        "runbook" => app(cockpit_runbook::run()),
        "todo" => app(cockpit_todo::run()),
        "meetings" => app(cockpit_meetings::run()),
        "monitor" => app(cockpit_monitor::run()),
        "routines" => app(cockpit_routines::run()),
        "show" => app(cockpit_show::run(rest.first().map_or("show", String::as_str))),
        "palette" => app(cockpit_palette::run()),
        "agents" => cockpit_agents::cli::main(rest),
        "search" => find::search(rest),
        "events" => find::events(rest),
        "hub" if rest.is_empty() => {
            gui::register();
            cockpit_hub::run()
        }
        "hub" if rest == ["start"] => gui::hub_start(),
        "hub" if rest == ["stop"] => gui::hub_stop(),
        "hub" if rest.len() == 2 && rest[0] == "shown" => gui::hub_shown(&rest[1]),
        "app" => gui::main(rest),
        "hub" if rest == ["ping"] => std::process::exit(if ping(Duration::from_secs(2)) { 0 } else { 1 }),
        "update-apps" => update::main(rest),
        "actions" => actions::main(rest),
        "act" => act::main(rest),
        "notify" => notify(rest),
        "focus" => focus::main(rest),
        "doctor" => doctor::main(rest),
        "space" => space::main(rest),
        "browse" => browse::main(rest),
        "shots" => shots::main(rest),
        "store" => store::main(rest),
        "routine" => routine::main(rest),
        "resume" => resume(rest),
        "queue" => queue::main(rest),
        _ => proc::fail(USAGE),
    }
}

/// The agents a restart stopped: listed as JSON, all resumed, one resumed, or one left stopped.
fn resume(args: &[String]) {
    use cockpit_core::resume;
    let done = match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        [] => {
            println!("{}", serde_json::Value::from(resume::lost()));
            return;
        }
        ["look"] => {
            let (record, newly) = resume::remember();
            println!("{}", serde_json::json!({"record": record, "newly_lost": newly}));
            return;
        }
        ["all"] => resume::resume_all(),
        ["dismiss", handle] => {
            resume::dismiss(handle);
            Ok(String::new())
        }
        [handle] => resume::resume(handle),
        _ => Err("cockpit resume [look|all|<terminal>|dismiss <terminal>]".into()),
    };
    match done {
        Ok(note) if !note.is_empty() => println!("{note}"),
        Ok(_) => {}
        Err(err) => proc::fail(&err),
    }
}

/// A system notification with the cockpit icon: a click does the action, each `<label>=<action>` button its own.
fn notify(args: &[String]) {
    let system = args.first().is_some_and(|a| a == "--system");
    let args = if system { &args[1..] } else { args };
    let [title, body, rest @ ..] = args else { proc::fail("cockpit notify [--system] <title> <body> [action] [<label>=<action> ...]") };
    let action = rest.first().map_or("", String::as_str);
    let buttons = rest.get(1..).unwrap_or_default();
    if system { cockpit_core::notify::post_system(title, body, action, buttons) } else { cockpit_core::notify::post(title, body, action, buttons) }
}

/// An app's end: an error that stopped it is printed, and `cockpit space` starts it again.
fn app(ended: std::io::Result<()>) {
    if let Err(error) = ended {
        proc::fail(&format!("cockpit: {error}"));
    }
}

/// Whether the hub answers a ping on its socket in time; one that is not running is not started.
fn ping(timeout: Duration) -> bool {
    let Ok(mut stream) = UnixStream::connect(cockpit_core::hub::socket()) else { return false };
    stream.set_read_timeout(Some(timeout)).ok();
    stream.set_write_timeout(Some(timeout)).ok();
    if writeln!(stream, "{{\"op\": \"ping\"}}").is_err() {
        return false;
    }
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).is_ok()
        && serde_json::from_str::<serde_json::Value>(&line).is_ok_and(|answer| answer["ok"] == true)
}
