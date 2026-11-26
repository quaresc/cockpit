//! The one catalog of models: what each agent offers wherever a person picks one (Start work, the chat's chips, a
//! routine). An agent with a listing command (`Agent::model_listing`) is asked in the background and its answer kept in
//! `~/.cache/cockpit/models.json`; one with a catalog request over its stream (`Listing::Probe`) is asked that way and
//! kept by its version, asked again only once that changed; any other offers the models its table row names.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Read as _, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;
use std::time::{Duration, SystemTime};

use serde_json::{Value, json};

use crate::agent::{self, Agent, Listing, Offer};
use crate::files;

/// How long a list is kept before the agent is asked again, in ms; for a probe, how long before its version is checked.
pub(crate) const KEPT: u64 = 24 * 3_600_000;
/// How long after a listing failed the agent is asked again, in ms.
const RETRY: u64 = 30 * 60_000;
/// The most an agent's listing, or its catalog probe, may take.
const LIMIT: Duration = Duration::from_secs(30);
/// The most a version takes to print.
const VERSION_LIMIT: Duration = Duration::from_secs(20);
/// The most of a listing read: Codex's catalog comes close to a megabyte.
const LISTING_CAP: usize = 8 << 20;
/// The request id a probe's catalog request goes under, which its answer names.
const PROBE_ID: &str = "cockpit-catalog";

/// Whether this process asks agents for their models.
static ASKS: AtomicBool = AtomicBool::new(false);
/// The agents being asked now, each with whom to tell once it answered.
static ASKING: Askers = Askers::new();
static WRITING: Mutex<()> = Mutex::new(());
type Read = HashMap<PathBuf, (Option<SystemTime>, u64, Value)>;
/// Each file as last read, with its modification time and size: read again only once it changed.
static READ: Mutex<Option<Read>> = Mutex::new(None);
type Tried = HashMap<(PathBuf, String), (u64, bool)>;
/// When this process last asked each agent (its models, or `help:<id>` its help), and whether that failed or its answer
/// could not be kept, by file.
static TRIED: Mutex<Option<Tried>> = Mutex::new(None);

/// What an agent offers: its models, and whether its own listing is being read now.
#[derive(Clone, Debug, PartialEq)]
pub struct Listed {
    pub offers: Vec<Offer>,
    pub loading: bool,
}

/// Who to tell once each agent being asked answered, by id.
type Told = HashMap<String, Vec<fn()>>;

/// The agents a process is asking something of in the background, by id, each with whom to tell once it answered.
pub(crate) struct Askers(Mutex<Option<Told>>);

impl Askers {
    pub(crate) const fn new() -> Askers {
        Askers(Mutex::new(None))
    }

    /// Joins the ask of `id`, `ready` told once it answers, beginning it when `start` and nobody asks yet: (whether it
    /// is being asked, whether this call began it and runs it).
    pub(crate) fn join(&self, id: &str, ready: fn(), start: bool) -> (bool, bool) {
        let mut asking = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let asking = asking.get_or_insert_with(HashMap::new);
        let begun = start && !asking.contains_key(id);
        if begun {
            asking.insert(id.to_string(), Vec::new());
        }
        let loading = asking.get_mut(id).is_some_and(|told| {
            if !told.iter().any(|known| std::ptr::fn_addr_eq(*known, ready)) {
                told.push(ready);
            }
            true
        });
        (loading, begun)
    }

    /// Ends the ask of `id`: whom to tell.
    pub(crate) fn finish(&self, id: &str) -> Vec<fn()> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).as_mut().and_then(|asking| asking.remove(id)).unwrap_or_default()
    }
}

pub(crate) fn path() -> PathBuf {
    files::cache().join("models.json")
}

