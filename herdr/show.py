#!/usr/bin/env python3
"""A view a Claude session shows beside itself: markdown, tables, charts and diffs, from ~/.cache/cockpit/show/<name>.json.

The cockpit mod's show tool writes that file and opens this app in a herdr pane split from the session's own. Each later
call with the same name rewrites the file and the pane redraws; a repo diff also follows the working tree. q hides it.
"""

import atexit
import os
import re
import subprocess
import sys
import time
from datetime import datetime

import cockpit_config
import view
from view import BOLD, CAP_CLOSE, CAP_OPEN, DIM, GREEN, HERDR, LINK, ORANGE, PILL_MAIN, RED, Line, fit, note, width

NAME = sys.argv[1] if len(sys.argv) > 1 else "show"
DIR = os.path.join(os.path.expanduser("~"), ".cache", "cockpit", "show")
SPEC = os.path.join(DIR, f"{NAME}.json")
ALIVE = os.path.join(DIR, f"{NAME}.pid")
# A repo diff is read again this often, so the pane follows the agent's edits.
DIFF_EVERY = 3
# herdr-reviewr, when installed, takes over a repo diff for line comments sent back to the agent.
REVIEWR = "persiyanov.reviewr"
PLUGINS = os.path.join(os.path.expanduser("~"), ".config", "herdr", "plugins.json")

BLUE, PINK = "\033[38;2;160;160;255m", "\033[38;2;255;153;204m"
SERIES = [ORANGE, GREEN, BLUE, PINK, RED]


# ---------------------------------------------------------------------------
# Text: inline Markdown, wrapped to the pane


INLINE = re.compile(r"\*\*(.+?)\*\*|`([^`]+)`|\[([^\]]+)\]\((https?://[^)\s]+)\)")


def inline(text, style=""):
    """Segments (text, style, action) of one line: **bold**, `code` and [links](url)."""
    out, position = [], 0
    for match in INLINE.finditer(text):
        if match.start() > position:
            out.append((text[position:match.start()], style, None))
        if match[1] is not None:
            out.append((match[1], style + BOLD, None))
        elif match[2] is not None:
            out.append((match[2], ORANGE, None))
        else:
            out.append((match[3], style + LINK, lambda url=match[4]: cockpit_config.open_url(url)))
        position = match.end()
    if position < len(text):
        out.append((text[position:], style, None))
    return out


def wrap(segments, room):
    """Splits styled segments into rows of at most `room` cells, breaking between words."""
    rows, row, used = [], [], 0
    for text, style, action in segments:
        for word in re.findall(r"\S+\s*|\s+", text):
            size = width(word.rstrip())
            if used and used + size > room:
                rows.append(row)
                row, used = [], 0
                word = word.lstrip()
            while width(word) > room:
                row.append((word[:room], style, action))
                rows.append(row)
                row, used, word = [], 0, word[room:]
            row.append((word, style, action))
            used += width(word)
    rows.append(row)
    return rows


def markdown(block, cols, line):
    room = max(20, cols - 4)
    in_code = False
    for raw in block["text"].splitlines():
        if raw.strip().startswith("```"):
            in_code = not in_code
            continue
        if in_code:
            line().add("  │ ", DIM).add(fit(raw.replace("\t", "    "), room - 2), DIM)
            continue
        heading = re.match(r"^(#{1,3})\s+(.*)", raw)
        bullet = re.match(r"^(\s*)([-*•]|\d+[.)])\s+(.*)", raw)
        if heading:
            line().add("  ").add(heading[2], BOLD + (ORANGE if len(heading[1]) == 1 else ""))
        elif bullet:
            indent = "  " + bullet[1] + ("• " if bullet[2] in "-*•" else bullet[2] + " ")
            for n, row in enumerate(wrap(inline(bullet[3]), room - width(indent) + 2)):
                item = line().add(indent if n == 0 else " " * width(indent), DIM)
                for text, style, action in row:
                    item.add(text, style, action)
        elif raw.startswith(">"):
            for row in wrap(inline(raw.lstrip("> "), DIM), room - 2):
                item = line().add("  ▎ ", DIM)
                for text, style, action in row:
                    item.add(text, style, action)
        elif not raw.strip():
            line()
        else:
            for row in wrap(inline(raw), room):
                item = line().add("  ")
                for text, style, action in row:
                    item.add(text, style, action)


