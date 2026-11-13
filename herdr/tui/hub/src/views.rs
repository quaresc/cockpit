//! Views: what the Herdr app's cockpit mod draws, as JSON view models the app crates build. The `cockpit` binary
//! registers them before the hub starts (the hub does not depend on the app crates); the hub builds one on `view`, and
//! pushes it to `subscribe {views}` connections again whenever a store document or a folder it reads changes, or one of
//! its own actions ran.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use notify::{RecursiveMode, Watcher};
use serde_json::{Value, json};

use crate::outbox::Outbox;

/// Reads a store document by name (null before anyone wrote it).
pub type Doc<'a> = &'a dyn Fn(&str) -> Value;
/// A view's own actions: Some(Ok(note)) or Some(Err(error)) when it knows the action, None when it does not.
pub type Act = fn(&str, &[Value]) -> Option<Result<String, String>>;

/// One view an app crate provides.
pub struct Def {
    /// The view's name, the app it belongs to: `todo`, `inbox`, ...
    pub name: &'static str,
    /// Store documents whose change rebuilds it.
    pub docs: &'static [&'static str],
    /// Folders whose change rebuilds it, watched without their subfolders.
    pub dirs: fn() -> Vec<PathBuf>,
    /// The view model for these arguments (`subscribe` and `view` pass them through).
    pub build: fn(&Value, Doc) -> Value,
    /// Its own actions, asked with `act {app: name, action, args}` when the hub has no such action.
    pub act: Act,
}

fn defs() -> MutexGuard<'static, Vec<&'static Def>> {
    static DEFS: Mutex<Vec<&'static Def>> = Mutex::new(Vec::new());
    DEFS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Adds a view; the binary calls this for each app before `cockpit_hub::run`.
pub fn register(def: Def) {
    defs().push(Box::leak(Box::new(def)));
}

fn def(name: &str) -> Option<&'static Def> {
    defs().iter().copied().find(|d| d.name == name)
}

fn read_doc(name: &str) -> Value {
    if crate::store::is_doc(name) { crate::store::global().get(name) } else { Value::Null }
}

/// The view model, or {error} for an unknown view.
pub fn build(name: &str, args: &Value) -> Value {
    match def(name) {
        Some(def) => (def.build)(args, &read_doc),
        None => json!({"error": format!("no view {name:?}")}),
    }
}

/// Runs a view's own action and rebuilds that view for its subscribers; None when the view knows no such action.
pub fn act(app: &str, action: &str, args: &[Value]) -> Option<Value> {
    let def = def(app)?;
    let done = (def.act)(action, args)?;
    changed(Dirty::View(app.to_string()));
    Some(match done {
        Ok(note) => json!({"note": note, "pending": {}, "refreshing": {}, "error": null}),
        Err(error) => json!({"note": "", "pending": {}, "refreshing": {}, "error": format!("{action}: {error}")}),
    })
}

// ---------------------------------------------------------------------------------------------------------------------
// Subscribers

enum Dirty {
    Doc(String),
    Path(PathBuf),
    View(String),
}

struct Sub {
    tx: Outbox,
    /// It shows the cockpit notifications (`subscribe {notify: true}`): one copy of the mod per window asks.
    notify: bool,
    /// Each view it follows.
    views: Vec<Followed>,
}

/// A view a subscriber follows, with the top-level fields it asked for (all when `pick` is None) and what it was sent.
struct Followed {
    name: String,
    args: Value,
    pick: Option<Vec<String>>,
    last: Value,
}

/// The fields of the view value that `pick` names, or all of it.
fn picked(value: &Value, pick: &Option<Vec<String>>) -> Value {
    match (value, pick) {
        (Value::Object(map), Some(pick)) => Value::Object(map.iter().filter(|(k, _)| pick.contains(k)).map(|(k, v)| (k.clone(), v.clone())).collect()),
        _ => value.clone(),
    }
}

fn subs() -> MutexGuard<'static, Vec<Sub>> {
    static SUBS: Mutex<Vec<Sub>> = Mutex::new(Vec::new());
    SUBS.lock().unwrap_or_else(|e| e.into_inner())
}

fn dirty() -> &'static Mutex<Sender<Dirty>> {
    static TX: OnceLock<Mutex<Sender<Dirty>>> = OnceLock::new();
    TX.get_or_init(|| {
        let (tx, rx) = channel();
        std::thread::spawn(move || keep(rx));
        Mutex::new(tx)
    })
}

fn changed(what: Dirty) {
    let _ = dirty().lock().unwrap_or_else(|e| e.into_inner()).send(what);
}

/// A store document changed: the views reading it are built again.
pub fn doc_changed(name: &str) {
    if has_subscribers() {
        changed(Dirty::Doc(name.to_string()));
    }
}

/// A view's own data moved (a poller saw it): it is built again for its subscribers.
pub fn touch(name: &str) {
    if has_subscribers() {
        changed(Dirty::View(name.to_string()));
    }
}

