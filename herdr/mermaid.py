"""Mermaid, the part a terminal can draw: flowcharts become flow blocks and sequence diagrams sequence blocks.

`parse(text)` returns the block it describes. Styling, classes, subgraphs and sequence boxes (loop, alt, rect) are read
past, since a terminal draws none of them.
"""

import re

NODE = re.compile(r"\s*([A-Za-z0-9_.\-]+)\s*(\(\[.*?\]\)|\[\[.*?\]\]|\[\(.*?\)\]|\(\(.*?\)\)|\{\{.*?\}\}|\[.*?\]|\(.*?\)|\{.*?\}|>.*?\])?")
LINK = re.compile(r"\s*(?:(?:--|==|-\.)\s*([^-=>|.][^>|]*?)\s*(?:-->|==>|\.->)|(?:-->|---|==>|===|-\.->|-\.-|--x|--o)(?:\s*\|([^|]*)\|)?)")
ARROW = re.compile(r"^\s*([^\s:\-+>]+)\s*(-->>|->>|-->|->|--x|-x|--\)|-\))\s*[+-]?\s*([^\s:]+)\s*:\s*(.*)$")


def node(match):
    """A node's id, label and kind from `A`, `A[label]`, `A(label)`, `A{label}` and the rest."""
    ident, shape = match[1], match[2] or ""
    if not shape:
        return ident, ident, "box"
    kind = "decision" if shape.startswith("{") else "round" if shape.startswith("(") else "box"
    label = shape.strip("[](){}>").strip().strip('"')
    return ident, label or ident, kind


def flowchart(lines, header):
    direction = "right" if re.search(r"\b(LR|RL)\b", header) else "down"
    nodes, edges = {}, []

    def remember(ident, label, kind, is_shaped):
        if ident not in nodes or is_shaped:
            nodes[ident] = {"id": ident, "label": label, "kind": kind}

    for raw in lines:
        for statement in raw.split(";"):
            text = statement.strip()
            if not text or text.startswith(("%%", "classDef", "class ", "style ", "linkStyle", "click ", "subgraph", "end", "direction")):
                continue
            at, previous = 0, None
            while at < len(text):
                found = NODE.match(text, at)
                if not found or not found[1]:
                    break
                ident, label, kind = node(found)
                remember(ident, label, kind, bool(found[2]))
                if previous:
                    edges.append({"from": previous[0], "to": ident, **({"label": previous[1]} if previous[1] else {})})
                at = found.end()
                link = LINK.match(text, at)
                if not link:
                    break
                previous = (ident, (link[1] or link[2] or "").strip())
                at = link.end()
    return {"type": "flow", "direction": direction, "nodes": list(nodes.values()), "edges": edges}


def sequence(lines):
    actors, messages = [], []

    def actor(ident, label=None):
        for a in actors:
            if a["id"] == ident:
                if label:
                    a["label"] = label
                return
        actors.append({"id": ident, "label": label or ident})

    for raw in lines:
        text = raw.strip()
        declared = re.match(r"^(participant|actor)\s+(\S+)(?:\s+as\s+(.+))?$", text)
        note = re.match(r"^[Nn]ote\s+(?:left of|right of|over)\s+([^,:]+)(?:,[^:]+)?\s*:\s*(.*)$", text)
        arrow = ARROW.match(text)
        if declared:
            actor(declared[2], declared[3])
        elif note:
            actor(note[1].strip())
            messages.append({"note": note[2], "over": note[1].strip()})
        elif arrow:
            actor(arrow[1])
            actor(arrow[3])
            messages.append({"from": arrow[1], "to": arrow[3], "label": arrow[4].strip(), "reply": arrow[2].startswith("--")})
    return {"type": "sequence", "actors": actors, "messages": messages}


def parse(text):
    lines = [l for l in text.splitlines() if l.strip() and not l.strip().startswith("%%")]
    if not lines:
        raise ValueError("empty mermaid")
    header = lines[0].strip()
    if header.startswith("sequenceDiagram"):
        return sequence(lines[1:])
    if header.startswith(("graph", "flowchart")):
        return flowchart(lines[1:], header)
    raise ValueError(f"only flowchart and sequenceDiagram are drawn, not {header.split()[0]}")
