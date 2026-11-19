//! The hub's side of the Chromium side browser: it starts the shell, hands it the slot the Herdr window's mod reports,
//! keeps what the mod draws (`view browser`), and delivers the elements a person picks to the session the browser
//! belongs to. A browser is named by its owner, the session's Herdr app terminal (`app:t3`).

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use cockpit_core::browser::{self, Engine};
use cockpit_core::{files, frontend, options, peer};
use cockpit_hub::views::{self, Def};
use serde_json::{Map, Value, json};

use super::{install, ports};

const TIMEOUT: Duration = Duration::from_secs(10);

/// What the shell last said: `{running, shown, frame, current, opened, tabs: {owner: [...]}}`.
fn state() -> MutexGuard<'static, Value> {
    static STATE: Mutex<Value> = Mutex::new(Value::Null);
    let mut held = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if held.is_null() {
        *held = json!({"running": false, "shown": false, "frame": null, "tabs": {}, "current": null, "opened": null});
    }
    held
}

/// The secret the shell is started with: a request that carries it is one the hub checked a person made.
fn token() -> &'static str {
    static TOKEN: OnceLock<String> = OnceLock::new();
    TOKEN.get_or_init(|| {
        let mut bytes = [0u8; 16];
        if let Ok(mut random) = std::fs::File::open("/dev/urandom") {
            use std::io::Read;
            let _ = random.read_exact(&mut bytes);
        }
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    })
}

/// The view the mod's slot reads, and what keeps the hub while the browser runs.
pub fn register() {
    views::register(Def { name: "browser", docs: &[], dirs: Vec::new, build: view, act });
    cockpit_hub::on_start(adopt);
    cockpit_hub::keep_alive(running);
}

fn running() -> bool {
    state()["running"] == true
}

/// A shell an earlier hub started keeps running: this hub follows it again.
fn adopt() {
    if browser::connect().is_some() {
        follow();
    }
}

/// The options the shell reads, from config.json.
fn shell_options() -> Value {
    let config = files::config();
    let of = |key: &str| options::get(key).map_or(Value::Null, |s| options::value(&config, s));
    json!({"dock": of("browser.dock"), "profileScope": of("browser.profileScope"), "popups": of("browser.popups"), "downloads": of("browser.downloads"), "idleMinutes": of("browserIdleMinutes")})
}

fn record_file() -> PathBuf {
    browser::cache().join("browser.json")
}

/// Starts the shell unless it answers already, and follows what it says.
pub fn ensure() -> Result<(), String> {
    if browser::connect().is_some() {
        follow();
        return Ok(());
    }
    let pins = install::pins()?;
    let electron = install::electron(&pins);
    if !electron.is_file() {
        return Err("the Chromium side browser is not installed: run cockpit browser install".into());
    }
    let run = browser::cache().join("run");
    std::fs::create_dir_all(&run).map_err(|e| e.to_string())?;
    super::agent::prune();
    let log = || {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(browser::cache().join("browser.out"))
            .map_or_else(|_| std::process::Stdio::null(), std::process::Stdio::from)
    };
    use std::os::unix::process::CommandExt;
    let mut child = std::process::Command::new(&electron)
        .arg(install::shell_dir())
        // Never a worktree's folder: a program working in one keeps it from being removed.
        .current_dir(&run)
        .env("COCKPIT_BROWSER_SOCKET", browser::socket())
        .env("COCKPIT_BROWSER_TOKEN", token())
        .env("COCKPIT_BROWSER_OPTIONS", shell_options().to_string())
        .env("COCKPIT_BIN", std::env::current_exe().unwrap_or_default())
        .env_remove("ELECTRON_RUN_AS_NODE")
        .stdin(std::process::Stdio::null())
        .stdout(log())
        .stderr(log())
        .process_group(0)
        .spawn()
        .map_err(|e| format!("the Chromium side browser did not start: {e}"))?;
    let _ = files::write(&record_file(), &json!({"pid": child.id()}));
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(15) {
        std::thread::sleep(Duration::from_millis(50));
        if browser::connect().is_some() {
            follow();
            return Ok(());
        }
    }
    Err("the Chromium side browser did not start; what it said is in ~/.cache/cockpit/browser/browser.out".into())
}

