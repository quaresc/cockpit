"""The whole cockpit in one picture: herdr's sidebar, a Claude session and its footer, from fictional data."""
import os, sys
sys.path.insert(0, os.path.dirname(__file__))
from term2svg import screen_of, svg

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
# Two agents on web's quota-ui branch: both rows carry the same-branch warning.
agents = [("✓", TEAL, "[6] daily-quotas-api", "◑ 52%", ORANGE, " api · ENG-412", DIMC, ""),
          ("●", ORANGE, "[7] quota-banner", "◔ 31%", DIMC, " web · ENG-413   5/9", ORANGE, "⚠ same branch as quota-copy"),
          ("✓", TEAL, "[8] quota-copy", "◔ 18%", DIMC, " web · ENG-413", DIMC, "⚠ same branch as quota-banner"),
          ("○", DIMC, "[9] invoice-csv", "◕ 74%", RED, " billing · ENG-398", DIMC, "")]
r = 18
for i, (state, sc, name, ctx, cc, work, wc, clash) in enumerate(agents):
    put(r, 2, state, sc); put(r, 4, name, bold=i == 1); put(r, 4 + len(name), "  " + ctx, cc)
    put(r + 1, 4, work, wc)
    if clash:
        put(r + 2, 4, clash, RED, bold=True)
    r += 3 if clash else 2
for r in range(1, ROWS + 1):
    put(r, SIDE + 1, "│", "2C2C30")

# The Claude session.
L = SIDE + 4
put(2, L, "❯ ", DIMC); put(2, L + 2, "Add the quota banner to the settings page, ENG-413 in Linear, design in the Figma link.", WHITE)
put(4, L, "●", WHITE); put(4, L + 2, "Reading the ticket, its Figma frame and the quota fields the API now returns.")
put(5, L + 2, "Read 4 files, ran 3 shell commands", DIMC)
put(7, L, "●", WHITE); put(7, L + 2, "The banner sits above the search form and reads dailyLimit and usedToday from")
put(8, L + 2, "/v1/account. At the limit it turns red and disables New search, as in the frame.")
put(10, L + 2, "web/src/settings/QuotaBanner.tsx       new, 58 lines", DIMC)
put(11, L + 2, "web/src/settings/SearchForm.tsx        +12 −3", DIMC)
put(12, L + 2, "web/src/settings/QuotaBanner.test.tsx  new, 4 tests", DIMC)
put(14, L + 2, "Tests pass. I pushed to me/eng-413-quota-ui: CI is running on web#1836 (5/9 jobs)")
put(15, L + 2, "and the preview sandbox picks it up once the image is built.")
put(17, L, "✻ Worked for 4m 12s", DIMC)
put(ROWS - 4, L, "─" * (COLS - L), "2C2C30"); put(ROWS - 3, L, "❯ ", DIMC); put(ROWS - 2, L, "─" * (COLS - L), "2C2C30")
put(ROWS - 1, L, "⏵⏵ auto mode on", ORANGE)
PILLS = [(" ENG-413", True), (" 1836", False), ("\U000f03d8 figma", False), ("\U000f0b15 notion", False),
                    ("\U000f0833 quotas", False), (" 09:30 · Team daily", False)]
c = COLS - sum(len(label) + 4 for label, _ in PILLS) - 6
for label, main in PILLS:
    c = pill(ROWS - 1, c, label, main) + 1
put(ROWS - 1, c, "\U000f09d1 31%", DIMC)

screen = screen_of("".join(ops), COLS, ROWS)
with open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "overview.svg"), "w") as f:
    f.write(svg(screen, "herdr · cockpit"))
