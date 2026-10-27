"""The real herdr apps, animated: the cursor steps through the rows, each row showing its buttons. Run demo_data.py first.
--full draws the apps that have a Textual version with it, into <name>-full.svg; run it with the venv's python, which has Textual."""
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
# The Textual inbox: down the tree, then the command palette finding a row's action.
FULL_SHOTS = {
    "inbox": (120, 44, ["j"] * 9 + ["\x10", "r", "e", "v", "i", "e", "w", "\x1b"], 0.9),
}
FULL = "--full" in sys.argv


def cropped(screen, rows):
    """The screen's first rows, as a screen of that height."""
    out = Screen(screen.columns, rows)
    for y in range(rows):
        out.buffer[y] = screen.buffer[y]
    return out


shots_of = FULL_SHOTS if FULL else APPS_SHOTS
for name in [a for a in sys.argv[1:] if a != "--full"] or shots_of:
    cols, rows, keys, step = shots_of[name]
    argv = [sys.executable, f"{name}_tx.py"] if FULL else ["python3", f"{name}.py"]
    shots = [screen_of(t, cols, rows) for t in run_steps(argv, cols, rows, env, keys, cwd=APPS, settle=4.0 if FULL else 2.0, gap=0.6)]
    used = min(rows, max(last_used_row(s) for s in shots) + 1)
    frames = [(step * (2 if i in (0, len(shots) - 1) else 1), cropped(s, used)) for i, s in enumerate(shots)]
    open(os.path.join(OUT, f"{name}-full.svg" if FULL else f"{name}.svg"), "w").write(animate(frames, f"{name} · full" if FULL else name))
    print(name, len(frames), "frames,", used, "rows")
