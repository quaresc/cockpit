//! The review space on the acme fixture: its screens (no agent, each scope, a clean tree, the PR scope without gh or
//! a pull request, comments, side by side, the walkthrough none, writing, failed and done, the worktree picker, the
//! keys overlay) and what its keys do (scopes, hunks, the comment sent to the agent, steps, the picker).

use cockpit_core::testing::render;
use cockpit_review::app::{App, Cmd, Mode, Recipient, Scope, Sent, To};
use cockpit_show::gui::hunk_ids;
use cockpit_show::patch::{parse_patch, prepare};
use cockpit_show::review::Read;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use serde_json::{Value, json};

const UNCOMMITTED: &str = include_str!("fixtures/uncommitted.patch");
const BRANCH: &str = include_str!("fixtures/branch.patch");
const API: &str = "/work/acme/api";

fn agents() -> Value {
    json!({"agents": [
        {"terminal": "w2:p1", "frontend": "tui", "program": "claude", "title": "Daily quotas: enforce limits", "status": "working", "since": 2000, "cwd": "/work/acme/api", "branch": "me/eng-412-daily-quotas"},
        {"terminal": "w3:p1", "frontend": "tui", "program": "codex", "title": "Quota banner", "status": "done", "since": 1000, "cwd": "/work/acme/web", "branch": "me/eng-413-quota-ui"},
        {"terminal": "w4:p1", "frontend": "tui", "program": "claude", "title": "Notes", "status": "idle", "since": 3000, "cwd": "/Users/me/notes"},
        {"terminal": "claude:demo-421", "program": "claude", "title": "Per-plan limits", "status": "idle", "since": 1500, "cwd": "/work/acme/api", "driven": true},
    ]})
}

fn toplevel(cwd: &str) -> Option<String> {
    cwd.starts_with("/work/acme/").then(|| cwd.to_string())
}

fn about(scope: Scope) -> Value {
    match scope {
        Scope::Uncommitted => json!({"branch": "me/eng-412-daily-quotas", "base": "HEAD"}),
        Scope::Branch => json!({"branch": "me/eng-412-daily-quotas", "base": "main", "commits": 2}),
        Scope::Pr => {
            json!({"branch": "me/eng-412-daily-quotas", "base": "main", "pr": {"number": 2198, "url": "https://github.com/acme/api/pull/2198", "title": "Daily quotas", "isDraft": false}})
        }
    }
}

fn read(scope: Scope, patch: &str, walkthrough: Value) -> Read {
    Read {
        repo: API.into(),
        scope: scope.id().into(),
        hash: cockpit_show::diff::hash(patch),
        files: Some(prepare(patch)),
        about: about(scope),
        walkthrough,
        error: None,
    }
}

fn failed(scope: Scope, error: &str) -> Read {
    Read {
        repo: API.into(),
        scope: scope.id().into(),
        hash: 0,
        files: None,
        about: json!({"branch": "me/eng-412-daily-quotas"}),
        walkthrough: json!({"status": "none"}),
        error: Some(error.into()),
    }
}

/// The space with nothing read yet, its clock fixed.
fn new_app() -> App {
    let mut app = App::new(false);
    app.clock = |_| "10:42:07".to_string();
    app
}

/// The space on the acme agents, the uncommitted changes read.
fn app() -> App {
    let mut app = new_app();
    app.set_agents(&agents(), &toplevel);
    app.arrive(read(Scope::Uncommitted, UNCOMMITTED, json!({"status": "none"})));
    app
}

fn press(app: &mut App, c: char) {
    key(app, KeyCode::Char(c));
}

fn key(app: &mut App, code: KeyCode) {
    app.event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
}

fn typed(app: &mut App, text: &str) {
    for c in text.chars() {
        press(app, c);
    }
}

fn last(app: &App) -> Sent {
    app.sent.last().cloned().expect("a call was sent")
}

/// The line under the cursor, as the diff draws it.
fn cursor_row(app: &mut App) -> String {
    let screen = render(app, 100, 40);
    screen.lines().find(|l| l.contains('▌')).unwrap_or_default().to_string()
}

