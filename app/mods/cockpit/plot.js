// Charts for the show view, drawn natively: line, area and bar charts (stacked, to 100%, horizontal) with a value axis,
// sparse x labels, a legend that hides a series and a hovered row's values; donuts with their legend; figure tiles.

import { div, PathBuilder, Background } from "gpui-kit";
import { h_flex, v_flex } from "gpui-base";
import { settings } from "herdr";
import { tokens, alpha, TEXT, RADIUS, StatTile, Tiles } from "./ui.js";

/** Series colours readable on light and dark themes, in a fixed order. */
export const SERIES = ["#5b8def", "#3fb27f", "#e0a23c", "#d96c92", "#8f6fe0", "#41b3ba", "#e2704f"];

/** How many slices a donut draws before folding the rest into "Other". */
const SLICES = 6;

const trimmed = (t) => (t.includes(".") ? t.replace(/0+$/, "").replace(/\.$/, "") : t);

/** A number in five or six characters: 1234 → 1.2k, 3400000 → 3.4M. */
export function compact(v) {
  if (v === null || v === undefined || !Number.isFinite(v)) return "—";
  for (const [limit, suffix] of [[1e9, "B"], [1e6, "M"], [1e3, "k"]]) if (Math.abs(v) >= limit) return trimmed((v / limit).toFixed(1)) + suffix;
  return trimmed(v.toFixed(2));
}

const percent = (v) => (!Number.isFinite(v) ? "—" : v > 0 && v < 0.1 ? "<0.1%" : `${trimmed(v.toFixed(v < 10 ? 1 : 0))}%`);
const finite = (v) => typeof v === "number" && Number.isFinite(v);
/** 21234 → "21,234": the mod's runtime has no locale formatting. */
const grouped = (v) => { const [whole, part] = String(v).split("."); return whole.replace(/\B(?=(\d{3})+(?!\d))/g, ",") + (part ? `.${part}` : ""); };

/** Round ticks from lo to hi: 0, 250, 500, 750, 1000. */
function ticks(lo, hi, count = 4) {
  if (hi === lo) hi = lo + 1;
  const raw = (hi - lo) / count;
  const mag = 10 ** Math.floor(Math.log10(raw));
  const step = [1, 2, 2.5, 5, 10].map((m) => m * mag).find((s) => s >= raw) ?? raw;
  const start = Math.floor(lo / step) * step;
  const out = [];
  for (let v = start; v <= hi + step * 0.001; v += step) out.push(Number(v.toPrecision(12)));
  if (out[out.length - 1] < hi) out.push(out[out.length - 1] + step);
  return out;
}

/** Which labels to print under the x axis: at most `n`, evenly, always the first and last. */
function sparse(count, n) {
  if (count <= n) return [...Array(count).keys()];
  const out = new Set([0, count - 1]);
  for (let i = 1; i < n - 1; i++) out.add(Math.round((i * (count - 1)) / (n - 1)));
  return [...out].sort((a, b) => a - b);
}

/**
 * The rows of `win` and the series as drawn: coloured by their place in the model (a hidden one keeps its colour), the
 * values turned into shares of their row when `percent`, and `base` (where each shown value starts) when stacked.
 */
function prepared(model, win, hidden) {
  const all = model.labels ?? [];
  const start = win?.start ?? 0, len = win?.len ?? all.length;
  const labels = all.slice(start, start + len);
  const series = (model.series ?? []).map((s, i) => ({ ...s, color: SERIES[i % SERIES.length], raw: (s.values ?? []).slice(start, start + len) }));
  const shown = series.filter((s) => !hidden.has(s.name));
  const stacked = Boolean(model.stacked || model.percent) && model.kind !== "line";
  const totals = labels.map((_, i) => shown.reduce((sum, s) => sum + (finite(s.raw[i]) ? s.raw[i] : 0), 0));
  for (const s of series) s.values = model.percent ? s.raw.map((v, i) => (finite(v) && totals[i] ? (v / totals[i]) * 100 : null)) : s.raw;
  if (stacked) {
    const floor = labels.map(() => 0);
    for (const s of shown) {
      s.base = floor.slice();
      s.values.forEach((v, i) => finite(v) && (floor[i] += v));
    }
  }
  return { labels, series, shown, stacked, totals, start };
}

