//! One MCP server, reached over HTTP (with its OAuth login) or started as a local command.

use rmcp::model::{CallToolRequestParams, ClientCapabilities, ClientConfig, Implementation, ProtocolVersion};
use rmcp::service::RunningService;
use rmcp::transport::auth::{AuthClient, AuthorizationManager, AuthorizationRequest, OAuthClientConfig, OAuthState};
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{StreamableHttpClientTransport, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Map, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::credentials::Store;

/// Where a server is: a URL logged in to through OAuth, a URL called with headers (an API key, or a bearer token a
/// command prints, as `gh auth token`), or a command, its arguments and the variables it needs.
#[derive(Clone, Debug, PartialEq)]
pub enum Server {
    Http(String),
    /// A URL whose authorization server only knows apps registered with it beforehand (Google's): the person's own
    /// client id, secret and scopes.
    App {
        url: String,
        client_id: String,
        client_secret: String,
        scopes: Vec<String>,
    },
    Keyed {
        url: String,
        headers: Vec<(String, String)>,
        bearer_command: Vec<String>,
    },
    Command {
        argv: Vec<String>,
        env: Vec<(String, String)>,
    },
}

impl Server {
    pub fn parse(spec: &str) -> Server {
        if spec.starts_with("http://") || spec.starts_with("https://") {
            Server::Http(spec.to_string())
        } else {
            Server::Command { argv: spec.split_whitespace().map(str::to_string).collect(), env: Vec::new() }
        }
    }

    /// A `servers` entry of the config: `{"url"}` (OAuth), `{"url", "headers"|"bearerCommand"}`, or
    /// `{"command", "args", "env"}`.
    pub fn from_entry(entry: &Value) -> Option<Server> {
        let strings = |v: &Value| v.as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect::<Vec<_>>();
        let pairs = |v: &Value| v.as_object().into_iter().flatten().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect::<Vec<_>>();
        if let Some(url) = entry["url"].as_str().filter(|u| !u.is_empty())
            && let Some(client_id) = entry["clientId"].as_str().filter(|c| !c.is_empty())
        {
            let client_secret = entry["clientSecret"].as_str().unwrap_or_default().to_string();
            return Some(Server::App { url: url.to_string(), client_id: client_id.to_string(), client_secret, scopes: strings(&entry["scopes"]) });
        }
        if let Some(url) = entry["url"].as_str().filter(|u| !u.is_empty()) {
            let (headers, bearer_command) = (pairs(&entry["headers"]), strings(&entry["bearerCommand"]));
            if headers.is_empty() && bearer_command.is_empty() {
                return Some(Server::Http(url.to_string()));
            }
            return Some(Server::Keyed { url: url.to_string(), headers, bearer_command });
        }
        let command = entry["command"].as_str().filter(|c| !c.is_empty())?;
        Some(Server::Command { argv: std::iter::once(command.to_string()).chain(strings(&entry["args"])).collect(), env: pairs(&entry["env"]) })
    }

    /// Ready to call: a command always, a URL once cockpit has a login for it.
    pub fn is_ready(&self) -> bool {
        match self {
            Server::Http(url) | Server::App { url, .. } => Store::new(url).has_login(),
            Server::Keyed { .. } | Server::Command { .. } => true,
        }
    }

    /// Its address or command, as setup and doctor say it.
    pub fn describe(&self) -> String {
        match self {
            Server::Http(url) | Server::App { url, .. } | Server::Keyed { url, .. } => url.clone(),
            Server::Command { argv, .. } => argv.join(" "),
        }
    }

    /// The URL a browser login is for, when the server takes one.
    pub fn login_url(&self) -> Option<&str> {
        match self {
            Server::Http(url) | Server::App { url, .. } => Some(url),
            _ => None,
        }
    }
}

/// The token a command printed, its last line that could be a header value: `gh` behind a mise or asdf shim prints the
/// tool it resolved first, and a header value holds no newline.
pub fn printed_token(stdout: &str) -> Option<String> {
    stdout.lines().map(str::trim).rfind(|line| !line.is_empty() && line.bytes().all(|b| b.is_ascii_graphic())).map(str::to_string)
}

pub struct Connection(RunningService<RoleClient, ClientConfig>);

/// cockpit as an MCP client, opening with the newest protocol version that still has an `initialize` handshake: rmcp's
/// latest has none, and servers such as Google's refuse a handshake under it.
fn handler() -> ClientConfig {
    ClientConfig::new(ClientCapabilities::default(), Implementation::new("cockpit", env!("CARGO_PKG_VERSION")))
        .with_protocol_version(ProtocolVersion::LATEST_WITH_INITIALIZE)
}

/// Logs in to a server through the browser: discovery, registration (or the person's own app), PKCE, and a callback
/// on a local port.
pub async fn login_server(server: &Server, open: impl Fn(&str)) -> Result<(), String> {
    match server {
        Server::App { url, client_id, client_secret, scopes } => login_as(url, Some((client_id, client_secret, scopes)), open).await,
        Server::Http(url) => login_as(url, None, open).await,
        _ => Err("this server takes no browser login".into()),
    }
}

pub async fn login(url: &str, open: impl Fn(&str)) -> Result<(), String> {
    login_as(url, None, open).await
}

async fn login_as(url: &str, app: Option<(&String, &String, &Vec<String>)>, open: impl Fn(&str)) -> Result<(), String> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.map_err(|e| e.to_string())?;
    let redirect = format!("http://127.0.0.1:{}/callback", listener.local_addr().map_err(|e| e.to_string())?.port());
    let mut manager = AuthorizationManager::new(url).await.map_err(|e| format!("{url}: {e}"))?;
    manager.set_credential_store(Store::new(url));
    let mut state = OAuthState::Unauthorized(manager);
    let mut request = AuthorizationRequest::new(&redirect).with_client_name("cockpit");
    if let Some((client_id, secret, scopes)) = app {
        request = request.with_preregistered_client(client_id.as_str());
        if !secret.is_empty() {
            request = request.with_client_secret(secret.as_str());
        }
        if !scopes.is_empty() {
            request = request.with_scopes(scopes.iter().map(String::as_str));
        }
    }
    state.start_authorization(request).await.map_err(|e| format!("{url} refused cockpit's login: {e}"))?;
    let auth_url = state.get_authorization_url().await.map_err(|e| e.to_string())?;
    open(&auth_url);
    let callback = tokio::time::timeout(std::time::Duration::from_secs(300), callback(&listener, &redirect))
        .await
        .map_err(|_| "no answer from the browser in 5 minutes".to_string())??;
    state.handle_callback_url(&callback).await.map_err(|e| format!("login not completed: {e}"))?;
    Ok(())
}

