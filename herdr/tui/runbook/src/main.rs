//! The runbook app's binary: the library's app in the terminal loop.

fn main() -> std::io::Result<()> {
    cockpit_core::answer_version("cockpit-runbook");
    cockpit_core::term::run(&mut cockpit_runbook::app::App::new())
}
