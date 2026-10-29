//! `cockpit doctor`: checks every part of cockpit and prints one line per check, ok, warn or fail, with what it found
//! and the fix.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use cockpit_core::files;
use serde_json::Value;

use crate::proc::{self, Out};

const SESSION: &str = "no Claude session with cockpit is running";
const NO_FEED: &str = "neither a cockpit app (its hub) nor a Claude session with cockpit is running";
const SPACES: [&str; 6] = ["inbox", "terminal", "actions", "to-do", "runbook", "meetings"];

fn ok(line: &str) {
    println!("ok    {line}");
}

fn warn(line: &str) {
    println!("warn  {line}");
}

fn fail(line: &str) {
    println!("fail  {line}");
}

/// What the checks read and run with.
struct Doctor {
    /// The herdr plugin folder this binary belongs to, its links resolved.
    root: PathBuf,
    /// ~/.config/cockpit/herdr, the link kept on the current install.
    link: PathBuf,
    /// The plugin folder herdr's registry names.
    registered: String,
    cache: PathBuf,
    /// The config as the mod resolved it, else the raw file; None when neither reads as JSON.
    settings: Option<Value>,
    settings_path: PathBuf,
    /// ~/.config/cockpit/config.json as written.
    config: Option<Value>,
    herdr: String,
    path: String,
    now: i64,
}

pub fn main(_: &[String]) {
    let doctor = Doctor::new();
    doctor.tools();
    doctor.installs();
    let installed = doctor.claude_plugin();
    doctor.sessions(&installed);
    doctor.caches();
    doctor.spaces();
    doctor.notifications();
    doctor.terminal_browser();
    doctor.connectors();
}

impl Doctor {
    fn new() -> Self {
        let home = files::home();
        let path = format!("/opt/homebrew/bin:/usr/local/bin:{0}/.local/bin:{0}/.claude/local:{1}", home.display(), std::env::var("PATH").unwrap_or_default());
        let herdr_dir = files::herdr_dir();
        let root = herdr_dir.canonicalize().unwrap_or(herdr_dir);
        let link = files::config_dir().join("herdr");
        let plugins = load(&home.join(".config/herdr/plugins.json")).unwrap_or(Value::Null);
        let registered = values(&plugins).filter(|p| p["plugin_id"] == "cockpit").map(|p| raw(&p["plugin_root"])).collect::<Vec<_>>().join("\n");
        // herdr installs each update in a new folder: only the folder its registry names moves the link.
        if registered == root.to_string_lossy() && readlink(&link) != registered {
            if let Some(parent) = link.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if std::fs::symlink_metadata(&link).is_ok_and(|m| m.file_type().is_symlink()) {
                let _ = std::fs::remove_file(&link);
            }
            let _ = std::os::unix::fs::symlink(&root, &link);
        }
        let config_path = files::config_dir().join("config.json");
        let resolved = files::cache().join("config.json");
        let settings_path = if resolved.is_file() { resolved } else { config_path.clone() };
        let settings = load(&settings_path);
        let herdr = std::env::var("HERDR_BIN_PATH").ok().filter(|h| !h.is_empty()).unwrap_or_else(|| "herdr".into());
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or_default();
        Doctor { root, link, registered, cache: files::cache(), settings, settings_path, config: load(&config_path), herdr, path, now }
    }

    fn set(&self) -> &Value {
        self.settings.as_ref().unwrap_or(&Value::Null)
    }

    fn run(&self, argv: &[&str], timeout: Duration) -> Option<Out> {
        proc::run(argv, &[("PATH", &self.path)], timeout)
    }

    /// A command's output as `$(…)` reads it: stdout without its trailing newlines, empty when it cannot run.
    fn output(&self, argv: &[&str]) -> String {
        self.run(argv, Duration::from_secs(60)).map(|o| o.stdout.trim_end_matches('\n').to_string()).unwrap_or_default()
    }

    fn succeeds(&self, argv: &[&str]) -> bool {
        self.run(argv, Duration::from_secs(60)).is_some_and(|o| o.ok())
    }

    fn herdr_json(&self, args: &[&str]) -> Value {
        let argv: Vec<&str> = std::iter::once(self.herdr.as_str()).chain(args.iter().copied()).collect();
        serde_json::from_str(&self.output(&argv)).unwrap_or(Value::Null)
    }

    fn running(&self, pattern: &str) -> bool {
        self.succeeds(&["pgrep", "-f", pattern])
    }

    /// The first executable of that name on the doctor's PATH.
    fn which(&self, name: &str) -> Option<PathBuf> {
        self.path.split(':').filter(|d| !d.is_empty()).map(|d| Path::new(d).join(name)).find(|p| proc::executable(p))
    }

    fn enabled(&self, source: &str) -> bool {
        is_true(or(&self.set()[source]["enabled"], &Value::Bool(false)))
    }

    fn has_token(&self, source: &str) -> bool {
        self.config.as_ref().is_some_and(|c| length(&c[source]["token"]) > 0.0)
    }

