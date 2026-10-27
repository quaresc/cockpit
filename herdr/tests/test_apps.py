"""Rules of the herdr apps that need no terminal: python3 -m unittest discover -s herdr/tests"""

import os
import shutil
import subprocess
import sys
import tempfile
import time
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
sys.argv = sys.argv[:1]

import actions  # noqa: E402
import cockpit_config  # noqa: E402
import inbox  # noqa: E402
import show  # noqa: E402
import view  # noqa: E402


class HasTicket(unittest.TestCase):
    def test_whole_key_only(self):
        self.assertTrue(cockpit_config.has_ticket("me/eng-4-login", "ENG-4"))
        self.assertTrue(cockpit_config.has_ticket("ENG-4", "eng-4"))
        self.assertFalse(cockpit_config.has_ticket("me/eng-42-login", "ENG-4"))
        self.assertFalse(cockpit_config.has_ticket("me/xeng-4-login", "ENG-4"))
        self.assertFalse(cockpit_config.has_ticket(None, "ENG-4"))
        self.assertFalse(cockpit_config.has_ticket("eng-4", ""))


class AgeDays(unittest.TestCase):
    def test_unreadable_dates_have_no_age(self):
        self.assertIsNone(inbox.age_days(""))
        self.assertIsNone(inbox.age_days(None))
        self.assertEqual(inbox.ago(""), "")

    def test_a_date(self):
        self.assertAlmostEqual(inbox.age_days("2000-01-01T00:00:00Z"), (time.time() - 946684800) / 86400, places=2)


class Settle(unittest.TestCase):
    def test_a_request_given_up_on_is_deleted(self):
        with tempfile.TemporaryDirectory() as folder:
            inbox.REQUESTS = folder
            open(os.path.join(folder, "1-rerun.json"), "w").close()
            state = {"pending": {"row": {"file": "1-rerun.json", "at": time.time() - inbox.CLAIM_WAIT - 1, "label": "x"}},
                     "outcome": {}, "note": "", "note_at": 0}
            inbox.settle(state)
            self.assertFalse(os.path.exists(os.path.join(folder, "1-rerun.json")))
            self.assertEqual(state["outcome"]["row"]["message"], "no Claude session picked it up")


class TicketCosts(unittest.TestCase):
    def test_sums_every_session(self):
        with tempfile.TemporaryDirectory() as folder:
            inbox.COSTS = folder
            view.write_json(os.path.join(folder, "a.json"), {"ENG-1": 1.5, "ENG-2": 0.25})
            view.write_json(os.path.join(folder, "b.json"), {"ENG-1": 2.0})
            self.assertEqual(inbox.ticket_costs(), {"ENG-1": 3.5, "ENG-2": 0.25})


