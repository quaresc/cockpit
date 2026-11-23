//! `cockpit browse`: opens a page in terminal-browser, split right of a pane, else in the default browser, for an
//! agent's E2E tests and browser views. terminal-browser is used when it is installed and the pane is in herdr; a page
//! already open beside the pane comes to the front instead of opening twice, and the keyboard stays where it was. In
//! the Herdr app the browser is a terminal split right of the agent's, found again by its tty, and every terminal-browser
//! call gets that session's pane and the app's socket. A listing or a herdr call that does not answer is an error, never
//! taken for "no browser there". With `sideBrowser: chromium`, in the Herdr app on macOS, the same commands go to
//! cockpit's own Chromium window instead (`crate::browser::side`).

use std::path::{Path, PathBuf};
use std::time::Duration;

use cockpit_core::browser::{self as chromium, Engine};
use cockpit_core::frontend::{self, Frontend};
use serde_json::{Value, json};

use crate::browser::side;
use crate::proc::{self, Out, dumps, fail, open_url, quote};

const USAGE: &str = "Opens a page in the side browser (terminal-browser split right of a pane, or cockpit's Chromium window), else in the default browser.

    cockpit browse [--size <fraction>] [--reload] <url> [<session id>]          beside the session's pane, else the calling one
    cockpit browse --beside [<session id>]                                      the browser beside it, as JSON ({} when none)
    cockpit browse --external [<session id>]                                    its page, opened in the default browser
    cockpit browse --browsers [<session id>]                                    every browser beside it, as JSON
    cockpit browse --env [<session id>]                                         in the Herdr app, the variables terminal-browser needs, as exports
    cockpit browse action [--session <session id>] [--] <command>               an agent-browser command (snapshot -i, click @e3, ...) on the page beside it; `action done` ends a run";

/// herdr sends a picture of at most 16 MiB through a file, a larger one inline through the terminal, which is far slower.
const MAX_FRAME: f64 = 16.0 * 1024.0 * 1024.0;
pub const DEFAULT_SIZE: f64 = 0.5;
const TIMEOUT: Duration = Duration::from_secs(10);
/// How many half seconds a browser just split in gets to start.
const STARTS: usize = 20;