    // -----------------------------------------------------------------------------------------------------------------
    // Tools and config

    fn tools(&self) {
        let said = self.output(&[&self.herdr, "--version"]);
        let v = said.lines().map(|l| l.split_whitespace().nth(1).unwrap_or_default()).collect::<Vec<_>>().join("\n");
        let v = v.trim_end_matches('\n');
        if v.is_empty() || glob("0.[0-8].*", v) {
            fail(&format!("herdr {}: cockpit needs 0.9 or later → herdr update", if v.is_empty() { "missing" } else { v }));
        } else {
            ok(&format!("herdr {v}"));
        }
        if self.succeeds(&["gh", "auth", "status"]) {
            ok("gh signed in")
        } else {
            fail("gh not signed in → gh auth login")
        }

        // The release's binary, else a dev build: it runs every app and the hub.
        let root = self.root.display();
        let mut dir = self.root.join("bin");
        let dev = self.root.join("tui/target/release");
        if !proc::executable(&dir.join("cockpit")) && proc::executable(&dev.join("cockpit")) {
            dir = dev;
        }
        let toml = std::fs::read_to_string(self.root.join("herdr-plugin.toml")).unwrap_or_default();
        let want = toml.lines().filter_map(|l| l.strip_prefix("version = \"")?.strip_suffix('"')).collect::<Vec<_>>().join("\n");
        let have = std::fs::read_to_string(dir.join(".version")).unwrap_or_default().trim_end_matches('\n').to_string();
        let binary = dir.join("cockpit");
        let runs = self.output(&[&binary.to_string_lossy(), "--version"]);
        if !proc::executable(&binary) {
            fail(&format!("cockpit binary missing, the spaces stay empty → sh {root}/fetch-apps.sh, or cargo build --release in {root}/tui"));
        } else if dir == self.root.join("bin") && have != want {
            let have = if have.is_empty() { "an unknown version" } else { &have };
            warn(&format!("cockpit binary from {have}, the plugin is on {want}: the hub installs it once the release has it → sh {root}/fetch-apps.sh"));
        } else if runs.is_empty() {
            fail(&format!("cockpit binary in {} does not run on this machine → rm -rf {root}/bin && sh {root}/fetch-apps.sh", dir.display()));
        } else {
            ok(&format!("cockpit binary in {} ({})", dir.display(), runs.strip_prefix("cockpit ").unwrap_or(&runs)));
        }
        // A hub left with a dead socket keeps every action and search waiting.
        if self.running("cockpit (inbox|todo|runbook|meetings|monitor|routines) cockpit-space") {
            if !self.running("(^|/)cockpit hub$") {
                ok("hub not started yet: the apps start it at their first action or search");
            } else if self.succeeds(&[&binary.to_string_lossy(), "hub", "ping"]) {
                ok("hub answers");
            } else {
                fail("hub not answering → pkill -f '(^|/)cockpit hub$'; the apps start a new one");
            }
        }
        let config = files::config_dir().join("config.json");
        if !config.is_file() {
            fail("no ~/.config/cockpit/config.json → /cockpit:setup");
        } else if std::fs::read_to_string(&config).is_ok_and(|text| serde_json::Deserializer::from_str(&text).into_iter::<Value>().all(|v| v.is_ok())) {
            ok("config.json");
        } else {
            fail("config.json is not valid JSON → fix it or rerun /cockpit:setup");
        }
    }

    // -----------------------------------------------------------------------------------------------------------------
    // Both halves installed, and the link on the current herdr install

