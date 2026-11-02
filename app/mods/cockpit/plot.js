// Charts for the show view, drawn natively: line and bar charts with a value axis, sparse x labels, a legend that hides
// a series, and a hovered point's crosshair and values. Lines are paths in percentages of the plot; bars are boxes.

import { div, PathBuilder, Background } from "gpui-kit";
import { h_flex, v_flex } from "gpui-base";
import { settings } from "herdr";
import { tokens, alpha, TEXT, RADIUS } from "./ui.js";

/** Series colours readable on light and dark themes. */
export const SERIES = ["#5b8def", "#3fb27f", "#e0a23c", "#d96c92", "#8f6fe0", "#41b3ba", "#e2704f"];

const trimmed = (t) => (t.includes(".") ? t.replace(/0+$/, "").replace(/\.$/, "") : t);

/** A number in five or six characters: 1234 → 1.2k, 3400000 → 3.4M. */
export function compact(v) {
  if (v === null || v === undefined || !Number.isFinite(v)) return "—";
  for (const [limit, suffix] of [[1e9, "B"], [1e6, "M"], [1e3, "k"]]) if (Math.abs(v) >= limit) return trimmed((v / limit).toFixed(1)) + suffix;
  return trimmed(v.toFixed(2));
}

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
 * A chart: model { kind: "line" | "bar", x, labels, series: [{name, values}] } over the rows of `window` ({start,
 * len}, all when null), `width` its room for x labels. `hover` is the hovered row (absolute), `hidden` a Set of hidden series names; `onHover(i, cx)`
 * and `onToggle(name, cx)` report the pointer.
 */
