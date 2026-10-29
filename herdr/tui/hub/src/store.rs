//! The documents the apps, the mod and the hub share: inbox, ci, meetings, brief, monitor and routines. The hub is their
//! only writer. Each lives in ~/.cache/cockpit/store.redb with a week of its past versions, and is mirrored to
//! <name>.json, written whole through a rename, for whatever reads files. Subscribers get each new version pushed, and
//! leases let one Claude session at a time do a job for all of them, such as fetching the tickets.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use redb::{Database, ReadableDatabase, TableDefinition};
use serde_json::{Map, Value, json};

/// The documents the store keeps; any other name is refused, so no request names a file of its own.
pub const DOCS: [&str; 6] = ["inbox", "ci", "meetings", "brief", "monitor", "routines"];

const CURRENT: TableDefinition<&str, &[u8]> = TableDefinition::new("current");
const PAST: TableDefinition<(&str, u64), &[u8]> = TableDefinition::new("past");
/// What happened, by time and order within it (see events).
const EVENTS: TableDefinition<(u64, u32), &[u8]> = TableDefinition::new("events");
const KEEP_PAST_MS: u64 = 7 * 86_400_000;
/// Fields that change on every write without saying anything new: a version differing only by them is not kept as past.
const VOLATILE: [&str; 3] = ["at", "modAt", "hub"];
/// How long another hub may hold the database, as when a newer binary takes over, before this one runs on the mirrors.
const OPEN_WAIT: Duration = Duration::from_secs(10);

pub struct Store {
    dir: PathBuf,
    db: Option<Database>,
    docs: HashMap<String, Value>,
    subscribers: Vec<(Vec<String>, Sender<String>)>,
    leases: HashMap<String, (String, Instant)>,
    /// Handed over to a newer hub: this one writes nothing more.
    closed: bool,
}

static STORE: OnceLock<Mutex<Store>> = OnceLock::new();

/// The hub's store, on ~/.cache/cockpit; opened by the first call.
pub fn global() -> MutexGuard<'static, Store> {
    STORE.get_or_init(|| Mutex::new(Store::open(&cockpit_core::files::cache(), OPEN_WAIT))).lock().unwrap_or_else(|e| e.into_inner())
}

