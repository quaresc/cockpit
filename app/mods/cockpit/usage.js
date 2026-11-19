// Every agent's limits and token use, from the hub's `usage` view (see docs/app.md): each agent under its logo with its
// state and account, read-only, its windows beneath with where each reading comes from and how fresh it is, and the last
// seven days' tokens with what they would cost at API prices, always marked as an estimate. Opened by a quota bar, Home's
// spend tile, or the palette's "Usage and limits"; esc closes it.

import { div } from "gpui-kit";
import { h_flex, v_flex } from "gpui-base";
import { props, openOverlay, closeOverlay } from "herdr";
import * as hub from "./hub.js";
import { ago } from "./fresh.js";
import { tokens, alpha, TEXT, RADIUS, Icon, Button, Badge, Spinner, KeyHints, PageHeader, Toaster, HubDown, sourceKey, runSource, AgentLogo, iconOf } from "./ui.js";

export function openUsage() {
  openOverlay({ mod: "cockpit", props: { app: "usage" }, kind: "dialog", key: "cockpit-usage", width: 680, height: 0.88 });
}

// ---------------------------------------------------------------------------------------------------------------------
// Words and figures, the same on every surface

/** "~$1.84", "~$0.004": a cost cockpit estimated, never billed as such; "–" when there is none. */
export function estimate(usd) {
  if (usd === null || usd === undefined) return "–";
  return usd > 0 && usd < 0.01 ? `~$${usd.toFixed(3)}` : usd >= 100 ? `~$${Math.round(usd)}` : `~$${usd.toFixed(2)}`;
}

/** A day's or a week's cost: its estimate, "no price" when none of its tokens has one, "·" when it has none. */
export function price(t) {
  if (!t?.total) return "·";
  return t.unpriced === t.total && !t.cost ? "no price" : estimate(t.cost);
}

/** "1.2M", "34K", "950": a token count for a narrow column. */
export function count(n) {
  const v = n ?? 0;
  if (v < 1000) return String(v);
  if (v < 1_000_000) return `${Math.round(v / 1000)}K`;
  return v < 10_000_000 ? `${(v / 1e6).toFixed(1)}M` : `${Math.round(v / 1e6)}M`;
}

/** What a window reads as, the hub's `figure`: "full" once its limit is reached, its percent ("≥12%" while a stale
 *  reading is a floor), else its verdict in a word; never a number the source did not give. */
export function valueWords(q) {
  if (q?.figure) return q.figure;
  if (q?.status === "rejected") return "full";
  if (q?.percent !== null && q?.percent !== undefined) return `${q.floor ? "≥" : ""}${q.percent}%`;
  return q?.status === "allowed_warning" ? "near" : "ok";
}

/** How full a window is, 0 to 100: a limit reached is full whatever percent came before it. */
function fullness(q) {
  if (q?.status === "rejected") return 100;
  return q?.percent ?? (q?.status === "allowed_warning" ? 80 : 0);
}

/** The tone of a bar: red when reached or nearly spent, amber past 80 % or ahead of pace, else green. */
export function barTone(q) {
  if (fullness(q) >= 95) return "destructive";
  return fullness(q) >= 80 || q?.warn || q?.ahead ? "warning" : "success";
}

/** The window nearest its limit, or null. */
export function tightestOf(quota) {
  return [...(quota ?? [])].sort((a, b) => fullness(b) - fullness(a) || (a.resets_at ?? Infinity) - (b.resets_at ?? Infinity))[0] ?? null;
}

/** The windows grouped by provider, with source capability kept separate from a missing reading. */
export function byProvider(quota, running = []) {
  const groups = [];
  for (const q of quota ?? []) {
    const agent = q.agent ?? q.provider ?? "";
    if (!agent || agent === "agent") continue;
    let g = groups.find((x) => x.agent === agent);
    if (!g) groups.push((g = { agent, icon: iconOf(q.icon ?? "agent"), name: q.provider ?? agent, windows: [] }));
    g.windows.push(q);
  }
  for (const r of running ?? []) {
    const agent = r.agent || (typeof r.program === "string" ? r.program : r.program?.key) || "";
    const icon = iconOf(r);
    if (!agent || agent === "agent" || groups.some((g) => g.agent === agent || (icon !== "agent" && g.icon === icon))) continue;
    groups.push({ agent, icon, program: r.program, name: r.agent_name ?? r.program?.name ?? agent, windows: [], unreported: r.reports_limits === false, unlimited: r.limits === "unlimited" });
  }
  for (const g of groups) g.tightest = tightestOf(g.windows);
  return groups;
}

