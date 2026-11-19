//! The apps' actions: each one takes the JSON arguments its app passes and leaves a note, the requests it filed and the
//! refreshes it started.

mod book;
mod doctor;
mod inbox;
mod land;
mod mail;
mod meetings;
mod monitor;
mod pins;
mod routines;
mod runbook;
mod show;
mod sources;
mod space;
mod start;
mod tickets;
mod todo;

pub use book::{read_books, runbooks_dir};
#[cfg(test)]
pub(crate) use inbox::pick_agent;
pub(crate) use inbox::{agent_on, failure_lines, one_line, quoted};
pub use land::{plan as start_plan, pr_merge};
pub use start::{plan_json as start_work, running as start_running, view as start_view};
pub use tickets::card as ticket_card;

use std::collections::HashMap;

use serde_json::{Map, Value, json};

use crate::config::{self, Config};
use crate::value::Res;

/// What an action leaves behind, for its reply.
pub struct Ctx {
    /// The confirm key a dialog already asked about: `confirmed` holds for it alone.
    armed: Option<String>,
    note: String,
    pending: Map<String, Value>,
    refreshing: Map<String, Value>,
    cfg: Config,
    /// The asking app's environment, for what the hub's own cannot say (its pane).
    env: HashMap<String, String>,
    /// Why the action refused, said by a helper that returns nothing.
    refused: Option<String>,
}

impl Ctx {
    pub fn new(armed: Option<String>, env: HashMap<String, String>) -> Self {
        Self { armed, note: String::new(), pending: Map::new(), refreshing: Map::new(), cfg: config::load(), env, refused: None }
    }

    /// The action fails with these words, whatever it returns.
    fn refuse(&mut self, why: impl Into<String>) {
        self.refused = Some(why.into());
    }

    fn note(&mut self, text: impl Into<String>) {
        self.note = text.into();
    }

    /// view.confirmed: true on the press a dialog already asked about.
    fn confirmed(&self, key: &str) -> bool {
        self.armed.as_deref() == Some(key)
    }

    fn repos(&self) -> String {
        self.cfg.repos()
    }
}

type Action = fn(&mut Ctx, &[Value]) -> Res;

fn action(app: &str, name: &str) -> Option<Action> {
    let found: Action = match (app, name) {
        ("inbox", "review") => inbox::review,
        ("inbox", "details") => inbox::details,
        ("inbox", "open_url") => inbox::open_url,
        ("inbox", "set_muted") => inbox::set_muted,
        ("home", "pin_order") => pins::pin_order,
        ("inbox", "fix_ci") => inbox::fix_ci,
        ("inbox", "address_review") => inbox::address_review,
        ("inbox", "rerun") => inbox::rerun,
        ("inbox", "redeploy") => inbox::redeploy,
        ("inbox", "create_preview") => inbox::create_preview,
        ("inbox", "plan") => inbox::plan,
        ("inbox", "continue_ticket") => inbox::continue_ticket,
        ("inbox", "talk") => inbox::talk,
        ("inbox", "mark_done") => inbox::mark_done,
        ("inbox", "extend") => inbox::extend,
        ("inbox", "stop_preview") => inbox::stop_preview,
        ("inbox", "close_space") => inbox::close_space,
        ("inbox", "run") => inbox::run,
        ("inbox", "dismiss_brief") => inbox::dismiss_brief,
        ("inbox", "request_refresh") => inbox::request_refresh,
        ("inbox", "merge") => land::merge,
        ("inbox", "automerge") => land::automerge,
        ("inbox", "ready") => land::ready,
        ("inbox", "draft") => land::draft,
        ("inbox", "dequeue") => land::dequeue,
        ("inbox", "start") => land::start,
        ("inbox", "remove_worktree") => land::remove_worktree,
        ("inbox", "open_worktree") => land::open_worktree,
        ("inbox", "set_state") => tickets::set_state,
        ("inbox", "assign") => tickets::assign,
        ("inbox", "file_issue") => tickets::file_issue,
        ("mail", "mail_open") => mail::open,
        ("mail", "mail_archive") => mail::archive,
        ("mail", "mail_read") => mail::read,
        ("mail", "mail_reply") => mail::reply,
        ("mail", "mail_thread") => mail::thread,
        ("mail", "request_refresh") => inbox::request_refresh,
        ("runbook", "run_step") => runbook::run_step,
        ("runbook", "mark_done") => runbook::mark_done,
        ("runbook", "open_pr") => runbook::open_pr,
        ("runbook", "focus_session") => runbook::focus_session,
        ("runbook", "update") => runbook::update,
        ("runbook", "talk") => runbook::talk,
        ("runbook", "delete") => runbook::delete,
        ("runbook", "announce") => runbook::announce,
        ("runbook", "copy") => runbook::copy,
        ("show", "open_reviewr") => show::open_reviewr,
        ("meetings", "prepare") => meetings::prepare,
        ("meetings", "open_url") => meetings::open_url,
        ("meetings", "join") => meetings::join,
        ("todo", "open_link") => todo::open_link,
        ("monitor", "investigate") => monitor::investigate,
        ("monitor", "fix") => monitor::fix,
        ("monitor", "bug") => monitor::bug,
        ("monitor", "roll_back") => monitor::roll_back,
        ("monitor", "talk") => monitor::talk,
        ("monitor", "pin") => monitor::pin,
        ("monitor", "mute") => monitor::mute,
        ("monitor", "request_refresh") => monitor::request_refresh,
        ("monitor", "open_url") => monitor::open_url,
        ("routines", "add") => routines::add,
        ("routines", "run") => routines::run,
        ("routines", "stop") => routines::stop,
        ("routines", "pause") => routines::pause,
        ("routines", "model") => routines::set_model,
        ("routines", "delete") => routines::delete,
        ("routines", "edit") => routines::edit,
        ("routines", "output") => routines::output,
        ("routines", "talk") => routines::talk,
        ("routines", "request_refresh") => routines::request_refresh,
        ("routines", "update") => routines::update,
        ("routines", "read") => routines::read,
        ("routines", "read_all") => routines::read_all,
        ("routines", "open_worktree") => routines::open_worktree,
        ("routines", "cleanup") => routines::cleanup,
        ("start", "work") => start::work,
        ("start", "cancel") => start::cancel,
        ("start", "retry") => start::retry,
        ("start", "dismiss") => start::dismiss,
        ("inbox", "start_retry") => start::retry,
        ("inbox", "start_cancel") => start::inbox_cancel,
        ("inbox", "start_dismiss") => start::dismiss,
        ("sources", "sign_in") => sources::sign_in,
        ("sources", "refresh") => sources::refresh,
        ("doctor", "fix") => doctor::fix,
        ("doctor", "fix_all") => doctor::fix_all,
        ("update", "check") => check_update,
        _ => return None,
    };
    Some(found)
}