    fn installs(&self) {
        let registered = &self.registered;
        if !registered.is_empty() && Path::new(registered).is_dir() {
            ok(&format!("herdr plugin at {registered}"));
        } else {
            fail("herdr plugin not installed → herdr plugin install quaresc/cockpit/herdr --yes");
        }
        let linked = readlink(&self.link);
        if linked == *registered {
            ok("link ~/.config/cockpit/herdr is current");
        } else {
            fail(&format!("link ~/.config/cockpit/herdr points at {linked} → sh {registered}/ensure-spaces.sh"));
        }
        let is_github_install = registered.strip_suffix("/checkout/herdr").is_some_and(|rest| rest.contains("/github-installations/"));
        if is_github_install {
            let install = Path::new(registered).parent().and_then(Path::parent).unwrap_or(Path::new("/"));
            let installs = install.parent().unwrap_or(Path::new("/"));
            let old =
                std::fs::read_dir(installs).into_iter().flatten().flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir()) && e.path() != install).count();
            if old == 0 { ok("no old herdr installs") } else { warn(&format!("{old} old herdr installs → sh {registered}/ensure-spaces.sh prunes them")) }
        }
    }

    /// The Claude plugin's installed version, empty when it is not installed.
    fn claude_plugin(&self) -> String {
        let listed = self.output(&["claude", "plugin", "list"]);
        let mut found = false;
        let mut installed = String::new();
        for line in listed.lines() {
            found |= line.contains("cockpit@cockpit");
            if found && line.contains("Version:") {
                installed = line.split_whitespace().nth(1).unwrap_or_default().to_string();
                break;
            }
        }
        if installed.is_empty() {
            fail("Claude plugin not installed → /plugin install cockpit --marketplace quaresc/cockpit");
        } else {
            ok(&format!("Claude plugin cockpit {installed}"));
        }
        let update = load(&self.cache.join("update.json")).unwrap_or(Value::Null);
        let latest = if truthy(&update["latest"]) { raw(&update["latest"]) } else { String::new() };
        if !latest.is_empty() && !installed.is_empty() && latest != installed && newest(&latest, &installed) == latest {
            warn(&format!("cockpit {latest} is published → sh {}/update.sh", self.link.display()));
        }
        installed
    }

    // -----------------------------------------------------------------------------------------------------------------
    // Claude sessions: each one in herdr should beat with the installed version

    fn sessions(&self, installed: &str) {
        let dir = self.cache.join("sessions");
        prune(&dir, Duration::from_secs(5 * 60));
        let panes = self.herdr_json(&["pane", "list"]);
        let claude: Vec<&Value> = panes["result"]["panes"].as_array().into_iter().flatten().filter(|p| p["agent"] == "claude").collect();
        if claude.is_empty() {
            return;
        }
        let spaces = self.herdr_json(&["workspace", "list"]);
        let all = beats(&dir);
        for pane in claude {
            let (pane, ws) = (raw(&pane["pane_id"]), &pane["workspace_id"]);
            let label = values(&spaces["result"]["workspaces"]).filter(|w| w["workspace_id"] == *ws).map(|w| raw(&w["label"])).collect::<Vec<_>>().join("\n");
            let label = strip_index(&label);
            // A pane can hold beats from more than one instance (the conversation before a /clear on old code): the one
            // on the installed version is the session; any other still beating is a leftover.
            let beats: Vec<&Value> = all.iter().filter(|b| b["pane"] == pane.as_str()).collect();
            let current: Vec<&Value> = beats.iter().copied().filter(|b| b["version"] == installed).collect();
            let beat = max_by_at(&current).or_else(|| max_by_at(&beats));
            let version = beat.map(|b| &b["version"]).filter(|v| truthy(v)).map(raw).unwrap_or_default();
            let others = beats.iter().filter(|b| b["version"] != version.as_str()).count();
            if beat.is_none() {
                warn(&format!("session \"{label}\" ({pane}) runs no cockpit → /reload-plugins in it, or restart it"));
            } else if !installed.is_empty() && version != installed {
                warn(&format!("session \"{label}\" runs cockpit {version}, {installed} is installed → /reload-plugins in it"));
            } else if others > 0 {
                warn(&format!(
                    "session \"{label}\" on cockpit {version}, but an older cockpit still runs in its pane → restart Claude there (exit, then claude --resume)"
                ));
            } else {
                ok(&format!("session \"{label}\" on cockpit {version}"));
            }
        }
    }

    // -----------------------------------------------------------------------------------------------------------------
    // Shared caches: fresh, and without errors. The hub fetches GitHub (and Linear or CircleCI given a token) while an
    // app runs; the rest comes from a Claude session.

    fn check_cache(&self, file: &str, name: &str, limit: i64, by: &str) {
        let path = self.cache.join(file);
        if !path.is_file() {
            warn(&format!("{name}: no data yet → open the inbox space or start a Claude session in herdr; it fills within a minute"));
            return;
        }
        let data = load(&path).unwrap_or(Value::Null);
        let age = self.now - millis_to_secs(or(&data["at"], &Value::from(0)));
        let mut errors: Vec<String> = match &data["errors"] {
            Value::Array(items) => items.iter().map(joined).collect(),
            Value::Object(items) => items.values().map(joined).collect(),
            _ => Vec::new(),
        };
        if truthy(&data["error"]) {
            errors.push(joined(&data["error"]));
        }
        let errors = errors.join("; ");
        if !errors.is_empty() {
            fail(&format!("{name}: {errors}"));
        } else if age > limit {
            warn(&format!("{name}: last refresh {} min ago → {by}", age / 60));
        } else {
            ok(&format!("{name}: refreshed {} min ago", age / 60));
        }
    }

    fn caches(&self) {
        self.check_cache("inbox.json", "inbox", 900, NO_FEED);
        // Tickets come from the ticket connector, in a Claude session, unless tickets.token lets the hub fetch them.
        let inbox = self.cache.join("inbox.json");
        if self.enabled("tickets") && !self.has_token("tickets") && inbox.is_file() {
            let data = load(&inbox).unwrap_or(Value::Null);
            let fallback = if truthy(&data["hub"]) { Value::from(0) } else { data["at"].clone() };
            let at = or(&data["modAt"], or(&fallback, &Value::from(0))).clone();
            let t = self.now - millis_to_secs(&at);
            if t > 900 {
                warn(&format!("tickets: last fetched {} min ago → {SESSION} (tickets.token lets the hub fetch Linear without one)", t / 60));
            }
        }
        if self.settings.is_some() && self.set()["ci"]["enabled"] != false {
            let server = &self.set()["ci"]["server"];
            let by = if !truthy(server) || raw(server).is_empty() || self.has_token("ci") {
                NO_FEED.to_string()
            } else {
                format!("{SESSION} (ci.token lets the hub fetch CircleCI without one)")
            };
            self.check_cache("ci.json", "CI", 600, &by);
        }
        if self.enabled("calendar") {
            self.check_cache("meetings.json", "meetings", 900, SESSION);
        }
        // The monitor space is opt-in: its document is checked only when the space is pinned.
        if pinned_spaces(self.settings.as_ref()).iter().any(|s| s == "monitor") {
            self.check_cache("monitor.json", "monitor", 900, SESSION);
        }
    }

    // -----------------------------------------------------------------------------------------------------------------
    // Pinned spaces and their apps

    fn spaces(&self) {
        let listed = self.herdr_json(&["workspace", "list"]);
        let labels: Vec<String> = values(&listed["result"]["workspaces"]).map(|w| raw(&w["label"])).collect();
        let herdr_config = std::fs::read_to_string(files::home().join(".config/herdr/config.toml")).ok();
        let link = self.link.display();
        for space in pinned_spaces(self.settings.as_ref()) {
            if space.is_empty() {
                continue;
            }
            if !labels.iter().any(|l| l.ends_with(&format!("  {space}"))) {
                fail(&format!("space {space} missing → sh {link}/ensure-spaces.sh"));
                continue;
            }
            let shown = herdr_config.as_ref().is_some_and(|c| c.contains("contains = \"  \"") || c.contains(&format!("contains = \"  {space}\"")));
            if !shown {
                warn(&format!(
                    "space {space} is hidden in the sidebar → add {{ contains = \"  \" }} before the hiding rule of ui.sidebar.spaces in ~/.config/herdr/config.toml"
                ));
            }
            if !["inbox", "to-do", "runbook", "meetings", "monitor", "routines"].contains(&space.as_str()) {
                ok(&format!("space {space}"));
                continue;
            }
            let app = space.replace("to-do", "todo");
            if self.running(&format!("(^|/)cockpit space {app}$")) {
                ok(&format!("space {space}, app running"));
            } else {
                fail(&format!("space {space} has no app running → sh {link}/ensure-spaces.sh"));
            }
        }
    }

    // -----------------------------------------------------------------------------------------------------------------
    // Notifications

    fn notifications(&self) {
        if self.settings.is_none() || !is_true(or(&self.set()["notifications"], &Value::Bool(true))) {
            return;
        }
        let root = self.root.display();
        if cfg!(target_os = "macos") {
            if self.run(&["sh", &format!("{root}/notifier/build.sh")], Duration::from_secs(600)).is_some_and(|o| o.ok()) {
                let app = files::config_dir().join("Cockpit.app/Contents/MacOS/cockpit-notify");
                let status = self.output(&[&app.to_string_lossy(), "status"]);
                if status == "allowed" {
                    ok("notifications allowed");
                } else {
                    let status = if status.is_empty() { "unknown" } else { &status };
                    fail(&format!("notifications {status} → System Settings → Notifications → Cockpit → Allow"));
                }
            } else {
                fail(&format!("notifier not built → xcode-select --install, then sh {root}/notifier/build.sh"));
            }
        } else if self.which("notify-send").is_some() {
            ok("notify-send");
        } else {
            warn("notify-send missing: herdr's toasts are used instead");
        }
        let herdr_config = std::fs::read_to_string(files::home().join(".config/herdr/config.toml")).unwrap_or_default();
        let delivery = toast_delivery(&herdr_config);
        if delivery == "herdr" {
            ok("herdr toasts in-app");
        } else {
            let delivery = if delivery.is_empty() { "system" } else { &delivery };
            warn(&format!("herdr ui.toast.delivery is \"{delivery}\": notifications arrive twice → set it to \"herdr\""));
        }
    }

    // -----------------------------------------------------------------------------------------------------------------
    // terminal-browser for an agent's E2E tests beside it: optional, they run in Claude in Chrome without it

    fn terminal_browser(&self) {
        if self.which("terminal-browser").is_none() {
            ok("terminal-browser not installed: E2E tests run in Claude in Chrome (brew install --cask terminal-browser runs them beside the agent)");
            return;
        }
        let said = self.output(&["terminal-browser", "--version"]);
        ok(&format!("terminal-browser {}", said.lines().map(|l| l.split_whitespace().nth(1).unwrap_or_default()).collect::<Vec<_>>().join("\n")));
        let mut installs: Vec<PathBuf> = self
            .path
            .split(':')
            .filter(|d| !d.is_empty())
            .map(|d| Path::new(d).join("terminal-browser"))
            .filter(|p| proc::executable(p))
            .filter_map(|p| p.canonicalize().ok())
            .collect();
        installs.sort();
        installs.dedup();
        if installs.len() > 1 {
            warn(&format!(
                "{} terminal-browser installs, each with its own sign-ins → keep the brew one: rm ~/.local/bin/terminal-browser && rm -rf ~/.local/share/terminal-browser",
                installs.len()
            ));
        }
        let herdr_config = std::fs::read_to_string(files::home().join(".config/herdr/config.toml")).unwrap_or_default();
        let graphics_off = herdr_config.lines().any(|l| {
            let l = l.trim_start();
            l.strip_prefix("kitty_graphics").is_some_and(|rest| rest.trim_start().strip_prefix('=').is_some_and(|v| v.trim_start().starts_with("false")))
        });
        if graphics_off {
            fail("herdr kitty_graphics is off: terminal-browser panes stay blank → set it to true under [experimental]");
        } else {
            ok("herdr kitty graphics on");
        }
        let left = self.output(&["sh", &format!("{}/prune-browsers.sh", self.root.display()), "--count"]);
        if left.trim().parse::<i64>() == Ok(0) {
            ok("no agent-browser daemons left by closed browsers");
        } else {
            warn(&format!("{left} agent-browser daemons left by closed browsers → sh {}/prune-browsers.sh", self.link.display()));
        }
        let transport = self.output(&["terminal-browser", "config", "get", "render.transport"]);
        if transport == "auto" {
            ok("terminal-browser frame transport auto");
        } else {
            let transport = if transport.is_empty() { "unset" } else { &transport };
            warn(&format!(
                "terminal-browser render.transport is {transport}: herdr cannot hand its frames to the terminal as files → terminal-browser config set render.transport auto"
            ));
        }
        let fps = self.output(&["terminal-browser", "config", "get", "render.fps"]);
        if fps == "60" {
            ok("terminal-browser at 60 frames per second");
        } else {
            let fps = if fps.is_empty() { "display" } else { &fps };
            warn(&format!("terminal-browser render.fps is {fps}: 60 scrolls smoothly and leaves herdr room → terminal-browser config set render.fps 60"));
        }
        // herdr hands frames to Ghostty as files in /tmp, which Ghostty reads on macOS from 1.3.2 on (1.3.1 and older
        // refuse them).
        let ghostty = "/Applications/Ghostty.app/Contents/MacOS/ghostty";
        if cfg!(target_os = "macos") && proc::executable(Path::new(ghostty)) {
            let info = self.output(&[ghostty, "--version"]);
            let field = |name: &str| info.lines().filter_map(|l| l.trim_start_matches(' ').strip_prefix(name)).collect::<Vec<_>>().join("\n");
            let (version, channel) = (field("- version: "), field("- channel: "));
            let said = format!("{channel}:{version}");
            if ["tip:*", "*:1.3.[2-9]*", "*:1.[4-9].*", "*:[2-9].*"].iter().any(|p| glob(p, &said)) {
                ok(&format!("Ghostty {version} ({channel}) reads herdr's frame files"));
            } else {
                warn(&format!(
                    "Ghostty {version} ({channel}) refuses herdr's frame files, so terminal-browser panes are slow → update Ghostty to 1.4.0 or newer: Ghostty → Check for Updates…, or ghostty.org/download"
                ));
            }
        }
    }

    // -----------------------------------------------------------------------------------------------------------------
    // Connectors: their tools as /cockpit:setup mapped them, and the permission rules their background calls need

    fn connectors(&self) {
        let set = self.set();
        let config = self.config.as_ref().unwrap_or(&Value::Null);
        // The platform connector's tools, mapped to what cockpit does with deploys and previews.
        if self.enabled("platform") {
            let tools = &set["platform"]["tools"];
            let mut mapped: Vec<&str> =
                tools.as_object().into_iter().flatten().filter(|(_, v)| length(or(&v["tool"], &Value::from(""))) > 0.0).map(|(k, _)| k.as_str()).collect();
            let order = platform_tools_order(&self.settings_path);
            mapped.sort_by_key(|name| order.iter().position(|o| o == name).unwrap_or(usize::MAX));
            if mapped.is_empty() {
                warn("platform is on but no tool is mapped: no deploys or previews → /cockpit:setup maps them");
            } else {
                ok(&format!("platform tools mapped: {}", mapped.join(", ")));
            }
        }
        // The ticket connector's tools (Linear's by default).
        if truthy(&config["linear"]) && !truthy(&config["tickets"]) {
            warn("config has the old linear block: it still works as Linear's mapping → /cockpit:setup rewrites it as tickets");
        } else if self.enabled("tickets") {
            let missing = unmapped(&set["tickets"]["tools"], &["listMine", "get", "listStates", "setState"]);
            if missing.is_empty() {
                ok("ticket tools mapped");
            } else {
                warn(&format!("ticket tools not mapped: {missing}; cockpit skips what they do → /cockpit:setup maps them"));
            }
        }
        // The CI: GitHub's checks through gh, or a CI connector.
        if truthy(&config["circleci"]) && !truthy(&config["ci"]) {
            warn("config has the old circleci block: it still works as CircleCI's mapping → /cockpit:setup rewrites it as ci");
        } else if truthy(&set["ci"]["server"]) && !raw(&set["ci"]["server"]).is_empty() {
            let missing = unmapped(&set["ci"]["tools"], &["listRuns", "listJobs"]);
            if missing.is_empty() {
                ok("CI tools mapped");
            } else {
                warn(&format!("CI tools not mapped: {missing}; no CI runs → /cockpit:setup maps them"));
            }
        } else {
            ok("CI from GitHub checks");
        }

        // Permission rules for the background MCP calls.
        let claude = load(&files::home().join(".claude/settings.json"));
        let allow: Vec<&str> = claude.as_ref().map(|c| values(&c["permissions"]["allow"]).filter_map(Value::as_str).collect()).unwrap_or_default();
        for source in ["tickets", "ci", "platform", "calendar"] {
            let block = &set[source];
            if !is_true(or(&block["enabled"], &Value::Bool(false))) {
                continue;
            }
            let fallback = Value::from(if source == "tickets" { "claude.ai Linear" } else { "" });
            let server = raw(or(&block["server"], &fallback));
            if server.is_empty() {
                continue;
            }
            let rule = format!("mcp__{}", rule_name(&server));
            if claude.is_some() && allow.iter().any(|a| *a == rule || a.starts_with(&format!("{rule}__"))) {
                ok(&format!("allow rule for {server}"));
            } else {
                fail(&format!("no allow rule for {server}: background refreshes are refused → add \"{rule}\" to permissions.allow"));
            }
        }
        // The read-only tools live show panes call, and the monitor space's when it is pinned.
        let mut tools = vec![("datadog", "get_datadog_metric"), ("metabase", "query")];
        if pinned_spaces(self.settings.as_ref()).iter().any(|s| s == "monitor") {
            tools.extend([("datadog", "search_datadog_monitors"), ("datadog", "search_datadog_error_tracking_issues")]);
        }
        for (source, tool) in tools {
            let server = &set[source]["server"];
            let server = if truthy(server) && !raw(server).is_empty() { raw(server) } else { source.replacen("datadog", "claude.ai Datadog", 1) };
            let rule = format!("mcp__{}__{tool}", rule_name(&server));
            if allow.iter().any(|a| rule == *a || rule.starts_with(&format!("{a}__"))) {
                ok(&format!("allow rule for {rule}"));
            } else {
                warn(&format!("no allow rule for {rule}: its live pane asks or is refused → add it to permissions.allow"));
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Reading values the way the checks always have

fn load(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn readlink(path: &Path) -> String {
    std::fs::read_link(path).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The items of an array, or the values of an object.
fn values(value: &Value) -> Box<dyn Iterator<Item = &Value> + '_> {
    match value {
        Value::Array(items) => Box::new(items.iter()),
        Value::Object(items) => Box::new(items.values()),
        _ => Box::new(std::iter::empty()),
    }
}

/// A value printed raw: a string as is, anything else as JSON.
fn raw(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// A value joined into a list: null as nothing, a string as is, anything else as JSON.
fn joined(value: &Value) -> String {
    if value.is_null() { String::new() } else { raw(value) }
}

fn truthy(value: &Value) -> bool {
    !matches!(value, Value::Null | Value::Bool(false))
}

/// The value, else the fallback when it is null or false.
fn or<'a>(value: &'a Value, fallback: &'a Value) -> &'a Value {
    if truthy(value) { value } else { fallback }
}

fn is_true(value: &Value) -> bool {
    *value == true || *value == "true"
}

/// A string's characters, a list's items, a number's size; 0 for null and anything else.
fn length(value: &Value) -> f64 {
    match value {
        Value::String(s) => s.chars().count() as f64,
        Value::Array(items) => items.len() as f64,
        Value::Object(items) => items.len() as f64,
        Value::Number(n) => n.as_f64().unwrap_or_default().abs(),
        _ => 0.0,
    }
}

fn millis_to_secs(value: &Value) -> i64 {
    (value.as_f64().unwrap_or_default() / 1000.0).floor() as i64
}

/// The keys of the settings' `platform.tools` in the order the file writes them.
fn platform_tools_order(path: &Path) -> Vec<String> {
    #[derive(serde::Deserialize)]
    struct Settings {
        platform: Option<Platform>,
    }
    #[derive(serde::Deserialize)]
    struct Platform {
        tools: Option<Keys>,
    }
    struct Keys(Vec<String>);
    struct KeysVisitor;
    impl<'de> serde::de::Visitor<'de> for KeysVisitor {
        type Value = Keys;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("an object")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Keys, A::Error> {
            let mut keys = Vec::new();
            while let Some((key, _)) = map.next_entry::<String, serde::de::IgnoredAny>()? {
                keys.push(key);
            }
            Ok(Keys(keys))
        }
    }
    impl<'de> serde::Deserialize<'de> for Keys {
        fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            d.deserialize_map(KeysVisitor)
        }
    }
    let text = std::fs::read_to_string(path).unwrap_or_default();
    serde_json::from_str::<Settings>(&text).ok().and_then(|s| s.platform?.tools).map(|k| k.0).unwrap_or_default()
}

/// The tools of a connector's mapping with no tool named, as "a, b"; empty when there is no mapping at all.
fn unmapped(tools: &Value, wanted: &[&str]) -> String {
    let Some(tools) = tools.as_object().filter(|_| truthy(tools)) else { return String::new() };
    wanted
        .iter()
        .filter(|name| raw(or(&tools.get(**name).unwrap_or(&Value::Null)["tool"], &Value::from(""))).is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(", ")
}

/// A server's name as Claude Code writes it in a permission rule: every character outside `A-Za-z0-9_-` as `_`, runs
/// of `_` as one, without a trailing one.
fn rule_name(server: &str) -> String {
    let mut name = String::new();
    for c in server.chars() {
        let c = if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' };
        if !(c == '_' && name.ends_with('_')) {
            name.push(c);
        }
    }
    name.strip_suffix('_').map(str::to_string).unwrap_or(name)
}

/// A space label without its "[N] " position.
fn strip_index(label: &str) -> String {
    label
        .lines()
        .map(|line| {
            line.strip_prefix('[')
                .map(|rest| rest.trim_start_matches(|c: char| c.is_ascii_digit()))
                .and_then(|rest| rest.strip_prefix(']'))
                .map_or(line, |rest| rest.trim_start_matches(' '))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The pinned space names: each is "name" (a built-in, or a space in reposDir) or { name, … }.
fn pinned_spaces(settings: Option<&Value>) -> Vec<String> {
    let defaults = || SPACES.iter().map(|s| s.to_string()).collect();
    let Some(settings) = settings else { return defaults() };
    match &settings["spaces"] {
        Value::Array(spaces) => spaces.iter().map(|s| if s.is_object() { joined(&s["name"]) } else { joined(s) }).collect(),
        Value::Null | Value::Bool(false) => defaults(),
        _ => defaults(),
    }
}

/// The `delivery` of herdr's `[ui.toast]` table.
fn toast_delivery(config: &str) -> String {
    let mut inside = false;
    let mut found = Vec::new();
    for line in config.lines() {
        let in_table = if inside {
            inside = !line.starts_with('[');
            true
        } else {
            inside = line.starts_with("[ui.toast]");
            inside
        };
        if !in_table {
            continue;
        }
        let Some(rest) = line.strip_prefix("delivery") else { continue };
        let Some(rest) = rest.trim_start_matches(' ').strip_prefix('=') else { continue };
        let Some(rest) = rest.trim_start_matches(' ').strip_prefix('"') else { continue };
        if let Some(end) = rest.rfind('"') {
            found.push(format!("{}{}", &rest[..end], &rest[end + 1..]));
        }
    }
    found.join("\n")
}

/// Deletes the session beats older than `age`.
fn prune(dir: &Path, age: Duration) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            prune(&path, age);
        } else if path.extension().is_some_and(|e| e == "json")
            && entry.metadata().and_then(|m| m.modified()).is_ok_and(|at| at.elapsed().unwrap_or_default() > age)
        {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Every beat of the session files; none when one does not read as JSON.
fn beats(dir: &Path) -> Vec<Value> {
    let mut names: Vec<PathBuf> =
        std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "json")).collect();
    names.sort();
    let mut all = Vec::new();
    for name in names {
        let text = std::fs::read_to_string(&name).unwrap_or_default();
        for value in serde_json::Deserializer::from_str(&text).into_iter::<Value>() {
            let Ok(value) = value else { return Vec::new() };
            all.push(value);
        }
    }
    all
}

/// The beat with the greatest `at`, the last of equals.
fn max_by_at<'a>(beats: &[&'a Value]) -> Option<&'a Value> {
    beats.iter().copied().fold(None, |best: Option<&Value>, b| match best {
        Some(best) if order(&b["at"], &best["at"]) == Ordering::Less => Some(best),
        _ => Some(b),
    })
}

/// Values in jq's order: null, false, true, numbers, strings, arrays, objects.
fn order(a: &Value, b: &Value) -> Ordering {
    let rank = |v: &Value| match v {
        Value::Null => 0,
        Value::Bool(false) => 1,
        Value::Bool(true) => 2,
        Value::Number(_) => 3,
        Value::String(_) => 4,
        Value::Array(_) => 5,
        Value::Object(_) => 6,
    };
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64().unwrap_or_default().total_cmp(&y.as_f64().unwrap_or_default()),
        (Value::String(x), Value::String(y)) => x.cmp(y),
        (Value::Array(x), Value::Array(y)) => x.iter().zip(y).map(|(x, y)| order(x, y)).find(|o| o.is_ne()).unwrap_or(x.len().cmp(&y.len())),
        _ => rank(a).cmp(&rank(b)),
    }
}

/// The newer of two versions, by their first three numbers, then as text.
fn newest(a: &str, b: &str) -> String {
    let key = |v: &str| -> Vec<u64> {
        let mut fields = v.split('.');
        (0..3).map(|_| fields.next().map_or(0, |f| f.chars().take_while(char::is_ascii_digit).collect::<String>().parse().unwrap_or(0))).collect()
    };
    match key(a).cmp(&key(b)).then_with(|| a.cmp(b)) {
        Ordering::Greater => a.to_string(),
        _ => b.to_string(),
    }
}

/// Whether text matches a shell `case` pattern: `*`, `?` and `[…]` classes with ranges.
fn glob(pattern: &str, text: &str) -> bool {
    fn matches(p: &[char], t: &[char]) -> bool {
        match p.first() {
            None => t.is_empty(),
            Some('*') => (0..=t.len()).any(|i| matches(&p[1..], &t[i..])),
            Some('?') => !t.is_empty() && matches(&p[1..], &t[1..]),
            Some('[') => {
                let Some(close) = p.iter().skip(2).position(|c| *c == ']').map(|i| i + 2) else { return t.first() == Some(&'[') && matches(&p[1..], &t[1..]) };
                let Some(c) = t.first() else { return false };
                let (negated, class) = match p[1] {
                    '!' | '^' => (true, &p[2..close]),
                    _ => (false, &p[1..close]),
                };
                let mut hit = false;
                let mut i = 0;
                while i < class.len() {
                    if i + 2 < class.len() && class[i + 1] == '-' {
                        hit |= (class[i]..=class[i + 2]).contains(c);
                        i += 3;
                    } else {
                        hit |= class[i] == *c;
                        i += 1;
                    }
                }
                hit != negated && matches(&p[close + 1..], &t[1..])
            }
            Some(c) => t.first() == Some(c) && matches(&p[1..], &t[1..]),
        }
    }
    let (p, t): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
    matches(&p, &t)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn case_patterns_match_like_the_shell() {
        assert!(glob("0.[0-8].*", "0.8.2"));
        assert!(!glob("0.[0-8].*", "0.10.0"));
        assert!(!glob("0.[0-8].*", "0.9.1"));
        assert!(glob("*:1.3.[2-9]*", "stable:1.3.2"));
        assert!(!glob("*:1.3.[2-9]*", "stable:1.3.1"));
        assert!(glob("tip:*", "tip:1.2.0"));
        assert!(glob("*:[2-9].*", "stable:2.0.0"));
    }

    #[test]
    fn the_newest_version_by_its_numbers() {
        assert_eq!(newest("0.18.0", "0.9.3"), "0.18.0");
        assert_eq!(newest("0.17.2", "0.18.0"), "0.18.0");
        assert_eq!(newest("1.0.0", "1.0.0"), "1.0.0");
    }

    #[test]
    fn permission_rule_names() {
        assert_eq!(rule_name("claude.ai Linear"), "claude_ai_Linear");
        assert_eq!(rule_name("my  server!"), "my_server");
        assert_eq!(rule_name("a__b-c"), "a_b-c");
    }

    #[test]
    fn labels_lose_their_position() {
        assert_eq!(strip_index("[3] x  inbox"), "x  inbox");
        assert_eq!(strip_index("[] eng-4"), "eng-4");
        assert_eq!(strip_index("eng-4"), "eng-4");
    }

    #[test]
    fn the_toast_delivery_of_its_table() {
        assert_eq!(toast_delivery("[ui]\ndelivery = \"x\"\n[ui.toast]\ndelivery = \"herdr\"\n[keys]\ndelivery = \"y\"\n"), "herdr");
        assert_eq!(toast_delivery("[ui.toast]\nsound = true\n"), "");
    }

    #[test]
    fn the_latest_beat_wins_the_last_of_equals() {
        let (a, b, c) = (json!({"at": 1, "n": "a"}), json!({"at": 3, "n": "b"}), json!({"at": 3, "n": "c"}));
        assert_eq!(max_by_at(&[&a, &b, &c]).unwrap()["n"], "c");
        assert_eq!(max_by_at(&[]), None);
    }

    #[test]
    fn unmapped_tools() {
        let tools = json!({"listMine": {"tool": "x"}, "get": {"tool": ""}, "listStates": {}});
        assert_eq!(unmapped(&tools, &["listMine", "get", "listStates", "setState"]), "get, listStates, setState");
        assert_eq!(unmapped(&Value::Null, &["a"]), "");
    }
}