const empty = (k) => div().h(48).flex().items_center().text_size(TEXT.sm).text_color(k.mutedFg).child("No data to draw.");

/** The legend under a chart: one chip per series, a press hides or shows it. */
function Legend(id, series, hidden, onToggle, ml) {
  const k = tokens();
  return h_flex().gap(4).flex_wrap().ml(ml).children(series.map((s) => {
    const off = hidden.has(s.name);
    return h_flex().id(`${id}-l-${s.name}`).h(22).px(6).gap(6).items_center().rounded(RADIUS.sm).cursor_pointer().text_size(TEXT.xs)
      .text_color(off ? k.dim : k.mutedFg).hover((st) => st.bg(k.accent))
      .when(Boolean(onToggle), (e) => e.on_click((_e, cx) => onToggle(s.name, cx)))
      .tooltip(off ? `Show ${s.name}` : `Hide ${s.name}`)
      .child(div().size(8).rounded(2).bg(off ? alpha(s.color, 0.3) : s.color))
      .child(s.name);
  }));
}

/** The hovered row's card: its label, each shown series' value (and share), the total when stacked. */
function Tip(label, shown, at, { percent: pct, totals, stacked }) {
  const k = tokens();
  const line = (color, name, value) =>
    h_flex().gap(6).items_center().text_size(TEXT.xs)
      .child(div().size(8).rounded(2).when(Boolean(color), (e) => e.bg(color)))
      .child(div().flex_1().text_color(k.mutedFg).child(name))
      .child(div().font_medium().text_color(k.foreground).child(value));
  return v_flex().gap(4).p(8).min_w(150).rounded(RADIUS.md).border_1().border_color(k.border).bg(k.popover).shadow_md()
    .child(div().text_size(TEXT.xs).font_semibold().text_color(k.foreground).child(String(label ?? "")))
    .children(shown.map((s) => line(s.color, s.name, pct ? `${compact(s.raw[at])} · ${percent(s.values[at])}` : compact(s.values[at]))))
    .when(stacked && shown.length > 1, (e) => e.child(line(null, "Total", compact(totals[at]))));
}

/**
 * A chart: model { kind: "line" | "area" | "bar" | "donut", x, labels, series: [{name, values}], stacked, percent,
 * horizontal, show_values } over the rows of `window` ({start, len}, all when null), `width` its room. `hover` is the
 * hovered row (absolute), `hidden` a Set of hidden series names; `onHover(i, cx)` and `onToggle(name, cx)` report the
 * pointer.
 */
export function Chart(id, model, opts = {}) {
  if (model.kind === "donut") return Donut(id, model, opts);
  if (model.kind === "bar" && model.horizontal) return Rows(id, model, opts);
  return Plot(id, model, opts);
}

