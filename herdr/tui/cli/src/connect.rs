//! `cockpit agents connect|disconnect|connected`: cockpit's MCP server (`cockpit mcp`) in each installed agent's own
//! config but Claude Code's, which has the plugin. A file is backed up before it changes, and only cockpit's entry
//! is touched.

use std::path::{Path, PathBuf};

use cockpit_core::{agent, files};
use serde_json::{Value, json};

use crate::jsonedit;
use crate::proc::{self, fail};

const USAGE: &str = "cockpit agents connect [<agent>...]      cockpit's tools (show, search, changes, report_step) in each agent's MCP config
                                         (every installed agent but Claude Code, which has the plugin, when none is named)
cockpit agents disconnect [<agent>...]   takes them out again
cockpit agents connected                 which agents have them, as JSON";

/// The entry's name in every agent's config.
pub const NAME: &str = "cockpit";

/// Variables a herdr pane or a Herdr app terminal gives its programs, which tell the server where its agent sits; Codex
/// hands its servers only the ones it is told to.
const FORWARDED: [&str; 12] = [
    "HERDR_ENV",
    "HERDR_PANE_ID",
    "HERDR_TAB_ID",
    "HERDR_SOCKET_PATH",
    "HERDR_BIN_PATH",
    "HC_TERMINAL_ID",
    "HC_TERMINAL_SOCKET",
    "HC_SESSION",
    "HERDR_APP_HOME",
    "HERDR_APP_BIN",
    "COCKPIT_FRONTEND",
    "COCKPIT_HC_BIN",
];

/// How an agent's config holds an MCP server.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Shape {
    /// `mcpServers.<name>` = `{command, args}` plus these fixed members.
    Servers(&'static [(&'static str, &'static str)]),
    /// opencode's: `mcp.<name>` = `{type: "local", command: [bin, "mcp"], enabled: true}`.
    Local,
    /// `[mcp_servers.<name>]` with command and args; `env_vars` too when true (Codex).
    Toml(bool),
}

/// One agent cockpit can connect, its config location checked against the agent's own documentation.
pub struct Target {
    pub id: &'static str,
    pub name: &'static str,
    bin: &'static str,
    /// The config file under HOME, or under the variable's folder when it is set.
    file: &'static str,
    home_var: Option<(&'static str, &'static str)>,
    shape: Shape,
    /// Said once connected, when the agent needs one more step.
    after: &'static str,
}

const STDIO: &[(&str, &str)] = &[("type", "stdio")];

