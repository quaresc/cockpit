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
    assert_eq!((after(tab, "--workspace"), after(tab, "--label")), ("w3", "review api#4"));
    assert!(calls.contains(&vec!["pane".into(), "run".into(), "wT:p1".into(), "claude '/code-review 4'".into()]), "{calls:?}");
    assert_eq!(reply["note"], "started \"review api#4\" in the actions space");
    let kept = std::fs::read_to_string(sandbox.home().join(".cache/cockpit/actions.json")).unwrap();
    assert_eq!(serde_json::from_str::<Value>(&kept).unwrap()["review api#4"], "wT:p1");
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
    assert!(run[3].starts_with("claude --resume s-main --fork-session "), "{run:?}");
    assert_eq!(reply["note"], "started \"talk eng-403\" in a new space");
}

#[test]
fn continue_moves_the_plan_out_of_the_actions_space() {
    let sandbox = with_herdr(
        r#"{"pane get": {"pane": {"pane_id": "w9:p2", "tab_id": "w9:t2", "agent": "claude"}}, "pane move": {"workspace_id": "wN"}}"#,
    );
    let (reply, calls) = act(&sandbox, "continue_ticket", &format!("[{TICKET1}, []]"));
    assert!(calls.contains(&["pane", "move", "w9:p2", "--new-workspace", "--label", "eng-401", "--focus"].map(String::from).to_vec()), "{calls:?}");
    assert!(!calls.iter().any(|c| c[0] == "workspace" && c[1] == "create" || c[0] == "pane" && c[1] == "run"), "{calls:?}");
    assert_eq!(reply["note"], "moved the plan of ENG-401 to a space of its own");
    let kept: Value = serde_json::from_str(&std::fs::read_to_string(sandbox.home().join(".cache/cockpit/actions.json")).unwrap()).unwrap();
    assert!(kept.get("plan eng-401").is_none(), "{kept}");
}

#[test]
fn an_open_space_on_the_branchs_worktree_comes_into_view() {
    let sandbox = with_herdr("{}");
    let tree = worktree(&sandbox, "me/eng-4-login");
    let spaces = json!({"workspace list": {"workspaces": [{"workspace_id": "w7", "label": "[7] eng-4", "worktree": {"checkout_path": tree}}]}});
    std::fs::write(sandbox.root.join("herdr.json"), spaces.to_string()).unwrap();
    let (reply, calls) = act(&sandbox, "fix_ci", &fix_ci_here("me/eng-4-login"));
    let acted: Vec<&Vec<String>> = calls.iter().filter(|c| c[..2] != ["workspace", "list"]).collect();
    assert_eq!(acted, [&["workspace", "focus", "w7"].map(String::from).to_vec()]);
    assert_eq!(reply["note"], "me/eng-4-login is already open in \"eng-4\": focused it");
}

#[test]
fn a_new_space_opens_in_the_branchs_worktree() {
    let sandbox = with_herdr("{}");
    let tree = worktree(&sandbox, "me/eng-4-login");
    let (_, calls) = act(&sandbox, "fix_ci", &fix_ci_here("me/eng-4-login"));
    let created = find(&calls, &["workspace", "create"]);
    assert_eq!(real(after(created, "--cwd")), real(&tree.to_string_lossy()));
    assert!(find(&calls, &["pane", "run"])[3].contains("is checked out here"), "{calls:?}");
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
    assert!(find(&calls, &["pane", "run"])[3].contains("Check the branch me/eng-9 out first."), "{calls:?}");
}

#[test]
fn talk_forks_the_session_that_spent_most_and_still_has_its_transcript() {
    // s-gone spent more on ENG-401, but its transcript's folder is gone: s-main, from its own folder, is the one.
    let sandbox = with_herdr("{}");
    let (_, calls) = act(&sandbox, "talk", &format!(r#"[{TICKET1}, [], {{}}]"#));
    let tab = find(&calls, &["tab", "create"]);
    assert_eq!(real(after(tab, "--cwd")), real(&sandbox.root.join("repos/api").to_string_lossy()));
    assert!(find(&calls, &["pane", "run"])[3].starts_with("claude --resume s-main --fork-session "), "{calls:?}");
}

#[test]
fn talk_with_no_session_on_the_ticket_starts_fresh() {
    let sandbox = with_herdr("{}");
    let (_, calls) = act(&sandbox, "talk", &format!(r#"[{TICKET3}, [], {{}}]"#));
    let run = find(&calls, &["pane", "run"]);
    assert!(!run[3].contains("--resume") && run[3].contains("ENG-403"), "{run:?}");
    assert!(Path::new(after(find(&calls, &["tab", "create"]), "--cwd")).is_dir());
}
