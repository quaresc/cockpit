"""The show tool, animated: an agent opens a chart beside the conversation, q hides it to a footer chip, the chip brings it
back. Also stills of a flow chart and of a repo diff with its reviewr button, from fictional data, drawn by the real show.py."""
import json, os, shutil, subprocess, sys, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from term2svg import run, rows_svg, screen_of, svg, window

HERE = os.path.dirname(os.path.abspath(__file__))
HOME = os.path.join(HERE, "home")
APPS = os.path.join(HERE, "..", "..", "herdr")
OUT = os.path.join(HERE, "..")
COLS, ROWS, LEFT = 150, 30, 76
PANE = COLS - LEFT - 1
ORANGE, DIMC, INK, QUIET, WHITE, LINE = "FFC799", "707070", "101010", "2C2C30", "D2D2D2", "2C2C30"
env = {"HOME": HOME, "TERM": "xterm-256color", "HERDR_BIN_PATH": "/usr/bin/true"}


def pane(name, spec, cols, rows, keys=()):
    """The show.py screen for this spec, at this size."""
    os.makedirs(os.path.join(HOME, ".cache", "cockpit", "show"), exist_ok=True)
    json.dump({**spec, "at": int(time.time() * 1000)}, open(os.path.join(HOME, ".cache", "cockpit", "show", f"{name}.json"), "w"))
    return screen_of(run(["python3", "show.py", name], cols, rows, env, keys, wait=2.0, cwd=APPS), cols, rows)


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

# One loop: the first frame is the base, each later one a layer shown from its start to the loop's end.
anim = sum(d for d, _ in frames)
keys = ["\n".join("".join(s.buffer[y][x].data + str(s.buffer[y][x].fg) + str(s.buffer[y][x].bg) for x in range(COLS)) for y in range(ROWS))
        for _, s in frames]
chars, layers, style, begin = set(), [], "", 0
for i, (duration, screen) in enumerate(frames):
    if i:
        before, now = keys[i - 1].split("\n"), keys[i].split("\n")
        differ = [y for y in range(ROWS) if before[y] != now[y]]
        a = 100 * begin / anim
        style += f"@keyframes s{i}{{0%,{a:.2f}%{{visibility:hidden}}{a:.2f}%,100%{{visibility:visible}}}}" \
                 f".s{i}{{visibility:hidden;animation:s{i} {anim:.2f}s step-end infinite}}"
        layers.append(f'<g class="s{i}">{rows_svg(screen, differ, chars, cover=True)}</g>')
    begin += duration
body = rows_svg(frames[0][1], range(ROWS), chars) + "".join(layers)
open(os.path.join(OUT, "show.svg"), "w").write(window(COLS, ROWS, "herdr · show", chars, body, style))
print(len(frames), "frames,", f"{anim:.1f}s loop")

# The flow chart still: a sign-in that refreshes an expired token, with a loop back.
N = lambda i, label, kind="box": {"id": i, "label": label, "kind": kind}
E = lambda a, b, label=None: {"from": a, "to": b, **({"label": label} if label else {})}
signin = {"title": "Calling the API", "blocks": [{"type": "flow", "nodes": [
    N("req", "Request from the app", "round"), N("valid", "Access token valid?", "decision"), N("api", "Call the API"),
    N("refresh", "Refresh the token"), N("login", "Send to sign-in", "round"), N("done", "Show the data", "round")],
    "edges": [E("req", "valid"), E("valid", "api", "yes"), E("valid", "refresh", "no"), E("refresh", "valid", "new token"),
              E("refresh", "login", "refused"), E("api", "done")]}]}
chart_flow = pane("flow", signin, 72, 30)
used = max(y for y in range(30) if "".join(chart_flow.buffer[y][x].data for x in range(72)).strip()) + 1
open(os.path.join(OUT, "show-flow.svg"), "w").write(svg(chart_flow, "show · flow", crop_rows=used))

# The diff still: a small repo with work in progress, reviewr installed so the header carries its button.
repo = os.path.join(HOME, "code", "web")
shutil.rmtree(repo, ignore_errors=True); os.makedirs(os.path.join(repo, "src"))
git = lambda *a: subprocess.run(["git", "-C", repo, *a], capture_output=True, check=True)
git("init", "-q"); git("config", "user.email", "me@example.com"); git("config", "user.name", "me")
banner = os.path.join(repo, "src", "QuotaBanner.tsx")
open(banner, "w").write("export function QuotaBanner({ used, limit }: Props) {\n  return (\n    <Banner tone=\"info\">\n"
                        "      {used} of {limit} searches today\n    </Banner>\n  )\n}\n")
git("add", "-A"); git("commit", "-qm", "Quota banner")
open(banner, "w").write("export function QuotaBanner({ used, limit }: Props) {\n  const isFull = used >= limit\n  return (\n"
                        "    <Banner tone={isFull ? \"danger\" : \"info\"}>\n      {used} of {limit} searches today\n"
                        "      {isFull && <Link href=\"/plans\">Raise your limit</Link>}\n    </Banner>\n  )\n}\n")
open(os.path.join(repo, "src", "QuotaBanner.test.tsx"), "w").write("it('turns red at the limit', () => {\n"
                                                                   "  render(<QuotaBanner used={20} limit={20} />)\n})\n")
os.makedirs(os.path.join(HOME, ".config", "herdr"), exist_ok=True)
json.dump([{"plugin_id": "persiyanov.reviewr", "plugin_root": "/dev/null"}], open(os.path.join(HOME, ".config", "herdr", "plugins.json"), "w"))
diff = pane("changes", {"title": "Quota banner, work in progress", "blocks": [{"type": "diff", "repo": repo}]}, 96, 26)
used = max(y for y in range(26) if "".join(diff.buffer[y][x].data for x in range(96)).strip()) + 1
open(os.path.join(OUT, "show-diff.svg"), "w").write(svg(diff, "show · changes", crop_rows=used))