/// The agents whose MCP config location and shape their documentation states. Left out: Claude Code (the plugin), pi
/// (no MCP), Letta Code (servers per agent, no user file), Hermes (YAML) and Qoder (its file's shape is undocumented).
pub const TARGETS: &[Target] = &[
    Target {
        id: "codex",
        name: "Codex",
        bin: "codex",
        file: ".codex/config.toml",
        home_var: Some(("CODEX_HOME", "config.toml")),
        shape: Shape::Toml(true),
        after: "",
    },
    Target { id: "opencode", name: "opencode", bin: "opencode", file: ".config/opencode/opencode.json", home_var: None, shape: Shape::Local, after: "" },
    Target { id: "kilo", name: "Kilo", bin: "kilo", file: ".config/kilo/kilo.jsonc", home_var: None, shape: Shape::Local, after: "" },
    Target {
        id: "copilot",
        name: "GitHub Copilot CLI",
        bin: "copilot",
        file: ".copilot/mcp-config.json",
        home_var: Some(("COPILOT_HOME", "mcp-config.json")),
        shape: Shape::Servers(&[("type", "local"), ("tools", "*")]),
        after: "",
    },
    Target { id: "droid", name: "Droid", bin: "droid", file: ".factory/mcp.json", home_var: None, shape: Shape::Servers(STDIO), after: "" },
    Target {
        id: "kimi",
        name: "Kimi Code",
        bin: "kimi",
        file: ".kimi-code/mcp.json",
        home_var: Some(("KIMI_CODE_HOME", "mcp.json")),
        shape: Shape::Servers(&[]),
        after: "",
    },
    Target { id: "qwen", name: "Qwen Code", bin: "qwen", file: ".qwen/settings.json", home_var: None, shape: Shape::Servers(&[]), after: "" },
    Target { id: "gemini", name: "Gemini CLI", bin: "gemini", file: ".gemini/settings.json", home_var: None, shape: Shape::Servers(&[]), after: "" },
    Target {
        id: "cursor",
        name: "Cursor CLI",
        bin: "cursor-agent",
        file: ".cursor/mcp.json",
        home_var: None,
        shape: Shape::Servers(STDIO),
        after: "approve it once with `cursor-agent mcp enable cockpit`",
    },
    Target {
        id: "omp",
        name: "oh-my-pi",
        bin: "omp",
        file: ".omp/agent/mcp.json",
        home_var: Some(("PI_CODING_AGENT_DIR", "mcp.json")),
        shape: Shape::Servers(&[("type", "stdio"), ("enabled", "true")]),
        after: "",
    },
    Target {
        id: "devin",
        name: "Devin for Terminal",
        bin: "devin",
        file: ".config/devin/mcp_config.json",
        home_var: None,
        shape: Shape::Servers(&[]),
        after: "",
    },
    Target { id: "mastracode", name: "Mastra Code", bin: "mastracode", file: ".mastracode/mcp.json", home_var: None, shape: Shape::Servers(&[]), after: "" },
    Target {
        id: "antigravity-cli",
        name: "Antigravity CLI",
        bin: "agy",
        file: ".gemini/config/mcp_config.json",
        home_var: None,
        shape: Shape::Servers(&[]),
        after: "",
    },
    Target { id: "grok", name: "Grok Build", bin: "grok", file: ".grok/config.toml", home_var: None, shape: Shape::Toml(false), after: "" },
];

pub fn target(id: &str) -> Option<&'static Target> {
    let id = agent::get(id).map_or(id.to_string(), |a| a.id);
    TARGETS.iter().find(|t| t.id == id)
}

impl Target {
    pub fn path(&self) -> PathBuf {
        let set = self.home_var.and_then(|(var, file)| std::env::var(var).ok().filter(|d| !d.is_empty()).map(|d| PathBuf::from(d).join(file)));
        set.unwrap_or_else(|| {
            let default = files::home().join(self.file);
            // Kilo reads kilo.json too; opencode opencode.jsonc: the one that exists is the one to edit.
            let other = match default.extension().and_then(|e| e.to_str()) {
                Some("jsonc") => default.with_extension("json"),
                Some("json") if self.shape == Shape::Local => default.with_extension("jsonc"),
                _ => return default,
            };
            if !default.exists() && other.exists() { other } else { default }
        })
    }

    pub fn installed(&self) -> bool {
        agent::get(self.id).is_some_and(|a| a.installed()) || proc::which(self.bin).is_some()
    }

    /// What the config holds for cockpit's server, in this agent's terms.
    fn entry(&self, bin: &str) -> Value {
        match self.shape {
            Shape::Local => json!({"type": "local", "command": [bin, "mcp"], "enabled": true}),
            Shape::Servers(extra) => {
                let mut entry = serde_json::Map::new();
                for (key, value) in extra {
                    let value = match *value {
                        "true" => json!(true),
                        "*" => json!(["*"]),
                        text => json!(text),
                    };
                    entry.insert(key.to_string(), value);
                }
                entry.insert("command".into(), bin.into());
                entry.insert("args".into(), json!(["mcp"]));
                Value::Object(entry)
            }
            Shape::Toml(_) => json!({"command": bin, "args": ["mcp"]}),
        }
    }

