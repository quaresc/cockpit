"""Rules of the herdr apps that need no terminal: python3 -m unittest discover -s herdr/tests"""

import hashlib
import http.server
import io
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import threading
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


HAS_UV = bool(shutil.which("uv") or os.path.exists("/opt/homebrew/bin/uv"))


class AppMode(unittest.TestCase):
    """app.sh runs the Rust binary when apps is rust or unset, from bin/ before a dev build; one missing or failing at once
    gives way to the Textual version (with uv), then the text one."""

    def run_app(self, mode, textual=None, rust=None, dev=None):
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
            for folder, body in (("bin", rust), (os.path.join("tui", "target", "release"), dev)):
                if body is None:
                    continue
                os.makedirs(os.path.join(apps, folder))
                binary = os.path.join(apps, folder, "cockpit-demo")
                with open(binary, "w", encoding="utf-8") as handle:
                    handle.write(body)
                os.chmod(binary, 0o755)
            done = subprocess.run(["sh", os.path.join(apps, "app.sh"), "demo", "cockpit-space"], capture_output=True, text=True,
                                  env={**os.environ, "HOME": home}, timeout=120)
            return done.stdout.split()

    def test_rust_is_the_default(self):
        self.assertEqual(self.run_app(None, rust="#!/bin/sh\necho rust \"$@\"\n"), ["rust", "cockpit-space"])

    def test_the_release_binary_comes_before_a_dev_build(self):
        self.assertEqual(self.run_app("rust", rust="#!/bin/sh\necho rust\n", dev="#!/bin/sh\necho dev\n"), ["rust"])
        self.assertEqual(self.run_app("rust", dev="#!/bin/sh\necho dev \"$@\"\n"), ["dev", "cockpit-space"])

    def test_the_binary_knows_its_herdr_folder(self):
        self.assertEqual(len(self.run_app("rust", rust="#!/bin/sh\necho \"$COCKPIT_HERDR_DIR\"\n")), 1)

    def test_rust_missing_or_failing_at_once_falls_back_to_text(self):
        self.assertEqual(self.run_app("rust"), ["text", "cockpit-space"])
        self.assertEqual(self.run_app("rust", rust="#!/bin/sh\necho rust\nexit 1\n"), ["rust", "text", "cockpit-space"])

    def test_light_and_full_leave_the_binary_alone(self):
        self.assertEqual(self.run_app("light", "print('textual')", rust="#!/bin/sh\necho rust\n"), ["text", "cockpit-space"])
        self.assertEqual(self.run_app("full", rust="#!/bin/sh\necho rust\n"), ["text", "cockpit-space"])

    @unittest.skipUnless(HAS_UV, "uv is not installed")
    def test_rust_missing_or_failing_at_once_falls_back_to_textual(self):
        textual = "import sys\nprint('textual', *sys.argv[1:])\n"
        self.assertEqual(self.run_app("rust", textual), ["textual", "cockpit-space"])
        self.assertEqual(self.run_app(None, textual, rust="#!/bin/sh\necho rust\nexit 1\n"), ["rust", "textual", "cockpit-space"])

    @unittest.skipUnless(HAS_UV, "uv is not installed")
    def test_full_runs_the_textual_app_and_falls_back_when_it_fails_at_once(self):
        self.assertEqual(self.run_app("full", "import sys\nprint('textual', *sys.argv[1:])\n"), ["textual", "cockpit-space"])
        self.assertEqual(self.run_app("full", "print('textual')\nraise SystemExit(1)\n"), ["textual", "text", "cockpit-space"])


class UpdateStopsTheApps(unittest.TestCase):
    """update.sh stops every space app by the argument that marks it, whichever version runs it."""

    def test_the_pattern_catches_every_version(self):
        with open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "update.sh"), encoding="utf-8") as handle:
            pattern = re.search(r'pkill -f "(.*cockpit-space)"', handle.read()).group(1)
        for line in ("python3 inbox.py cockpit-space", "uv run --quiet --script todo_tx.py cockpit-space",
                     "/x/herdr/bin/cockpit-runbook cockpit-space", "/x/herdr/tui/target/release/cockpit-meetings cockpit-space"):
            self.assertRegex(line, pattern)
        self.assertNotRegex("/x/herdr/bin/cockpit-show demo", pattern)


TARGETS = {("Darwin", "arm64"): "aarch64-apple-darwin", ("Darwin", "x86_64"): "x86_64-apple-darwin",
           ("Linux", "x86_64"): "x86_64-unknown-linux-gnu", ("Linux", "aarch64"): "aarch64-unknown-linux-gnu"}


