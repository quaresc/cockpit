// Diagrams drawn natively for the show view: Mermaid flowcharts and flow blocks as a layered layout (nodes as boxes,
// edges as bezier paths with arrow heads), and sequence charts as lifelines and arrows. Paths are drawn in percentages
// of their box, so they follow the zoom; node text is estimated, never measured.
// The flowchart parser and layered layout are ported from https://github.com/chappse6/herdr-blueprint-app's `mermaid.js`
// and `layout.js` (MIT, Copyright (c) 2026 chappse6); see THIRD_PARTY_NOTICES.md.

import { div, PathBuilder } from "gpui-kit";
import { v_flex } from "gpui-base";
import { settings } from "herdr";
import { tokens, alpha, TEXT, RADIUS } from "./ui.js";

// ---------------------------------------------------------------------------------------------------------------------
// Mermaid flowchart subset → { title, dir, nodes: Map<id, node>, edges: [] }

const SHAPES = [
  ["([", "])", "stadium"],
  ["[(", ")]", "cylinder"],
  ["((", "))", "circle"],
  ["{{", "}}", "diamond"],
  ["[[", "]]", "rect"],
  ["[", "]", "rect"],
  ["(", ")", "round"],
  ["{", "}", "diamond"],
  [">", "]", "rect"],
];

const EDGES = [
  [/^\s*-->\s*(?:\|([^|]*)\|)?/, "solid", true],
  [/^\s*-\.->\s*(?:\|([^|]*)\|)?/, "dashed", true],
  [/^\s*==>\s*(?:\|([^|]*)\|)?/, "thick", true],
  [/^\s*---\s*(?:\|([^|]*)\|)?/, "solid", false],
  [/^\s*-\.-\s*(?:\|([^|]*)\|)?/, "dashed", false],
  [/^\s*===\s*(?:\|([^|]*)\|)?/, "thick", false],
  [/^\s*--[xo]\s*(?:\|([^|]*)\|)?/, "solid", true],
  [/^\s*--\s+([^>]+?)\s+-->/, "solid", true],
  [/^\s*==\s+([^>]+?)\s+==>/, "thick", true],
  [/^\s*-\.\s+([^>]+?)\s+\.->/, "dashed", true],
];

const unquote = (s) => {
  s = s.trim();
  return s.length > 1 && s.startsWith('"') && s.endsWith('"') ? s.slice(1, -1) : s;
};

