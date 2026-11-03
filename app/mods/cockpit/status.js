// The status bar under the cockpit layout: the agents by whose move it is (each count a click), then on the right the
// focused agent's links (branch, ticket, CI, PR), queued prompts, stopped agents, and the hub only when it is offline.
// Always shown, it also runs the shell's window-wide keys: Home, the agent view, the side panel, next agent waiting,
// the focused branch's PR and ticket, resume.

import { div } from "gpui-kit";
import { h_flex } from "gpui-base";
import { run, focused, shared, setShared, layout, exec } from "herdr";
import * as hub from "./hub.js";
import { fleet, goToAgent, counts } from "./fleet.js";
import { show } from "./sidebar.js";
import { tokens, TEXT, RADIUS, Icon, Spinner, Mono } from "./ui.js";

/** A node of the shown layout by id. */
export function findNode(node, id) {
  if (!node || typeof node !== "object") return null;
  if (node.id === id) return node;
  for (const c of [...(node.children ?? []), ...(node.floating ?? [])]) {
    const found = findNode(c, id);
    if (found) return found;
  }
  return null;
}

/** The side panel's children by the names keys and the activity bar use. */
export const PANELS = { show: "show", review: "review", runbook: "runbook-panel", files: "files", changes: "changes" };

/** Opens the side panel on that child, or closes it when it already shows it. */
export function togglePanel(name) {
  const child = PANELS[name] ?? name;
  const panel = findNode(layout(), "panel");
  const body = findNode(layout(), "panel-body");
  if (panel && !panel.hidden && (!child || body?.active === child)) run("layout.hide", { id: "panel" });
  else {
    if (child) run("layout.select", { stack: "panel-body", child });
    run("layout.show", { id: "panel" });
    if ((shared("cockpit.app") ?? "home") !== "work") show("work");
  }
}

export default class Status {
  init(_props, cx) {
    this.rows = null;
    this.home = null;
    this.queue = null;
    this.state = "connecting";
    this.sub = hub.subscribe(
      cx,
      { views: ["agents", "home", "queue"] },
      (m, cx) => {
        if (m.view === "agents") this.rows = m.value;
        if (m.view === "home") this.home = m.value;
        if (m.view === "queue") this.queue = m.value;
        cx.notify();
      },
      (state, cx) => {
        this.state = state;
        cx.notify();
      },
    );
  }

  close() {
    this.sub?.stop();
  }

  /** The agent the person looks at: the agent block's, else the focused pane's. */
  current(list) {
    const f = focused();
    const address = f?.agent ?? f?.terminal ?? null;
    return list.find((a) => a.address === address) ?? null;
  }

  next(list, stage) {
    const waiting = list.filter((a) => (stage ? a.stage === stage || a.status === stage : a.stage === "needs")).sort((a, b) => (a.since ?? 0) - (b.since ?? 0));
    const here = focused()?.terminal;
    const i = waiting.findIndex((a) => a.address === here);
    return waiting[(i + 1) % Math.max(1, waiting.length)] ?? null;
  }

  command(c, cx) {
    const list = fleet(this.rows, this.home);
    const me = this.current(list);
    if (c === "home") show((shared("cockpit.app") ?? "home") === "home" ? "work" : "home");
    else if (c === "agent-view") {
      const on = shared("cockpit.view") === "agent";
      run("layout.select", { stack: "view", child: on ? "main" : "agent" });
      setShared("cockpit.view", on ? "main" : "agent");
      if ((shared("cockpit.app") ?? "home") !== "work") show("work");
    } else if (c === "panel") togglePanel(null);
    else if (c.startsWith("panel:")) togglePanel(c.slice(6));
    else if (c === "next-waiting" || c === "next:blocked" || c === "next:done") {
      const a = this.next(list, c === "next:done" ? "done" : c === "next:blocked" ? "needs" : null) ?? (c === "next-waiting" ? this.next(list, "done") : null);
      if (a) goToAgent(a.address);
    } else if (c === "pr" && me?.pr?.url) cx.spawn(async () => exec("open", [me.pr.url]));
    else if (c === "ticket" && me?.ticketUrl) cx.spawn(async () => exec("open", [me.ticketUrl]));
    else if (c === "resume") cx.spawn(async () => hub.act("home", "resume", []));
    cx.notify();
  }