# ---------------------------------------------------------------------------
# Tables


def is_number(value):
    return isinstance(value, (int, float)) and not isinstance(value, bool)


def cell(value):
    if value is None:
        return "—"
    if isinstance(value, bool):
        return "yes" if value else "no"
    if isinstance(value, int):
        return f"{value:,}"
    if isinstance(value, float):
        return f"{value:,.2f}".rstrip("0").rstrip(".")
    return str(value)


def table(block, cols, line):
    columns, rows = block["columns"], block["rows"]
    texts = [[cell(v) for v in row] for row in rows]
    sizes = [max([width(str(c))] + [width(t[i]) for t in texts]) for i, c in enumerate(columns)]
    # Too wide for the pane: the widest columns give way first, down to 6 cells.
    room = cols - 4 - 2 * (len(columns) - 1)
    while sum(sizes) > room and max(sizes) > 6:
        sizes[sizes.index(max(sizes))] -= 1
    numeric = [all(is_number(r[i]) or r[i] is None for r in rows) and any(is_number(r[i]) for r in rows) for i in range(len(columns))]

    def aligned(text, i):
        return (" " * max(0, sizes[i] - width(text)) + text) if numeric[i] and width(text) <= sizes[i] else fit(text, sizes[i])

    head = line().add("  ")
    for i, column in enumerate(columns):
        head.add(aligned(str(column), i) + ("  " if i < len(columns) - 1 else ""), BOLD + DIM)
    line().add("  " + "─" * min(cols - 4, sum(sizes) + 2 * (len(columns) - 1)), DIM)
    for row, values in zip(texts, rows):
        item = line().add("  ")
        for i, text in enumerate(row):
            item.add(aligned(text, i) + ("  " if i < len(columns) - 1 else ""), DIM if values[i] is None else "")


# ---------------------------------------------------------------------------
# Charts: braille line charts (2×4 dots a cell) and horizontal bar charts


def compact(value):
    """A number in at most five or six characters: 1234 → 1.2k, 3400000 → 3.4M."""
    for limit, suffix in ((1e9, "B"), (1e6, "M"), (1e3, "k")):
        if abs(value) >= limit:
            return f"{value / limit:.1f}".rstrip("0").rstrip(".") + suffix
    return f"{value:.2f}".rstrip("0").rstrip(".") if isinstance(value, float) else str(value)


def legend(series, line):
    if len(series) > 1:
        item = line().add("  ")
        for n, name in enumerate(series):
            item.add("● ", SERIES[n % len(SERIES)]).add(f"{name}   ", DIM)


BITS = ((0x01, 0x08), (0x02, 0x10), (0x04, 0x20), (0x40, 0x80))