function Plot(id, model, { height = 180, width = 480, hover = null, hidden = new Set(), window: win = null, onHover, onToggle } = {}) {
  const k = tokens();
  const z = settings().zoom ?? 1;
  const { labels, series, shown, stacked, totals, start } = prepared(model, win, hidden);
  const top = (s, i) => (stacked ? s.base[i] + s.values[i] : s.values[i]);
  const values = shown.flatMap((s) => s.values.map((v, i) => (finite(v) ? top(s, i) : NaN))).filter(finite);
  if (!labels.length || !values.length) return empty(k);
  const bars = model.kind === "bar";
  const area = model.kind === "area";
  const [vmin, vmax] = [Math.min(...values), Math.max(...values)];
  // Bars, areas and stacks start at zero; a line too, unless its values sit far from it.
  const fromZero = bars || area || stacked || (vmin >= 0 && vmin < vmax / 2);
  const yt = model.percent ? [0, 25, 50, 75, 100] : ticks(fromZero ? Math.min(0, vmin) : vmin, vmax);
  const [y0, y1] = [yt[0], yt[yt.length - 1]];
  const yOf = (v) => 100 - ((v - y0) / (y1 - y0 || 1)) * 100;
  const n = labels.length;
  const xOf = (i) => (bars ? ((i + 0.5) / n) * 100 : n === 1 ? 50 : (i / (n - 1)) * 100);
  const at = hover !== null && hover >= start && hover < start + n ? hover - start : null;
  const tick = (t) => (model.percent ? `${t}%` : compact(t));
  // Value labels: over every bar or stack, on a line's points only when there are few.
  const labelled = Boolean(model.show_values) && (bars ? n <= 24 : n <= 12);
  const valueAt = (i, v, text) => div().absolute().left(`${xOf(i)}%`).top(`${yOf(v)}%`).mt(-16).ml(-28).w(56).text_center().text_size(TEXT.xs).font_medium().text_color(k.foreground).child(text);

  let grid = PathBuilder.stroke(z);
  for (const t of yt) grid = grid.move_to("0%", `${yOf(t)}%`).line_to("100%", `${yOf(t)}%`);
  let plot = div().relative().flex_1().min_w(0).h(height).child(window.paint_path(grid.build(), alpha(k.border, 0.7)).absolute().inset_0());

  if (bars) {
    const px = (v) => (finite(v) ? Math.max(1, (Math.abs(v) / (y1 - y0 || 1)) * height) : 0);
    const groups = labels.map((_, i) => {
      const lit = at === null || at === i;
      const paint = (s) => (lit ? s.color : alpha(s.color, 0.45));
      // A stack is one column, its first series at the bottom, 2px of surface between segments.
      const column = stacked
        ? [v_flex().flex_1().max_w(56).h_full().justify_end().gap(2).children(shown.slice().reverse().filter((s) => finite(s.values[i]) && s.values[i] > 0)
            .map((s, j) => div().w_full().flex_none().h(Math.max(1, px(s.values[i]) - 2)).when(j === 0, (e) => e.rounded_t(RADIUS.sm)).bg(paint(s))))]
        : shown.map((s) => div().flex_1().max_w(56).h(px(s.values[i])).rounded_t(RADIUS.sm).bg(paint(s)));
      return h_flex().flex_1().min_w(0).h_full().items_end().justify_center().gap(2).px(n > 24 ? 1 : 4).children(column);
    });
    plot = plot.child(h_flex().absolute().inset_0().children(groups));
    if (labelled) {
      for (let i = 0; i < n; i++) {
        if (stacked && !model.percent && totals[i]) plot = plot.child(valueAt(i, totals[i], compact(totals[i])));
        else if (!stacked && shown.length === 1 && finite(shown[0].values[i])) plot = plot.child(valueAt(i, shown[0].values[i], compact(shown[0].values[i])));
      }
    }
  } else {
    // Areas first, lines over them: a stacked area fills between its floor and its top, a plain one down to the axis.
    for (const s of shown) {
      const pts = s.values.map((v, i) => [v, i]).filter(([v]) => finite(v));
      if (pts.length < 2 || !(area || shown.length === 1)) continue;
      const lo = (i) => (stacked ? s.base[i] : y0);
      let fill = PathBuilder.fill().move_to(`${xOf(pts[0][1])}%`, `${yOf(lo(pts[0][1]))}%`);
      for (const [, i] of pts) fill = fill.line_to(`${xOf(i)}%`, `${yOf(top(s, i))}%`);
      for (const [, i] of pts.slice().reverse()) fill = fill.line_to(`${xOf(i)}%`, `${yOf(lo(i))}%`);
      const opacity = stacked ? 0.4 : area && shown.length > 1 ? 0.16 : 0.12;
      plot = plot.child(window.paint_path(fill.close().build(), Background.solid(s.color).opacity(opacity)).absolute().inset_0());
    }
    for (const s of shown) {
      const pts = s.values.map((v, i) => [v, i]).filter(([v]) => finite(v)).map(([, i]) => [top(s, i), i]);
      if (pts.length > 1) {
        let line = PathBuilder.stroke(2 * z);
        pts.forEach(([v, i], j) => (line = j ? line.line_to(`${xOf(i)}%`, `${yOf(v)}%`) : line.move_to(`${xOf(i)}%`, `${yOf(v)}%`)));
        plot = plot.child(window.paint_path(line.build(), s.color).absolute().inset_0());
      } else if (pts.length === 1) {
        plot = plot.child(div().absolute().left(`${xOf(pts[0][1])}%`).top(`${yOf(pts[0][0])}%`).ml(-3).mt(-3).size(6).rounded_full().bg(s.color));
      }
      if (labelled && shown.length === 1) for (const [v, i] of pts) plot = plot.child(valueAt(i, v, model.percent ? percent(v) : compact(s.raw[i])));
    }
    if (at !== null) {
      plot = plot.child(div().absolute().top(0).bottom(0).left(`${xOf(at)}%`).w(1).bg(alpha(k.foreground, 0.35)));
      for (const s of shown) {
        if (!finite(s.values[at])) continue;
        plot = plot.child(div().absolute().left(`${xOf(at)}%`).top(`${yOf(top(s, at))}%`).ml(-4).mt(-4).size(8).rounded_full().border_2().border_color(k.background).bg(s.color));
      }
    }
  }

  // The pointer: one invisible column per row (every few rows on a long series) reports which one it is over.
  if (onHover) {
    const every = Math.max(1, Math.ceil(n / 240));
    const cols = Math.ceil(n / every);
    plot = plot.child(
      h_flex().id(`${id}-hover`).absolute().inset_0().on_hover((on, cx) => !on && onHover(null, cx))
        .children([...Array(cols).keys()].map((c) => div().id(`${id}-c${c}`).flex_1().h_full().on_hover((on, cx) => on && onHover(start + Math.min(n - 1, c * every), cx)))),
    );
  }

  if (at !== null) {
    const left = xOf(at) > 60;
    const tip = Tip(labels[at], shown, at, { percent: model.percent, totals, stacked });
    plot = plot.child(div().absolute().top(4).when(left, (e) => e.right(`${100 - xOf(at) + 2}%`)).when(!left, (e) => e.left(`${xOf(at) + 2}%`)).child(tip));
  }

  const axis = v_flex().w(44).flex_none().h(height).relative()
    .children(yt.map((t) => div().absolute().right(6).top(`${yOf(t)}%`).mt(-7).text_size(TEXT.xs).text_color(k.mutedFg).child(tick(t))));
  const room = Math.max(2, Math.min(8, n, Math.floor((width - 44) / 64)));
  const xlabels = div().relative().h(16).ml(44)
    .children(sparse(n, room).map((i) => {
      const x = xOf(i);
      const el = div().absolute().top(2).text_size(TEXT.xs).text_color(at === i ? k.foreground : k.mutedFg).whitespace_nowrap().child(labels[i] ?? "");
      return i === 0 && !bars ? el.left(0) : i === n - 1 && !bars ? el.right(0) : el.left(`${x}%`).ml(-16);
    }));
  return v_flex().id(id).gap(6).w_full()
    .child(h_flex().w_full().child(axis).child(plot))
    .child(xlabels)
    .when(series.length > 1, (e) => e.child(Legend(id, series, hidden, onToggle, 44)));
}

