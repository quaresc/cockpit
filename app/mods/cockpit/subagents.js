// The Subagents page of the side panel: the focused agent's sub-agents (OpenCode child sessions, Claude Code Task agents),
// working ones first, and, for the one picked, what it did: its prompt, its text and the tools it ran with their output
// (herdr/helpers/cockpit-subagent-output.py, read on the agent's own machine). A working sub-agent's output follows it live.

import { div } from "gpui-kit";
import { h_flex, v_flex } from "gpui-base";
import { focused, agents, exec } from "herdr";
import * as hub from "./hub.js";
import { tokens, alpha, TEXT, RADIUS, Empty, Mono, Spinner, dur } from "./ui.js";
import { SubagentsModel, localSessions, sessionFor, machineOf, helperSource } from "./agentsui.js";

export default class Subagents {
  init(_p, cx) {
    this.rows = null;
    this.address = null;
    this.sel = null;
    this.out = null;
    this.error = null;
    this.pending = false;
    this.subs = new SubagentsModel();
    // Every agent's sub-agents, as the badge on the rail counts them; the focused agent's come first.
    this.subs.follow(cx, () => localSessions(this.rows?.agents));
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
    if (hit) this.address = hit.address;
    return rows.find((r) => r.address === this.address) ?? null;
  }

  /** The agents that have sub-agents, the focused one first: [{row, list}], each list with the working ones first. */
  groups() {
    const rows = this.rows?.agents ?? [];
    this.current();
    return rows
      .map((row) => {
        const session = sessionFor(row);
        const all = (session && this.subs.by[session]) || [];
        const list = [...all].sort((x, y) => (x.status === "working" ? 0 : 1) - (y.status === "working" ? 0 : 1) || (y.since ?? 0) - (x.since ?? 0));
        return { row, list };
      })
      .filter((g) => g.list.length)
      .sort((x, y) => (y.row.address === this.address) - (x.row.address === this.address) || y.list.filter((s) => s.status === "working").length - x.list.filter((s) => s.status === "working").length);
  }

  pick(cx, row, s) {
    this.sel = { id: s.id, address: row.address };
    this.out = null;
    this.error = null;
    this.refresh(cx, true);
    cx.notify();
  }

  /** Reads the picked sub-agent's output: again while it works, once when it has ended. */
  refresh(cx, force = false) {
    const g = this.groups().find((x) => x.row.address === this.sel?.address);
    const s = g?.list.find((x) => x.id === this.sel?.id);
    if (!g || !s || this.pending) return;
    if (!force && this.out?.id === s.id && s.status !== "working" && this.out.at > (s.until ?? 0)) return;
    const row = g.row;
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
    const now = Date.now();
    const shell = (...kids) => v_flex().id("subagents").size_full().bg(k.background).text_color(k.foreground).text_size(TEXT.base).p(14).gap(10).children(kids.filter(Boolean));
    if (!row) return shell(Empty({ icon: "git-fork", title: "No agent in focus", description: "Click an agent to see its sub-agents here." }));
    const session = sessionFor(row);
    const known = Boolean(session) && session in this.subs.by;
    const list = this.groups().find((g) => g.row.address === row.address)?.list ?? [];
    const working = list.filter((s) => s.status === "working").length;
    // The page is for the agent in focus: its own count, zero included. The others are one line below.
    const others = this.groups().filter((g) => g.row.address !== row.address);
    const head = v_flex()
      .gap(2)
      .child(div().text_size(TEXT.sm).font_semibold().truncate().child(row.title ?? row.address))
      .child(h_flex().gap(8).items_baseline().child(div().text_size(20).font_semibold().text_color(list.length ? k.foreground : k.dim).child(String(list.length))).child(div().text_size(TEXT.sm).text_color(k.mutedFg).child(`sub-agent${list.length === 1 ? "" : "s"}${working ? ` · ${working} working` : ""}`)));
    const elsewhere = others.length
      ? div().text_size(TEXT.xs).text_color(k.dim).child(`Other agents: ${others.map((g) => `${g.row.title ?? g.row.address} (${g.list.length}${g.list.some((s) => s.status === "working") ? ", working" : ""})`).join(" · ")}`)
      : null;
    if (!list.length) {
      return shell(head, known || !session ? div().text_size(TEXT.sm).text_color(k.mutedFg).child(session ? "This agent has not started any." : "No session known for this agent yet.") : h_flex().gap(8).items_center().text_color(k.mutedFg).child(Spinner({ size: 14 })).child("Looking for sub-agents…"), elsewhere);
    }
    const picked = this.sel?.address === row.address ? list.find((s) => s.id === this.sel?.id) : null;
    return shell(
      head,
      v_flex()
        .id("sa-list")
        .gap(2)
        .max_h(260)
        .overflow_y_scrollbar()
        .children(
          list.map((s) =>
            h_flex()
              .id(`sa-${row.address}-${s.id}`)
              .h(28)
              .px(8)
              .gap(8)
              .items_center()
              .rounded(RADIUS.md)
              .cursor_pointer()
              .when(s.id === picked?.id, (e) => e.bg(k.selected))
              .when(s.id !== picked?.id, (e) => e.hover((x) => x.bg(k.accent)))
              .on_click((_e, cx) => this.pick(cx, row, s))
              .child(div().flex_none().px(5).rounded(RADIUS.sm).bg(alpha(s.status === "failed" ? k.destructive : k.info, 0.14)).text_size(10).text_color(s.status === "failed" ? k.destructive : k.info).child(s.kind ?? "agent"))
              .child(div().flex_1().min_w(0).truncate().text_size(TEXT.sm).text_color(s.status === "working" ? k.foreground : k.mutedFg).child(s.name))
              .child(div().flex_none().text_size(TEXT.xs).text_color(s.status === "working" ? k.working : k.dim).child(s.status === "working" ? dur(now - (s.since ?? now)) : s.status)),
          ),
        ),
      elsewhere,
      div().h(1).bg(k.border),
      picked
        ? v_flex()
            .id("sa-out")
            .flex_1()
            .min_h(0)
            .gap(6)
            .overflow_y_scrollbar()
            .child(h_flex().gap(8).items_center().child(div().flex_1().min_w(0).truncate().text_size(TEXT.sm).font_semibold().child(this.out?.title ?? picked.name)).when(picked.status === "working", (e) => e.child(Spinner({ size: 12, color: k.working }))))
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
