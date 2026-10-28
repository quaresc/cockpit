//! An action run in the background: the hub runs it, or bridge.py runs the text apps' own Python version while the hub
//! has none by that name. A call names a module (the app), a function (the action) and its JSON arguments; its reply (the
//! note the action left, the requests it filed) comes back on a channel, so a slow herdr or git call never holds up the keys.

use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};

use serde_json::{Value, json};

use crate::files;

#[derive(Debug, Default)]
pub struct Reply {
    pub tag: String,
    pub note: String,
    pub pending: Value,
    pub refreshing: Value,
    pub error: Option<String>,
}

pub struct Bridge {
    tx: Sender<Reply>,
    pub replies: Receiver<Reply>,
}

impl Default for Bridge {
    fn default() -> Self {
        let (tx, replies) = channel();
        Self { tx, replies }
    }
}

impl Bridge {
    /// Runs `module.function(state, *args)` in the background; `armed` is the key a twice-pressed action checks, given
    /// when a dialog already asked. `tag` comes back with the reply, to tell calls apart.
    pub fn call(&self, tag: &str, module: &str, function: &str, args: Vec<Value>, armed: Option<String>) {
        let tx = self.tx.clone();
        let request = json!({"module": module, "function": function, "args": args, "armed": armed});
        let tag = tag.to_string();
        std::thread::spawn(move || {
            let _ = tx.send(through_hub(&tag, &request).unwrap_or_else(|| run(&tag, &request)));
        });
    }
}

/// The hub's answer, or None when it has no such action (or is not there), for bridge.py to run it.
fn through_hub(tag: &str, request: &Value) -> Option<Reply> {
    let ask = json!({"op": "act", "app": request["module"], "action": request["function"], "args": request["args"], "armed": request["armed"]});
    let answer = crate::hub::request(&ask, std::time::Duration::from_secs(120)).ok()?;
    if answer["unknown"].as_bool() == Some(true) {
        return None;
    }
    Some(Reply {
        tag: tag.into(),
        note: answer["note"].as_str().unwrap_or_default().to_string(),
        pending: answer["pending"].clone(),
        refreshing: answer["refreshing"].clone(),
        error: answer["error"].as_str().map(str::to_string),
    })
}

fn run(tag: &str, request: &Value) -> Reply {
    let dir = files::herdr_dir();
    let child = Command::new("python3")
        .arg(dir.join("bridge.py"))
        .current_dir(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(err) => return Reply { tag: tag.into(), error: Some(format!("python3 did not start: {err}")), ..Default::default() },
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(request.to_string().as_bytes());
    }
    let out = match child.wait_with_output() {
        Ok(out) => out,
        Err(err) => return Reply { tag: tag.into(), error: Some(err.to_string()), ..Default::default() },
    };
    let Ok(answer) = serde_json::from_slice::<Value>(&out.stdout) else {
        let error = String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("bridge.py failed").to_string();
        return Reply { tag: tag.into(), error: Some(error), ..Default::default() };
    };
    Reply {
        tag: tag.into(),
        note: answer["note"].as_str().unwrap_or_default().to_string(),
        pending: answer["pending"].clone(),
        refreshing: answer["refreshing"].clone(),
        error: answer["error"].as_str().map(str::to_string),
    }
}