/// `update check`: a look at the published version now, its answer the note ("Up to date", "0.41.0 is out") or why it
/// failed. Anyone may ask: it reads what is published and writes update.json, and installs nothing.
fn check_update(ctx: &mut Ctx, _args: &[Value]) -> Res {
    ctx.note(crate::feed::upgrade::check_now(cockpit_core::seen::now_ms())?);
    Ok(())
}

/// The runbook actions whose first argument is a runbook: named by its slug alone (`{"$book": slug}`), so its path and
/// the session its owner file names are the runbooks folder's, never a caller's.
const BY_SLUG: [&str; 5] = ["run_step", "mark_done", "update", "talk", "delete"];

/// Why a runbook action's runbook is refused: one handed over whole rather than by its slug, or a slug no runbook has.
fn book_refused(app: &str, name: &str, args: &[Value]) -> Option<String> {
    if app != "runbook" || !BY_SLUG.contains(&name) {
        return None;
    }
    let Some(slug) = args.first().and_then(|a| a.get("$book")).and_then(Value::as_str) else {
        return Some("a runbook is named by its slug, {\"$book\": slug}".into());
    };
    let known = read_books(&runbooks_dir()).iter().any(|b| b["slug"] == slug);
    (!known).then(|| format!("no runbook {slug:?}"))
}

/// The arguments' markers: {"$book": slug} for that runbook, {"$handoff": id} for that handoff, {"$inbox": true} for the
/// inbox cache, {"$monitor": id} for that app of the monitor document.
fn resolve(arg: Value) -> Value {
    if let Some(slug) = arg.get("$book") {
        return read_books(&runbooks_dir()).into_iter().find(|b| &b["slug"] == slug).unwrap_or(Value::Null);
    }
    if let Some(id) = arg.get("$handoff").and_then(Value::as_str) {
        return book::read_handoffs().remove(id).unwrap_or(Value::Null);
    }
    if arg.get("$inbox").is_some_and(crate::value::truthy) {
        return config::read_inbox();
    }
    if let Some(id) = arg.get("$monitor") {
        return monitor::app_of(id);
    }
    arg
}

