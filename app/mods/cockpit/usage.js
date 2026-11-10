// The Usage tab: each agent account's real rate limits (Claude Code, Codex, Antigravity), the 5-hour session window and
// the 7-day week, with % used, a tick where the window's time stands, the pace, a projection and the reset countdown.
// herdr/helpers/cockpit-usage-sync.py asks each agent (cached, so cheap to call) and writes ~/.cache/cockpit/agent-usage.json;
// Home's Usage & Limits card and the sidebar's badge draw the same data through followUsage and AgentLimits.
// Context windows are not limits: they stay on Home's agent cards.

import { div } from "gpui-kit";
import { h_flex, v_flex } from "gpui-base";
import { exec, readTextFile, setShared } from "herdr";
import * as hub from "./hub.js";
import { openCode } from "./code.js";
import { tokens, alpha, TEXT, RADIUS, Icon, Badge, Button, Card, PageHeader, Empty, Mono, Spinner, dur } from "./ui.js";

const WINDOW_LABEL = { "5h": "5-hour session", "7d": "Weekly" };

/** Runs the sync (it answers from its cache while fresh) and reads what it wrote. force asks every agent again. */
export async function syncUsage({ force = false } = {}) {
  const { cache, home } = await hub.install();
  const script = `${home}/.local/share/cockpit/herdr/helpers/cockpit-usage-sync.py`;
  await exec("python3", force ? [script, "--force"] : [script], { timeout: 90000 }).catch(() => null);
  const text = await readTextFile(`${cache}/agent-usage.json`);
  return text ? JSON.parse(text) : null;
}

/** Keeps a view's usage fresh: now, then every `every` ms; also sets the sidebar's Usage badge. */
export function followUsage(cx, onData, { every = 15000 } = {}) {
  const pull = (cx, force) =>
    cx.spawn(async (cx) => {
      try {
        const data = await syncUsage({ force });
        if (data) {
          setShared("badge.usage", usageBadge(data));
          onData(data, cx);
        }
      } catch {}
      cx.notify();
    });
  pull(cx, false);
  cx.timer.every(every, (cx) => pull(cx, false));
  return { refresh: (cx) => pull(cx, true) };
}

/** The highest % among an agent's (or every agent's) official windows. */
export function worst(windows) {
  return Math.max(0, ...(windows ?? []).filter((w) => w.source !== "estimate" || w.budget_tokens).map((w) => w.percent ?? 0));
}

/** The sidebar badge: the worst window when it is at 80% or more. */
export function usageBadge(data) {
  const p = Math.round(worst((data?.agents ?? []).flatMap((a) => a.windows ?? [])));
  return p >= 80 ? { count: `${p}%`, tone: p >= 95 ? "destructive" : "warning" } : null;
}

export function toneOf(k, w) {
  const p = w.percent ?? 0;
  return p >= 95 ? k.destructive : p >= 80 || w.ahead ? k.warning : k.success;
}

/** When the window hits 100% at its current pace, in ms from now, if that comes before its reset; else null. */
export function projection(w) {
  if (!w.resets_at || w.elapsed == null || !w.percent || w.percent >= 100) return null;
  const spent = w.elapsed * w.window_mins * 60000;
  if (spent < 10 * 60000) return null;
  const toFull = ((100 - w.percent) / w.percent) * spent;
  return toFull < w.resets_at - Date.now() ? toFull : null;
}

const tokensText = (n) => (n >= 1e6 ? `${(n / 1e6).toFixed(1)}M` : n >= 1000 ? `${Math.round(n / 1000)}k` : `${n ?? 0}`);

