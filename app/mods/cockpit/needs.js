// Needs you: every agent waiting on an answer, its question answerable where it is listed. What the Home board's
// NEXT MOVE shows, as a screen of its own for the reading layout.

import { div } from "gpui-kit";
import { h_flex, v_flex } from "gpui-base";
import * as hub from "./hub.js";
import { AgentsModel, QuestionCard, stageColor } from "./agentsui.js";
import { rowsOf, goToAgent } from "./fleet.js";
import { show } from "./screens.js";
import { tokens, TEXT, RADIUS, Icon, Empty, Spinner, dur } from "./ui.js";

const READ_W = 800;

export default class Needs {
  init(_props, cx) {
    this.model = new AgentsModel();
    this.model.follow(cx);
  }

  close() {
    this.model.stop();
  }

  command(c, cx) {
    this.model.command(c, cx);
    cx.notify();
  }

  /** The waiting agents, the one waiting longest first (the agents view already sorts them that way). */
  waiting() {
    return rowsOf(this.model.value).filter((r) => r.stage === "needs_you" || r.question);
  }

  header(row, k) {
    const since = row.waiting_since ?? row.since;
    return h_flex()
      .w_full()
      .gap(8)
      .items_center()
      .child(div().size(8).flex_none().rounded(4).bg(stageColor("needs_you")))
      .child(div().text_size(TEXT.base).font_semibold().text_color(k.foreground).child(row.title ?? row.terminal))
      .child(div().text_size(TEXT.sm).text_color(k.mutedFg).child("asks you"))
      .child(div().flex_1())
      .when(Boolean(since), (e) => e.child(div().text_size(TEXT.xs).text_color(k.dim).child(dur(Date.now() - since))))
      .child(
        div()
          .id(`needs-open-${row.terminal}`)
          .size(24)
          .flex()
          .items_center()
          .justify_center()
          .rounded(RADIUS.md)
          .cursor_pointer()
          .text_color(k.dim)
          .hover((s) => s.bg(k.accent).text_color(k.foreground))
          .tooltip("Read the conversation")
          .on_click((_e, cx) => {
            goToAgent({ address: row.address });
            show("chat");
            cx.notify();
          })
          .child(Icon("message-square", { size: 13 })),
      );
  }

  render(_cx) {
    const k = tokens();
    if (!this.model.value) return v_flex().size_full().bg(k.canvas).items_center().justify_center().child(Spinner({ size: 18, color: k.mutedFg }));
    const rows = this.waiting();
    const body =
      rows.length === 0
        ? v_flex().flex_1().items_center().justify_center().child(Empty({ icon: "check", title: "Nobody is waiting", description: "Every agent is working or done." }))
        : v_flex()
            .id("needs-list")
            .flex_1()
            .min_h(0)
            .px(24)
            .py(16)
            .gap(12)
            .items_center()
            .overflow_y_scrollbar()
            .children(rows.map((r) => div().w_full().max_w(READ_W).child(QuestionCard(`needs-${r.terminal}`, r, this.model, { wide: true, header: this.header(r, k) }))));
    return v_flex().id("needs").size_full().bg(k.canvas).text_color(k.foreground).child(body);
  }
}