def line_chart(block, cols, line):
    columns, rows = block["columns"], block["rows"]
    xi, ys = columns.index(block["x"]), [columns.index(y) for y in block["y"]]
    values = [r[i] for r in rows for i in ys if is_number(r[i])]
    if not values:
        line().add("  no numbers to draw", DIM)
        return
    lo, hi = min(0, min(values)), max(values)
    hi = hi if hi > lo else lo + 1
    labels = [compact(hi), compact((hi + lo) / 2), compact(lo)]
    axis = max(width(l) for l in labels) + 1
    w, h = max(10, cols - axis - 5), block.get("height") or 12
    grid = [[0] * w for _ in range(h)]
    owner = [[None] * w for _ in range(h)]

    def dot(px, py, n):
        cx, cy = px // 2, py // 4
        if 0 <= cx < w and 0 <= cy < h:
            grid[cy][cx] |= BITS[py % 4][px % 2]
            owner[cy][cx] = n

    span = max(1, len(rows) - 1)
    for n, i in enumerate(ys):
        points = [(round(k * (w * 2 - 1) / span), round((hi - r[i]) * (h * 4 - 1) / (hi - lo)))
                  for k, r in enumerate(rows) if is_number(r[i])]
        for (x0, y0), (x1, y1) in zip(points, points[1:] + points[-1:]):
            steps = max(abs(x1 - x0), abs(y1 - y0), 1)
            for s in range(steps + 1):
                dot(round(x0 + (x1 - x0) * s / steps), round(y0 + (y1 - y0) * s / steps), n)
    for y in range(h):
        label = labels[0] if y == 0 else labels[1] if y == h // 2 else labels[2] if y == h - 1 else ""
        item = line().add("  " + " " * (axis - 1 - width(label)) + label + " ", DIM).add("┤" if label else "│", DIM)
        for x in range(w):
            n = owner[y][x]
            item.add(chr(0x2800 + grid[y][x]) if grid[y][x] else " ", SERIES[n % len(SERIES)] if n is not None else "")
    line().add("  " + " " * axis + "└" + "─" * w, DIM)
    first, middle, last = (cell(rows[k][xi]) for k in (0, len(rows) // 2, -1))
    gap = max(1, (w - width(first) - width(middle) - width(last)) // 2)
    line().add("  " + " " * (axis + 1) + first + " " * gap + middle + " " * gap + last, DIM)
    legend(block["y"], line)


EIGHTHS = " ▏▎▍▌▋▊▉█"


def bar_chart(block, cols, line):
    columns, rows = block["columns"], block["rows"]
    xi, ys = columns.index(block["x"]), [columns.index(y) for y in block["y"]]
    values = [r[i] for r in rows for i in ys if is_number(r[i])]
    if not values:
        line().add("  no numbers to draw", DIM)
        return
    top = max(max(values), 0) or 1
    label_room = min(28, max(width(cell(r[xi])) for r in rows))
    value_room = max(width(compact(v)) for v in values) + 1
    room = max(8, cols - 4 - label_room - value_room - 2)
    for r in rows:
        for n, i in enumerate(ys):
            value = r[i] if is_number(r[i]) else None
            eighths = round(max(0, value or 0) / top * room * 8)
            bar = "█" * (eighths // 8) + (EIGHTHS[eighths % 8] if eighths % 8 else "")
            label = fit(cell(r[xi]), label_room) if n == 0 else " " * label_room
            line().add("  " + label + " ", DIM if n else "").add(bar, SERIES[n % len(SERIES)]).add(" " + (compact(value) if value is not None else "—"), DIM)
    legend(block["y"], line)


# ---------------------------------------------------------------------------
# Diffs: a patch given whole, or a repo's working tree against a base, read again every few seconds


def git_diff(repo, base):
    """The repo's changes against `base`, untracked files included as new ones."""
    run = lambda *args: subprocess.run(["git", "-C", repo, *args], capture_output=True, text=True, timeout=15)
    patch = run("diff", base or "HEAD", "--")
    if patch.returncode != 0:
        return None, patch.stderr.strip() or f"git diff {base or 'HEAD'} failed"
    untracked = [f for f in run("ls-files", "--others", "--exclude-standard").stdout.splitlines() if f][:30]
    extra = "".join(run("diff", "--no-index", "--", "/dev/null", f).stdout for f in untracked)
    return patch.stdout + extra, None


def parse_patch(patch):
    """Files of a unified diff: their path, counts and lines, each line (kind, old number, new number, text)."""
    files, current, old, new = [], None, 0, 0
    for raw in patch.splitlines():
        if raw.startswith("diff --git "):
            current = {"path": raw.split(" b/", 1)[-1], "added": 0, "removed": 0, "lines": []}
            files.append(current)
        elif current is None:
            continue
        elif raw.startswith("+++ ") and raw[4:] != "/dev/null":
            current["path"] = raw[4:].removeprefix("b/")
        elif raw.startswith(("--- ", "index ", "new file", "deleted file", "similarity", "rename ", "old mode", "new mode")):
            continue
        elif raw.startswith("@@"):
            numbers = re.match(r"@@ -(\d+)(?:,\d+)? \+(\d+)", raw)
            old, new = (int(numbers[1]), int(numbers[2])) if numbers else (0, 0)
            current["lines"].append(("hunk", None, None, raw))
        elif raw.startswith("+"):
            current["added"] += 1
            current["lines"].append(("add", None, new, raw[1:]))
            new += 1
        elif raw.startswith("-"):
            current["removed"] += 1
            current["lines"].append(("del", old, None, raw[1:]))
            old += 1
        elif raw.startswith("\\"):
            current["lines"].append(("note", None, None, raw))
        else:
            current["lines"].append(("same", old, new, raw[1:]))
            old, new = old + 1, new + 1
    return files


def has_reviewr():
    return any(p.get("plugin_id") == REVIEWR for p in view.load_json(PLUGINS, []))


def open_reviewr(state, repo):
    """Opens herdr-reviewr on the repo, zoomed over this pane, with the keyboard."""
    argv = [HERDR, "plugin", "pane", "open", "--plugin", REVIEWR, "--entrypoint", "pane", "--cwd", repo, "--placement", "zoomed", "--focus"]
    if os.environ.get("HERDR_PANE_ID"):
        argv += ["--target-pane", os.environ["HERDR_PANE_ID"]]
    try:
        opened = subprocess.run(argv, capture_output=True, text=True, timeout=10)
    except (OSError, subprocess.SubprocessError) as err:
        note(state, f"reviewr did not open: {err}")
        return
    note(state, "opened reviewr; q there comes back" if opened.returncode == 0 else f"reviewr did not open: {opened.stderr.strip()[:120]}")


def diff(block, cols, line, state, key):
    if block.get("patch") is not None:
        patch, error = block["patch"], None
    else:
        patch, error = state["diffs"].get(key, ("", None))
    if error:
        line().add(f"  {error}", RED)
        return
    files = parse_patch(patch or "")
    if not files:
        line().add("  no changes", DIM)
        return
    added, removed = sum(f["added"] for f in files), sum(f["removed"] for f in files)
    against = "" if block.get("patch") is not None else f" against {block.get('base') or 'HEAD'}"
    head = line().add(f"  {len(files)} file{'s' if len(files) != 1 else ''}{against}  ", DIM).add(f"+{added}", GREEN).add(" ").add(f"−{removed}", RED)
    if block.get("repo") and state["reviewr"]:
        review = lambda: open_reviewr(state, block["repo"])
        state["review"] = state["review"] or review
        head.add("   ").add(CAP_OPEN, PILL_MAIN[0], review).add("\uf06e reviewr", PILL_MAIN[1], review).add(CAP_CLOSE, PILL_MAIN[0], review)
        head.primary = review
    for f in files:
        fold = (key, f["path"])
        is_folded = fold in state["folded"]
        head = line()
        toggle = lambda fold=fold: state["folded"].__ixor__({fold})
        head.add(f"  {'▸' if is_folded else '▾'} ", DIM, toggle).add(f["path"], BOLD, toggle)
        head.add("  ").add(f"+{f['added']}", GREEN).add(" ").add(f"−{f['removed']}", RED)
        head.primary = toggle
        if is_folded:
            continue
        for kind, old, new, text in f["lines"]:
            text = text.replace("\t", "    ")
            if kind == "hunk":
                line().add("    " + fit(text, cols - 6), BLUE)
                continue
            number = new if new is not None else old
            gutter = f"{number:>5} " if number is not None else "      "
            sign, style = {"add": ("+", GREEN), "del": ("-", RED), "same": (" ", ""), "note": (" ", DIM)}[kind]
            line().add("  " + gutter, DIM).add(sign + " " + fit(text, cols - 12), style)


# ---------------------------------------------------------------------------
# The app


def build(state, cols):
    spec = state["spec"]
    lines = []

    def line():
        lines.append(Line())
        return lines[-1]

    state["review"] = None
    if not spec:
        line().add(f"  waiting for the view {NAME}…", DIM)
        return lines
    head = line().add("◆  ", ORANGE).add(spec.get("title") or NAME, BOLD + ORANGE)
    head.add(f"   {NAME} · {datetime.fromtimestamp(spec.get('at', 0) / 1000).strftime('%H:%M:%S')}", DIM)
    for n, block in enumerate(spec.get("blocks", [])):
        line()
        if block.get("title"):
            line().add("  " + block["title"], BOLD)
        kind = block["type"]
        if kind == "markdown":
            markdown(block, cols, line)
        elif kind == "table":
            table(block, cols, line)
        elif kind == "chart":
            (bar_chart if block.get("kind") == "bar" else line_chart)(block, cols, line)
        elif kind == "diff":
            diff(block, cols, line, state, n)
    line()
    files = " · j/k files · enter folds" if any(b["type"] == "diff" for b in spec.get("blocks", [])) else ""
    files += " · r reviewr" if state["review"] else ""
    line().add(view.noted(state) or f"q hide · space/b page · g/G top/bottom{files}", DIM)
    return lines


def read_diffs(state):
    """Runs git again for every repo diff of the view; True when one changed."""
    changed = False
    for n, block in enumerate((state["spec"] or {}).get("blocks", [])):
        if block.get("type") == "diff" and block.get("patch") is None and block.get("repo"):
            result = git_diff(block["repo"], block.get("base"))
            changed |= state["diffs"].get(n) != result
            state["diffs"][n] = result
    state["diffs_at"] = time.time()
    return changed


def poll(state):
    mtime = os.path.getmtime(SPEC) if os.path.exists(SPEC) else 0
    if mtime != state["mtime"]:
        state["mtime"], state["spec"] = mtime, view.load_json(SPEC, None)
        state["reviewr"] = has_reviewr()
        state["diffs"] = {}
        read_diffs(state)
        return True
    if time.time() - state["diffs_at"] > DIFF_EVERY:
        return read_diffs(state)
    return False


def on_key(state, data):
    """Pages scroll the view itself, since most of its lines are not rows to select."""
    rows = view.size().lines
    moves = {" ": rows - 2, "f": rows - 2, view.PAGE_DOWN: rows - 2, "b": -(rows - 2), view.PAGE_UP: -(rows - 2),
             "d": rows // 2, "u": -(rows // 2), "G": 10 ** 6, "g": -(10 ** 6)}
    if data == "q":
        sys.exit(0)
    if data == "r" and state["review"]:
        state["review"]()
        return True
    if data not in moves:
        return False
    state["top"], state["has_keyboard"], state["hover"] = max(0, state["top"] + moves[data]), False, -1
    return True


def forget():
    try:
        os.remove(ALIVE)
    except OSError:
        pass


def main():
    os.makedirs(DIR, exist_ok=True)
    # The mod reads this to update the open pane instead of splitting a new one.
    with open(ALIVE, "w", encoding="utf-8") as handle:
        handle.write(str(os.getpid()))
    atexit.register(forget)
    state = view.new_state(spec=None, mtime=None, diffs={}, diffs_at=0, folded=set(), reviewr=has_reviewr(), review=None)
    view.App(NAME, state, build, {}, poll=poll, on_key=on_key).run()


if __name__ == "__main__":
    main()