pub fn main(args: &[String]) {
    let mut args: Vec<String> = args.to_vec();
    let or_fail = |why: String| -> ! { fail(&format!("browse: {why}")) };
    // Asked silently here: only opening a page says why Chromium was wanted and is not used.
    let side_browser = chromium::engine_for(&cockpit_core::files::config(), frontend::current(), cockpit_core::options::CHROMIUM_HERE).0;
    if args.first().is_some_and(|a| a == "action") {
        let asked = action_args(&args[1..]);
        match side_browser {
            Engine::Chromium => side::action(&asked.command, asked.session.as_deref()),
            Engine::Terminal => terminal_action(&asked),
        }
    }
    if side_browser == Engine::Chromium {
        let session = args.get(1).map(String::as_str);
        match args.first().map(String::as_str) {
            Some("--beside") => return println!("{}", side::beside_line(session)),
            Some("--browsers") => return println!("{}", side::browsers(session)),
            Some("--external") => return side::external(session),
            Some("--env") => return,
            _ => {}
        }
    }
    match args.first().map(String::as_str) {
        Some("--beside") => {
            println!("{}", beside_json(&beside(args.get(1).map(String::as_str)).unwrap_or_else(|why| or_fail(why))));
            return;
        }
        Some("--browsers") => {
            let place = place_of(args.get(1).map(String::as_str)).unwrap_or_else(|why| or_fail(why));
            let browsers = terminal_browser().map(|bin| browsers_at(&tb_at(bin, &place), &place)).transpose().unwrap_or_else(|why| or_fail(why));
            println!("{}", Value::Array(browsers.unwrap_or_default()));
            return;
        }
        Some("--external") => {
            let page = beside(args.get(1).map(String::as_str)).unwrap_or_else(|why| or_fail(why));
            let Some(page) = page else { fail("no terminal-browser beside this pane") };
            open_url(page["url"].as_str().unwrap_or_default());
            return;
        }
        Some("--env") => {
            match place_of(args.get(1).map(String::as_str)).unwrap_or_else(|why| or_fail(why)) {
                Place::App(app) => app_env(&app).iter().for_each(|(k, v)| println!("export {k}={}", quote(v))),
                Place::Herdr(_) => {}
                Place::Nowhere(why) => or_fail(why),
            }
            return;
        }
        _ => {}
    }
    let (mut size, mut reload) = (DEFAULT_SIZE, false);
    while args.first().is_some_and(|a| a.starts_with("--")) {
        let flag = args.remove(0);
        if flag == "--size" && !args.is_empty() {
            let value = args.remove(0);
            size = value.parse().unwrap_or_else(|_| fail(&format!("--size wants a fraction, not {value}")));
        } else if flag == "--reload" {
            reload = true;
        }
    }
    let Some(url) = args.first() else { fail(USAGE) };
    let session = args.get(1).map(String::as_str);
    let not_beside = match terminal_browser() {
        _ if chromium::engine() == Engine::Chromium => match side::open(url, session, reload) {
            Ok(()) => return,
            Err(why) => why,
        },
        Some(bin) => {
            let place = place_of(session).unwrap_or_else(|why| or_fail(why));
            match open_at(&tb_at(bin, &place), &place, url, size, reload).unwrap_or_else(|why| or_fail(why)) {
                Opened::Yes => return,
                Opened::No(why) => why,
            }
        }
        None => "terminal-browser is not installed".into(),
    };
    eprintln!("browse: {not_beside}: opening the page in the default browser");
    open_url(&if url.starts_with('/') { format!("file://{url}") } else { url.clone() });
}

/// What `cockpit browse action` was given: the session it names, terminal-browser's own flags, and the command.
#[derive(Debug, Default, PartialEq, Eq)]
struct Action {
    session: Option<String>,
    flags: Vec<String>,
    command: Vec<String>,
}

/// The words after `action`, split at `--`: a session and terminal-browser's flags before it, the command after.
fn action_args(args: &[String]) -> Action {
    let mut asked = Action::default();
    let mut words = args.iter();
    while let Some(word) = words.next() {
        match word.as_str() {
            "--" => break,
            "--session" => asked.session = words.next().cloned(),
            "--browser" | "--tab" | "--target" => asked.flags.extend([Some(word.clone()), words.next().cloned()].into_iter().flatten()),
            "--follow" => asked.flags.push(word.clone()),
            _ => {
                asked.command.push(word.clone());
                break;
            }
        }
    }
    asked.command.extend(words.cloned());
    asked
}

/// The command in the terminal-browser beside the session: its browser is named for it in the Herdr app, where
/// terminal-browser cannot tell which one is meant.
fn terminal_action(asked: &Action) -> ! {
    let Some(bin) = terminal_browser() else { fail("browse action: terminal-browser is not installed") };
    let place = place_of(asked.session.as_deref()).unwrap_or_else(|why| fail(&format!("browse action: {why}")));
    let tb = tb_at(bin, &place);
    let mut args: Vec<String> = vec!["action".into()];
    args.extend(asked.flags.iter().cloned());
    if matches!(place, Place::App(_)) && !asked.flags.iter().any(|f| f == "--browser") {
        let key = browsers_at(&tb, &place).ok().and_then(|b| b.first().map(|b| field(b, "key"))).filter(|k| !k.is_empty());
        args.extend(key.into_iter().flat_map(|k| ["--browser".to_string(), k]));
    }
    if asked.command != ["done"] {
        args.push("--".into());
    }
    args.extend(asked.command.iter().cloned());
    let status = std::process::Command::new(&tb.bin).args(&args).env("PATH", proc::path()).envs(tb.env.iter().map(|(k, v)| (k.as_str(), v.as_str()))).status();
    std::process::exit(status.ok().and_then(|s| s.code()).unwrap_or(1))
}

