// The side panel as a drawer on the window's right edge: the view the agent showed beside itself, peeking until the
// pointer slides it out. It holds a `show` of its own only while the pinned panel is closed, so one of the two is ever
// subscribed; pinning it (⌘⌥B) gives the full panel with review, the runbook, files and changes.

import { div } from "gpui-kit";
import { h_flex, v_flex } from "gpui-base";
import { shared } from "herdr";
import * as hub from "./hub.js";
import { PANELED, openPanel } from "./status.js";
import { peekBar, peekHead, MAX_H } from "./side-peek.js";
import Show from "./show.js";
import { tokens } from "./ui.js";

const WIDTH = 420;

export default class PanelPeek {
  init(_props, cx) {
    this.show = null;
    this.cx = cx;
    this.sync(cx);
    cx.spawn(async (cx) => {
      for (;;) {
        await hub.sleep(cx, 500);
        this.sync(cx);
      }
    });
  }

  async close(cx) {
    await this.show?.close?.(cx);
    this.show = null;
  }

  /** Keeps a `show` alive only while the pinned panel is away. */
  sync(cx) {
    const wanted = shared(PANELED) !== true;
    if (wanted === Boolean(this.show)) return;
    if (!wanted) {
      this.show?.close?.(cx);
      this.show = null;
    } else {
      this.show = new Show("show");
      this.show.init({}, cx);
    }
    cx.notify();
  }

  render(cx) {
    const k = tokens();
    // Herdr hides a floating block that draws nothing: the drawer stands down while the panel is pinned.
    if (!this.show) return div();
    const body = v_flex()
      .w(WIDTH)
      .h(MAX_H)
      .text_color(k.foreground)
      .child(peekHead(k, "Beside the agent", null, { tooltip: "Keep the panel open  ⌘⌥B", onPin: () => openPanel(null) }))
      .child(div().flex_1().min_h(0).child(this.show.render(cx)));
    // The bar is on the edge side: this drawer comes out of the right.
    return h_flex().id("panel-peek").child(body).child(peekBar(k));
  }
}