    fn json_path(&self) -> &'static str {
        if self.shape == Shape::Local { "mcp" } else { "mcpServers" }
    }

    /// cockpit's entry as the config holds it now; Err when the file cannot be read as the agent's format.
    pub fn current(&self) -> Result<Option<Value>, String> {
        let text = std::fs::read_to_string(self.path()).unwrap_or_default();
        match self.shape {
            Shape::Toml(_) => {
                let doc: toml_edit::DocumentMut = text.parse().map_err(|e: toml_edit::TomlError| e.to_string())?;
                let Some(server) = doc.get("mcp_servers").and_then(|s| s.get(NAME)) else { return Ok(None) };
                let command = server.get("command").and_then(|c| c.as_str()).unwrap_or_default();
                let args: Vec<&str> = server.get("args").and_then(|a| a.as_array()).map(|a| a.iter().filter_map(|v| v.as_str()).collect()).unwrap_or_default();
                Ok(Some(json!({"command": command, "args": args})))
            }
            _ => jsonedit::get(&text, &[self.json_path()], NAME),
        }
    }

    /// The config's text with cockpit's server in it.
    fn with(&self, text: &str, bin: &str) -> Result<String, String> {
        match self.shape {
            Shape::Toml(forward) => {
                let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e: toml_edit::TomlError| e.to_string())?;
                let servers = doc.entry("mcp_servers").or_insert_with(|| {
                    let mut table = toml_edit::Table::new();
                    table.set_implicit(true);
                    toml_edit::Item::Table(table)
                });
                let servers = servers.as_table_like_mut().ok_or("mcp_servers is not a table")?;
                let mut server = toml_edit::Table::new();
                server.insert("command", toml_edit::value(bin));
                server.insert("args", toml_edit::value(toml_edit::Array::from_iter(["mcp"])));
                if forward {
                    server.insert("env_vars", toml_edit::value(toml_edit::Array::from_iter(FORWARDED)));
                }
                servers.insert(NAME, toml_edit::Item::Table(server));
                Ok(doc.to_string())
            }
            _ => jsonedit::set(text, &[self.json_path()], NAME, &self.entry(bin)),
        }
    }

    /// The config's text without cockpit's server; None when it has none.
    fn without(&self, text: &str) -> Result<Option<String>, String> {
        match self.shape {
            Shape::Toml(_) => {
                let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e: toml_edit::TomlError| e.to_string())?;
                let Some(servers) = doc.get_mut("mcp_servers").and_then(|s| s.as_table_like_mut()) else { return Ok(None) };
                if servers.remove(NAME).is_none() {
                    return Ok(None);
                }
                if servers.is_empty() {
                    doc.remove("mcp_servers");
                }
                Ok(Some(doc.to_string()))
            }
            _ => {
                let Some(next) = jsonedit::remove(text, &[self.json_path()], NAME)? else { return Ok(None) };
                // The servers' object cockpit's entry leaves empty goes with it.
                let empty = jsonedit::get(&next, &[], self.json_path())?.is_some_and(|v| v == json!({}));
                Ok(Some(if empty { jsonedit::remove(&next, &[], self.json_path())?.unwrap_or(next) } else { next }))
            }
        }
    }

    /// Whether the config holds cockpit's server on this binary, or the agent's cockpit plugin.
    pub fn is_connected(&self, bin: &str) -> bool {
        self.has_plugin() || self.current().ok().flatten().is_some_and(|e| self.same(&e, bin))
    }

    /// Whether Codex has cockpit's plugin on, which brings the server itself.
    pub fn has_plugin(&self) -> bool {
        let text = std::fs::read_to_string(self.path()).unwrap_or_default();
        let doc = text.parse::<toml_edit::DocumentMut>();
        self.id == "codex"
            && doc.is_ok_and(|d| {
                d.get("plugins").and_then(|p| p.get("cockpit@cockpit")).is_some_and(|p| p.get("enabled").and_then(|e| e.as_bool()) != Some(false))
            })
    }

    fn same(&self, entry: &Value, bin: &str) -> bool {
        match self.shape {
            Shape::Toml(_) => entry["command"] == bin && entry["args"] == json!(["mcp"]),
            _ => *entry == self.entry(bin),
        }
    }

    /// Adds cockpit's server, or puts it on this binary; the line says what was done.
    pub fn connect(&self, bin: &str) -> Result<String, String> {
        let path = self.path();
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        // Codex's cockpit plugin brings the same server: a second one under the same name would clash.
        if self.has_plugin() {
            return Ok(format!("{} has cockpit's tools from its cockpit plugin ({})", self.name, tilde(&path)));
        }
        let current = self.current().map_err(|e| format!("{} does not read as the agent's config ({e}); left as it is", tilde(&path)))?;
        if current.as_ref().is_some_and(|e| self.same(e, bin)) {
            return Ok(format!("{} already has cockpit's tools ({})", self.name, tilde(&path)));
        }
        let next = self.with(&text, bin)?;
        let backup = write(&path, &text, &next)?;
        let after = if self.after.is_empty() { String::new() } else { format!("; {}", self.after) };
        Ok(format!("{} has cockpit's tools in {}{}{after}", self.name, tilde(&path), backup.map(|b| format!(" (backup {})", tilde(&b))).unwrap_or_default()))
    }

    pub fn disconnect(&self) -> Result<String, String> {
        let path = self.path();
        let Ok(text) = std::fs::read_to_string(&path) else { return Ok(format!("{} has no config at {}", self.name, tilde(&path))) };
        match self.without(&text).map_err(|e| format!("{} does not read as the agent's config ({e}); left as it is", tilde(&path)))? {
            None => Ok(format!("{} has no cockpit server in {}", self.name, tilde(&path))),
            Some(next) => {
                let backup = write(&path, &text, &next)?;
                Ok(format!(
                    "{}: cockpit's tools taken out of {}{}",
                    self.name,
                    tilde(&path),
                    backup.map(|b| format!(" (backup {})", tilde(&b))).unwrap_or_default()
                ))
            }
        }
    }
}

