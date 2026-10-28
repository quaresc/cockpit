//! cockpit: the command the plugin's scripts and herdr's events call, for what is not an app of its own.

mod act;
mod actions;
mod browse;
mod proc;
mod shots;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

const USAGE: &str = "cockpit, the plugin's command.

    cockpit actions tab <cwd> <label> [key]   a tab in the actions space, its pane id printed
    cockpit actions sidebar                    the actions space's sessions listed under its sidebar row
    cockpit act <review|ci|rerun|plan> <ref>   what a notification's button does
    cockpit browse ...                         a page in terminal-browser beside a pane (cockpit browse for more)
    cockpit shots ...                          screenshots at phone, tablet and desktop sizes (cockpit shots for more)
    cockpit hub ping                           exits 0 when the hub answers within 2 seconds";

fn main() {
    cockpit_core::answer_version("cockpit");
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((command, rest)) = args.split_first() else { proc::fail(USAGE) };
    match command.as_str() {
        "actions" => actions::main(rest),
        "act" => act::main(rest),
        "browse" => browse::main(rest),
        "shots" => shots::main(rest),
        "hub" if rest == ["ping"] => std::process::exit(if ping(Duration::from_secs(2)) { 0 } else { 1 }),
        _ => proc::fail(USAGE),
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
