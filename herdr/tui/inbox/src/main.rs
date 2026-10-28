//! The inbox space's app; everything lives in the library, which the snapshot tests drive too.

fn main() -> std::io::Result<()> {
    cockpit_core::answer_version("cockpit-inbox");
    cockpit_core::term::run(&mut cockpit_inbox::app::App::start())
}
