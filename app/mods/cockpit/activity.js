// The side panel's activity bar: one icon per panel (Documents, Review, Runbook, Files, Changes, Context, Subagents), the shown one marked by
// an accent edge, each with the badge its view publishes as shared `badge.<name>` = {count, tone}; a click switches the
// panel, the last button closes it.

import { div } from "gpui-kit";
import { h_flex, v_flex } from "gpui-base";
import { run, layout, shared, setShared, readTextFile, writeTextFile } from "herdr";
import * as hub from "./hub.js";
import { tokens, RADIUS, Icon, CornerCount } from "./ui.js";
import { findNode, PANELS } from "./status.js";

const ITEMS = [
  { name: "show", icon: "layers", label: "Documents", key: "⌃⌘1" },
  { name: "review", icon: "message-square", label: "Review", key: "⌃⌘2" },
  { name: "runbook", icon: "book", label: "Runbook", key: "⌃⌘3" },
  { name: "files", icon: "folder", label: "Files", key: "⌃⌘4" },
  { name: "changes", icon: "git-commit", label: "Changes", key: "⌃⌘5" },
  { name: "context", icon: "gauge", label: "Context", key: "⌃⌘6" },
  { name: "subagents", icon: "git-fork", label: "Subagents", key: "⌃⌘7" },
];

export default class Activity {
  init(_p, cx) {
    // The order of the icons (drag one onto another): `panel.order` in the cockpit config.
    this.order = ITEMS.map((i) => i.name);
    this.drag = null;
    const read = (cx) =>
      cx.spawn(async (cx) => {
        try {
          const { home } = await hub.install();
          const cfg = JSON.parse((await readTextFile(`${home}/.config/cockpit/config.json`)) ?? "{}");
          const saved = (cfg.panel?.order ?? []).filter((n) => ITEMS.some((i) => i.name === n));
          const next = [...saved, ...ITEMS.map((i) => i.name).filter((n) => !saved.includes(n))];
          if (next.join() !== this.order.join() && !this.drag) {
            this.order = next;
            cx.notify();
          }
        } catch {}
      });
    read(cx);
    cx.timer.every(6000, (cx) => read(cx));
    // The tab bar restores a space's panel only among these, so the cockpit's own panels survive a space switch.
    const panels = shared("workspace.panels");
    const want = Object.values(PANELS);
    const have = Array.isArray(panels) ? panels : ["files", "changes"];
    if (want.some((p) => !have.includes(p))) setShared("workspace.panels", [...new Set([...have, ...want])]);
  }

  /** Another layout has no such panels: its tab bar would try to restore one and fail. */
  close() {
    const panels = shared("workspace.panels");
    const ours = Object.values(PANELS).filter((p) => p !== "files" && p !== "changes");
    if (Array.isArray(panels) && panels.some((p) => ours.includes(p))) setShared("workspace.panels", panels.filter((p) => !ours.includes(p)));
  }

  /** Moves an icon to where another one is, then keeps the order in the config. */
  move(from, to, cx) {
    const order = this.order.filter((n) => n !== from);
    const at = order.indexOf(to) + (this.order.indexOf(from) < this.order.indexOf(to) ? 1 : 0);
    order.splice(at, 0, from);
    this.order = order;
    cx.spawn(async () => {
      try {
        const { home } = await hub.install();
        const path = `${home}/.config/cockpit/config.json`;
        const before = await readTextFile(path);
        const cfg = before ? JSON.parse(before) : {};
        await writeTextFile(path, JSON.stringify({ ...cfg, panel: { ...(cfg.panel ?? {}), order } }, null, 2) + "\n", before);
      } catch {}
    });
  }

  render() {
    const k = tokens();
    const active = findNode(layout(), "panel-body")?.active;
    const button = (item) => {
      const child = PANELS[item.name];
      const on = active === child;
      const badge = shared(`badge.${item.name}`);
      const tone = badge?.tone ? k[badge.tone] ?? badge.tone : k.ring;
      const d = this.drag;
      const dragging = d && d.from === item.name && d.over !== item.name;
      const over = d && d.over === item.name && d.from !== item.name;
      const after = over && this.order.indexOf(d.from) < this.order.indexOf(item.name);
      return h_flex()
        .id(`act-${item.name}`)
        .w_full()
        .h(40)
        .relative()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .text_color(on ? k.foreground : k.mutedFg)
        .hover((s) => s.text_color(k.foreground))
        .tooltip(`${item.label}  ${item.key}`)
        .on_click(() => run("layout.select", { stack: "panel-body", child }))
        .when(Boolean(dragging), (e) => e.opacity(0.4))
        .when(Boolean(over), (e) => e.child(div().absolute().left(6).right(6).h(2).rounded(1).bg(k.ring).when(after, (l) => l.bottom(0)).when(!after, (l) => l.top(0))))
        .on_mouse_down("left", () => {
          this.drag = { from: item.name, over: item.name };
        })
        .on_mouse_move((_e, cx) => {
          if (this.drag && this.drag.over !== item.name) {
            this.drag.over = item.name;
            cx.notify();
          }
        })
        .on_mouse_up("left", (_e, cx) => {
          const dr = this.drag;
          this.drag = null;
          if (dr && dr.from !== item.name) {
            cx.stop_propagation();
            this.move(dr.from, item.name, cx);
            cx.notify();
          }
        })
        .when(on, (e) => e.child(div().absolute().left(0).top(10).bottom(10).w(2).rounded(1).bg(k.ring)))
        .child(h_flex().size(30).items_center().justify_center().rounded(RADIUS.md).when(on, (e) => e.bg(k.selected)).child(Icon(item.icon, { size: 17 })))
        .when(Boolean(badge?.count), (e) => e.child(CornerCount(badge.count, { tone, top: 4, right: 4 })))
        .when(Boolean(badge?.zero), (e) => e.child(h_flex().absolute().top(4).right(4).h(15).min_w(15).px(4).items_center().justify_center().rounded(8).bg(k.muted).text_size(10).text_color(k.dim).child("0")));
    };
    return v_flex()
      .id("activity")
      .size_full()
      .py(6)
      .gap(2)
      .border_l_1()
      .border_color(k.border)
      .bg(k.chrome)
      .on_mouse_up("left", (_e, cx) => {
        if (this.drag) {
          this.drag = null;
          cx.notify();
        }
      })
      .children(this.order.map((n) => ITEMS.find((i) => i.name === n)).filter(Boolean).map(button))
      .child(div().flex_1())
      .child(
        h_flex()
          .id("act-close")
          .w_full()
          .h(36)
          .items_center()
          .justify_center()
          .cursor_pointer()
          .text_color(k.mutedFg)
          .hover((s) => s.text_color(k.foreground))
          .tooltip("Close the panel  ⌘⌥B")
          .on_click(() => run("layout.hide", { id: "panel" }))
          .child(Icon("panel-right", { size: 16 })),
      );
  }
}