/// Writes the config through a temporary file, after copying what it held beside it; the backup's path.
fn write(path: &Path, before: &str, after: &str) -> Result<Option<PathBuf>, String> {
    let err = |e: std::io::Error| format!("{}: {e}", tilde(path));
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(err)?;
    }
    // The backup and the new file keep the config's mode: one may hold the agent's own tokens.
    let mode = std::fs::metadata(path).ok().map(|m| m.permissions());
    let backup = if let Some(mode) = &mode {
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let name = format!("{}.bak-cockpit-{stamp}", path.file_name().unwrap_or_default().to_string_lossy());
        let backup = path.with_file_name(name);
        std::fs::write(&backup, before).map_err(err)?;
        std::fs::set_permissions(&backup, mode.clone()).map_err(err)?;
        Some(backup)
    } else {
        None
    };
    let tmp = path.with_file_name(format!(".{}.cockpit-tmp", path.file_name().unwrap_or_default().to_string_lossy()));
    std::fs::write(&tmp, after).map_err(err)?;
    if let Some(mode) = mode {
        let _ = std::fs::set_permissions(&tmp, mode);
    }
    std::fs::rename(&tmp, path).map_err(err)?;
    Ok(backup)
}

fn tilde(path: &Path) -> String {
    let home = files::home();
    path.strip_prefix(&home).map_or(path.display().to_string(), |rest| format!("~/{}", rest.display()))
}

/// The cockpit binary agents start: through the link that follows updates when it names this one.
pub fn binary() -> String {
    let exe = std::env::current_exe().unwrap_or_default();
    let real = exe.canonicalize().unwrap_or(exe.clone());
    let stable = [files::config_dir().join("herdr/bin/cockpit"), files::home().join(".local/bin/cockpit")];
    stable.into_iter().find(|p| p.canonicalize().is_ok_and(|c| c == real)).unwrap_or(exe).to_string_lossy().into_owned()
}

