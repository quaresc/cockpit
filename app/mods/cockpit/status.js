// The status bar under the cockpit layout: the agents by whose move it is (each count a click), then on the right the
// focused agent's branch and its links named after their service (GitHub, Linear, CircleCI, Argo, Slack…), queued
// prompts, stopped agents, the hub only when it is offline, and the agent's documents at the right end.
// Always shown, it also runs the shell's window-wide keys: Home, the agent view, the side panel, next agent waiting,
// the focused branch's PR and ticket, resume.

import { div } from "gpui-kit";
import { h_flex } from "gpui-base";
import { run, focused, shared, setShared, layout } from "herdr";
import * as hub from "./hub.js";
import { fleet, goToAgent, counts } from "./fleet.js";
import { show, showWork, screens } from "./screens.js";
import { tokens, TEXT, RADIUS, Icon, Spinner, Mono, openMenu } from "./ui.js";

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

/** Whether the side panel is open, and on what: the layout file cannot say, and the drawer on the edge reads it. */
export const PANELED = "cockpit.panel.open";
export const PANEL_CHILD = "cockpit.panel.child";

/** Opens the side panel on that child, left open when it already shows it. */
export function openPanel(name) {
  const child = PANELS[name] ?? name ?? shared(PANEL_CHILD) ?? "show";
  setShared(PANELED, true);
  setShared(PANEL_CHILD, child);
  run("layout.select", { stack: "panel-body", child });
  run("layout.show", { id: "panel" });
  onWorkScreen();
}

/** Closes the side panel, and says so: the drawer on the edge takes over from the flag. */
export function closePanel() {
  setShared(PANELED, false);
  run("layout.hide", { id: "panel" });
}

/** Opens the side panel on that child, or closes it when it already shows it. */
export function togglePanel(name) {
  const child = PANELS[name] ?? name;
  if (shared(PANELED) === true && (!child || shared(PANEL_CHILD) === child)) {
    closePanel();
  } else {
    openPanel(child);
  }
}

