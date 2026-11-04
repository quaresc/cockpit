// A text a run or a session wrote, read as a page: its Markdown rendered (headings, lists, code, links), selectable,
// copied in one click. Opened as a window over the cockpit with `openReader({ path | text, title, meta })`.

import { div } from "gpui-kit";
import { h_flex, v_flex, TextView } from "gpui-base";
import { props, readTextFile, openOverlay, closeOverlay } from "herdr";
import { openUrl } from "./hub.js";
import { tokens, TEXT, RADIUS, Icon, Button, Toaster, Skeleton } from "./ui.js";

/** Opens the reader window on a file or a text. */
export function openReader({ path, text, title, meta }) {
  openOverlay({ mod: "cockpit", kind: "window", key: `reader:${path ?? title}`, width: 760, height: 0.8, props: { app: "reader", path, text, title, meta } });
}

/** Markdown as cockpit draws it in a panel: selectable, links to the browser. */
export function Markdown(id, text, { size = TEXT.sm } = {}) {
  return TextView.markdown(id, String(text ?? "")).selectable(true).text_size(size).on_link_click((url) => openUrl(url));
}

export default class Reader {
  init(p, cx) {
    this.p = p ?? props() ?? {};
    this.text = this.p.text ?? null;
    this.toasts = new Toaster();
    if (this.text === null && this.p.path)
      cx.spawn(async (cx) => {
        this.text = (await readTextFile(this.p.path).catch(() => "")) ?? "";
        cx.notify();
      });
  }

  key(e, cx) {
    if (e.keystroke === "escape") closeOverlay(props().overlay);
    else if (e.keystroke === "c" || e.keystroke === "cmd-c") this.copy(cx);
    else return false;
    return true;
  }

  copy(cx) {
    cx.write_to_clipboard(this.text ?? "");
    this.toasts.push(cx, "Copied", { kind: "success" });
    cx.notify();
  }

  render(cx) {
    const k = tokens();
    return v_flex()
      .id("reader")
      .size_full()
      .bg(k.background)
      .text_color(k.foreground)
      .child(
        h_flex()
          .h(52)
          .px(20)
          .gap(10)
          .items_center()
          .border_b_1()
          .border_color(k.border)
          .child(Icon("file-text", { size: 16, color: k.mutedFg }))
          .child(v_flex().flex_1().min_w(0).child(div().truncate().font_semibold().child(this.p.title ?? "Output")).when(Boolean(this.p.meta), (e) => e.child(div().truncate().text_size(TEXT.xs).text_color(k.mutedFg).child(this.p.meta))))
          .child(Button("reader-copy", { label: "Copy", icon: "copy", variant: "outline", size: "sm", kbd: "c", onClick: (_e, cx) => this.copy(cx) }))
          .child(Button("reader-close", { icon: "x", variant: "ghost", size: "icon-sm", tooltip: "Close  esc", onClick: () => closeOverlay(props().overlay) })),
      )
      .child(
        v_flex()
          .id("reader-scroll")
          .flex_1()
          .min_h(0)
          .overflow_y_scrollbar()
          .px(28)
          .py(20)
          .child(this.text === null ? Skeleton({ h: 120 }) : this.text.trim() ? Markdown("reader-md", this.text, { size: TEXT.base }) : div().text_color(k.mutedFg).child("Nothing was written.")),
      )
      .child(this.toasts.render() ?? div());
  }
}
