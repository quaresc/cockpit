//! What the headers' source chips do: sign a source in again in a tab of the actions space, or read it again at once.

use cockpit_core::{files, frontend};
use serde_json::Value;

use super::{Ctx, arg, inbox, space};
use crate::feed::fresh;
use crate::value::{Res, join, text};

/// The source's record in the document it fills, if the feed wrote one.
fn record(source: &str) -> Value {
    crate::store::global().get(fresh::doc_of(source))["hub"]["fresh"][source].clone()
}

fn label(source: &str) -> String {
    record(source)["label"].as_str().filter(|l| !l.is_empty()).map(str::to_string).unwrap_or_else(|| source.to_string())
}

/// `[source]`: its login (`gh auth login`, `cockpit connector login <server>` or `cockpit setup`) typed into a new tab of
/// the actions space, else a space of its own; pressed again while that tab or space is open, it comes back into view.
pub fn sign_in(ctx: &mut Ctx, args: &[Value]) -> Res {
    let source = text(arg(args, 0)?);
    let raw = files::load(&files::config_dir().join("config.json")).unwrap_or(Value::Null);
    // The record names the server that failed; `login` takes it only when the config's servers hold it.
    let via = record(&source)["via"].as_str().map(str::to_string);
    let login = fresh::login(&source, &raw, &ctx.cfg.0, via.as_deref())?;
    let place = in_tab(ctx, &format!("sign in · {}", label(&source)), &join(&login), &fresh::shown(&login))?;
    ctx.note(match place.strip_suffix(", already open") {
        Some(open) => format!("{}: the sign-in is already open {open}: finish it there", label(&source)),
        None => format!("{}: signing in again {place}", label(&source)),
    });
    Ok(())
}

/// A command typed into a new tab of the actions space named `name`, else a space of that name, in the home folder;
/// that tab or space comes back into view while it is open. `shown` is the command as the person would type it; the
/// answer says where it runs.
pub(super) fn in_tab(ctx: &mut Ctx, name: &str, line: &str, shown: &str) -> Res<&'static str> {
    if let Some(pane) = space::pane_of(name)
        && space::focus(&pane)
    {
        return Ok("in its tab, already open");
    }
    if frontend::focus_space_named(name) {
        return Ok("in its space, already open");
    }
    let home = files::home().to_string_lossy().into_owned();
    let (pane, place) = match space::open_tab(&ctx.cfg, &home, name, Some(name)) {
        Some(pane) => (Some(pane), "in the actions space"),
        None => (frontend::new_space(&home, name), "in a space of its own"),
    };
    let Some(pane) = pane else {
        return Err(format!("no tab to run it in: run {shown} in a terminal"));
    };
    frontend::run_in(&pane, line);
    Ok(place)
}

/// `[source]`: every source read again at once, as the space's refresh does; a rate-limited one waits for its reset.
pub fn refresh(ctx: &mut Ctx, args: &[Value]) -> Res {
    let source = args.first().map(text).unwrap_or_default();
    // A usage window is read from the agents' own files, again at once; nothing is fetched.
    if source.starts_with("usage-") {
        crate::usage::read_now();
        ctx.note("Usage read again: an agent reports its limits while one of its sessions runs");
        return Ok(());
    }
    inbox::request_refresh(ctx, args)?;
    let record = record(&source);
    let limited = record["error"]["kind"] == "rate";
    let next = record["next"].as_u64().map(cockpit_core::header::clock);
    ctx.note(match (limited, next) {
        (true, Some(next)) => format!("{} is read again at {next}, when its limit resets", label(&source)),
        _ if source.is_empty() => "refresh requested, it lands within a minute".to_string(),
        _ => format!("reading {} again", label(&source)),
    });
    Ok(())
}