/// The actions only a person takes: each starts or resumes an agent, hands it words, merges, deploys or rolls back,
/// runs a command, writes to a tracker or a mailbox, changes a routine, removes or closes something, or brings a space
/// to the front.
const BY_A_PERSON: [(&str, &str); 58] = [
    ("inbox", "review"),
    ("inbox", "fix_ci"),
    ("inbox", "address_review"),
    ("inbox", "rerun"),
    ("inbox", "redeploy"),
    ("inbox", "create_preview"),
    ("inbox", "plan"),
    ("inbox", "continue_ticket"),
    ("inbox", "talk"),
    ("inbox", "mark_done"),
    ("inbox", "extend"),
    ("inbox", "stop_preview"),
    ("inbox", "close_space"),
    ("inbox", "run"),
    ("inbox", "merge"),
    ("inbox", "automerge"),
    ("inbox", "ready"),
    ("inbox", "draft"),
    ("inbox", "dequeue"),
    ("inbox", "start"),
    ("inbox", "remove_worktree"),
    ("inbox", "open_worktree"),
    ("inbox", "set_state"),
    ("inbox", "assign"),
    ("inbox", "file_issue"),
    ("inbox", "start_retry"),
    ("inbox", "start_cancel"),
    ("inbox", "start_dismiss"),
    ("mail", "mail_archive"),
    ("mail", "mail_read"),
    ("mail", "mail_reply"),
    ("runbook", "run_step"),
    ("runbook", "mark_done"),
    ("runbook", "update"),
    ("runbook", "focus_session"),
    ("runbook", "talk"),
    ("runbook", "delete"),
    ("show", "open_reviewr"),
    ("meetings", "prepare"),
    ("meetings", "join"),
    ("monitor", "investigate"),
    ("monitor", "fix"),
    ("monitor", "bug"),
    ("monitor", "roll_back"),
    ("monitor", "talk"),
    ("routines", "delete"),
    ("routines", "edit"),
    ("routines", "talk"),
    ("routines", "open_worktree"),
    ("routines", "cleanup"),
    ("routines", "update"),
    ("start", "work"),
    ("start", "cancel"),
    ("start", "retry"),
    ("start", "dismiss"),
    ("sources", "sign_in"),
    ("doctor", "fix"),
    ("doctor", "fix_all"),
];

/// The permission modes a routine an agent's tools add may run in: none asks less than the default.
const ASKS_AS_MUCH: [&str; 3] = ["", "default", "plan"];

/// Whether the person must be the one asking: a person's action, any action a confirm armed, whatever key it brings, or
/// a routine added in a mode that asks less than the default.
fn needs_a_person(app: &str, name: &str, armed: bool, args: &[Value]) -> bool {
    let mode = || opt(args, 11).as_str().unwrap_or_default().trim().to_string();
    armed || BY_A_PERSON.contains(&(app, name)) || ((app, name) == ("routines", "add") && !ASKS_AS_MUCH.contains(&mode().as_str()))
}

/// Runs one action: its reply {note, pending, refreshing, error}, or {"unknown": true} when the hub has no such action.
/// What only a person does is refused to an agent's own tools and to a sender cockpit cannot place.
pub fn act(app: &str, name: &str, args: Vec<Value>, armed: Option<String>, env: HashMap<String, String>) -> Value {
    let Some(run) = action(app, name) else {
        return json!({"unknown": true});
    };
    if needs_a_person(app, name, armed.is_some(), &args)
        && let Err(why) = cockpit_core::peer::person(&format!("{name} is done by a person, not by an agent's tools"))
    {
        return json!({"note": "", "pending": {}, "refreshing": {}, "error": format!("{name}: {why}")});
    }
    if let Some(why) = book_refused(app, name, &args) {
        return json!({"note": "", "pending": {}, "refreshing": {}, "error": format!("{name}: {why}")});
    }
    let mut ctx = Ctx::new(armed, env);
    let mut args: Vec<Value> = args.into_iter().map(resolve).collect();
    if app == "inbox" {
        args = land::shorthand(name, args);
    }
    let error = run(&mut ctx, &args).err().or_else(|| ctx.refused.take());
    // The person reads the error as it is; the log keeps which action said it.
    if let Some(err) = &error {
        eprintln!("cockpit hub: {app} {name}: {err}");
    }
    json!({"note": ctx.note, "pending": ctx.pending, "refreshing": ctx.refreshing, "error": error})
}

/// The n-th argument; missing, it fails like a call short of a positional argument.
fn arg(args: &[Value], n: usize) -> Res<&Value> {
    args.get(n).ok_or_else(|| format!("missing positional argument {}", n + 1))
}

/// The n-th argument, or null when not given (a keyword argument left to its default).
fn opt(args: &[Value], n: usize) -> &Value {
    static NULL: Value = Value::Null;
    args.get(n).unwrap_or(&NULL)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An agent's tools may add a routine that asks as much as the default; changing one, or adding one that asks less,
    /// is the person's, and so are dismissing a start's record and a doctor fix, one or all.
    #[test]
    fn a_routine_that_asks_less_or_a_change_to_one_needs_a_person() {
        let add = |mode: Value| {
            let mut args = vec![json!("Sweep"), json!("hourly"), json!("Sum up.")];
            args.resize(11, json!(""));
            args.push(mode);
            args
        };
        for mode in [Value::Null, json!(""), json!("default"), json!(" plan ")] {
            assert!(!needs_a_person("routines", "add", false, &add(mode.clone())), "{mode}");
        }
        for mode in ["acceptEdits", "bypassPermissions", "auto", "dontAsk", "made-up"] {
            assert!(needs_a_person("routines", "add", false, &add(json!(mode))), "{mode} asks less: a person's");
        }
        for (app, name) in [("routines", "update"), ("start", "dismiss"), ("inbox", "start_dismiss"), ("doctor", "fix"), ("doctor", "fix_all")] {
            assert!(needs_a_person(app, name, false, &[]), "{app} {name}");
            assert!(action(app, name).is_some(), "{app} {name} is an action");
        }
        assert!(!needs_a_person("routines", "read", false, &[]));
    }
}