export function Chart(id, model, { height = 180, width = 480, hover = null, hidden = new Set(), window: win = null, onHover, onToggle } = {}) {
  const k = tokens();
  const z = settings().zoom ?? 1;
  const all = model.labels ?? [];
  const start = win?.start ?? 0, len = win?.len ?? all.length;
  const labels = all.slice(start, start + len);
  const series = (model.series ?? []).map((s, i) => ({ ...s, color: SERIES[i % SERIES.length], values: (s.values ?? []).slice(start, start + len) }));
  const shown = series.filter((s) => !hidden.has(s.name));
  const values = shown.flatMap((s) => s.values).filter((v) => Number.isFinite(v));
  if (!labels.length || !values.length) return div().h(48).flex().items_center().text_size(TEXT.sm).text_color(k.mutedFg).child("No data to draw.");
  const bars = model.kind === "bar";
  const [vmin, vmax] = [Math.min(...values), Math.max(...values)];
  // Bars start at zero; a line too, unless its values sit far from it.
  const yt = ticks(bars || (vmin >= 0 && vmin < vmax / 2) ? Math.min(0, vmin) : vmin, vmax);
  const [y0, y1] = [yt[0], yt[yt.length - 1]];
  const yOf = (v) => 100 - ((v - y0) / (y1 - y0 || 1)) * 100;
  const n = labels.length;
  const xOf = (i) => (bars ? ((i + 0.5) / n) * 100 : n === 1 ? 50 : (i / (n - 1)) * 100);
  const at = hover !== null && hover >= start && hover < start + len ? hover - start : null;

  let grid = PathBuilder.stroke(z);
  for (const t of yt) grid = grid.move_to("0%", `${yOf(t)}%`).line_to("100%", `${yOf(t)}%`);
  let plot = div().relative().flex_1().min_w(0).h(height).child(window.paint_path(grid.build(), alpha(k.border, 0.7)).absolute().inset_0());

  if (bars) {
    const groups = labels.map((label, i) =>
      h_flex().flex_1().min_w(0).h_full().items_end().justify_center().gap(2).px(n > 24 ? 1 : 4)
        .children(shown.map((s) => {
          const v = s.values[i];
          const hgt = Number.isFinite(v) ? Math.max(1, ((v - y0) / (y1 - y0 || 1)) * height) : 0;
          return div().flex_1().max_w(56).h(hgt).rounded_t(RADIUS.sm).bg(at === null || at === i ? s.color : alpha(s.color, 0.45));
        })),
    );
    plot = plot.child(h_flex().absolute().inset_0().children(groups));
  } else {
    for (const s of shown) {
      const pts = s.values.map((v, i) => [v, i]).filter(([v]) => Number.isFinite(v));
      if (pts.length > 1) {
        if (shown.length === 1) {
          let fill = PathBuilder.fill().move_to(`${xOf(pts[0][1])}%`, "100%");
          for (const [v, i] of pts) fill = fill.line_to(`${xOf(i)}%`, `${yOf(v)}%`);
          fill = fill.line_to(`${xOf(pts[pts.length - 1][1])}%`, "100%").close();
          plot = plot.child(window.paint_path(fill.build(), Background.solid(s.color).opacity(0.12)).absolute().inset_0());
        }
        let line = PathBuilder.stroke(2 * z);
        pts.forEach(([v, i], j) => (line = j ? line.line_to(`${xOf(i)}%`, `${yOf(v)}%`) : line.move_to(`${xOf(i)}%`, `${yOf(v)}%`)));
        plot = plot.child(window.paint_path(line.build(), s.color).absolute().inset_0());
      } else if (pts.length === 1) {
        plot = plot.child(div().absolute().left(`${xOf(pts[0][1])}%`).top(`${yOf(pts[0][0])}%`).ml(-3).mt(-3).size(6).rounded_full().bg(s.color));
      }
    }
    if (at !== null) {
      plot = plot.child(div().absolute().top(0).bottom(0).left(`${xOf(at)}%`).w(1).bg(alpha(k.foreground, 0.35)));
      for (const s of shown) {
        const v = s.values[at];
        if (!Number.isFinite(v)) continue;
        plot = plot.child(div().absolute().left(`${xOf(at)}%`).top(`${yOf(v)}%`).ml(-4).mt(-4).size(8).rounded_full().border_2().border_color(k.background).bg(s.color));
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
    const tip = v_flex().gap(4).p(8).min_w(140).rounded(RADIUS.md).border_1().border_color(k.border).bg(k.popover).shadow_md()
      .child(div().text_size(TEXT.xs).font_semibold().text_color(k.foreground).child(labels[at] ?? ""))
      .children(shown.map((s) =>
        h_flex().gap(6).items_center().text_size(TEXT.xs)
          .child(div().size(8).rounded(2).bg(s.color))
          .child(div().flex_1().text_color(k.mutedFg).child(s.name))
          .child(div().font_medium().text_color(k.foreground).child(compact(s.values[at])))));
    plot = plot.child(div().absolute().top(4).when(left, (e) => e.right(`${100 - xOf(at) + 2}%`)).when(!left, (e) => e.left(`${xOf(at) + 2}%`)).child(tip));
  }

  const axis = v_flex().w(44).flex_none().h(height).relative()
    .children(yt.map((t) => div().absolute().right(6).top(`${yOf(t)}%`).mt(-7).text_size(TEXT.xs).text_color(k.mutedFg).child(compact(t))));
  const room = Math.max(2, Math.min(8, n, Math.floor((width - 44) / 64)));
  const xlabels = div().relative().h(16).ml(44)
    .children(sparse(n, room).map((i) => {
      const x = xOf(i);
      const el = div().absolute().top(2).text_size(TEXT.xs).text_color(at === i ? k.foreground : k.mutedFg).whitespace_nowrap().child(labels[i] ?? "");
      return i === 0 && !bars ? el.left(0) : i === n - 1 && !bars ? el.right(0) : el.left(`${x}%`).ml(-16);
    }));
  const legend = series.length > 1
    ? h_flex().gap(4).flex_wrap().ml(44).children(series.map((s) => {
        const off = hidden.has(s.name);
        return h_flex().id(`${id}-l-${s.name}`).h(22).px(6).gap(6).items_center().rounded(RADIUS.sm).cursor_pointer().text_size(TEXT.xs)
          .text_color(off ? k.dim : k.mutedFg).hover((st) => st.bg(k.accent))
          .when(Boolean(onToggle), (e) => e.on_click((_e, cx) => onToggle(s.name, cx)))
          .tooltip(off ? `Show ${s.name}` : `Hide ${s.name}`)
          .child(div().size(8).rounded(2).bg(off ? alpha(s.color, 0.3) : s.color))
          .child(s.name);
      }))
    : null;
  return v_flex().id(id).gap(6).w_full()
    .child(h_flex().w_full().child(axis).child(plot))
    .child(xlabels)
    .when(Boolean(legend), (e) => e.child(legend));
}

/** The window after a zoom in (`closer`) or out around `centre`; null shows every row. */
export function zoomed(win, total, centre, closer) {
  const start = win?.start ?? 0, len = win?.len ?? total;
  const mid = centre ?? start + Math.floor(len / 2);
  const next = closer ? Math.max(Math.min(3, total), Math.floor(len / 2)) : len * 2;
  if (next >= total) return null;
  return { start: Math.max(0, Math.min(total - next, mid - Math.floor(next / 2))), len: next };
}