/** One window as a bar: label, % used (▲ when ahead of pace), a tick at the time gone, and the reset countdown. */
export function LimitRow(id, w, { compact = false, now = Date.now() } = {}) {
  const k = tokens();
  const est = w.source === "estimate";
  const p = Math.max(0, Math.min(100, w.percent ?? 0));
  const tone = est && !w.budget_tokens ? k.mutedFg : toneOf(k, w);
  const left = w.resets_at ? dur(w.resets_at - now) : null;
  const full = projection(w);
  const value = est && !w.budget_tokens ? `${tokensText(w.used_tokens)} tok` : `${Math.round(p)}%`;
  const tip = [
    `${w.group ? `${w.group} · ` : ""}${WINDOW_LABEL[w.name] ?? w.name}: ${est && !w.budget_tokens ? `${tokensText(w.used_tokens)} tokens (no budget set)` : `${p}% used`}`,
    left ? `resets in ${left}` : null,
    w.elapsed != null ? `${Math.round(w.elapsed * 100)}% of the window gone${w.ahead ? ", ahead of pace" : ""}` : null,
    full ? `at this pace full in ~${dur(full)}` : null,
    est ? "estimated from local transcripts" : null,
  ]
    .filter(Boolean)
    .join(" · ");
  return h_flex()
    .id(id)
    .gap(8)
    .items_center()
    .tooltip(tip)
    .child(div().w(compact ? 20 : 26).flex_none().text_size(TEXT.xs).font_medium().text_color(k.mutedFg).child(w.name))
    .child(
      div()
        .relative()
        .flex_1()
        .h(compact ? 5 : 7)
        .rounded(4)
        .bg(k.muted)
        .child(div().absolute().left(0).top(0).h_full().rounded(4).bg(tone).w(`${est && !w.budget_tokens ? 0 : p}%`).transition("width", { duration: 400, easing: "ease-out" }))
        .when(w.elapsed != null, (e) => e.child(div().absolute().top(-2).h(compact ? 9 : 11).w(2).rounded(1).bg(k.foreground).opacity(0.6).left(`${Math.min(99, w.elapsed * 100)}%`))),
    )
    .child(
      h_flex()
        .w(compact ? 40 : 52)
        .flex_none()
        .justify_end()
        .gap(2)
        .text_size(TEXT.xs)
        .font_family("JetBrains Mono")
        .text_color(w.ahead || p >= 80 ? tone : k.foreground)
        .when(Boolean(w.ahead), (e) => e.child("▲"))
        .child(value),
    )
    .child(div().w(compact ? 30 : 40).flex_none().text_right().text_size(TEXT.xs).font_family("JetBrains Mono").text_color(k.dim).child(left ?? "—"));
}

/** An agent's windows, grouped by model group (Antigravity has three), under the agent's name. */
export function AgentLimits(id, a, { compact = false, now = Date.now() } = {}) {
  const k = tokens();
  const groups = [];
  for (const w of a.windows ?? []) {
    let g = groups.find((x) => x.name === (w.group ?? null));
    if (!g) groups.push((g = { name: w.group ?? null, windows: [] }));
    g.windows.push(w);
  }
  for (const g of groups) g.windows.sort((x, y) => (x.name === "5h" ? -1 : 1) - (y.name === "5h" ? -1 : 1));
  const badge = a.source === "estimate" ? "est." : a.stale ? "stale" : null;
  return v_flex()
    .id(id)
    .gap(compact ? 4 : 6)
    .child(
      h_flex()
        .gap(6)
        .items_center()
        .child(Icon(a.icon ?? "bot", { size: 13, color: k.mutedFg }))
        .child(div().flex_1().min_w(0).truncate().text_size(TEXT.sm).font_medium().text_color(k.foreground).child(a.name))
        .when(Boolean(badge), (e) => e.child(div().text_size(TEXT.xs).text_color(a.stale ? k.warning : k.dim).child(badge))),
    )
    .children(
      groups.map((g) =>
        v_flex()
          .gap(3)
          .when(Boolean(g.name), (e) => e.child(div().pl(1).text_size(TEXT.xs).text_color(k.dim).child(g.name)))
          .children(g.windows.map((w) => LimitRow(`${id}-${g.name ?? ""}-${w.name}`, w, { compact, now }))),
      ),
    );
}

export default class Usage {
  init(_props, cx) {
    this.data = null;
    this.now = Date.now();
    this.refreshing = false;
    this.follow = followUsage(cx, (data, cx) => {
      this.data = data;
      this.refreshing = false;
      this.now = Date.now();
    });
    cx.timer.every(30000, (cx) => {
      this.now = Date.now();
      cx.notify();
    });
  }

  command(c, cx) {
    if (c === "refresh") this.refresh(cx);
  }

  refresh(cx) {
    this.refreshing = true;
    this.follow.refresh(cx);
    cx.notify();
  }

  editBudgets(cx) {
    cx.spawn(async () => {
      const { home } = await hub.install();
      await openCode(`${home}/.config/cockpit/config.json`);
    });
  }

