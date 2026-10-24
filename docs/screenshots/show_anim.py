"""The show tool, animated: an agent opens a chart beside the conversation, q hides it to a footer chip, the chip brings it
back, drawn by the real show.py from fictional data."""
import json, os, shutil, subprocess, sys, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from term2svg import animate, run, screen_of

HERE = os.path.dirname(os.path.abspath(__file__))
HOME = os.path.join(HERE, "home")
APPS = os.path.join(HERE, "..", "..", "herdr")
OUT = os.path.join(HERE, "..")
COLS, ROWS, LEFT = 150, 30, 76
PANE = COLS - LEFT - 1
ORANGE, DIMC, INK, QUIET, WHITE, LINE = "FFC799", "707070", "101010", "2C2C30", "D2D2D2", "2C2C30"
env = {"HOME": HOME, "TERM": "xterm-256color", "HERDR_BIN_PATH": "/usr/bin/true"}


def pane(name, spec, cols, rows, keys=(), wait=2.0):
    """The show.py screen for this spec, at this size."""
    os.makedirs(os.path.join(HOME, ".cache", "cockpit", "show"), exist_ok=True)
    json.dump({**spec, "at": int(time.time() * 1000)}, open(os.path.join(HOME, ".cache", "cockpit", "show", f"{name}.json"), "w"))
    return screen_of(run(["python3", "show.py", name], cols, rows, env, keys, wait=wait, cwd=APPS), cols, rows)


# A quarter of fictional weekly signups, by sign-in method.
weeks = ["07-06", "07-13", "07-20", "07-27", "08-03", "08-10", "08-17", "08-24", "08-31", "09-07", "09-14", "09-21"]
google = [4120, 3980, 4310, 4060, 3550, 3210, 3390, 3330, 3300, 4140, 5310, 4780]
linkedin = [1540, 1650, 1700, 1610, 1400, 1310, 1230, 1220, 1250, 1390, 1890, 1590]
email = [1110, 1080, 1150, 1060, 960, 900, 930, 980, 990, 1170, 1490, 1330]
chart = {"title": "Signups, Q3", "blocks": [
    {"type": "chart", "kind": "bar", "title": "By sign-in method", "columns": ["method", "signups"],
     "rows": [["google", sum(google)], ["email", sum(email)], ["linkedin", sum(linkedin)]], "x": "method", "y": ["signups"]},
    {"type": "chart", "kind": "line", "title": "Per week", "columns": ["week", "google", "linkedin", "email"],
     "rows": [[w, g, l, e] for w, g, l, e in zip(weeks, google, linkedin, email)], "x": "week", "y": ["google", "linkedin", "email"], "height": 9},
]}

ops = []


def put(r, c, text, fg=None, bg=None, bold=False):
    sgr = ["0"] + (["1"] if bold else [])
    if fg: sgr.append("38;2;%d;%d;%d" % tuple(int(fg[i:i + 2], 16) for i in (0, 2, 4)))
    if bg: sgr.append("48;2;%d;%d;%d" % tuple(int(bg[i:i + 2], 16) for i in (0, 2, 4)))
    ops.append(f"\033[{r};{c}H\033[{';'.join(sgr)}m{text}\033[0m")


def pill(r, c, label, main=False):
    fill, ink = (ORANGE, INK) if main else (QUIET, WHITE)
    put(r, c, "", fill); put(r, c + 1, label, ink, fill); put(r, c + 1 + len(label), "", fill)
    return c + len(label) + 3


def session(width, turns, chip):
    """The Claude session, `width` columns wide: the conversation so far and its footer, with the view's chip or none."""
    ops.clear()
    put(2, 2, "❯ ", DIMC); put(2, 4, "Chart last quarter's signups by sign-in method", WHITE)
    lines = [("●", "Querying the members table for July to September, by method and week."),
             ("", "Ran 2 queries"),
             ("●", "Google sign-in brings 61% of signups, LinkedIn 23% and email 17%."),
             ("", "The chart is open beside this session.")]
    for n, (mark, text) in enumerate(lines[:turns]):
        r = 4 + n + (1 if n >= 2 else 0)
        if mark:
            put(r, 2, mark, WHITE)
        put(r, 4, text, DIMC if not mark and n == 1 else None)
    put(ROWS - 4, 2, "─" * (width - 2), LINE); put(ROWS - 3, 2, "❯ ", DIMC); put(ROWS - 2, 2, "─" * (width - 2), LINE)
    put(ROWS - 1, 2, "⏵⏵ auto mode on", ORANGE)
    pills = [(" ENG-410", True), ("\U000f0b15 notion", False)] + ([(" signups-q3", chip == "open")] if chip else [])
    c = width - sum(len(label) + 3 for label, _ in pills) - len(pills) + 1
    for label, main in pills:
        c = pill(ROWS - 1, c, label, main) + 1
    return screen_of("".join(ops), COLS, ROWS)


def beside(left, right):
    """The session with the view's pane on its right, a herdr split between them."""
    screen = left
    for y in range(ROWS):
        screen.buffer[y][LEFT] = screen.buffer[y][LEFT]._replace(data="│", fg="2c2c30")
        for x in range(PANE):
            screen.buffer[y][LEFT + 1 + x] = right.buffer[y][x]
    return screen


chart_pane = pane("signups-q3", chart, PANE, ROWS)
frames = [  # (seconds on screen, screen)
    (1.4, session(COLS, 0, None)),
    (1.2, session(COLS, 2, None)),
    (3.6, beside(session(LEFT, 4, "open"), chart_pane)),
    (2.6, session(COLS, 4, "hidden")),
    (3.4, beside(session(LEFT, 4, "open"), chart_pane)),
]

open(os.path.join(OUT, "show.svg"), "w").write(animate(frames, "herdr · show"))
print(len(frames), "frames")
