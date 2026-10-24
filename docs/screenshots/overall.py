"""The whole cockpit at once, animated: herdr's sidebar, an agent working on a ticket, the page it tests in terminal-browser
beside it and its footer, from fictional data. The page is a real screenshot of the fake app in webapp/ (see README.md).

    overall.py [<shots dir>]
"""
import os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from term2svg import BAR, CW, LH, PAD, animate, put_ops, screen_of, timed_pictures, with_pictures

SHOTS = sys.argv[1] if len(sys.argv) > 1 else os.path.expanduser("~/.cache/cockpit/shots/acme-web-quota")
COLS, ROWS, SIDE, B = 150, 30, 34, 104
L = SIDE + 4
ORANGE, TEAL, RED, DIMC, INK, QUIET, WHITE, LINE = "FFC799", "99FFE4", "FF8080", "707070", "101010", "2C2C30", "D2D2D2", "2C2C30"
TALK = [
    [("❯ ", "Add the quota banner, ENG-413. Check it on", WHITE), ("", "phone and desktop before the PR.", WHITE)],
    [("●", "The banner reads dailyLimit and usedToday", None), ("", "from /v1/account; at the limit it turns red", None),
     ("", "and disables Search, as in the Figma frame.", None)],
    [("●", "side-browser  localhost:5173/main/", ORANGE), ("", "iPhone 16 · 1440 × 900, main then branch", DIMC)],
    [("●", "Checked both sizes: the banner wraps on the", None), ("", "phone and Search disables at the limit.", None),
     ("", "Before/after is in the PR, ready to save.", None)],
]
TICKET, PR, FIGMA, NOTION, ARGO, BROWSER = (" ENG-413", " 1836", "\U000f03d8 figma", "\U000f0b15 notion",
                                            "\U000f0833 pv-quotas", " browser")


def frame(turns, ci, done, failing, chips, url=None, review="1 you · 2 team"):
    ops, put = put_ops()

    def pill(r, c, label, main=False):
        fill, ink = (ORANGE, INK) if main else (QUIET, WHITE)
        put(r, c, "", fill); put(r, c + 1, label, ink, fill); put(r, c + 1 + len(label), "", fill)
        return c + len(label) + 3

    # Sidebar: pinned spaces with their counts, then one row per agent.
    put(1, 2, "spaces", DIMC)
    rows = [("[1] \U000f01ef  inbox", None, True), (f"     {review}", DIMC, False),
            ("     1 failing" if failing else "     all green", RED if failing else TEAL, False),
            ("     4 active · 7", DIMC, False), ("    \U000f029a 5h 38% · 7d 21%", DIMC, False),
            ("[2]   terminal", None, False), ("[3]   to-do", None, False), ("    3 open", DIMC, False),
            ("[4]   runbook", None, False), ("    2 runbooks · 8 to do", DIMC, False),
            ("[5]   meetings", None, False), ("    tomorrow 09:30 · Team daily", ORANGE, False)]
    for i, (text, fg, bold) in enumerate(rows):
        put(3 + i, 2, text, fg, bold=bold)
    put(16, 2, "agents", DIMC)
    state = ("✓", TEAL) if done else ("●", ORANGE)
    agents = [("✓", TEAL, "daily-quotas-api", "", "◑ 52%", ORANGE, " api · ENG-412", DIMC, ""),
              (*state, "quota-ui", " Talent" if url else "", "◔ 31%", DIMC, f" web · ENG-413{ci}", ORANGE if ci else DIMC,
               "⚠ same branch as quota-copy"),
              ("✓", TEAL, "quota-copy", "", "◔ 18%", DIMC, " web · ENG-413", DIMC, "⚠ same branch as quota-ui"),
              ("○", DIMC, "invoice-csv", "", "◕ 74%", RED, " billing · ENG-398", DIMC, "")]
    r = 18
    for st, sc, name, side, ctx, cc, work, wc, clash in agents:
        put(r, 2, st, sc); put(r, 4, name, bold=True)
        c = 4 + len(name)
        if side:
            put(r, c + 1, "│", DIMC); put(r, c + 3, side, ORANGE); c += 3 + len(side)
        put(r, c + 2, ctx, cc)
        put(r + 1, 4, work, wc)
        if clash:
            put(r + 2, 4, clash, RED, bold=True)
        r += 3 if clash else 2
    for r in range(1, ROWS + 1):
        put(r, SIDE + 1, "│", LINE)

    # The Claude session.
    r = 2
    for turn in TALK[:turns]:
        for mark, text, fg in turn:
            if mark:
                put(r, L, mark, DIMC if mark.startswith("❯") else WHITE)
            put(r, L + 2, text, fg)
            r += 1
        r += 1
    if done:
        put(r, L, "✻ Worked for 6m 40s", DIMC)
    put(ROWS - 4, L, "─" * (B - L - 1), LINE); put(ROWS - 3, L, "❯ ", DIMC); put(ROWS - 2, L, "─" * (B - L - 1), LINE)
    put(ROWS - 1, L, "⏵⏵ auto mode on", ORANGE)

    # terminal-browser beside the agent, once it opened.
    for r in range(1, ROWS - 1):
        put(r, B, "│", LINE)
    if url:
        put(1, B + 2, "", DIMC); put(1, B + 4, url, WHITE); put(1, COLS - 12, "iPhone 16", DIMC)
        put(2, B + 2, " agent in control ", INK, ORANGE)
    else:
        put(ROWS // 2 - 1, B + 12, "no page beside the agent", DIMC)

    # Footer chips, right-aligned; the ticket's first, filled.
    c = COLS - sum(len(label) + 4 for label in chips) - 1
    for n, label in enumerate(chips):
        c = pill(ROWS - 1, c, label, n == 0) + 1
    return screen_of("".join(ops), COLS, ROWS)


frames = [
    (1.6, frame(1, "", False, True, [TICKET])),
    (1.6, frame(2, "", False, True, [TICKET, FIGMA])),
    (2.4, frame(3, "", False, True, [TICKET, FIGMA, BROWSER], "localhost:5173/main/")),
    (1.4, frame(3, "    3/9", False, True, [TICKET, PR, FIGMA, BROWSER], "localhost:5173/branch/")),
    (1.2, frame(3, "    7/9", False, True, [TICKET, PR, FIGMA, NOTION, BROWSER], "localhost:5173/branch/")),
    (3.6, frame(4, "", True, False, [TICKET, PR, FIGMA, NOTION, ARGO, BROWSER], "localhost:5173/branch/", "2 you · 2 team")),
]
box = (PAD + B * CW + 6, BAR + PAD + 3 * LH, (COLS - B - 1) * CW - 12, (ROWS - 6) * LH)
style, images = timed_pictures(frames, {0: None, 2: os.path.join(SHOTS, "before", "mobile.png"), 3: os.path.join(SHOTS, "after", "mobile.png")}, box)
open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "overview.svg"), "w").write(
    with_pictures(animate(frames, "herdr · cockpit"), style, images))
print(len(frames), "frames")