pub fn run(argv: &[&str], timeout: Duration) -> Result<Out, String> {
    proc::run(argv, &[], timeout)
}

fn herdr_bin() -> String {
    std::env::var("HERDR_BIN_PATH").ok().filter(|p| !p.is_empty()).or_else(|| proc::which("herdr")).unwrap_or_else(|| "herdr".into())
}

/// herdr's answer: None where there is no herdr or it refuses (outside herdr), an error when it did not answer.
fn herdr(args: &[&str]) -> Result<Option<Value>, String> {
    let bin = herdr_bin();
    if !proc::executable(Path::new(&bin)) {
        return Ok(None);
    }
    let argv: Vec<&str> = std::iter::once(bin.as_str()).chain(args.iter().copied()).collect();
    let out = run(&argv, TIMEOUT)?;
    Ok(out.ok().then(|| serde_json::from_str::<Value>(&out.stdout).ok()?.get("result").cloned()).flatten())
}

/// A terminal-browser listing's browsers; an error when it did not run, failed or printed no list.
fn listed(out: Result<Out, String>) -> Result<Vec<Value>, String> {
    let out = out.map_err(|why| format!("terminal-browser ls: {why}"))?;
    if !out.ok() {
        let said = out.stderr.trim().lines().last().map_or_else(|| format!("exit {}", out.code), str::to_string);
        return Err(format!("terminal-browser ls: {said}"));
    }
    serde_json::from_str::<Value>(&out.stdout).ok().and_then(|v| v["browsers"].as_array().cloned()).ok_or_else(|| "terminal-browser ls printed no list".into())
}

/// The terminal-browser command, or None when it is missing; COCKPIT_TERMINAL_BROWSER names the one to use instead,
/// missing when it is not an executable.
pub fn terminal_browser() -> Option<String> {
    match std::env::var("COCKPIT_TERMINAL_BROWSER") {
        Ok(path) if !path.is_empty() => proc::executable(std::path::Path::new(&path)).then_some(path),
        _ => proc::which("terminal-browser"),
    }
}

/// terminal-browser with the variables that tell it where it runs.
pub struct Tb {
    pub bin: String,
    env: Vec<(String, String)>,
}

impl Tb {
    pub fn run(&self, args: &[&str], timeout: Duration) -> Result<Out, String> {
        let argv: Vec<&str> = std::iter::once(self.bin.as_str()).chain(args.iter().copied()).collect();
        let env: Vec<(&str, &str)> = self.env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        proc::run(&argv, &env, timeout)
    }
}

/// Where a session sits: a Herdr app pane, a herdr pane (with its tab and space), or nowhere a browser can go, and why.
pub enum Place {
    App(AppPlace),
    Herdr(Value),
    Nowhere(String),
}

impl Place {
    /// Whether it is a window terminal-browser can open beside: a herdr pane or a Herdr app terminal.
    pub fn is_placed(&self) -> bool {
        !matches!(self, Place::Nowhere(_))
    }
}

/// A session in the Herdr app: its terminal and the app pane showing it.
#[derive(Debug, PartialEq, Eq)]
pub struct AppPlace {
    pub terminal: String,
    pub pane: String,
}

/// The place of an agent session, else of the calling one; an error only when herdr does not answer.
pub fn place_of(session: Option<&str>) -> Result<Place, String> {
    if frontend::current() == Frontend::App {
        return Ok(app_place(session, &frontend::app_spaces()).map_or_else(Place::Nowhere, Place::App));
    }
    Ok(match pane_of(session)? {
        Some(pane) => Place::Herdr(pane),
        None => Place::Nowhere("this session is in neither a herdr pane nor a Herdr app terminal".into()),
    })
}

