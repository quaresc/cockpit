"""Animated SVGs composed from fictional screens: the sidebar's rows changing, the footer filling with links, and an E2E
test in terminal-browser beside the agent, its page drawn from real screenshots of the fake app in webapp/.

    compose_anim.py [<shots dir>]   the before/ and after/ screenshots shots.sh took (see README.md)
"""
import os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from term2svg import BAR, CW, LH, PAD, animate, put_ops, screen_of, timed_pictures, with_pictures

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "..")
SHOTS = sys.argv[1] if len(sys.argv) > 1 else os.path.expanduser("~/.cache/cockpit/shots/acme-web-quota")
ORANGE, TEAL, RED, DIMC, INK, QUIET, WHITE, LINE = "FFC799", "99FFE4", "FF8080", "707070", "101010", "2C2C30", "D2D2D2", "2C2C30"


def pill(put, r, c, label, main=False):
    fill, ink = (ORANGE, INK) if main else (QUIET, WHITE)
    put(r, c, "", fill); put(r, c + 1, label, ink, fill); put(r, c + 1 + len(label), "", fill)
    return c + len(label) + 3


def pills(put, r, right, labels):
    """Footer chips, right-aligned to column `right`; the first is the ticket's, filled."""
    c = right - sum(len(label) + 4 for label in labels) + 1
    for n, label in enumerate(labels):
        c = pill(put, r, c, label, main=n == 0) + 1


# ---------------------------------------------------------------------------
# Sidebar: CI runs on an agent's branch, the agent finishes, a review request arrives.

COLS, ROWS = 40, 25


def sidebar(review, failing, ci, quota_state, invoice_state):
    ops, put = put_ops()
    put(1, 2, "spaces", DIMC)
    rows = [("[1] \U000f01ef  inbox", None, True), (f"     {review}", DIMC, False),
            (f"     {failing}" if failing else "     all green", RED if failing else TEAL, False),
            ("     4 active · 7", DIMC, False), ("    \U000f029a 5h 38% · 7d 21%", DIMC, False),
            ("[2]   terminal", None, False), ("[3]   to-do", None, False), ("    3 open", DIMC, False),
            ("[4]   runbook", None, False), ("    2 runbooks · 8 to do", DIMC, False),
            ("[5]   meetings", None, False), ("    tomorrow 09:30 · Team daily", ORANGE, False)]
    for i, (text, fg, bold) in enumerate(rows):
        put(3 + i, 2, text, fg, bold=bold)
    put(16, 2, "agents", DIMC)
    agents = [(*quota_state, "quota-ui", " Talent search", "◔ 31%", DIMC, f" web · ENG-413{ci}", ORANGE if ci else DIMC),
              ("✓", TEAL, "daily-quotas", "", "◑ 52%", ORANGE, " api · ENG-412", DIMC),
              (*invoice_state, "invoice-csv", "", "◕ 74%", RED, " billing · ENG-398", DIMC)]
    r = 18
    for state, sc, name, side, ctx, cc, work, wc in agents:
        put(r, 2, state, sc); put(r, 4, name, bold=True)
        c = 4 + len(name)
        if side:
            put(r, c + 1, "│", DIMC); put(r, c + 3, side, ORANGE); c += 3 + len(side)
        put(r, c + 2, ctx, cc)
        put(r + 1, 4, work, wc)
        r += 2
    return screen_of("".join(ops), COLS, ROWS)


working, done, waiting = ("●", ORANGE), ("✓", TEAL), ("⏸", RED)
frames = [(1.6, sidebar("1 you · 2 team", "1 failing", "   3/9", working, done)),
          (1.0, sidebar("1 you · 2 team", "1 failing", "   6/9", working, done)),
          (1.0, sidebar("1 you · 2 team", "1 failing", "   9/9", working, done)),
          (1.6, sidebar("1 you · 2 team", "", "", done, done)),
          (1.4, sidebar("2 you · 2 team", "", "", done, done)),
          (2.4, sidebar("2 you · 2 team", "", "", done, waiting))]
open(os.path.join(OUT, "sidebar.svg"), "w").write(animate(frames, "herdr · sidebar"))

# ---------------------------------------------------------------------------
# Footer: the chips follow what the conversation names.

COLS, ROWS = 110, 16
TALK = [("❯", "Pick up ENG-413, the quota banner. Design: figma.com/design/q7/Quotas", WHITE),
        ("●", "Read ENG-413 and the Figma frame; the spec page is on Notion.", None),
        ("●", "Opened web#1836 on me/eng-413-quota-ui. The preview pv-quotas is building.", None),
        ("●", "Sentry shows the old banner crashing on null limits: sentry.io/acme/issues/4512", None)]