/// A done walkthrough of the branch: one step over the code, one outdated over the tests, the README not covered.
fn walkthrough() -> Value {
    let files = parse_patch(BRANCH);
    let ids = |path: &str| -> Vec<String> { files.iter().filter(|f| f.path == path).flat_map(|f| hunk_ids(f).into_iter().map(|(_, _, id)| id)).collect() };
    let mut code = ids("src/quotas.py");
    code.extend(ids("src/api.py"));
    json!({
        "status": "done", "fresh": false, "model": "haiku", "cost": 0.0021, "outdated": 1,
        "title": "Calls past a plan's daily limit are refused",
        "summary": "A plan that used its daily calls gets a 429 instead of an answer, and the remaining count never goes below zero.",
        "parts": [{"title": "Enforcing the limit", "blurb": "", "steps": [
            {"title": "Refuse calls past the limit", "weight": "key", "hunks": code, "missing": 0, "outdated": false,
             "text": "enforce raises QuotaExceeded once nothing remains, and the handler answers it with a 429.",
             "notes": [{"kind": "choice", "text": "A call made exactly at the limit is refused, not the one after it."}]},
            {"title": "Tests for the limit", "weight": "normal", "hunks": ids("tests/test_quotas.py"), "missing": 1, "outdated": true,
             "text": "Two tests pin the floor at zero and the refusal at the limit.", "notes": []},
        ]}],
        "uncovered": [{"path": "README.md", "hunks": ids("README.md"), "note": "The README says what a call past the limit gets."}],
    })
}

// ---------------------------------------------------------------------------------------------------------------------
// Screens

#[test]
fn looking_for_the_agents() {
    insta::assert_snapshot!(render(&mut new_app(), 90, 8));
}

#[test]
fn no_agent_in_a_worktree() {
    let mut app = new_app();
    app.set_agents(
        &json!({"agents": [{"terminal": "w4:p1", "frontend": "tui", "program": "claude", "status": "idle", "since": 1, "cwd": "/Users/me/notes"}]}),
        &toplevel,
    );
    assert!(app.subject().is_none());
    insta::assert_snapshot!(render(&mut app, 90, 10));
}

#[test]
fn the_hub_does_not_answer() {
    let mut app = new_app();
    app.take_agents(Err("the cockpit hub did not start; what it said is in ~/.cache/cockpit/hub.log".into()));
    insta::assert_snapshot!(render(&mut app, 90, 10));
}

#[test]
fn reading_the_changes() {
    let mut app = new_app();
    app.set_agents(&agents(), &toplevel);
    insta::assert_snapshot!(render(&mut app, 90, 8));
}

#[test]
fn uncommitted_scope() {
    insta::assert_snapshot!(render(&mut app(), 100, 40));
}

#[test]
fn branch_scope() {
    let mut app = app();
    press(&mut app, '2');
    assert_eq!(app.scope, Scope::Branch);
    app.arrive(read(Scope::Branch, BRANCH, json!({"status": "none"})));
    insta::assert_snapshot!(render(&mut app, 100, 40));
}

#[test]
fn pr_scope() {
    let mut app = app();
    press(&mut app, '3');
    app.arrive(read(Scope::Pr, BRANCH, json!({"status": "none"})));
    insta::assert_snapshot!(render(&mut app, 100, 12));
}

#[test]
fn pr_scope_without_gh() {
    let mut app = app();
    press(&mut app, '3');
    app.arrive(failed(Scope::Pr, "gh did not start: No such file or directory (os error 2)"));
    insta::assert_snapshot!(render(&mut app, 90, 10));
}

#[test]
fn pr_scope_with_no_pull_request() {
    let mut app = app();
    press(&mut app, '3');
    app.arrive(failed(Scope::Pr, "no pull requests found for branch \"me/eng-412-daily-quotas\""));
    insta::assert_snapshot!(render(&mut app, 90, 10));
}

#[test]
fn an_error_reading_the_changes() {
    let mut app = app();
    press(&mut app, '2');
    app.arrive(failed(Scope::Branch, "no base branch found (origin/HEAD, main or master)"));
    insta::assert_snapshot!(render(&mut app, 90, 10));
}