/// Lets this process ask agents for their models and help, as the hub does at its start. It stays off in the demo world
/// and with COCKPIT_HUB_MODELS=off, which the sandbox and the tests set, so neither runs an agent found on the machine.
pub fn ask_agents() {
    ASKS.store(may_ask(std::env::var_os("COCKPIT_AGENTS_DEMO").as_deref(), std::env::var_os("COCKPIT_HUB_MODELS").as_deref()), Ordering::SeqCst);
}

pub(crate) fn asks() -> bool {
    ASKS.load(Ordering::SeqCst)
}

fn may_ask(demo: Option<&OsStr>, switch: Option<&OsStr>) -> bool {
    !demo.is_some_and(|world| !world.is_empty()) && switch != Some(OsStr::new("off"))
}

/// The models the agent offers: the ones it listed when last asked, else its table's. A listing over a day old (half an
/// hour for one that failed) is asked again in the background, `ready` called once it answered; a probed catalog a day
/// old has the agent's version checked instead, and is asked again only once that changed.
pub fn of(agent: &Agent, ready: fn()) -> Listed {
    of_in(&path(), agent, crate::seen::now_ms(), asks(), ready)
}

/// The models a routine of the agent picks from, its own default first (""): the ones only a routine takes, then the
/// ones it offers everywhere.
pub fn for_routine(agent: &Agent, ready: fn()) -> Vec<String> {
    let offered = of(agent, ready).offers.into_iter().map(|o| o.id);
    let mut seen = std::collections::HashSet::new();
    std::iter::once(String::new()).chain(agent.routine_only()).chain(offered).filter(|m| seen.insert(m.clone())).collect()
}

fn of_in(file: &Path, agent: &Agent, now: u64, asks: bool, ready: fn()) -> Listed {
    let Some((argv, how)) = agent.model_listing() else { return Listed { offers: agent.offers(), loading: false } };
    let kept = load(file);
    let entry = &kept[&agent.id];
    let listed: Vec<Offer> = entry["models"].as_array().into_iter().flatten().filter_map(Offer::of_json).collect();
    let start = asks && !fresh(file, entry, &agent.id, now) && agent.installed();
    let (loading, begun) = ASKING.join(&agent.id, ready, start);
    if begun {
        let (id, file, agent, kept_version) = (agent.id.clone(), file.to_path_buf(), agent.clone(), entry["version"].as_str().map(str::to_string));
        std::thread::spawn(move || {
            let answer = match how {
                Listing::Probe => match version(&agent, kept_version) {
                    Version::Same => Answer::Unchanged,
                    Version::Changed(v) => Answer::Models(probe(&argv, agent.catalog_request().unwrap_or_default()).map(|r| agent::catalog(&id, &r)), v),
                },
                _ => Answer::Models(run(&argv).and_then(|out| agent::listed_models(&out, how)), None),
            };
            keep(&file, &id, &answer, now);
            for ready in ASKING.finish(&id) {
                ready();
            }
        });
    }
    Listed { offers: if listed.is_empty() { agent.offers() } else { listed }, loading }
}

/// Whether what `entry` holds, asked under `key` (an agent id, or `help:<id>`), is recent enough not to ask again: a
/// day since it was last attempted, half an hour after a failure. A file that could not be written holds this process
/// back the same way (`tried`).
pub(crate) fn fresh(file: &Path, entry: &Value, key: &str, now: u64) -> bool {
    let every = if entry["failed"] == true { RETRY } else { KEPT };
    let fresh = entry["attempted"].as_u64().or_else(|| entry["at"].as_u64()).is_some_and(|at| now.saturating_sub(at) < every);
    let tried = TRIED.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).get(&(file.to_path_buf(), key.to_string())).copied();
    fresh || tried.is_some_and(|(at, failed)| now.saturating_sub(at) < if failed { RETRY } else { KEPT })
}

/// Whether an agent's version is the one something was kept under.
pub(crate) enum Version {
    Same,
    /// Its version now, None where it prints none or printed nothing.
    Changed(Option<String>),
}

