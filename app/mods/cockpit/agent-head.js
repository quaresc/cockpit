// The header over a lone agent's terminal (the work area's agent view, the agent opened beside Home, the reading layout's
// terminal window): which agent, its state and how long, its branch, PR and ticket, and buttons to go back to its tab,
// queue a prompt, stop its turn, recap it or close. Its keys are the shown layout's.

import { div } from "gpui-kit";
import { h_flex } from "gpui-base";
import { run, focused, props, setShared, closeOverlay, notify } from "herdr";
import * as hub from "./hub.js";
import { sharing } from "./feed.js";
import { fleet, goToAgent, agentAct } from "./fleet.js";
import { hint, keyFor, keyText, available } from "./commands.js";
import { layoutName } from "./screens.js";
import { statusColor } from "./agentsui.js";
import { tokens, TEXT, Spinner, Button, Chip, Mono, dur, AgentLogo } from "./ui.js";

// The agents, Home and queue views, followed once per window whichever copies ask (feed.js).
const following = sharing(hub.subscribe);

const WORD = { blocked: "needs you", failed: "stopped with an error", working: "working", done: "done", idle: "idle" };

/** The agents a Stop was sent to and has not answered yet: their button waits. */
const stopping = new Set();

/** Stops the agent's turn once at a time; a refusal (it finished meanwhile) is said. */
async function stop(a, cx) {
  if (stopping.has(a.handle)) return;
  stopping.add(a.handle);
  cx.notify();
  const r = await agentAct("interrupt", { terminal: a.handle }).catch((e) => ({ error: String(e?.message ?? e) }));
  stopping.delete(a.handle);
  cx.notify();
  if (r?.error) notify({ title: "It did not stop", body: String(r.error), level: "warning", timeout: 5000 });
}

/**
 * Stop, naming the keys the agent's row says stop its turn in its terminal: a Kbd in Cockpit, the tooltip in Cockpit
 * Chat (Button moves it there), nothing when none is measured.
 */
export function stopButton(a, interrupt) {
  const busy = stopping.has(a.handle);
  return Button("head-stop", {
    label: "Stop",
    icon: "square",
    variant: "ghost",
    size: "sm",
    kbd: busy ? undefined : (interrupt ?? undefined),
    disabled: busy,
    tooltip: busy ? "Stopping…" : interrupt ? "Stop its turn, also in its terminal" : "Stop its turn",
    onClick: (_e, cx) => cx.spawn(async (cx) => stop(a, cx)),
  });
}

export default class AgentHead {
  init(_props, cx) {
    this.place = props()?.place ?? "work";
    // The agent a window over the chat was opened for; else the one in front.
    this.named = props()?.agent ?? null;
    this.rows = null;
    this.home = null;
    this.now = Date.now();
    this.sub = following(cx, { views: ["agents", "home"] }, (m, cx) => {
      if (m.view === "agents") this.rows = m.value;
      if (m.view === "home") this.home = m.value;
      cx.notify();
    });
  }

  close() {
    this.sub?.stop();
  }

  /** The times shown move on every 30 s, on the copy's clock (main.js). */
  tick(cx, now) {
    if (now - this.now < 30_000) return;
    this.now = now;
    cx.notify();
  }

  render() {
    const k = tokens();
    const list = fleet(this.rows, this.home);
    const f = focused();
    const a = this.named
      ? (list.find((x) => x.address === this.named) ?? null)
      : (list.find((x) => x.address === f?.agent) ?? list.find((x) => x.address === f?.terminal) ?? list[0] ?? null);
    const tabKey = keyFor("cockpit.agent-view");
    const back =
      this.place === "home"
        ? Button("head-close", { icon: "x", variant: "ghost", size: "icon-sm", tooltip: "Close  f", onClick: () => run("layout.hide", { id: "home-agent" }) })
        : this.place === "overlay"
          ? // The reading layout's terminal window: the chat is under it, so closing it is the way back.
            Button("head-close", { icon: "x", variant: "ghost", size: "icon-sm", tooltip: "Close", onClick: () => closeOverlay(props()?.overlay) })
          : Button("head-tab", {
              label: "Tab",
              icon: "minimize",
              variant: "ghost",
              size: "sm",
              kbd: tabKey ? keyText(tabKey) : undefined,
              tooltip: "Back to the whole tab",
              onClick: () => {
                run("layout.select", { stack: "view", child: "main" });
                setShared("cockpit.view", "main");
              },
            });
    // The queue drawer is the cockpit layout's; the reading layout queues from its own box.
    const queue = available(layoutName(), "queue");
    if (!a)
      return h_flex().id("agent-head").size_full().px(10).gap(8).items_center().border_b_1().border_color(k.border).text_size(TEXT.sm).text_color(k.mutedFg).child("No agent").child(div().flex_1()).child(back);
    const color = statusColor(k, a.status);
    const since = a.status === "working" ? a.working : a.since;
    // The keys the agent's own table says stop a turn, typed in its terminal: what Stop sends.
    const interrupt = Array.isArray(a.row?.interrupt) && a.row.interrupt.length ? a.row.interrupt.join(" ") : null;
    const icon = AgentLogo(a, { size: 14, color: k.mutedFg, tooltip: a.agent_name, id: "agent-head-logo" });
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
          .child(div().text_size(TEXT.xs).text_color(a.status === "blocked" || a.status === "failed" ? color : k.mutedFg).child(`${a.status === "done" && a.interrupted ? "stopped by you" : (WORD[a.status] ?? a.status)}${since ? ` · ${dur(this.now - since)}` : ""}`)),
      )
      .when(Boolean(a.branch), (e) => e.child(div().min_w(0).truncate().child(Mono(a.branch, { color: k.dim }))))
      .children(
        (a.row?.links ?? [])
          .filter((l) => l.kind === "ticket" || l.kind === "pr")
          .map((l) => Chip(l.label ?? l.name, { id: `head-${l.kind}`, icon: l.icon, tooltip: hint(l.detail ? `${l.name} · ${l.detail}` : l.name, l.kind === "pr" ? "cockpit.pr" : "cockpit.ticket"), onClick: (_e, cx) => cx.spawn(async () => hub.openUrl(l.url)) })),
      )
      .child(div().flex_1())
      .when(queue, (e) =>
        e.child(
          Button("head-queue", {
            icon: "list-ordered",
            variant: "ghost",
            size: "icon-sm",
            tooltip: hint("Queue a prompt for it", "cockpit.queue"),
            onClick: () => {
              setShared("cockpit.queue.for", a.address);
              run("cockpit.queue", {});
            },
          }),
        ),
      )
      .when(a.status === "working" && Boolean(a.row?.stoppable) && !a.remote, (e) =>
        e.child(stopButton(a, interrupt)),
      )
      .child(Button("head-recap", { icon: "sparkles", variant: "ghost", size: "icon-sm", tooltip: "Recap", onClick: (_e, cx) => cx.spawn(async () => agentAct("recap", { terminal: a.handle })) }))
      .when(this.place === "home", (e) => e.child(Button("head-go", { icon: "maximize", variant: "ghost", size: "icon-sm", tooltip: "Go to its tab", onClick: () => goToAgent(a) })))
      .child(back);
  }
}