/**
 * Horizontal bars, one row per label: its name, its bar (one per series, or one stack) and its value written after it,
 * so there is no axis. For rankings, shares and long category names.
 */
function Rows(id, model, { width = 480, hover = null, hidden = new Set(), window: win = null, onHover, onToggle } = {}) {
  const k = tokens();
  const { labels, series, shown, stacked, totals, start } = prepared(model, win, hidden);
  const n = labels.length;
  const end = (i) => (stacked ? (model.percent ? 100 : totals[i]) : Math.max(0, ...shown.map((s) => (finite(s.values[i]) ? s.values[i] : 0))));
  const max = Math.max(0, ...labels.map((_, i) => end(i)));
  if (!n || !max) return empty(k);
  const at = hover !== null && hover >= start && hover < start + n ? hover - start : null;
  const labelW = Math.min(Math.max(...labels.map((l) => String(l ?? "").length)) * 7 + 12, Math.max(80, Math.floor(width * 0.32)));
  const grouped = shown.length > 1 && !stacked;
  const thick = grouped ? Math.max(6, Math.floor(20 / shown.length)) : 18;
  // Bars scale to 78% of the row, leaving room for the value written after the longest.
  const share = (v) => `${(Math.max(v, max / 400) / max) * 78}%`;
  const sum = totals.reduce((a, b) => a + b, 0);
  const written = (i) => {
    // A row to 100% writes its first part's share, the one the stack is usually about.
    if (stacked) return model.percent ? percent(shown[0]?.values[i]) : compact(totals[i]);
    if (grouped) return "";
    const v = shown[0]?.values[i];
    return finite(v) ? `${compact(v)}${model.show_values && sum ? ` · ${percent((v / sum) * 100)}` : ""}` : "—";
  };
  const rows = labels.map((label, i) => {
    const paint = (s) => (at === null || at === i ? s.color : alpha(s.color, 0.45));
    const bar = stacked
      ? h_flex().flex_none().h(thick).w(share(end(i))).gap(2).rounded_r(RADIUS.sm).overflow_hidden()
          .children(shown.filter((s) => finite(s.values[i]) && s.values[i] > 0).map((s) => div().h_full().flex_grow(s.values[i]).bg(paint(s))))
      : grouped
        ? v_flex().flex_1().gap(2).children(shown.map((s) => div().h(thick).w(finite(s.values[i]) ? share(s.values[i]) : "0%").rounded_r(RADIUS.sm).bg(paint(s))))
        : div().flex_none().h(thick).w(finite(shown[0]?.values[i]) ? share(shown[0].values[i]) : "0%").rounded_r(RADIUS.sm).bg(paint(shown[0]));
    const text = written(i);
    let row = h_flex().id(`${id}-r${i}`).relative().w_full().min_h(grouped ? shown.length * (thick + 2) + 8 : thick + 10).py(2).px(4).gap(8).items_center().rounded(RADIUS.sm)
      .when(at === i, (e) => e.bg(alpha(k.foreground, 0.07)))
      .child(div().w(labelW).flex_none().truncate().text_right().text_size(TEXT.xs).text_color(at === i ? k.foreground : k.mutedFg).child(String(label ?? "")))
      .child(h_flex().flex_1().min_w(0).items_center().gap(8).child(bar)
        .when(Boolean(text), (e) => e.child(div().flex_none().whitespace_nowrap().text_size(TEXT.xs).font_medium().text_color(k.foreground).child(text))));
    if (onHover) row = row.on_hover((on, cx) => onHover(on ? start + i : null, cx));
    if (at === i && shown.length > 1) row = row.child(div().absolute().top(thick + 8).right(0).child(Tip(label, shown, i, { percent: model.percent, totals, stacked })));
    return row;
  });
  return v_flex().id(id).gap(2).w_full()
    .children(rows)
    .when(series.length > 1, (e) => e.child(Legend(id, series, hidden, onToggle, labelW + 12)));
}

