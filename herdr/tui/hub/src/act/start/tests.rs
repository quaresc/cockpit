use std::cell::RefCell;
use std::process::Command;

use super::*;

/// A checkout of `acme/api` cloned from a bare remote with one commit on main, and a home and a cache of its own.
struct Fixture {
    dir: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Fixture {
        let dir = tempfile::tempdir().expect("a temp dir");
        let root = dir.path();
        sh(root, &["git", "init", "-q", "--bare", "-b", "main", "remote.git"]);
        sh(root, &["git", "clone", "-q", "remote.git", "repos/api"]);
        let api = root.join("repos/api");
        std::fs::write(api.join("README.md"), "acme api\n").expect("a file");
        commit(&api, "first");
        sh(&api, &["git", "push", "-q", "origin", "main"]);
        sh(&api, &["git", "remote", "set-head", "origin", "main"]);
        std::fs::create_dir_all(root.join("home")).expect("a home");
        Fixture { dir }
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn api(&self) -> PathBuf {
        self.root().join("repos/api")
    }

    fn cfg(&self, start: Value) -> Config {
        Config(json!({"reposDir": self.root().join("repos"), "tickets": {"name": "Linear", "keys": ["ENG"]}, "start": start}))
    }

    fn cache(&self) -> PathBuf {
        self.root().join("cache")
    }
}

fn sh(dir: &Path, argv: &[&str]) {
    let out = Command::new(argv[0]).args(&argv[1..]).current_dir(dir).output().expect("the command runs");
    assert!(out.status.success(), "{argv:?}: {}", String::from_utf8_lossy(&out.stderr));
}

fn commit(dir: &Path, message: &str) {
    sh(dir, &["git", "add", "."]);
    sh(dir, &["git", "-c", "user.name=Maya", "-c", "user.email=maya@example.com", "commit", "-q", "-m", message]);
}

fn stub(mode: bool) -> Agent {
    let mut a = agent::builtin()[0].clone();
    a.id = "stub".into();
    a.name = "Stub".into();
    a.bin = "/bin/echo".into();
    a.aliases = vec![];
    a.models = vec![];
    a.start = vec!["{bin}".into(), "{prompt}".into()];
    a.model = vec!["--model".into(), "{model}".into()];
    a.mode = mode.then(|| vec!["--permission-mode".into(), "{mode}".into()]);
    a
}

/// A window that records what it was asked: spaces opened, lines typed, spaces closed.
#[derive(Default)]
struct Fake {
    /// Open spaces: (label, folder).
    open: Vec<(String, String)>,
    refuse: bool,
    opened: RefCell<Vec<(String, String)>>,
    typed: RefCell<Vec<String>>,
    closed: RefCell<Vec<String>>,
    focused: RefCell<Vec<String>>,
    /// Live sessions cockpit drives: (folder, key).
    sessions: Vec<(String, String)>,
}

impl Desk for Fake {
    fn space_named(&self, label: &str) -> Option<(String, String)> {
        self.open.iter().find(|(l, _)| l == label).map(|(l, f)| (format!("w-{l}"), f.clone()))
    }
    fn space_on(&self, _: &str) -> Option<String> {
        None
    }
    fn session_on(&self, path: &str) -> Option<String> {
        let real = |p: &str| std::fs::canonicalize(p).unwrap_or_else(|_| PathBuf::from(p));
        self.sessions.iter().find(|(folder, _)| real(folder) == real(path)).map(|(_, key)| key.clone())
    }
    fn focus_id(&self, space: &str) {
        self.focused.borrow_mut().push(space.into());
    }
    fn open(&self, cwd: &str, label: &str) -> Res<(String, String)> {
        if self.refuse {
            return Err("the space did not open".into());
        }
        self.opened.borrow_mut().push((cwd.into(), label.into()));
        Ok(("w1".into(), "p1".into()))
    }
    fn type_line(&self, _: &str, line: &str) -> bool {
        self.typed.borrow_mut().push(line.into());
        true
    }
    fn close(&self, space: &str) -> bool {
        self.closed.borrow_mut().push(space.into());
        true
    }
}

fn offline(_: &Value) -> Res<Value> {
    Err("gh: To get started with GitHub CLI, please run: gh auth login".into())
}

fn no_tracker(_: &str) -> Res<Option<Value>> {
    Ok(None)
}

fn no_take(_: &str, _: &Map<String, Value>) -> Res<String> {
    Ok(String::new())
}

fn inbox() -> Value {
    json!({
        "mine": [{"url": "https://github.com/acme/api/pull/2198", "repo": "api", "number": 2198, "title": "Daily quotas: store and enforce per-account limits",
                  "branch": "me/eng-412-daily-quotas", "base": "main", "isCrossRepository": false}],
        "team": [{"url": "https://github.com/acme/api/pull/77", "repo": "api", "number": 77, "title": "Typo in the README",
                  "branch": "main", "base": "main", "isCrossRepository": true, "headOwner": "omar"}],
        "tickets": [{"id": "ENG-412", "title": "Store and enforce daily quotas", "url": "https://linear.app/acme/issue/ENG-412/store-and-enforce-daily-quotas", "statusType": "started"},
                    {"id": "ENG-415", "title": "Quota reset at midnight UTC", "url": "https://linear.app/acme/issue/ENG-415/quota-reset", "statusType": "unstarted"}],
    })
}

fn world<'a>(f: &Fixture, cfg: Config, desk: &'a Fake) -> World<'a> {
    let mut moded = stub(true);
    moded.id = "moded".into();
    moded.name = "Moded".into();
    World {
        cfg,
        inbox: inbox(),
        home: f.root().join("home"),
        cache: f.cache(),
        agents: vec![stub(false), moded],
        default_agent: "stub".into(),
        desk,
        github: &offline,
        tracker: &no_tracker,
        take: no_take,
        accounts: Vec::new(),
    }
}

fn request(v: Value) -> Request {
    Request::from(&v)
}

fn blocker_steps(p: &Plan) -> Vec<&str> {
    p.blockers.iter().map(|b| b.step).collect()
}

fn shown(id: &str) -> Value {
    view(&json!({"id": id}), &|_| Value::Null)["run"].clone()
}

/// Where a run's setup leaves its exit status.
fn marker_of(id: &str) -> PathBuf {
    runs().iter().find(|r| r.id == id).map(|r| r.made.marker.clone()).expect("a run that handed its line over")
}