/// terminal-browser for that place. In the Herdr app it gets the session's pane and the app's socket: without them it
/// takes the app's terminals for Ghostty's and fails.
pub fn tb_at(bin: String, place: &Place) -> Tb {
    let env = match place {
        Place::App(app) => app_env(app),
        _ => Vec::new(),
    };
    Tb { bin, env }
}

fn app_env(app: &AppPlace) -> Vec<(String, String)> {
    let socket = frontend::app_home().join("herdr-app.sock");
    vec![("HERDR_PANE_ID".into(), app.pane.clone()), ("HERDR_SOCKET_PATH".into(), socket.to_string_lossy().into_owned())]
}

// ---------------------------------------------------------------------------------------------------------------------
// The Herdr app

/// The app terminal and pane of an agent session (as the agents document has it), else of the calling one.
fn app_place(session: Option<&str>, spaces: &[Value]) -> Result<AppPlace, String> {
    let by_session = session.and_then(|s| {
        let doc = cockpit_core::hub::request(&json!({"op": "store.get", "name": "agents"}), TIMEOUT).ok()?["value"].clone();
        doc.as_object()?.values().find(|f| f["session"] == s)?["terminal"].as_str().map(|t| t.trim_start_matches(frontend::APP).to_string())
    });
    let own = std::env::var("HC_TERMINAL_ID").ok().filter(|t| !t.is_empty());
    place_in(spaces, by_session.or(own), || calling_terminal(spaces))
}

/// The pane of the named terminal, else of the listed terminal the caller descends from; never another session's, and
/// why not when there is none.
fn place_in(spaces: &[Value], named: Option<String>, calling: impl FnOnce() -> Option<String>) -> Result<AppPlace, String> {
    if app_terminals(spaces).next().is_none() {
        return Err("the Herdr window lists no terminals (is it open?)".into());
    }
    let terminal = named.or_else(calling).ok_or("this session runs in no terminal of the Herdr window")?;
    let pane = pane_in(spaces, &terminal).ok_or_else(|| format!("the Herdr window shows terminal {terminal} in no pane"))?;
    Ok(AppPlace { terminal, pane })
}

/// The app pane that shows a terminal, as `herdr-app ls` lists them.
fn pane_in(spaces: &[Value], terminal: &str) -> Option<String> {
    app_terminals(spaces).find(|t| t["terminal"] == terminal).and_then(|t| t["pane"].as_str().map(str::to_string))
}

fn app_terminals(spaces: &[Value]) -> impl Iterator<Item = &Value> {
    spaces.iter().flat_map(|s| s["tabs"].as_array().into_iter().flatten()).flat_map(|t| t["terminals"].as_array().into_iter().flatten())
}

/// The listed terminal whose program this process is or descends from.
fn calling_terminal(spaces: &[Value]) -> Option<String> {
    let pids: Vec<(String, u64)> = app_terminals(spaces)
        .filter_map(|t| t["terminal"].as_str())
        .filter_map(|t| Some((t.to_string(), frontend::hc(&["get", t])?["pid"].as_u64()?)))
        .collect();
    nearest(&pids, &ancestors())
}

/// This process and its ancestors, nearest first.
fn ancestors() -> Vec<u64> {
    let mut line = vec![u64::from(std::process::id())];
    while line.len() < 64 {
        let pid = line[line.len() - 1].to_string();
        match run(&["ps", "-o", "ppid=", "-p", &pid], TIMEOUT).ok().and_then(|o| o.stdout.trim().parse::<u64>().ok()) {
            Some(parent) if parent > 1 => line.push(parent),
            _ => break,
        }
    }
    line
}

/// The terminal whose program is the nearest of these ancestors.
fn nearest(terminals: &[(String, u64)], ancestors: &[u64]) -> Option<String> {
    ancestors.iter().find_map(|a| terminals.iter().find(|(_, pid)| pid == a).map(|(t, _)| t.clone()))
}