TICKET, PR, FIGMA, NOTION, SENTRY = "\uf41b ENG-413", "\uf407 1836", "\U000f03d8 figma", "\U000f0b15 notion", "\U000f0a30 sentry"
ARGO, PREVIEW = "\U000f0833 pv-quotas", "\uf0ac preview"


def footer(turns, chips):
    ops, put = put_ops()
    for n, (mark, text, fg) in enumerate(TALK[:turns]):
        put(2 + n * 2, 2, mark, DIMC if mark == "❯" else WHITE); put(2 + n * 2, 4, text, fg)
    put(ROWS - 4, 2, "─" * (COLS - 2), LINE); put(ROWS - 3, 2, "❯ ", DIMC); put(ROWS - 2, 2, "─" * (COLS - 2), LINE)
    put(ROWS - 1, 2, "⏵⏵ auto mode on", ORANGE)
    if chips:
        pills(put, ROWS - 1, COLS - 1, chips)
    return screen_of("".join(ops), COLS, ROWS)


frames = [(1.2, footer(0, [])), (1.4, footer(1, [TICKET, FIGMA])), (1.4, footer(2, [TICKET, FIGMA, NOTION])),
          (1.6, footer(3, [TICKET, PR, FIGMA, NOTION, ARGO, PREVIEW])), (2.8, footer(4, [TICKET, PR, FIGMA, NOTION, SENTRY, ARGO, PREVIEW]))]
open(os.path.join(OUT, "footer.svg"), "w").write(animate(frames, "herdr · footer"))

# ---------------------------------------------------------------------------
# E2E tests: the agent opens its page in terminal-browser beside it, phone then desktop, main then branch.

COLS, ROWS, B = 150, 30, 84
STEPS = [("❯", "Check the quota banner on phone and desktop, main against the branch.", WHITE),
         ("●", "side-browser  localhost:5173/main/ · iPhone 16", ORANGE),
         ("●", "Before: no banner, Search enabled.", None),
         ("●", "side-browser  localhost:5173/branch/ · iPhone 16", ORANGE),
         ("●", "After: the banner wraps on the phone and Search is disabled.", None),
         ("●", "side-browser  localhost:5173/branch/ · 1440 × 900", ORANGE),
         ("●", "Desktop matches the frame.", None),
         ("●", "show  before / after; the screenshots are in the PR, ready to save.", ORANGE)]


def e2e(turns, url=None, device=""):
    ops, put = put_ops()
    width = B - 2 if url else COLS - 2
    for n, (mark, text, fg) in enumerate(STEPS[:turns]):
        put(2 + n * 2, 2, mark, DIMC if mark == "❯" else WHITE); put(2 + n * 2, 4, text[:width - 4], fg)
    put(ROWS - 4, 2, "─" * (width - 1), LINE); put(ROWS - 3, 2, "❯ ", DIMC); put(ROWS - 2, 2, "─" * (width - 1), LINE)
    put(ROWS - 1, 2, "⏵⏵ auto mode on", ORANGE)
    labels = [" ENG-413", " 1836"] + ([" browser"] if url else [])
    pills(put, ROWS - 1, width, labels)
    if url:
        for r in range(1, ROWS + 1):
            put(r, B, "│", LINE)
        put(1, B + 2, "", DIMC); put(1, B + 4, url, WHITE); put(1, COLS - len(device) - 1, device, DIMC)
        put(2, B + 2, " agent in control ", INK, ORANGE)
    return screen_of("".join(ops), COLS, ROWS)


frames = [(1.4, e2e(1)), (1.0, e2e(2)), (2.4, e2e(3, "localhost:5173/main/", "iPhone 16")),
          (1.0, e2e(4, "localhost:5173/main/", "iPhone 16")), (2.6, e2e(5, "localhost:5173/branch/", "iPhone 16")),
          (1.0, e2e(6, "localhost:5173/branch/", "iPhone 16")), (2.6, e2e(7, "localhost:5173/branch/", "1440 × 900")),
          (3.6, e2e(8, "show · quota-ui-shots", "before / after"))]
pages = {2: "before/mobile.png", 4: "after/mobile.png", 6: "after/desktop.png", 7: os.path.join(OUT, "compare.png")}
# The page area: below the browser's two bars, right of the split.
box = (PAD + B * CW + 6, BAR + PAD + 3 * LH, (COLS - B - 1) * CW - 12, (ROWS - 4) * LH)
style, images = timed_pictures(frames, {i: path if os.path.isabs(path) else os.path.join(SHOTS, path) for i, path in pages.items()}, box)
svg = with_pictures(animate(frames, "herdr · E2E test in terminal-browser"), style, images)
open(os.path.join(OUT, "e2e.svg"), "w").write(svg)
print("sidebar, footer, e2e")
