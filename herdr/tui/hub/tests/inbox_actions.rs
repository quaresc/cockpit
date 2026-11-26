//! The inbox's Claude-session actions through the socket, by what they ask herdr: a tab of the actions space or a space of
//! their own, the PR's branch found in a worktree, a plan's tab moved out, and the ticket's busiest session forked.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::{Hub, Sandbox};
use serde_json::{Value, json};

const PR4: &str = r#"{"repo": "api", "number": 4, "url": "https://github.com/acme/api/pull/4", "branch": "me/eng-4-login"}"#;
const TICKET1: &str = r#"{"id": "ENG-401", "title": "Sign in with Google", "url": "https://linear.app/acme/issue/ENG-401/sign-in"}"#;
const TICKET3: &str = r#"{"id": "ENG-403", "title": "Password reset", "url": "https://linear.app/acme/issue/ENG-403/reset"}"#;
const NO_ACTIONS: &str = r#"{"workspace list": {"workspaces": [{"workspace_id": "w1", "label": "[1] x  inbox", "focused": false}]}}"#;

/// Runs one inbox action on the sandbox: its reply and the herdr and claude calls it made, as argument lists.
fn act(sandbox: &Sandbox, action: &str, args: &str) -> (Value, Vec<Vec<String>>) {
    let hub = Hub::start(sandbox, &[]);
    let reply = hub.request(&json!({"op": "act", "app": "inbox", "action": action, "args": sandbox.fill(args), "armed": null}));
    assert_eq!(reply["error"], Value::Null, "{action}: {reply}");
    let herdr = sandbox
        .calls()
        .iter()
        .filter(|c| c["cmd"] == "herdr")
        .map(|c| c["args"].as_array().unwrap().iter().map(|a| a.as_str().unwrap().to_string()).collect())
        .collect();
    (reply, herdr)
}

fn with_herdr(answers: &str) -> Sandbox {
    Sandbox::new(&serde_json::from_str(answers).unwrap(), &json!({}))
}

fn real(path: &str) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path))
}

/// The argument after `flag` in a call.
fn after<'a>(call: &'a [String], flag: &str) -> &'a str {
    &call[call.iter().position(|a| a == flag).unwrap_or_else(|| panic!("no {flag} in {call:?}")) + 1]
}

fn find<'a>(calls: &'a [Vec<String>], verb: &[&str]) -> &'a Vec<String> {
    calls.iter().find(|c| c.starts_with(&verb.iter().map(|v| v.to_string()).collect::<Vec<_>>())).unwrap_or_else(|| panic!("no herdr {verb:?} in {calls:?}"))
}

/// A worktree of repos/api on a branch of its own, next to the repos.
fn worktree(sandbox: &Sandbox, branch: &str) -> PathBuf {
    let tree = sandbox.root.join("wt");
    let api = sandbox.root.join("repos/api");
    let done = Command::new("git").arg("-C").arg(&api).args(["worktree", "add", "-q", "-b", branch]).arg(&tree).output().unwrap();
    assert!(done.status.success(), "git worktree add: {}", String::from_utf8_lossy(&done.stderr));
    tree
}

/// The prompt a `sh <script>` line hands over, read from the file beside its script.
fn prompt_of(line: &str) -> String {
    let script = line.strip_prefix("sh ").unwrap_or_else(|| panic!("not handed over by file: {line}")).trim_matches('\'');
    std::fs::read_to_string(script.replace(".sh", ".prompt")).unwrap()
}

/// The command a `sh <script>` line runs, the prompt read from its file.
fn script_of(line: &str) -> String {
    let script = line.strip_prefix("sh ").unwrap_or_else(|| panic!("not handed over by file: {line}")).trim_matches('\'');
    std::fs::read_to_string(script).unwrap()
}

