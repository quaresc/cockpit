//! cockpit-todo: the to-do space's app.

fn main() -> std::io::Result<()> {
    cockpit_core::answer_version("cockpit-todo");
    cockpit_core::term::run(&mut cockpit_todo::app::App::start())
}
