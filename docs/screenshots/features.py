"""Shots of single features: the recap popup, and the inbox's sandbox and stop buttons."""
import os, sys, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from term2svg import run, run_steps, screen_of, rows_svg, window, svg

HERE = os.path.dirname(os.path.abspath(__file__))
HOME = os.path.join(HERE, "home")
APPS = os.path.join(HERE, "..", "..", "herdr")
OUT = os.path.join(HERE, "..")
env = {"HOME": HOME, "TERM": "xterm-256color", "HERDR_BIN_PATH": "/usr/bin/true"}
CAP_END = chr(0xE0B4)


def text_of(screen, y):
    return "".join(c.data for c in screen.buffer[y].values())


def row_with(screen, label):
    """The row showing a button with this label: the label sits right before the pill's closing cap."""
    return next((y for y in range(screen.lines) if f"{label}{CAP_END}" in text_of(screen, y)), None)


# Recap: what the quota-banner agent did in the 52 minutes since its space was left.
since = int(time.time()) - 52 * 60
text = run(["sh", "recap.sh"], 84, 28, {**env, "HERDR_BIN_PATH": os.path.join(HERE, "fake-herdr.sh"), "RECAP_SPACE": "w7",
                                        "RECAP_SINCE": str(since)}, wait=2.0, cwd=APPS)
screen = screen_of(text, 84, 28)
used = [y for y in range(28) if text_of(screen, y).strip()]
chars = set()
body = rows_svg(screen, range(used[0], used[-1] + 1), chars, at=0)
open(os.path.join(OUT, "recap.svg"), "w").write(window(84, used[-1] - used[0] + 1, "recap · quota-banner", chars, body))

# Inbox: step through the rows; keep the screens where the stop and the sandbox buttons show.
cols, rows = 120, 44
shots = [screen_of(t, cols, rows) for t in run_steps(["python3", "inbox.py"], cols, rows, env, ["j"] * 24, cwd=APPS)]
stop = next(s for s in shots if row_with(s, "stop") is not None)
box = next(s for s in shots if row_with(s, "sandbox") is not None)
last = max(y for y in range(rows) if text_of(box, y).strip())
open(os.path.join(OUT, "inbox.svg"), "w").write(svg(box, "inbox", crop_rows=min(rows, last + 2)))

# Both buttons in one small picture: the merged sandbox with stop, the built branch with sandbox.
y_stop, y_box = row_with(stop, "stop"), row_with(box, "sandbox")
chars = set()
body = rows_svg(stop, [y_stop], chars, at=0) + rows_svg(box, [y_box], chars, at=2)
open(os.path.join(OUT, "sandboxes.svg"), "w").write(window(cols, 3, "inbox · sandboxes", chars, body))
print("ok")