/// Whether the other end of a connection is the shell this hub, or one before it, started.
fn is_ours(stream: &std::os::unix::net::UnixStream) -> bool {
    let recorded = files::load(&record_file()).and_then(|r| r["pid"].as_u64());
    recorded.is_some() && cockpit_hub::peer_pid(stream).map(u64::from) == recorded
}

/// Reads the shell's events on a thread of its own, one at a time, until the shell goes away.
fn follow() {
    static FOLLOWING: AtomicBool = AtomicBool::new(false);
    if FOLLOWING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        if let Some(mut stream) = browser::connect() {
            let ours = is_ours(&stream);
            if writeln!(stream, "{}", json!({"op": "subscribe"})).is_ok() {
                {
                    let mut held = state();
                    held["running"] = json!(true);
                    // Which run of the shell this is: a slot told to the run before is told again.
                    held["run"] = json!(cockpit_core::seen::now_ms());
                }
                seed();
                views::touch("browser");
                for line in BufReader::new(stream).lines().map_while(Result::ok) {
                    if let Ok(event) = serde_json::from_str::<Value>(&line) {
                        heard(&event, ours);
                    }
                }
            }
        }
        let opened = state()["opened"].take();
        *state() = json!({"running": false, "shown": false, "frame": null, "tabs": {}, "current": null, "opened": opened});
        views::touch("browser");
        FOLLOWING.store(false, Ordering::SeqCst);
    });
}

/// What a shell that was running before this hub followed it already shows: the browser in view and every tab.
fn seed() {
    let current = browser::request(&json!({"op": "status"}), TIMEOUT).map(|s| s["current"].clone()).unwrap_or(Value::Null);
    let listed = browser::request(&json!({"op": "tabs.list"}), TIMEOUT).ok();
    let mut held = state();
    held["current"] = current;
    for entry in listed.iter().flat_map(|l| l["browsers"].as_array().into_iter().flatten()) {
        if let Some(owner) = entry["owner"].as_str() {
            held["tabs"][owner] = entry["tabs"].clone();
        }
    }
}

/// One event of the shell: its tabs and its frame are kept for the mod, and picked elements go to the browser's session.
fn heard(event: &Value, ours: bool) {
    match event["event"].as_str().unwrap_or_default() {
        "tabs" => {
            if let Some(owner) = event["owner"].as_str() {
                state()["tabs"][owner] = event["tabs"].clone();
            }
        }
        "current" => state()["current"] = event["owner"].clone(),
        "docked" => {
            let mut held = state();
            held["shown"] = json!(event["shown"] == true);
            if !event["frame"].is_null() || event["shown"] == true {
                held["frame"] = event["frame"].clone();
            }
        }
        "grab.send" => {
            let sent = if ours { send_notes(event) } else { Err("cockpit cannot check which program asked, so nothing was sent".into()) };
            let answer = match &sent {
                Ok(()) => json!({"op": "notes.sent", "owner": event["owner"], "ok": true}),
                Err(why) => json!({"op": "notes.sent", "owner": event["owner"], "ok": false, "error": why}),
            };
            let _ = browser::request(&answer, TIMEOUT);
            return;
        }
        _ => return,
    }
    views::touch("browser");
}

/// The picked elements as the next prompt of the session the browser belongs to: its owner is that session's handle.
fn send_notes(event: &Value) -> Result<(), String> {
    let handle = event["owner"].as_str().filter(|h| h.starts_with(frontend::APP)).ok_or("this browser belongs to no agent session")?;
    let text = event["markdown"].as_str().filter(|t| !t.trim().is_empty()).ok_or("nothing to send")?;
    peer::with(peer::Sender::InProcess, || cockpit_core::deliver::deliver(handle, text))
}

