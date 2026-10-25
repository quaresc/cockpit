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
