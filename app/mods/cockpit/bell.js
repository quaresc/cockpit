// The bell at the top right of the cockpit layout: how many agents wait for you, breathing while any does. A click
// goes to the one that has waited longest; resting the pointer lists them with their questions; a right-click picks one.

import { div } from "gpui-kit";
import { h_flex, v_flex } from "gpui-base";
import * as hub from "./hub.js";
import { fleet, goToAgent } from "./fleet.js";
import { tokens, TEXT, RADIUS, alpha, Icon, HoverCard, Mono, CornerCount, dur, openMenu } from "./ui.js";

export default class Bell {
  init(_props, cx) {
    this.rows = null;
    this.home = null;
    this.now = Date.now();
    cx.timer.every(30000, (cx) => {
      this.now = Date.now();
      cx.notify();
    });
    this.sub = hub.subscribe(cx, { views: ["agents", "home"] }, (m, cx) => {
      if (m.view === "agents") this.rows = m.value;
      if (m.view === "home") this.home = m.value;
      cx.notify();
    });
  }

  close() {
    this.sub?.stop();
  }

  waiting() {
    return fleet(this.rows, this.home)
      .filter((a) => a.stage === "needs")
      .sort((a, b) => (a.since ?? 0) - (b.since ?? 0));
  }

  command(c) {
    const [scope, address] = c.split(":");
    if (scope === "bell" && address) goToAgent(address);
  }

  render() {
    const k = tokens();
    const list = this.waiting();
    const done = fleet(this.rows, this.home).filter((a) => a.status === "done").length;
    const n = list.length;
    const button = h_flex()
      .id("bell-button")
      .size(32)
      .relative()
      .items_center()
      .justify_center()
      .rounded(RADIUS.md)
      .cursor_pointer()
      .text_color(n ? k.blocked : k.mutedFg)
      .hover((s) => s.bg(k.accent).text_color(n ? k.blocked : k.foreground))
      .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
      .on_click(() => {
        if (list[0]) goToAgent(list[0].address);
      })
      .on_mouse_down("right", (_e, cx) => {
        cx.stop_propagation();
        if (list.length) openMenu(list.map((a) => ({ label: `${a.title}${a.since ? ` · ${dur(this.now - a.since)}` : ""}`, icon: "hand", command: `bell:${a.address}` })), { width: 300 });
      })
      .child(Icon(n ? "bell-dot" : "bell", { size: 17 }))
      .when(n > 0, (e) => e.child(CornerCount(n, { id: "bell-count", top: 2, right: 1 }).loop("opacity", { from: 0.7, to: 1, period: 1400 })))
      .when(n === 0 && done > 0, (e) => e.child(div().absolute().top(6).right(6).size(6).rounded(3).bg(k.done)));
    const card = v_flex()
      .gap(8)
      .child(div().text_size(TEXT.sm).font_semibold().child(n ? `${n} agent${n === 1 ? "" : "s"} waiting for you` : "Nothing waits for you"))
      .when(n === 0, (e) => e.child(div().text_size(TEXT.sm).text_color(k.mutedFg).child(done ? `${done} finished while you were away.` : "Questions show up here the moment an agent asks.")))
      .children(
        list.slice(0, 6).map((a) =>
          v_flex()
            .id(`bell-row-${a.address}`)
            .p(8)
            .gap(2)
            .rounded(RADIUS.md)
            .bg(alpha(k.blocked, 0.08))
            .cursor_pointer()
            .hover((s) => s.bg(alpha(k.blocked, 0.16)))
            .on_click(() => goToAgent(a.address))
            .child(h_flex().gap(6).child(div().flex_1().truncate().text_size(TEXT.sm).font_medium().child(a.title)).child(Mono(a.since ? dur(this.now - a.since) : "", { color: k.blocked })))
            .when(Boolean(a.question?.text), (e) => e.child(div().whitespace_normal().text_size(TEXT.xs).text_color(k.mutedFg).child(a.question?.text ?? ""))),
        ),
      )
      .child(h_flex().gap(6).text_size(TEXT.xs).text_color(k.dim).child("click: the longest waiting · ⌘J: next"));
    return h_flex().id("bell").size_full().items_center().justify_center().pr(8).child(HoverCard("bell-card", button, card, { width: 300, openDelay: 300 }));
  }
}