  /** A big card per agent: its windows, each with %, reset time, pace and projection spelled out. */
  agentCard(k, a) {
    const facts = [];
    for (const w of a.windows ?? []) {
      const full = projection(w);
      if (full) facts.push({ tone: k.warning, text: `${w.group ? `${w.group} ` : ""}${WINDOW_LABEL[w.name]}: at this pace full in ~${dur(full)}, before it resets in ${dur(w.resets_at - this.now)}` });
    }
    if (a.litellm?.spend != null) facts.push({ tone: k.mutedFg, text: `LiteLLM key ${a.litellm.key_alias ?? ""}: $${Number(a.litellm.spend).toFixed(2)}${a.litellm.max_budget ? ` of $${a.litellm.max_budget}` : ""}${a.litellm.budget_reset_at ? `, resets ${new Date(a.litellm.budget_reset_at).toLocaleString()}` : ""}` });
    if (a.source === "estimate") facts.push({ tone: k.dim, text: `No limits from ${a.name}${a.error ? ` (${a.error})` : ""}: tokens from local transcripts${worst(a.windows) ? " against usage.budgets" : "; set usage.budgets to see %"}.` });
    else if (a.stale) facts.push({ tone: k.warning, text: `Last answer ${a.fetched_at ? dur(this.now - a.fetched_at) : ""} ago: ${a.error ?? "the latest ask failed"}` });
    const ctx = a.context;
    return Card({
      id: `usage-card-${a.id}`,
      padding: 18,
      gap: 14,
      children: [
        AgentLimits(`usage-tab-${a.id}`, a, { now: this.now }),
        ...facts.map((f, i) => h_flex().id(`usage-fact-${a.id}-${i}`).gap(6).items_start().child(div().mt(5).size(5).rounded(3).flex_none().bg(f.tone)).child(div().flex_1().text_size(TEXT.sm).text_color(k.mutedFg).child(f.text))),
        h_flex()
          .gap(12)
          .items_center()
          .pt(2)
          .border_t_1()
          .border_color(k.border)
          .child(div().text_size(TEXT.xs).text_color(k.dim).child(a.fetched_at ? `asked ${dur(this.now - a.fetched_at)} ago` : "never asked"))
          .child(div().flex_1())
          .when(Boolean(ctx), (e) => e.child(div().text_size(TEXT.xs).text_color(k.dim).child(`latest session context ${Math.round(ctx.percent)}% of ${tokensText(ctx.window)}`))),
      ],
    });
  }

  render(cx) {
    const k = tokens();
    const agents = this.data?.agents ?? [];
    const top = Math.round(worst(agents.flatMap((a) => a.windows ?? [])));
    const body = !this.data
      ? h_flex().h(160).justify_center().items_center().gap(8).text_color(k.mutedFg).child(Spinner({ size: 14 })).child("Asking the agents for their limits…")
      : !agents.length
        ? Empty({ icon: "gauge", title: "No agent usage yet", description: "Claude Code, Codex and Antigravity show up once they are installed and signed in." })
        : div().w_full().flex().flex_row().flex_wrap().gap(16).children(agents.map((a) => div().min_w(320).flex_1().child(this.agentCard(k, a))));
    return v_flex()
      .id("usage")
      .size_full()
      .bg(k.background)
      .text_color(k.foreground)
      .text_size(TEXT.base)
      .child(
        PageHeader({
          icon: "gauge",
          title: "Usage & Limits",
          description: "5-hour session and weekly limits per agent account · the tick marks how much of the window has gone",
          aside: this.data ? Badge(top ? `highest ${top}%` : "all clear", top >= 95 ? "destructive" : top >= 80 ? "warning" : "success", { dot: top >= 95 ? k.destructive : top >= 80 ? k.warning : k.success }) : null,
          actions: [
            Button("usage-budgets", { label: "Budgets", icon: "file-text", variant: "ghost", size: "sm", tooltip: "Edit usage.budgets (for estimated windows) in the cockpit config", onClick: (_e, cx) => this.editBudgets(cx) }),
            Button("usage-refresh", { label: this.refreshing ? "Asking…" : "Refresh", icon: "refresh-cw", variant: "outline", size: "sm", disabled: this.refreshing, onClick: (_e, cx) => this.refresh(cx) }),
          ],
        }),
      )
      .child(
        v_flex()
          .id("usage-scroll")
          .flex_1()
          .min_h(0)
          .px(24)
          .py(20)
          .gap(16)
          .overflow_y_scrollbar()
          .child(body)
          .when(Boolean(this.data?.updated_at), (e) => e.child(div().text_size(TEXT.xs).text_color(k.dim).child(`Updated ${dur(this.now - this.data.updated_at)} ago · Claude from Anthropic's usage endpoint, Codex from its app-server, Antigravity from agy /quota`))),
      );
  }
}