/// The other terminals in the terminal's tab, each with its app pane.
fn tab_mates(spaces: &[Value], terminal: &str) -> Vec<(String, String)> {
    for tab in spaces.iter().flat_map(|s| s["tabs"].as_array().into_iter().flatten()) {
        let terminals = tab["terminals"].as_array().cloned().unwrap_or_default();
        if terminals.iter().any(|t| t["terminal"] == terminal) {
            return terminals
                .iter()
                .filter(|t| t["terminal"] != terminal)
                .filter_map(|t| Some((t["terminal"].as_str()?.into(), t["pane"].as_str()?.into())))
                .collect();
        }
    }
    Vec::new()
}

/// The tty a terminal's program runs on, when it is a terminal-browser.
fn browser_tty(terminal: &str) -> Option<String> {
    let found = frontend::hc(&["get", terminal])?;
    if !found["foreground"]["name"].as_str().is_some_and(|n| n.contains("terminal-browser")) {
        return None;
    }
    let pid = found["foreground"]["pid"].as_u64()?.to_string();
    let tty = run(&["ps", "-o", "tty=", "-p", &pid], TIMEOUT).ok()?.stdout.trim().to_string();
    (!tty.is_empty() && tty != "??").then(|| format!("/dev/{}", tty.trim_start_matches("/dev/")))
}

/// The terminal-browsers in the app terminal's tab, each with the app `pane` showing it.
fn app_browsers(tb: &Tb, terminal: &str) -> Result<Vec<Value>, String> {
    let ttys: Vec<(String, String)> = tab_mates(&frontend::app_spaces(), terminal).into_iter().filter_map(|(t, pane)| Some((browser_tty(&t)?, pane))).collect();
    if ttys.is_empty() {
        return Ok(Vec::new());
    }
    let all = listed(tb.run(&["ls", "--all", "--json"], TIMEOUT))?;
    Ok(all
        .into_iter()
        .filter_map(|mut b| {
            let pane = ttys.iter().find(|(tty, _)| b["tty"].as_str() == Some(tty))?.1.clone();
            b["app_pane"] = json!(pane);
            Some(b)
        })
        .collect())
}

/// Whether the page is open beside the session now, else why not.
pub enum Opened {
    Yes,
    No(String),
}

/// The page beside a session, in whichever frontend it runs.
pub fn open_at(tb: &Tb, place: &Place, url: &str, size: f64, reload: bool) -> Result<Opened, String> {
    match place {
        Place::App(app) => app_open_beside(tb, app, url, reload),
        Place::Herdr(pane) => open_beside(tb, pane, url, size, reload),
        Place::Nowhere(why) => Ok(Opened::No(why.clone())),
    }
}

/// The browsers beside a session, in whichever frontend it runs.
pub fn browsers_at(tb: &Tb, place: &Place) -> Result<Vec<Value>, String> {
    match place {
        Place::App(app) => app_browsers(tb, &app.terminal),
        Place::Herdr(pane) => browsers_beside(tb, pane),
        Place::Nowhere(_) => Ok(Vec::new()),
    }
}