/// The script a run's line points at, as written.
fn script_of(line: &str) -> String {
    let path = line.trim_start_matches("sh ").trim_matches('\'');
    std::fs::read_to_string(path).expect("the start's script")
}

#[test]
fn branches_are_named_from_the_template() {
    let cfg = Config(json!({"start": {"prefix": "me"}}));
    assert_eq!(branch_name(&cfg, "ENG-412", "Store and enforce daily quotas"), "me/eng-412-store-enforce-daily-quotas");
    assert_eq!(branch_name(&cfg, "", "Fix the login redirect, again!"), "me/fix-login-redirect-again");
    let bare = Config(json!({}));
    assert_eq!(branch_name(&bare, "ENG-412", "Store quotas"), "eng-412-store-quotas", "no prefix, no empty part");
    let own = Config(json!({"start": {"branch": "{key}/{slug}"}}));
    assert_eq!(branch_name(&own, "ENG-9", "Ship it"), "eng-9/ship-it");
    assert_eq!(slug("A very long title that goes on and on"), "very-long-title-that-goes");
    assert_eq!(branch_name(&cfg, "", "« … »"), "", "nothing to name it after");
}

#[test]
fn a_model_an_agent_would_read_as_a_switch_is_never_passed() {
    let f = Fixture::new();
    let desk = Fake::default();
    let w = world(&f, f.cfg(json!({})), &desk);
    for model in ["-p", "--dangerously-skip-permissions", &"m".repeat(200)] {
        let p = plan_in(&w, &request(json!({"text": "Fix the CSV export", "repo": "api", "model": model})));
        assert!(p.model.is_empty() && p.notes.iter().any(|n| n.contains("does not take the model")), "{model:?}: {:?}", p.notes);
    }
    let p = plan_in(&w, &request(json!({"text": "Fix the CSV export", "repo": "api", "model": "acme/large"})));
    assert_eq!(p.model, "acme/large");
}

#[test]
fn a_source_is_a_pr_a_known_ticket_or_a_prompt() {
    let cfg = Config(json!({"tickets": {"keys": ["ENG"]}}));
    let data = inbox();
    let pr = |text: &str| resolve(&request(json!({"text": text})), &data, &cfg);
    let (known, _) = pr("https://github.com/acme/api/pull/2198");
    assert!(matches!(&known, Target::Pr(p) if p["branch"] == "me/eng-412-daily-quotas" && p["cross"] == false), "the inbox knows its branch: {known:?}");
    let (unknown, _) = pr("https://github.com/acme/web/pull/77/");
    assert!(matches!(&unknown, Target::Pr(p) if p["repo"] == "web" && p["number"] == 77 && p["branch"].is_null()));
    let (ticket, extra) = pr("ENG-412 keep the API side only");
    assert!(matches!(&ticket, Target::Ticket(t) if t["title"] == "Store and enforce daily quotas"));
    assert_eq!(extra, "keep the API side only");
    let (lower, _) = pr("eng-415");
    assert!(matches!(&lower, Target::Ticket(t) if t["id"] == "ENG-415"), "a configured prefix, lowercased");
    let (linear, _) = pr("https://linear.app/acme/issue/ENG-415/quota-reset");
    assert!(matches!(&linear, Target::Ticket(t) if t["id"] == "ENG-415"));
    for words in ["utf-8 handling in exports", "UTF-8 export breaks", "OPS-125 restart the worker"] {
        assert!(matches!(pr(words).0, Target::Prompt | Target::Maybe(_)), "{words}: no prefix nor inbox knows it");
    }
    assert_eq!(pr("UTF-8 export breaks").0, Target::Maybe("UTF-8".into()), "the tracker decides");
    let (forced, _) = resolve(&request(json!({"source": "prompt", "text": "ENG-412"})), &data, &cfg);
    assert_eq!(forced, Target::Prompt);
}

#[test]
fn a_key_nobody_knows_is_a_prompt_unless_the_tracker_has_it() {
    let f = Fixture::new();
    let desk = Fake::default();
    let w = world(&f, f.cfg(json!({})), &desk);
    let p = plan_in(&w, &request(json!({"text": "UTF-8 export breaks", "repo": "api"})));
    assert_eq!((p.kind, p.prompt.as_str()), ("prompt", "UTF-8 export breaks"));
    let tracked = |id: &str| -> Res<Option<Value>> {
        Ok((id == "OPS-125").then(|| json!({"identifier": "OPS-125", "title": "Restart the worker", "state": {"type": "started"}, "assignee": {"isMe": true}})))
    };
    let mut w = world(&f, f.cfg(json!({})), &desk);
    w.tracker = &tracked;
    let p = plan_in(&w, &request(json!({"text": "OPS-125", "repo": "api"})));
    assert_eq!((p.kind, p.label.as_str()), ("ticket", "ops-125"));
    assert_eq!(p.options[0], ("Use as a prompt".to_string(), json!({"source": "prompt"})));
    let asked = plan_in(&w, &request(json!({"text": "OPS-125", "repo": "api", "source": "prompt"})));
    assert_eq!(asked.kind, "prompt", "the form sends source prompt once chosen");
}