#[test]
fn a_clean_tree() {
    let mut app = new_app();
    app.set_agents(&agents(), &toplevel);
    app.arrive(read(Scope::Uncommitted, "", json!({"status": "none"})));
    insta::assert_snapshot!(render(&mut app, 90, 10));
}

#[test]
fn a_comment_typed_then_waiting() {
    let mut app = app();
    for _ in 0..3 {
        press(&mut app, 'n');
    }
    press(&mut app, 'j');
    press(&mut app, 'c');
    typed(&mut app, "keep the grace in the plan");
    let typing = render(&mut app, 100, 40);
    key(&mut app, KeyCode::Enter);
    insta::assert_snapshot!("typing", typing);
    insta::assert_snapshot!("waiting", render(&mut app, 100, 40));
}

#[test]
fn side_by_side() {
    let mut app = app();
    press(&mut app, 's');
    insta::assert_snapshot!(render(&mut app, 140, 40));
}

#[test]
fn narrow() {
    insta::assert_snapshot!(render(&mut app(), 60, 20));
}

#[test]
fn walkthrough_not_written_yet() {
    let mut app = app();
    press(&mut app, 'w');
    insta::assert_snapshot!(render(&mut app, 100, 16));
}

#[test]
fn walkthrough_being_written() {
    let mut app = app();
    press(&mut app, 'w');
    app.arrive(read(Scope::Uncommitted, UNCOMMITTED, json!({"status": "running", "model": "haiku", "since": 1})));
    insta::assert_snapshot!(render(&mut app, 100, 10));
}

#[test]
fn walkthrough_failed() {
    let mut app = app();
    press(&mut app, 'w');
    app.arrive(read(Scope::Uncommitted, UNCOMMITTED, json!({"status": "failed", "model": "haiku", "error": "Stopped after 900 seconds without an answer."})));
    insta::assert_snapshot!(render(&mut app, 100, 16));
}

#[test]
fn walkthrough_done() {
    let mut app = app();
    press(&mut app, '2');
    app.arrive(read(Scope::Branch, BRANCH, walkthrough()));
    press(&mut app, 'w');
    insta::assert_snapshot!("first_step", render(&mut app, 100, 50));
    press(&mut app, 'l');
    insta::assert_snapshot!("an_outdated_step", render(&mut app, 100, 40));
    press(&mut app, 'l');
    insta::assert_snapshot!("not_covered", render(&mut app, 100, 40));
}

#[test]
fn the_worktree_picker() {
    let mut app = app();
    press(&mut app, 'W');
    insta::assert_snapshot!(render(&mut app, 100, 20));
}

#[test]
fn the_keys_overlay_lists_the_space_keys() {
    let mut app = app();
    press(&mut app, 'n');
    press(&mut app, 'j');
    press(&mut app, 'c');
    typed(&mut app, "x");
    key(&mut app, KeyCode::Enter);
    let acts: Vec<_> = app.acts().into_iter().map(|(a, _)| a).collect();
    let text: Vec<String> = cockpit_core::help::lines("review", &acts).iter().map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect::<String>()).collect();
    let all = text.join("\n");
    for key in ["c  ", "e  ", "D  ", "S  ", "n  ", "p  ", "s  ", "z  ", "W  ", "1  ", "2  ", "3  ", "w  ", "R  "] {
        assert!(all.contains(&format!("  {key}")), "{key} in\n{all}");
    }
    // The key column is as wide as the widest key, herdr's own space keys included, which come from this machine's config.
    let row = |l: &str| match l.strip_prefix("  ").and_then(|r| r.split_once(' ')) {
        Some((key, what)) => format!("  {key}  {}", what.trim_start()),
        None => l.to_string(),
    };
    insta::assert_snapshot!(all.lines().take_while(|l| !l.starts_with("Moving")).map(row).collect::<Vec<_>>().join("\n"));
}

// ---------------------------------------------------------------------------------------------------------------------
// What the keys do

#[test]
fn the_subject_is_the_most_recently_active_agent_in_a_worktree() {
    let app = app();
    let agent = app.subject().expect("an agent");
    assert_eq!((agent.terminal.as_str(), agent.repo.as_str()), ("w2:p1", API));
    assert_eq!(app.repo.as_deref(), Some(API));
}

