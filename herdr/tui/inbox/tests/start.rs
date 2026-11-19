//! The Start work form drawn on plans and runs as the hub gives them: ready, blocked with its fixes, starting step by
//! step, a failed setup, narrow and scrolled; its keys; and opened from the inbox, on a ticket's row and on a start's.

mod common;

use cockpit_core::testing::render;
use cockpit_inbox::start::{Form, Outcome, Popup};
use common::{app, focus, typed};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::{Value, json};

fn modes() -> Value {
    json!([{"id": "default", "label": "Asks first"}, {"id": "acceptEdits", "label": "Edits freely"}, {"id": "plan", "label": "Plan only"}, {"id": "bypassPermissions", "label": "Full access"}])
}

fn choices() -> Value {
    json!({
        "repos": ["api", "web"], "agent": "claude", "repo": "api", "mode": "default",
        "agents": [{"id": "claude", "name": "Claude Code", "installed": true, "modes": modes()}, {"id": "codex", "name": "Codex", "installed": true, "modes": []}],
    })
}

fn plan() -> Value {
    json!({
        "ok": true, "kind": "ticket", "title": "ENG-412 Store and enforce daily quotas", "label": "eng-412", "place": "worktree",
        "repo": "api", "base": "main", "branch": "me/eng-412-store-enforce-daily-quotas", "agent": "claude", "agentName": "Claude Code",
        "mode": "plan", "choices": choices(), "options": [{"label": "Use as a prompt", "set": {"source": "prompt"}}],
        "steps": [
            {"id": "fetch", "text": "fetch origin/main"},
            {"id": "worktree", "text": "add a worktree at /home/me/.herdr/worktrees/api/me-eng-412-store-enforce-daily-quotas on a new branch me/eng-412-store-enforce-daily-quotas from origin/main"},
            {"id": "space", "text": "open a space \u{201c}eng-412\u{201d} there"},
            {"id": "setup", "text": "run \u{201c}npm ci\u{201d} in it, then the agent once it succeeds"},
            {"id": "agent", "text": "start Claude Code (plan only) on the ticket"},
        ],
        "blockers": [], "notes": [],
    })
}

fn blocked() -> Value {
    let mut p = plan();
    p["ok"] = json!(false);
    p["blockers"] = json!([
        {"step": "branch", "text": "The branch me/eng-412-store-enforce-daily-quotas exists already",
         "fixes": [{"label": "Use it", "set": {"existing": true}}, {"label": "Rename", "set": {"branch": "me/eng-412-store-enforce-daily-quotas-2"}}]},
        {"step": "setup", "text": "api's setup command changed since you last ran it: npm ci", "fixes": [{"label": "Run it", "set": {"trust": "a1"}}, {"label": "Skip it this time", "set": {"setup": false}}]},
    ]);
    p["notes"] = json!(["Your checkout of api has 2 uncommitted changes: the worktree starts from origin without them"]);
    p
}