#[test]
fn a_prompt_plans_a_new_worktree_from_its_remote() {
    let f = Fixture::new();
    let desk = Fake::default();
    let w = world(&f, f.cfg(json!({"prefix": "me"})), &desk);
    let p = plan_in(&w, &request(json!({"text": "Fix the login redirect", "repo": "api"})));
    assert!(p.ok(), "{:?}", p.blockers);
    assert_eq!(
        (p.place, p.branch.as_str(), p.base.as_str(), p.label.as_str(), p.remote.as_str()),
        ("worktree", "me/fix-login-redirect", "main", "fix-login-redirect", "origin")
    );
    assert!(p.new_branch);
    assert!(p.path.ends_with("home/.herdr/worktrees/api/me-fix-login-redirect"), "{}", p.path);
    let steps: Vec<&str> = p.steps.iter().map(|(s, _)| *s).collect();
    assert_eq!(steps, ["fetch", "worktree", "space", "agent"]);
    assert_eq!(p.prompt, "Fix the login redirect");
    let alone = plan_in(&w, &request(json!({"text": "Fix it"})));
    assert_eq!(alone.repo, "api", "the only checkout needs no pick");
    sh(f.root(), &["git", "clone", "-q", "remote.git", "repos/web"]);
    let none = plan_in(&w, &request(json!({"text": "Fix it"})));
    assert_eq!(blocker_steps(&none), ["repo"], "no repo picked and none used before");
    assert_eq!(none.blockers[0].fixes.iter().map(|(l, _)| l.as_str()).collect::<Vec<_>>(), ["api", "web"], "each checkout a pick");
    let missing = plan_in(&w, &request(json!({"text": "Fix it", "repo": "billing"})));
    assert!(missing.blockers[0].text.starts_with("No checkout of billing"), "{:?}", missing.blockers);
    let outside = plan_in(&w, &request(json!({"text": "Fix it", "repo": "/etc"})));
    assert_eq!(blocker_steps(&outside), ["repo"], "only the repos folder's checkouts");
    let nope = plan_in(&w, &request(json!({"text": "Fix it", "repo": "api", "base": "nope"})));
    assert_eq!(blocker_steps(&nope), ["base"], "a typed base the remote lacks is refused before any fetch");
    sh(&f.api(), &["git", "remote", "set-head", "origin", "--delete"]);
    sh(&f.api(), &["git", "update-ref", "refs/remotes/origin/master", "HEAD"]);
    let unsure = plan_in(&w, &request(json!({"text": "Fix it", "repo": "api"})));
    assert_eq!(blocker_steps(&unsure), ["base"], "neither HEAD nor a single main or master: the base is asked");
    assert_eq!(unsure.blockers[0].fixes.iter().map(|(l, _)| l.as_str()).collect::<Vec<_>>(), ["main", "master"]);
    sh(&f.api(), &["git", "update-ref", "-d", "refs/remotes/origin/master"]);
    let empty = plan_in(&w, &request(json!({"text": "", "repo": "api"})));
    assert_eq!(blocker_steps(&empty), ["empty"]);
    let bare = plan_in(&w, &request(json!({"text": "", "repo": "api", "worktree": "here"})));
    assert!(bare.ok() && bare.prompt.is_empty(), "an agent with no prompt, in the checkout: {:?}", bare.blockers);
    assert!(bare.steps.iter().any(|(_, t)| t.ends_with("with no prompt")));
}

