//! The apps' actions: each one takes the JSON arguments its app passes and leaves a note, the requests it filed and the
//! refreshes it started.

mod book;
mod inbox;
mod land;
mod mail;
mod meetings;
mod monitor;
mod pins;
mod routines;
mod runbook;
mod show;
mod space;
mod tickets;
mod todo;

pub use book::{read_books, runbooks_dir};
pub use land::{plan as start_plan, pr_merge};
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
}

impl Ctx {
    pub fn new(armed: Option<String>, env: HashMap<String, String>) -> Self {
        Self { armed, note: String::new(), pending: Map::new(), refreshing: Map::new(), cfg: config::load(), env }
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
        ("meetings", "toggle_brief") => meetings::toggle_brief,
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
        _ => return None,
    };
    Some(found)
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

/// Runs one action: its reply {note, pending, refreshing, error}, or {"unknown": true} when the hub has no such action.
pub fn act(app: &str, name: &str, args: Vec<Value>, armed: Option<String>, env: HashMap<String, String>) -> Value {
    let Some(run) = action(app, name) else {
        return json!({"unknown": true});
    };
    let mut ctx = Ctx::new(armed, env);
    let mut args: Vec<Value> = args.into_iter().map(resolve).collect();
    if app == "inbox" {
        args = land::shorthand(name, args);
    }
    let error = run(&mut ctx, &args).err().map(|err| format!("{name}: {err}"));
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
