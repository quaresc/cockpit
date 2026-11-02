// The header over a lone agent's terminal (the work area's agent view, the agent opened beside Home): which agent, its
// state and how long, its branch, PR and ticket, and buttons to go back to its tab, queue a prompt, recap it or close.

import { div, svg, image } from "gpui-kit";
import { h_flex } from "gpui-base";
import { run, focused, props, setShared, exec } from "herdr";
import * as hub from "./hub.js";
import { fleet, goToAgent, agentAct } from "./fleet.js";
import { tokens, TEXT, Icon, Spinner, Button, Chip, Mono, dur } from "./ui.js";

const WORD = { blocked: "needs you", working: "working", done: "done", idle: "idle" };

export default class AgentHead {
  init(_props, cx) {
    this.place = props()?.place ?? "work";
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

  render() {
    const k = tokens();
    const list = fleet(this.rows, this.home);
    const f = focused();
    const a = list.find((x) => x.address === f?.agent) ?? list.find((x) => x.address === f?.terminal) ?? list[0] ?? null;
    const back =
      this.place === "home"
        ? Button("head-close", { icon: "x", variant: "ghost", size: "icon-sm", tooltip: "Close  f", onClick: () => run("layout.hide", { id: "home-agent" }) })
        : Button("head-tab", {
            label: "Tab",
            icon: "minimize",
            variant: "ghost",
            size: "sm",
            kbd: "⌘⌥A",
            tooltip: "Back to the whole tab",
            onClick: () => {
              run("layout.select", { stack: "view", child: "main" });
              setShared("cockpit.view", "main");
            },
          });
    if (!a)
      return h_flex().id("agent-head").size_full().px(10).gap(8).items_center().border_b_1().border_color(k.border).text_size(TEXT.sm).text_color(k.mutedFg).child("No agent").child(div().flex_1()).child(back);
    const color = k[a.status] ?? k.idle;
    const since = a.status === "working" ? a.working : a.since;
    const p = a.program;
    const icon = p?.icon ? (p.tint ? svg(p.icon).size(14).flex_none().text_color(k.mutedFg) : image(p.icon).w(14).h(14).flex_none()) : Icon("bot", { size: 14, color: k.mutedFg });
    return h_flex()
      .id("agent-head")
      .size_full()
      .pl(12)
      .pr(6)
      .gap(8)
      .items_center()
      .border_b_1()
      .border_color(k.border)
      .child(icon)
      .child(div().min_w(0).truncate().text_size(TEXT.base).font_medium().text_color(k.foreground).child(a.title))
      .child(
        h_flex()
          .flex_none()
          .gap(5)
          .items_center()
          .child(a.status === "working" ? Spinner({ size: 11, color }) : div().size(7).rounded(4).bg(color))
          .child(div().text_size(TEXT.xs).text_color(a.status === "blocked" ? color : k.mutedFg).child(`${WORD[a.status] ?? a.status}${since ? ` · ${dur(this.now - since)}` : ""}`)),
      )
      .when(Boolean(a.branch), (e) => e.child(div().min_w(0).truncate().child(Mono(a.branch, { color: k.dim }))))
      .when(Boolean(a.pr?.number), (e) => e.child(Chip(`#${a.pr?.number}`, { id: "head-pr", mono: true, icon: "git-pull-request", tooltip: "Open the PR  ⌘G", onClick: (_e, cx) => cx.spawn(async () => exec("open", [a.pr.url])) })))
      .when(Boolean(a.ticket), (e) => e.child(Chip(a.ticket, { mono: true, icon: "ticket" })))
      .child(div().flex_1())
      .child(
        Button("head-queue", {
          icon: "list-ordered",
          variant: "ghost",
          size: "icon-sm",
          tooltip: "Queue a prompt for it  ⌘⌥Q",
          onClick: () => {
            setShared("cockpit.queue.for", a.address);
            run("cockpit.queue", {});
          },
        }),
      )
      .child(Button("head-recap", { icon: "sparkles", variant: "ghost", size: "icon-sm", tooltip: "Recap  ⌘R", onClick: (_e, cx) => cx.spawn(async () => agentAct("recap", { terminal: a.handle })) }))
      .when(this.place === "home", (e) => e.child(Button("head-go", { icon: "maximize", variant: "ghost", size: "icon-sm", tooltip: "Go to its tab", onClick: () => goToAgent(a.address) })))
      .child(back);
  }
}