class OnBranch(unittest.TestCase):
    def setUp(self):
        self.folder = tempfile.TemporaryDirectory()
        repos = self.folder.name
        self.main = os.path.join(repos, "api")
        git = lambda *args: subprocess.run(["git", *args], check=True, capture_output=True)
        git("init", "-q", "-b", "main", self.main)
        git("-C", self.main, "-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "init")
        self.tree = os.path.join(repos, "wt")
        git("-C", self.main, "worktree", "add", "-q", "-b", "me/eng-4-login", self.tree)
        self.saved = inbox.REPOS, inbox.run, inbox.spaces
        inbox.REPOS = repos
        self.herdr = []
        real = inbox.run
        inbox.run = lambda argv: self.herdr.append(argv) or subprocess.CompletedProcess(argv, 1, "", "") if argv[0] == inbox.HERDR else real(argv)

    def tearDown(self):
        inbox.REPOS, inbox.run, inbox.spaces = self.saved
        self.folder.cleanup()

    def state(self):
        return {"note": "", "note_at": 0}

    def test_finds_the_worktree_of_a_branch(self):
        self.assertEqual(os.path.realpath(inbox.checkout_of("api", "me/eng-4-login")), os.path.realpath(self.tree))
        self.assertEqual(os.path.realpath(inbox.checkout_of("api", "main")), os.path.realpath(self.main))
        self.assertIsNone(inbox.checkout_of("api", "me/eng-9"))

    def test_an_open_space_on_the_checkout_comes_into_view(self):
        inbox.spaces = lambda: [{"workspace_id": "w7", "label": "eng-4", "worktree": {"checkout_path": self.tree}}]
        inbox.on_branch(self.state(), {"repo": "api", "branch": "me/eng-4-login"}, "eng-4", "Continue.")
        self.assertEqual(self.herdr, [[inbox.HERDR, "workspace", "focus", "w7"]])

    def test_a_new_space_opens_in_the_checkout(self):
        inbox.spaces = lambda: []
        inbox.on_branch(self.state(), {"repo": "api", "branch": "me/eng-4-login"}, "eng-4", "Continue.")
        create = next(argv for argv in self.herdr if argv[1:3] == ["workspace", "create"])
        self.assertEqual(os.path.realpath(create[create.index("--cwd") + 1]), os.path.realpath(self.tree))


class Actions(unittest.TestCase):
    def setUp(self):
        self.saved = inbox.run, inbox.spaces, actions.pane_of, actions.open_tab, actions.promote
        self.herdr = []
        inbox.run = lambda argv: self.herdr.append(argv) or subprocess.CompletedProcess(argv, 0, '{"result": {"root_pane": {"pane_id": "w9:p1"}}}', "")
        inbox.spaces = lambda: []
        actions.pane_of = lambda key: None

    def tearDown(self):
        inbox.run, inbox.spaces, actions.pane_of, actions.open_tab, actions.promote = self.saved

    def state(self):
        return {"note": "", "note_at": 0}

    def test_an_action_runs_in_a_tab_of_the_actions_space(self):
        actions.open_tab = lambda cwd, label, key=None: "w3:p7"
        inbox.start_action(self.state(), "/repos", "review api#4", "/code-review 4")
        self.assertEqual(self.herdr, [[inbox.HERDR, "pane", "run", "w3:p7", "claude '/code-review 4'"]])

    def test_without_the_actions_space_it_gets_a_space(self):
        actions.open_tab = lambda cwd, label, key=None: None
        inbox.start_action(self.state(), "/repos", "talk eng-4", "Talk.", ("--resume", "s1", "--fork-session"))
        self.assertEqual(self.herdr[0][1:3], ["workspace", "create"])
        self.assertEqual(self.herdr[1], [inbox.HERDR, "pane", "run", "w9:p1", "claude --resume s1 --fork-session Talk."])

    def test_continue_moves_the_plan_out_of_the_actions_space(self):
        moved = []
        actions.promote = lambda key, label: moved.append((key, label)) or True
        inbox.continue_ticket(self.state(), {"id": "ENG-4", "url": "u"}, [])
        self.assertEqual(moved, [("plan eng-4", "eng-4")])
        self.assertEqual(self.herdr, [])

    def test_a_merged_session_in_a_pinned_space_closes_its_tab_only(self):
        saved = inbox.heartbeats, inbox.linked_spaces, inbox.CONFIG
        inbox.CONFIG = {**inbox.CONFIG, "spaces": ["inbox", "actions"]}
        now = time.time() * 1000
        inbox.heartbeats = lambda: [{"at": now, "space": "w2", "tab": "w2:t3", "repo": "api", "branch": "b"},
                                    {"at": now, "space": "w5", "tab": "w5:t1", "repo": "api", "branch": "c"}]
        inbox.linked_spaces = lambda: {"w2": {"label": "\uf0e7  actions"}, "w5": {"label": "eng-4"}}
        try:
            self.assertEqual(inbox.merged_sessions({("api", "b"), ("api", "c")}, []), {"tab:w2:t3": ("api", "b"), "w5": ("api", "c")})
        finally:
            inbox.heartbeats, inbox.linked_spaces, inbox.CONFIG = saved


class TicketSession(unittest.TestCase):
    def test_the_session_that_spent_most_and_still_has_its_transcript(self):
        with tempfile.TemporaryDirectory() as folder:
            saved = inbox.COSTS, inbox.PROJECTS
            inbox.COSTS, inbox.PROJECTS = os.path.join(folder, "costs"), os.path.join(folder, "projects")
            try:
                view.write_json(os.path.join(inbox.COSTS, "gone.json"), {"ENG-1": 9.0})
                view.write_json(os.path.join(inbox.COSTS, "small.json"), {"ENG-1": 0.5})
                view.write_json(os.path.join(inbox.COSTS, "big.json"), {"ENG-1": 4.0, "ENG-2": 1.0})
                os.makedirs(os.path.join(inbox.PROJECTS, "-repos"))
                for session in ("small", "big"):
                    with open(os.path.join(inbox.PROJECTS, "-repos", f"{session}.jsonl"), "w", encoding="utf-8") as handle:
                        handle.write('{"type": "summary"}\n{"cwd": "%s"}\n' % folder)
                self.assertEqual(inbox.ticket_session("ENG-1"), ("big", folder))
                self.assertEqual(inbox.ticket_session("ENG-3"), (None, None))
            finally:
                inbox.COSTS, inbox.PROJECTS = saved


class IsPinned(unittest.TestCase):
    def test_by_name_after_the_icon(self):
        config = {"spaces": ["inbox", "actions", {"name": "notes"}]}
        self.assertTrue(cockpit_config.is_pinned("[3] \uf0e7  actions", config))
        self.assertTrue(cockpit_config.is_pinned("x  notes", config))
        self.assertFalse(cockpit_config.is_pinned("[6] eng-4 actions", config))


class NewFile(unittest.TestCase):
    def test_matches_git(self):
        with tempfile.TemporaryDirectory() as repo:
            subprocess.run(["git", "init", "-q", repo], check=True)
            files = {"lines.txt": b"a\nb\n", "no-newline.txt": b"a", "binary.bin": b"\0\1", "empty.txt": b""}
            for name, body in files.items():
                with open(os.path.join(repo, name), "wb") as handle:
                    handle.write(body)
            for name in files:
                git = subprocess.run(["git", "-C", repo, "diff", "--no-index", "--", "/dev/null", name], capture_output=True, text=True).stdout
                self.assertEqual(show.parse_patch(show.new_file(repo, name)), show.parse_patch(git), name)


class AppMode(unittest.TestCase):
    """app.sh runs the Textual version when apps is full or unset and it starts; anything else gets the text one."""

    def run_app(self, mode, textual=None, rust=None):
        with tempfile.TemporaryDirectory() as home, tempfile.TemporaryDirectory() as apps:
            os.makedirs(os.path.join(home, ".config", "cockpit"))
            with open(os.path.join(home, ".config", "cockpit", "config.json"), "w", encoding="utf-8") as handle:
                handle.write(f'{{"apps": "{mode}"}}' if mode else "{}")
            shutil.copy(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "app.sh"), apps)
            with open(os.path.join(apps, "demo.py"), "w", encoding="utf-8") as handle:
                handle.write("import sys\nprint('text', *sys.argv[1:])\n")
            if textual is not None:
                with open(os.path.join(apps, "demo_tx.py"), "w", encoding="utf-8") as handle:
                    handle.write(textual)
            if rust is not None:
                os.makedirs(os.path.join(apps, "tui", "target", "release"))
                binary = os.path.join(apps, "tui", "target", "release", "cockpit-demo")
                with open(binary, "w", encoding="utf-8") as handle:
                    handle.write(rust)
                os.chmod(binary, 0o755)
            done = subprocess.run(["sh", os.path.join(apps, "app.sh"), "demo", "cockpit-space"], capture_output=True, text=True,
                                  env={**os.environ, "HOME": home}, timeout=120)
            return done.stdout.split()

    def test_light_runs_the_text_app(self):
        self.assertEqual(self.run_app("light", "print('textual')"), ["text", "cockpit-space"])

    def test_rust_runs_the_binary_and_falls_back_when_it_fails_at_once(self):
        self.assertEqual(self.run_app("rust", rust="#!/bin/sh\necho rust \"$@\"\n"), ["rust", "cockpit-space"])
        self.assertEqual(self.run_app("rust", rust="#!/bin/sh\necho rust\nexit 1\n"), ["rust", "text", "cockpit-space"])
        self.assertEqual(self.run_app("light", rust="#!/bin/sh\necho rust\n"), ["text", "cockpit-space"])

    def test_full_without_a_textual_version_runs_the_text_app(self):
        self.assertEqual(self.run_app("full"), ["text", "cockpit-space"])
        self.assertEqual(self.run_app(None), ["text", "cockpit-space"])

    @unittest.skipUnless(shutil.which("uv") or os.path.exists("/opt/homebrew/bin/uv"), "uv is not installed")
    def test_full_runs_the_textual_app_and_falls_back_when_it_fails_at_once(self):
        self.assertEqual(self.run_app("full", "import sys\nprint('textual', *sys.argv[1:])\n"), ["textual", "cockpit-space"])
        self.assertEqual(self.run_app(None, "import sys\nprint('textual', *sys.argv[1:])\n"), ["textual", "cockpit-space"])
        self.assertEqual(self.run_app("full", "print('textual')\nraise SystemExit(1)\n"), ["textual", "text", "cockpit-space"])


if __name__ == "__main__":
    unittest.main()