/** A donut of the first series' values over the labels, past six slices the rest as "Other", with its legend. */
function Donut(id, model, { width = 480, hover = null, onHover } = {}) {
  const k = tokens();
  const z = settings().zoom ?? 1;
  const values = model.series?.[0]?.values ?? [];
  let slices = (model.labels ?? []).map((label, i) => ({ label, value: values[i] })).filter((s) => finite(s.value) && s.value > 0);
  if (slices.length > SLICES + 1) slices = [...slices.slice(0, SLICES), { label: "Other", value: slices.slice(SLICES).reduce((a, s) => a + s.value, 0), other: true }];
  slices = slices.map((s, i) => ({ ...s, color: s.other ? k.mutedFg : SERIES[i % SERIES.length] }));
  const total = slices.reduce((a, s) => a + s.value, 0);
  if (!total) return empty(k);
  const size = Math.min(180, Math.max(120, Math.floor(width * 0.4)));
  const stroke = Math.round(size * 0.16);
  const r = ((size - stroke - 4) / 2) * z;
  const c = (size / 2) * z;
  const at = hover !== null && hover < slices.length ? hover : null;
  const gap = slices.length > 1 ? 0.03 : 0;
  const point = (a) => [c + r * Math.sin(a), c - r * Math.cos(a)];
  let ring = div().relative().size(size).flex_none();
  let from = 0;
  for (const [i, s] of slices.entries()) {
    const sweep = (s.value / total) * 2 * Math.PI;
    const a0 = from + gap / 2;
    const a1 = slices.length === 1 ? a0 + 2 * Math.PI - 0.0001 : from + Math.max(gap / 2 + 0.002, sweep - gap / 2);
    from += sweep;
    const [x0, y0] = point(a0);
    const [x1, y1] = point(a1);
    const arc = PathBuilder.stroke((at === i ? stroke + 4 : stroke) * z).move_to(x0, y0).arc_to(r, r, 0, a1 - a0 > Math.PI, true, x1, y1);
    ring = ring.child(window.paint_path(arc.build(), at === null || at === i ? s.color : alpha(s.color, 0.35)).absolute().inset_0());
  }
  const focus = at !== null ? slices[at] : null;
  ring = ring.child(v_flex().absolute().inset_0().items_center().justify_center()
    .child(div().text_size(TEXT.lg).font_semibold().text_color(k.foreground).child(focus ? percent((focus.value / total) * 100) : compact(total)))
    .child(div().max_w(Math.max(40, size - stroke * 2 - 12)).truncate().text_size(TEXT.xs).text_color(k.mutedFg).child(focus ? String(focus.label) : String(model.series?.[0]?.name ?? "total"))));
  const legend = v_flex().flex_1().min_w(0).max_w(380).gap(2).children(slices.map((s, i) =>
    h_flex().id(`${id}-s${i}`).h(24).px(6).gap(8).items_center().rounded(RADIUS.sm).text_size(TEXT.xs)
      .when(at === i, (e) => e.bg(alpha(k.foreground, 0.07)))
      .when(Boolean(onHover), (e) => e.on_hover((on, cx) => onHover(on ? i : null, cx)))
      .child(div().size(8).flex_none().rounded(2).bg(s.color))
      .child(div().flex_1().min_w(0).truncate().text_color(at === i ? k.foreground : k.mutedFg).child(String(s.label)))
      .child(div().flex_none().font_medium().text_color(k.foreground).child(compact(s.value)))
      .child(div().w(44).flex_none().text_right().text_color(k.mutedFg).child(percent((s.value / total) * 100)))));
  return h_flex().id(id).w_full().gap(20).items_center().child(ring).child(legend);
}

