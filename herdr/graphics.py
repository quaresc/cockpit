"""Charts and flow charts as SVG, rasterized with resvg for terminals that draw images (Kitty graphics, Sixel).

Every size is in pixels of the pane, so a chart drawn at the pane's cell size fills whole cells; `font` is the label size.
"""

import html
import math
import re

import resvg_py

import flow

BG, PANEL, GRID, AXIS, TEXT, FG = "#101010", "#1E1E22", "#2A2A2E", "#6E6E74", "#9A9AA0", "#D2D2D2"
SERIES = ["#FFC799", "#99FFE4", "#A0A0FF", "#FF99CC", "#FF8080"]
SANS = "Helvetica Neue, Helvetica, Arial, sans-serif"


def png(svg, width=None):
    return bytes(resvg_py.svg_to_bytes(svg_string=svg, width=width))


def text_width(text, font):
    """A label's width, estimated: resvg lays text out itself, so this only has to leave enough room."""
    return sum(0.62 if c.isupper() or c.isdigit() else 0.5 for c in str(text)) * font


def esc(text):
    return html.escape(str(text))


def compact(value):
    for limit, suffix in ((1e9, "B"), (1e6, "M"), (1e3, "k")):
        if abs(value) >= limit:
            return f"{value / limit:.1f}".rstrip("0").rstrip(".") + suffix
    return f"{value:.2f}".rstrip("0").rstrip(".") if isinstance(value, float) else str(value)


def ticks(lo, hi, count=4):
    """Round tick values covering lo..hi: 0, 200, 400… rather than 0, 213.3, 426.7…"""
    span = (hi - lo) or 1
    step = 10 ** math.floor(math.log10(span / count))
    step *= next(m for m in (1, 2, 2.5, 5, 10) if span / (step * m) <= count)
    start, end = math.floor(lo / step) * step, math.ceil(hi / step) * step
    return [start + i * step for i in range(round((end - start) / step) + 1)]


def svg(width, height, body, defs=""):
    return (f'<svg xmlns="http://www.w3.org/2000/svg" width="{width:.0f}" height="{height:.0f}" viewBox="0 0 {width:.0f} {height:.0f}" '
            f'font-family="{SANS}"><defs>{defs}</defs>{body}</svg>')


def legend(names, x, y, font):
    out = []
    for n, name in enumerate(names):
        color = SERIES[n % len(SERIES)]
        out.append(f'<circle cx="{x + font * 0.35:.1f}" cy="{y - font * 0.32:.1f}" r="{font * 0.3:.1f}" fill="{color}"/>'
                   f'<text x="{x + font * 0.9:.1f}" y="{y:.1f}" font-size="{font:.1f}" fill="{TEXT}">{esc(name)}</text>')
        x += font * 2 + text_width(name, font)
    return "".join(out)