/** When a window was read, as every freshness chip says it: "as of 12:04", or "stale, 40 min ago" once it is stale. */
export const asOfWords = (q, now = Date.now()) => (q.state !== "stale" ? q.as_of_words : `stale, ${q.as_of ? ago(q.as_of, now) : q.as_of_words}`);

/** One agent's windows, a line each, for a tooltip: "Claude 5h 64% · resets in 1 h 35 min · as of 12:04". */
export function windowsWords(quota, agent) {
  const mine = (quota ?? []).filter((q) => !agent || q.agent === agent);
  if (!mine.length) return "";
  return mine.map((q) => [`${q.label} ${valueWords(q)}`, q.resets_words, asOfWords(q)].filter(Boolean).join(" · ")).join("\n");
}

/** Home's spend tile: today's value and the week under it, both estimates, saying what they cover when some tokens
 *  have no price ("Claude only"); null when nothing has one. */
export function spendTile(spend) {
  if (!spend) return null;
  const covers = typeof spend.covers === "string" && spend.covers ? ` · ${spend.covers}` : "";
  return { value: estimate(spend.today ?? 0), hint: `${estimate(spend.week ?? 0)} these 7 days, estimated${covers}` };
}

const STATE_WORDS = { fresh: "Up to date", stale: "Stale", none: "No recent reading", unreported: "Not reported", unavailable: "Usage unavailable" };

// ---------------------------------------------------------------------------------------------------------------------
// A bar

/** A window as a labelled bar with its pace tick: dimmed when stale, amber past 80 %, red once reached, its tooltip
 *  saying where the reading comes from and when it was made. `mono` draws it in the text's colour, its figure in words. */
export function QuotaRow(q, { id, labelW = 64, compact = false, onClick, tooltip, label, mono = false } = {}) {
  const k = tokens();
  const tone = mono ? alpha(k.foreground, 0.6) : (k[barTone(q)] ?? k.success);
  const p = Math.max(0, Math.min(100, fullness(q)));
  const known = (q.percent !== null && q.percent !== undefined) || q.status === "rejected";
  const stale = q.state === "stale";
  let row = h_flex()
    .id(id)
    .gap(8)
    .items_center()
    .when(stale, (e) => e.opacity(0.5))
    .tooltip(tooltip ?? q.tooltip ?? `${q.label}: ${valueWords(q)}`)
    .child(div().w(labelW).flex_none().truncate().text_size(TEXT.xs).font_medium().text_color(k.mutedFg).child(label ?? q.label ?? q.name))
    .child(
      div()
        .relative()
        .flex_1()
        .h(6)
        .rounded(3)
        .bg(k.muted)
        .when(known, (e) => e.child(div().absolute().left(0).top(0).h_full().rounded(3).bg(tone).w(`${p}%`).transition("width", { duration: 400, easing: "ease-out" })))
        .when(!known, (e) => e.border_1().border_color(alpha(tone, 0.6)))
        .when(q.elapsed !== null && q.elapsed !== undefined, (e) => e.child(div().absolute().top(-2).h(10).w(2).rounded(1).bg(k.foreground).opacity(0.7).left(`${Math.min(99, q.elapsed * 100)}%`))),
    )
    .child(
      h_flex()
        .w(compact ? 38 : 46)
        .flex_none()
        .justify_end()
        .gap(2)
        .text_size(TEXT.xs)
        .font_family("JetBrains Mono")
        .text_color(!mono && (q.ahead || q.warn) ? tone : k.mutedFg)
        .when(Boolean(q.ahead), (e) => e.child("▲"))
        .child(valueWords(q)),
    );
  if (onClick) row = row.cursor_pointer().on_click(onClick);
  return row;
}