#[test]
fn scopes_switch_and_keep_their_comments() {
    let mut app = app();
    press(&mut app, 'n');
    press(&mut app, 'j');
    press(&mut app, 'c');
    typed(&mut app, "ok");
    key(&mut app, KeyCode::Enter);
    press(&mut app, '2');
    assert_eq!(app.scope, Scope::Branch);
    assert!(app.diff().notes().is_empty(), "each scope has its own comments");
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.scope, Scope::Pr);
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.scope, Scope::Uncommitted);
    assert_eq!(app.diff().notes().len(), 1);
}

#[test]
fn n_and_p_step_through_the_hunks() {
    let mut app = app();
    press(&mut app, 'n');
    assert!(cursor_row(&mut app).contains("# acme/api"), "{}", cursor_row(&mut app));
    press(&mut app, 'n');
    assert!(cursor_row(&mut app).contains("Daily quotas for the acme API"));
    press(&mut app, 'n');
    assert!(cursor_row(&mut app).contains("18  20"), "{}", cursor_row(&mut app));
    press(&mut app, 'p');
    assert!(cursor_row(&mut app).contains("Daily quotas for the acme API"));
}

#[test]
fn a_review_goes_to_the_agent_as_one_message() {
    let mut app = app();
    for _ in 0..3 {
        press(&mut app, 'n');
    }
    for _ in 0..3 {
        press(&mut app, 'j');
    }
    press(&mut app, 'c');
    typed(&mut app, "keep the old check");
    key(&mut app, KeyCode::Enter);
    press(&mut app, 'j');
    press(&mut app, 'c');
    typed(&mut app, "GRACE belongs to the plan");
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.diff().notes().len(), 2);
    press(&mut app, 'S');
    let sent = last(&app);
    assert_eq!(sent.action, "send");
    assert_eq!(sent.args[0], "w2:p1");
    assert_eq!(sent.args[1]["scope"], "uncommitted");
    assert_eq!(sent.args[1]["repo"], API);
    let line = cockpit_show::review::message(&sent.args[1]).unwrap();
    assert_eq!(
        line,
        "My review of the uncommitted changes (2 comments), please address each: 1) src/quotas.py:21 (removed line) `if remaining(plan, used) == 0:`: keep the old check 2) src/quotas.py:23 `if used >= daily_limit(plan) + GRACE:`: GRACE belongs to the plan"
    );
    assert_eq!(app.diff().notes().len(), 2, "waiting until the agent took it");
    app.replied("send-1", "review sent to the agent (2 comments)", None);
    assert!(app.diff().notes().is_empty());
    assert!(render(&mut app, 100, 40).contains("✓ keep the old check"));
}

#[test]
fn a_review_that_did_not_go_keeps_its_comments() {
    let mut app = app();
    press(&mut app, 'n');
    press(&mut app, 'j');
    press(&mut app, 'c');
    typed(&mut app, "ok");
    key(&mut app, KeyCode::Enter);
    press(&mut app, 'S');
    app.replied("send-1", "", Some("send: the agent's terminal did not take it".into()));
    assert_eq!(app.diff().notes().len(), 1);
    assert!(render(&mut app, 100, 40).contains("the agent's terminal did not take it"));
}

#[test]
fn nothing_to_send_says_how_to_comment() {
    let mut app = app();
    press(&mut app, 'S');
    assert!(app.sent.is_empty());
    assert!(render(&mut app, 100, 40).contains("No comment to send"));
}

#[test]
fn a_comment_is_edited_deleted_and_brought_back() {
    let mut app = app();
    press(&mut app, 'n');
    press(&mut app, 'j');
    press(&mut app, 'c');
    typed(&mut app, "first");
    key(&mut app, KeyCode::Enter);
    press(&mut app, 'e');
    typed(&mut app, " word");
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.diff().notes()[0].text, "first word");
    press(&mut app, 'D');
    assert!(app.diff().notes().is_empty());
    press(&mut app, 'u');
    assert_eq!(app.diff().notes()[0].text, "first word");
}

