//! `cockpit browse` on the Chromium side browser: the same commands as with terminal-browser, answered by the shell.
//! A browser belongs to the session it was opened for, named by its Herdr app terminal (`app:t3`): a page is opened
//! through the hub, which starts the shell; the rest reads the running shell.

use std::time::Duration;

use cockpit_core::{browser, frontend};
use serde_json::{Value, json};

use super::agent::{self, Tabs};
use crate::browse::{Place, place_of};
use crate::proc::{dumps, fail, open_url};

const TIMEOUT: Duration = Duration::from_secs(20);

/// The browser's owner for a Herdr app terminal.
pub fn owner_of(terminal: &str) -> String {
    format!("{}{terminal}", frontend::APP)
}

/// The owner of the browser beside an agent session, else beside the calling terminal; why not when it runs in none.
pub fn owner(session: Option<&str>) -> Result<String, String> {
    match place_of(session)? {
        Place::App(app) => Ok(owner_of(&app.terminal)),
        Place::Herdr(_) => Err(browser::APP_ONLY.into()),
        Place::Nowhere(why) => Err(why),
    }
}

/// A browser's tabs as the shell lists them; none while the shell is not running.
fn tabs_of(owner: &str) -> Vec<Value> {
    browser::request(&json!({"op": "tabs.list", "owner": owner}), TIMEOUT).ok().and_then(|a| a["tabs"].as_array().cloned()).unwrap_or_default()
}

/// The active tab beside a session, shaped as terminal-browser's: its browser key, tab id, URL and title.
pub fn beside(session: Option<&str>) -> Option<Value> {
    let tab = tabs_of(&owner(session).ok()?).into_iter().find(|t| t["active"] == true)?;
    Some(json!({"key": "chromium", "tab": tab["id"], "url": tab["url"], "title": tab["title"]}))
}

/// Every browser beside a session, shaped as terminal-browser's listing: one browser, the session's tabs.
pub fn browsers(session: Option<&str>) -> Value {
    let tabs = owner(session).map(|owner| tabs_of(&owner)).unwrap_or_default();
    if tabs.is_empty() { json!([]) } else { json!([{"key": "chromium", "tabs": tabs}]) }
}

pub fn external(session: Option<&str>) {
    match beside(session) {
        Some(page) => open_url(page["url"].as_str().unwrap_or_default()),
        None => fail("no page is open in the side browser for this session"),
    }
}

/// Opens a page in the session's browser; the hub starts the shell when it is not running.
pub fn open(url: &str, session: Option<&str>, reload: bool) -> Result<(), String> {
    let owner = owner(session)?;
    let cwd = std::env::current_dir().unwrap_or_default();
    let request =
        json!({"op": "act", "app": "browser", "action": "open", "args": [{"url": url, "owner": owner, "session": session, "cwd": cwd, "reload": reload}]});
    let answer = cockpit_core::hub::request(&request, Duration::from_secs(40))?;
    match answer["error"].as_str() {
        Some(why) => Err(why.to_string()),
        None => Ok(()),
    }
}

/// `cockpit browse action [-- ] <agent-browser command>`: the command on the active tab of the session's browser.
pub fn action(command: &[String], session: Option<&str>) -> ! {
    if command.is_empty() || command == ["done"] {
        std::process::exit(0);
    }
    if let Some(why) = agent::refusal(command) {
        fail(&format!("browse action: {why}"));
    }
    let owner = owner(session).unwrap_or_else(|why| fail(&format!("browse action: {why}")));
    let ask = |request: Value| browser::request(&request, TIMEOUT).unwrap_or_else(|why| fail(&format!("browse action: {why}")));
    if let Some(asked) = agent::tabs(command) {
        let listed = tabs_of(&owner);
        let named = |which: &str| agent::tab_named(&listed, which).map(str::to_string).unwrap_or_else(|| fail(&format!("browse action: no tab {which}")));
        match asked {
            Tabs::List { json: true } => println!("{}", json!({"success": true, "data": {"tabs": listed}, "error": null})),
            Tabs::List { json: false } => {
                for (at, tab) in listed.iter().enumerate() {
                    let mark = if tab["active"] == true { "*" } else { " " };
                    println!("{mark} t{} {}  {}", at + 1, tab["title"].as_str().unwrap_or_default(), tab["url"].as_str().unwrap_or_default());
                }
            }
            Tabs::New(url) => match open(url.as_deref().unwrap_or(""), session, false) {
                Ok(()) => println!("✓ Opened {}", url.unwrap_or_else(|| "a new tab".into())),
                Err(why) => fail(&format!("browse action: {why}")),
            },
            Tabs::Close(which) => {
                ask(json!({"op": "tab.close", "owner": owner, "tab": which.as_deref().map(named)}));
                println!("✓ Closed");
            }
            Tabs::Switch(which) => {
                ask(json!({"op": "tab.activate", "owner": owner, "tab": named(&which)}));
                println!("✓ Switched to {which}");
            }
        }
        std::process::exit(0);
    }
    let pinned = agent::pinned(&owner, None, "").unwrap_or_else(|why| fail(&format!("browse action: {why}")));
    let code = pinned.pass(command);
    let _ = browser::request(&json!({"op": "agent.touch", "owner": owner, "tab": pinned.tab}), TIMEOUT);
    std::process::exit(code)
}

/// `--beside` printed as terminal-browser's is.
pub fn beside_line(session: Option<&str>) -> String {
    match beside(session) {
        Some(p) => dumps(&[("key", p["key"].clone()), ("tab", p["tab"].clone()), ("url", p["url"].clone()), ("title", p["title"].clone())]),
        None => "{}".into(),
    }
}
