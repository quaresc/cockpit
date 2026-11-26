// A row of pills to pick one thing among a few, each named and counted: the Changes and Review panels pick the
// repository they show with it.

import { div } from "gpui-kit";
import { h_flex } from "gpui-base";
import { tokens, TEXT } from "./ui.js";

/**
 * Pills for `items` ([{value, label, count?, tooltip?}]), the one at `value` filled, `count` in the warning tone when
 * above zero; a click gives `onPick(value, cx)`. Each pill has the id `<id>-<value>`.
 */
export function Pills(id, items, value, onPick) {
  const k = tokens();
  return h_flex()
    .id(id)
    .gap(4)
    .flex_wrap()
    .children(
      items.map((item) => {
        const on = item.value === value;
        return h_flex()
          .id(`${id}-${item.value}`)
          .h(24)
          .px(9)
          .gap(5)
          .items_center()
          .rounded(12)
          .cursor_pointer()
          .text_size(TEXT.xs)
          .text_color(on ? k.foreground : k.mutedFg)
          .when(on, (e) => e.bg(k.selected))
          .hover((s) => s.bg(k.accent))
          .when(Boolean(item.tooltip), (e) => e.tooltip(item.tooltip))
          .on_click((_e, cx) => onPick(item.value, cx))
          .child(div().child(item.label))
          .when(item.count > 0, (e) => e.child(div().text_color(k.blocked).child(String(item.count))));
      }),
    );
}
