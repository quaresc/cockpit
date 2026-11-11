// The drawer on the window's left edge: a plain bar until the pointer slides it out, then the agents alone — whose
// move it is, what each is called, the ticket it is on and how long it has waited. The pinned sidebar is where the
// quota bars, the apps and the spaces live; this is the short version of it. It follows the agents only while that
// sidebar is away, so one of the two is ever subscribed.

import { div } from "gpui-kit";
import { h_flex, v_flex } from "gpui-base";
import { shared, focused } from "herdr";
import * as hub from "./hub.js";
import { DOCKED, showSidebar, show } from "./screens.js";
import { rowsOf, goToAgent } from "./fleet.js";
import { statusMark } from "./chat-top.js";
import { tokens, TEXT, RADIUS, Icon, CountPill, dur } from "./ui.js";

const WIDTH = 222;
/** A drawer sizes to what it holds; past this it scrolls instead of running off the window. */
export const MAX_H = 620;
/** What peeks at the edge: a bar of colour, no content. */
export const BAR = 10;
/** The hub's stages, in the order they are listed. */
const STAGES = [
  { id: "needs_you", label: "Needs you", tone: "blocked" },
  { id: "fixing", label: "To fix", tone: "blocked" },
  { id: "working", label: "Working", tone: "working" },
  { id: "review", label: "In review", tone: "done" },
  { id: "done", label: "Done", tone: "done" },
  { id: "merged", label: "Merged", tone: "done" },
  { id: "idle", label: "Idle", tone: "idle" },
  { id: "snoozed", label: "Snoozed", tone: "idle" },
];

/** The bar on the edge side of a drawer. */
export function peekBar(k) {
  return div().w(BAR).h_full().flex_none().bg(k.selected);
}

/** The head both drawers wear: what it is on the left, a count, then the button that pins it open. */
export function peekHead(k, label, count, { tooltip, onPin }) {
  const pin = div()
    .id(`peek-pin-${label}`)
    .size(24)
    .flex()
    .items_center()
    .justify_center()
    .rounded(RADIUS.md)
    .cursor_pointer()
    .text_color(k.dim)
    .hover((s) => s.bg(k.accent).text_color(k.foreground))
    .tooltip(tooltip)
    .on_click((_e, cx) => {
      onPin();
      cx.notify();
    })
    .child(Icon("panel-right", { size: 13 }));
  return h_flex()
    .h(38)
    .px(10)
    .gap(6)
    .flex_none()
    .items_center()
    .child(div().flex_1().min_w(0).truncate().text_size(11.5).font_semibold().text_color(k.mutedFg).child(label))
    .when(count !== null && count !== undefined, (e) => e.child(CountPill(count)))
    .child(pin);
}

export default class SidePeek {
  init(_props, cx) {
    this.rows = null;
    cx.spawn(async (cx) => {
      for (;;) {
        this.listen(cx);
        await hub.sleep(cx, 500);
      }
    });
  }

  close() {
    this.sub?.stop();
  }

  /** Follows the agents only while the pinned sidebar is away, so the two never subscribe at once. */
  listen(cx) {
    const wanted = shared(DOCKED) === false;
    if (wanted === Boolean(this.sub)) return;
    if (!wanted) {
      this.sub?.stop();
      this.sub = null;
      this.rows = null;
      return;
    }
    this.sub = hub.subscribe(cx, { views: [{ name: "agents", args: { frontend: "app" } }] }, (m, cx) => {
      if (m.view === "agents") this.rows = m.value;
      cx.notify();
    });
  }

  row(a, here, k) {
    const meta = [a.ticket?.key, a.branch].filter(Boolean)[0] ?? null;
    return h_flex()
      .id(`peek-${a.terminal}`)
      .min_h(meta ? 34 : 28)
      .px(8)
      .gap(8)
      .items_center()
      .rounded(RADIUS.md)
      .cursor_pointer()
      .text_size(TEXT.sm)
      .when(a.address === here, (e) => e.bg(k.selected))
      .when(a.address !== here, (e) => e.hover((s) => s.bg(k.accent)))
      .on_click((_e, cx) => {
        goToAgent({ address: a.address });
        show("chat");
        cx.notify();
      })
      .child(div().w(14).flex_none().flex().justify_center().child(statusMark(a.status, k)))
      .child(
        v_flex()
          .flex_1()
          .min_w(0)
          .gap(1)
          .child(div().w_full().truncate().text_color(k.foreground).child(a.title ?? a.terminal))
          .when(Boolean(meta), (e) => e.child(div().w_full().truncate().text_size(TEXT.xs).text_color(k.dim).child(meta))),
      )
      .when(Boolean(a.since) && a.stage === "needs_you", (e) =>
        e.child(div().flex_none().text_size(TEXT.xs).text_color(k.blocked).child(dur(Date.now() - a.since))),
      );
  }

  render(_cx) {
    const k = tokens();
    // Herdr hides a floating block that draws nothing: the drawer stands down while the sidebar is pinned.
    if (shared(DOCKED) !== false) return div();
    const rows = rowsOf(this.rows);
    const f = focused();
    const here = f?.agent ?? f?.terminal ?? null;
    const groups = STAGES.map((s) => [s, rows.filter((r) => r.stage === s.id)]).filter(([, list]) => list.length > 0);
    const body = v_flex()
      .w(WIDTH)
      .max_h(MAX_H)
      .overflow_y_scrollbar()
      .px(5)
      .pb(6)
      .gap(1)
      .text_color(k.foreground)
      .child(peekHead(k, "Agents", rows.length, { tooltip: "Keep the sidebar open  ⌘B", onPin: () => showSidebar(true) }))
      .when(rows.length === 0, (e) => e.child(div().px(8).py(6).text_size(TEXT.sm).text_color(k.dim).child("No agent yet.")))
      .child(
        v_flex()
          .w_full()
          .gap(1)
          .children(
        groups.flatMap(([s, list]) => [
          h_flex()
            .px(7)
            .pt(7)
            .pb(2)
            .gap(6)
            .items_center()
            .child(div().size(6).flex_none().rounded(3).bg(k[s.tone] ?? k.dim))
            .child(div().flex_1().text_size(11.5).font_medium().text_color(k.mutedFg).child(s.label))
            .child(div().text_size(TEXT.xs).text_color(k.dim).child(String(list.length))),
          ...list.map((a) => this.row(a, here, k)),
        ]),
          ),
      );
    return h_flex().id("side-peek").child(peekBar(k)).child(body);
  }
}