pub fn has_subscribers() -> bool {
    !subs().is_empty()
}

/// The views asked for, as names or {name, args, pick}; unknown ones are left out. `pick` lists the top-level fields
/// the subscriber reads, such as a sidebar's badge counts, so a version moving only the others is not sent.
fn wanted(request: &Value) -> Vec<(String, Value, Option<Vec<String>>)> {
    request["views"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| match v {
            Value::String(name) => Some((name.clone(), Value::Null, None)),
            other => {
                let pick = other["pick"].as_array().map(|p| p.iter().filter_map(Value::as_str).map(str::to_string).collect());
                Some((other["name"].as_str()?.to_string(), other["args"].clone(), pick))
            }
        })
        .filter(|(name, _, _)| def(name).is_some())
        .collect()
}

/// Sends the views now, then each new value of them, as {view, args, value} lines; also {focus, target} lines when
/// the palette or a notification asks the view's app to bring something into view.
pub fn subscribe(request: &Value, tx: Outbox) {
    let views: Vec<Followed> = wanted(request)
        .into_iter()
        .map(|(name, args, pick)| {
            let value = picked(&build(&name, &args), &pick);
            let _ = tx.send(json!({"view": name, "args": args, "value": value}).to_string());
            Followed { name, args, pick, last: value }
        })
        .collect();
    let notify = request["notify"] == true;
    if !views.is_empty() || notify {
        dirty();
        subs().push(Sub { tx, views, notify });
    }
}

/// Hands a notification {title, body, action, buttons} to the first window that shows them, as a {notify} line; false
/// when none follows the hub.
pub fn notify(notification: &Value) -> bool {
    let line = json!({"notify": notification}).to_string();
    let mut subs = subs();
    subs.retain(|s| s.tx.send(String::new()).is_ok());
    subs.iter().find(|s| s.notify).is_some_and(|s| s.tx.send(line).is_ok())
}

/// Asks the subscribers of the app's view to bring this target into view.
pub fn focus(app: &str, target: &Value) {
    let app = if app == "to-do" { "todo" } else { app };
    let line = json!({"focus": app, "target": target}).to_string();
    subs().retain(|s| !s.views.iter().any(|f| f.name == app) || s.tx.send(line.clone()).is_ok());
}

/// The folders every registered view reads.
fn watched() -> HashMap<PathBuf, Vec<&'static str>> {
    let mut dirs: HashMap<PathBuf, Vec<&'static str>> = HashMap::new();
    for def in defs().iter() {
        for dir in (def.dirs)() {
            dirs.entry(dir).or_default().push(def.name);
        }
    }
    dirs
}

/// Rebuilds the views something they read touched, a burst of changes at once, and sends the ones whose value moved.
fn keep(rx: Receiver<Dirty>) {
    let dirs = watched();
    let (tx, files) = channel();
    let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if let Ok(event) = event {
            for path in event.paths {
                let _ = tx.send(path);
            }
        }
    });
    // Keyed by the real path, as the events name it (the cache can sit behind a symlink).
    let dirs: HashMap<PathBuf, Vec<&'static str>> = dirs
        .into_iter()
        .map(|(dir, names)| {
            let _ = std::fs::create_dir_all(&dir);
            (std::fs::canonicalize(&dir).unwrap_or(dir), names)
        })
        .collect();
    let _watcher = watcher.ok().map(|mut w| {
        for dir in dirs.keys() {
            let _ = w.watch(dir, RecursiveMode::NonRecursive);
        }
        w
    });
    loop {
        let mut names: HashSet<&'static str> = HashSet::new();
        let take = |what: Dirty, names: &mut HashSet<&'static str>| match what {
            Dirty::Doc(doc) => names.extend(defs().iter().filter(|d| d.docs.contains(&doc.as_str())).map(|d| d.name)),
            Dirty::View(view) => names.extend(def(&view).map(|d| d.name)),
            Dirty::Path(path) => {
                let parent = path.parent().map(PathBuf::from).unwrap_or_default();
                names.extend(dirs.get(&parent).into_iter().flatten().copied());
            }
        };
        // Wakes for the store's word or a file's, whichever comes first.
        let first = loop {
            match rx.recv_timeout(Duration::from_millis(250)) {
                Ok(what) => break Some(what),
                Err(RecvTimeoutError::Disconnected) => return,
                Err(RecvTimeoutError::Timeout) => {
                    if let Ok(path) = files.try_recv() {
                        break Some(Dirty::Path(path));
                    }
                }
            }
        };
        if let Some(first) = first {
            take(first, &mut names);
        }
        // A burst (a file written through its .tmp, a patch then a put) settles first.
        std::thread::sleep(Duration::from_millis(120));
        while let Ok(what) = rx.try_recv() {
            take(what, &mut names);
        }
        while let Ok(path) = files.try_recv() {
            take(Dirty::Path(path), &mut names);
        }
        if names.is_empty() {
            continue;
        }
        let keys: Vec<(String, Value)> =
            subs().iter().flat_map(|s| s.views.iter().filter(|f| names.contains(f.name.as_str())).map(|f| (f.name.clone(), f.args.clone()))).collect();
        let mut built: Vec<((String, Value), Value)> = Vec::new();
        for key in keys {
            if !built.iter().any(|(k, _)| *k == key) {
                let value = build(&key.0, &key.1);
                built.push((key, value));
            }
        }
        subs().retain_mut(|sub| {
            for followed in sub.views.iter_mut() {
                let Some((_, value)) = built.iter().find(|((n, a), _)| *n == followed.name && *a == followed.args) else { continue };
                let value = picked(value, &followed.pick);
                if value != followed.last {
                    followed.last = value.clone();
                    if sub.tx.send(json!({"view": followed.name, "args": followed.args, "value": value}).to_string()).is_err() {
                        return false;
                    }
                }
            }
            true
        });
    }
}