/// A folder's worktree and the repo it belongs to, remembered for half a minute.
fn place(cwd: &Path) -> (PathBuf, PathBuf) {
    type Places = HashMap<PathBuf, (Instant, PathBuf, PathBuf)>;
    static PLACES: Mutex<Option<Places>> = Mutex::new(None);
    let mut held = PLACES.lock().unwrap_or_else(|e| e.into_inner());
    let places = held.get_or_insert_with(HashMap::new);
    if let Some((at, root, repo)) = places.get(cwd)
        && at.elapsed() < Duration::from_secs(30)
    {
        return (root.clone(), repo.clone());
    }
    let root = browser::worktree_of(cwd);
    let common = std::process::Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .stderr(std::process::Stdio::null())
        .output();
    let repo = match common {
        Ok(out) if out.status.success() => {
            let dir = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
            if dir.file_name().is_some_and(|n| n == ".git") { dir.parent().map(Path::to_path_buf).unwrap_or_else(|| root.clone()) } else { root.clone() }
        }
        _ => root.clone(),
    };
    places.insert(cwd.to_path_buf(), (Instant::now(), root.clone(), repo.clone()));
    (root, repo)
}

/// The folder an agent session works in, as the agents document has it.
fn session_cwd(session: &str) -> Option<PathBuf> {
    let doc = cockpit_hub::store::global().get("agents");
    doc.as_object()?.values().find(|f| f["session"] == session)?["cwd"].as_str().map(PathBuf::from)
}

/// The worktree a request is about, and its repo: its session's, else the folder it names; none when it names neither.
fn place_of(arg: &Value) -> Option<(PathBuf, PathBuf)> {
    let cwd = arg["session"]
        .as_str()
        .filter(|s| !s.is_empty())
        .and_then(session_cwd)
        .or_else(|| arg["cwd"].as_str().filter(|c| c.starts_with('/')).map(PathBuf::from));
    cwd.map(|cwd| place(&cwd))
}

/// The browser a request is about: the owner it names, else the Herdr app terminal it names.
fn owner_of(arg: &Value) -> Result<String, String> {
    let named = arg["owner"].as_str().filter(|o| o.starts_with(frontend::APP)).map(str::to_string);
    let terminal = arg["terminal"].as_str().filter(|t| !t.is_empty()).map(|t| format!("{}{t}", frontend::APP));
    named.or(terminal).ok_or_else(|| "which session? a browser belongs to the agent session it is opened for".into())
}

/// Closes the browsers of sessions whose terminal is gone, at most once a minute, and never while the Herdr window
/// lists no terminal at all: that is a window that does not answer, not one whose sessions ended.
fn close_ended() {
    static LAST: Mutex<Option<Instant>> = Mutex::new(None);
    {
        let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
        if last.is_some_and(|at| at.elapsed() < Duration::from_secs(60)) {
            return;
        }
        *last = Some(Instant::now());
    }
    std::thread::spawn(|| {
        let spaces = frontend::app_spaces();
        let live: Vec<String> = spaces
            .iter()
            .flat_map(|s| s["tabs"].as_array().into_iter().flatten())
            .flat_map(|t| t["terminals"].as_array().into_iter().flatten())
            .filter_map(|t| t["terminal"].as_str().map(|t| format!("{}{t}", frontend::APP)))
            .collect();
        if live.is_empty() {
            return;
        }
        let owners = browser::request(&json!({"op": "owners.list"}), TIMEOUT).ok();
        for owner in owners.iter().flat_map(|o| o["owners"].as_array().into_iter().flatten()).filter_map(|o| o["owner"].as_str()) {
            if owner.starts_with(frontend::APP) && !live.iter().any(|l| l == owner) {
                let _ = browser::request(&json!({"op": "owner.forget", "owner": owner}), TIMEOUT);
            }
        }
    });
}