/// The browser's one request to the redirect URL, answered with a page saying it can be closed.
async fn callback(listener: &tokio::net::TcpListener, redirect: &str) -> Result<String, String> {
    loop {
        let (mut socket, _) = listener.accept().await.map_err(|e| e.to_string())?;
        let mut buf = vec![0u8; 8192];
        let n = socket.read(&mut buf).await.map_err(|e| e.to_string())?;
        let head = String::from_utf8_lossy(&buf[..n]);
        let Some(path) = head.lines().next().and_then(|l| l.split_whitespace().nth(1)) else { continue };
        if !path.starts_with("/callback") {
            let _ = socket.write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\r\n").await;
            continue;
        }
        let body = "<html><body style=\"font-family:system-ui;padding:3em\">cockpit is logged in. You can close this tab.</body></html>";
        let reply = format!("HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\n\r\n{body}", body.len());
        let _ = socket.write_all(reply.as_bytes()).await;
        let base = redirect.trim_end_matches("/callback");
        return Ok(format!("{base}{path}"));
    }
}

pub async fn connect(server: &Server) -> Result<Connection, String> {
    let service = match server {
        Server::Http(url) => {
            let store = Store::new(url);
            let config = StreamableHttpClientTransportConfig::with_uri(url.as_str());
            if store.has_login() {
                let mut manager = AuthorizationManager::new(url.as_str()).await.map_err(|e| format!("{url}: {e}"))?;
                manager.set_credential_store(store);
                manager.initialize_from_store().await.map_err(|e| format!("{url}: stored login unusable ({e}) → cockpit connector login {url}"))?;
                let client = AuthClient::new(reqwest::Client::default(), manager);
                handler().serve(StreamableHttpClientTransport::with_client(client, config)).await.map_err(|e| format!("{url}: {e}"))?
            } else {
                let transport = StreamableHttpClientTransport::with_client(reqwest::Client::default(), config);
                handler().serve(transport).await.map_err(|e| format!("{url}: {e} (a server that needs a login: cockpit connector login {url})"))?
            }
        }
        Server::App { url, client_id, client_secret, .. } => {
            let mut manager = AuthorizationManager::new(url.as_str()).await.map_err(|e| format!("{url}: {e}"))?;
            manager.set_credential_store(Store::new(url));
            let known = manager.initialize_from_store().await.map_err(|e| format!("{url}: stored login unusable ({e}) → cockpit connector login {url}"))?;
            if !known {
                return Err(format!("cockpit is not logged in to {url} → cockpit connector login {url}"));
            }
            // A refresh presents the app's secret, as the person's own app is a confidential one.
            let mut app = OAuthClientConfig::new(client_id.as_str(), "http://127.0.0.1/callback");
            if !client_secret.is_empty() {
                app = app.with_client_secret(client_secret.as_str());
            }
            manager.configure_client(app).map_err(|e| format!("{url}: {e}"))?;
            let client = AuthClient::new(reqwest::Client::default(), manager);
            handler()
                .serve(StreamableHttpClientTransport::with_client(client, StreamableHttpClientTransportConfig::with_uri(url.as_str())))
                .await
                .map_err(|e| format!("{url}: {e}"))?
        }
        Server::Keyed { url, headers, bearer_command } => {
            let mut config = StreamableHttpClientTransportConfig::with_uri(url.as_str());
            if let Some((program, args)) = bearer_command.split_first() {
                let out = tokio::process::Command::new(program).args(args).output().await.map_err(|e| format!("{program}: {e}"))?;
                let token = out.status.success().then(|| printed_token(&String::from_utf8_lossy(&out.stdout))).flatten();
                let Some(token) = token else {
                    return Err(format!("{} printed no token", bearer_command.join(" ")));
                };
                config = config.auth_header(token);
            }
            let mut custom = std::collections::HashMap::new();
            for (name, value) in headers {
                let name = reqwest::header::HeaderName::from_bytes(name.as_bytes()).map_err(|e| format!("header {name}: {e}"))?;
                let value = reqwest::header::HeaderValue::from_str(&expand_env(value)).map_err(|e| format!("header {name}: {e}"))?;
                custom.insert(name, value);
            }
            let transport = StreamableHttpClientTransport::with_client(reqwest::Client::default(), config.custom_headers(custom));
            handler().serve(transport).await.map_err(|e| format!("{url}: {e}"))?
        }
        Server::Command { argv, env } => {
            let (program, args) = argv.split_first().ok_or("no command")?;
            let mut command = tokio::process::Command::new(program);
            command.args(args).envs(env.iter().map(|(k, v)| (k, expand_env(v))));
            let transport = TokioChildProcess::new(command).map_err(|e| format!("{program}: {e}"))?;
            handler().serve(transport).await.map_err(|e| format!("{program}: {e}"))?
        }
    };
    Ok(Connection(service))
}