@unittest.skipUnless(shutil.which("curl") and (os.uname().sysname, os.uname().machine) in TARGETS, "no curl or no build for this machine")
class FetchApps(unittest.TestCase):
    """fetch-apps.sh installs the release's binaries for this version and machine, against a fake release served locally."""

    def setUp(self):
        self.folder = tempfile.TemporaryDirectory()
        root = self.folder.name
        self.herdr, self.release = os.path.join(root, "herdr"), os.path.join(root, "release")
        os.makedirs(self.herdr)
        os.makedirs(os.path.join(self.release, "v9.9.9"))
        shutil.copy(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "fetch-apps.sh"), self.herdr)
        with open(os.path.join(self.herdr, "herdr-plugin.toml"), "w", encoding="utf-8") as handle:
            handle.write('id = "cockpit"\nversion = "9.9.9"\n')
        self.asset = os.path.join(self.release, "v9.9.9", f"cockpit-apps-9.9.9-{TARGETS[(os.uname().sysname, os.uname().machine)]}.tar.gz")
        self.requests = []
        requests, release = self.requests, self.release

        class Handler(http.server.SimpleHTTPRequestHandler):
            def __init__(self, *args, **kwargs):
                super().__init__(*args, directory=release, **kwargs)

            def log_message(self, *args):
                requests.append(self.path)

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()
        self.folder.cleanup()

    def publish(self, checksum=None):
        with tarfile.open(self.asset, "w:gz") as archive:
            for app in ("hub", "inbox"):
                body = f"#!/bin/sh\necho {app}\n".encode()
                info = tarfile.TarInfo(f"cockpit-{app}")
                info.size, info.mode = len(body), 0o644
                archive.addfile(info, io.BytesIO(body))
        with open(self.asset, "rb") as handle:
            digest = checksum or hashlib.sha256(handle.read()).hexdigest()
        with open(f"{self.asset}.sha256", "w", encoding="utf-8") as handle:
            handle.write(f"{digest}  {os.path.basename(self.asset)}\n")

    def fetch(self):
        url = f"http://127.0.0.1:{self.server.server_address[1]}"
        return subprocess.run(["sh", os.path.join(self.herdr, "fetch-apps.sh")], capture_output=True, text=True,
                              env={**os.environ, "COCKPIT_RELEASES_URL": url}, timeout=60)

    def bin(self, *names):
        return os.path.join(self.herdr, "bin", *names)

    def test_installs_the_binaries_once_per_version(self):
        self.publish()
        done = self.fetch()
        self.assertEqual((done.returncode, done.stdout, done.stderr), (0, "", ""))
        self.assertTrue(os.access(self.bin("cockpit-inbox"), os.X_OK))
        self.assertEqual(subprocess.run([self.bin("cockpit-hub")], capture_output=True, text=True).stdout, "hub\n")
        with open(self.bin(".version"), encoding="utf-8") as handle:
            self.assertEqual(handle.read(), "9.9.9\n")
        seen = len(self.requests)
        self.assertEqual(self.fetch().returncode, 0)
        self.assertEqual(len(self.requests), seen)
        self.assertEqual(sorted(os.listdir(self.herdr)), ["bin", "fetch-apps.sh", "herdr-plugin.toml"])

    def test_a_release_without_the_apps_changes_nothing(self):
        done = self.fetch()
        self.assertEqual((done.returncode, done.stderr), (0, ""))
        self.assertFalse(os.path.exists(self.bin()))

    def test_a_wrong_checksum_installs_nothing_and_says_so(self):
        self.publish(checksum="0" * 64)
        done = self.fetch()
        self.assertNotEqual(done.returncode, 0)
        self.assertIn("checksum mismatch", done.stderr)
        self.assertEqual(sorted(os.listdir(self.herdr)), ["fetch-apps.sh", "herdr-plugin.toml"])

    def test_a_checkout_with_its_own_build_fetches_nothing(self):
        self.publish()
        os.makedirs(os.path.join(self.herdr, "tui", "target", "release"))
        hub = os.path.join(self.herdr, "tui", "target", "release", "cockpit-hub")
        open(hub, "w").close()
        os.chmod(hub, 0o755)
        self.assertEqual(self.fetch().returncode, 0)
        self.assertEqual(self.requests, [])


if __name__ == "__main__":
    unittest.main()
