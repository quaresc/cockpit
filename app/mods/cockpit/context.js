// The Context page of the side panel: what the focused agent's session holds in its context window, as Claude's own /context
// shows it (the tokens in use of the window, a colored bar, each part with its tokens and share, the free space), plus the
// session's biggest tool outputs. Read by herdr/helpers/cockpit-session-context.py from the agent's own session data, on its machine.

import { div } from "gpui-kit";
import { h_flex, v_flex } from "gpui-base";
import { focused, agents, exec, readTextFile } from "herdr";
import * as hub from "./hub.js";
import { tokens, TEXT, Empty, Mono, Progress, Spinner } from "./ui.js";
import { sessionFor, machineOf, helperSource } from "./agentsui.js";

const COLORS = {
  Messages: "#5b8def",
  Reasoning: "#a278e0",
  Shell: "#e0803f",
  Files: "#3fb4d6",
  "Web and search": "#4fae93",
  "MCP tools": "#d6a03c",
  "Other tools": "#8a8f98",
  "System, tools and memory": "#c3c7cf",
};
const FREE = "#2c2f36";
const k1 = (n) => (n >= 1e6 ? `${(n / 1e6).toFixed(1)}M` : n >= 1000 ? `${(n / 1000).toFixed(n >= 10000 ? 0 : 1)}k` : `${n ?? 0}`);

export default class Context {
  init(_p, cx) {
    this.rows = null;
    this.address = null;
    this.data = null;
    this.error = null;
    this.pending = false;
    this.sub = hub.subscribe(
      cx,
      { views: ["agents"] },
      (m, cx) => {
        if (m.view === "agents") this.rows = m.value;
        cx.notify();
      },
      () => {},
    );
    this.pull(cx);
    cx.timer.every(6000, (cx) => this.pull(cx));
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

  pull(cx) {
    const row = this.current();
    if (!row || this.pending) return;
    const session = sessionFor(row);
    if (!session) {
      this.data = null;
      this.error = "No session known for this agent yet.";
      cx.notify();
      return;
    }
    const program = String(row.program ?? agents().find((a) => a.address === row.address)?.app ?? "opencode").toLowerCase();
    const machine = machineOf(row.address);
    this.pending = true;
    cx.spawn(async (cx) => {
      try {
        const { home } = await hub.install();
        const cfg = JSON.parse((await readTextFile(`${home}/.config/cockpit/config.json`).catch(() => null)) ?? "{}");
        const args = [program, session, "--limits", JSON.stringify(cfg.context?.limits ?? {})];
        const r = machine
          ? await exec("python3", ["-c", await helperSource("cockpit-session-context.py"), ...args], { on: machine, timeout: 20000 })
          : await exec("python3", [`${home}/.local/share/cockpit/herdr/helpers/cockpit-session-context.py`, ...args], { timeout: 20000 });
        this.data = { ...JSON.parse(r.stdout || "null"), agent: row.title ?? row.address, machine };
        this.error = null;
      } catch (e) {
        this.error = String(e?.message ?? e).slice(0, 160);
      }
      this.pending = false;
      cx.notify();
    });
  }

  render() {
    const k = tokens();
    const d = this.data;
    const shell = (child) => v_flex().id("context").size_full().bg(k.background).text_color(k.foreground).text_size(TEXT.base).p(16).gap(14).overflow_y_scrollbar().child(child);
    if (!d?.used) {
      return shell(
        this.error ? Empty({ icon: "gauge", title: "No context to show", description: this.error }) : h_flex().h(120).justify_center().items_center().gap(8).text_color(k.mutedFg).child(Spinner({ size: 14 })).child("Reading the focused agent's session…"),
      );
    }
    const used = d.used;
    const window = d.window ?? used;
    const free = Math.max(0, window - used);
    const pct = (n) => `${((n * 100) / window).toFixed(1)}%`;
    const row = (id, name, color, tokensN, share) =>
      h_flex().id(id).gap(10).items_center().child(div().size(12).flex_none().rounded(3).bg(color)).child(div().flex_1().min_w(0).truncate().text_size(TEXT.sm).text_color(k.foreground).child(name)).child(div().w(52).flex_none().text_right().child(Mono(k1(tokensN), { size: TEXT.sm, color: k.mutedFg }))).child(div().w(50).flex_none().text_right().child(Mono(share, { size: TEXT.sm, color: k.foreground })));
    const maxTool = Math.max(1, ...d.tools.map((t) => t.tokens));
    return shell(
      v_flex()
        .gap(14)
        .child(
          v_flex()
            .gap(2)
            .child(div().text_size(TEXT.base).font_semibold().truncate().child(d.title ?? d.agent))
            .child(div().text_size(TEXT.xs).text_color(k.dim).truncate().child([d.agent, d.machine, d.model].filter(Boolean).join(" · "))),
        )
        .child(
          v_flex()
            .gap(8)
            .child(h_flex().items_end().child(div().flex_1().text_size(20).font_semibold().child(d.percent != null ? `${Math.round(d.percent)}% used` : `${k1(used)} used`)).child(Mono(`${k1(used)} / ${k1(window)}`, { color: k.mutedFg })))
            .child(
              h_flex()
                .id("ctx-bar")
                .w_full()
                .h(8)
                .gap(1)
                .rounded(4)
                .overflow_hidden()
                .bg(FREE)
                .children(d.parts.map((p) => div().h_full().w(`${Math.max(0.6, (p.tokens * 100) / window)}%`).bg(COLORS[p.name] ?? "#8a8f98"))),
            ),
        )
        .child(
          v_flex()
            .gap(8)
            .children(d.parts.map((p, i) => row(`ctx-p-${i}`, p.name, COLORS[p.name] ?? "#8a8f98", p.tokens, pct(p.tokens))))
            .child(row("ctx-free", "Free space", FREE, free, pct(free))),
        )
        .when(Boolean(d.note), (e) => e.child(div().text_size(TEXT.xs).text_color(k.dim).child(`Estimated from the transcript; ${d.note}.`)))
        .when(d.tools.length > 0, (e) =>
          e.child(
            v_flex()
              .gap(6)
              .child(div().text_size(TEXT.sm).font_semibold().child("Biggest tool outputs"))
              .children(
                d.tools.map((t, i) =>
                  v_flex()
                    .id(`ctx-t-${i}`)
                    .gap(3)
                    .tooltip(`${t.name}: ${t.calls} calls, about ${k1(t.tokens)} tokens`)
                    .child(h_flex().gap(8).child(div().flex_1().min_w(0).truncate().text_size(TEXT.xs).text_color(k.mutedFg).child(t.name)).child(div().text_size(TEXT.xs).text_color(k.dim).child(`${t.calls}×`)).child(Mono(k1(t.tokens), { size: TEXT.xs, color: k.foreground })))
                    .child(Progress(t.tokens / maxTool, { color: COLORS["Other tools"], h: 3 })),
                ),
              ),
          ),
        )
        .child(div().text_size(TEXT.xs).text_color(k.dim).child([d.turns ? `${d.turns} prompts` : null, d.cost ? `$${d.cost.toFixed(2)}` : null, "parts estimated at 4 characters a token"].filter(Boolean).join(" · "))),
    );
  }
}