#[test]
fn a_start_nobody_watches_never_guesses_the_repo() {
    let f = Fixture::new();
    sh(f.root(), &["git", "clone", "-q", "remote.git", "repos/web"]);
    std::fs::create_dir_all(f.cache()).expect("a cache");
    std::fs::write(f.cache().join("start.json"), r#"{"repo": "web"}"#).expect("the last start");
    let desk = Fake::default();
    let w = world(&f, f.cfg(json!({})), &desk);
    assert_eq!(plan_in(&w, &request(json!({"text": "ENG-415"}))).repo, "web", "a form picks the last repo, in view");
    let legacy = plan_in(&w, &request(json!({"text": "ENG-415", "lastRepo": false})));
    assert_eq!(blocker_steps(&legacy), ["repo"]);
    assert!(legacy.blockers[0].text.contains("start it from the Start work form"));
}

#[test]
fn an_existing_branch_is_used_or_renamed() {
    let f = Fixture::new();
    sh(&f.api(), &["git", "branch", "me/eng-415-quota-reset-midnight-utc"]);
    let desk = Fake::default();
    let w = world(&f, f.cfg(json!({"prefix": "me"})), &desk);
    let p = plan_in(&w, &request(json!({"text": "ENG-415", "repo": "api"})));
    assert_eq!(blocker_steps(&p), ["branch"]);
    let fixes: Vec<(&str, &Value)> = p.blockers[0].fixes.iter().map(|(l, v)| (l.as_str(), v)).collect();
    assert_eq!(fixes, [("Use it", &json!({"existing": true})), ("Rename", &json!({"branch": "me/eng-415-quota-reset-midnight-utc-2"}))]);
    let used = plan_in(&w, &request(json!({"text": "ENG-415", "repo": "api", "existing": true})));
    assert!(used.ok() && !used.new_branch, "{:?}", used.blockers);
    assert!(!used.has("fetch"), "a branch only here is not fetched");
    let desk_used = Fake::default();
    let w_used = world(&f, f.cfg(json!({"prefix": "me"})), &desk_used);
    run_in(&w_used, &json!({"id": "t-used", "text": "ENG-415", "repo": "api", "existing": true})).expect("it starts on the branch");
    assert!(has_ref(&f.api(), "refs/heads/me/eng-415-quota-reset-midnight-utc"), "a branch it did not make stays");
    cancel_in(&w_used, "t-used").expect_err("a finished start is not cancelled");
    let renamed = plan_in(&w, &request(json!({"text": "ENG-415", "repo": "api", "branch": "me/eng-415-quota-reset-midnight-utc-2"})));
    assert!(renamed.ok() && renamed.new_branch);
    assert!(renamed.prompt.starts_with("Start ENG-415 (https://linear.app/acme/issue/ENG-415/quota-reset): Quota reset at midnight UTC."));
    assert!(renamed.prompt.contains("First move ENG-415 to In Progress"), "no tracker token: the agent is asked");
}

#[test]
fn a_dirty_checkout_blocks_work_in_it_only() {
    let f = Fixture::new();
    std::fs::write(f.api().join("README.md"), "changed\n").expect("a change");
    let desk = Fake::default();
    let w = world(&f, f.cfg(json!({})), &desk);
    let here = plan_in(&w, &request(json!({"text": "Look around", "repo": "api", "worktree": "here"})));
    assert_eq!(blocker_steps(&here), ["worktree"]);
    assert!(here.blockers[0].text.contains("1 uncommitted change"), "{}", here.blockers[0].text);
    let anyway = plan_in(&w, &request(json!({"text": "Look around", "repo": "api", "worktree": "here", "dirty": true})));
    assert!(anyway.ok() && anyway.place == "here");
    let new = plan_in(&w, &request(json!({"text": "Look around", "repo": "api"})));
    assert!(new.ok() && new.notes.iter().any(|n| n.contains("starts from origin without them")));
}

#[test]
fn an_offline_gh_blocks_a_pr_the_inbox_does_not_know() {
    let f = Fixture::new();
    let desk = Fake::default();
    let w = world(&f, f.cfg(json!({})), &desk);
    let unknown = plan_in(&w, &request(json!({"text": "https://github.com/acme/api/pull/2301"})));
    assert_eq!(blocker_steps(&unknown), ["github"]);
    assert!(unknown.blockers[0].text.contains("gh auth login"));
    sh(&f.api(), &["git", "push", "-q", "origin", "main:me/eng-412-daily-quotas"]);
    sh(&f.api(), &["git", "fetch", "-q"]);
    let known = plan_in(&w, &request(json!({"text": "https://github.com/acme/api/pull/2198"})));
    assert!(known.ok(), "the inbox's PR needs no gh: {:?}", known.blockers);
    assert_eq!((known.label.as_str(), known.branch.as_str(), known.remote_only), ("api#2198", "me/eng-412-daily-quotas", true));
    let mut no_fork_word = inbox();
    no_fork_word["mine"][0].as_object_mut().expect("a PR").remove("isCrossRepository");
    let mut w = world(&f, f.cfg(json!({})), &desk);
    w.inbox = no_fork_word;
    let unsure = plan_in(&w, &request(json!({"text": "https://github.com/acme/api/pull/2198"})));
    assert_eq!(blocker_steps(&unsure), ["github"], "whether it comes from a fork is GitHub's to say");
}

#[test]
fn a_fork_pr_is_fetched_apart_and_never_lands_on_a_branch_of_yours() {
    let f = Fixture::new();
    let fork = f.root().join("fork");
    sh(f.root(), &["git", "clone", "-q", "remote.git", "fork"]);
    std::fs::write(fork.join("README.md"), "acme api, with a typo fixed\n").expect("a change");
    commit(&fork, "typo");
    sh(&fork, &["git", "push", "-q", "origin", "HEAD:refs/pull/77/head"]);
    let main_before = tip(&f.api(), "refs/heads/main");
    let desk = Fake::default();
    let w = world(&f, f.cfg(json!({"setup": {"api": "make deps"}})), &desk);
    let p = plan_in(&w, &request(json!({"text": "https://github.com/acme/api/pull/77"})));
    assert!(p.ok(), "{:?}", p.blockers);
    assert_eq!((p.branch.as_str(), p.cross.as_deref(), p.setup.is_none()), ("pr/77", Some("77"), true), "a fork's setup stays off");
    assert!(p.options.iter().any(|(l, _)| l == "Run setup from the fork's tree"));
    assert!(p.notes.iter().any(|n| n.contains("post-checkout")));
    run_in(&w, &json!({"id": "t-fork", "text": "https://github.com/acme/api/pull/77"})).expect("it starts");
    let path = f.root().join("home/.herdr/worktrees/api/pr-77");
    assert_eq!(std::fs::read_to_string(path.join("README.md")).expect("the fork's tree"), "acme api, with a typo fixed\n");
    assert_eq!(tip(&f.api(), "refs/heads/main"), main_before, "main is never moved by a fork's PR from main");
    assert_eq!(git(&f.api(), &["branch", "--show-current"]).stdout.trim(), "main");
    cancel_in(&w, "t-fork").expect_err("it started");
    sh(&f.api(), &["git", "worktree", "remove", "--force", &path.to_string_lossy()]);
    sh(&f.api(), &["git", "branch", "-f", "pr/77", "main"]);
    let other = plan_in(&w, &request(json!({"text": "https://github.com/acme/api/pull/77"})));
    assert_eq!(blocker_steps(&other), ["branch"], "a pr/77 of yours that is not its head");
}

#[test]
fn a_folder_in_the_way_takes_another_folder_even_for_a_pr() {
    let f = Fixture::new();
    sh(&f.api(), &["git", "push", "-q", "origin", "main:me/eng-412-daily-quotas"]);
    sh(&f.api(), &["git", "fetch", "-q"]);
    let taken = f.root().join("home/.herdr/worktrees/api/me-eng-412-daily-quotas");
    std::fs::create_dir_all(&taken).expect("a folder in the way");
    let desk = Fake::default();
    let w = world(&f, f.cfg(json!({})), &desk);
    let p = plan_in(&w, &request(json!({"text": "https://github.com/acme/api/pull/2198"})));
    assert_eq!(blocker_steps(&p), ["worktree"]);
    let (label, set) = p.blockers[0].fixes[0].clone();
    assert_eq!(label, "Use another folder");
    assert_eq!(p.blockers[0].fixes.len(), 1, "a PR's branch is not renamed");
    let mut asked = json!({"id": "t-folder", "text": "https://github.com/acme/api/pull/2198"});
    asked["path"] = set["path"].clone();
    run_in(&w, &asked).expect("it starts beside");
    assert!(f.root().join("home/.herdr/worktrees/api/me-eng-412-daily-quotas-2/README.md").exists());
}

#[test]
fn a_setup_command_asks_once_until_it_changes() {
    let f = Fixture::new();
    let desk = Fake::default();
    let w = world(&f, f.cfg(json!({"setup": {"api": "echo ready"}})), &desk);
    let asked = plan_in(&w, &request(json!({"text": "Add a health check", "repo": "api"})));
    assert_eq!(blocker_steps(&asked), ["setup"]);
    assert!(asked.blockers[0].text.contains("is new here: echo ready"));
    let h = hash("echo ready");
    assert_eq!(asked.blockers[0].fixes[0], ("Run it".to_string(), json!({"trust": h})));
    let steps: Vec<&str> = asked.steps.iter().map(|(s, _)| *s).collect();
    assert_eq!(steps, ["fetch", "worktree", "space", "setup", "agent"], "in the order they run");
    let id = "t-setup-once";
    run_in(&w, &json!({"id": id, "text": "Add a health check", "repo": "api", "trust": h})).expect("it starts");
    let script = script_of(&desk.typed.borrow()[0]);
    let setup = format!("\n{} setup-run /bin/sh -c 'echo ready'\ns=$?\n", quote(&cockpit_bin()));
    assert!(script.contains(&setup), "the setup runs as cockpit's own work, never as the person's: {script}");
    let again = plan_in(&w, &request(json!({"text": "Add another check", "repo": "api"})));
    assert!(again.ok(), "approved once: {:?}", again.blockers);
    let changed = world(&f, f.cfg(json!({"setup": {"api": "echo ready && make deps"}})), &desk);
    let asked = plan_in(&changed, &request(json!({"text": "Add another check", "repo": "api"})));
    assert!(asked.blockers[0].text.contains("changed since you last ran it"), "{:?}", asked.blockers);
    let skipped = plan_in(&changed, &request(json!({"text": "Add another check", "repo": "api", "setup": false})));
    assert!(skipped.ok() && skipped.setup.is_none());
    cancel_in(&w, id).expect("cancelled");
}

#[test]
fn a_start_passes_the_effort_level_and_fast_mode_picked_and_notes_what_its_agent_does_not_take() {
    let f = Fixture::new();
    let desk = Fake::default();
    let w = world(&f, f.cfg(json!({"prefix": "me"})), &desk);
    let planned = plan_in(&w, &request(json!({"text": "Add the reset job", "repo": "api", "effort": "high", "fast": true})));
    assert_eq!((planned.effort.as_str(), planned.fast), ("high", Some(true)));
    assert!(planned.steps.iter().any(|(id, t)| *id == "agent" && t.contains("(high effort, fast mode)")), "{:?}", planned.steps);
    let odd = plan_in(&w, &request(json!({"text": "Add the reset job", "repo": "api", "effort": "ultra", "fast": false})));
    assert_eq!((odd.effort.as_str(), odd.fast), ("", Some(false)));
    assert!(odd.notes.iter().any(|n| n.contains("takes no effort level ultra")), "{:?}", odd.notes);
    run_in(&w, &json!({"id": "t-picks", "text": "Add the reset job", "repo": "api", "effort": "high", "fast": true})).expect("it starts");
    let script = script_of(&desk.typed.borrow()[0]);
    assert!(script.ends_with("--effort high --settings '{\"fastMode\":true}'\n"), "{script}");
}

#[test]
fn a_run_makes_one_worktree_one_space_and_one_agent() {
    let f = Fixture::new();
    let desk = Fake::default();
    let w = world(&f, f.cfg(json!({"prefix": "me"})), &desk);
    let note = run_in(&w, &json!({"id": "t-run", "text": "ENG-415 start with the cron", "repo": "api", "model": "fast"})).expect("it starts");
    assert_eq!(note, "started Stub in \u{201c}eng-415\u{201d}");
    let path = f.root().join("home/.herdr/worktrees/api/me-eng-415-quota-reset-midnight-utc");
    assert!(path.join("README.md").exists());
    assert_eq!(desk.opened.borrow().as_slice(), [(path.to_string_lossy().into_owned(), "eng-415".to_string())]);
    let line = desk.typed.borrow()[0].clone();
    assert!(line.starts_with("sh ") && !line.contains("ENG-415"), "nothing from the ticket is typed: {line}");
    let script = script_of(&line);
    assert!(script.ends_with("exec /bin/echo ''\"$(cat \"$P\")\"'' --model fast\n"), "{script}");
    let file = script.lines().find_map(|l| l.strip_prefix("P=")).expect("the prompt's file").trim_matches('\'').to_string();
    assert!(file.starts_with(&f.cache().join("start/t-run-").to_string_lossy().into_owned()) && file.ends_with(".prompt"), "a name of its own: {file}");
    let prompt = std::fs::read_to_string(&file).expect("the prompt's file");
    assert!(prompt.starts_with("Start ENG-415") && prompt.contains("start with the cron"), "{prompt}");
    let run = shown("t-run");
    assert_eq!(run["state"], "done");
    assert!(list(&run["steps"]).iter().all(|s| s["state"] == "done"), "{run}");
    let folder = path.to_string_lossy().into_owned();
    let open = Fake { open: vec![("eng-415".into(), folder.clone())], ..Fake::default() };
    let again = world(&f, f.cfg(json!({"prefix": "me"})), &open);
    assert_eq!(run_in(&again, &json!({"text": "ENG-415", "repo": "api"})).expect("focused"), "\u{201c}eng-415\u{201d} already runs: brought it into view");
    assert_eq!(open.focused.borrow().as_slice(), ["w-eng-415"]);
    assert!(open.opened.borrow().is_empty(), "pressing again makes nothing");
    let named = plan_in(&w, &request(json!({"text": "Review the quota branch", "repo": "api", "label": "review · api"})));
    assert_eq!(named.label, "review · api", "a caller names the space");
    let elsewhere = Fake { open: vec![("eng-415".into(), f.root().to_string_lossy().into_owned())], ..Fake::default() };
    let other = world(&f, f.cfg(json!({"prefix": "me"})), &elsewhere);
    let p = plan_in(&other, &request(json!({"text": "ENG-415", "repo": "api", "branch": "me/eng-415-again"})));
    assert_eq!((p.place, p.label.as_str()), ("worktree", "eng-415 (api)"), "a space of that name on another folder is another start");
}

/// Start again on a ticket whose worktree a session cockpit drives works in: that session comes up, nothing is made.
#[test]
fn pressing_start_again_brings_up_the_session_cockpit_drives_there() {
    let f = Fixture::new();
    let desk = Fake::default();
    let w = world(&f, f.cfg(json!({"prefix": "me"})), &desk);
    run_in(&w, &json!({"id": "t-first", "text": "ENG-415", "repo": "api"})).expect("it starts");
    let path = f.root().join("home/.herdr/worktrees/api/me-eng-415-quota-reset-midnight-utc");
    let driving = Fake { sessions: vec![(path.to_string_lossy().into_owned(), "claude:1794-0".into())], ..Fake::default() };
    let again = world(&f, f.cfg(json!({"prefix": "me"})), &driving);
    let p = plan_in(&again, &request(json!({"text": "ENG-415", "repo": "api", "driven": true})));
    assert_eq!((p.place, p.session, p.space.as_str()), ("space", true, "claude:1794-0"), "{:?}", p.steps);
    let note = run_in(&again, &json!({"id": "t-again", "text": "ENG-415", "repo": "api", "driven": true})).expect("brought up");
    assert_eq!(note, "\u{201c}eng-415\u{201d} is open already: brought it up");
    assert_eq!(driving.focused.borrow().as_slice(), ["claude:1794-0"]);
    assert!(driving.opened.borrow().is_empty() && driving.typed.borrow().is_empty(), "nothing is made a second time");
    let run = shown("t-again");
    assert_eq!((run["state"].as_str(), run["made"]["pane"].as_str()), (Some("done"), Some("claude:1794-0")), "the chat that asked follows it: {run}");
}

#[test]
fn a_mode_goes_only_to_an_agent_whose_row_takes_one() {
    let f = Fixture::new();
    let desk = Fake::default();
    let w = world(&f, f.cfg(json!({})), &desk);
    let plain = plan_in(&w, &request(json!({"text": "Tidy the logs", "repo": "api", "mode": "bypassPermissions"})));
    assert!(plain.ok() && plain.mode.is_empty(), "no mode column, no mode and no confirm: {:?}", plain.blockers);
    let moded = plan_in(&w, &request(json!({"text": "Tidy the logs", "repo": "api", "agent": "moded", "mode": "bypassPermissions"})));
    assert_eq!(blocker_steps(&moded), ["agent"], "full access is confirmed first");
    let confirmed = plan_in(&w, &request(json!({"text": "Tidy the logs", "repo": "api", "agent": "moded", "mode": "plan"})));
    assert!(confirmed.steps.iter().any(|(_, t)| t.contains("(plan only)")), "{:?}", confirmed.steps);
    let script = script_for(&confirmed, Path::new("/m"), Path::new("/p"));
    assert!(script.ends_with("--permission-mode plan\n"), "{script}");
    assert_eq!(modes_of(&stub(false)), Vec::<Value>::new());
    // A mode cockpit does not offer is refused, never handed to the agent's command line.
    let odd = plan_in(&w, &request(json!({"text": "Tidy the logs", "repo": "api", "agent": "moded", "mode": "plan --dangerously-skip-permissions"})));
    assert_eq!(blocker_steps(&odd), ["agent"]);
    assert!(odd.blockers[0].text.starts_with("No mode \"plan --dangerously-skip-permissions\""), "{:?}", odd.blockers);
    assert!(!script_for(&odd, Path::new("/m"), Path::new("/p")).contains("dangerously"));
    // Auto runs tools without asking too: confirmed like full access.
    let auto = plan_in(&w, &request(json!({"text": "Tidy the logs", "repo": "api", "agent": "moded", "mode": "auto"})));
    assert_eq!(blocker_steps(&auto), ["agent"]);
    assert_eq!(auto.blockers[0].fixes[0], ("Allow auto mode".to_string(), json!({"confirm": true})));
    let allowed = plan_in(&w, &request(json!({"text": "Tidy the logs", "repo": "api", "agent": "moded", "mode": "auto", "confirm": true})));
    assert!(allowed.ok(), "{:?}", allowed.blockers);
}

#[test]
fn a_refused_start_is_recorded_so_every_surface_can_say_why() {
    let f = Fixture::new();
    let desk = Fake::default();
    let w = world(&f, f.cfg(json!({})), &desk);
    let err = run_in(&w, &json!({"id": "t-refused", "text": "Look around", "repo": "billing"})).expect_err("no checkout");
    assert!(err.starts_with("can't start: No checkout of billing"), "{err}");
    let run = shown("t-refused");
    assert_eq!((run["state"].as_str(), run["error"]["step"].as_str(), run["retry"].as_bool()), (Some("failed"), Some("plan"), Some(true)));
    retry_in(&w, "t-refused").expect_err("still no checkout");
    assert_eq!(cancel_in(&w, "t-refused").expect("cancelled"), "cancelled", "nothing made, nothing to remove");
    let bad = run_in(&w, &json!({"id": "../escape", "text": "Look around", "repo": "billing"})).expect_err("refused");
    assert!(!bad.is_empty());
    assert!(shown("../escape").is_null(), "an id unfit for a file name is replaced");
}

#[test]
fn a_live_run_is_never_dropped_nor_evicted() {
    let f = Fixture::new();
    let desk = Fake::default();
    let w = world(&f, f.cfg(json!({})), &desk);
    let plan = Plan { label: "x".into(), main: f.api(), ..Plan::default() };
    begin(&w, "t-live", &plan, &json!({})).expect("a run");
    assert_eq!(begin(&w, "t-live", &plan, &json!({})).expect_err("live"), "this start runs already");
    for n in 0..35 {
        let id = format!("t-over-{n}");
        begin(&w, &id, &plan, &json!({})).expect("a run");
        update(&id, |r| r.phase = Phase::Over);
    }
    assert_eq!(shown("t-live")["state"], "running", "a live run outlasts thirty finished ones");
}

#[test]
fn a_failed_step_leaves_nothing_half_made() {
    let f = Fixture::new();
    let refusing = Fake { refuse: true, ..Fake::default() };
    let w = world(&f, f.cfg(json!({})), &refusing);
    let err = run_in(&w, &json!({"id": "t-space", "text": "Tidy the logs", "repo": "api"})).expect_err("no space");
    assert!(err.starts_with("space: the space did not open"), "{err}");
    assert!(err.contains("deleted the branch tidy-logs"), "{err}");
    assert!(!f.root().join("home/.herdr/worktrees/api/tidy-logs").exists(), "the worktree it made is gone");
    assert!(!has_ref(&f.api(), "refs/heads/tidy-logs"), "and its branch");
    let run = shown("t-space");
    assert_eq!((run["state"].as_str(), run["error"]["step"].as_str(), run["retry"].as_bool()), (Some("failed"), Some("space"), Some(true)));
    assert!(list(&run["steps"]).iter().filter(|s| s["id"] == "agent").all(|s| s["state"] == "skipped"), "the steps after it are skipped: {run}");

    let desk = Fake::default();
    let w = world(&f, f.cfg(json!({})), &desk);
    sh(&f.api(), &["git", "remote", "set-url", "origin", "/nowhere/remote.git"]);
    let err = run_in(&w, &json!({"id": "t-fetch", "text": "Tidy the logs", "repo": "api"})).expect_err("no remote");
    assert!(err.starts_with("fetch: git fetch:"), "{err}");
    assert!(desk.opened.borrow().is_empty() && !f.root().join("home/.herdr/worktrees/api/tidy-logs").exists());
}

#[test]
fn a_worktree_add_cut_short_is_cleaned_up() {
    let f = Fixture::new();
    let path = f.root().join("home/.herdr/worktrees/api/half");
    std::fs::create_dir_all(&path).expect("what a cut-short add leaves");
    let made = Made { main: f.api(), worktree: Some(path.to_string_lossy().into_owned()), ..Made::default() };
    let done = undo(&Fake::default(), &made);
    assert!(!path.exists(), "{done:?}");
}

#[test]
fn cancel_removes_what_the_start_made_and_retry_runs_setup_again() {
    let f = Fixture::new();
    let desk = Fake::default();
    let h = hash("make deps");
    let w = world(&f, f.cfg(json!({"setup": {"api": "make deps"}})), &desk);
    let note = run_in(&w, &json!({"id": "t-cancel", "text": "Bump the deps", "repo": "api", "trust": h})).expect("it starts");
    assert!(note.contains("starts once its setup succeeds"), "{note}");
    let path = f.root().join("home/.herdr/worktrees/api/bump-deps");
    assert!(path.exists());
    std::fs::write(marker_of("t-cancel"), "2").expect("the setup's exit status");
    let failed = wait_for("t-cancel", "failed");
    assert_eq!(failed["error"]["step"], "setup");
    assert!(path.exists(), "a failed setup keeps its space and worktree, to read its output");
    retry_in(&w, "t-cancel").expect("it runs again");
    assert_eq!(desk.typed.borrow().len(), 2, "the same line, typed again");
    assert_eq!(desk.typed.borrow()[0], desk.typed.borrow()[1]);
    let note = cancel_in(&w, "t-cancel").expect("cancelled");
    assert!(note.contains("removed") && note.contains("deleted the branch bump-deps"), "{note}");
    assert_eq!(desk.closed.borrow().as_slice(), ["w1"]);
    assert!(!path.exists() && !has_ref(&f.api(), "refs/heads/bump-deps"));
    assert_eq!(shown("t-cancel")["state"], "cancelled");
}

/// Cancel during a driven start's setup ends the setup's process group first, then removes the worktree, and the run
/// stays cancelled whatever the setup's end says. The hub is kept alive meanwhile.
#[test]
fn cancel_ends_a_driven_setup_before_its_worktree_goes() {
    let f = Fixture::new();
    let desk = Fake::default();
    let pids = f.root().join("pids");
    let command = format!("echo $$ > {p}; sleep 30 & echo $! >> {p}; wait", p = pids.display());
    let w = world(&f, f.cfg(json!({"setup": {"api": command.clone()}})), &desk);
    let note = run_in(&w, &json!({"id": "t-dcancel", "text": "Bump the deps", "repo": "api", "driven": true, "trust": hash(&command)})).expect("it starts");
    assert!(note.ends_with("the session starts once its worktree is ready"), "{note}");
    let read = || std::fs::read_to_string(&pids).unwrap_or_default().lines().filter_map(|l| l.trim().parse::<i32>().ok()).collect::<Vec<_>>();
    let started = Instant::now();
    while read().len() < 2 && started.elapsed() < Duration::from_secs(20) {
        std::thread::sleep(Duration::from_millis(50));
    }
    let setup = read();
    assert_eq!(setup.len(), 2, "the setup and its child run");
    assert!(running(), "the hub stays while it runs a start itself");
    let path = f.root().join("home/.herdr/worktrees/api/bump-deps");
    assert!(path.exists());
    let note = cancel_in(&w, "t-dcancel").expect("cancelled");
    assert!(note.contains("removed") && note.contains("deleted the branch bump-deps"), "{note}");
    assert!(!path.exists());
    for pid in setup {
        let gone = Instant::now();
        // SAFETY: signal 0 only asks whether the process is there.
        while unsafe { libc::kill(pid, 0) } == 0 && gone.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_ne!(unsafe { libc::kill(pid, 0) }, 0, "the setup's process {pid} was ended");
    }
    std::thread::sleep(Duration::from_millis(300));
    let run = shown("t-dcancel");
    assert_eq!(run["state"], "cancelled", "the setup's end never overwrites a cancel: {run}");
}

#[test]
fn a_handed_over_worktree_with_changes_is_kept() {
    let f = Fixture::new();
    let desk = Fake::default();
    let w = world(&f, f.cfg(json!({"setup": {"api": "make deps"}})), &desk);
    run_in(&w, &json!({"id": "t-kept", "text": "Bump the deps", "repo": "api", "trust": hash("make deps")})).expect("it starts");
    let path = f.root().join("home/.herdr/worktrees/api/bump-deps");
    std::fs::write(path.join("README.md"), "edited by hand\n").expect("work of the person's");
    let note = cancel_in(&w, "t-kept").expect("cancelled");
    assert!(note.contains("kept") && note.contains("it has changes"), "{note}");
    assert!(path.join("README.md").exists(), "never forced once handed over");
}

#[test]
fn runs_come_back_after_a_restart() {
    let f = Fixture::new();
    std::fs::create_dir_all(f.cache()).expect("a cache");
    let marker = f.root().join("t-term.setup");
    let steps = json!([{"id": "worktree", "text": "add", "state": "done"}, {"id": "setup", "text": "make deps", "state": "running"}, {"id": "agent", "text": "start", "state": "todo"}]);
    let kept = json!({"runs": [
        {"id": "t-cut", "state": "running", "setup": false, "steps": [{"id": "fetch", "text": "fetch", "state": "running"}], "made": {}, "request": {}},
        {"id": "t-done", "state": "done", "steps": [], "made": {}, "request": {}},
        // A driven start's setup ran in the hub, and its session was not started yet.
        {"id": "t-driven", "state": "running", "setup": true, "steps": steps, "made": {"worktree": "/w/api/bump-deps"}, "request": {"driven": true}},
        {"id": "t-term", "state": "running", "setup": true, "steps": steps, "made": {"pane": "p1", "line": "sh t-term.sh", "marker": marker}, "request": {}},
    ]});
    std::fs::write(f.cache().join("start-runs.json"), kept.to_string()).expect("the kept runs");
    load_runs(&f.cache());
    let cut = shown("t-cut");
    assert_eq!((cut["state"].as_str(), cut["retry"].as_bool()), (Some("failed"), Some(true)), "{cut}");
    assert_eq!(shown("t-done")["state"], "done");
    let driven = shown("t-driven");
    assert_eq!((driven["state"].as_str(), driven["retry"].as_bool(), driven["cancel"].as_bool()), (Some("failed"), Some(true), Some(true)), "{driven}");
    assert_eq!(driven["error"]["step"], "setup");
    assert!(driven["error"]["text"].as_str().unwrap().starts_with("the hub restarted during it"), "{driven}");
    let states: Vec<&str> = list(&driven["steps"]).iter().map(|s| s["state"].as_str().unwrap()).collect();
    assert_eq!(states, ["done", "failed", "skipped"]);
    let term = shown("t-term");
    assert_eq!((term["state"].as_str(), term["setup"].as_bool()), (Some("running"), Some(true)), "a terminal's setup goes on: {term}");
    std::fs::write(&marker, "0").expect("its setup succeeded meanwhile");
    assert_eq!(wait_for("t-term", "done")["state"], "done", "and is followed again");
}

#[test]
fn a_ticket_moves_only_once_setup_succeeds() {
    fn taking(id: &str, _: &Map<String, Value>) -> Res<String> {
        Ok(format!("{id} moved"))
    }
    let tracked = |_: &str| -> Res<Option<Value>> { Ok(Some(json!({"identifier": "ENG-415", "state": {"type": "unstarted"}, "assignee": {"isMe": true}}))) };
    let f = Fixture::new();
    let desk = Fake::default();
    let mut w = world(&f, f.cfg(json!({"setup": {"api": "make deps"}})), &desk);
    w.tracker = &tracked;
    w.take = taking;
    run_in(&w, &json!({"id": "t-move", "text": "ENG-415", "repo": "api", "trust": hash("make deps")})).expect("it starts");
    let waiting = shown("t-move");
    assert!(list(&waiting["steps"]).iter().any(|s| s["id"] == "ticket" && s["state"] == "todo"), "{waiting}");
    std::fs::write(marker_of("t-move"), "0").expect("setup succeeded");
    let done = wait_for("t-move", "done");
    assert!(list(&done["steps"]).iter().any(|s| s["id"] == "ticket" && s["note"] == "ENG-415 moved"), "{done}");
}

fn wait_for(id: &str, state: &str) -> Value {
    for _ in 0..50 {
        let run = shown(id);
        if run["state"] == state {
            return run;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("{id} never became {state}");
}

#[test]
fn the_choices_say_which_agents_cockpit_drives_itself() {
    let f = Fixture::new();
    let desk = Fake::default();
    let mut w = world(&f, f.cfg(json!({})), &desk);
    w.agents[1].stream = None;
    let agents = choices(&w)["agents"].clone();
    let drives: Vec<(String, bool)> = agents.as_array().unwrap().iter().map(|a| (a["id"].as_str().unwrap().to_string(), a["drives"] == true)).collect();
    assert_eq!(drives, vec![("stub".to_string(), true), ("moded".to_string(), false)]);
}

#[test]
fn an_agent_that_takes_no_prompt_as_it_starts_is_never_said_to_start_on_the_work() {
    let f = Fixture::new();
    let desk = Fake::default();
    let mut w = world(&f, f.cfg(json!({"prefix": "me"})), &desk);
    let mut bare = stub(false);
    (bare.id, bare.name, bare.start) = ("bare".into(), "Bare".into(), vec!["{bin}".into()]);
    w.agents.push(bare);
    let step = |p: &Plan| p.steps.iter().find(|(id, _)| *id == "agent").map(|(_, t)| t.clone()).unwrap_or_default();
    let p = plan_in(&w, &request(json!({"text": "Fix the login redirect", "repo": "api", "agent": "bare"})));
    assert!(p.ok(), "{:?}", p.blockers);
    assert_eq!(step(&p), "start Bare, then queue your prompt for it (Bare takes no prompt as it starts)");
    assert!(!step(&p).contains(" on Fix"), "never said to start on the work");
    let stub = plan_in(&w, &request(json!({"text": "Fix the login redirect", "repo": "api"})));
    assert!(step(&stub).starts_with("start Stub on "), "{}", step(&stub));
    assert_eq!(queue_first_prompt(&stub, "app:t9"), "", "an agent that took it on its command line queues nothing");
    assert_eq!(queue_first_prompt(&p, "app:t9"), " · the prompt was not queued: paste it once the agent is up", "no queue here: it says so");
}

#[test]
fn an_agent_on_several_accounts_starts_on_the_one_picked_else_the_default() {
    let f = Fixture::new();
    let desk = Fake::default();
    let mut w = world(&f, f.cfg(json!({})), &desk);
    let account = |dir: &str, name: &str, default: bool| accounts::Account { dir: dir.into(), name: name.into(), default, signed_in: Value::Null };
    w.accounts = vec![account("/home/me/.claude", "Personal", true), account("/home/me/.claude-work", "Work", false)];
    w.agents[0].account_env = Some("CLAUDE_CONFIG_DIR".into());
    w.agents[1].account_env = None;
    let names: Vec<Value> = choices(&w)["agents"].as_array().unwrap().iter().map(|a| json!(a["accounts"].as_array().unwrap().len())).collect();
    assert_eq!(names, [json!(2), json!(0)], "only an agent with an account variable offers the accounts");

    let default = plan_in(&w, &request(json!({"text": "Tidy the logs", "repo": "api"})));
    assert_eq!((default.account.as_str(), default.json()["accountName"].as_str()), ("/home/me/.claude", Some("Personal")));
    let work = plan_in(&w, &request(json!({"text": "Tidy the logs", "repo": "api", "account": "/home/me/.claude-work/"})));
    assert!(work.ok() && work.steps.iter().any(|(_, t)| t.contains("(Work account)")), "{:?}", work.steps);
    let script = script_for(&work, Path::new("/m"), Path::new("/p"));
    assert!(script.contains("\nexport CLAUDE_CONFIG_DIR=/home/me/.claude-work\n"), "{script}");

    let gone = plan_in(&w, &request(json!({"text": "Tidy the logs", "repo": "api", "account": "/home/me/.claude-gone"})));
    assert_eq!(blocker_steps(&gone), ["agent"]);
    assert_eq!(gone.blockers[0].fixes[0], ("Use the default account".to_string(), json!({"account": ""})));
    // A session cockpit drives runs on the account picked too.
    let driven = plan_in(&w, &request(json!({"text": "Tidy the logs", "repo": "api", "account": "/home/me/.claude-work", "driven": true})));
    assert!(driven.ok() && driven.account == "/home/me/.claude-work", "{:?}", driven.blockers.iter().map(|b| &b.step).collect::<Vec<_>>());

    let other = plan_in(&w, &request(json!({"text": "Tidy the logs", "repo": "api", "agent": "moded", "account": "/home/me/.claude-work"})));
    assert!(other.account.is_empty() && other.notes.iter().any(|n| n.contains("one account")), "{:?}", other.notes);
    assert!(!script_for(&other, Path::new("/m"), Path::new("/p")).contains("export"));
    w.accounts.truncate(1);
    assert!(plan_in(&w, &request(json!({"text": "Tidy the logs", "repo": "api"}))).account.is_empty(), "one account, nothing to pick");
}
