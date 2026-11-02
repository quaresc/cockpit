// Each agent's prompt queue: follow-ups written while it works, sent one at a time or all at once as one numbered
// prompt (hub view and actions `queue`). It floats as a drawer on the window's right edge (its count peeks out and the
// rest slides out on hover) and opens as a dialog on ⌘⌥Q for the agent in front.

import { div } from "gpui-kit";
import { h_flex, v_flex, InputState } from "gpui-base";
import { placement, props, focused, shared, setShared, openOverlay, layout } from "herdr";
import { findNode } from "./status.js";
import * as hub from "./hub.js";
import { fleet } from "./fleet.js";
import { tokens, TEXT, RADIUS, Icon, Button, Kbd, TextField, Chip, CountPill, CornerCount, Mono, dur } from "./ui.js";

const EDGE = 44;
const WIDTH = 340;

export default class Queue {
  init(_props, cx) {
    this.dialog = props()?.mode === "dialog";
    this.rows = null;
    this.home = null;
    this.model = null;
    this.error = null;
    this.editing = null;
    this.input = InputState.new({ placeholder: "A follow-up for when it is done…" });
    this.input.on("submit", (_e, cx) => this.add(cx));
    this.edit = InputState.new({ placeholder: "" });
    this.edit.on("submit", (_e, cx) => this.saveEdit(cx));
    this.sub = hub.subscribe(cx, { views: ["agents", "home", "queue"] }, (m, cx) => {
      if (m.view === "agents") this.rows = m.value;
      if (m.view === "home") this.home = m.value;
      if (m.view === "queue") this.model = m.value;
      cx.notify();
    });
    if (this.dialog) cx.timer.after(50, () => this.input.focus());
  }

  close() {
    this.sub?.stop();
  }

  command(c, cx) {
    if (c === "queue" && !this.dialog && placement()) {
      const f = focused();
      if (!shared("cockpit.queue.for") && f?.terminal) setShared("cockpit.queue.for", f.terminal);
      openOverlay({ mod: "cockpit", props: { app: "queue", mode: "dialog" }, kind: "dialog", width: 460, height: 440 });
    }
    cx.notify();
  }

  /** The agent whose queue shows: the one asked for, else the one in front, else the first with prompts queued. */
  target(list) {
    const want = shared("cockpit.queue.for");
    const f = focused();
    const queues = this.model?.queues ?? {};
    return (
      list.find((a) => a.address === want) ??
      list.find((a) => a.address === f?.agent || a.address === f?.terminal) ??
      list.find((a) => queues[a.handle]?.length) ??
      list[0] ??
      null
    );
  }

  async run(cx, action, args) {
    const r = await hub.act("queue", action, args).catch((e) => ({ error: String(e?.message ?? e) }));
    this.error = r?.error || null;
    cx.notify();
    return r;
  }

  add(cx) {
    const a = this.target(fleet(this.rows, this.home));
    const text = this.input.value().trim();
    if (!a || !text) return;
    this.input.set_value("");
    cx.spawn(async (cx) => this.run(cx, "add", [a.handle, text]));
  }

  saveEdit(cx) {
    const e = this.editing;
    if (!e) return;
    this.editing = null;
    cx.spawn(async (cx) => this.run(cx, "edit", [e.handle, e.id, this.edit.value()]));
  }

