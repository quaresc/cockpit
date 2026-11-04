//! `cockpit mcp` spoken to over stdio, and `cockpit agents connect` on a fake HOME.

mod common;

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use common::{Sandbox, executable, stdout};
use serde_json::{Value, json};

/// A running `cockpit mcp` with the sandbox's environment and these extra variables.
struct Server {
    child: std::process::Child,
    out: BufReader<std::process::ChildStdout>,
    next: u64,
}

impl Server {
    fn start(sandbox: &Sandbox, extra: &[(&str, &str)]) -> Server {
        let mut child = Command::new(env!("CARGO_BIN_EXE_cockpit"))
            .arg("mcp")
            .env_clear()
            .envs(sandbox.env())
            .envs(extra.iter().map(|(k, v)| (k.to_string(), v.to_string())))
            .current_dir(&sandbox.root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let out = BufReader::new(child.stdout.take().unwrap());
        Server { child, out, next: 1 }
    }

    fn ask(&mut self, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{}", json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})).unwrap();
        let mut line = String::new();
        self.out.read_line(&mut line).unwrap();
        let answer: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(answer["id"], id);
        answer
    }

    fn tell(&mut self, method: &str) {
        writeln!(self.child.stdin.as_mut().unwrap(), "{}", json!({"jsonrpc": "2.0", "method": method})).unwrap();
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

#[test]
fn an_agent_in_a_herdr_pane_gets_a_view_beside_it_and_the_persons_answer_back() {
    let sandbox = Sandbox::new();
    sandbox.herdr(&[("pane split", r#"{"result": {"pane": {"pane_id": "w1:p2"}}}"#)]);
    let mut server = Server::start(&sandbox, &[("HERDR_ENV", "1"), ("HERDR_PANE_ID", "w1:p1")]);
    let hello = server.ask("initialize", json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "codex", "version": "0"}}));
    assert_eq!(hello["result"]["protocolVersion"], "2025-06-18");
    server.tell("notifications/initialized");
    assert_eq!(server.ask("tools/list", json!({}))["result"]["tools"].as_array().unwrap().len(), 4);

    let view = json!({"pane": "deploy", "title": "Deploy", "blocks": [{"type": "choice", "prompt": "Which env?", "options": [{"label": "staging"}, {"label": "prod"}]}]});
    let shown = server.ask("tools/call", json!({"name": "show", "arguments": view}));
    assert_eq!(shown["result"]["isError"], false, "{shown}");
    assert_eq!(shown["result"]["content"][0]["text"], "Opened the deploy pane beside this session; q hides it.");
    let spec: Value = serde_json::from_str(&std::fs::read_to_string(sandbox.cache().join("show/deploy.json")).unwrap()).unwrap();
    assert_eq!(spec["title"], "Deploy");
    assert!(spec["session"].as_str().unwrap().starts_with("mcp-"));
    let calls = sandbox.calls();
    assert!(calls.contains(&"herdr pane split w1:p1 --direction right".to_string()), "{calls:?}");
    assert!(calls.iter().any(|c| c.starts_with("herdr pane run w1:p2 exec ") && c.ends_with(" show deploy")), "{calls:?}");
    assert!(calls.contains(&"herdr pane focus --pane w1:p2 --direction left".to_string()), "{calls:?}");

    // The person picks in the pane: the answer file it writes reaches the agent as its next prompt.
    std::fs::write(sandbox.cache().join("show/deploy.0.answer"), r#"{"block": 0, "type": "choice", "value": "prod", "label": "prod"}"#).unwrap();
    let asked = "herdr agent prompt w1:p1 In the deploy pane I picked \"prod\" for: Which env.".to_string();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !sandbox.calls().contains(&asked) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(sandbox.calls().contains(&asked), "{:?}", sandbox.calls());
    assert!(!sandbox.cache().join("show/deploy.0.answer").exists());

    let wrong = server.ask("tools/call", json!({"name": "show", "arguments": {"pane": "x", "title": "t", "blocks": [{"type": "chart", "kind": "pie"}]}}));
    assert_eq!(wrong["result"]["isError"], true);
    assert_eq!(wrong["result"]["content"][0]["text"], "at /blocks/0/kind: a chart is one of \"line\", \"area\", \"bar\", \"donut\"");
}

#[test]
fn outside_herdr_the_view_is_written_without_a_pane() {
    let sandbox = Sandbox::new();
    let mut server = Server::start(&sandbox, &[]);
    let shown = server.ask(
        "tools/call",
        json!({"name": "show", "arguments": {"pane": "plan", "title": "Plan", "blocks": [{"type": "blueprint", "text": "flowchart TD\n  a --> b"}]}}),
    );
    let text = shown["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("but this session is in neither herdr nor the Herdr app, so no pane opened"), "{text}");
    assert!(text.contains("Keep the blueprint current"), "{text}");
    assert!(!sandbox.calls().iter().any(|c| c.starts_with("herdr pane split")));
}

#[test]
fn a_runbook_step_is_reported_from_any_agent() {
    let sandbox = Sandbox::new();
    let book = sandbox.home().join(".config/cockpit/runbooks/eng-412.md");
    sandbox.write(
        "home/.config/cockpit/runbooks/eng-412.md",
        "| # | Step | Where | Status | Command |\n|---|---|---|---|---|\n| 2 | Deploy | api | | `make deploy` |\n",
    );
    let handoff = json!({"id": "eng-412-2", "slug": "eng-412", "step": "2", "title": "Deploy", "path": book, "state": "sent", "sentAt": 1});
    sandbox.write("home/.cache/cockpit/handoffs/eng-412-2.json", &handoff.to_string());
    let mut server = Server::start(&sandbox, &[]);
    let said = server.ask("tools/call", json!({"name": "report_step", "arguments": {"handoff": "eng-412-2", "outcome": "blocked", "message": "no VPN"}}));
    assert_eq!(said["result"]["content"][0]["text"], "Recorded: step 2 of ENG-412 blocked; its runbook status now reads \"❌ blocked\".");
    assert!(std::fs::read_to_string(&book).unwrap().contains("| 2 | Deploy | api | ❌ blocked |"));
}

#[test]
fn connect_writes_each_agents_own_config_and_disconnect_takes_it_out() {
    let sandbox = Sandbox::new();
    let codex_home = sandbox.home().join("codex-home");
    std::fs::create_dir_all(&codex_home).unwrap();
    let toml = "model = \"o3\"\n\n[mcp_servers.docs]\ncommand = \"docs\"\n";
    std::fs::write(codex_home.join("config.toml"), toml).unwrap();
    let jsonc = "{\n  // mine\n  \"theme\": \"dark\",\n}\n";
    sandbox.write("home/.config/opencode/opencode.json", jsonc);
    for bin in ["codex", "opencode"] {
        executable(&sandbox.root.join("bin").join(bin), "#!/bin/sh\n");
    }
    let fake_codex = sandbox.root.join("bin/codex");
    let env = [("CODEX_HOME", codex_home.to_str().unwrap()), ("COCKPIT_CODEX", fake_codex.to_str().unwrap())];
    let out = sandbox.cockpit(&["agents", "connect", "codex", "opencode"], &env);
    assert!(out.status.success(), "{}", common::stderr(&out));
    let said = stdout(&out);
    assert!(said.contains("Codex has cockpit's tools in ") && said.contains("(backup "), "{said}");
    let exe = std::fs::canonicalize(env!("CARGO_BIN_EXE_cockpit")).unwrap();
    let written = std::fs::read_to_string(codex_home.join("config.toml")).unwrap();
    assert!(written.starts_with(toml), "{written}");
    assert!(written.contains(&format!("[mcp_servers.cockpit]\ncommand = \"{}\"\nargs = [\"mcp\"]\nenv_vars = [", exe.display())), "{written}");
    let opencode = std::fs::read_to_string(sandbox.home().join(".config/opencode/opencode.json")).unwrap();
    assert!(opencode.starts_with("{\n  // mine\n  \"theme\": \"dark\",\n  \"mcp\": {"), "{opencode}");
    assert!(
        std::fs::read_dir(sandbox.home().join(".config/opencode"))
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with("opencode.json.bak-cockpit-"))
    );

    let again = stdout(&sandbox.cockpit(&["agents", "connect", "codex", "opencode"], &env));
    assert!(again.contains("Codex already has cockpit's tools") && again.contains("opencode already has cockpit's tools"), "{again}");
    let listed: Value = serde_json::from_str(&stdout(&sandbox.cockpit(&["agents", "connected"], &env))).unwrap();
    let codex = listed.as_array().unwrap().iter().find(|t| t["id"] == "codex").unwrap();
    assert_eq!((codex["installed"].clone(), codex["connected"].clone()), (json!(true), json!(true)));

    let out = sandbox.cockpit(&["agents", "disconnect", "codex", "opencode"], &env);
    assert!(out.status.success(), "{}", common::stderr(&out));
    assert_eq!(std::fs::read_to_string(codex_home.join("config.toml")).unwrap(), toml);
    assert_eq!(std::fs::read_to_string(sandbox.home().join(".config/opencode/opencode.json")).unwrap(), jsonc);

    let claude = sandbox.cockpit(&["agents", "connect", "claude"], &env);
    assert!(!claude.status.success() && common::stderr(&claude).contains("plugin"));
}

#[test]
fn a_config_that_does_not_read_is_left_alone() {
    let sandbox = Sandbox::new();
    sandbox.write("home/.qwen/settings.json", "{ \"a\": }");
    let out = sandbox.cockpit(&["agents", "connect", "qwen"], &[]);
    assert!(!out.status.success());
    assert!(common::stderr(&out).contains("does not read as the agent's config"), "{}", common::stderr(&out));
    assert_eq!(std::fs::read_to_string(sandbox.home().join(".qwen/settings.json")).unwrap(), "{ \"a\": }");
}