/** Figure tiles: [{label, value, note, delta, good, spark}], two per row when `narrow`. */
export function Figures(id, items, { narrow = false } = {}) {
  const k = tokens();
  const tiles = (items ?? []).map((t, i) => StatTile(`${id}-${i}`, {
    label: String(t.label ?? ""),
    value: finite(t.value) ? grouped(t.value) : String(t.value ?? "—"),
    delta: t.delta === undefined || t.delta === null ? undefined : String(t.delta),
    deltaGood: typeof t.good === "boolean" ? t.good : undefined,
    hint: t.note ? String(t.note) : undefined,
    spark: Array.isArray(t.spark) && t.spark.filter(finite).length > 1 ? t.spark.filter(finite) : undefined,
    sparkColor: k.ring,
  }));
  return Tiles(tiles, { narrow });
}

/** The window after a zoom in (`closer`) or out around `centre`; null shows every row. */
export function zoomed(win, total, centre, closer) {
  const start = win?.start ?? 0, len = win?.len ?? total;
  const mid = centre ?? start + Math.floor(len / 2);
  const next = closer ? Math.max(Math.min(3, total), Math.floor(len / 2)) : len * 2;
  if (next >= total) return null;
  return { start: Math.max(0, Math.min(total - next, mid - Math.floor(next / 2))), len: next };
}
