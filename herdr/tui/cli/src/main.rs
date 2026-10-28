//! cockpit, the plugin's one binary: the space apps, the show view and the hub run as its subcommands, beside what the
//! plugin's scripts and herdr's events call.

mod act;
mod actions;
mod browse;
mod proc;
mod shots;
mod update;

/// mimalloc hands freed pages back to the system soon after: the system allocator kept the tens of megabytes a show
/// picture's draw frees, so an idle pane held them.
#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

const USAGE: &str = "cockpit, the plugin's command.

    cockpit inbox|runbook|todo|meetings        a space's app (the space loops add cockpit-space, which update.sh looks for)
    cockpit show <name>                        the view a Claude session shows beside itself
    cockpit hub                                the background process the apps share; the first app that needs it starts it
    cockpit hub ping                           exits 0 when the hub answers within 2 seconds
    cockpit update-apps [herdr folder]         installs the release's binary for the plugin's version
    cockpit actions tab <cwd> <label> [key]   a tab in the actions space, its pane id printed
    cockpit actions sidebar                    the actions space's sessions listed under its sidebar row
    cockpit act <review|ci|rerun|plan> <ref>   what a notification's button does
    cockpit browse ...                         a page in terminal-browser beside a pane (cockpit browse for more)
    cockpit shots ...                          screenshots at phone, tablet and desktop sizes (cockpit shots for more)";

fn main() {
    cockpit_core::answer_version("cockpit");
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((command, rest)) = args.split_first() else { proc::fail(USAGE) };
    match command.as_str() {
        "inbox" => app(cockpit_inbox::run()),
        "runbook" => app(cockpit_runbook::run()),
        "todo" => app(cockpit_todo::run()),
        "meetings" => app(cockpit_meetings::run()),
        "show" => app(cockpit_show::run(rest.first().map_or("show", String::as_str))),
        "hub" if rest.is_empty() => cockpit_hub::run(),
        "hub" if rest == ["ping"] => std::process::exit(if ping(Duration::from_secs(2)) { 0 } else { 1 }),
        "update-apps" => update::main(rest),
        "actions" => actions::main(rest),
        "act" => act::main(rest),
        "browse" => browse::main(rest),
        "shots" => shots::main(rest),
        _ => proc::fail(USAGE),
    }
}

/// An app's end: an error that stopped it is printed, and the space's loop starts it again.
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
