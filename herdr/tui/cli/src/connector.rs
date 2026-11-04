//! `cockpit connector`: an MCP server called by cockpit itself, its login, its tools and my tickets through it.

use cockpit_connectors::{Server, block_on, client, connect, tickets};
use serde_json::Value;

use crate::proc;

const USAGE: &str = "usage: cockpit connector login <url or servers key> | logout <url> | tools <server> | call <server> <tool> [json args] | tickets <server>
       | rows <source json> (a show block's Metabase or Datadog source, as {columns, rows})
       | map <tickets|ci|calendar|platform> <servers key> (your default agent maps the server's tools into config.json)
<server> is a servers key of config.json, an MCP server's URL, or the command that starts it";

pub fn main(args: &[String]) {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = match args.as_slice() {
        ["login", spec] => {
            let url = configured_url(spec);
            block_on(client::login(&url, |link| {
                println!("Opening the login page; if it does not open:\n{link}");
                let _ = std::process::Command::new(if cfg!(target_os = "macos") { "open" } else { "xdg-open" }).arg(link).status();
            }))
            .map(|()| format!("Logged in to {url}"))
        }
        ["logout", url] => {
            cockpit_connectors::credentials::Store::new(url).forget();
            Ok(format!("Forgot the login to {url}"))
        }
        ["tools", server] => block_on(async {
            let c = connect(&resolve(server)).await?;
            let tools = c.tools().await;
            c.close().await;
            Ok(tools?.into_iter().map(|(name, about)| format!("{name}  {}", about.lines().next().unwrap_or(""))).collect::<Vec<_>>().join("\n"))
        }),
        ["call", server, tool, rest @ ..] => block_on(async {
            let args: Value = serde_json::from_str(rest.first().copied().unwrap_or("{}")).map_err(|e| format!("arguments: {e}"))?;
            let c = connect(&resolve(server)).await?;
            let answer = c.call(tool, args.as_object().cloned().unwrap_or_default()).await;
            c.close().await;
            Ok(serde_json::to_string_pretty(&answer?).unwrap_or_default())
        }),
        ["tickets", server] => block_on(async {
            let config = cockpit_core::files::load(&cockpit_core::files::config_dir().join("config.json")).unwrap_or(Value::Null);
            let mapping = tickets::mapping(&config["tickets"]);
            let c = connect(&resolve(server)).await?;
            let mine = tickets::mine(&cockpit_connectors::tools::Mapped { connection: &c, section: &mapping }, &mapping).await;
            c.close().await;
            Ok(serde_json::to_string_pretty(&mine?).unwrap_or_default())
        }),
        ["rows", source] => rows(source),
        ["map", section, key] => map(section, key),
        _ => Err(USAGE.to_string()),
    };
    match out {
        Ok(text) => println!("{text}"),
        Err(why) => proc::fail(&why),
    }
}

/// The server cockpit reaches for a section or a show source by its config name: `servers.<name>`, ready to call.
pub fn configured(name: &str) -> Option<Server> {
    let raw = cockpit_core::files::load(&cockpit_core::files::config_dir().join("config.json")).unwrap_or(Value::Null);
    Server::from_entry(&raw["servers"][name]).filter(Server::is_ready)
}

/// A show block's source as rows, through the server the config names for it.
fn rows(source: &str) -> Result<String, String> {
    use cockpit_connectors::sources;
    let source: Value = serde_json::from_str(source).map_err(|e| format!("source: {e}"))?;
    let config = cockpit_core::files::load(&cockpit_core::files::config_dir().join("config.json")).unwrap_or(Value::Null);
    let name = sources::server(&source, &config);
    let server = configured(&name).ok_or_else(|| format!("no server of cockpit's own for {name} (servers.{name} in config.json)"))?;
    block_on(async {
        let c = connect(&server).await?;
        let rows = sources::rows(&cockpit_connectors::tools::Mapped { connection: &c, section: &Value::Null }, &source, &config).await;
        c.close().await;
        Ok(rows?.to_string())
    })
}

/// Maps a configured server's tools for a section with the default agent, and writes the section.
fn map(section: &str, key: &str) -> Result<String, String> {
    let path = cockpit_core::files::config_dir().join("config.json");
    let raw = cockpit_core::files::load(&path).unwrap_or(Value::Null);
    let server = Server::from_entry(&raw["servers"][key]).ok_or_else(|| format!("no servers.{key} in config.json"))?;
    let name = raw[section]["name"].as_str().filter(|n| !n.is_empty()).unwrap_or(key).to_string();
    let (mapping, sample) = crate::mapper::map(section, &server, &name, &cockpit_core::agent::default())?;
    let mut text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    for (k, v) in mapping.as_object().into_iter().flatten() {
        text = crate::jsonedit::set(&text, &[section], k, v)?;
    }
    for (k, v) in [("server", Value::from(key)), ("name", Value::from(name)), ("enabled", Value::Bool(true))] {
        text = crate::jsonedit::set(&text, &[section], k, &v)?;
    }
    std::fs::write(&path, text).map_err(|e| e.to_string())?;
    Ok(format!("Mapped {key} for {section}: {sample}"))
}

/// A server's URL from its `servers` key, or the spec as given.
fn configured_url(spec: &str) -> String {
    let raw = cockpit_core::files::load(&cockpit_core::files::config_dir().join("config.json")).unwrap_or(Value::Null);
    raw["servers"][spec]["url"].as_str().map(str::to_string).unwrap_or_else(|| spec.to_string())
}

/// A server by its `servers` key, else by its URL or command.
fn resolve(spec: &str) -> Server {
    let raw = cockpit_core::files::load(&cockpit_core::files::config_dir().join("config.json")).unwrap_or(Value::Null);
    Server::from_entry(&raw["servers"][spec]).unwrap_or_else(|| Server::parse(spec))
}