  render() {
    const k = tokens();
    const list = fleet(this.rows, this.home);
    const a = this.target(list);
    const queues = this.model?.queues ?? {};
    const items = a ? queues[a.handle] ?? [] : [];
    const total = this.model?.count ?? 0;
    const p = placement();
    const now = Date.now();

    const row = (item, i) => {
      const editing = this.editing?.id === item.id;
      return h_flex()
        .id(`q-${item.id}`)
        .w_full()
        .min_h(36)
        .px(8)
        .py(6)
        .gap(8)
        .items_start()
        .rounded(RADIUS.md)
        .hover((s) => s.bg(k.accent))
        .child(Mono(i + 1, { color: k.info }))
        .child(
          editing
            ? div().flex_1().child(TextField(this.edit))
            : div()
                .id(`q-${item.id}-text`)
                .flex_1()
                .min_w(0)
                .whitespace_normal()
                .text_size(TEXT.sm)
                .text_color(k.foreground)
                .cursor_text()
                .tooltip(`queued ${dur(now - (item.at ?? now))} ago · double-click to edit`)
                .on_click((e, cx) => {
                  if (e.click_count > 1) {
                    this.editing = { handle: a.handle, id: item.id };
                    this.edit.set_value(item.text);
                    this.edit.focus();
                    cx.notify();
                  }
                })
                .child(item.text),
        )
        .child(
          h_flex()
            .flex_none()
            .gap(2)
            .when(i > 0, (e) => e.child(Button(`q-${item.id}-up`, { icon: "arrow-up", variant: "ghost", size: "icon-sm", tooltip: "Earlier", onClick: (_e, cx) => cx.spawn(async (cx) => this.run(cx, "move", [a.handle, item.id, i - 1])) })))
            .child(Button(`q-${item.id}-send`, { icon: "send", variant: "ghost", size: "icon-sm", tooltip: "Send this one now", onClick: (_e, cx) => cx.spawn(async (cx) => this.run(cx, "send", [a.handle, item.id])) }))
            .child(Button(`q-${item.id}-rm`, { icon: "x", variant: "ghost", size: "icon-sm", tooltip: "Remove", onClick: (_e, cx) => cx.spawn(async (cx) => this.run(cx, "remove", [a.handle, item.id])) })),
        );
    };

    const picker = h_flex()
      .flex_wrap()
      .gap(4)
      .children(
        list.slice(0, 8).map((x) => {
          const n = queues[x.handle]?.length ?? 0;
          const on = x.address === a?.address;
          return Chip(`${x.title}${n ? ` · ${n}` : ""}`, { id: `q-pick-${x.address}`, color: on ? k.ring : undefined, onClick: () => setShared("cockpit.queue.for", x.address) });
        }),
      );

    const status = a ? (a.status === "working" ? "working: they go when you send them" : a.status === "blocked" ? "waiting on a question: answer it first" : "ready for the next prompt") : "";
    const body = v_flex()
      .when(!this.dialog, (e) => e.w(WIDTH).flex_none())
      .when(this.dialog, (e) => e.size_full())
      .p(12)
      .gap(10)
      .child(
        h_flex()
          .gap(8)
          .items_center()
          .child(Icon("list-ordered", { size: 15, color: k.info }))
          .child(div().text_size(TEXT.base).font_semibold().text_color(k.foreground).child("Prompt queue"))
          .child(CountPill(items.length))
          .child(div().flex_1())
          .when(this.dialog, (e) => e.child(Kbd("esc"))),
      )
      .when(list.length > 1, (e) => e.child(picker))
      .when(Boolean(a), (e) => e.child(div().text_size(TEXT.xs).text_color(k.mutedFg).child(`${a?.title} · ${status}`)))
      .child(
        v_flex()
          .id("q-list")
          .gap(2)
          .max_h(this.dialog ? 230 : 300)
          .overflow_y_scrollbar()
          .when(!items.length, (e) => e.child(div().py(14).text_center().text_size(TEXT.sm).text_color(k.dim).child(a ? "Nothing queued for this agent." : "No agent to queue for.")))
          .children(items.map(row)),
      )
      .when(Boolean(a), (e) => e.child(h_flex().gap(6).child(TextField(this.input, { leading: Icon("plus", { size: 14, color: k.mutedFg }) })).child(Button("q-add", { icon: "plus", size: "icon-sm", variant: "outline", tooltip: "Queue it  ⏎", onClick: (_e, cx) => this.add(cx) }))))
      .when(Boolean(this.error), (e) => e.child(div().text_size(TEXT.xs).text_color(k.destructive).child(this.error ?? "")))
      .when(items.length > 0, (e) =>
        e.child(
          h_flex()
            .gap(6)
            .child(Button("q-clear", { label: "Clear", variant: "ghost", size: "sm", onClick: (_e, cx) => cx.spawn(async (cx) => this.run(cx, "clear", [a.handle])) }))
            .child(div().flex_1())
            .child(Button("q-send-next", { label: "Send next", variant: "outline", size: "sm", onClick: (_e, cx) => cx.spawn(async (cx) => this.run(cx, "send", [a.handle, items[0].id])) }))
            .child(Button("q-send-all", { label: items.length > 1 ? `Send all · ${items.length}` : "Send", icon: "send", size: "sm", tooltip: "One numbered prompt", onClick: (_e, cx) => cx.spawn(async (cx) => this.run(cx, "send", [a.handle, "all"])) })),
        ),
      );

    if (this.dialog) return v_flex().id("queue-dialog").size_full().bg(k.popover).child(body);
    // Nothing queued, an app with its own side panel in front, or the work area's panel open on that edge (its activity
    // bar is there): the drawer steps aside, and the status bar's count opens the queue.
    const shownApp = shared("cockpit.app") ?? "home";
    const panel = findNode(layout(), "panel");
    const panelOpen = Boolean(panel && !panel.hidden) && shownApp === "work";
    const aside = !["home", "work"].includes(shownApp) || (panelOpen && p?.edge !== "left");
    if ((!total && !p?.hovered) || aside) return div().size(0);
    // The drawer: its edge strip on the side it hangs from, the queue beside it.
    const edge = v_flex()
      .id("q-edge")
      .w(EDGE)
      .flex_none()
      .py(10)
      .gap(6)
      .items_center()
      .child(h_flex().relative().size(28).items_center().justify_center().child(Icon("list-ordered", { size: 16, color: total ? k.info : k.mutedFg })).when(total > 0, (e) => e.child(CornerCount(total, { tone: k.info, top: -2, right: -4 }))));
    // What peeks is the side on the edge: the strip goes there, the queue opens away from it.
    const left = p?.edge === "left";
    return h_flex().id("queue-drawer").items_start().when(left, (e) => e.child(edge).child(body)).when(!left, (e) => e.child(body).child(edge));
  }
}
