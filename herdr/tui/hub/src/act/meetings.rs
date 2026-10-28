//! meetings.py's actions: a meeting's brief written in the background by meeting-brief.sh, and its links opened.

use std::path::PathBuf;

use cockpit_core::files;
use serde_json::Value;

use super::{Ctx, arg, inbox};
use crate::config;
use crate::proc;
use crate::py::{Res, s};

fn brief_path(id: &str, suffix: &str) -> PathBuf {
    let name: String = id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect();
    files::cache().join("meeting-briefs").join(format!("{name}{suffix}"))
}

/// Writes (or rewrites) a meeting's brief in the background; its .pending file says so until it lands.
pub fn prepare(ctx: &mut Ctx, args: &[Value]) -> Res {
    let meeting = arg(args, 0)?;
    let id = s(meeting, "id")?;
    if brief_path(&id, ".pending").exists() {
        ctx.note("already preparing that brief");
        return Ok(());
    }
    let script = config::script("meeting-brief.sh");
    if !script.exists() {
        ctx.note(format!("cannot write briefs: {} is missing", script.display()));
        return Ok(());
    }
    proc::spawn(&[String::from("sh"), script.to_string_lossy().into_owned(), id], None, None, |_| {}).map_err(|e| e.to_string())?;
    ctx.note(format!("preparing the brief of {}, about a minute", s(meeting, "title")?));
    Ok(())
}

/// The brief's file side of opening it: a meeting with no brief yet and none on the way gets one prepared; the app
/// folds and unfolds the brief itself.
pub fn toggle_brief(ctx: &mut Ctx, args: &[Value]) -> Res {
    let id = s(arg(args, 0)?, "id")?;
    if brief_path(&id, ".md").exists() || brief_path(&id, ".pending").exists() {
        return Ok(());
    }
    prepare(ctx, args)
}

pub fn open_url(ctx: &mut Ctx, args: &[Value]) -> Res {
    inbox::open_url(ctx, args)
}

/// The meeting's video call.
pub fn join(ctx: &mut Ctx, args: &[Value]) -> Res {
    let meeting = arg(args, 0)?;
    inbox::open_url(ctx, &[Value::from(s(meeting, "join")?)])
}
