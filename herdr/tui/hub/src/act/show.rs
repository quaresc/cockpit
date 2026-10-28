//! The show view's one action that needs herdr: herdr-reviewr opened on a repo, zoomed over the asking app's pane.

use std::time::Duration;

use serde_json::Value;

use super::{Ctx, arg};
use crate::herdr;
use crate::proc;
use crate::value::{Res, cut, text};

const REVIEWR: &str = "persiyanov.reviewr";

/// Opens herdr-reviewr on the repo with the keyboard; over the asking app's pane when its request names it, since the
/// hub's own HERDR_PANE_ID is whichever app started it.
pub fn open_reviewr(ctx: &mut Ctx, args: &[Value]) -> Res {
    let repo = text(arg(args, 0)?);
    let mut argv = proc::strings(&["plugin", "pane", "open", "--plugin", REVIEWR, "--entrypoint", "pane", "--cwd", &repo, "--placement", "zoomed", "--focus"]);
    if let Some(pane) = ctx.env.get("HERDR_PANE_ID").filter(|p| !p.is_empty()) {
        argv.extend(["--target-pane".to_string(), pane.clone()]);
    }
    argv.insert(0, herdr::bin());
    let opened = proc::run(&argv, Duration::from_secs(10), None);
    if !opened.started {
        ctx.note(format!("reviewr did not open: {}", opened.stderr));
        return Ok(());
    }
    ctx.note(if opened.ok() { "opened reviewr; q there comes back".to_string() } else { format!("reviewr did not open: {}", cut(opened.stderr.trim(), 120)) });
    Ok(())
}
