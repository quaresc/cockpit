"""The real herdr apps, animated: the cursor steps through the rows, each row showing its buttons. Run demo_data.py first."""
import os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from term2svg import Screen, animate, last_used_row, run_steps, screen_of

HERE = os.path.dirname(os.path.abspath(__file__))
HOME = os.path.join(HERE, "home")
APPS = os.path.join(HERE, "..", "..", "herdr")
OUT = os.path.join(HERE, "..")
env = {"HOME": HOME, "TERM": "xterm-256color", "HERDR_BIN_PATH": "/usr/bin/true"}
# app: (columns, rows, keys, seconds per step)
APPS_SHOTS = {
    "inbox": (120, 44, ["j"] * 14, 0.9),
    "todo": (100, 14, ["j", "j", "j", "k", "k"], 0.9),
    "meetings": (96, 16, ["j", "j", "j", "j"], 0.9),
}


def cropped(screen, rows):
    """The screen's first rows, as a screen of that height."""
    out = Screen(screen.columns, rows)
    for y in range(rows):
        out.buffer[y] = screen.buffer[y]
    return out


for name in sys.argv[1:] or APPS_SHOTS:
    cols, rows, keys, step = APPS_SHOTS[name]
    shots = [screen_of(t, cols, rows) for t in run_steps(["python3", f"{name}.py"], cols, rows, env, keys, cwd=APPS, settle=2.0, gap=0.6)]
    used = min(rows, max(last_used_row(s) for s in shots) + 1)
    frames = [(step * (2 if i in (0, len(shots) - 1) else 1), cropped(s, used)) for i, s in enumerate(shots)]
    open(os.path.join(OUT, f"{name}.svg"), "w").write(animate(frames, name))
    print(name, len(frames), "frames,", used, "rows")