#[test]
fn the_walkthrough_is_written_stopped_and_stepped() {
    let mut app = app();
    press(&mut app, 'w');
    assert_eq!(app.mode, Mode::Walk);
    press(&mut app, 'm');
    press(&mut app, 'r');
    assert_eq!(last(&app), Sent { app: "review".into(), action: "walkthrough".into(), args: vec![json!(API), json!("uncommitted"), json!("sonnet")] });
    app.arrive(read(Scope::Uncommitted, UNCOMMITTED, json!({"status": "running", "model": "sonnet"})));
    press(&mut app, 's');
    assert_eq!(last(&app), Sent { app: "review".into(), action: "stop_walkthrough".into(), args: vec![json!(API), json!("uncommitted")] });
    press(&mut app, '2');
    app.arrive(read(Scope::Branch, BRANCH, walkthrough()));
    let first = render(&mut app, 100, 60);
    assert!(first.contains("src/quotas.py") && !first.contains("test_remaining_never_goes_below_zero"), "{first}");
    key(&mut app, KeyCode::Right);
    let second = render(&mut app, 100, 60);
    assert!(second.contains("test_remaining_never_goes_below_zero") && !second.contains("class QuotaExceeded"), "{second}");
    press(&mut app, 'r');
    assert_eq!(last(&app).action, "walkthrough", "an outdated walkthrough is written again with r");
    press(&mut app, 'w');
    assert!(render(&mut app, 100, 80).contains("README.md"), "back to the whole diff");
}

#[test]
fn the_picker_moves_the_review_to_another_agent() {
    let mut app = app();
    press(&mut app, 'W');
    press(&mut app, 'j');
    key(&mut app, KeyCode::Enter);
    let agent = app.subject().expect("an agent");
    assert_eq!((agent.terminal.as_str(), agent.repo.as_str()), ("w3:p1", "/work/acme/web"));
    assert!(!app.diff().has_read(), "the other worktree's changes are read afresh");
    for _ in 0..3 {
        app.set_agents(&json!({"agents": [agents()["agents"][0]]}), &toplevel);
    }
    assert_eq!(app.subject().map(|a| a.terminal.as_str()), Some("w2:p1"), "back to the most recently active once it is gone");
}

#[test]
fn the_sidebar_says_what_waits() {
    let mut app = app();
    assert_eq!(cockpit_review::pinned::sidebar_text(&app), "\u{f4af}  api · 2 files +7 −1");
    press(&mut app, 'n');
    press(&mut app, 'j');
    press(&mut app, 'c');
    typed(&mut app, "ok");
    key(&mut app, KeyCode::Enter);
    assert_eq!(cockpit_review::pinned::sidebar_text(&app), "\u{f4af}  1 comment to send");
    assert_eq!(cockpit_review::pinned::sidebar_text(&App::new(true)), "\u{f4af}  no agent yet");
}

// ---------------------------------------------------------------------------------------------------------------------
// What stays put

/// A comment on the removed `if remaining(plan, used) == 0:` line of the uncommitted changes.
fn commented(app: &mut App, text: &str) {
    for _ in 0..3 {
        press(app, 'n');
    }
    for _ in 0..3 {
        press(app, 'j');
    }
    press(app, 'c');
    typed(app, text);
    key(app, KeyCode::Enter);
}

#[test]
fn another_agent_starting_work_leaves_the_review_where_it_is() {
    let mut app = app();
    commented(&mut app, "keep the old check");
    let mut busier = agents();
    busier["agents"][1]["status"] = json!("working");
    busier["agents"][1]["since"] = json!(9000);
    app.set_agents(&busier, &toplevel);
    assert_eq!(app.subject().map(|a| a.terminal.as_str()), Some("w2:p1"), "the review stays on the agent worked on");
    assert_eq!(app.diff().notes().len(), 1, "and its comments stay");
}

#[test]
fn before_any_work_the_review_follows_the_most_recent_agent_and_says_so() {
    let mut app = app();
    let mut busier = agents();
    busier["agents"][1]["status"] = json!("working");
    busier["agents"][1]["since"] = json!(9000);
    app.set_agents(&busier, &toplevel);
    assert_eq!(app.repo.as_deref(), Some("/work/acme/web"));
    assert!(render(&mut app, 100, 40).contains("Now reviewing Quota banner · web"));
    app.set_agents(&agents(), &toplevel);
    assert!(app.diff().has_read(), "back on api, its changes are as they were left");
}