// Entities written the HTML way (&lt;) or Mermaid's (#lt;), which agents use to keep < > " out of the syntax.
const NAMED = { lt: "<", gt: ">", amp: "&", quot: '"', apos: "'", nbsp: " " };
function decode(s) {
  if (!s || !/[&#]/.test(s)) return s;
  return s.replace(/&#x([0-9a-f]+);|[&#]#?(\d+);|[&#](lt|gt|amp|quot|apos|nbsp);/gi, (all, hex, dec, name) =>
    hex ? String.fromCodePoint(parseInt(hex, 16)) : dec ? String.fromCodePoint(Number(dec)) : NAMED[name.toLowerCase()] ?? all);
}

/** A Mermaid flowchart's nodes and edges; `class a,b done|active` (or `a:::done`) marks progress, as blueprints do. */
export function parseFlowchart(src) {
  const g = { title: null, dir: "TB", nodes: new Map(), edges: [] };
  const lines = String(src ?? "").split("\n").map((l) => l.trim());
  let i = Math.max(0, lines.findIndex(Boolean));
  if (lines[i] === "---") {
    for (i++; i < lines.length && lines[i] !== "---"; i++) {
      const m = lines[i].match(/^title:\s*(.+)$/);
      if (m) g.title = decode(unquote(m[1]));
    }
    i++;
  }
  for (; i < lines.length; i++) {
    const line = lines[i].replace(/;$/, "");
    let m;
    if (!line || line.startsWith("%%")) continue;
    if ((m = line.match(/^(?:flowchart|graph)(?:\s+(LR|RL|TB|TD|BT)\b.*|\s*)$/))) {
      g.dir = m[1] === "LR" || m[1] === "RL" ? "LR" : "TB";
      continue;
    }
    if ((m = line.match(/^class\s+([\w,\s]+?)\s+(\w+)$/))) {
      for (const id of m[1].split(",")) node(g, id.trim()).status = m[2];
      continue;
    }
    if (/^(classDef|style|linkStyle|subgraph|end|direction|click)\b/.test(line)) continue;
    statement(g, line);
  }
  return g;
}

function node(g, id) {
  let n = g.nodes.get(id);
  if (!n) {
    n = { id, shape: "rect", title: id, sub: null, status: null };
    g.nodes.set(id, n);
  }
  return n;
}

// A node's `:::name` class, which sets its status; the position after it.
function classed(n, s, pos) {
  const m = /^:::(\w+)/.exec(s.slice(pos));
  if (!m) return pos;
  n.status = m[1];
  return pos + m[0].length;
}

function readNode(g, s, pos) {
  const m = /^\s*([\w.-]+?)(?=[\s[({>]|:::|-->|---|==|-\.|$)/.exec(s.slice(pos)) ?? /^\s*(\w+)/.exec(s.slice(pos));
  if (!m) return null;
  const id = m[1];
  pos += m[0].length;
  const n = node(g, id);
  for (const [open, close, shape] of SHAPES) {
    if (!s.startsWith(open, pos)) continue;
    const q = /^\s*"/.exec(s.slice(pos + open.length));
    const shut = q ? s.indexOf('"', pos + open.length + q[0].length) : -1;
    const end = s.indexOf(close, shut >= 0 ? shut + 1 : pos + open.length);
    if (end < 0) break;
    const [title, ...rest] = unquote(s.slice(pos + open.length, end)).split(/<br\s*\/?>/i);
    Object.assign(n, { shape, title: decode(title.trim()), sub: decode(rest.join(" ").trim()) || null });
    return [id, classed(n, s, end + close.length)];
  }
  return [id, classed(n, s, pos)];
}

function statement(g, s) {
  let r = readNode(g, s, 0);
  while (r) {
    const [from, pos] = r;
    let hit = null;
    for (const [re, style, arrow] of EDGES) {
      const m = re.exec(s.slice(pos));
      if (m) {
        hit = { len: m[0].length, label: decode(unquote(m[1] ?? "")) || null, style, arrow };
        break;
      }
    }
    if (!hit) return;
    const next = readNode(g, s, pos + hit.len);
    if (!next) return;
    g.edges.push({ from, to: next[0], label: hit.label, style: hit.style, arrow: hit.arrow });
    r = next;
  }
}

/** A show `flow` block as the same graph: kinds box/round/decision become shapes. */
export function flowGraph(block) {
  const g = { title: null, dir: block.direction === "right" ? "LR" : "TB", nodes: new Map(), edges: [] };
  const shape = { round: "round", decision: "diamond" };
  for (const n of block.nodes ?? []) {
    const [title, ...rest] = String(n.label ?? n.id).split(/\n|<br\s*\/?>/i);
    g.nodes.set(n.id, { id: n.id, shape: shape[n.kind] ?? "rect", title, sub: rest.join(" ") || null, status: n.status ?? null });
  }
  for (const e of block.edges ?? []) g.edges.push({ from: e.from, to: e.to, label: e.label ?? null, style: e.style ?? "solid", arrow: e.arrow ?? true });
  return g;
}

// ---------------------------------------------------------------------------------------------------------------------
// Layered layout (Sugiyama-style), in graph units: ranks by longest path, order by barycenter sweeps, positions by
// weighted least squares, edges as cubic beziers through one dummy point per rank they cross.

const SEP = { LR: 28, TB: 36 };
const DUMMY_SEP = 14;
const GAP = { LR: 56, TB: 44 };

export function layout(g, dir, size, labelSize) {
  const LR = dir === "LR";
  const ids = [...g.nodes.keys()];
  const edges = g.edges.filter((e) => e.from !== e.to && g.nodes.has(e.from) && g.nodes.has(e.to));

  const state = new Map(), back = new Set();
  const visit = (v) => {
    state.set(v, 1);
    for (const e of edges) {
      if (e.from !== v) continue;
      const s = state.get(e.to);
      if (s === 1) back.add(e);
      else if (!s) visit(e.to);
    }
    state.set(v, 2);
  };
  const hasIn = new Set(edges.map((e) => e.to));
  for (const id of ids) if (!hasIn.has(id) && !state.get(id)) visit(id);
  for (const id of ids) if (!state.get(id)) visit(id);
  const forward = edges.filter((e) => !back.has(e));

  const rank = new Map(ids.map((id) => [id, 0]));
  const ranked = [...forward.map((e) => [e.from, e.to]), ...[...back].map((e) => [e.to, e.from])];
  for (let k = 0; k < ids.length; k++) for (const [u, v] of ranked) if (rank.get(v) < rank.get(u) + 1) rank.set(v, rank.get(u) + 1);
  // A box with more edges out than in moves down to just above what it leads to: it shortens more edges than it lengthens.
  const order = [...ids].sort((a, b) => rank.get(b) - rank.get(a));
  for (let pass = 0; pass < 4; pass++) {
    let moved = false;
    for (const id of order) {
      const outs = ranked.filter(([u]) => u === id).map(([, v]) => rank.get(v));
      const ins = ranked.filter(([, v]) => v === id).length;
      if (!outs.length || outs.length <= ins) continue;
      const to = Math.min(...outs) - 1;
      if (to > rank.get(id)) {
        rank.set(id, to);
        moved = true;
      }
    }
    if (!moved) break;
  }
  const top = Math.min(...rank.values());
  for (const id of ids) rank.set(id, rank.get(id) - top);
  const R = Math.max(0, ...rank.values()) + 1;

  const layers = Array.from({ length: R }, () => []);
  for (const id of ids) layers[rank.get(id)].push(id);
  const dummy = new Set();
  const up = new Map(ids.map((id) => [id, []])), down = new Map(ids.map((id) => [id, []]));
  const chains = forward.map((e, i) => {
    const chain = [e.from];
    for (let r = rank.get(e.from) + 1; r < rank.get(e.to); r++) {
      const id = `\u0000${i}:${r}`;
      dummy.add(id);
      rank.set(id, r);
      layers[r].push(id);
      up.set(id, []);
      down.set(id, []);
      chain.push(id);
    }
    chain.push(e.to);
    for (let k = 0; k + 1 < chain.length; k++) {
      down.get(chain[k]).push(chain[k + 1]);
      up.get(chain[k + 1]).push(chain[k]);
    }
    return chain;
  });

  const crossings = () => {
    let c = 0;
    for (let r = 0; r + 1 < R; r++) {
      const pos = new Map(layers[r + 1].map((id, i) => [id, i]));
      const segs = [];
      layers[r].forEach((a, i) => down.get(a).forEach((b) => segs.push([i, pos.get(b)])));
      for (let x = 0; x < segs.length; x++) for (let y = x + 1; y < segs.length; y++) if ((segs[x][0] - segs[y][0]) * (segs[x][1] - segs[y][1]) < 0) c++;
    }
    return c;
  };
  let best = layers.map((l) => [...l]), fewest = crossings();
  for (let it = 0; it < 16 && fewest > 0; it++) {
    const downward = it % 2 === 0;
    for (let k = 1; k < R; k++) {
      const r = downward ? k : R - 1 - k;
      const pos = new Map(layers[downward ? r - 1 : r + 1].map((id, i) => [id, i]));
      const bc = new Map(layers[r].map((id, i) => {
        const nb = (downward ? up : down).get(id);
        return [id, nb.length ? nb.reduce((s, n) => s + pos.get(n), 0) / nb.length : i];
      }));
      layers[r].sort((a, b) => bc.get(a) - bc.get(b));
    }
    const c = crossings();
    if (c < fewest) {
      best = layers.map((l) => [...l]);
      fewest = c;
    }
  }
  best.forEach((l, r) => (layers[r] = l));

  const sz = new Map(ids.map((id) => [id, size(g.nodes.get(id))]));
  const along = (id) => (dummy.has(id) ? 0 : LR ? sz.get(id).w : sz.get(id).h);
  const across = (id) => (dummy.has(id) ? 0 : LR ? sz.get(id).h : sz.get(id).w);
  const apart = (a, b) => (across(a) + across(b)) / 2 + (dummy.has(a) || dummy.has(b) ? DUMMY_SEP : SEP[dir]);

  const labels = new Map(edges.filter((e) => e.label).map((e) => [e, labelSize(e)]));
  const gapAfter = Array(R).fill(GAP[dir]);
  for (const e of forward) {
    const l = labels.get(e);
    if (l) gapAfter[rank.get(e.from)] = Math.max(gapAfter[rank.get(e.from)], (LR ? l.w : l.h) + (LR ? 44 : 40));
  }
  const mainPos = [];
  let acc = 0;
  for (let r = 0; r < R; r++) {
    const ext = Math.max(0, ...layers[r].map(along));
    mainPos.push(acc + ext / 2);
    acc += ext + gapAfter[r];
  }

  const cross = new Map();
  for (const layer of layers) {
    let x = 0;
    layer.forEach((id, i) => {
      if (i) x += apart(layer[i - 1], id);
      cross.set(id, x);
    });
    for (const id of layer) cross.set(id, cross.get(id) - x / 2);
  }
  // Each layer in turn goes where its edges are shortest while keeping its order and spacing: it only ever settles.
  const weight = (a, b) => (dummy.has(a) && dummy.has(b) ? 4 : dummy.has(a) || dummy.has(b) ? 2 : 1);
  for (let it = 0; it < 24; it++) {
    const downward = it % 2 === 0;
    for (let k = 0; k < R; k++) {
      const layer = layers[downward ? k : R - 1 - k];
      const want = [], w = [];
      for (const id of layer) {
        let sum = 0, total = 0;
        for (const n of [...up.get(id), ...down.get(id)]) {
          const wt = weight(id, n);
          sum += wt * cross.get(n);
          total += wt;
        }
        want.push(total ? sum / total : cross.get(id));
        w.push(total || 0.01);
      }
      const x = spaced(want, w, layer.map((id, i) => (i ? apart(layer[i - 1], id) : 0)));
      layer.forEach((id, i) => cross.set(id, x[i]));
    }
  }
  for (let round = 0; round < 2; round++) {
    for (let r = 1; r < R; r++) {
      const layer = layers[r];
      layer.forEach((b, i) => {
        if (up.get(b).length !== 1) return;
        const a = up.get(b)[0];
        if (down.get(a).length !== 1) return;
        let x = cross.get(a);
        if (i > 0) x = Math.max(x, cross.get(layer[i - 1]) + apart(layer[i - 1], b));
        if (i + 1 < layer.length) x = Math.min(x, cross.get(layer[i + 1]) - apart(b, layer[i + 1]));
        cross.set(b, x);
      });
    }
  }
  const P = new Map();
  for (const [id, r] of rank) P.set(id, LR ? { x: mainPos[r], y: cross.get(id) } : { x: cross.get(id), y: mainPos[r] });

  const box = (id) => ({ ...P.get(id), ...sz.get(id), shape: g.nodes.get(id).shape });
  const main = LR ? [1, 0] : [0, 1];
  const ins = new Map(), outs = new Map();
  forward.forEach((e, i) => {
    const ch = chains[i];
    if (!ins.has(e.to)) ins.set(e.to, []);
    if (!outs.has(e.from)) outs.set(e.from, []);
    ins.get(e.to).push({ i, at: P.get(ch[ch.length - 2]) });
    outs.get(e.from).push({ i, at: P.get(ch[1]) });
  });
  const spread = (map, id, i) => {
    const list = map.get(id) ?? [];
    if (list.length < 2 || g.nodes.get(id).shape === "diamond") return 0;
    const key = (p) => (LR ? p.at.y : p.at.x);
    list.sort((a, b) => key(a) - key(b));
    const step = Math.min(18, ((LR ? sz.get(id).h : sz.get(id).w) * 0.5) / (list.length - 1));
    return (list.findIndex((x) => x.i === i) - (list.length - 1) / 2) * step;
  };
  const shift = (p, d) => (LR ? { x: p.x, y: p.y + d } : { x: p.x + d, y: p.y });
  const out = [];
  forward.forEach((e, i) => {
    const pts = chains[i].map((id) => ({ ...P.get(id) }));
    const u = box(e.from), v = box(e.to);
    const start = exitPort(u, pts[1], LR), end = entryPort(v, LR);
    if (!(u.shape === "diamond" && (LR ? start.t[1] : start.t[0]))) start.p = shift(start.p, spread(outs, e.from, i));
    end.p = shift(end.p, spread(ins, e.to, i));
    pts[0] = start.p;
    pts[pts.length - 1] = end.p;
    const tan = pts.map((_, k) => (k === 0 ? start.t : k === pts.length - 1 ? end.t : main));
    const segs = [];
    for (let k = 0; k + 1 < pts.length; k++) segs.push(bezier(pts[k], pts[k + 1], tan[k], tan[k + 1]));
    const last = segs[segs.length - 1];
    out.push({ e, segs, label: e.label ? at(segs[0], 0.45) : null, dir: norm(sub(last[3], last[2])) });
  });

  let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
  const grow = (x, y, hw = 0, hh = 0) => {
    minX = Math.min(minX, x - hw); maxX = Math.max(maxX, x + hw);
    minY = Math.min(minY, y - hh); maxY = Math.max(maxY, y + hh);
  };
  for (const id of ids) grow(P.get(id).x, P.get(id).y, sz.get(id).w / 2 + 10, sz.get(id).h / 2 + 10);

  // Back edges: an arc outside everything they span.
  for (const e of back) {
    const a = box(e.from), b = box(e.to);
    const lo = Math.min(rank.get(e.from), rank.get(e.to)), hi = Math.max(rank.get(e.from), rank.get(e.to));
    let side = -Infinity;
    for (const id of ids) {
      if (rank.get(id) < lo || rank.get(id) > hi) continue;
      side = Math.max(side, LR ? P.get(id).y + sz.get(id).h / 2 : P.get(id).x + sz.get(id).w / 2);
    }
    side += 36;
    const p0 = LR ? { x: a.x, y: a.y + a.h / 2 } : { x: a.x + a.w / 2, y: a.y };
    const p3 = LR ? { x: b.x, y: b.y + b.h / 2 } : { x: b.x + b.w / 2, y: b.y };
    const seg = LR ? [p0, { x: p0.x, y: side }, { x: p3.x, y: side }, p3] : [p0, { x: side, y: p0.y }, { x: side, y: p3.y }, p3];
    out.push({ e, segs: [seg], label: e.label ? at(seg, 0.5) : null, dir: norm(sub(p3, seg[2])), back: true });
    grow(LR ? p0.x : side, LR ? side : p0.y, 4, 4);
  }
  // A label goes where it covers no other label and no box: the usual spot first, then further along its path.
  const taken = ids.map((id) => ({ x: P.get(id).x, y: P.get(id).y, w: sz.get(id).w, h: sz.get(id).h }));
  const clear = (c, l) => taken.every((b) => Math.abs(c.x - b.x) * 2 >= l.w + b.w + 4 || Math.abs(c.y - b.y) * 2 >= l.h + b.h + 4);
  for (const ed of out) {
    const l = ed.label && labels.get(ed.e);
    if (!l) continue;
    const spots = [ed.label];
    for (const sg of ed.segs) for (const t of [0.45, 0.25, 0.65, 0.85]) spots.push(at(sg, t));
    ed.label = spots.find((c) => clear(c, l)) ?? ed.label;
    ed.labelSize = l;
    taken.push({ ...ed.label, w: l.w, h: l.h });
    grow(ed.label.x, ed.label.y, l.w / 2, l.h / 2);
  }
  if (!ids.length) minX = minY = maxX = maxY = 0;
  return { nodes: ids.map((id) => ({ node: g.nodes.get(id), ...P.get(id), ...sz.get(id) })), edges: out, bounds: { minX, minY, maxX, maxY } };
}

// Positions closest to `want` (weighted least squares) that keep the order with at least gaps[i] between neighbours.
function spaced(want, w, gaps) {
  const off = [];
  gaps.forEach((g, i) => off.push((i ? off[i - 1] : 0) + g));
  const blocks = [];
  want.forEach((v, i) => {
    blocks.push({ v: v - off[i], w: w[i], n: 1 });
    while (blocks.length > 1 && blocks[blocks.length - 2].v > blocks[blocks.length - 1].v) {
      const b2 = blocks.pop(), b1 = blocks.pop();
      blocks.push({ v: (b1.v * b1.w + b2.v * b2.w) / (b1.w + b2.w), w: b1.w + b2.w, n: b1.n + b2.n });
    }
  });
  const out = [];
  for (const b of blocks) for (let k = 0; k < b.n; k++) out.push(b.v + off[out.length]);
  return out;
}

function exitPort(u, next, LR) {
  if (LR) {
    if (u.shape === "diamond" && Math.abs(next.y - u.y) > u.h / 4) {
      const s = Math.sign(next.y - u.y);
      return { p: { x: u.x, y: u.y + (s * u.h) / 2 }, t: [0, s] };
    }
    return { p: { x: u.x + u.w / 2, y: u.y }, t: [1, 0] };
  }
  if (u.shape === "diamond" && Math.abs(next.x - u.x) > u.w / 4) {
    const s = Math.sign(next.x - u.x);
    return { p: { x: u.x + (s * u.w) / 2, y: u.y }, t: [s, 0] };
  }
  return { p: { x: u.x, y: u.y + u.h / 2 }, t: [0, 1] };
}

function entryPort(v, LR) {
  return LR ? { p: { x: v.x - v.w / 2, y: v.y }, t: [1, 0] } : { p: { x: v.x, y: v.y - v.h / 2 }, t: [0, 1] };
}

function bezier(p0, p1, t0, t1) {
  const d = sub(p1, p0);
  const k0 = Math.max(16, Math.abs(d.x * t0[0] + d.y * t0[1]) * 0.5);
  const k1 = Math.max(16, Math.abs(d.x * t1[0] + d.y * t1[1]) * 0.5);
  return [p0, { x: p0.x + t0[0] * k0, y: p0.y + t0[1] * k0 }, { x: p1.x - t1[0] * k1, y: p1.y - t1[1] * k1 }, p1];
}

function at([a, b, c, d], t) {
  const s = 1 - t;
  return {
    x: s * s * s * a.x + 3 * s * s * t * b.x + 3 * s * t * t * c.x + t * t * t * d.x,
    y: s * s * s * a.y + 3 * s * s * t * b.y + 3 * s * t * t * c.y + t * t * t * d.y,
  };
}

const sub = (a, b) => ({ x: a.x - b.x, y: a.y - b.y });
const norm = (v) => {
  const l = Math.hypot(v.x, v.y) || 1;
  return { x: v.x / l, y: v.y / l };
};

// ---------------------------------------------------------------------------------------------------------------------
// Drawing

const TITLE = 12.5;
const SUB = 11;
const LABEL = 11;
const wide = (ch) => ch.codePointAt(0) > 0x2e80;
const estimate = (s, px) => [...String(s ?? "")].reduce((w, ch) => w + px * (wide(ch) ? 1.05 : 0.58), 0);

function nodeSize(n) {
  const m = { w: Math.max(estimate(n.title, TITLE), n.sub ? estimate(n.sub, SUB) : 0), h: n.sub ? 32 : 16 };
  switch (n.shape) {
    case "diamond": return { w: Math.max(2 * m.w * 0.75 + 28, 96), h: Math.max(2 * m.h + 22, 64) };
    case "circle": { const d = Math.max(m.w, m.h) + 32; return { w: d, h: d }; }
    case "cylinder": return { w: Math.max(m.w + 40, 96), h: m.h + 38 };
    case "stadium": return { w: Math.max(m.w + 44, 96), h: Math.max(m.h + 22, 44) };
    default: return { w: Math.max(m.w + 32, 96), h: Math.max(m.h + 22, 44) };
  }
}
const labelSize = (e) => ({ w: estimate(e.label, LABEL) + 12, h: 18 });

/** A graph's layout, kept per graph text: laying out is the expensive part, drawing is cheap. */
const laid = new Map();
export function laidOut(key, g) {
  const hit = laid.get(key);
  if (hit) return hit;
  const L = layout(g, g.dir, nodeSize, labelSize);
  if (laid.size > 40) laid.delete(laid.keys().next().value);
  laid.set(key, L);
  return L;
}

const PAD = 12;

/**
 * A laid out graph drawn `scale` times its size: edges as one stroke path per look and one arrow path per colour, nodes
 * as boxes (rect, round, stadium, circle, diamond, cylinder), a check on done nodes and a dot on active ones.
 * `hovered`/`onHover`/`onClick` light a node and its edges.
 */
export function Graph(id, L, { scale = 1, hovered = null, onHover, onClick } = {}) {
  const k = tokens();
  const z = settings().zoom ?? 1;
  const { minX, minY, maxX, maxY } = L.bounds;
  const gw = maxX - minX + 2 * PAD, gh = maxY - minY + 2 * PAD;
  const W = gw * scale, H = gh * scale;
  const X = (x) => (x - minX + PAD) * scale, Y = (y) => (y - minY + PAD) * scale;
  const px = (x) => `${(X(x) / W) * 100}%`, py = (y) => `${(Y(y) / H) * 100}%`;
  const lit = (e) => !hovered || e.from === hovered || e.to === hovered;
  const active = k.working;

  const groups = new Map();
  const group = (key, color, width, dash) => {
    if (!groups.has(key)) {
      let b = PathBuilder.stroke(width * z);
      if (dash) b = b.dash_array(dash.map((d) => d * z));
      groups.set(key, { b, color, fill: false });
    }
    return groups.get(key);
  };
  const heads = new Map();
  const tips = new Set();
  const head = 7 * Math.sqrt(scale);
  for (const ed of L.edges) {
    const on = lit(ed.e);
    const dashed = ed.e.style === "dashed";
    const color = hovered ? (on ? k.foreground : alpha(k.mutedFg, 0.25)) : alpha(k.mutedFg, 0.85);
    const width = (ed.e.style === "thick" ? 2.4 : on && hovered ? 1.8 : 1.3) * Math.max(0.75, Math.sqrt(scale));
    const gr = group(`${color}:${dashed}:${width}`, color, width, dashed ? [4, 4] : null);
    const pts = ed.segs.map((sg) => sg.map((p) => ({ x: X(p.x), y: Y(p.y) })));
    const end = pts[pts.length - 1][3], d = ed.dir;
    const stop = ed.e.arrow ? { x: end.x - d.x * head * 0.8, y: end.y - d.y * head * 0.8 } : end;
    const P = (p) => [`${(p.x / W) * 100}%`, `${(p.y / H) * 100}%`];
    gr.b = gr.b.move_to(...P(pts[0][0]));
    pts.forEach((p, i) => {
      const to = i === pts.length - 1 ? stop : p[3];
      gr.b = gr.b.cubic_bezier_to(...P(to), ...P(p[1]), ...P(p[2]));
    });
    const tip = `${Math.round(end.x)}:${Math.round(end.y)}`;
    if (ed.e.arrow && !tips.has(tip)) {
      tips.add(tip);
      const side = { x: -d.y, y: d.x }, w = head * 0.55;
      if (!heads.has(color)) heads.set(color, PathBuilder.fill());
      heads.set(color, heads.get(color).add_polygon([P(end), P({ x: end.x - d.x * head + side.x * w, y: end.y - d.y * head + side.y * w }), P({ x: end.x - d.x * head - side.x * w, y: end.y - d.y * head - side.y * w })], true));
    }
  }
  const layer = (path, color) => window.paint_path(path, color).absolute().inset_0();
  let canvas = div().id(id).relative().w(W).h(H).flex_none();
  for (const gr of groups.values()) canvas = canvas.child(layer(gr.b.build(), gr.color));
  for (const [color, b] of heads) canvas = canvas.child(layer(b.build(), color));

  for (const ed of L.edges) {
    if (!ed.label || !ed.labelSize) continue;
    const lw = ed.labelSize.w * scale, lh = ed.labelSize.h * scale;
    canvas = canvas.child(
      div().absolute().left(X(ed.label.x) - lw / 2).top(Y(ed.label.y) - lh / 2).w(lw).h(lh).flex().items_center().justify_center()
        .rounded(4).bg(k.background).text_size(LABEL * scale).text_color(lit(ed.e) ? k.mutedFg : alpha(k.mutedFg, 0.35)).child(ed.e.label),
    );
  }
  for (const item of L.nodes) canvas = canvas.child(nodeBox(id, item, k, scale, X, Y, z, hovered, onHover, onClick, active));
  return canvas;
}

function nodeBox(id, item, k, scale, X, Y, z, hovered, onHover, onClick, active) {
  const n = item.node, w = item.w * scale, h = item.h * scale;
  const left = X(item.x) - w / 2, top = Y(item.y) - h / 2;
  const on = hovered === n.id;
  const status = n.status === "active" || n.status === "current" ? "active" : n.status === "done" ? "done" : null;
  const stroke = on ? k.ring : status === "active" ? active : status === "done" ? alpha(k.done, 0.8) : k.input;
  const bw = on || status === "active" ? 2 : 1;
  const bg = on ? k.muted : k.card;
  const label = v_flex().items_center().gap(2 * scale).px(6 * scale)
    .child(div().whitespace_nowrap().text_size(TITLE * scale).font_semibold().text_color(k.foreground).child(n.title))
    .when(Boolean(n.sub), (el) => el.child(div().whitespace_nowrap().text_size(SUB * scale).text_color(k.mutedFg).child(n.sub ?? "")));
  let box = div().id(`${id}-n-${n.id}`).absolute().left(left).top(top).w(w).h(h).flex().items_center().justify_center()
    .when(Boolean(onHover), (el) => el.on_hover((hov, cx) => onHover(hov ? n.id : null, cx)))
    .when(Boolean(onClick), (el) => el.cursor_pointer().on_click((_e, cx) => onClick(n, cx)));
  const full = (path, color) => window.paint_path(path, color).absolute().inset_0();
  const pct = (x, y) => [`${(x / w) * 100}%`, `${(y / h) * 100}%`];
  const inset = (bw / 2) * z;
  if (n.shape === "diamond") {
    const poly = [pct(w / 2, inset / z), pct(w - inset / z, h / 2), pct(w / 2, h - inset / z), pct(inset / z, h / 2)];
    box = box.child(full(PathBuilder.fill().add_polygon(poly, true).build(), bg)).child(full(PathBuilder.stroke(bw * z).add_polygon(poly, true).build(), stroke));
  } else if (n.shape === "cylinder") {
    const ry = 6 * scale;
    box = box.bg(bg).border(bw).border_color(stroke).rounded(10 * scale).child(div().absolute().left(0).right(0).top(ry).h(1).bg(stroke));
  } else {
    const r = n.shape === "stadium" ? h / 2 : n.shape === "circle" ? w / 2 : n.shape === "round" ? 12 * scale : RADIUS.lg * scale;
    box = box.bg(bg).border(bw).border_color(stroke).rounded(r).when(on, (el) => el.shadow_md());
  }
  box = box.child(label);
  if (!status) return box;
  const d = Math.max(10, 15 * scale);
  // A diamond's corner is empty: its mark sits on the upper right edge instead.
  const [mr, mt] = n.shape === "diamond" ? [w * 0.25 - d / 2, h * 0.25 - d / 2] : [-d / 3, -d / 3];
  const mark = status === "done"
    ? div().absolute().right(mr).top(mt).size(d).rounded_full().bg(k.done).flex().items_center().justify_center().text_size(d * 0.62).font_semibold().text_color(k.background).child("✓")
    : div().id(`${id}-a-${n.id}`).absolute().right(mr).top(mt).size(d * 0.7).rounded_full().bg(active).loop("opacity", { from: 0.45, to: 1, period: 1600 });
  return box.child(mark);
}

/** The natural size of a laid out graph, for fitting it to a width. */
export function graphSize(L) {
  const { minX, minY, maxX, maxY } = L.bounds;
  return { w: maxX - minX + 2 * PAD, h: maxY - minY + 2 * PAD };
}

// ---------------------------------------------------------------------------------------------------------------------
// Sequence charts: actors on top, a dashed lifeline under each, messages as arrows (replies dashed), notes as boxes.

const ROW = 34;
const HEAD = 34;

export function sequenceSize(block) {
  const actors = block.actors ?? [];
  const col = (id) => actors.findIndex((a) => a.id === id);
  const boxW = (a) => Math.max(estimate(a.label ?? a.id, TEXT.base) + 24, 80);
  // The room between neighbours: their boxes, and the longest label of a message spanning them, shared over its span.
  const gaps = actors.slice(1).map((a, i) => Math.max(120, (boxW(actors[i]) + boxW(a)) / 2 + 24));
  let selfRoom = 0;
  for (const m of block.messages ?? []) {
    if (m.note !== undefined) continue;
    const [a, b] = [col(m.from), col(m.to)].sort((x, y) => x - y);
    if (a < 0 || b < 0) continue;
    const need = estimate(m.label, TEXT.sm) + 28;
    if (a === b) {
      if (a < gaps.length) gaps[a] = Math.max(gaps[a], need + 40);
      else selfRoom = Math.max(selfRoom, need + 40);
      continue;
    }
    for (let i = a; i < b; i++) gaps[i] = Math.max(gaps[i], need / (b - a));
  }
  const xs = [];
  let x = actors.length ? boxW(actors[0]) / 2 + 4 : 0;
  actors.forEach((_, i) => {
    xs.push(x);
    x += gaps[i] ?? 0;
  });
  const last = actors.length ? xs[xs.length - 1] + Math.max(boxW(actors[actors.length - 1]) / 2, selfRoom) + 4 : 0;
  return { xs, w: Math.max(last, 120), h: HEAD * 2 + (block.messages?.length ?? 0) * ROW + 24 };
}

export function Sequence(id, block, { scale = 1 } = {}) {
  const k = tokens();
  const z = settings().zoom ?? 1;
  const actors = block.actors ?? [];
  const { xs, w, h } = sequenceSize(block);
  const W = w * scale, H = h * scale;
  const col = (who) => actors.findIndex((a) => a.id === who);
  const P = (x, y) => [`${((x * scale) / W) * 100}%`, `${((y * scale) / H) * 100}%`];
  let lifelines = PathBuilder.stroke(z).dash_array([3 * z, 4 * z]);
  xs.forEach((x) => (lifelines = lifelines.move_to(...P(x, HEAD)).line_to(...P(x, h - HEAD))));
  let solid = PathBuilder.stroke(1.4 * z), dashed = PathBuilder.stroke(1.4 * z).dash_array([5 * z, 4 * z]), arrows = PathBuilder.fill();
  const texts = [];
  const head = 7;
  (block.messages ?? []).forEach((m, i) => {
    const y = HEAD + 22 + i * ROW + 12;
    if (m.note !== undefined) {
      const c = col(m.over);
      if (c < 0) return;
      const nw = estimate(m.note, TEXT.sm) + 20;
      texts.push(div().absolute().left((xs[c] - nw / 2) * scale).top((y - 13) * scale).w(nw * scale).h(24 * scale).flex().items_center().justify_center()
        .rounded(RADIUS.md).bg(alpha(k.warning, 0.15)).border_1().border_color(alpha(k.warning, 0.4)).text_size(TEXT.sm * scale).text_color(k.foreground).child(m.note));
      return;
    }
    const a = col(m.from), b = col(m.to);
    if (a < 0 || b < 0) return;
    const path = (b2) => (m.reply ? (dashed = b2(dashed)) : (solid = b2(solid)));
    if (a === b) {
      const x = xs[a];
      path((p) => p.move_to(...P(x, y - 6)).line_to(...P(x + 28, y - 6)).line_to(...P(x + 28, y + 6)).line_to(...P(x + head, y + 6)));
      arrows = arrows.add_polygon([P(x, y + 6), P(x + head, y + 6 - head / 2), P(x + head, y + 6 + head / 2)], true);
      texts.push(div().absolute().left((x + 34) * scale).top((y - 14) * scale).text_size(TEXT.sm * scale).text_color(k.foreground).whitespace_nowrap().child(m.label ?? ""));
      return;
    }
    const [x0, x1] = [xs[a], xs[b]];
    const s = Math.sign(x1 - x0);
    path((p) => p.move_to(...P(x0, y)).line_to(...P(x1 - s * head, y)));
    arrows = arrows.add_polygon([P(x1, y), P(x1 - s * head, y - head / 2), P(x1 - s * head, y + head / 2)], true);
    const mid = (x0 + x1) / 2, lw = estimate(m.label, TEXT.sm) + 8;
    texts.push(div().absolute().left((mid - lw / 2) * scale).top((y - 19) * scale).w(lw * scale).flex().justify_center().text_size(TEXT.sm * scale)
      .text_color(m.reply ? k.mutedFg : k.foreground).whitespace_nowrap().child(m.label ?? ""));
  });
  const layer = (p, color) => window.paint_path(p.build(), color).absolute().inset_0();
  const actor = (a, i, y) => {
    const aw = Math.max(estimate(a.label, TEXT.base) + 24, 80);
    return div().absolute().left((xs[i] - aw / 2) * scale).top(y * scale).w(aw * scale).h(28 * scale).flex().items_center().justify_center()
      .rounded(RADIUS.md).border_1().border_color(k.input).bg(k.muted).text_size(TEXT.base * scale).font_medium().text_color(k.foreground).child(a.label ?? a.id);
  };
  return div().id(id).relative().w(W).h(H).flex_none()
    .child(layer(lifelines, alpha(k.mutedFg, 0.45)))
    .child(layer(solid, k.foreground))
    .child(layer(dashed, k.mutedFg))
    .child(layer(arrows, k.foreground))
    .children(texts)
    .children(actors.map((a, i) => actor(a, i, 2)))
    .children(actors.map((a, i) => actor(a, i, h - HEAD + 4)));
}

/** A box `inner` wide drawn into a pane `room` wide: the scale that fits it, never above 1 nor under `min`. */
export const fit = (inner, room, min = 0.5) => Math.max(min, Math.min(1, room / Math.max(1, inner)));
