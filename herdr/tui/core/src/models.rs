//! The one catalog of models: what each agent offers wherever a person picks one (Start work, the chat's chips, a
//! routine). An agent with a listing command (`Agent::model_listing`) is asked in the background and its answer kept in
//! `~/.cache/cockpit/models.json`; any other offers the models its table row names.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use serde_json::{Value, json};

use crate::agent::{self, Agent, Offer};
use crate::files;

/// How long a list is kept before the agent is asked again, in ms.
const KEPT: u64 = 24 * 3_600_000;
/// How long after a listing failed the agent is asked again, in ms.
const RETRY: u64 = 30 * 60_000;
/// The most an agent's listing may take.
const LIMIT: Duration = Duration::from_secs(30);
/// The most of a listing read: Codex's catalog comes close to a megabyte.
const LISTING_CAP: usize = 8 << 20;

/// Whether this process asks agents for their models.
static ASKS: AtomicBool = AtomicBool::new(false);
type Asking = HashMap<String, Vec<fn()>>;
/// The agents being asked now, each with whom to tell once it answered.
static ASKING: Mutex<Option<Asking>> = Mutex::new(None);
static WRITING: Mutex<()> = Mutex::new(());
type Read = HashMap<PathBuf, (Option<SystemTime>, u64, Value)>;
/// Each file as last read, with its modification time and size: read again only once it changed.
static READ: Mutex<Option<Read>> = Mutex::new(None);
type Tried = HashMap<(PathBuf, String), (u64, bool)>;
/// When this process last asked each agent, and whether that failed or its answer could not be kept, by file.
static TRIED: Mutex<Option<Tried>> = Mutex::new(None);

/// What an agent offers: its models, and whether its own listing is being read now.
#[derive(Clone, Debug, PartialEq)]
pub struct Listed {
    pub offers: Vec<Offer>,
    pub loading: bool,
}

fn path() -> PathBuf {
    files::cache().join("models.json")
}

/// Lets this process ask agents for their models, as the hub does at its start. It stays off in the demo world and with
/// COCKPIT_HUB_MODELS=off, which the sandbox and the tests set, so neither runs an agent found on the machine.
pub fn ask_agents() {
    ASKS.store(may_ask(std::env::var_os("COCKPIT_AGENTS_DEMO").as_deref(), std::env::var_os("COCKPIT_HUB_MODELS").as_deref()), Ordering::SeqCst);
}

fn may_ask(demo: Option<&OsStr>, switch: Option<&OsStr>) -> bool {
    !demo.is_some_and(|world| !world.is_empty()) && switch != Some(OsStr::new("off"))
}

/// The models the agent offers: the ones it listed when last asked, else its table's. A listing over a day old (half an
/// hour for one that failed) is asked again in the background, `ready` called once it answered.
pub fn of(agent: &Agent, ready: fn()) -> Listed {
    of_in(&path(), agent, crate::seen::now_ms(), ASKS.load(Ordering::SeqCst), ready)
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
    let every = if entry["failed"] == true { RETRY } else { KEPT };
    let fresh = entry["attempted"].as_u64().or_else(|| entry["at"].as_u64()).is_some_and(|at| now.saturating_sub(at) < every);
    // A file that could not be written still holds this process back, as after a failed listing.
    let tried = TRIED.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).get(&(file.to_path_buf(), agent.id.clone())).copied();
    let fresh = fresh || tried.is_some_and(|(at, failed)| now.saturating_sub(at) < if failed { RETRY } else { KEPT });
    let mut asking = ASKING.lock().unwrap_or_else(|e| e.into_inner());
    let asking = asking.get_or_insert_with(HashMap::new);
    if asks && !fresh && !asking.contains_key(&agent.id) && agent.installed() {
        asking.insert(agent.id.clone(), Vec::new());
        let (id, file) = (agent.id.clone(), file.to_path_buf());
        std::thread::spawn(move || {
            let models = run(&argv).and_then(|out| agent::listed_models(&out, how));
            keep(&file, &id, models.as_deref(), now);
            let told = ASKING.lock().unwrap_or_else(|e| e.into_inner()).as_mut().and_then(|asking| asking.remove(&id)).unwrap_or_default();
            for ready in told {
                ready();
            }
        });
    }
    let loading = asking.get_mut(&agent.id).is_some_and(|told| {
        if !told.iter().any(|known| std::ptr::fn_addr_eq(*known, ready)) {
            told.push(ready);
        }
        true
    });
    Listed { offers: if listed.is_empty() { agent.offers() } else { listed }, loading }
}

