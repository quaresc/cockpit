"""Sequence diagrams in a terminal: one lifeline per actor, each message an arrow between two of them.

`draw(actors, messages, room)` returns rows of (text, style) runs. A message is {from, to, label, reply}; a reply is
drawn dashed. A message to oneself loops back on the same lifeline, and {note, over} writes a note beside an actor.
"""

import textwrap

from flow import Canvas, width
from view import BOLD, DIM, ORANGE

GAP = 4


def draw(actors, messages, room=80):
    names = [a["id"] if isinstance(a, dict) else a for a in actors]
    labels = {a["id"]: a.get("label") or a["id"] for a in actors if isinstance(a, dict)}
    for m in messages:
        for end in (m.get("from"), m.get("to"), m.get("over")):
            if end and end not in names:
                names.append(end)
    labels = {a: labels.get(a, a) for a in names}
    # Neighbouring lifelines sit far enough apart for the longest label between them.
    boxes = {a: width(labels[a]) + 4 for a in names}
    spacing = [0] * max(0, len(names) - 1)
    for m in messages:
        if m.get("from") in names and m.get("to") in names and m["from"] != m["to"]:
            i, j = sorted((names.index(m["from"]), names.index(m["to"])))
            need = width(m.get("label") or "") + 4
            per = -(-need // (j - i))
            for k in range(i, j):
                spacing[k] = max(spacing[k], per)
        elif m.get("from") in names and m.get("from") == m.get("to") and names.index(m["from"]) < len(spacing):
            k = names.index(m["from"])
            spacing[k] = max(spacing[k], width(m.get("label") or "") + 4)
    # Too wide for the pane: every gap shrinks to its share of the room, and the labels wrap to fit.
    least = sum(boxes.values()) // 2 + GAP * max(0, len(names) - 1)
    if spacing and least + sum(spacing) > room:
        share = max(14, (room - sum(boxes[a] for a in names) // 2) // len(spacing))
        spacing = [min(s, share) for s in spacing]
    centres, at = [], 0
    for k, a in enumerate(names):
        if k:
            at = max(centres[-1] + boxes[names[k - 1]] // 2 + boxes[a] // 2 + GAP, centres[-1] + spacing[k - 1] + 1)
        else:
            at = boxes[a] // 2
        centres.append(at)
    centre = dict(zip(names, centres))

    canvas = Canvas()

    def heads(row):
        for a in names:
            w, left = boxes[a], centre[a] - boxes[a] // 2
            canvas.put(row, left, "┌" + "─" * (w - 2) + "┐", ORANGE)
            canvas.put(row + 1, left, "│", ORANGE)
            canvas.put(row + 1, left + 2, labels[a], BOLD)
            canvas.put(row + 1, left + w - 1, "│", ORANGE)
            canvas.put(row + 2, left, "└" + "─" * (w - 2) + "┘", ORANGE)

    heads(0)
    row = 4
    arrows = []
    for m in messages:
        if m.get("note") is not None:
            canvas.put(row, centre[m["over"]] + 2, f"▏{m['note']}", DIM)
            row += 2
            continue
        a, b, label = m["from"], m["to"], m.get("label") or ""
        ca, cb = centre[a], centre[b]
        room_for_label = max(8, (abs(cb - ca) if a != b else room - ca) - 3)
        lines = textwrap.wrap(label, room_for_label) or [""]
        for line in lines:
            canvas.put(row, (min(ca, cb) if a != b else ca) + 2, line, "")
            row += 1
        row -= 1
        if a == b:
            arrows.append(((row + 1, ca), (row + 1, ca + 3), (row + 2, ca + 3), (row + 2, ca + 1), "◀", m.get("reply")))
            row += 4
            continue
        arrows.append(((row + 1, ca), (row + 1, cb + (-2 if cb > ca else 2)), None, None, "▶" if cb > ca else "◀", m.get("reply")))
        row += 3
    bottom = row
    for a in names:
        canvas.segment((3, centre[a]), (bottom, centre[a]))
    for start, end, turn, back, head, is_reply in arrows:
        points = [start, end] + ([turn, back] if turn else [])
        for p, q in zip(points, points[1:]):
            canvas.segment(p, q)
        tip = back if turn else end
        canvas.put(tip[0], tip[1] + (0 if turn else (1 if head == "▶" else -1)), head, ORANGE)
        if is_reply:
            r, c0 = start
            c1 = end[1]
            for c in range(min(c0, c1), max(c0, c1) + 1):
                if c not in centre.values():
                    canvas.put(r, c, "╌", DIM)
    heads(bottom + 1)
    return canvas.rows()