#[test]
fn an_agent_missing_from_one_poll_is_kept_and_let_go_after_a_few() {
    let mut app = app();
    commented(&mut app, "ok");
    let without = json!({"agents": [agents()["agents"][1]]});
    app.set_agents(&without, &toplevel);
    app.set_agents(&without, &toplevel);
    assert_eq!(app.subject().map(|a| a.terminal.as_str()), Some("w2:p1"), "two polls without it are a hiccup");
    app.set_agents(&agents(), &toplevel);
    for _ in 0..3 {
        app.set_agents(&without, &toplevel);
    }
    assert_eq!(app.subject().map(|a| a.terminal.as_str()), Some("w3:p1"));
    assert!(render(&mut app, 100, 40).contains("is gone"));
    assert!(app.repos.contains_key(API), "a worktree with comments waiting is kept");
}

#[test]
fn a_comment_typed_while_the_changes_move_stays_on_its_line() {
    let mut app = app();
    for _ in 0..3 {
        press(&mut app, 'n');
    }
    for _ in 0..3 {
        press(&mut app, 'j');
    }
    press(&mut app, 'c');
    typed(&mut app, "keep the old check");
    let moved = UNCOMMITTED
        .replace("@@ -1,6 +1,8 @@\n", "@@ -1,6 +1,9 @@\n")
        .replace("+GRACE = 10\n", "+GRACE = 10\n+BURST = 5\n")
        .replace("@@ -18,5 +20,5 @@", "@@ -18,5 +21,5 @@");
    app.arrive(read(Scope::Uncommitted, &moved, json!({"status": "none"})));
    key(&mut app, KeyCode::Enter);
    let notes: Vec<(usize, bool, String)> = app.diff().notes().iter().map(|n| (n.line, n.old, n.code.clone())).collect();
    assert_eq!(notes, [(21, true, "    if remaining(plan, used) == 0:".to_string())], "on the line it was typed about");
    assert!(render(&mut app, 100, 40).contains("BURST = 5"), "the read that waited for the field is taken in");
}

#[test]
fn comments_follow_their_line_and_say_when_it_is_gone() {
    let mut app = app();
    for _ in 0..3 {
        press(&mut app, 'n');
    }
    for _ in 0..4 {
        press(&mut app, 'j');
    }
    press(&mut app, 'c');
    typed(&mut app, "GRACE belongs to the plan");
    key(&mut app, KeyCode::Enter);
    let shifted = UNCOMMITTED.replace("@@ -18,5 +20,5 @@", "@@ -18,5 +22,5 @@");
    app.arrive(read(Scope::Uncommitted, &shifted, json!({"status": "none"})));
    assert_eq!(app.diff().notes().iter().map(|n| n.line).collect::<Vec<_>>(), [25], "it follows its code down two lines");
    let gone = UNCOMMITTED.replace("+    if used >= daily_limit(plan) + GRACE:", "+    if used > daily_limit(plan) + GRACE:");
    app.arrive(read(Scope::Uncommitted, &gone, json!({"status": "none"})));
    assert!(app.diff().notes().is_empty());
    assert_eq!(app.unsent(), 1, "kept, not dropped");
    let screen = render(&mut app, 100, 40);
    assert!(screen.contains("1 comment on lines no longer in the diff"), "{screen}");
    press(&mut app, 'S');
    assert!(app.sent.is_empty());
    assert!(render(&mut app, 100, 40).contains("no longer in the diff"));
}

#[test]
fn a_review_goes_once_however_often_s_is_pressed() {
    let mut app = app();
    commented(&mut app, "keep the old check");
    press(&mut app, 'S');
    press(&mut app, 'S');
    assert_eq!(app.sent.iter().filter(|s| s.action == "send").count(), 1);
    assert!(render(&mut app, 100, 40).contains("still on its way"));
    app.replied("send-1", "review sent to the agent (1 comment)", None);
    press(&mut app, 'S');
    assert_eq!(app.sent.iter().filter(|s| s.action == "send").count(), 1, "nothing left to send");
}

