// The side panel's activity bar: one icon per panel (Documents, Review, Runbook, Files, Changes), the shown one marked by
// an accent edge, each with the badge its view publishes as shared `badge.<name>` = {count, tone}; a click switches the
// panel, the last button closes it.

import { div } from "gpui-kit";
import { h_flex, v_flex } from "gpui-base";
import { run, layout, shared, setShared } from "herdr";
import { tokens, RADIUS, Icon, CornerCount } from "./ui.js";
import { findNode, PANELS } from "./status.js";

const ITEMS = [
  { name: "show", icon: "layers", label: "Documents", key: "⌃⌘1" },
  { name: "review", icon: "message-square", label: "Review", key: "⌃⌘2" },
  { name: "runbook", icon: "book", label: "Runbook", key: "⌃⌘3" },
  { name: "files", icon: "folder", label: "Files", key: "⌃⌘4" },
  { name: "changes", icon: "git-commit", label: "Changes", key: "⌃⌘5" },
  { name: "context", icon: "gauge", label: "Context", key: "⌃⌘6" },
];

export default class Activity {
  init() {
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

  render() {
    const k = tokens();
    const active = findNode(layout(), "panel-body")?.active;
    const button = (item) => {
      const child = PANELS[item.name];
      const on = active === child;
      const badge = shared(`badge.${item.name}`);
      const tone = badge?.tone ? k[badge.tone] ?? badge.tone : k.ring;
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
        .when(on, (e) => e.child(div().absolute().left(0).top(10).bottom(10).w(2).rounded(1).bg(k.ring)))
        .child(h_flex().size(30).items_center().justify_center().rounded(RADIUS.md).when(on, (e) => e.bg(k.selected)).child(Icon(item.icon, { size: 17 })))
        .when(Boolean(badge?.count), (e) => e.child(CornerCount(badge.count, { tone, top: 4, right: 4 })));
    };
    return v_flex()
      .id("activity")
      .size_full()
      .py(6)
      .gap(2)
      .border_l_1()
      .border_color(k.border)
      .bg(k.chrome)
      .children(ITEMS.map(button))
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
