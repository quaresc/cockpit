// A menu (shadcn's <ContextMenu>), opened by ui.js's openMenu() at the pointer or under the control a key opened it
// from: each item runs `cockpit.<command>`, which reaches the shown views' command(), and an `info` item only says
// something. An item's `logo` (an agent's icon name) draws in place of its icon, `check` marks the one picked. ↑/↓ and Enter work, Esc closes;
// one a window command opened (`clickOnly`) takes a click only, since the person's keys were meant for something else.

import { div } from "gpui-kit";
import { h_flex, v_flex } from "gpui-base";
import { props, run, closeOverlay } from "herdr";
import { tokens, TEXT, RADIUS, Icon, Kbd, AgentLogo } from "./ui.js";

export default class Menu {
  init(_props, cx) {
    this.at = props()?.picked ?? -1;
    this.handle = cx.focus_handle();
    this.handle.focus();
  }

  choose(item) {
    closeOverlay(props().overlay);
    if (item?.command) run(`cockpit.${item.command}`);
  }

  render() {
    const k = tokens();
    const items = props()?.items ?? [];
    const pickable = items.filter((i) => i !== "separator" && !i.info);
    return v_flex()
      .id("menu")
      .track_focus(this.handle)
      .size_full()
      .p(4)
      .rounded(RADIUS.lg)
      .border_1()
      .border_color(k.border)
      .bg(k.popover)
      .shadow_lg()
      .text_size(TEXT.base)
      .on_key_down((e, cx) => {
        if (e.key === "escape") return closeOverlay(props().overlay);
        if (e.key === "down") this.at = Math.min(this.at + 1, pickable.length - 1);
        else if (e.key === "up") this.at = Math.max(this.at - 1, 0);
        else if (e.key === "enter") return props()?.clickOnly ? undefined : this.choose(pickable[this.at]);
        else return;
        cx.stop_propagation();
        cx.notify();
      })
      .children(
        items.map((item, i) =>
          item === "separator"
            ? div().h(1).my(4).mx(-4).bg(k.border)
            : item.info
              ? h_flex()
                  .id(`menu-${i}`)
                  .h(32)
                  .px(8)
                  .gap(8)
                  .items_center()
                  .text_size(TEXT.sm)
                  .text_color(k.mutedFg)
                  .child(item.icon ? Icon(item.icon, { size: 14, color: k.mutedFg }) : div().w(16))
                  .child(div().flex_1().truncate().child(item.label))
                  .tooltip(item.label)
            : h_flex()
                .id(`menu-${i}`)
                .h(32)
                .px(8)
                .gap(8)
                .items_center()
                .rounded(RADIUS.md)
                .cursor_pointer()
                .text_color(item.destructive ? k.destructive : k.foreground)
                .when(pickable[this.at] === item, (e) => e.bg(k.accent))
                .hover((s) => s.bg(k.accent))
                .on_click(() => this.choose(item))
                .child(item.logo ? AgentLogo(item.logo, { size: 16, color: k.mutedFg }) : item.icon ? Icon(item.icon, { size: 16, color: item.destructive ? k.destructive : k.mutedFg }) : div().w(16))
                .child(div().flex_1().truncate().child(item.label))
                .when(Boolean(item.check), (e) => e.child(Icon("check", { size: 14, color: k.mutedFg })))
                .when(Boolean(item.kbd), (e) => e.child(Kbd(item.kbd))),
        ),
      );
  }
}