/** "OpenCode", "OpenCode and Gemini", "OpenCode, Gemini and Kiro". */
function names(list) {
  return list.length < 2 ? (list[0] ?? "") : `${list.slice(0, -1).join(", ")} and ${list.at(-1)}`;
}

/** Each provider's logo and limits, with separate words for readings not received yet, sources not read here and plans with no limit. */
export function QuotaGroups(quota, { id = "quota", rows = [], compact = false, mono = false, tightest = false, onClick = () => openUsage() } = {}) {
  const k = tokens();
  const all = byProvider(quota, rows);
  const groups = all.filter((g) => g.windows.length);
  const silent = all.filter((g) => !g.windows.length);
  if (!all.length) return null;
  const labelW = Math.max(compact ? 36 : 44, ...groups.flatMap((g) => g.windows.map((q) => Math.ceil(String(q.name ?? "").length * 6.2))));
  const size = compact ? 12 : 13;
  const logo = (g, i) => AgentLogo(g, { size, color: mono ? k.mutedFg : undefined, tooltip: g.name, id: `${id}-${i}-logo` });
  const lines = groups.map((g, i) => {
    if (tightest) {
      return h_flex()
        .id(`${id}-${i}`)
        .gap(6)
        .items_center()
        .child(logo(g, i))
        .child(div().flex_1().min_w(0).child(QuotaRow(g.tightest, { id: `${id}-${i}-bar`, labelW, compact: true, mono, label: g.tightest.name, tooltip: windowsWords(g.windows), onClick })));
    }
    return v_flex()
      .gap(compact ? 3 : 4)
      .child(h_flex().id(`${id}-${i}`).gap(6).items_center().child(logo(g, i)).child(div().flex_1().min_w(0).truncate().text_size(TEXT.xs).font_medium().text_color(k.mutedFg).child(g.name)))
      .children(
        g.windows.map((q, j) =>
          div()
            .pl(size + 6)
            .child(QuotaRow(q, { id: `${id}-${i}-${q.name ?? j}`, labelW, compact, mono, label: q.name, tooltip: q.tooltip ? `${g.name} ${q.name}: ${q.tooltip}` : undefined, onClick })),
        ),
      );
  });
  const why = (g) => (g.unreported ? "unreported" : g.unlimited ? "unlimited" : "waiting");
  for (const [kind, words] of [["waiting", "no reading yet"], ["unlimited", "unlimited"], ["unreported", "not reported"]]) {
    const missing = silent.filter((g) => why(g) === kind);
    if (!missing.length) continue;
    const who = names(missing.map((g) => g.name));
    lines.push(
      h_flex()
        .id(`${id}-${kind}`)
        .gap(6)
        .items_center()
        .tooltip(`${who} · ${words}`)
        .children(missing.map((g) => AgentLogo(g, { size, color: k.dim })))
        .child(div().flex_1().min_w(0).truncate().text_size(TEXT.xs).text_color(k.dim).child(missing.length > 1 || tightest ? words : `${who} · ${words}`)),
    );
  }
  return v_flex()
    .id(id)
    .gap(tightest ? 5 : compact ? 6 : 8)
    .children(lines);
}

// ---------------------------------------------------------------------------------------------------------------------
// The sheet