/// The Herdr app's process, the one at the other end of its control socket.
fn host_pid() -> Option<u32> {
    let stream = std::os::unix::net::UnixStream::connect(frontend::app_home().join("herdr-app.sock")).ok()?;
    cockpit_hub::peer_pid(&stream)
}

/// The servers listening in each worktree, looked up off the request's thread and kept a quarter of a minute.
fn servers_in(root: &Path) -> Vec<Value> {
    type Known = HashMap<PathBuf, (Instant, Vec<Value>)>;
    static KNOWN: Mutex<Option<Known>> = Mutex::new(None);
    static LOOKING: AtomicBool = AtomicBool::new(false);
    let mut held = KNOWN.lock().unwrap_or_else(|e| e.into_inner());
    let known = held.get_or_insert_with(HashMap::new);
    let fresh = known.get(root).is_some_and(|(at, _)| at.elapsed() < Duration::from_secs(15));
    if !fresh && !LOOKING.swap(true, Ordering::SeqCst) {
        let root = root.to_path_buf();
        std::thread::spawn(move || {
            let found = ports::servers(&root);
            KNOWN.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(root, (Instant::now(), found));
            LOOKING.store(false, Ordering::SeqCst);
        });
    }
    known.get(root).map(|(_, found)| found.clone()).unwrap_or_default()
}

/// What the mod's slot draws: whether Chromium is the side browser here, whether it runs and shows, the last frame to
/// draw while it is hidden, how many tabs each session's browser has, and the page an agent opened last.
fn view(_args: &Value, _doc: views::Doc) -> Value {
    let (engine, _) = browser::engine_for(&files::config(), frontend::current(), options::CHROMIUM_HERE);
    let installed = install::pins().map(|p| install::installed(&p)).unwrap_or((false, false));
    let held = state().clone();
    let tabs: Map<String, Value> =
        held["tabs"].as_object().into_iter().flatten().map(|(owner, tabs)| (owner.clone(), json!(tabs.as_array().map_or(0, Vec::len)))).collect();
    json!({
        "enabled": engine == Engine::Chromium,
        "installed": installed.0 && installed.1,
        "docked": shell_options()["dock"] != false,
        "running": held["running"],
        "run": held["run"],
        "shown": held["shown"],
        "frame": held["frame"],
        "current": held["current"],
        "tabs": tabs,
        "opened": held["opened"],
    })
}

/// A request to the running shell, with the hub's token when a person asked.
fn ask(mut request: Value, person: bool) -> Result<Value, String> {
    if person {
        request["token"] = json!(token());
    }
    browser::request(&request, TIMEOUT)
}

fn done(answer: Result<Value, String>) -> Option<Result<String, String>> {
    Some(answer.map(|a| a.to_string()))
}

/// The browser's own actions: `open` a page for a session, the mod's `slot` and `unslot`, and what only a person asks.
fn act(action: &str, args: &[Value]) -> Option<Result<String, String>> {
    let arg = args.first().cloned().unwrap_or(Value::Null);
    match action {
        "open" => done(open(&arg)),
        "slot" => done(slot(&arg)),
        "unslot" => done(if running() { ask(json!({"op": "slot.drop"}), false) } else { Ok(Value::Null) }),
        "forget" => done(arg["worktree"].as_str().map_or(Err("which worktree?".into()), |root| {
            if running() { ask(json!({"op": "worktree.forget", "worktree": root}), false) } else { Ok(Value::Null) }
        })),
        "quit" => {
            let quit = if browser::connect().is_some() { ask(json!({"op": "quit"}), false) } else { Ok(Value::Null) };
            super::agent::prune();
            done(quit)
        }
        "profile_create" | "profile_delete" | "import_cookies" | "profile_use" => {
            let op = match action {
                "profile_create" => "profile.create",
                "profile_delete" => "profile.delete",
                "profile_use" => "profile.use",
                _ => "profile.import",
            };
            done(peer::person("a browser profile is the person's to change").and_then(|()| ensure()).and_then(|()| {
                let mut request = arg.as_object().cloned().unwrap_or_else(Map::new);
                request.insert("op".into(), json!(op));
                if op == "profile.use" {
                    request.insert("owner".into(), json!(owner_of(&arg)?));
                }
                ask(Value::Object(request), true)
            }))
        }
        _ => None,
    }
}

