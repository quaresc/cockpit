"""Flow charts drawn in a terminal: boxes in layers, edges routed at right angles with arrowheads.

`draw(nodes, edges, direction, width)` lays the chart out top to bottom ("down") or left to right ("right") and returns
its rows, each a list of (text, style) runs. An edge back to an earlier layer runs around the chart on its far side.
"""

import textwrap
import unicodedata

from view import BOLD, DIM, GREEN, ORANGE

LABEL_WIDTH = 24
# Room between neighbours across a layer: columns when going down, rows when going right.
CROSS_GAP = {"down": 3, "right": 1}
EDGE, ARROW, LABEL = DIM, ORANGE, DIM
BOX = {"box": ("┌┐└┘─│", ""), "round": ("╭╮╰╯─│", ORANGE), "decision": ("╔╗╚╝═║", GREEN)}
U, D, L, R = 1, 2, 4, 8
LINES = {U: "│", D: "│", U | D: "│", L: "─", R: "─", L | R: "─", D | R: "╭", D | L: "╮", U | R: "╰", U | L: "╯",
         U | D | R: "├", U | D | L: "┤", D | L | R: "┬", U | L | R: "┴", U | D | L | R: "┼"}


def width(text):
    return sum(2 if unicodedata.east_asian_width(c) in "WF" else 1 for c in text)


class Canvas:
    """Cells of line directions, merged into box-drawing joins, under cells of text."""

    def __init__(self):
        self.lines, self.text = {}, {}

    def segment(self, a, b):
        """A straight line between two (row, col) cells, inclusive."""
        (r0, c0), (r1, c1) = a, b
        if r0 == r1:
            for c in range(min(c0, c1), max(c0, c1)):
                self.lines[(r0, c)] = self.lines.get((r0, c), 0) | R
                self.lines[(r0, c + 1)] = self.lines.get((r0, c + 1), 0) | L
        else:
            for r in range(min(r0, r1), max(r0, r1)):
                self.lines[(r, c0)] = self.lines.get((r, c0), 0) | D
                self.lines[(r + 1, c0)] = self.lines.get((r + 1, c0), 0) | U

    def put(self, row, col, text, style):
        for c in text:
            self.text[(row, col)] = (c, style)
            col += width(c)

    def rows(self):
        cells = set(self.lines) | set(self.text)
        if not cells:
            return []
        height, wide = max(r for r, _ in cells) + 1, max(c for _, c in cells) + 1
        out = []
        for r in range(height):
            runs, c = [], 0
            while c < wide:
                if (r, c) in self.text:
                    ch, style = self.text[(r, c)]
                elif (r, c) in self.lines:
                    ch, style = LINES[self.lines[(r, c)]], EDGE
                else:
                    ch, style = " ", ""
                if runs and runs[-1][1] == style:
                    runs[-1][0] += ch
                else:
                    runs.append([ch, style])
                c += max(1, width(ch))
            out.append([(t.rstrip() if i == len(runs) - 1 else t, s) for i, (t, s) in enumerate(runs)])
        return out


def layers_of(ids, edges):
    """Each node's layer, the longest path to it over forward edges; the edges that close a cycle point backwards."""
    after = {n: [] for n in ids}
    for e in edges:
        after[e["from"]].append(e["to"])
    state, back = {}, set()

    def visit(n):
        state[n] = "open"
        for m in after[n]:
            if state.get(m) == "open":
                back.add((n, m))
            elif m not in state:
                visit(m)
        state[n] = "done"

    for n in ids:
        if n not in state:
            visit(n)
    layer = {n: 0 for n in ids}
    for _ in ids:
        for e in edges:
            if (e["from"], e["to"]) not in back and e["from"] != e["to"]:
                layer[e["to"]] = max(layer[e["to"]], layer[e["from"]] + 1)
    return layer, back