pub fn is_doc(name: &str) -> bool {
    DOCS.contains(&name)
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn create(path: &Path) -> Result<Database, redb::DatabaseError> {
    Database::builder().set_cache_size(4 << 20).create(path)
}

fn pack(value: &Value) -> Vec<u8> {
    lz4_flex::compress_prepend_size(value.to_string().as_bytes())
}

fn unpack(bytes: &[u8]) -> Option<Value> {
    serde_json::from_slice(&lz4_flex::decompress_size_prepended(bytes).ok()?).ok()
}

/// The document without its volatile fields, to tell a new version from a refreshed one.
fn content(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(map.iter().filter(|(k, _)| !VOLATILE.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect()),
        other => other.clone(),
    }
}

impl Store {
    /// The store in this folder: the database's documents, and for one it does not hold yet, its mirror. When another
    /// process keeps the database past `wait`, the store runs on the mirrors alone.
    pub fn open(dir: &Path, wait: Duration) -> Store {
        let _ = std::fs::create_dir_all(dir);
        remove_locks(dir);
        let path = dir.join("store.redb");
        let start = Instant::now();
        let db = loop {
            match create(&path) {
                Ok(db) => break Some(db),
                Err(redb::DatabaseError::DatabaseAlreadyOpen) if start.elapsed() < wait => std::thread::sleep(Duration::from_millis(100)),
                Err(_) => break None,
            }
        };
        let mut store = Store { dir: dir.to_path_buf(), db, docs: HashMap::new(), subscribers: Vec::new(), leases: HashMap::new(), closed: false };
        for name in DOCS {
            let kept = store.read_current(name);
            if let Some(value) = kept.or_else(|| cockpit_core::files::load(&dir.join(format!("{name}.json")))) {
                store.docs.insert(name.to_string(), value);
            }
        }
        store
    }

    fn read_current(&self, name: &str) -> Option<Value> {
        let read = self.db.as_ref()?.begin_read().ok()?;
        let table = read.open_table(CURRENT).ok()?;
        let bytes = table.get(name).ok()??;
        unpack(bytes.value())
    }

    /// The document, or null before anyone wrote it.
    pub fn get(&self, name: &str) -> Value {
        self.docs.get(name).cloned().unwrap_or(Value::Null)
    }

    /// Writes the document `merge` makes of the current one, and returns both; nothing is written when it is the same.
    pub fn update(&mut self, name: &str, merge: impl FnOnce(&Value) -> Value) -> (Value, Value) {
        let before = self.get(name);
        if self.closed {
            return (before.clone(), before);
        }
        let after = merge(&before);
        if after != before {
            self.persist(name, &before, &after);
            let _ = mirror(&self.dir.join(format!("{name}.json")), &after);
            self.docs.insert(name.to_string(), after.clone());
            self.push(name, &after);
        }
        (before, after)
    }

    /// Into the database, opened again first when another hub held it at the start.
    fn persist(&mut self, name: &str, before: &Value, after: &Value) {
        if self.db.is_none() {
            self.db = create(&self.dir.join("store.redb")).ok();
        }
        let happened = crate::events::of(name, before, after, &self.get("inbox"));
        let Some(db) = &self.db else { return };
        let now = now_ms();
        let write = || -> Result<(), redb::Error> {
            let txn = db.begin_write()?;
            {
                txn.open_table(CURRENT)?.insert(name, pack(after).as_slice())?;
                if content(before) != content(after) {
                    let mut past = txn.open_table(PAST)?;
                    past.insert((name, now), pack(after).as_slice())?;
                    past.retain_in((name, 0)..(name, now.saturating_sub(KEEP_PAST_MS)), |_, _| false)?;
                    let mut table = txn.open_table(EVENTS)?;
                    for (n, event) in happened.iter().enumerate() {
                        let mut event = event.clone();
                        event["at"] = json!(now);
                        event["doc"] = json!(name);
                        table.insert((now, n as u32), pack(&event).as_slice())?;
                    }
                    table.retain_in((0, 0)..(now.saturating_sub(KEEP_PAST_MS), 0), |_, _| false)?;
                }
            }
            txn.commit()?;
            Ok(())
        };
        let _ = write();
    }

    /// Lets the hub taking over open the database; this store writes nothing more.
    pub fn close(&mut self) {
        self.db = None;
        self.closed = true;
    }

    /// What happened since `since_ms`, newest first, at most `limit` of them: {at, doc, app, key, title} each.
    pub fn events(&self, since_ms: u64, limit: usize) -> Vec<Value> {
        let read = || -> Option<Vec<Value>> {
            let txn = self.db.as_ref()?.begin_read().ok()?;
            let table = txn.open_table(EVENTS).ok()?;
            let range = table.range((since_ms, 0)..).ok()?;
            Some(range.rev().flatten().take(limit).filter_map(|(_, bytes)| unpack(bytes.value())).collect())
        };
        read().unwrap_or_default()
    }

    /// The document's versions since `since_ms` (epoch milliseconds), oldest first, each as {at, value}.
    pub fn past(&self, name: &str, since_ms: u64) -> Vec<Value> {
        let read = || -> Option<Vec<Value>> {
            let txn = self.db.as_ref()?.begin_read().ok()?;
            let table = txn.open_table(PAST).ok()?;
            let range = table.range((name, since_ms)..=(name, u64::MAX)).ok()?;
            Some(range.flatten().filter_map(|(key, bytes)| Some(json!({"at": key.value().1, "value": unpack(bytes.value())?}))).collect())
        };
        read().unwrap_or_default()
    }

    /// Sends the documents now, then each new version of them, as one JSON line {doc, value} apiece.
    pub fn subscribe(&mut self, names: Vec<String>, tx: Sender<String>) {
        for name in &names {
            let _ = tx.send(json!({"doc": name, "value": self.get(name)}).to_string());
        }
        self.subscribers.push((names, tx));
    }

    pub fn has_subscribers(&self) -> bool {
        !self.subscribers.is_empty()
    }

    fn push(&mut self, name: &str, value: &Value) {
        let line = json!({"doc": name, "value": value}).to_string();
        self.subscribers.retain(|(names, tx)| !names.iter().any(|n| n == name) || tx.send(line.clone()).is_ok());
    }

    /// Takes the lease `name` for `holder` until `ttl` from now, or extends the holder's; false while another holds it.
    /// A renewal never takes a lease back: one that arrives after its release or after another took over is refused.
    pub fn lease(&mut self, name: &str, holder: &str, ttl: Duration, renew: bool) -> bool {
        let now = Instant::now();
        let granted = match self.leases.get(name) {
            Some((other, until)) if other != holder && *until > now => false,
            Some((own, _)) if own == holder => true,
            _ => !renew,
        };
        if granted {
            self.leases.insert(name.to_string(), (holder.to_string(), now + ttl));
        }
        granted
    }

    pub fn release(&mut self, name: &str, holder: &str) {
        if self.leases.get(name).is_some_and(|(own, _)| own == holder) {
            self.leases.remove(name);
        }
    }
}

/// Written beside it, then moved in place, so no reader sees half of it.
fn mirror(path: &Path, value: &Value) -> std::io::Result<()> {
    let nanos = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
    let tmp = path.with_file_name(format!("{}.hub{}-{nanos}.tmp", path.file_name().and_then(|n| n.to_str()).unwrap_or("data"), std::process::id()));
    std::fs::write(&tmp, value.to_string())?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// The lock folders the mod and the hub took before the store, and the stale ones they renamed away and left.
fn remove_locks(dir: &Path) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".lock") || name.contains(".lock.stale-") {
            let path = entry.path();
            let _ = std::fs::remove_dir(&path).or_else(|_| std::fs::remove_file(&path));
        }
    }
}

