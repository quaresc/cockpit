// The Subagents page of the side panel: the focused agent's sub-agents (OpenCode child sessions, Claude Code Task agents),
// working ones first, and, for the one picked, what it did: its prompt, its text and the tools it ran with their output
// (herdr/helpers/cockpit-subagent-output.py, read on the agent's own machine). A working sub-agent's output follows it live.

import { div } from "gpui-kit";
import { h_flex, v_flex } from "gpui-base";
import { focused, agents, exec } from "herdr";
import * as hub from "./hub.js";
import { tokens, alpha, TEXT, RADIUS, Empty, Mono, Spinner, dur } from "./ui.js";
import { SubagentsModel, sessionFor, machineOf, helperSource } from "./agentsui.js";

export default class Subagents {
  init(_p, cx) {
    this.rows = null;
    this.address = null;
    this.sel = null;
    this.out = null;
    this.error = null;
    this.pending = false;
    this.subs = new SubagentsModel();
    this.subs.follow(cx, () => {
      const row = this.current();
      const session = row && sessionFor(row);
      return session ? [{ session, machine: machineOf(row.address) }] : [];
    });
    this.sub = hub.subscribe(
      cx,
      { views: ["agents"] },
      (m, cx) => {
        if (m.view === "agents") this.rows = m.value;
        cx.notify();
      },
      () => {},
    );
    cx.timer.every(3000, (cx) => this.refresh(cx));
  }

  close() {
    this.sub?.stop();
  }

  command() {}

  /** The agent in focus (kept while the focus is on the panel itself). */
  current() {
    const rows = this.rows?.agents ?? [];
    const f = focused()?.terminal ?? agents().find((a) => a.focused)?.address;
    const hit = rows.find((r) => r.address === f);
    if (hit && hit.address !== this.address) {
      this.address = hit.address;
      this.sel = null;
      this.out = null;
    }
    return rows.find((r) => r.address === this.address) ?? null;
  }

  list() {
    const row = this.current();
    const session = row && sessionFor(row);
    const all = (session && this.subs.by[session]) || [];
    return [...all].sort((a, b) => (a.status === "working" ? 0 : 1) - (b.status === "working" ? 0 : 1) || (b.since ?? 0) - (a.since ?? 0));
  }

  pick(cx, s) {
    this.sel = s.id;
    this.out = null;
    this.error = null;
    this.refresh(cx, true);
    cx.notify();
  }

  /** Reads the picked sub-agent's output: again while it works, once when it has ended. */
  refresh(cx, force = false) {
    const row = this.current();
    const s = this.list().find((x) => x.id === this.sel);
    if (!row || !s || this.pending) return;
    if (!force && this.out?.id === s.id && s.status !== "working" && this.out.at > (s.until ?? 0)) return;
    const program = String(row.program ?? agents().find((a) => a.address === row.address)?.app ?? "opencode").toLowerCase();
    const machine = machineOf(row.address);
    this.pending = true;
    cx.spawn(async (cx) => {
      try {
        const { home } = await hub.install();
        const r = machine
          ? await exec("python3", ["-c", await helperSource("cockpit-subagent-output.py"), program, s.id], { on: machine, timeout: 20000 })
          : await exec("python3", [`${home}/.local/share/cockpit/herdr/helpers/cockpit-subagent-output.py`, program, s.id], { timeout: 15000 });
        const d = JSON.parse(r.stdout || "null");
        if (d?.error) this.error = d.error;
        else if (d) this.out = { ...d, at: Date.now() };
      } catch (e) {
        this.error = String(e?.message ?? e).slice(0, 140);
      }
      this.pending = false;
      cx.notify();
    });
  }

  item(k, it, i) {
    const body = (color, italic = false) => div().text_size(TEXT.xs).font_family("JetBrains Mono").text_color(color).whitespace_normal().when(italic, (e) => e.italic()).child(it.text);
    if (it.kind === "tool")
      return v_flex().id(`sa-i-${i}`).gap(3).p(8).rounded(RADIUS.md).bg(alpha(k.muted, 0.7)).child(div().text_size(TEXT.xs).font_semibold().text_color(k.mutedFg).child(it.name ?? "tool")).child(body(k.foreground));
    if (it.kind === "prompt") return v_flex().id(`sa-i-${i}`).gap(3).p(8).rounded(RADIUS.md).border_1().border_color(k.border).child(div().text_size(TEXT.xs).font_semibold().text_color(k.info).child("Prompt")).child(body(k.foreground));
    if (it.kind === "reasoning") return v_flex().id(`sa-i-${i}`).px(8).child(body(k.dim, true));
    return v_flex().id(`sa-i-${i}`).px(8).child(body(k.foreground));
  }

