import os, sys
sys.path.insert(0, os.path.dirname(__file__))
from term2svg import run, screen_of, svg, last_used_row
S = os.path.dirname(os.path.abspath(__file__)); HOME = os.path.join(S, "home")
APPS = os.path.join(S, "..", "..", "herdr"); OUT = os.path.join(S, "..")
env = {"HOME": HOME, "TERM": "xterm-256color", "HERDR_BIN_PATH": "/usr/bin/true"}
shots = {"inbox": (120, 44, ["j"] * 9), "todo": (100, 14, ["j"]), "runbook": (140, 26, ["j", "j", "j"]), "meetings": (96, 16, ["j"])}
for name in sys.argv[1:] or shots:
    cols, rows, keys = shots[name]
    text = run(["python3", f"{name}.py"], cols, rows, env, keys, cwd=APPS)
    screen = screen_of(text, cols, rows)
    with open(os.path.join(OUT, f"{name}.svg"), "w") as f:
        f.write(svg(screen, name, crop_rows=min(rows, last_used_row(screen) + 1)))
    print(name, "rows used", last_used_row(screen))