/// The kept file, from memory while it has not changed since it was last read.
fn load(file: &Path) -> Value {
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

/// Writes what a listing answered, or that it failed: a failure keeps the models of the last one that answered.
fn keep(file: &Path, id: &str, models: Option<&[Offer]>, now: u64) {
    let _one = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    let mut kept = files::load(file).filter(Value::is_object).unwrap_or_else(|| json!({}));
    if let Some(models) = models {
        kept[id] = json!({"at": now, "models": models.iter().map(Offer::json).collect::<Vec<_>>()});
    }
    if !kept[id].is_object() {
        kept[id] = json!({});
    }
    kept[id]["attempted"] = json!(now);
    kept[id]["failed"] = json!(models.is_none());
    let written = files::write(file, &kept).is_ok();
    let mut tried = TRIED.lock().unwrap_or_else(|e| e.into_inner());
    tried.get_or_insert_with(HashMap::new).insert((file.to_path_buf(), id.to_string()), (now, models.is_none() || !written));
}

/// The listing's output, None when it failed or ran past its limit.
fn run(argv: &[String]) -> Option<String> {
    let mut cmd = Command::new(argv.first()?);
    cmd.args(&argv[1..]);
    let out = crate::run::output(&mut cmd, LIMIT, LISTING_CAP, &Default::default()).ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::AtomicUsize;

    // The agents being asked are one list for the process, by agent id: a test that asks an agent (`asks` true) or reads
    // `loading` keeps an id no other such test uses (opencode, codex, omp, maki, kilo, pi); the others read offers only.
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
    fn every_surface_reads_the_same_models_of_an_agent() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("models.json");
        let table = agent::builtin();
        let row = |id: &str| table.iter().find(|a| a.id == id).unwrap().clone();
        let catalog =
            json!({"models": [{"slug": "gpt-acme-1", "display_name": "GPT Acme 1", "visibility": "list", "supported_reasoning_levels": [{"effort": "low"}]}]});
        keep(&file, "codex", agent::listed_models(&catalog.to_string(), agent::Listing::Codex).as_deref(), 1);
        keep(&file, "opencode", agent::listed_models("acme/large\nacme/small\n", agent::Listing::Lines).as_deref(), 1);
        let codex = of_in(&file, &row("codex"), 2, false, nobody);
        assert_eq!(codex.offers, [Offer { id: "gpt-acme-1".into(), label: "GPT Acme 1".into(), efforts: Some(vec!["low".into()]), fast: Some(false) }]);
        assert_eq!(ids(&of_in(&file, &row("opencode"), 2, false, nobody)), ["acme/large", "acme/small"]);
        assert_eq!(ids(&of_in(&file, &row("claude"), 2, false, nobody)), ["opus", "sonnet", "haiku", "fable"], "no listing: its table's");
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
        assert_eq!(of_in(&file, &agent, 1, ASKS.load(Ordering::SeqCst), nobody), Listed { offers: vec![], loading: false });
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
        let large = [Offer::named("acme/large".into())];
        keep(&file, "opencode", Some(&large), 1);
        keep(&file, "opencode", None, 2);
        let kept = files::load(&file).unwrap();
        assert_eq!(kept["opencode"], json!({"models": [{"id": "acme/large", "label": "acme/large"}], "at": 1, "attempted": 2, "failed": true}));
        // Asked again half an hour after a failure, a day after an answer: the stub answers the second ask.
        let mut agent = stub(dir.path(), "opencode", "#!/bin/sh\nprintf 'acme/new\\n'\n");
        agent.id = "omp".into();
        assert!(agent.model_listing().is_some());
        keep(&file, "omp", Some(&large), 1);
        keep(&file, "omp", None, 2);
        assert!(!of_in(&file, &agent, 2 + RETRY - 1, true, nobody).loading, "not yet");
        assert!(of_in(&file, &agent, 2 + RETRY, true, nobody).loading, "asked again");
    }
}