  render() {
    const k = tokens();
    const row = this.current();
    const list = this.list();
    const now = Date.now();
    const shell = (...kids) => v_flex().id("subagents").size_full().bg(k.background).text_color(k.foreground).text_size(TEXT.base).p(14).gap(10).children(kids);
    if (!row) return shell(Empty({ icon: "git-fork", title: "No agent in focus", description: "Click an agent to see its sub-agents here." }));
    if (!list.length) {
      const session = sessionFor(row);
      const known = session && session in this.subs.by;
      return shell(
        div().text_size(TEXT.sm).font_semibold().truncate().child(row.title ?? row.address),
        known ? Empty({ icon: "git-fork", title: "No sub-agents", description: "This agent has not started any." }) : h_flex().h(80).justify_center().items_center().gap(8).text_color(k.mutedFg).child(Spinner({ size: 14 })).child("Looking for sub-agents…"),
      );
    }
    const sel = list.find((s) => s.id === this.sel);
    const working = list.filter((s) => s.status === "working").length;
    return shell(
      v_flex().gap(2).child(div().text_size(TEXT.sm).font_semibold().truncate().child(row.title ?? row.address)).child(div().text_size(TEXT.xs).text_color(k.dim).child(`${list.length} sub-agent${list.length === 1 ? "" : "s"}${working ? ` · ${working} working` : ""}`)),
      v_flex()
        .id("sa-list")
        .gap(2)
        .max_h(230)
        .overflow_y_scrollbar()
        .children(
          list.map((s) =>
            h_flex()
              .id(`sa-${s.id}`)
              .h(30)
              .px(8)
              .gap(8)
              .items_center()
              .rounded(RADIUS.md)
              .cursor_pointer()
              .when(s.id === this.sel, (e) => e.bg(k.selected))
              .when(s.id !== this.sel, (e) => e.hover((x) => x.bg(k.accent)))
              .on_click((_e, cx) => this.pick(cx, s))
              .child(div().flex_none().px(5).rounded(RADIUS.sm).bg(alpha(s.status === "failed" ? k.destructive : k.info, 0.14)).text_size(10).text_color(s.status === "failed" ? k.destructive : k.info).child(s.kind ?? "agent"))
              .child(div().flex_1().min_w(0).truncate().text_size(TEXT.sm).text_color(s.status === "working" ? k.foreground : k.mutedFg).child(s.name))
              .child(div().flex_none().text_size(TEXT.xs).text_color(s.status === "working" ? k.working : k.dim).child(s.status === "working" ? dur(now - (s.since ?? now)) : s.status)),
          ),
        ),
      div().h(1).bg(k.border),
      sel
        ? v_flex()
            .id("sa-out")
            .flex_1()
            .min_h(0)
            .gap(6)
            .overflow_y_scrollbar()
            .child(h_flex().gap(8).items_center().child(div().flex_1().min_w(0).truncate().text_size(TEXT.sm).font_semibold().child(this.out?.title ?? sel.name)).when(sel.status === "working", (e) => e.child(Spinner({ size: 12, color: k.working }))))
            .when(Boolean(this.error), (e) => e.child(div().text_size(TEXT.xs).text_color(k.warning).child(this.error)))
            .when(!this.out && !this.error, (e) => e.child(div().text_size(TEXT.xs).text_color(k.dim).child("Reading…")))
            .when(Boolean(this.out), (e) =>
              e
                .child(div().text_size(TEXT.xs).text_color(k.dim).child(`${this.out.total} steps${this.out.total > this.out.items.length ? `, the last ${this.out.items.length} shown` : ""} · newest first`))
                .children([...this.out.items].reverse().map((it, i) => this.item(k, it, i))),
            )
        : div().text_size(TEXT.xs).text_color(k.dim).child("Click a sub-agent to read what it did."),
    );
  }
}