/// The args of every live subscription to a view: which of its copies are drawn now (a show view's `{name}`).
pub fn shown(name: &str) -> Vec<Value> {
    let mut subs = subs();
    subs.retain(|s| s.tx.send(String::new()).is_ok());
    subs.iter().flat_map(|s| s.views.iter().filter(|f| f.name == name).map(|f| f.args.clone())).collect()
}

/// Forgets the subscribers whose connection went (their channel closed).
pub fn prune() {
    subs().retain(|s| s.tx.send(String::new()).is_ok());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outbox::channel;

    fn counter(args: &Value, doc: Doc) -> Value {
        json!({"args": args, "inbox": doc("nope")})
    }

    fn no_act(action: &str, _: &[Value]) -> Option<Result<String, String>> {
        (action == "ping").then(|| Ok("pong".to_string()))
    }

    #[test]
    fn a_registered_view_builds_acts_and_pushes() {
        register(Def { name: "test-view", docs: &[], dirs: Vec::new, build: counter, act: no_act });
        assert_eq!(build("test-view", &json!({"a": 1}))["args"]["a"], 1);
        assert!(build("nope", &Value::Null)["error"].is_string());
        assert_eq!(act("test-view", "ping", &[]).unwrap()["note"], "pong");
        assert!(act("test-view", "other", &[]).is_none());
        let (tx, rx) = channel();
        subscribe(&json!({"views": ["test-view", {"name": "test-view", "args": {"b": 2}}, "unknown"], "notify": true}), tx);
        // Empty lines are the keep-alives other tests' notifications send to every subscriber.
        let next = || -> Value { serde_json::from_str(&rx.iter().find(|l| !l.is_empty()).unwrap()).unwrap() };
        let first = next();
        let second = next();
        assert_eq!((first["view"].as_str(), second["value"]["args"]["b"].as_i64()), (Some("test-view"), Some(2)));
        focus("test-view", &json!({"key": "x"}));
        let focus = next();
        assert_eq!(focus["target"]["key"], "x");
        assert!(notify(&json!({"title": "hi"})));
        let lines: Vec<String> = rx.try_iter().filter(|l| !l.is_empty()).collect();
        assert_eq!(serde_json::from_str::<Value>(&lines[0]).unwrap()["notify"]["title"], "hi");
    }

    static BUILDS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    /// A view whose `count` moves every second build and whose `noise` moves on each.
    fn noisy(_: &Value, _: Doc) -> Value {
        let n = BUILDS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        json!({"count": n / 2, "noise": n})
    }

    #[test]
    fn a_picked_subscription_gets_its_fields_and_only_their_changes() {
        register(Def { name: "noisy-view", docs: &[], dirs: Vec::new, build: noisy, act: no_act });
        let (tx, rx) = channel();
        subscribe(&json!({"views": [{"name": "noisy-view", "pick": ["count"]}]}), tx);
        let next = || -> Option<Value> {
            let line = std::iter::from_fn(|| rx.recv_timeout(Duration::from_secs(2)).ok()).find(|l| !l.is_empty())?;
            Some(serde_json::from_str::<Value>(&line).unwrap()["value"].clone())
        };
        assert_eq!(next(), Some(json!({"count": 0})), "the picked fields alone");
        touch("noisy-view");
        std::thread::sleep(Duration::from_millis(500));
        touch("noisy-view");
        assert_eq!(next(), Some(json!({"count": 1})), "a build that moved only `noise` sends nothing");
    }

    #[test]
    fn shown_lists_the_args_of_live_subscriptions_only() {
        register(Def { name: "shown-view", docs: &[], dirs: Vec::new, build: counter, act: no_act });
        let (tx, rx) = channel();
        subscribe(&json!({"views": [{"name": "shown-view", "args": {"name": "quota"}}]}), tx);
        assert_eq!(shown("shown-view"), vec![json!({"name": "quota"})]);
        drop(rx);
        assert!(shown("shown-view").is_empty());
    }
}