#[test]
fn an_agent_waiting_on_a_question_gets_no_review() {
    let mut app = new_app();
    let mut asking = agents();
    asking["agents"][0]["status"] = json!("blocked");
    asking["agents"][0]["question"] = json!({"text": "Allow the edit?", "permission": true});
    app.set_agents(&asking, &toplevel);
    app.arrive(read(Scope::Uncommitted, UNCOMMITTED, json!({"status": "none"})));
    commented(&mut app, "ok");
    press(&mut app, 'S');
    assert!(app.sent.is_empty());
    let screen = render(&mut app, 200, 40);
    assert!(screen.contains("Daily quotas: enforce limits is waiting on"), "{screen}");
}

#[test]
fn leaving_with_comments_not_sent_asks_first() {
    let mut app = app();
    assert!(app.key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)), "nothing to lose: q leaves");
    commented(&mut app, "ok");
    assert!(!app.key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)));
    assert!(render(&mut app, 100, 40).contains("Leave with 1 comment not sent?"));
    assert!(!app.key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE)), "no stays");
    assert!(!app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)));
    assert!(app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)), "ctrl+c twice leaves anyway");
    let mut again = self::app();
    commented(&mut again, "ok");
    again.key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
    assert!(again.key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE)));
}

#[test]
fn w_switches_the_view_and_shift_w_the_worktree() {
    let mut app = app();
    press(&mut app, 'w');
    assert_eq!(app.mode, Mode::Walk);
    press(&mut app, '1');
    app.arrive(read(Scope::Uncommitted, "", json!({"status": "none"})));
    press(&mut app, 'r');
    assert!(app.sent.is_empty(), "nothing to walk through on a clean scope");
    assert!(render(&mut app, 100, 12).contains("w goes back to the diff"));
    press(&mut app, 'w');
    assert_eq!(app.mode, Mode::Diff, "w leaves the walkthrough of a clean scope too");
    press(&mut app, 'W');
    assert!(app.dialog.is_some());
}

#[test]
fn a_walkthrough_asked_for_twice_starts_once() {
    let mut app = app();
    press(&mut app, 'w');
    press(&mut app, 'r');
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.sent.iter().filter(|s| s.action == "walkthrough").count(), 1);
}

// ---------------------------------------------------------------------------------------------------------------------
// Who gets the review

#[test]
fn the_review_goes_to_the_followed_agent_unless_another_is_picked() {
    let mut app = app();
    assert_eq!(app.recipient().map(|r| r.name()), Some("Daily quotas: enforce limits".to_string()));
    press(&mut app, 'T');
    let screen = render(&mut app, 100, 30);
    assert!(screen.contains("Who gets the comments on this worktree?"), "{screen}");
    assert!(screen.contains("Per-plan limits · in chat, idle") && screen.contains("A new agent, started on the review"), "{screen}");
    insta::assert_snapshot!("recipients", screen);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Enter);
    commented(&mut app, "make the grace per plan");
    press(&mut app, 'S');
    let sent = last(&app);
    assert_eq!(
        (sent.app.as_str(), sent.action.as_str(), &sent.args[0]),
        ("review", "send", &json!("claude:demo-421")),
        "a driven session takes it through deliver"
    );
    assert_eq!(sent.args[1]["to"], "Per-plan limits");
    app.replied("send-1", "review sent to Per-plan limits (1 comment)", None);
    assert!(render(&mut app, 100, 40).contains("review sent to Per-plan limits"), "the toast names who took it");
    assert!(render(&mut app, 100, 40).contains("To Per-plan limits"), "the header says who gets the next one");
}

#[test]
fn the_choice_is_remembered_for_its_worktree() {
    let mut app = app();
    app.run(Cmd::To(To::New));
    app.run(Cmd::Pick("w3:p1".into()));
    assert_eq!(app.recipient(), Some(Recipient::Agent(app.subject().unwrap().clone())), "web's comments go to its agent");
    app.run(Cmd::Pick("w2:p1".into()));
    app.run(Cmd::To(To::New));
    app.run(Cmd::Pick("w3:p1".into()));
    app.set_agents(&agents(), &toplevel);
    app.run(Cmd::Switch);
    app.dialog = None;
    app.chosen = None;
    app.set_agents(&agents(), &toplevel);
    assert_eq!(app.repo.as_deref(), Some(API));
    assert_eq!(app.recipient(), Some(Recipient::New), "api keeps the new agent it was given");
}

