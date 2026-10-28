"""The recap popup: what an agent did while its space was out of view."""
import os, sys, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from term2svg import run, screen_of, rows_svg, window

HERE = os.path.dirname(os.path.abspath(__file__))
HOME = os.path.join(HERE, "home")
APPS = os.path.join(HERE, "..", "..", "herdr")
OUT = os.path.join(HERE, "..")
env = {"HOME": HOME, "TERM": "xterm-256color", "HERDR_BIN_PATH": "/usr/bin/true"}


def text_of(screen, y):
    return "".join(c.data for c in screen.buffer[y].values())


# Recap: what the quota-banner agent did in the 52 minutes since its space was left.
since = int(time.time()) - 52 * 60
text = run(["sh", "recap.sh"], 84, 28, {**env, "HERDR_BIN_PATH": os.path.join(HERE, "fake-herdr.sh"), "RECAP_TAB": "t7",
                                        "RECAP_SINCE": str(since)}, wait=2.0, cwd=APPS)
screen = screen_of(text, 84, 28)
used = [y for y in range(28) if text_of(screen, y).strip()]
chars = set()
body = rows_svg(screen, range(used[0], used[-1] + 1), chars, at=0)
open(os.path.join(OUT, "recap.svg"), "w").write(window(84, used[-1] - used[0] + 1, "recap · quota-banner", chars, body))

print("ok")