/// The agent's version against the one `kept`: an agent without a version command, or whose printed none, reads as
/// changed, so what was kept of it is asked again as before.
pub(crate) fn version(agent: &Agent, kept: Option<String>) -> Version {
    let now =
        agent.version_argv().and_then(|argv| run_for(&argv, VERSION_LIMIT, 64 << 10)).and_then(|out| out.lines().next().map(|l| crate::text::clean(l, 128)));
    match now.map(|v| v.trim().to_string()).filter(|v| !v.is_empty()) {
        Some(v) if kept.as_deref() == Some(v.as_str()) => Version::Same,
        Some(v) => Version::Changed(Some(v)),
        None => Version::Changed(None),
    }
}

/// What a background ask ended with.
enum Answer {
    /// The models listed (None: the ask failed), under the agent's version where one is known.
    Models(Option<Vec<Offer>>, Option<String>),
    /// The agent's version is the one kept: nothing new.
    Unchanged,
}

/// The kept file, from memory while it has not changed since it was last read.
pub(crate) fn load(file: &Path) -> Value {
    let meta = std::fs::metadata(file).ok();
    let stamp = (meta.as_ref().and_then(|m| m.modified().ok()), meta.as_ref().map_or(0, |m| m.len()));
    let mut read = READ.lock().unwrap_or_else(|e| e.into_inner());
    let read = read.get_or_insert_with(HashMap::new);
    if let Some((at, len, value)) = read.get(file)
        && (*at, *len) == stamp
    {
        return value.clone();
    }
    let value = files::load(file).unwrap_or(Value::Null);
    read.insert(file.to_path_buf(), (stamp.0, stamp.1, value.clone()));
    value
}

/// Changes the agent's entry of the kept file under one lock, `change` given the entry as an object: whether the file
/// was written.
pub(crate) fn update(file: &Path, id: &str, change: impl FnOnce(&mut Value)) -> bool {
    let _one = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    let mut kept = files::load(file).filter(Value::is_object).unwrap_or_else(|| json!({}));
    if !kept[id].is_object() {
        kept[id] = json!({});
    }
    change(&mut kept[id]);
    files::write(file, &kept).is_ok()
}

/// Notes that this process asked under `key` at `now`, and whether that failed or its answer was not kept.
pub(crate) fn tried(file: &Path, key: &str, now: u64, failed: bool) {
    let mut tried = TRIED.lock().unwrap_or_else(|e| e.into_inner());
    tried.get_or_insert_with(HashMap::new).insert((file.to_path_buf(), key.to_string()), (now, failed));
}

/// Writes what a listing answered, that it failed, or that the version is the one kept: a failure keeps the models of
/// the last one that answered.
fn keep(file: &Path, id: &str, answer: &Answer, now: u64) {
    let failed = matches!(answer, Answer::Models(None, _));
    let written = update(file, id, |entry| {
        if let Answer::Models(Some(models), version) = answer {
            entry["at"] = json!(now);
            entry["models"] = json!(models.iter().map(Offer::json).collect::<Vec<_>>());
            entry["version"] = version.as_deref().map_or(Value::Null, Value::from);
        }
        entry["attempted"] = json!(now);
        entry["failed"] = json!(failed);
    });
    tried(file, id, now, failed || !written);
}

/// The listing's output, None when it failed or ran past its limit.
fn run(argv: &[String]) -> Option<String> {
    run_for(argv, LIMIT, LISTING_CAP)
}