impl Connection {
    pub async fn tools(&self) -> Result<Vec<(String, String)>, String> {
        let tools = self.0.list_all_tools().await.map_err(|e| e.to_string())?;
        Ok(tools.into_iter().map(|t| (t.name.to_string(), t.description.map(|d| d.to_string()).unwrap_or_default())).collect())
    }

    /// Every tool with its description and input schema, as a mapping prompt shows them.
    pub async fn tool_list(&self) -> Result<Value, String> {
        let tools = self.0.list_all_tools().await.map_err(|e| e.to_string())?;
        Ok(Value::Array(
            tools
                .into_iter()
                .map(|t| serde_json::json!({"name": t.name, "description": t.description, "inputSchema": Value::Object((*t.input_schema).clone())}))
                .collect(),
        ))
    }

    /// One call's first text block as it is, for servers that answer in text (Datadog's `<JSON_DATA>`, YAML).
    pub async fn call_text(&self, tool: &str, args: Map<String, Value>) -> Result<String, String> {
        let result = self.0.call_tool(CallToolRequestParams::new(tool.to_string()).with_arguments(args)).await.map_err(|e| format!("{tool}: {e}"))?;
        let body = result.content.iter().find_map(|c| c.as_text().map(|t| t.text.clone())).unwrap_or_default();
        if result.is_error == Some(true) {
            return Err(format!("{tool}: {}", body.chars().take(200).collect::<String>()));
        }
        Ok(body)
    }