def line_chart(block, width, height, font, is_number, axes, cell):
    """Lines over a soft area, round y ticks, the last value of each series at its end."""
    rows = block["rows"]
    xi, ys, names = axes(block)
    values = [r[i] for r in rows for i in ys if is_number(r[i])]
    if not rows or not values:
        return None
    marks = ticks(min(0, min(values)), max(values))
    lo, hi = marks[0], marks[-1]
    s = font / 16
    left = max(text_width(compact(m), font) for m in marks) + font * 1.2
    right = text_width(compact(values[-1]), font) + font * 1.4
    top, bottom = font * (2.4 if len(ys) > 1 else 1.2), font * 2.2
    pw, ph = width - left - right, height - top - bottom
    x_of = lambda k: left + (pw * k / max(1, len(rows) - 1))
    y_of = lambda v: top + ph * (1 - (v - lo) / ((hi - lo) or 1))
    body, defs = [f'<rect width="{width}" height="{height}" fill="{BG}"/>'], []
    for m in marks:
        y = y_of(m)
        body.append(f'<line x1="{left:.1f}" x2="{left + pw:.1f}" y1="{y:.1f}" y2="{y:.1f}" stroke="{GRID}" stroke-width="{s:.2f}"'
                    + ('' if m == lo else f' stroke-dasharray="{4 * s:.1f} {4 * s:.1f}"') + '/>')
        body.append(f'<text x="{left - font * 0.6:.1f}" y="{y + font * 0.35:.1f}" font-size="{font:.1f}" fill="{AXIS}" text-anchor="end">{esc(compact(m))}</text>')
    shown = max(2, min(len(rows), int(pw // (font * 6))))
    for k in sorted({round(i * (len(rows) - 1) / (shown - 1)) for i in range(shown)}):
        anchor = "start" if k == 0 else "end" if k == len(rows) - 1 else "middle"
        body.append(f'<text x="{x_of(k):.1f}" y="{height - font * 0.6:.1f}" font-size="{font:.1f}" fill="{AXIS}" text-anchor="{anchor}">{esc(cell(rows[k][xi]))}</text>')
    for n, i in enumerate(ys):
        color = SERIES[n % len(SERIES)]
        points = [(x_of(k), y_of(r[i])) for k, r in enumerate(rows) if is_number(r[i])]
        if not points:
            continue
        line = " ".join(f"{'M' if j == 0 else 'L'}{x:.1f},{y:.1f}" for j, (x, y) in enumerate(points))
        if n == 0:
            defs.append(f'<linearGradient id="area" x1="0" x2="0" y1="0" y2="1"><stop offset="0" stop-color="{color}" stop-opacity="0.28"/>'
                        f'<stop offset="1" stop-color="{color}" stop-opacity="0"/></linearGradient>')
            body.append(f'<path d="{line} L{points[-1][0]:.1f},{y_of(lo):.1f} L{points[0][0]:.1f},{y_of(lo):.1f} Z" fill="url(#area)"/>')
        body.append(f'<path d="{line}" fill="none" stroke="{color}" stroke-width="{2.4 * s:.2f}" stroke-linejoin="round" stroke-linecap="round"/>')
        x, y = points[-1]
        last = next(r[i] for r in reversed(rows) if is_number(r[i]))
        body.append(f'<circle cx="{x:.1f}" cy="{y:.1f}" r="{5 * s:.1f}" fill="{BG}" stroke="{color}" stroke-width="{2.4 * s:.2f}"/>'
                    f'<text x="{x + font * 0.6:.1f}" y="{y + font * 0.35:.1f}" font-size="{font:.1f}" fill="{color}" font-weight="600">{esc(compact(last))}</text>')
    if len(ys) > 1:
        body.append(legend(names, left, font * 1.2, font))
    return svg(width, height, "".join(body), "".join(defs))


def bar_chart(block, width, font, is_number, axes, cell):
    """Horizontal bars on a faint track, the value at each bar's end; returns the SVG and its height."""
    rows = block["rows"]
    xi, ys, names = axes(block)
    values = [r[i] for r in rows for i in ys if is_number(r[i])]
    if not rows or not values:
        return None, 0
    top_value = max(max(values), 0) or 1
    bar, gap = font * 1.15, font * 0.55
    label = min(text_width("M" * 22, font), max(text_width(cell(r[xi]), font) for r in rows)) + font
    value_room = max(text_width(compact(v), font) for v in values) + font * 1.2
    top = font * 2.4 if len(ys) > 1 else font * 0.4
    track = width - label - value_room
    body, y = [], top
    for r in rows:
        body.append(f'<text x="{label - font * 0.6:.1f}" y="{y + bar * 0.5 * len(ys) + font * 0.35:.1f}" font-size="{font:.1f}" fill="{FG}" '
                    f'text-anchor="end">{esc(cell(r[xi]))}</text>')
        for n, i in enumerate(ys):
            value = r[i] if is_number(r[i]) else 0
            color = SERIES[n % len(SERIES)]
            length = max(bar * 0.4, track * max(0, value) / top_value)
            body.append(f'<rect x="{label:.1f}" y="{y:.1f}" width="{track:.1f}" height="{bar * 0.82:.1f}" rx="{bar * 0.3:.1f}" fill="{PANEL}"/>'
                        f'<rect x="{label:.1f}" y="{y:.1f}" width="{length:.1f}" height="{bar * 0.82:.1f}" rx="{bar * 0.3:.1f}" fill="{color}" fill-opacity="0.9"/>'
                        f'<text x="{label + length + font * 0.5:.1f}" y="{y + bar * 0.41 + font * 0.35:.1f}" font-size="{font:.1f}" fill="{TEXT}">'
                        f'{esc(compact(value)) if is_number(r[i]) else "—"}</text>')
            y += bar
        y += gap
    if len(ys) > 1:
        body.insert(0, legend(names, label, font * 1.3, font))
    height = y + font * 0.2
    return svg(width, height, f'<rect width="{width}" height="{height:.0f}" fill="{BG}"/>' + "".join(body)), height


def flow_chart(block, font):
    """Boxes in layers, as flow.py lays them out, joined by curves with arrowheads; returns the SVG and its natural size."""
    nodes, edges = block["nodes"], block.get("edges", [])
    down = block.get("direction", "down") != "right"
    layer, back = flow.layers_of([n["id"] for n in nodes], edges)
    pad, s = font * 0.9, font / 16
    size = {}
    for n in nodes:
        w = text_width(n["label"], font) + pad * 2.4
        size[n["id"]] = (w + (font * 1.6 if n.get("kind") == "decision" else 0), font * 2.6)
    order = [[] for _ in range(max(layer.values()) + 1)]
    for n in nodes:
        order[layer[n["id"]]].append(n["id"])
    gap_main, gap_cross = font * (5.5 if not down else 3.2), font * 1.4
    main_of = lambda i: size[i][1] if down else size[i][0]
    cross_of = lambda i: size[i][0] if down else size[i][1]
    mains = [max(main_of(i) for i in ids) for ids in order]
    crosses = [sum(cross_of(i) for i in ids) + gap_cross * (len(ids) - 1) for ids in order]
    widest = max(crosses)
    margin = font * 1.2
    centre = {}
    position = margin
    for l, ids in enumerate(order):
        c = margin + (widest - crosses[l]) / 2
        for i in ids:
            mid_main, mid_cross = position + mains[l] / 2, c + cross_of(i) / 2
            centre[i] = (mid_cross, mid_main) if down else (mid_main, mid_cross)
            c += cross_of(i) + gap_cross
        position += mains[l] + gap_main
    total_main = position - gap_main + margin
    width, height = (widest + 2 * margin, total_main) if down else (total_main, widest + 2 * margin)
    defs = (f'<marker id="arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="{6:.1f}" markerHeight="{6:.1f}" orient="auto-start-reverse">'
            f'<path d="M0,0 L10,5 L0,10 z" fill="{AXIS}"/></marker>')
    body = [f'<rect width="{width:.0f}" height="{height:.0f}" fill="{BG}"/>']

    def anchor(i, outgoing):
        x, y = centre[i]
        w, h = size[i]
        if down:
            return x, y + (h / 2 if outgoing else -h / 2)
        return x + (w / 2 if outgoing else -w / 2), y

    for e in edges:
        a, b = e["from"], e["to"]
        is_back = (a, b) in back or a == b
        (x1, y1), (x2, y2) = anchor(a, not is_back), anchor(b, is_back)
        if down:
            bend = max(font * 1.5, abs(y2 - y1) / 2)
            path = f"M{x1:.1f},{y1:.1f} C{x1:.1f},{y1 + bend:.1f} {x2:.1f},{y2 - bend:.1f} {x2:.1f},{y2:.1f}"
        else:
            bend = max(font * 1.5, abs(x2 - x1) / 2)
            path = f"M{x1:.1f},{y1:.1f} C{x1 + bend:.1f},{y1:.1f} {x2 - bend:.1f},{y2:.1f} {x2:.1f},{y2:.1f}"
        dash = f' stroke-dasharray="{5 * s:.1f} {4 * s:.1f}"' if is_back else ""
        body.append(f'<path d="{path}" fill="none" stroke="{AXIS}" stroke-width="{1.8 * s:.2f}" marker-end="url(#arrow)"{dash}/>')
        if e.get("label"):
            mx, my = (x1 + x2) / 2, (y1 + y2) / 2
            w = text_width(e["label"], font * 0.85) + font * 0.8
            body.append(f'<rect x="{mx - w / 2:.1f}" y="{my - font * 0.7:.1f}" width="{w:.1f}" height="{font * 1.3:.1f}" rx="{font * 0.6:.1f}" fill="{BG}"/>'
                        f'<text x="{mx:.1f}" y="{my + font * 0.3:.1f}" font-size="{font * 0.85:.1f}" fill="{TEXT}" text-anchor="middle">{esc(e["label"])}</text>')
    for n in nodes:
        x, y = centre[n["id"]]
        w, h = size[n["id"]]
        kind = n.get("kind", "box")
        if kind == "decision":
            k = font * 0.8
            points = f"{x - w / 2:.1f},{y:.1f} {x - w / 2 + k:.1f},{y - h / 2:.1f} {x + w / 2 - k:.1f},{y - h / 2:.1f} {x + w / 2:.1f},{y:.1f} {x + w / 2 - k:.1f},{y + h / 2:.1f} {x - w / 2 + k:.1f},{y + h / 2:.1f}"
            body.append(f'<polygon points="{points}" fill="{PANEL}" stroke="{SERIES[1]}" stroke-width="{1.8 * s:.2f}"/>')
        else:
            radius = h / 2 if kind == "round" else 6 * s
            stroke = SERIES[0] if kind == "round" else "#3A3A40"
            body.append(f'<rect x="{x - w / 2:.1f}" y="{y - h / 2:.1f}" width="{w:.1f}" height="{h:.1f}" rx="{radius:.1f}" fill="{PANEL}" '
                        f'stroke="{stroke}" stroke-width="{1.8 * s:.2f}"/>')
        body.append(f'<text x="{x:.1f}" y="{y + font * 0.35:.1f}" font-size="{font:.1f}" fill="{FG}" text-anchor="middle">{esc(n["label"])}</text>')
    return svg(width, height, "".join(body), defs), width, height


def svg_size(text):
    """An SVG file's own width and height, from its width/height or its viewBox."""
    head = text[:2000]
    w, h = re.search(r'\swidth="([\d.]+)', head), re.search(r'\sheight="([\d.]+)', head)
    if w and h:
        return float(w[1]), float(h[1])
    box = re.search(r'viewBox="[\d.\s-]*?([\d.]+)\s+([\d.]+)"', head)
    return (float(box[1]), float(box[2])) if box else (300.0, 150.0)