  render(cx) {
    const k = tokens();
    const list = fleet(this.rows, this.home);
    const board = counts(list, this.home);
    const count = (stage) => board[stage];
    const me = this.current(list);
    const queued = this.queue?.count ?? 0;
    const lost = this.home?.lost?.length ?? 0;
    const ready = board.ready;

    const item = (id, el, { onClick, tooltip } = {}) => {
      let e = h_flex().id(id).h(22).px(6).gap(5).flex_none().items_center().rounded(RADIUS.sm).text_size(TEXT.xs).text_color(k.mutedFg);
      if (onClick) e = e.cursor_pointer().hover((s) => s.bg(k.accent).text_color(k.foreground)).on_click((ev, cx) => onClick(ev, cx));
      if (tooltip) e = e.tooltip(tooltip);
      return e.children(Array.isArray(el) ? el : [el]);
    };
    let dots = 0;
    const dot = (color, pulse = false) => div().id(`st-dot-${dots++}`).size(6).flex_none().rounded(3).bg(color).when(pulse, (e) => e.loop("opacity", { from: 0.35, to: 1, period: 1200 }));
    const sep = () => div().w(1).h(12).mx(4).flex_none().bg(k.border);

    const needs = count("needs");
    // What a narrow bar leaves out, least needed first: quota (the sidebar has it), ahead/files, CI/PR, ready/done.
    const w = this.width ?? 1280;
    const left = [
      item("st-needs", [dot(needs ? k.blocked : k.border, needs > 0), div().when(needs > 0, (e) => e.text_color(k.blocked).font_medium()).child(`${needs} need${needs === 1 ? "s" : ""} you`)], {
        onClick: (_e, cx) => this.command("next-waiting", cx),
        tooltip: "Next agent waiting  ⌘J",
      }),
      item("st-working", [count("working") ? Spinner({ size: 10, color: k.working }) : dot(k.border), div().child(`${count("working")} working`)], { onClick: () => show("home"), tooltip: "Home  ⌘⌥0" }),
      w >= 640 && item("st-ready", [dot(ready ? k.success : k.border), div().child(`${ready} ready`)], { onClick: () => show("home"), tooltip: "Agents whose PR is approved and green" }),
      w >= 640 && item("st-done", [dot(count("done") ? k.done : k.border), div().child(`${count("done")} done`)], { onClick: (_e, cx) => this.command("next:done", cx), tooltip: "Next finished agent  ⌘⇧U" }),
    ].filter(Boolean);

    // The focused agent's links, on the right: branch, ticket, CI and PR, each opening its page.
    const right = [];
    const open = (url) => (url ? (_e, cx) => cx.spawn(async () => exec("open", [url])) : undefined);
    if (me) {
      const ch = me.changes ?? {};
      const repoUrl = me.pr?.url?.replace(/\/pull\/\d+.*$/, "") ?? null;
      const branchUrl = repoUrl && me.branch ? `${repoUrl}/tree/${me.branch}` : null;
      // The branch gives way first when the bar runs out of room.
      right.push(item("st-branch", [Icon("git-branch", { size: 12 }), Mono(me.branch ?? me.space_name ?? me.address).flex_shrink(1).min_w(0).truncate()], { tooltip: branchUrl ? `Open ${me.branch} on GitHub` : `${me.title} · ${me.branch ?? ""}`, onClick: open(branchUrl) }).flex_shrink(1).min_w(60));
      if (w >= 900 && (ch.ahead || ch.behind)) right.push(item("st-ahead", [Mono(`↑${ch.ahead ?? 0} ↓${ch.behind ?? 0}`)], { tooltip: "Commits ahead of and behind its base" }));
      if (w >= 900 && ch.files) right.push(item("st-files", [Mono(`±${ch.files}`)], { tooltip: `${ch.files} files changed`, onClick: () => togglePanel("changes") }));
      if (me.ticket?.key) right.push(item("st-ticket", [Icon("ticket", { size: 12 }), Mono(me.ticket.key)], { tooltip: me.ticket.title ? `${me.ticket.key} · ${me.ticket.title}` : me.ticket.key, onClick: open(me.ticket.url) }));
      const ci = me.ci;
      if (ci && w >= 720) {
        const color = ci === "SUCCESS" ? k.success : ci === "FAILURE" || ci === "ERROR" ? k.destructive : k.warning;
        right.push(item("st-ci", [dot(color, ci === "PENDING"), div().child("CI")], { tooltip: `CI ${String(ci).toLowerCase()}: open the checks`, onClick: open(me.pr?.url ? `${me.pr.url}/checks` : null) }));
      }
      if (me.pr?.number) right.push(item("st-pr", [Icon("git-pull-request", { size: 12 }), Mono(`#${me.pr.number}`)], { tooltip: "Open the PR  ⌘G", onClick: open(me.pr.url) }));
    }
    if (queued) right.push(item("st-queue", [Icon("list-ordered", { size: 12, color: k.info }), div().child(w >= 800 ? `${queued} queued` : String(queued))], { tooltip: "Prompts waiting to be sent  ⌘⌥Q", onClick: () => run("cockpit.queue", {}) }));
    if (lost) right.push(item("st-lost", [Icon("rotate-ccw", { size: 12, color: k.info }), div().text_color(k.info).child(`${lost} to resume`)], { tooltip: "Agents a restart stopped: resume them  ⌘⌥R", onClick: (_e, cx) => cx.spawn(async () => hub.act("home", "resume", [])) }));
    // The hub shows only when it does not answer.
    if (this.state !== "live") right.push(item("st-hub", [dot(this.state === "connecting" ? k.warning : k.destructive), div().child(this.state === "connecting" ? "connecting" : "hub offline")], { tooltip: this.state }));

    return h_flex()
      .id("cockpit-status")
      .size_full()
      .on_size_change((size, cx) => {
        if (Math.abs((this.width ?? 0) - size.width) < 1) return;
        this.width = size.width;
        cx.notify();
      })
      .px(8)
      .gap(2)
      .items_center()
      .bg(k.chrome)
      .border_t_1()
      .border_color(k.border)
      .children(left)
      .child(div().flex_1().min_w(8))
      .children(right);
  }
}
