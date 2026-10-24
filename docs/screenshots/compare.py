"""A before/after page from shots.sh, from fictional screens: the same phone page on main and on the branch."""
import os, sys
sys.path.insert(0, os.path.dirname(__file__))
from term2svg import screen_of, svg

COLS, ROWS = 76, 24
ORANGE, RED, DIMC, WHITE, FRAME = "FFC799", "FF8080", "707070", "D2D2D2", "505050"
ops = []

def put(r, c, text, fg=None, bg=None, bold=False):
    sgr = ["0"] + (["1"] if bold else [])
    if fg: sgr.append("38;2;%d;%d;%d" % tuple(int(fg[i:i + 2], 16) for i in (0, 2, 4)))
    if bg: sgr.append("48;2;%d;%d;%d" % tuple(int(bg[i:i + 2], 16) for i in (0, 2, 4)))
    ops.append(f"\033[{r};{c}H\033[{';'.join(sgr)}m{text}\033[0m")

def phone(x, label, banner):
    w = 26
    put(4, x, label, DIMC)
    put(5, x, "╭" + "─" * (w - 2) + "╮", FRAME)
    for r in range(6, 22):
        put(r, x, "│" + " " * (w - 2) + "│", FRAME)
    put(22, x, "╰" + "─" * (w - 2) + "╯", FRAME)
    put(7, x + 2, "Settings", WHITE, bold=True)
    top = 9
    if banner:
        put(9, x + 2, " " * (w - 4), None, "3A1C1C"); put(9, x + 3, "⚠ Limit reached", RED, "3A1C1C")
        put(10, x + 2, " " * (w - 4), None, "3A1C1C"); put(10, x + 3, "20 / 20 searches", "C8A0A0", "3A1C1C")
        top = 12
    put(top, x + 2, "Search", DIMC)
    put(top + 1, x + 2, " " * (w - 4), None, "202024"); put(top + 1, x + 3, "Role, skill…", "606060", "202024")
    put(top + 3, x + 2, " New search ", "707070" if banner else "101010", "2C2C30" if banner else ORANGE)
    put(top + 6, x + 2, "Saved searches", DIMC)
    put(top + 7, x + 2, "Frontend · Paris", WHITE)

put(2, 3, "Before / after", ORANGE, bold=True)
put(2, 20, "mobile · iPhone 16", DIMC)
phone(6, "BEFORE  main", False)
phone(42, "AFTER  me/eng-413-quota-ui", True)

screen = screen_of("".join(ops), COLS, ROWS)
with open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "compare.svg"), "w") as f:
    f.write(svg(screen, "terminal-browser · before / after"))
