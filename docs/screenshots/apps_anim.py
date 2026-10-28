"""The Rust apps, built in herdr/tui, animated: the cursor steps through the rows. Run demo_data.py first."""
import os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from term2svg import Screen, animate, last_used_row, run_steps, screen_of

HERE = os.path.dirname(os.path.abspath(__file__))
HOME = os.path.join(HERE, "home")
APPS = os.path.join(HERE, "..", "..", "herdr")
OUT = os.path.join(HERE, "..")
BINARIES = os.path.join(APPS, "tui", "target", "release")
env = {"HOME": HOME, "TERM": "xterm-256color", "HERDR_BIN_PATH": "/usr/bin/true", "COCKPIT_HERDR_DIR": APPS,
       "COCKPIT_HUB_IDLE_SECS": "20"}
# app: (columns, rows, keys, seconds per step). The inbox: down the tree, a filter typed and kept, then the palette
# searching every app through the hub.
SHOTS = {
    "inbox": (120, 44, ["j"] * 6 + ["/", "q", "u", "o", "t", "a", "\r", "\x1b", "\x10", "p", "l", "a", "n", "\x1b"], 0.9),
    "todo": (100, 16, ["j", "j", "j", "k", "k"], 0.9),
    "meetings": (96, 18, ["j", "j", "j", "j"], 0.9),
}


def cropped(screen, rows):
    """The screen's first rows, as a screen of that height."""
    out = Screen(screen.columns, rows)
    for y in range(rows):
        out.buffer[y] = screen.buffer[y]
    return out


for name in sys.argv[1:] or SHOTS:
    cols, rows, keys, step = SHOTS[name]
    argv = [os.path.join(BINARIES, f"cockpit-{name}")]
    shots = [screen_of(t, cols, rows) for t in run_steps(argv, cols, rows, env, keys, cwd=APPS, settle=4.0, gap=0.6)]
    used = min(rows, max(last_used_row(s) for s in shots) + 1)
    frames = [(step * (2 if i in (0, len(shots) - 1) else 1), cropped(s, used)) for i, s in enumerate(shots)]
    open(os.path.join(OUT, f"{name}.svg"), "w").write(animate(frames, name))
    print(name, len(frames), "frames,", used, "rows")