fn run(states: [&str; 5], error: Option<&str>) -> Value {
    let steps: Vec<Value> = plan()["steps"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .zip(states)
        .map(|(mut s, st)| {
            s["state"] = json!(st);
            s
        })
        .collect();
    let failed = error.is_some();
    json!({"state": if failed { "failed" } else { "running" }, "steps": steps, "error": error.map(|e| json!({"step": "setup", "text": e})), "retry": failed, "cancel": true})
}

fn popup(form: Form) -> Popup {
    Popup { form }
}

fn press(form: &mut Form, code: KeyCode) -> Outcome {
    form.key(KeyEvent::new(code, KeyModifiers::NONE))
}

#[test]
fn a_ready_plan_shows_its_steps_before_enter() {
    let screen = render(&mut popup(Form::fixture("ENG-412", plan(), None)), 100, 34);
    assert!(screen.contains("▸ Start ⏎") && screen.contains("Use as a prompt"), "{screen}");
    insta::assert_snapshot!("start_ready", screen);
}

#[test]
fn blockers_name_their_step_and_offer_fixes() {
    let mut form = Form::fixture("ENG-412", blocked(), None);
    press(&mut form, KeyCode::Enter);
    let screen = render(&mut popup(form), 100, 36);
    assert!(screen.contains("✗ branch") && screen.contains("✗ setup"), "{screen}");
    insta::assert_snapshot!("start_blocked", screen);
}

#[test]
fn a_fix_sets_what_it_says_and_asks_again() {
    let mut form = Form::fixture("ENG-412", blocked(), None);
    press(&mut form, KeyCode::Enter);
    press(&mut form, KeyCode::Down);
    press(&mut form, KeyCode::Enter);
    assert_eq!(form.request()["branch"], "me/eng-412-store-enforce-daily-quotas-2", "Rename fills the branch");
    let mut form = Form::fixture("ENG-412", blocked(), None);
    press(&mut form, KeyCode::Enter);
    for _ in 0..2 {
        press(&mut form, KeyCode::Tab);
    }
    press(&mut form, KeyCode::Enter);
    assert_eq!(form.request()["trust"], "a1", "Run it approves the setup command");
    let mut form = Form::fixture("ENG-412", blocked(), None);
    press(&mut form, KeyCode::Enter);
    press(&mut form, KeyCode::Up);
    press(&mut form, KeyCode::Enter);
    assert_eq!(form.request()["source"], "prompt", "Use as a prompt, drawn under the source, comes before the fixes");
}

#[test]
fn digits_pick_the_options_then_the_fixes_as_drawn_away_from_a_text_field() {
    // The Herdr app's dialog numbers them the same way: 1 the option under the source, then each blocker's fixes.
    let picks = [("1", "source", "prompt"), ("2", "existing", "true"), ("3", "branch", "me/eng-412-store-enforce-daily-quotas-2"), ("4", "trust", "a1")];
    for (digit, field, value) in picks {
        let mut form = Form::fixture("ENG-412", blocked(), None);
        press(&mut form, KeyCode::Tab);
        press(&mut form, KeyCode::Char(digit.chars().next().unwrap()));
        assert_eq!(form.request()[field].to_string().trim_matches('"'), value, "{digit} picks {field}");
    }
    let mut form = Form::fixture("ENG-412", blocked(), None);
    press(&mut form, KeyCode::Char('1'));
    assert!(form.request()["source"].is_null() && form.request()["text"] == "ENG-4121", "on the source field a digit is typed, not a pick: {}", form.request());
    let screen = render(&mut popup(Form::fixture("ENG-412", blocked(), None)), 100, 36);
    assert!(screen.contains(" Use as a prompt 1 ") && screen.contains(" Use it 2 ") && screen.contains(" Skip it this time 5 "), "{screen}");
}

#[test]
fn enter_waits_for_the_plan_of_what_is_typed() {
    let mut form = Form::fixture("ENG-412", plan(), None);
    press(&mut form, KeyCode::Char('3'));
    press(&mut form, KeyCode::Enter);
    assert!(form.run.is_none(), "a plan of what was there before is not started");
}

#[test]
fn choices_cycle_with_the_arrows_and_the_mode_shows_for_an_agent_that_takes_one() {
    let mut form = Form::fixture("ENG-412", plan(), None);
    press(&mut form, KeyCode::Tab);
    press(&mut form, KeyCode::Right);
    assert_eq!(form.request()["repo"], "web");
    press(&mut form, KeyCode::Tab);
    press(&mut form, KeyCode::Char(' '));
    assert_eq!(form.request()["worktree"], "here");
    for _ in 0..3 {
        press(&mut form, KeyCode::Tab);
    }
    press(&mut form, KeyCode::Left);
    assert_eq!(form.request()["mode"], "acceptEdits");
    let screen = render(&mut popup(Form::fixture("ENG-412", plan(), None)), 100, 34);
    assert!(screen.contains("Mode      ‹"), "{screen}");
    let mut codex = plan();
    codex["agent"] = json!("codex");
    let screen = render(&mut popup(Form::fixture("ENG-412", codex, None)), 100, 34);
    assert!(!screen.contains("Mode      ‹"), "an agent whose row takes no mode shows none: {screen}");
    assert!(screen.contains("Model     "), "{screen}");
    let mut amp = plan();
    amp["agent"] = json!("amp");
    amp["choices"]["agents"].as_array_mut().unwrap().push(json!({"id": "amp", "name": "Amp", "installed": true, "models": false, "modes": []}));
    let screen = render(&mut popup(Form::fixture("ENG-412", amp, None)), 100, 34);
    assert!(!screen.contains("Model     "), "an agent whose row takes no model shows no model field: {screen}");
    assert!(matches!(press(&mut form, KeyCode::Esc), Outcome::Close(None)), "esc leaves before anything started");
}

/// Claude Code's and Codex's picks as the hub sends them: the table's models, effort levels and fast mode.
fn with_picks(agent: &str) -> Value {
    let mut p = plan();
    let claude = json!({"agent": "Claude Code", "models": [{"id": "opus", "label": "Opus"}, {"id": "haiku", "label": "Haiku"}],
                        "efforts": ["low", "medium", "high", "xhigh", "max"], "fast": true, "start": {"model": true, "effort": true, "fast": true}});
    let codex = json!({"agent": "Codex", "models": [{"id": "gpt-acme-1", "label": "GPT Acme 1", "efforts": ["low", "high"], "fast": true},
                       {"id": "gpt-acme-mini", "label": "GPT Acme mini", "efforts": [], "fast": false}],
                       "efforts": ["low", "high"], "fast": true, "start": {"model": true, "effort": true, "fast": true}});
    p["choices"]["agents"][0]["picks"] = claude;
    p["choices"]["agents"][1]["picks"] = codex;
    p["agent"] = json!(agent);
    p
}

#[test]
fn the_model_effort_and_speed_cycle_among_what_the_agent_takes_at_start() {
    let mut form = Form::fixture("ENG-412", with_picks("claude"), None);
    // Repo, Worktree, Base, Branch, Agent, then Model.
    for _ in 0..6 {
        press(&mut form, KeyCode::Tab);
    }
    press(&mut form, KeyCode::Right);
    assert_eq!(form.request()["model"], "opus");
    press(&mut form, KeyCode::Tab);
    press(&mut form, KeyCode::Left);
    assert_eq!(form.request()["effort"], "max", "back from its default to the highest");
    press(&mut form, KeyCode::Tab);
    assert!(form.request().get("fast").is_none(), "fast mode goes only once picked");
    press(&mut form, KeyCode::Char(' '));
    assert_eq!(form.request()["fast"], true);
    let screen = render(&mut popup(form), 100, 36);
    assert!(screen.contains("Model     ‹ Opus ›") && screen.contains("Effort    ‹ max ›") && screen.contains("Speed     ‹ fast ›"), "{screen}");
    insta::assert_snapshot!("start_picks", screen);

    // A model without effort levels or fast mode offers neither, and what was picked for another goes.
    let mut codex = Form::fixture("ENG-412", with_picks("codex"), None);
    for _ in 0..6 {
        press(&mut codex, KeyCode::Tab);
    }
    press(&mut codex, KeyCode::Right);
    press(&mut codex, KeyCode::Tab);
    press(&mut codex, KeyCode::Right);
    assert_eq!((codex.request()["model"].as_str(), codex.request()["effort"].as_str()), (Some("gpt-acme-1"), Some("low")));
    press(&mut codex, KeyCode::BackTab);
    press(&mut codex, KeyCode::Right);
    assert_eq!((codex.request()["model"].as_str(), codex.request()["effort"].as_str()), (Some("gpt-acme-mini"), Some("")));
    let screen = render(&mut popup(codex), 100, 36);
    assert!(!screen.contains("Effort    ‹") && !screen.contains("Speed     ‹"), "{screen}");
    let plain = render(&mut popup(Form::fixture("ENG-412", plan(), None)), 100, 34);
    assert!(plain.contains("Model     its default") && !plain.contains("Effort"), "an agent with no picks: the model is typed, no effort: {plain}");
}

#[test]
fn a_run_shows_each_step_as_it_goes() {
    let started = Form::fixture("ENG-412", plan(), Some(run(["done", "done", "done", "running", "todo"], None)));
    let screen = render(&mut popup(started), 100, 34);
    assert!(screen.contains("✓ fetch origin/main") && screen.contains("leave it going") && screen.contains("c cancel"), "{screen}");
    insta::assert_snapshot!("start_running", screen);
}

#[test]
fn esc_leaves_a_start_going_and_cancel_asks_first() {
    let mut form = Form::fixture("ENG-412", plan(), Some(run(["done", "done", "done", "running", "todo"], None)));
    assert!(matches!(press(&mut form, KeyCode::Esc), Outcome::Close(Some(note)) if note.contains("goes on")));
    let mut form = Form::fixture("ENG-412", plan(), Some(run(["done", "done", "done", "running", "todo"], None)));
    press(&mut form, KeyCode::Char('c'));
    let screen = render(&mut popup(form), 100, 34);
    assert!(screen.contains("Remove what this start made?"), "{screen}");
    let mut form = Form::fixture("ENG-412", plan(), Some(run(["done", "done", "done", "running", "todo"], None)));
    press(&mut form, KeyCode::Char('c'));
    assert!(matches!(press(&mut form, KeyCode::Char('n')), Outcome::Stay), "no keeps it");
    assert!(form.run.is_some());
    press(&mut form, KeyCode::Char('c'));
    assert!(matches!(press(&mut form, KeyCode::Char('y')), Outcome::Close(Some(note)) if note.starts_with("cancelling")));
}

#[test]
fn a_failed_setup_offers_retry_and_cancel() {
    let failed = Form::fixture(
        "ENG-412",
        plan(),
        Some(run(["done", "done", "done", "failed", "skipped"], Some("the setup command exited with 1: its output is in the space"))),
    );
    let screen = render(&mut popup(failed), 100, 34);
    assert!(screen.contains("r retry") && screen.contains("cancel, removing what it made"), "{screen}");
    insta::assert_snapshot!("start_failed", screen);
}

#[test]
fn a_start_refused_before_it_began_reads_as_failed() {
    let mut form = Form::fixture("ENG-412", plan(), Some(json!({"state": "running", "steps": plan()["steps"]})));
    form.feed_reply(Err("can't start: No checkout of billing".into()));
    form.feed_run(Value::Null);
    let screen = render(&mut popup(form), 100, 34);
    assert!(screen.contains("No checkout of billing") && screen.contains("r retry"), "{screen}");
}

#[test]
fn narrow_and_tall_it_scrolls_to_what_is_in_focus() {
    let mut form = Form::fixture("ENG-412", blocked(), None);
    press(&mut form, KeyCode::Enter);
    for _ in 0..3 {
        press(&mut form, KeyCode::Tab);
    }
    let screen = render(&mut popup(form), 60, 24);
    assert!(screen.contains("Skip it this time") && screen.contains("can't start yet") && screen.contains("↕"), "{screen}");
    insta::assert_snapshot!("start_narrow", screen);
}

#[test]
fn n_on_a_ticket_opens_the_form_on_it() {
    for ticket in ["OPS-125", "ENG-405"] {
        let mut inbox = app(false);
        focus(&mut inbox, ticket);
        typed(&mut inbox, "n");
        let screen = render(&mut inbox, 100, 40);
        assert!(screen.contains("Start work") && screen.contains(&format!("Source    {ticket}")), "{screen}");
        typed(&mut inbox, "\x1b");
        assert!(!render(&mut inbox, 100, 40).contains("Start work"), "esc closes it");
    }
}

#[test]
fn an_unfinished_start_is_a_row_to_open_retry_or_cancel() {
    let mut inbox = app(false);
    let mut inputs = common::inputs();
    inputs.starts = vec![
        json!({"id": "g1", "title": "ENG-405 Re-import the supplier catalogue monthly", "state": "failed", "error": {"step": "setup", "text": "exited with 3"}, "cancel": true}),
        json!({"id": "g2", "title": "Done already", "state": "done"}),
    ];
    inbox.feed(inputs);
    focus(&mut inbox, "start:g1");
    let screen = render(&mut inbox, 110, 60);
    assert!(screen.contains("start ENG-405 Re-import the supplier catalogue monthly: setup failed"), "{screen}");
    assert!(screen.contains("rerun") && screen.contains("cancel") && !screen.contains("Done already"), "{screen}");
    typed(&mut inbox, "o");
    assert!(render(&mut inbox, 110, 60).contains("Start work"), "open shows the form on its run");
}