    /// One call, its first text block parsed as JSON (or kept as text when it is none).
    pub async fn call(&self, tool: &str, args: Map<String, Value>) -> Result<Value, String> {
        let result = self.0.call_tool(CallToolRequestParams::new(tool.to_string()).with_arguments(args)).await.map_err(|e| format!("{tool}: {e}"))?;
        if let Some(structured) = result.structured_content.clone() {
            return Ok(structured);
        }
        let body = result.content.iter().find_map(|c| c.as_text().map(|t| t.text.clone()));
        let body = body.unwrap_or_default();
        if result.is_error == Some(true) {
            return Err(format!("{tool}: {}", body.chars().take(200).collect::<String>()));
        }
        if body.is_empty() {
            return Ok(Value::Object(Map::new()));
        }
        serde_json::from_str(&body).map_err(|_| format!("{tool} answered no JSON: {}", body.chars().take(200).collect::<String>()))
    }

    pub async fn close(self) {
        let _ = self.0.cancel().await;
    }
}

/// A value with its `${VAR}` references filled from the environment, as the agents' configs write API keys.
fn expand_env(value: &str) -> String {
    let mut out = String::new();
    let mut rest = value;
    while let Some(open) = rest.find("${") {
        let Some(close) = rest[open..].find('}').map(|c| open + c) else { break };
        out.push_str(&rest[..open]);
        out.push_str(&std::env::var(&rest[open + 2..close]).unwrap_or_default());
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    out
}

/// One call to the tool a section's mapping gives a verb, its `{placeholders}` filled from `values` (an argument whose
/// placeholder has no value is left out), its answer as JSON. Err names the verb when nothing is mapped.
pub async fn call_mapped(connection: &Connection, section: &Value, verb: &str, values: &Value) -> Result<Value, String> {
    let recipe = &section["tools"][verb];
    let tool = recipe["tool"].as_str().filter(|t| !t.is_empty()).ok_or_else(|| format!("no tool mapped for {verb}"))?;
    let empty = Map::new();
    let values = values.as_object().unwrap_or(&empty);
    let args =
        crate::mapping::fill_args(recipe.get("args").unwrap_or(&Value::Object(Map::new())), values).and_then(|a| a.as_object().cloned()).unwrap_or_default();
    connection.call(tool, args).await
}

/// One call to the tool a section maps to a verb, its answer's text as it is (logs, which are no JSON).
pub async fn call_mapped_text(connection: &Connection, section: &Value, verb: &str, values: &Value) -> Result<String, String> {
    let recipe = &section["tools"][verb];
    let tool = recipe["tool"].as_str().filter(|t| !t.is_empty()).ok_or_else(|| format!("no tool mapped for {verb}"))?;
    let empty = Map::new();
    let args = crate::mapping::fill_args(recipe.get("args").unwrap_or(&Value::Object(Map::new())), values.as_object().unwrap_or(&empty))
        .and_then(|a| a.as_object().cloned())
        .unwrap_or_default();
    connection.call_text(tool, args).await
}

/// Runs a future on a runtime of its own, for the synchronous callers; TLS goes through ring.
pub fn block_on<F: std::future::Future>(future: F) -> F::Output {
    let _ = rustls::crypto::ring::default_provider().install_default();
    tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio runtime").block_on(future)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_handshake_names_a_version_that_has_one() {
        let info = handler();
        assert!(info.protocol_version.has_initialize());
        assert_eq!(info.client_info.name, "cockpit");
    }

    #[test]
    fn a_shims_resolution_line_is_not_the_token() {
        assert_eq!(printed_token("gho_abc123\n").as_deref(), Some("gho_abc123"));
        assert_eq!(printed_token("mise ~/.config/mise/config.toml tools: gh@2.103.3\ngho_abc123\n").as_deref(), Some("gho_abc123"));
        assert_eq!(printed_token("").as_deref(), None);
        assert_eq!(printed_token("mise ~/.config/mise/config.toml tools: gh@2.103.3\n").as_deref(), None);
    }
}
