"""Rules of the herdr apps that need no terminal: python3 -m unittest discover -s herdr/tests"""

import os
import subprocess
import sys
import tempfile
import time
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
sys.argv = sys.argv[:1]

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


if __name__ == "__main__":
    unittest.main()