/** The panel only shows beside the agent, so opening it brings that screen up in whichever layout is on. */
function onWorkScreen() {
  const here = shared("cockpit.app");
  if (here !== screens().work) showWork();
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
      if (a) goToAgent(a);
    } else if (c === "pr" && me?.pr?.url) cx.spawn(async () => hub.openUrl(me.pr.url));
    else if (c === "ticket" && me?.ticketUrl) cx.spawn(async () => hub.openUrl(me.ticketUrl));
    else if (c === "resume") cx.spawn(async () => hub.act("home", "resume", []));
    else if (c.startsWith("link:")) cx.spawn(async () => hub.openUrl(c.slice(5)));
    else if (c.startsWith("doc:")) {
      openPanel("show");
      run("cockpit.show:go:" + c.slice(4));
    }
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

    const item = (id, el, { onClick, tooltip, button = false } = {}) => {
      let e = h_flex().id(id).h(26).px(8).gap(6).flex_none().items_center().rounded(RADIUS.md).text_size(TEXT.sm).text_color(k.mutedFg);
      // The agent's links and documents read as buttons: filled and outlined, apart from the plain facts around them.
      if (button) e = e.h(24).px(10).bg(k.muted).border_1().border_color(k.border).text_color(k.foreground);
      if (onClick) e = e.cursor_pointer().hover((s) => s.bg(k.accent).text_color(k.foreground)).on_click((ev, cx) => onClick(ev, cx));
      if (tooltip) e = e.tooltip(tooltip);
      return e.children(Array.isArray(el) ? el : [el]);
    };
    let dots = 0;
    const dot = (color, pulse = false) => div().id(`st-dot-${dots++}`).size(7).flex_none().rounded(3).bg(color).when(pulse, (e) => e.loop("opacity", { from: 0.35, to: 1, period: 1200 }));
    const sep = () => div().w(1).h(12).mx(4).flex_none().bg(k.border);

    const needs = count("needs");
    // What a narrow bar leaves out, least needed first: quota (the sidebar has it), ahead/files, CI/PR, ready/done.
    const w = this.width ?? 1280;
    const left = [
      item("st-needs", [dot(needs ? k.blocked : k.border, needs > 0), div().when(needs > 0, (e) => e.text_color(k.blocked).font_medium()).child(`${needs} need${needs === 1 ? "s" : ""} you`)], {
        onClick: (_e, cx) => this.command("next-waiting", cx),
        tooltip: "Next agent waiting  ⌘J",
      }),
      item("st-working", [count("working") ? Spinner({ size: 12, color: k.working }) : dot(k.border), div().child(`${count("working")} working`)], { onClick: () => show("home"), tooltip: "Home  ⌘⌥0" }),
      w >= 640 && item("st-ready", [dot(ready ? k.success : k.border), div().child(`${ready} ready`)], { onClick: () => show("home"), tooltip: "Agents whose PR is approved and green" }),
      w >= 640 && item("st-done", [dot(count("done") ? k.done : k.border), div().child(`${count("done")} done`)], { onClick: (_e, cx) => this.command("next:done", cx), tooltip: "Next finished agent  ⌘⇧U" }),
    ].filter(Boolean);

    // The focused agent's branch and links on the right, always in the hub's order (PR, ticket, CI, deploy, cited pages);
    // past the room left, the rest in a "+N" menu. Documents come last.
    const right = [];
    const open = (url) => (url ? (_e, cx) => cx.spawn(async () => hub.openUrl(url)) : undefined);
    if (me) {
      const ch = me.changes ?? {};
      // The branch only says where the agent works (its PR is a link of its own); it gives way first when room runs out.
      right.push(item("st-branch", [Icon("git-branch", { size: 14 }), Mono(me.branch ?? me.space_name ?? me.address, { size: TEXT.sm }).flex_shrink(1).min_w(0).truncate()], { tooltip: `${me.title} · ${me.branch ?? ""}` }).flex_shrink(1).min_w(60));
      if (w >= 900 && (ch.ahead || ch.behind)) right.push(item("st-ahead", [Mono(`↑${ch.ahead ?? 0} ↓${ch.behind ?? 0}`, { size: TEXT.sm })], { tooltip: "Commits ahead of and behind its base" }));
      if (w >= 900 && ch.files) right.push(item("st-files", [Mono(`±${ch.files}`, { size: TEXT.sm })], { tooltip: `${ch.files} files changed`, onClick: () => togglePanel("changes") }));
      const links = me.row?.links ?? [];
      // As many links as the bar has room for beside the counts, the branch and the documents, by their labels' width.
      const docsCount = me.row?.documents?.length ?? 0;
      let budget = w - 440 - 140 - (w >= 900 ? 110 : 0) - (docsCount ? (w >= 800 ? 150 : 60) : 0);
      const wide = (l) => 46 + (l.label ?? l.name).length * 7.5;
      const fits = links.reduce((n, l, i) => (n === i && (budget -= wide(l)) >= 0 ? n + 1 : n), 0);
      const shown = fits >= links.length ? links : links.slice(0, Math.max(1, fits - 1));
      const ciColor = (state) => (state === "pass" ? k.success : state === "fail" ? k.destructive : k.warning);
      shown.forEach((l, i) => {
        const mark = l.kind === "ci" ? dot(ciColor(l.state), l.state === "running") : Icon(l.icon || "link", { size: 14 });
        right.push(item(`st-link-${i}`, [mark, div().child(l.label ?? l.name)], { tooltip: l.detail ? `${l.name} · ${l.detail}` : l.name, onClick: open(l.url), button: true }));
      });
      const rest = links.slice(shown.length);
      if (rest.length)
        right.push(
          item("st-links-more", [div().child(`+${rest.length}`)], {
            tooltip: rest.map((l) => l.label ?? l.name).join(", "),
          button: true,
            onClick: () => openMenu(rest.map((l) => ({ label: l.detail ? `${l.label ?? l.name} · ${l.detail}` : l.label ?? l.name, icon: l.icon || "link", command: `link:${l.url}` })), { width: 320 }),
          }),
        );
    }
    if (queued) right.push(item("st-queue", [Icon("list-ordered", { size: 14, color: k.info }), div().child(w >= 800 ? `${queued} queued` : String(queued))], { tooltip: "Prompts waiting to be sent  ⌘⌥Q", onClick: () => run("cockpit.queue", {}) }));
    if (lost) right.push(item("st-lost", [Icon("rotate-ccw", { size: 14, color: k.info }), div().text_color(k.info).child(`${lost} to resume`)], { tooltip: "Agents a restart stopped: resume them  ⌘⌥R", onClick: (_e, cx) => cx.spawn(async () => hub.act("home", "resume", [])) }));
    // The hub shows only when it does not answer.
    if (this.state !== "live") right.push(item("st-hub", [dot(this.state === "connecting" ? k.warning : k.destructive), div().child(this.state === "connecting" ? "connecting" : "hub offline")], { tooltip: this.state }));
    // The agent's documents always close the bar, so they sit at the same place whatever else shows.
    const docs = me?.row?.documents ?? [];
    if (docs.length)
      right.push(
        item("st-docs", [Icon("layers", { size: 14 }), div().child(w >= 800 ? `Documents · ${docs.length}` : String(docs.length))], {
          tooltip: "What this agent showed beside itself",
          button: true,
          onClick: (_e, cx) => (docs.length === 1 ? this.command(`doc:${docs[0].name}`, cx) : openMenu(docs.map((d) => ({ label: d.title || d.name, icon: "layers", command: `doc:${d.name}` })), { width: 280 })),
        }),
      );

    return h_flex()
      .id("cockpit-status")
      .size_full()
      .on_size_change((size, cx) => {
        if (Math.abs((this.width ?? 0) - size.width) < 1) return;
        this.width = size.width;
        cx.notify();
      })
      .px(12)
      .gap(6)
      .items_center()
      .bg(k.chrome)
      .border_t_1()
      .border_color(k.border)
      .children(left)
      .child(div().flex_1().min_w(8))
      .children(right);
  }
}