fn fix_ci_here(branch: &str) -> String {
    let mut pr: Value = serde_json::from_str(PR4).unwrap();
    pr["branch"] = json!(branch);
    format!(r#"[{pr}, {{}}, "here"]"#)
}

#[test]
fn an_action_runs_in_a_tab_of_the_actions_space() {
    let sandbox = with_herdr("{}");
    let (reply, calls) = act(&sandbox, "review", &format!("[{PR4}]"));
    let tab = find(&calls, &["tab", "create"]);
    assert_eq!((after(tab, "--workspace"), after(tab, "--label")), ("w3", "\u{f441}  api#4"));
    let run = find(&calls, &["pane", "run"]);
    assert_eq!(run[2], "wT:p1");
    let script = script_of(&run[3]);
    assert!(
        script.contains("exec env COCKPIT_REVIEW_PR=https://github.com/acme/api/pull/4 COCKPIT_REVIEW_BRANCH=me/eng-4-login claude ''\"$(cat \"$P\")\"''"),
        "{script}"
    );
    assert_eq!(prompt_of(&run[3]), "/code-review 4", "even a prompt of cockpit's own goes by file");
    assert_eq!(reply["note"], "started \"\u{f441}  api#4\" in the actions space");
    let kept = std::fs::read_to_string(sandbox.home().join(".cache/cockpit/actions.json")).unwrap();
    assert_eq!(serde_json::from_str::<Value>(&kept).unwrap()["\u{f441}  api#4"], "wT:p1");
}

#[test]
fn without_the_actions_space_an_action_gets_a_space() {
    let sandbox = with_herdr(NO_ACTIONS);
    let (reply, calls) = act(&sandbox, "talk", &format!(r#"[{TICKET3}, [], {{}}, "s-main"]"#));
    assert!(!calls.iter().any(|c| c.starts_with(&["tab".into(), "create".into()])), "{calls:?}");
    let created = find(&calls, &["workspace", "create"]);
    assert_eq!(after(created, "--label"), "talk eng-403");
    let run = find(&calls, &["pane", "run"]);
    assert_eq!(run[2], "wT:p1");
    assert!(script_of(&run[3]).contains("exec claude --resume s-main --fork-session "), "{run:?}");
    assert_eq!(reply["note"], "started \"talk eng-403\" in a new space");
}

#[test]
fn continue_moves_the_plan_out_of_the_actions_space() {
    let sandbox = with_herdr(r#"{"pane get": {"pane": {"pane_id": "w9:p2", "tab_id": "w9:t2", "agent": "claude"}}, "pane move": {"workspace_id": "wN"}}"#);
    let (reply, calls) = act(&sandbox, "continue_ticket", &format!("[{TICKET1}, []]"));
    assert!(calls.contains(&["pane", "move", "w9:p2", "--new-workspace", "--label", "eng-401", "--focus"].map(String::from).to_vec()), "{calls:?}");
    assert!(!calls.iter().any(|c| c[0] == "workspace" && c[1] == "create" || c[0] == "pane" && c[1] == "run"), "{calls:?}");
    assert_eq!(reply["note"], "moved the plan of ENG-401 to a space of its own");
    let kept: Value = serde_json::from_str(&std::fs::read_to_string(sandbox.home().join(".cache/cockpit/actions.json")).unwrap()).unwrap();
    assert!(kept.get("plan eng-401").is_none(), "{kept}");
}

#[test]
fn an_open_space_with_no_agent_on_the_branch_gets_one_in_a_new_tab_and_says_so() {
    let sandbox = with_herdr("{}");
    let tree = worktree(&sandbox, "me/eng-4-login");
    let spaces = json!({"workspace list": {"workspaces": [{"workspace_id": "w7", "label": "[7] eng-4", "worktree": {"checkout_path": tree}}]}});
    std::fs::write(sandbox.root.join("herdr.json"), spaces.to_string()).unwrap();
    let (reply, calls) = act(&sandbox, "fix_ci", &fix_ci_here("me/eng-4-login"));
    let tab = find(&calls, &["tab", "create"]);
    assert_eq!((after(tab, "--workspace"), after(tab, "--label")), ("w7", "ci api#4"));
    let prompt = prompt_of(&find(&calls, &["pane", "run"])[3]);
    assert!(
        prompt.starts_with("CI is failing on PR #4.") && prompt.contains("Work in the checkout named below") && prompt.contains("branch: \"me/eng-4-login\""),
        "{prompt}"
    );
    assert_eq!(reply["note"], "no agent found in \"eng-4\", where its branch is checked out: started one in a new tab there");
}

#[test]
fn a_new_space_opens_in_the_branchs_worktree() {
    let sandbox = with_herdr("{}");
    let tree = worktree(&sandbox, "me/eng-4-login");
    let (_, calls) = act(&sandbox, "fix_ci", &fix_ci_here("me/eng-4-login"));
    let created = find(&calls, &["workspace", "create"]);
    assert_eq!(real(after(created, "--cwd")), real(&tree.to_string_lossy()));
    assert!(prompt_of(&find(&calls, &["pane", "run"])[3]).contains("Work in the checkout named below"), "{calls:?}");
}

#[test]
fn the_branch_checked_out_in_the_main_checkout_opens_there() {
    let sandbox = with_herdr("{}");
    worktree(&sandbox, "me/eng-4-login");
    let (_, calls) = act(&sandbox, "fix_ci", &fix_ci_here("me/eng-401-login"));
    let created = find(&calls, &["workspace", "create"]);
    assert_eq!(real(after(created, "--cwd")), real(&sandbox.root.join("repos/api").to_string_lossy()));
}

#[test]
fn a_branch_checked_out_nowhere_is_checked_out_first() {
    let sandbox = with_herdr("{}");
    worktree(&sandbox, "me/eng-4-login");
    let (_, calls) = act(&sandbox, "fix_ci", &fix_ci_here("me/eng-9"));
    let created = find(&calls, &["workspace", "create"]);
    assert_eq!(real(after(created, "--cwd")), real(&sandbox.root.join("repos/api").to_string_lossy()));
    let prompt = prompt_of(&find(&calls, &["pane", "run"])[3]);
    assert!(
        prompt.starts_with("CI is failing on PR #4.") && prompt.contains("Check the branch named below out first.") && prompt.contains("branch: \"me/eng-9\""),
        "{prompt}"
    );
}

#[test]
fn talk_forks_the_session_that_spent_most_and_still_has_its_transcript() {
    // s-gone spent more on ENG-401, but its transcript's folder is gone: s-main, from its own folder, is the one.
    let sandbox = with_herdr("{}");
    let (_, calls) = act(&sandbox, "talk", &format!(r#"[{TICKET1}, [], {{}}]"#));
    let tab = find(&calls, &["tab", "create"]);
    assert_eq!(real(after(tab, "--cwd")), real(&sandbox.root.join("repos/api").to_string_lossy()));
    assert!(script_of(&find(&calls, &["pane", "run"])[3]).contains("exec claude --resume s-main --fork-session "), "{calls:?}");
}

#[test]
fn talk_with_no_session_on_the_ticket_starts_fresh() {
    let sandbox = with_herdr("{}");
    let (_, calls) = act(&sandbox, "talk", &format!(r#"[{TICKET3}, [], {{}}]"#));
    let run = find(&calls, &["pane", "run"]);
    assert!(!script_of(&run[3]).contains("--resume") && prompt_of(&run[3]).contains("ENG-403"), "{run:?}");
    assert!(Path::new(after(find(&calls, &["tab", "create"]), "--cwd")).is_dir());
}

/// git in `dir`, as the person would run it.
fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git").args(["-c", "user.name=me", "-c", "user.email=me@acme.example", "-C"]).arg(dir).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

/// repos/api pushed to a local bare remote, `origin/HEAD` set.
fn with_remote(sandbox: &Sandbox) -> PathBuf {
    let api = sandbox.root.join("repos/api");
    git(&sandbox.root, &["init", "-q", "--bare", "-b", "main", "remote.git"]);
    git(&api, &["checkout", "-q", "-B", "main"]);
    git(&api, &["remote", "add", "origin", &sandbox.root.join("remote.git").to_string_lossy()]);
    git(&api, &["push", "-q", "-u", "origin", "main"]);
    git(&api, &["remote", "set-head", "origin", "--auto"]);
    api
}

/// The armed removal of a worktree: the hub's reply.
fn remove(hub: &Hub, tree: &Path) -> Value {
    let path = tree.to_string_lossy();
    hub.request(&json!({"op": "act", "app": "inbox", "action": "remove_worktree", "args": [path], "armed": format!("remove-{path}")}))
}

#[test]
fn a_worktree_is_removed_only_when_nothing_keeps_it_and_the_refusal_names_what_does() {
    let sandbox = with_herdr("{}");
    let api = with_remote(&sandbox);
    let tree = worktree(&sandbox, "me/eng-412-quota-model");
    let hub = Hub::start(&sandbox, &[]);
    let path = tree.to_string_lossy().into_owned();
    // Locked, it stays, and nothing is closed or fetched on a refusal.
    git(&api, &["worktree", "lock", "--reason", "on a USB disk", &path]);
    assert_eq!(remove(&hub, &tree)["error"], format!("kept {path}: locked: on a USB disk"));
    assert!(!sandbox.calls().iter().any(|c| c["args"][0] == "workspace" && c["args"][1] == "close"), "{:?}", sandbox.calls());
    git(&api, &["worktree", "unlock", &path]);
    std::fs::write(tree.join("notes.md"), "draft").unwrap();
    git(&tree, &["commit", "-q", "--allow-empty", "-m", "quota model"]);
    let reply = remove(&hub, &tree);
    assert_eq!(reply["error"], format!("kept {path}: 1 uncommitted change, 1 commit not pushed, not merged into main"), "{reply}");
    assert!(tree.join("notes.md").is_file(), "never forced");
    std::fs::remove_file(tree.join("notes.md")).unwrap();
    git(&tree, &["push", "-q", "-u", "origin", "me/eng-412-quota-model"]);
    assert_eq!(remove(&hub, &tree)["error"], format!("kept {path}: not merged into main"));
    // A fetch sets origin/HEAD again on its own (git 2.48 on) unless told not to.
    git(&api, &["config", "remote.origin.followRemoteHEAD", "never"]);
    git(&api, &["remote", "set-head", "origin", "--delete"]);
    let reply = remove(&hub, &tree);
    assert_eq!(reply["error"], format!("kept {path}: its default branch is unknown (git remote set-head origin --auto)"));
    git(&api, &["remote", "set-head", "origin", "--auto"]);
    // Merged on the remote and its branch deleted there: the removal fetches that, then nothing keeps it.
    git(&tree, &["push", "-q", "origin", "me/eng-412-quota-model:main"]);
    git(&tree, &["push", "-q", "origin", "--delete", "me/eng-412-quota-model"]);
    let reply = remove(&hub, &tree);
    assert_eq!(reply["error"], Value::Null, "{reply}");
    // Its branch is merged on the remote, not into this checkout's main: git keeps it rather than be forced.
    assert_eq!(reply["note"], format!("removed {path}; the branch me/eng-412-quota-model stays until git counts it merged"));
    assert!(!tree.exists());
}

/// The Alerts' removal of a merged session's worktree: the processes of the closing space's panes (by pid, with all they
/// started) end with it and do not keep it; any other program does, a second session of the same agent included, and
/// herdr removes it once nothing does.
#[test]
fn a_merged_sessions_worktree_goes_with_its_space_unless_something_else_keeps_it() {
    let sandbox = with_herdr("{}");
    let _api = with_remote(&sandbox);
    let tree = worktree(&sandbox, "me/eng-430");
    git(&tree, &["commit", "-q", "--allow-empty", "-m", "x"]);
    git(&tree, &["push", "-q", "-u", "origin", "me/eng-430"]);
    git(&tree, &["push", "-q", "origin", "me/eng-430:main"]);
    let path = real(&tree.to_string_lossy()).to_string_lossy().into_owned();
    let bin = sandbox.root.join("progs");
    std::fs::create_dir_all(&bin).unwrap();
    let run = |name: &str| common::waiting(&bin, name, &tree, 60);
    let stop = |mut p: std::process::Child| {
        assert_eq!(p.try_wait().unwrap(), None, "the program ran all along");
        p.kill().unwrap();
        p.wait().unwrap();
    };
    // The space's own session: its pane's shell, as herdr says it.
    let session = run("claude");
    let answers = json!({
        "workspace list": {"workspaces": [{"workspace_id": "w7", "label": "[3] eng-430", "worktree": {"checkout_path": path, "is_linked_worktree": true}}]},
        "pane list": {"panes": [{"pane_id": "w7:p1", "workspace_id": "w7", "agent": "claude"}]},
        "pane process-info": {"process_info": {"pane_id": "w7:p1", "shell_pid": session.id(), "foreground_processes": [{"pid": session.id(), "name": "claude"}]}},
    });
    std::fs::write(sandbox.root.join("herdr.json"), answers.to_string()).unwrap();
    let hub = Hub::start(&sandbox, &[]);
    let close =
        || hub.request(&json!({"op": "act", "app": "inbox", "action": "close_space", "args": ["w7", "merged api me/eng-430", true], "armed": "close-w7"}));
    let removed = || sandbox.calls().iter().any(|c| c["args"] == json!(["worktree", "remove", "--workspace", "w7"]));
    let node = run("node");
    let reply = close();
    stop(node);
    assert_eq!(reply["error"], format!("kept {path}: node still runs in it"), "{reply}");
    // A second Claude Code in the worktree, outside the space, is not the session that ends.
    let other = run("claude");
    let reply = close();
    stop(other);
    assert_eq!(reply["error"], format!("kept {path}: Claude Code still runs in it"), "{reply}");
    assert!(!removed(), "nothing removed: {:?}", sandbox.calls());
    let reply = close();
    stop(session);
    assert_eq!(reply["error"], Value::Null, "{reply}");
    assert!(removed(), "{:?}", sandbox.calls());
}

#[test]
fn an_agent_s_tools_confirm_nothing_start_nothing_and_run_nothing_through_the_hub() {
    let sandbox = with_herdr(NO_ACTIONS);
    let tree = worktree(&sandbox, "me/eng-412-quota-model");
    let hub = Hub::start(&sandbox, &[(common::SENT_BY, "app:t9")]);
    let path = tree.to_string_lossy().into_owned();
    // A confirm key is the person's, whatever an agent's tools bring: the step's key, the worktree's.
    let removed = remove(&hub, &tree);
    assert!(removed["error"].as_str().unwrap().contains("by a person"), "{removed}");
    assert!(tree.is_dir(), "never removed");
    let book = json!({"slug": "eng-412", "path": sandbox.root.join("eng-412.md")});
    let step = hub.request(&json!({"op": "act", "app": "runbook", "action": "run_step", "args": [book, {"n": "2"}], "armed": "run-eng-412-2"}));
    assert!(step["error"].as_str().unwrap().contains("by a person"), "{step}");
    for (app, action, args) in [
        ("inbox", "run", json!([["sh", "-c", "touch owned"]])),
        ("inbox", "plan", sandbox.fill(&format!("[{TICKET1}]"))),
        ("monitor", "roll_back", json!([{"id": "api"}, "v1"])),
        ("start", "work", json!([{"prompt": "rm -rf"}])),
    ] {
        let reply = hub.request(&json!({"op": "act", "app": app, "action": action, "args": args, "armed": null}));
        assert!(reply["error"].as_str().is_some_and(|e| e.contains("by a person")), "{app} {action}: {reply}");
    }
    std::thread::sleep(std::time::Duration::from_millis(500));
    let log = std::fs::read_to_string(sandbox.root.join("calls.log")).unwrap_or_default();
    assert!(!log.contains("\"claude\"") && !log.contains("\"create\"") && !log.contains("\"run\""), "no agent started, nothing run: {log}");
    assert!(!Path::new(&path).join("owned").exists() && !sandbox.root.join("owned").exists());
}

#[test]
fn a_mail_s_words_reach_the_agent_by_file_never_typed_into_a_shell() {
    let sandbox = with_herdr("{}");
    let hub = Hub::start(&sandbox, &[]);
    let marker = sandbox.root.join("pwned");
    let body = format!("Hi,\n\u{15}touch {}\r\u{3}touch {}\r", marker.display(), marker.display());
    let message = json!({"id": "m1", "from": "ana@acme.example", "fromName": "Ana \u{1b}[31mLima", "subject": "Lunch?\u{7}", "body": body});
    let reply = hub.request(&json!({"op": "act", "app": "mail", "action": "mail_reply", "args": [message], "armed": null}));
    assert_eq!(reply["error"], Value::Null, "{reply}");
    let calls: Vec<Vec<String>> = sandbox
        .calls()
        .iter()
        .filter(|c| c["cmd"] == "herdr")
        .map(|c| c["args"].as_array().unwrap().iter().map(|a| a.as_str().unwrap().to_string()).collect())
        .collect();
    let line = &find(&calls, &["pane", "run"])[3];
    assert!(line.starts_with("sh ") && !line.contains("touch") && !line.contains("Ana"), "only the script's path is typed: {line}");
    let prompt = prompt_of(line);
    assert!(prompt.contains("touch") && !prompt.chars().any(|c| c.is_control() && c != '\n'), "the words, without what a terminal acts on: {prompt:?}");
    assert!(!marker.exists());
}

#[test]
fn what_cockpit_focus_or_act_sends_is_never_the_person_s() {
    // A window command or a notification's click runs `cockpit focus act:…`, which runs `cockpit act …`: neither is the
    // person's hand, so nothing only a person does passes from them, wherever they stand.
    let sandbox = with_herdr("{}");
    let hub = Hub::start(&sandbox, &[(common::PLACE_REAL, "1")]);
    let dir = tempfile::tempdir().unwrap();
    let run = json!({"op": "act", "app": "inbox", "action": "run", "args": [["sh", "-c", "touch owned"]], "armed": null});
    for word in ["focus", "act", "setup-run"] {
        let reply = common::sent_as_cockpit(dir.path(), word, &sandbox.socket(), &run);
        assert_eq!(reply["error"], "run: cockpit's own work", "cockpit {word}");
    }
    // The same binary as one of the person's own commands is not refused for that.
    let reply = common::sent_as_cockpit(dir.path(), "inbox", &sandbox.socket(), &run);
    assert!(!reply["error"].as_str().unwrap_or_default().contains("cockpit's own work"), "{reply}");
    drop(hub);
    assert!(!sandbox.root.join("owned").exists());
}
