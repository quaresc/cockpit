"""The /cockpit settings pane, from fictional values: each value a button that cycles it."""
import os, sys
sys.path.insert(0, os.path.dirname(__file__))
from term2svg import screen_of, svg

COLS, ROWS = 72, 14
ORANGE, DIMC, INK, QUIET, WHITE = "FFC799", "707070", "101010", "2C2C30", "D2D2D2"
ops = []

def put(r, c, text, fg=None, bg=None, bold=False):
    sgr = ["0"] + (["1"] if bold else [])
    if fg: sgr.append("38;2;%d;%d;%d" % tuple(int(fg[i:i + 2], 16) for i in (0, 2, 4)))
    if bg: sgr.append("48;2;%d;%d;%d" % tuple(int(bg[i:i + 2], 16) for i in (0, 2, 4)))
    ops.append(f"\033[{r};{c}H\033[{';'.join(sgr)}m{text}\033[0m")

def pill(r, c, label):
    put(r, c, "", QUIET); put(r, c + 1, label, WHITE, QUIET); put(r, c + 1 + len(label), "", QUIET)

put(2, 3, "Cockpit settings", WHITE, bold=True)
SETTINGS = [("Front checks run in", "the pane beside the agent"), ("Close browsers out of view", "after 1 h"),
            ("Notifications", "on"), ("Morning brief", "on"), ("Recap when back in a space", "on"),
            ("Check errors after a deploy", "off"), ("Meeting briefs", "on")]
for i, (label, value) in enumerate(SETTINGS):
    put(4 + i, 3, label.ljust(30)); pill(4 + i, 33, value)
put(12, 3, "terminal-browser 0.13.4 · press a value to change it", DIMC)
put(13, 3, "/cockpit <setting> <value> from any session", DIMC)

screen = screen_of("".join(ops), COLS, ROWS)
with open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "settings.svg"), "w") as f:
    f.write(svg(screen, "claude · /cockpit"))