/// Sets the fields given, keeping the others.
pub fn patch(doc: &Value, fields: &Map<String, Value>) -> Value {
    let mut out = if doc.is_object() { doc.clone() } else { json!({}) };
    for (key, value) in fields {
        out[key.as_str()] = value.clone();
    }
    out
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::channel;

    use super::*;

    fn open(dir: &Path) -> Store {
        Store::open(dir, Duration::ZERO)
    }

    #[test]
    fn a_write_lands_in_the_database_and_its_mirror_and_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        assert_eq!(store.get("ci"), Value::Null);
        let (before, after) = store.update("ci", |_| json!({"at": 1, "runs": []}));
        assert_eq!((before, &after), (Value::Null, &json!({"at": 1, "runs": []})));
        assert_eq!(cockpit_core::files::load(&dir.path().join("ci.json")), Some(after.clone()));
        drop(store);
        std::fs::remove_file(dir.path().join("ci.json")).unwrap();
        assert_eq!(open(dir.path()).get("ci"), after);
    }

    #[test]
    fn a_mirror_the_database_lacks_is_taken_in() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("inbox.json"), r#"{"version": 1, "tickets": [{"id": "ENG-1"}]}"#).unwrap();
        assert_eq!(open(dir.path()).get("inbox")["tickets"][0]["id"], "ENG-1");
    }

    #[test]
    fn a_held_database_leaves_the_store_on_its_mirrors() {
        let dir = tempfile::tempdir().unwrap();
        let _first = open(dir.path());
        std::fs::write(dir.path().join("brief.json"), r#"{"lines": []}"#).unwrap();
        let mut second = open(dir.path());
        assert_eq!(second.get("brief"), json!({"lines": []}));
        second.update("brief", |_| json!({"lines": ["x"]}));
        assert_eq!(cockpit_core::files::load(&dir.path().join("brief.json")), Some(json!({"lines": ["x"]})));
    }

    #[test]
    fn a_store_handed_over_lets_the_next_open_the_database_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut first = open(dir.path());
        first.update("ci", |_| json!({"runs": [1]}));
        let mut next = open(dir.path());
        first.close();
        assert_eq!(first.update("ci", |_| json!({"runs": [2]})).1, json!({"runs": [1]}));
        next.update("ci", |_| json!({"runs": [3]}));
        drop(next);
        assert_eq!(open(dir.path()).get("ci"), json!({"runs": [3]}), "the next store reached the database once free");
    }

    #[test]
    fn only_a_version_that_says_something_new_is_kept_as_past() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        store.update("inbox", |_| json!({"at": 1, "mine": []}));
        store.update("inbox", |_| json!({"at": 2, "mine": []}));
        std::thread::sleep(Duration::from_millis(2));
        store.update("inbox", |_| json!({"at": 3, "mine": [{"url": "u"}]}));
        let past = store.past("inbox", 0);
        assert_eq!(past.iter().map(|v| v["value"]["at"].clone()).collect::<Vec<_>>(), [json!(1), json!(3)]);
        assert!(store.past("inbox", now_ms() + 1).is_empty());
        assert!(store.past("ci", 0).is_empty());
    }

    #[test]
    fn what_happened_is_kept_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        store.update("inbox", |_| json!({"version": 1, "tickets": [{"id": "ENG-1", "status": "Todo"}]}));
        store.update("inbox", |_| json!({"version": 1, "tickets": [{"id": "ENG-1", "status": "Done"}]}));
        std::thread::sleep(Duration::from_millis(2));
        store.update("inbox", |_| json!({"version": 1, "tickets": [{"id": "ENG-1", "status": "Done"}, {"id": "ENG-2", "status": "Todo", "title": "b"}]}));
        let events = store.events(0, 10);
        let titles: Vec<&str> = events.iter().map(|e| e["title"].as_str().unwrap()).collect();
        assert_eq!(titles, ["🎫 Assigned: ENG-2 b", "🎫 ENG-1: Todo → Done"]);
        assert_eq!((&events[0]["doc"], &events[0]["key"]), (&json!("inbox"), &json!("ENG-2")));
        assert!(events[0]["at"].as_u64().unwrap() > events[1]["at"].as_u64().unwrap());
        assert_eq!(store.events(0, 1).len(), 1);
        assert!(store.events(now_ms() + 1, 10).is_empty());
    }

    #[test]
    fn subscribers_get_the_documents_then_each_change_of_theirs() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        store.update("ci", |_| json!({"runs": []}));
        let (tx, rx) = channel();
        store.subscribe(vec!["ci".into(), "brief".into()], tx);
        store.update("inbox", |_| json!({"mine": []}));
        store.update("ci", |_| json!({"runs": [1]}));
        store.update("ci", |_| json!({"runs": [1]}));
        let lines: Vec<Value> = rx.try_iter().map(|l| serde_json::from_str(&l).unwrap()).collect();
        assert_eq!(lines, [json!({"doc": "ci", "value": {"runs": []}}), json!({"doc": "brief", "value": null}), json!({"doc": "ci", "value": {"runs": [1]}})]);
        drop(rx);
        store.update("ci", |_| json!({"runs": [2]}));
        assert!(!store.has_subscribers(), "a subscriber gone is dropped at the next change");
    }

    #[test]
    fn a_lease_is_one_holders_until_it_lapses_and_a_late_renewal_takes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        let ttl = Duration::from_secs(60);
        assert!(store.lease("inbox", "a", ttl, false));
        assert!(!store.lease("inbox", "b", ttl, false));
        assert!(store.lease("inbox", "a", ttl, true));
        assert!(store.lease("ci", "b", ttl, false), "leases are by name");
        store.release("inbox", "b");
        assert!(!store.lease("inbox", "b", ttl, false), "only the holder releases");
        store.release("inbox", "a");
        assert!(!store.lease("inbox", "a", ttl, true), "a renewal after the release");
        assert!(store.lease("inbox", "b", Duration::ZERO, false));
        assert!(store.lease("inbox", "c", ttl, false), "a lapsed lease goes to the next");
        assert!(!store.lease("inbox", "b", ttl, true));
    }

    #[test]
    fn the_old_lock_folders_go_at_open() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("inbox.lock")).unwrap();
        std::fs::write(dir.path().join("ci.lock.stale-1e4lzu7s775"), "").unwrap();
        std::fs::write(dir.path().join("ci.json"), "{}").unwrap();
        open(dir.path());
        let left: Vec<String> = std::fs::read_dir(dir.path()).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert!(left.iter().all(|n| !n.contains(".lock")), "{left:?}");
        assert!(left.contains(&"ci.json".to_string()));
    }

    #[test]
    fn a_patch_sets_its_fields_and_keeps_the_rest() {
        let fields = json!({"dismissed": true}).as_object().unwrap().clone();
        assert_eq!(patch(&json!({"lines": ["a"]}), &fields), json!({"lines": ["a"], "dismissed": true}));
        assert_eq!(patch(&Value::Null, &fields), json!({"dismissed": true}));
    }
}