def draw(nodes, edges, direction="down", room=80):
    down = direction != "right"
    ids = [n["id"] for n in nodes]
    layer, back = layers_of(ids, edges)

    # Box sizes: the label wrapped, a border and a space each side.
    wrap = max(8, min(LABEL_WIDTH, room // 3 - 4))
    boxes = {}
    for n in nodes:
        lines = textwrap.wrap(n["label"], wrap) or [""]
        w, h = max(width(l) for l in lines) + 4, len(lines) + 2
        boxes[n["id"]] = {"lines": lines, "w": w, "h": h, "kind": n.get("kind", "box"),
                          "main": h if down else w, "cross": w if down else h}

    # An edge spanning several layers passes through a one-cell stand-in in each layer between.
    forward = [e for e in edges if (e["from"], e["to"]) not in back and e["from"] != e["to"]]
    order = [[] for _ in range(max(layer.values()) + 1)]
    for n in ids:
        order[layer[n]].append(n)
    paths = []
    for k, e in enumerate(forward):
        path = [e["from"]]
        for l in range(layer[e["from"]] + 1, layer[e["to"]]):
            dummy = f"\0{k}:{l}"
            boxes[dummy] = {"main": 0, "cross": 1, "dummy": True}
            layer[dummy] = l
            order[l].append(dummy)
            path.append(dummy)
        paths.append((path + [e["to"]], e))

    # Order each layer by where its parents sit, twice down the chart.
    parents = {}
    for path, _ in paths:
        for a, b in zip(path, path[1:]):
            parents.setdefault(b, []).append(a)
    for _ in range(2):
        for l in range(1, len(order)):
            index = {n: i for i, n in enumerate(order[l - 1])}
            order[l].sort(key=lambda n: sum(index[p] for p in parents.get(n, [])) / max(1, len(parents.get(n, []))) if parents.get(n) else 10 ** 6)

    # Across: each box as close as it can get to its parents' middle, never overlapping its neighbour.
    gap = CROSS_GAP["down" if down else "right"]
    pos = {}
    mid = lambda n: pos[n] + (boxes[n]["cross"] - 1) // 2
    for l, members in enumerate(order):
        edge_at = None
        for n in members:
            ps = parents.get(n, [])
            centre = sum(mid(p) for p in ps) / len(ps) if ps else (edge_at + gap if edge_at is not None else 0)
            start = round(centre) - (boxes[n]["cross"] - 1) // 2
            if edge_at is not None:
                start = max(start, edge_at + gap)
            pos[n] = start
            edge_at = start + boxes[n]["cross"]
    low = min(pos.values())
    for n in pos:
        pos[n] -= low

    # Along: each layer as thick as its biggest box, then a gap holding the stubs, the crossing tracks and the arrows.
    tracks, tail = [], []
    for l in range(len(order) - 1):
        spans = []
        for path, e in paths:
            for a, b in zip(path, path[1:]):
                if layer[a] == l:
                    ca, cb = mid(a), mid(b)
                    spans.append((min(ca, cb), max(ca, cb), a, b, e))
        lanes = []  # each a list of spans that share one track
        assigned = {}
        for span in sorted(spans, key=lambda s: s[0]):
            if span[0] == span[1]:
                continue
            for i, lane in enumerate(lanes):
                if all(span[0] > o[1] + 1 or span[1] < o[0] - 1 or o[2] == span[2] or o[3] == span[3] for o in lane):
                    lane.append(span)
                    assigned[(span[2], span[3])] = i
                    break
            else:
                lanes.append([span])
                assigned[(span[2], span[3])] = len(lanes) - 1
        tracks.append((max(1, len(lanes)), assigned))
        longest = max([width(s[4].get("label") or "") for s in spans if s[3] == s[4]["to"]] + [0])
        tail.append(1 if down or not longest else longest + 2)
    start, at = [], 0
    for l, members in enumerate(order):
        start.append(at)
        at += max([boxes[n]["main"] for n in members] + [1])
        if l < len(tracks):
            at += 1 + tracks[l][0] + tail[l]

    canvas = Canvas()
    cell = (lambda m, c: (m, c)) if down else (lambda m, c: (c, m))
    thick = [max([boxes[n]["main"] for n in members] + [1]) for members in order]

    # Forward edges: out of the source's far side, along a track, into the target with an arrow.
    for path, e in paths:
        for a, b in zip(path, path[1:]):
            l = layer[a]
            ca, cb = mid(a), mid(b)
            out = start[l] + (boxes[a]["main"] if not boxes[a].get("dummy") else thick[l]) - (0 if boxes[a].get("dummy") else 1)
            if boxes[a].get("dummy"):
                canvas.segment(cell(start[l], ca), cell(start[l] + thick[l], ca))
            arrow = start[l + 1] - 1
            if boxes[b].get("dummy"):
                arrow = start[l + 1]
            track = start[l] + thick[l] + 1 + tracks[l][1].get((a, b), 0)
            if ca == cb:
                canvas.segment(cell(out, ca), cell(arrow, cb))
            else:
                canvas.segment(cell(out, ca), cell(track, ca))
                canvas.segment(cell(track, ca), cell(track, cb))
                canvas.segment(cell(track, cb), cell(arrow, cb))
            if not boxes[b].get("dummy"):
                r, c = cell(arrow, cb)
                canvas.put(r, c, "▼" if down else "▶", ARROW)
                if e.get("label"):
                    if down:
                        canvas.put(r, c + 2, e["label"], LABEL)
                    else:
                        canvas.put(r - 1, c - width(e["label"]) - 1, e["label"], LABEL)

    # Back edges and loops: out of the source's side, around the chart, into the target's side.
    far = max(pos[n] + boxes[n]["cross"] for n in ids) + 1
    for k, e in enumerate(e for e in edges if (e["from"], e["to"]) in back or e["from"] == e["to"]):
        a, b = e["from"], e["to"]
        lane = far + 1 + 2 * k
        ma, mb = start[layer[a]] + boxes[a]["main"] // 2, start[layer[b]] + boxes[b]["main"] // 2
        side_a, side_b = pos[a] + boxes[a]["cross"], pos[b] + boxes[b]["cross"]
        canvas.segment(cell(ma, side_a), cell(ma, lane))
        canvas.segment(cell(ma, lane), cell(mb, lane))
        canvas.segment(cell(mb, lane), cell(mb, side_b))
        r, c = cell(mb, side_b)
        canvas.put(r, c, "◀" if down else "▲", ARROW)
        if e.get("label"):
            r, c = cell((ma + mb) // 2, lane)
            canvas.put(r, c + 2, e["label"], LABEL) if down else canvas.put(r + 1, c, e["label"], LABEL)

    # Boxes last, over any line that reached them.
    for n in ids:
        box = boxes[n]
        top, left = cell(start[layer[n]], pos[n])
        (tl, tr, bl, br, hz, vt), border = BOX.get(box["kind"], BOX["box"])
        w, h = box["w"], box["h"]
        canvas.put(top, left, tl + hz * (w - 2) + tr, border)
        for i, text in enumerate(box["lines"]):
            pad = w - 4 - width(text)
            canvas.put(top + 1 + i, left, vt, border)
            canvas.put(top + 1 + i, left + 1, " " * (1 + pad // 2) + text + " " * (1 + pad - pad // 2), BOLD if box["kind"] != "box" else "")
            canvas.put(top + 1 + i, left + w - 1, vt, border)
        canvas.put(top + h - 1, left, bl + hz * (w - 2) + br, border)
    return canvas.rows()