export default class Usage {
  init(_props, cx) {
    this.value = null;
    this.state = "connecting";
    this.toasts = new Toaster();
    this.handle = cx.focus_handle();
    this.handle.focus();
    this.sub = hub.subscribe(
      cx,
      { views: ["usage"] },
      (m, cx) => {
        if (m.view === "usage") this.value = m.value;
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

  dismiss() {
    closeOverlay(props().overlay);
  }

  /** esc closes; shift+R reads every agent's files again, as a stale window's chip does, even with no window yet. */
  key(e, cx) {
    if (e.key === "escape") {
      this.dismiss();
      return true;
    }
    if (e.keystroke === "shift-r" && !this.value?.sources?.length) {
      runSource({ id: "usage-all", action: { kind: "refresh", label: "Reading it again" } }, cx, this);
      return true;
    }
    return sourceKey(e, this.value?.sources, cx, this, { refresh: true });
  }

  section(k, title, aside, children) {
    return v_flex()
      .gap(10)
      .child(
        h_flex()
          .gap(8)
          .items_center()
          .child(div().text_size(TEXT.xs).font_semibold().text_color(k.mutedFg).child(title.toUpperCase()))
          .child(div().flex_1().h(1).bg(k.border))
          .when(Boolean(aside), (e) => e.child(aside)),
      )
      .children(children);
  }

  /** One window as a bar, with its reset, freshness and source under it. */
  window(k, q, id) {
    return v_flex()
      .gap(3)
      .child(QuotaRow(q, { id, labelW: 64, label: q.name }))
      .child(
        h_flex()
          .pl(72)
          .gap(6)
          .text_size(TEXT.xs)
          .text_color(k.dim)
          .when(Boolean(q.resets_words), (e) => e.child(q.resets_words).child("·"))
          .child(asOfWords(q))
          .child("·")
          .child(`from ${q.source_words}`)
          .when(!q.percent && q.percent !== 0 && Boolean(q.status_words), (e) => e.child("·").child(q.status_words)),
      );
  }

  /**
   * Each agent with its logo, the state of its usage and the account it runs under as it says, never more, then its
   * windows beneath, shortest first; one that reports nothing says so and gets no bar.
   */
  providers(k, providers) {
    if (!providers.length) return [div().text_size(TEXT.sm).text_color(k.mutedFg).child("No agent is installed here.")];
    return providers.map((p) => {
      const tone = { fresh: "success", stale: "secondary", unavailable: "destructive" }[p.state] ?? "outline";
      const credits = p.credits?.unlimited ? "Credits: unlimited" : p.credits?.has_credits ? `Credits: ${p.credits.balance}` : null;
      const windows = p.windows ?? [];
      return v_flex()
        .id(`usage-agent-${p.agent}`)
        .gap(10)
        .px(12)
        .py(10)
        .rounded(RADIUS.lg)
        .border_1()
        .border_color(k.border)
        .child(
          h_flex()
            .gap(10)
            .items_center()
            .child(h_flex().size(28).flex_none().items_center().justify_center().rounded(RADIUS.md).bg(k.muted).child(AgentLogo(p.icon ?? "agent", { size: 16 })))
            .child(
              v_flex()
                .flex_1()
                .min_w(0)
                .gap(2)
                .child(h_flex().gap(8).items_center().child(div().text_size(TEXT.base).font_medium().child(p.name)).child(Badge(STATE_WORDS[p.state] ?? p.state, tone)))
                .child(div().truncate().text_size(TEXT.xs).text_color(k.mutedFg).child(p.words))
                .when(Boolean(p.identity?.words), (e) =>
                  e.child(h_flex().gap(5).items_center().text_size(TEXT.xs).text_color(k.dim).child(Icon("user", { size: 12 })).child(p.identity.words)),
                )
                .when(Boolean(credits), (e) => e.child(div().text_size(TEXT.xs).text_color(k.dim).child(credits))),
            ),
        )
        .when(windows.length > 0, (e) => e.child(v_flex().pl(38).gap(8).children(windows.map((q, i) => this.window(k, q, `usage-w-${p.agent}-${i}`)))));
    });
  }

  /** The last seven days' tokens per agent, and their cost at API prices, labelled as the estimate it is. */
  days(k, t) {
    if (!t) return [h_flex().gap(8).items_center().text_size(TEXT.sm).text_color(k.mutedFg).child(Spinner({ size: 13 })).child("Loading usage…")];
    const agents = Object.keys(t.week?.agents ?? {});
    if (!agents.length) return [div().text_size(TEXT.sm).text_color(k.mutedFg).child("No usage data: the transcripts of these days hold no tokens.")];
    const most = Math.max(1, ...t.days.map((d) => d.total?.total ?? 0));
    const provider = (a) => this.value?.providers?.find((p) => p.agent === a);
    const name = (a) => h_flex().gap(5).items_center().child(AgentLogo(provider(a)?.icon ?? "agent", { size: 12 })).child(provider(a)?.name ?? t.names?.[a] ?? a);
    const cell = (c) => (c?.total ? `${count(c.total)} · ${price(c)}` : "·");
    // Two agents leave room for the day's bar; more share the row, each column as wide as the others.
    const bars = agents.length <= 2;
    const line = (id, label, cells, total, bar, strong = false) =>
      h_flex()
        .id(id)
        .h(24)
        .gap(10)
        .items_center()
        .text_size(TEXT.xs)
        .text_color(strong ? k.foreground : k.mutedFg)
        .when(strong, (e) => e.font_semibold())
        .child(div().w(64).flex_none().child(label))
        .children(cells.map((c) => (bars ? div().w(120).flex_none() : div().flex_1().min_w(0).truncate()).font_family("JetBrains Mono").child(c)))
        .child(
          h_flex()
            .when(bars, (e) => e.flex_1())
            .gap(8)
            .items_center()
            .when(bars, (e) => e.child(div().flex_1().h(6).rounded(3).bg(k.muted).child(div().h_full().rounded(3).bg(alpha(k.foreground, 0.35)).w(`${Math.round(bar * 100)}%`))))
            .child(div().w(56).flex_none().text_right().font_family("JetBrains Mono").child(total)),
        );
    return [
      line("usage-days-head", "", agents.map(name), "estimate", 0),
      ...t.days.map((d) => line(`usage-day-${d.date}`, d.label, agents.map((a) => cell(d.agents?.[a])), price(d.total), (d.total?.total ?? 0) / most)),
      line("usage-week", "7 days", agents.map((a) => cell(t.week.agents[a])), price(t.week.total), 1, true),
      div()
        .pt(6)
        .text_size(TEXT.xs)
        .text_color(k.dim)
        .whitespace_normal()
        .child(
          `Estimated at API prices from the agents' transcripts: a plan is not billed this way.${t.unpriced?.length ? ` No price is known for ${t.unpriced.join(", ")}, so it counts in tokens only.` : ""}`,
        ),
    ];
  }

  render(_cx) {
    const k = tokens();
    const v = this.value;
    const offline = !v && this.state.startsWith("error");
    const subtitle = offline ? hub.downWords(this.state) : v ? `${v.windows?.length ?? 0} limits · from what each agent reports, read on this machine` : "Loading usage…";
    const body = !v
      ? [
          offline
            ? h_flex().h(160).justify_center().items_center().child(HubDown("usage-hub-down", this, this.state.replace(/^error: /, ""), k))
            : h_flex().h(160).justify_center().items_center().gap(8).text_color(k.mutedFg).child(Spinner({ size: 14 })).child(subtitle),
        ]
      : [
          this.section(k, "Limits and accounts, by agent", null, this.providers(k, v.providers ?? [])),
          this.section(k, "Tokens, last 7 days", Badge("estimated", "outline"), this.days(k, v.loading ? null : v.tokens)),
        ];
    return v_flex()
      .id("usage")
      .track_focus(this.handle)
      .size_full()
      .bg(k.popover)
      .text_color(k.foreground)
      .on_key_down((e, cx) => {
        if (this.key(e, cx)) {
          cx.stop_propagation();
          cx.notify();
        }
      })
      .relative()
      // Each window's age as a source chip, as every page's header says how fresh its data is.
      .child(
        PageHeader({
          icon: "gauge",
          title: "Usage",
          description: subtitle,
          sources: v?.sources,
          view: this,
          actions: [Button("usage-close", { icon: "x", variant: "ghost", size: "icon-sm", tooltip: "Close  esc", onClick: () => this.dismiss() })],
        }),
      )
      .child(v_flex().id("usage-body").flex_1().min_h(0).px(24).py(16).gap(22).overflow_y_scrollbar().children(body))
      .child(KeyHints([["esc", "close"], ["⇧R", "read again"]]))
      .when(Boolean(this.toasts.items.length), (e) => e.child(this.toasts.render()));
  }
}
