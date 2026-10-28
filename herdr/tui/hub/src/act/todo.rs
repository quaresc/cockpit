//! todo.py's link action: an item's ticket key or URL opened; the item edits stay in the app.

use serde_json::Value;

use super::{Ctx, arg};
use crate::config;
use crate::py::{Res, text};

pub fn open_link(ctx: &mut Ctx, args: &[Value]) -> Res {
    let token = text(arg(args, 0)?);
    let url = if token.to_lowercase().starts_with("http") { Some(token.clone()) } else { config::ticket_url(&token, &ctx.cfg, None) };
    let Some(url) = url else {
        ctx.note(format!("no URL known for {}: set tickets.urlTemplate in the cockpit config", token.to_uppercase()));
        return Ok(());
    };
    config::open_url(&url);
    ctx.note(format!("opened {url}"));
    Ok(())
}