/// Every target with whether its agent is installed and whether it holds cockpit's server.
pub fn status(bin: &str) -> Value {
    let rows: Vec<Value> = TARGETS
        .iter()
        .map(|t| {
            let current = t.current();
            json!({
                "id": t.id,
                "name": t.name,
                "installed": t.installed(),
                "config": t.path(),
                "connected": t.has_plugin() || current.as_ref().ok().and_then(|c| c.as_ref()).is_some_and(|e| t.same(e, bin)),
                "entry": current.as_ref().ok().cloned().flatten(),
                "error": current.err(),
            })
        })
        .collect();
    Value::Array(rows)
}

/// The targets named, or every installed one; an unknown name fails.
fn chosen(ids: &[String], all: impl Fn(&Target) -> bool) -> Vec<&'static Target> {
    if ids.is_empty() {
        return TARGETS.iter().filter(|t| all(t)).collect();
    }
    ids.iter()
        .map(|id| {
            if agent::get(id).is_some_and(|a| a.id == "claude") {
                fail("Claude Code gets cockpit's tools from its plugin: /plugin install cockpit --marketplace quaresc/cockpit");
            }
            target(id).unwrap_or_else(|| fail(&format!("cockpit cannot connect {id}: its MCP config is not documented where cockpit can write it\n{USAGE}")))
        })
        .collect()
}

pub fn main(args: &[String]) {
    let Some((command, ids)) = args.split_first() else { fail(USAGE) };
    let bin = binary();
    let mut failed = false;
    match command.as_str() {
        "connect" => {
            let targets = chosen(ids, Target::installed);
            if targets.is_empty() {
                println!("no agent cockpit can connect is installed");
            }
            for t in targets {
                match t.connect(&bin) {
                    Ok(line) => println!("{line}"),
                    Err(e) => {
                        failed = true;
                        eprintln!("{}: {e}", t.name);
                    }
                }
            }
        }
        "disconnect" => {
            for t in chosen(ids, |t| t.current().ok().flatten().is_some()) {
                match t.disconnect() {
                    Ok(line) => println!("{line}"),
                    Err(e) => {
                        failed = true;
                        eprintln!("{}: {e}", t.name);
                    }
                }
            }
        }
        "connected" => println!("{}", serde_json::to_string_pretty(&status(&bin)).unwrap_or_default()),
        _ => fail(USAGE),
    }
    if failed {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn by(id: &str) -> &'static Target {
        TARGETS.iter().find(|t| t.id == id).unwrap()
    }

    #[test]
    fn each_agent_gets_its_documented_shape() {
        assert_eq!(by("opencode").entry("/c"), json!({"type": "local", "command": ["/c", "mcp"], "enabled": true}));
        assert_eq!(by("copilot").entry("/c"), json!({"type": "local", "tools": ["*"], "command": "/c", "args": ["mcp"]}));
        assert_eq!(by("cursor").entry("/c"), json!({"type": "stdio", "command": "/c", "args": ["mcp"]}));
        assert_eq!(by("omp").entry("/c"), json!({"type": "stdio", "enabled": true, "command": "/c", "args": ["mcp"]}));
        assert_eq!(by("qwen").entry("/c"), json!({"command": "/c", "args": ["mcp"]}));
    }

    #[test]
    fn codex_keeps_the_rest_of_its_config() {
        let text = "model = \"o3\" # mine\n\n[mcp_servers.docs]\ncommand = \"docs\"\n";
        let codex = by("codex");
        let done = codex.with(text, "/c").unwrap();
        assert!(done.starts_with("model = \"o3\" # mine\n\n[mcp_servers.docs]\ncommand = \"docs\"\n"), "{done}");
        assert!(done.contains("[mcp_servers.cockpit]\ncommand = \"/c\"\nargs = [\"mcp\"]\nenv_vars = [\"HERDR_ENV\""), "{done}");
        assert_eq!(codex.without(&done).unwrap().unwrap(), text);
        assert_eq!(codex.without(text).unwrap(), None);
        let grok = by("grok").with("", "/c").unwrap();
        assert_eq!(grok, "[mcp_servers.cockpit]\ncommand = \"/c\"\nargs = [\"mcp\"]\n");
        assert_eq!(by("grok").without(&grok).unwrap().unwrap(), "");
    }
}