fn run_for(argv: &[String], limit: Duration, cap: usize) -> Option<String> {
    let mut cmd = Command::new(argv.first()?);
    cmd.args(&argv[1..]);
    let out = crate::run::output(&mut cmd, limit, cap, &Default::default()).ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The answer to the agent's catalog `request` over its stream: `argv` opened apart from any session (`run::untie`) in a
/// group of its own, the request written as the first line, its events read until the `control_response` to it, then
/// the group ended. None when it failed, answered otherwise, or ran past LIMIT.
fn probe(argv: &[String], request: &str) -> Option<Value> {
    let mut cmd = Command::new(argv.first()?);
    cmd.args(&argv[1..]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).process_group(0);
    crate::run::untie(&mut cmd);
    let mut child = cmd.spawn().ok()?;
    let pid = child.id();
    let line = json!({"type": "control_request", "request_id": PROBE_ID, "request": {"subtype": request}}).to_string() + "\n";
    // Its input stays open until it answered: an end of input would end the session first.
    let mut stdin = child.stdin.take();
    if let Some(stdin) = stdin.as_mut() {
        let _ = stdin.write_all(line.as_bytes()).and_then(|()| stdin.flush());
    }
    let stdout = child.stdout.take();
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let Some(stdout) = stdout else { return };
        let mut reader = BufReader::new(stdout.take(LISTING_CAP as u64));
        let mut buf = String::new();
        while matches!(reader.read_line(&mut buf), Ok(n) if n > 0) {
            if let Ok(event) = serde_json::from_str::<Value>(&buf)
                && event["type"] == "control_response"
                && event["response"]["request_id"] == PROBE_ID
            {
                let _ = tx.send((event["response"]["subtype"] == "success").then(|| event["response"]["response"].clone()));
                return;
            }
            buf.clear();
        }
    });
    let answer = rx.recv_timeout(LIMIT).ok().flatten();
    crate::run::kill_group(pid);
    drop(stdin);
    let _ = child.wait();
    answer
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::AtomicUsize;

    // The agents being asked are one list for the process, by agent id: a test that asks an agent (`asks` true) or reads
    // `loading` keeps an id no other such test uses (opencode, codex, omp, maki, kilo, pi, claude); the others read offers only.
    fn nobody() {}

    /// A table row under an id of this test's own, its executable a script in `dir`.
    fn stub(dir: &Path, row: &str, script: &str) -> Agent {
        let bin = dir.join(row);
        std::fs::write(&bin, script).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut agent = agent::builtin().into_iter().find(|a| a.id == row).unwrap();
        agent.bin = bin.to_string_lossy().into_owned();
        agent
    }

    fn wait_for(file: &Path, id: &str) -> Value {
        for _ in 0..100 {
            if let Some(v) = files::load(file).filter(|v| v[id].is_object()) {
                return v;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("{id} was never listed");
    }

    fn ids(listed: &Listed) -> Vec<&str> {
        listed.offers.iter().map(|o| o.id.as_str()).collect()
    }

    #[test]
    fn an_agent_s_models_are_asked_once_a_day_in_the_background() {
        let dir = tempfile::tempdir().unwrap();
        let agent = stub(dir.path(), "opencode", "#!/bin/sh\n[ \"$1\" = models ] && printf 'acme/large\\nacme/small\\n\\nacme/large\\n'\n");
        let file = dir.path().join("models.json");
        assert!(of_in(&file, &agent, 1_000, true, nobody).offers.is_empty(), "nothing until it answered");
        assert_eq!(
            wait_for(&file, "opencode")["opencode"]["models"],
            json!([{"id": "acme/large", "label": "acme/large"}, {"id": "acme/small", "label": "acme/small"}])
        );
        assert_eq!(ids(&of_in(&file, &agent, 2_000, true, nobody)), ["acme/large", "acme/small"]);
        std::fs::write(&agent.bin, "#!/bin/sh\nprintf 'acme/new\\n'\n").unwrap();
        assert_eq!(ids(&of_in(&file, &agent, 3_000, true, nobody)), ["acme/large", "acme/small"], "kept for a day");
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(files::load(&file).unwrap()["opencode"]["at"], 1_000, "not asked again within the day");
    }

    #[test]
    fn a_catalog_is_probed_over_the_stream_once_and_again_only_on_a_new_version() {
        let dir = tempfile::tempdir().unwrap();
        let (version, probed, args) = (dir.path().join("version"), dir.path().join("probed"), dir.path().join("args"));
        std::fs::write(&version, "1.0.0 (Acme Code)\n").unwrap();
        let answer = json!({"type": "control_response", "response": {"subtype": "success", "request_id": PROBE_ID, "response": {"models": [
            {"value": "default", "resolvedModel": "acme-large-2", "displayName": "Default (recommended)", "supportsEffort": true, "supportedEffortLevels": ["low", "high"], "supportsFastMode": true},
            {"value": "acme-small", "displayName": "Acme Small", "description": "Fastest for quick answers"},
        ]}}});
        let script = format!(
            "#!/bin/sh\nif [ \"$1\" = --version ]; then cat '{}'; exit 0; fi\nprintf '%s\\n' \"$*\" > '{}'\nread -r line\necho x >> '{}'\nprintf '%s\\n' '{{\"type\":\"system\",\"subtype\":\"init\"}}'\nprintf '%s\\n' '{}'\nsleep 30\n",
            version.display(),
            args.display(),
            probed.display(),
            answer
        );
        let agent = stub(dir.path(), "claude", &script);
        let file = dir.path().join("models.json");
        assert_eq!(ids(&of_in(&file, &agent, 1_000, true, nobody)), ["opus", "sonnet", "haiku", "fable"], "its table's until it answered");
        let settled = |want: usize| {
            for _ in 0..400 {
                let count = std::fs::read_to_string(&probed).map(|c| c.lines().count()).unwrap_or(0);
                if count >= want && !ASKING.join("claude", nobody, false).0 {
                    return;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            panic!("not probed {want} times");
        };
        settled(1);
        let kept = files::load(&file).unwrap();
        assert_eq!(kept["claude"]["version"], "1.0.0 (Acme Code)");
        assert_eq!(
            kept["claude"]["models"][1],
            json!({"id": "acme-small", "label": "Acme Small", "efforts": [], "fast": false, "description": "Fastest for quick answers"})
        );
        let listed = of_in(&file, &agent, 2_000, true, nobody);
        assert_eq!(ids(&listed), ["default", "acme-small"]);
        assert_eq!((listed.offers[0].resolved.as_deref(), listed.offers[0].fast), (Some("acme-large-2"), Some(true)));
        let argv = std::fs::read_to_string(&args).unwrap();
        assert!(argv.contains("--input-format stream-json") && !argv.contains("--permission-mode") && !argv.contains("--session-id"), "{argv}");
        // A day on, its version is checked and found the same: not probed again.
        of_in(&file, &agent, 1_000 + KEPT, true, nobody);
        for _ in 0..400 {
            if files::load(&file).unwrap()["claude"]["attempted"] == 1_000 + KEPT {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        assert_eq!(std::fs::read_to_string(&probed).unwrap().lines().count(), 1, "the same version: not probed again");
        assert_eq!(files::load(&file).unwrap()["claude"]["at"], 1_000);
        std::fs::write(&version, "2.0.0 (Acme Code)\n").unwrap();
        of_in(&file, &agent, 1_000 + 2 * KEPT, true, nobody);
        settled(2);
        assert_eq!(files::load(&file).unwrap()["claude"]["version"], "2.0.0 (Acme Code)", "a new version: probed again");
        assert!(!of_in(&file, &agent, 1_000 + 2 * KEPT + 1, true, nobody).loading);
    }

    #[test]
    fn every_surface_reads_the_same_models_of_an_agent() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("models.json");
        let table = agent::builtin();
        let row = |id: &str| table.iter().find(|a| a.id == id).unwrap().clone();
        let catalog =
            json!({"models": [{"slug": "gpt-acme-1", "display_name": "GPT Acme 1", "visibility": "list", "supported_reasoning_levels": [{"effort": "low"}]}]});
        keep(&file, "codex", &Answer::Models(agent::listed_models(&catalog.to_string(), agent::Listing::Codex), None), 1);
        keep(&file, "opencode", &Answer::Models(agent::listed_models("acme/large\nacme/small\n", agent::Listing::Lines), None), 1);
        let codex = of_in(&file, &row("codex"), 2, false, nobody);
        assert_eq!(
            codex.offers,
            [Offer { id: "gpt-acme-1".into(), label: "GPT Acme 1".into(), efforts: Some(vec!["low".into()]), fast: Some(false), ..Offer::default() }]
        );
        assert_eq!(ids(&of_in(&file, &row("opencode"), 2, false, nobody)), ["acme/large", "acme/small"]);
        assert_eq!(ids(&of_in(&file, &row("claude"), 2, false, nobody)), ["opus", "sonnet", "haiku", "fable"], "not probed yet: its table's");
        assert!(of_in(&file, &row("cursor"), 2, false, nobody).offers.is_empty(), "neither a listing nor a table list");
        // What the pickers send and what a routine picks from are that one list.
        for a in table.iter().filter(|a| a.model_listing().is_none()) {
            let picked: Vec<String> = a.catalog_picks(nobody)["models"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap().to_string()).collect();
            let routine: Vec<String> = for_routine(a, nobody).into_iter().filter(|m| !m.is_empty() && !a.routine_only().contains(m)).collect();
            assert_eq!(picked, routine, "{}", a.id);
        }
        assert_eq!(for_routine(&row("claude"), nobody), ["", "lightest", "opus", "sonnet", "haiku", "fable"], "lightest is a routine's alone");
    }

    #[test]
    fn a_model_kept_by_its_id_alone_reads_as_an_offer() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("models.json");
        files::write(&file, &json!({"maki": {"at": 1, "attempted": 1, "models": ["acme/large", {"id": "acme/small", "label": "Small", "fast": true}]}}))
            .unwrap();
        let agent = agent::builtin().into_iter().find(|a| a.id == "maki").unwrap();
        let listed = of_in(&file, &agent, 2, false, nobody);
        assert_eq!(listed.offers[0], Offer::named("acme/large".into()));
        assert_eq!((listed.offers[1].label.as_str(), listed.offers[1].fast, listed.loading), ("Small", Some(true), false));
    }

    #[test]
    fn a_slow_listing_reads_as_loading_and_tells_each_asker_once() {
        static TOLD: AtomicUsize = AtomicUsize::new(0);
        fn told() {
            TOLD.fetch_add(1, Ordering::SeqCst);
        }
        let dir = tempfile::tempdir().unwrap();
        let gate = dir.path().join("ready");
        let script = format!(
            "#!/bin/sh\nwhile [ ! -f '{}' ]; do sleep 0.02; done\nprintf '%s' '{{\"models\":[{{\"slug\":\"gpt-acme-1\",\"visibility\":\"list\"}}]}}'\n",
            gate.display()
        );
        let agent = stub(dir.path(), "codex", &script);
        let file = dir.path().join("models.json");
        let pending = of_in(&file, &agent, 1, true, nobody);
        assert_eq!((pending.loading, pending.offers.len()), (true, 0));
        assert!(of_in(&file, &agent, 1, true, told).loading, "a second asker joins the read the first began");
        of_in(&file, &agent, 1, true, told);
        std::fs::write(gate, "ready").unwrap();
        let until = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let now = of_in(&file, &agent, 2, true, nobody);
            if !now.loading && TOLD.load(Ordering::SeqCst) > 0 {
                assert_eq!(ids(&now), ["gpt-acme-1"]);
                break;
            }
            assert!(std::time::Instant::now() < until, "the listing did not finish");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(TOLD.load(Ordering::SeqCst), 1, "told once when it answered");
        assert!(!of_in(&file, &agent, 3, true, nobody).loading, "an answer a moment old is not asked again");
    }

    #[test]
    fn nothing_is_asked_in_the_demo_world_a_sandbox_or_a_test() {
        assert!(may_ask(None, None) && may_ask(Some(OsStr::new("")), Some(OsStr::new("on"))));
        assert!(!may_ask(Some(OsStr::new("/tmp/demo.json")), None), "the demo world");
        assert!(!may_ask(None, Some(OsStr::new("off"))), "a sandbox, the tests");
        // A process that never said it asks, as every test is, runs no listing: the stub would leave its mark.
        let dir = tempfile::tempdir().unwrap();
        let mark = dir.path().join("ran");
        let agent = stub(dir.path(), "kilo", &format!("#!/bin/sh\ntouch '{}'\nprintf 'acme/large\\n'\n", mark.display()));
        let file = dir.path().join("models.json");
        assert_eq!(of_in(&file, &agent, 1, asks(), nobody), Listed { offers: vec![], loading: false });
        std::thread::sleep(Duration::from_millis(300));
        assert!(!mark.exists() && !file.exists(), "its listing never ran");
    }

    #[test]
    fn a_file_that_cannot_be_written_holds_the_agent_back_as_a_failed_listing_does() {
        let dir = tempfile::tempdir().unwrap();
        let count = dir.path().join("asked");
        let script = format!("#!/bin/sh\necho x >> '{}'\nprintf 'provider  model\\nacme  large\\n'\n", count.display());
        let agent = stub(dir.path(), "pi", &script);
        let file = Path::new("/dev/null/models.json");
        assert!(of_in(file, &agent, 1, true, nobody).loading, "asked");
        let until = std::time::Instant::now() + Duration::from_secs(5);
        while of_in(file, &agent, 1, false, nobody).loading {
            assert!(std::time::Instant::now() < until, "the listing did not finish");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!of_in(file, &agent, 2, true, nobody).loading, "not asked again at once");
        assert!(!of_in(file, &agent, 1 + RETRY - 1, true, nobody).loading, "nor within half an hour");
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(std::fs::read_to_string(&count).unwrap().lines().count(), 1);
        assert!(of_in(file, &agent, 1 + RETRY, true, nobody).loading, "then asked again");
    }

    #[test]
    fn the_kept_file_is_read_again_once_it_changed() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("models.json");
        let opencode = agent::builtin().into_iter().find(|a| a.id == "opencode").unwrap();
        files::write(&file, &json!({"opencode": {"at": 1, "attempted": 1, "models": ["acme/large"]}})).unwrap();
        assert_eq!(ids(&of_in(&file, &opencode, 2, false, nobody)), ["acme/large"]);
        assert_eq!(ids(&of_in(&file, &opencode, 2, false, nobody)), ["acme/large"], "from memory");
        files::write(&file, &json!({"opencode": {"at": 1, "attempted": 1, "models": ["acme/large", "acme/small"]}})).unwrap();
        assert_eq!(ids(&of_in(&file, &opencode, 2, false, nobody)), ["acme/large", "acme/small"]);
    }

    #[test]
    fn each_listing_reads_its_own_way() {
        use agent::{Listing, listed_models};
        let ids = |out: &str, how| listed_models(out, how).map(|l| l.into_iter().map(|o| o.id).collect::<Vec<_>>());
        let table = "provider   model             context  max-out\nanthropic  claude-sonnet-5  1M       64K\nacme       large             200K     8K\n";
        assert_eq!(ids(table, Listing::Table).unwrap(), ["anthropic/claude-sonnet-5", "acme/large"]);
        let lines = "gemini-3.8-flash-medium        Gemini 3.8 Flash (Medium)\nacme-pro  Acme Pro\n";
        assert_eq!(ids(lines, Listing::Lines).unwrap(), ["gemini-3.8-flash-medium", "acme-pro"]);
        let json = r#"{"models": [{"provider": "acme", "id": "large", "selector": "acme/large"}, {"selector": "acme/large"}, {"id": "x"}]}"#;
        assert_eq!(ids(json, Listing::Json).unwrap(), ["acme/large"]);
        assert_eq!(ids("Error: not signed in; run `acme login`", Listing::Table), None, "no header and row");
        assert_eq!(ids("$(touch x)\n", Listing::Lines), None, "nothing a shell would act on");
        assert_eq!(ids("not json", Listing::Codex), None);
    }

    #[test]
    fn a_listing_gives_only_ids_no_agent_reads_as_a_switch_and_only_so_many() {
        use agent::{Listing, MAX_MODELS, listed_models};
        let ids = |out: &str, how| listed_models(out, how).map(|l| l.into_iter().map(|o| o.id).collect::<Vec<_>>()).unwrap_or_default();
        let long = "m".repeat(200);
        let hostile = format!("--dangerously-skip-permissions\n-p\n{long}\nacme/large\n");
        assert_eq!(ids(&hostile, Listing::Lines), ["acme/large"], "no switch, nothing too long");
        let json =
            serde_json::json!({"models": [{"selector": "-p"}, {"selector": "--dangerously-skip-permissions"}, {"selector": long}, {"selector": "acme/large"}]});
        assert_eq!(ids(&json.to_string(), Listing::Json), ["acme/large"]);
        let codex = serde_json::json!({"models": [{"slug": "-p"}, {"slug": "--dangerously-skip-permissions"}, {"slug": long},
            {"slug": "gpt-acme-1", "display_name": "GPT\u{1b}[31m Acme\u{202e} 1"}]});
        let offers = listed_models(&codex.to_string(), Listing::Codex).unwrap();
        assert_eq!(offers.iter().map(|o| (o.id.as_str(), o.label.as_str())).collect::<Vec<_>>(), [("gpt-acme-1", "GPT Acme 1")], "its label one plain line");
        let many: String = (0..10_000).map(|i| format!("acme/model-{i}\n")).collect();
        let listed = ids(&many, Listing::Lines);
        assert_eq!((listed.len(), listed[0].as_str()), (MAX_MODELS, "acme/model-0"));
        let codex_many = serde_json::json!({"models": (0..10_000).map(|i| serde_json::json!({"slug": format!("gpt-{i}")})).collect::<Vec<_>>()});
        assert_eq!(listed_models(&codex_many.to_string(), Listing::Codex).unwrap().len(), MAX_MODELS);
        // What a models.json written by hand or by an older cockpit holds is read the same way.
        for kept in [serde_json::json!("-p"), serde_json::json!({"id": "--dangerously-skip-permissions"}), serde_json::json!(long)] {
            assert_eq!(Offer::of_json(&kept), None, "{kept}");
        }
    }

    #[test]
    fn a_failed_refresh_preserves_the_last_good_models_and_is_tried_again_sooner() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("models.json");
        let large = vec![Offer::named("acme/large".into())];
        keep(&file, "opencode", &Answer::Models(Some(large.clone()), None), 1);
        keep(&file, "opencode", &Answer::Models(None, None), 2);
        let kept = files::load(&file).unwrap();
        assert_eq!(
            kept["opencode"],
            json!({"models": [{"id": "acme/large", "label": "acme/large"}], "at": 1, "attempted": 2, "failed": true, "version": null})
        );
        // Asked again half an hour after a failure, a day after an answer: the stub answers the second ask.
        let mut agent = stub(dir.path(), "opencode", "#!/bin/sh\nprintf 'acme/new\\n'\n");
        agent.id = "omp".into();
        assert!(agent.model_listing().is_some());
        keep(&file, "omp", &Answer::Models(Some(large), None), 1);
        keep(&file, "omp", &Answer::Models(None, None), 2);
        assert!(!of_in(&file, &agent, 2 + RETRY - 1, true, nobody).loading, "not yet");
        assert!(of_in(&file, &agent, 2 + RETRY, true, nobody).loading, "asked again");
    }
}