#[test]
fn a_new_agent_starts_on_the_review_through_start_work() {
    let mut app = app();
    app.run(Cmd::To(To::New));
    commented(&mut app, "keep the old check");
    assert!(render(&mut app, 160, 40).contains("start an agent 1"), "S says it starts an agent");
    // The first new agent of a worktree asks first: a second S starts it.
    press(&mut app, 'S');
    assert!(app.sent.is_empty(), "asked first");
    press(&mut app, 'S');
    let sent = last(&app);
    assert_eq!((sent.app.as_str(), sent.action.as_str()), ("start", "work"));
    let request = &sent.args[0];
    assert_eq!(
        (request["source"].as_str(), request["repo"].as_str(), request["worktree"].as_str(), request["dirty"].as_bool()),
        (Some("prompt"), Some(API), Some("here"), Some(true))
    );
    let label = request["label"].as_str().unwrap().to_string();
    assert!(label.starts_with("review-") && request["remember"] == false, "a space of its own, Start work's choices left alone");
    assert!(
        request["text"]
            .as_str()
            .unwrap()
            .starts_with("My review of the uncommitted changes (1 comment), please address each: 1) src/quotas.py:21 (removed line)")
    );
    app.replied("send-1", "Started api in a space of its own", None);
    assert!(app.diff().notes().is_empty(), "handed to the new agent, the comments are sent");
    // A new agent is picked once: the next review goes to the agent it started, once it runs in the worktree.
    assert_eq!(app.to.get(API), Some(&To::Started(label.clone())));
    assert_eq!(app.recipient(), Some(Recipient::Starting));
    let mut rows = agents();
    let started = json!({"terminal": "w9:p1", "frontend": "tui", "program": "claude", "title": "Review", "status": "working", "since": 4000, "cwd": API, "space": {"name": label}});
    rows["agents"].as_array_mut().unwrap().push(started);
    app.set_agents(&rows, &toplevel);
    assert!(matches!(app.recipient(), Some(Recipient::Agent(a)) if a.terminal == "w9:p1"));
}

#[test]
fn a_choice_that_works_in_another_worktree_now_is_gone_here() {
    let mut app = app();
    let quotas = app.subject().unwrap().clone();
    app.run(Cmd::To(To::Agent(quotas)));
    let mut rows = agents();
    rows["agents"][0]["cwd"] = json!("/work/acme/web");
    app.set_agents(&rows, &toplevel);
    let repo = API.to_string();
    let to = app.to.get(&repo).cloned();
    assert!(matches!(to, Some(To::Agent(_))));
    assert!(matches!(app.recipient_of(&repo), Some(Recipient::Gone(_))), "{:?}", app.recipient_of(&repo));
}

#[test]
fn an_agent_gone_since_it_was_picked_gets_nothing() {
    let mut app = app();
    commented(&mut app, "ok");
    let quotas = app.subject().unwrap().clone();
    app.run(Cmd::To(To::Agent(quotas)));
    let without = json!({"agents": [agents()["agents"][1], agents()["agents"][4]]});
    app.set_agents(&without, &toplevel);
    press(&mut app, 'S');
    assert!(app.sent.is_empty());
    let screen = render(&mut app, 160, 40);
    assert!(screen.contains("is no longer"), "{screen}");
    assert!(screen.contains("To Daily quotas: enforce limits (gone)"), "{screen}");
}

#[test]
fn a_refused_delivery_says_why_without_the_action_s_name() {
    let mut app = app();
    commented(&mut app, "ok");
    press(&mut app, 'S');
    app.replied("send-1", "", Some("send: Daily quotas is waiting on a question: answer it first".into()));
    let screen = render(&mut app, 160, 40);
    assert!(screen.contains("✗ Daily quotas is waiting on a question"), "{screen}");
    assert_eq!(app.diff().notes().len(), 1, "kept for the next try");
}
