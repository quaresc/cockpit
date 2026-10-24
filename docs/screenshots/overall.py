"""The whole cockpit in one picture: herdr's sidebar, a Claude session and its footer, from fictional data."""
import os, sys
sys.path.insert(0, os.path.dirname(__file__))
from term2svg import BAR, CW, LH, PAD, picture, screen_of, svg

COLS, ROWS, SIDE = 150, 30, 34
ORANGE, TEAL, RED, DIMC, INK, QUIET, WHITE = "FFC799", "99FFE4", "FF8080", "707070", "101010", "2C2C30", "D2D2D2"
ops = []

def put(r, c, text, fg=None, bg=None, bold=False, dim=False):
    sgr = ["0"] + (["1"] if bold else []) + (["2"] if dim else [])
    if fg: sgr.append("38;2;%d;%d;%d" % tuple(int(fg[i:i + 2], 16) for i in (0, 2, 4)))
    if bg: sgr.append("48;2;%d;%d;%d" % tuple(int(bg[i:i + 2], 16) for i in (0, 2, 4)))
    ops.append(f"\033[{r};{c}H\033[{';'.join(sgr)}m{text}\033[0m")

def pill(r, c, label, main=False):
    fill, ink = (ORANGE, INK) if main else (QUIET, WHITE)
    put(r, c, "", fill); put(r, c + 1, label, ink, fill); put(r, c + 1 + len(label), "", fill)
    return c + len(label) + 3

# Sidebar: pinned spaces with their counts, then the agents.
put(1, 2, "spaces", DIMC)
rows = [("[1]   inbox", None, True), ("    1 you · 2 team", DIMC, False), ("    1 failing", RED, False),
        ("    4 active · 7", DIMC, False), ("  \U000f029a  5h 38% · 7d 21%", DIMC, False),
        ("[2]   terminal", None, False), ("[3]   to-do", None, False), ("    3 open", DIMC, False),
        ("[4]   runbook", None, False), ("    2 runbooks · 8 to do", DIMC, False),
        ("[5]   meetings", None, False), ("    tomorrow 09:30 · Team daily", ORANGE, False)]
for i, (text, fg, bold) in enumerate(rows):
    put(3 + i, 2, text, fg, bold=bold)
put(16, 2, "agents", DIMC)
# Each row: state, the agent's topic, the page beside it (split like a browser's split view) and its context use.
agents = [("✓", TEAL, "daily-quotas-api", "", "◑ 52%", ORANGE, "\uf418 api · ENG-412", DIMC, ""),
          ("●", ORANGE, "quota-ui", "\uf0ac Talent search", "◔ 31%", DIMC, "\uf418 web · ENG-413   5/9", ORANGE, "⚠ same branch as quota-copy"),
          ("✓", TEAL, "quota-copy", "", "◔ 18%", DIMC, "\uf418 web · ENG-413", DIMC, "⚠ same branch as quota-ui"),
          ("○", DIMC, "invoice-csv", "", "◕ 74%", RED, "\uf418 billing · ENG-398", DIMC, "")]
r = 18
for i, (state, sc, name, side, ctx, cc, work, wc, clash) in enumerate(agents):
    put(r, 2, state, sc); put(r, 4, name, bold=True)
    c = 4 + len(name)
    if side:
        put(r, c + 1, "│", DIMC); put(r, c + 3, side, ORANGE); c += 3 + len(side)
    put(r, c + 2, ctx, cc)
    put(r + 1, 4, work, wc)
    if clash:
        put(r + 2, 4, clash, RED, bold=True)
    r += 3 if clash else 2
for r in range(1, ROWS + 1):
    put(r, SIDE + 1, "│", "2C2C30")

# The Claude session, checking its change in the browser beside it.
L, B = SIDE + 4, 104
put(2, L, "❯ ", DIMC); put(2, L + 2, "Add the quota banner, ENG-413. Check it on", WHITE)
put(3, L + 2, "phone and desktop before the PR.", WHITE)
put(5, L, "●", WHITE); put(5, L + 2, "The banner reads dailyLimit and usedToday")
put(6, L + 2, "from /v1/account; at the limit it turns red")
put(7, L + 2, "and disables New search, as in the frame.")
put(9, L + 2, "web/src/settings/QuotaBanner.tsx   new", DIMC)
put(10, L + 2, "web/src/settings/SearchForm.tsx    +12 −3", DIMC)
put(12, L, "●", WHITE); put(12, L + 2, "side-browser", ORANGE); put(12, L + 15, "localhost:5173/branch/", DIMC)
put(13, L + 2, "iPhone 16 · iPad · 1440 × 900, before and after", DIMC)
put(15, L, "●", WHITE); put(15, L + 2, "Checked on all three sizes: the banner wraps")
put(16, L + 2, "on the phone, the button disables at the limit.")
put(17, L + 2, "Screenshots are in the PR description, ready")
put(18, L + 2, "for you to save.")
put(20, L, "✻ Worked for 6m 40s", DIMC)
put(ROWS - 4, L, "─" * (B - L - 1), "2C2C30"); put(ROWS - 3, L, "❯ ", DIMC); put(ROWS - 2, L, "─" * (B - L - 1), "2C2C30")
put(ROWS - 1, L, "⏵⏵ auto mode on", ORANGE)

# terminal-browser beside the agent: the page it is testing, emulated as a phone.
for r in range(1, ROWS - 1):
    put(r, B, "│", "2C2C30")
put(1, B + 2, " ", DIMC); put(1, B + 4, "localhost:5173/branch/", WHITE); put(1, COLS - 12, "iPhone 16", DIMC)
put(2, B + 2, " agent in control ", INK, ORANGE)
PILLS = [("\uf41b ENG-413", True), ("\uf407 1836", False), ("\U000f03d8 figma", False), ("\U000f0b15 notion", False),
         ("\U000f0833 quotas", False), ("\uf08e browser", False)]
c = COLS - sum(len(label) + 4 for label, _ in PILLS) - 2
for label, main in PILLS:
    c = pill(ROWS - 1, c, label, main) + 1

screen = screen_of("".join(ops), COLS, ROWS)
# The page beside the agent: a real screenshot of the fake app in webapp/, as shots.sh took it (see README.md).
shots = sys.argv[1] if len(sys.argv) > 1 else os.path.expanduser("~/.cache/cockpit/shots/acme-web-quota")
x0, y0, w, h = PAD + B * CW + 6, BAR + PAD + 3 * LH, (COLS - B - 1) * CW - 12, (ROWS - 5) * LH
data, (iw, ih) = picture(os.path.join(shots, "after", "mobile.png"), w, h)
page = f'<image href="data:image/jpeg;base64,{data}" x="{x0 + (w - iw) / 2:.1f}" y="{y0:.1f}" width="{iw}" height="{ih}"/>'
with open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "overview.svg"), "w") as f:
    f.write(svg(screen, "herdr · cockpit").replace("</svg>", page + "</svg>"))