/// Opens a page in a session's browser, starting the shell when it is not running.
fn open(arg: &Value) -> Result<Value, String> {
    let (engine, why) = browser::engine_for(&files::config(), frontend::current(), options::CHROMIUM_HERE);
    if engine != Engine::Chromium {
        return Err(why.unwrap_or("the side browser is terminal-browser: set sideBrowser to chromium first").into());
    }
    let owner = owner_of(arg)?;
    let (root, repo) = place_of(arg).unwrap_or_default();
    ensure()?;
    let _ = ask(json!({"op": "configure", "options": shell_options(), "servers": servers_in(&root)}), false);
    let answer = ask(
        json!({"op": "open", "owner": owner, "worktree": root, "repo": repo, "url": arg["url"], "reload": arg["reload"] == true, "background": arg["background"] == true}),
        false,
    )?;
    // The mod brings the side panel to the browser when this session is the one in front.
    state()["opened"] = json!({"owner": owner, "at": cockpit_core::seen::now_ms()});
    views::touch("browser");
    Ok(answer)
}

/// The slot the mod reports, completed with what only the hub knows: the Herdr window's process, the browser of the
/// terminal in focus, the worktree of its folder with its servers, and where the shell leaves its last frame.
fn slot(arg: &Value) -> Result<Value, String> {
    if !running() {
        return Ok(Value::Null);
    }
    close_ended();
    let mut slot = arg.as_object().cloned().unwrap_or_default();
    slot.insert("hostPid".into(), json!(host_pid()));
    slot.insert("frames".into(), json!(frontend::app_home().join("data/cockpit/browser")));
    slot.insert("owner".into(), json!(owner_of(arg).ok()));
    if let Some(cwd) = arg["cwd"].as_str().filter(|c| c.starts_with('/')) {
        let (root, repo) = place(Path::new(cwd));
        slot.insert("servers".into(), json!(servers_in(&root)));
        slot.insert("worktree".into(), json!(root));
        slot.insert("repo".into(), json!(repo));
    }
    ask(json!({"op": "slot.set", "slot": slot}), false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_browser_is_named_by_its_session_never_by_anything_else() {
        assert_eq!(owner_of(&json!({"owner": "app:t3"})), Ok("app:t3".into()));
        assert_eq!(owner_of(&json!({"terminal": "t7"})), Ok("app:t7".into()));
        assert_eq!(owner_of(&json!({"owner": "app:t3", "terminal": "t7"})), Ok("app:t3".into()), "the owner a command worked out wins");
        assert!(owner_of(&json!({"owner": "/work/acme/api"})).is_err(), "a folder is no session");
        assert!(owner_of(&json!({"terminal": ""})).is_err());
        assert!(owner_of(&json!({})).unwrap_err().starts_with("which session?"));
    }

    #[test]
    fn notes_go_to_the_session_the_browser_belongs_to_or_nowhere() {
        assert_eq!(
            send_notes(&json!({"owner": "wt:/work/acme/api", "markdown": "## Design Feedback"})),
            Err("this browser belongs to no agent session".into())
        );
        assert_eq!(send_notes(&json!({"owner": "app:t3", "markdown": "  "})), Err("nothing to send".into()));
        assert_eq!(send_notes(&json!({"markdown": "## Design Feedback"})), Err("this browser belongs to no agent session".into()));
    }
}
