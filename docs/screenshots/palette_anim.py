"""The palette popup, animated: what it offers before typing, then a ticket found across the apps and its history, then
a past session found by what it was about. Run demo_data.py and history.py first."""
import os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from term2svg import animate, last_used_row, run_steps, screen_of

HERE = os.path.dirname(os.path.abspath(__file__))
HOME = os.path.join(HERE, "home")
APPS = os.path.join(HERE, "..", "..", "herdr")
COCKPIT = os.path.join(APPS, "tui", "target", "release", "cockpit")
# Opened over a Claude session (fake-herdr.sh says pane p7 runs one), so its own keys show.
env = {"HOME": HOME, "TERM": "xterm-256color", "HERDR_BIN_PATH": os.path.join(HERE, "fake-herdr.sh"), "COCKPIT_HERDR_DIR": APPS,
       "COCKPIT_HUB_IDLE_SECS": "120", "COCKPIT_HUB_FEED": "off", "PALETTE_PANE": "p7"}
COLS, ROWS = 100, 24
keys = ["", *"ENG-413", "", "\x0e", *["\x7f"] * 7, *"the session where we fixed quotas", "", ""]
shots = [screen_of(t, COLS, ROWS) for t in run_steps([COCKPIT, "palette"], COLS, ROWS, env, keys, cwd=APPS, settle=3.0, gap=0.35)]
# The screens worth reading stay up; each key typed passes quickly.
frames = [(1.8 if i in (0, len(shots) - 1) or keys[i - 1:i] in ([""], ["\x0e"]) else 0.1, s) for i, s in enumerate(shots)]
open(os.path.join(HERE, "..", "palette.svg"), "w").write(animate(frames, "palette"))
print("palette", len(frames), "frames")
print("\n".join(l.rstrip() for l in shots[-1].display))
