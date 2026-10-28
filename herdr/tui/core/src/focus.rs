//! Requests for an app to bring something into view, one file per app in ~/.cache/cockpit/focus/, written by the hub
//! when the palette jumps across apps.

use std::path::PathBuf;
use std::time::SystemTime;

use serde_json::Value;

use crate::files;

fn path(app: &str) -> PathBuf {
    files::cache().join("focus").join(format!("{app}.json"))
}

pub fn request(app: &str, target: &Value) {
    let _ = files::write(&path(app), target);
}

/// The newest request since `seen`, taken once; `seen` starts at the app's start, so an old request is ignored.
pub fn take(app: &str, seen: &mut SystemTime) -> Option<Value> {
    let path = path(app);
    let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok()?;
    if modified <= *seen {
        return None;
    }
    *seen = modified;
    files::load(&path)
}