/// The page in a browser already beside the terminal, else a terminal-browser split right of it; the keyboard stays.
fn app_open_beside(tb: &Tb, app: &AppPlace, url: &str, reload: bool) -> Result<Opened, String> {
    let browsers = app_browsers(tb, &app.terminal)?;
    if let Some(done) = reuse(tb, &browsers, url, reload) {
        return Ok(done);
    }
    let split = frontend::app_do("pane.split", &json!({"direction": "right", "pane": app.pane, "argv": [tb.bin, "open", url]}));
    frontend::app_do("pane.focus", &json!({"terminal": app.terminal}));
    let Some(split) = split else {
        return Ok(Opened::No(format!("the Herdr window did not split pane {}", app.pane)));
    };
    if let Some(pane) = split["pane"].as_str() {
        frontend::app_do("pane.rename", &json!({"pane": pane, "name": "Browser"}));
    }
    // The browser takes a moment to start: once it lists, `--beside` and the agent's next action find it.
    for _ in 0..STARTS {
        if app_browsers(tb, &app.terminal).is_ok_and(|b| !b.is_empty()) {
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Ok(Opened::Yes)
}

/// A page already open in one of the browsers comes to the front (reloaded when asked), else a new tab in the first
/// one; None when there is no browser.
fn reuse(tb: &Tb, browsers: &[Value], url: &str, reload: bool) -> Option<Opened> {
    let done = |out: Result<Out, String>, what: &str| match out {
        Ok(o) if o.ok() => Opened::Yes,
        Ok(o) => Opened::No(format!("terminal-browser {what}: {}", o.stderr.trim().lines().last().unwrap_or_default())),
        Err(why) => Opened::No(format!("terminal-browser {what}: {why}")),
    };
    for browser in browsers {
        let key = field(browser, "key");
        for tab in browser["tabs"].as_array().into_iter().flatten() {
            if same_page(&field(tab, "url"), url) {
                let id = field(tab, "id");
                let mut args = vec!["action", "--browser", &key, "--tab", &id, "--follow", "--"];
                args.extend(if reload { &["reload"][..] } else { &["get", "url"][..] });
                return Some(done(tb.run(&args, TIMEOUT), "action"));
            }
        }
    }
    let first = browsers.first()?;
    Some(done(tb.run(&["new-tab", "--browser", &field(first, "key"), url], TIMEOUT), "new-tab"))
}

// ---------------------------------------------------------------------------------------------------------------------
// herdr

/// The pane of an agent session, else the calling one, with its tab and space; None outside herdr.
fn pane_of(session: Option<&str>) -> Result<Option<Value>, String> {
    let listed = herdr(&["pane", "list"]).map_err(|why| format!("herdr pane list: {why}"))?.unwrap_or(Value::Null);
    let panes = listed["panes"].as_array().cloned().unwrap_or_default();
    let by_session = session.and_then(|s| panes.iter().find(|p| p["agent_session"]["value"] == s));
    let calling = std::env::var("HERDR_PANE_ID").ok().and_then(|id| panes.iter().find(|p| p["pane_id"] == id.as_str()));
    Ok(by_session.or(calling).cloned())
}

fn field(value: &Value, name: &str) -> String {
    match &value[name] {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// The terminal-browsers open in the pane's tab.
fn browsers_beside(tb: &Tb, pane: &Value) -> Result<Vec<Value>, String> {
    let (pane_id, tab_id) = (field(pane, "pane_id"), field(pane, "tab_id"));
    listed(proc::run(&[&tb.bin, "ls", "--json"], &[("HERDR_PANE_ID", &pane_id), ("HERDR_TAB_ID", &tab_id)], TIMEOUT))
}

/// The path with its existing part's links resolved, the rest kept as written.
fn real_path(path: &Path) -> PathBuf {
    for base in path.ancestors() {
        if let Ok(real) = std::fs::canonicalize(base) {
            return real.join(path.strip_prefix(base).unwrap_or(Path::new("")));
        }
    }
    path.to_path_buf()
}

fn same_page(a: &str, b: &str) -> bool {
    let norm = |url: &str| {
        let path = if url.starts_with('/') { Some(url) } else { url.strip_prefix("file://") };
        let url = path.map_or_else(|| url.to_string(), |p| format!("file://{}", real_path(Path::new(p)).display()));
        url.trim_end_matches('/').to_string()
    };
    norm(a) == norm(b)
}

/// The tty's size in pixels, from the shell running in the pane.
fn pixels(pane: &Value) -> Option<(f64, f64)> {
    let info = herdr(&["pane", "process-info", "--pane", &field(pane, "pane_id")]).ok().flatten().unwrap_or(Value::Null);
    let pid = &info["process_info"]["shell_pid"];
    if pid.is_null() || pid == false || pid == 0 || pid == "" {
        return None;
    }
    let pid = field(&info["process_info"], "shell_pid");
    let tty = run(&["ps", "-o", "tty=", "-p", &pid], TIMEOUT).ok()?.stdout.trim().to_string();
    if tty.is_empty() || tty == "??" {
        return None;
    }
    let file = std::fs::File::open(format!("/dev/{tty}")).ok()?;
    let mut size = libc::winsize { ws_row: 0, ws_col: 0, ws_xpixel: 0, ws_ypixel: 0 };
    use std::os::fd::AsRawFd;
    // SAFETY: TIOCGWINSZ only writes a winsize into the struct it is given, which lives for the call.
    if unsafe { libc::ioctl(file.as_raw_fd(), libc::TIOCGWINSZ, &mut size) } != 0 || size.ws_xpixel == 0 || size.ws_ypixel == 0 {
        return None;
    }
    Some((f64::from(size.ws_xpixel), f64::from(size.ws_ypixel)))
}

/// The fraction of the pane a new split takes: as wanted, but small enough for herdr to send its frames as files.
fn split_size(pane: &Value, wanted: f64) -> f64 {
    let Some((width, height)) = pixels(pane) else { return wanted };
    fraction(wanted, width, height)
}

fn fraction(wanted: f64, width: f64, height: f64) -> f64 {
    let fits = MAX_FRAME / (4.0 * height) / width;
    round2(wanted.min(fits).clamp(0.2, 0.95))
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

fn open_beside(tb: &Tb, pane: &Value, url: &str, size: f64, reload: bool) -> Result<Opened, String> {
    let browsers = browsers_beside(tb, pane)?;
    if let Some(done) = reuse(tb, &browsers, url, reload) {
        return Ok(done);
    }
    // The split is herdr's own, so the keyboard stays where it is; its ratio is what the first pane keeps.
    let ratio = round2(1.0 - split_size(pane, size)).to_string();
    let pane_id = field(pane, "pane_id");
    let split = herdr(&["pane", "split", "--pane", &pane_id, "--direction", "right", "--ratio", &ratio, "--no-focus"])
        .map_err(|why| format!("herdr pane split: {why}"))?;
    let Some(browser) = split.and_then(|s| s["pane"]["pane_id"].as_str().filter(|p| !p.is_empty()).map(str::to_string)) else {
        return Ok(Opened::No(format!("herdr did not split pane {pane_id}")));
    };
    // exec, so the pane closes with the browser.
    let command = format!("exec {} open {}", quote(&tb.bin), quote(url));
    let ran = run(&[&herdr_bin(), "pane", "run", &browser, &command], TIMEOUT).is_ok_and(|o| o.ok());
    Ok(if ran { Opened::Yes } else { Opened::No(format!("herdr did not start terminal-browser in pane {browser}")) })
}

/// The active tab of the browser beside the pane: its browser key, tab id, URL and title.
pub fn beside(session: Option<&str>) -> Result<Option<Value>, String> {
    let Some(bin) = terminal_browser() else { return Ok(None) };
    let place = place_of(session)?;
    Ok(browsers_at(&tb_at(bin, &place), &place)?.iter().find_map(|browser| {
        let tab = browser["tabs"].as_array()?.iter().find(|t| t["active"].as_bool() == Some(true))?;
        Some(json!({"key": browser["key"], "tab": tab["id"], "url": tab.get("url").cloned().unwrap_or(json!("")), "title": tab.get("title").cloned().unwrap_or(json!(""))}))
    }))
}

fn beside_json(page: &Option<Value>) -> String {
    match page {
        Some(p) => dumps(&[("key", p["key"].clone()), ("tab", p["tab"].clone()), ("url", p["url"].clone()), ("title", p["title"].clone())]),
        None => "{}".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_split_small_enough_for_frames_sent_as_files() {
        assert_eq!(fraction(0.5, 1000.0, 1000.0), 0.5);
        assert_eq!(fraction(0.5, 4000.0, 2000.0), 0.5);
        assert_eq!(fraction(0.9, 5000.0, 2400.0), 0.35);
        assert_eq!(fraction(0.5, 20000.0, 4000.0), 0.2);
        assert_eq!(fraction(1.0, 100.0, 100.0), 0.95);
    }

    #[test]
    fn the_same_page_whatever_its_trailing_slash_or_link() {
        assert!(same_page("https://x.test/a/", "https://x.test/a"));
        assert!(!same_page("https://x.test/a", "https://x.test/b"));
        let dir = tempfile::tempdir().unwrap();
        let real = std::fs::canonicalize(dir.path()).unwrap();
        assert!(same_page(&format!("{}/page.html", dir.path().display()), &format!("file://{}/page.html", real.display())));
        assert!(
            same_page(&format!("file://{}/page.html", dir.path().display()), &format!("{}/page.html", dir.path().display())),
            "the browser's URL keeps the link"
        );
    }

    fn spaces() -> Vec<Value> {
        let tab = |id: &str, terminals: Value| json!({"id": id, "terminals": terminals});
        vec![json!({"id": "sp1", "tabs": [
            tab("tb1", json!([{"terminal": "t2", "pane": "p62d2"}, {"terminal": "t3", "pane": "pe496"}])),
            tab("tb2", json!([{"terminal": "t7", "pane": "pa11"}])),
        ]})]
    }

    #[test]
    fn the_app_pane_of_the_named_terminal_never_another() {
        let place = place_in(&spaces(), Some("t7".into()), || panic!("a named terminal needs no search"));
        assert_eq!(place, Ok(AppPlace { terminal: "t7".into(), pane: "pa11".into() }));
        assert_eq!(place_in(&spaces(), Some("t9".into()), || Some("t2".into())), Err("the Herdr window shows terminal t9 in no pane".into()));
    }

    #[test]
    fn without_a_terminal_id_the_one_the_caller_descends_from() {
        assert_eq!(place_in(&spaces(), None, || Some("t3".into())), Ok(AppPlace { terminal: "t3".into(), pane: "pe496".into() }));
        assert_eq!(place_in(&spaces(), None, || None), Err("this session runs in no terminal of the Herdr window".into()));
        assert_eq!(place_in(&[], Some("t2".into()), || None), Err("the Herdr window lists no terminals (is it open?)".into()));
    }

    #[test]
    fn the_nearest_ancestor_names_the_terminal() {
        let terminals = [("t2".to_string(), 500), ("t3".to_string(), 700)];
        assert_eq!(nearest(&terminals, &[900, 700, 500, 1]), Some("t3".into()));
        assert_eq!(nearest(&terminals, &[900, 800]), None);
    }

    #[test]
    fn an_action_is_split_into_its_session_its_flags_and_its_command() {
        let words = |line: &str| line.split_whitespace().map(str::to_string).collect::<Vec<_>>();
        let asked = action_args(&words("--session s-412 --browser b1 --follow -- click @e3 --new-tab"));
        assert_eq!(asked, Action { session: Some("s-412".into()), flags: words("--browser b1 --follow"), command: words("click @e3 --new-tab") });
        assert_eq!(action_args(&words("snapshot -i")).command, words("snapshot -i"));
        assert_eq!(action_args(&words("done")).command, ["done"]);
        assert_eq!(action_args(&words("-- eval --session")).command, words("eval --session"), "what follows -- is the command's own");
    }

    #[test]
    fn beside_prints_like_before() {
        let page = json!({"key": "b1", "tab": 2, "url": "https://x.test/", "title": "Café"});
        assert_eq!(beside_json(&Some(page)), r#"{"key": "b1", "tab": 2, "url": "https://x.test/", "title": "Caf\u00e9"}"#);
        assert_eq!(beside_json(&None), "{}");
    }
}
