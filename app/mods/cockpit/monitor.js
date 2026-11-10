// The monitor as a dashboard: the `monitor` view the hub pushes. Verdict tiles over the apps, each app a row with its
// monitors meter and its errors and p95 history strips, unfolded onto its alerting monitors, new errors and releases;
// the selected row's details beside the list (before/after meters, new errors, tickets) with its actions, the same as
// the TUI's (investigate, fix, bug, roll back after a confirm, talk, open, ArgoCD, pin, refresh); every release can pop
// out as a kept window (`monitor-window`). Keys: the TUI's, ctrl+p for every action, / to filter, m for a row's menu.

import { div } from "gpui-kit";
import { h_flex, v_flex, InputState } from "gpui-base";
import { props, openOverlay, closeOverlay } from "herdr";
import * as hub from "./hub.js";
import {
  tokens, TEXT, RADIUS, Icon, Badge, Button, Kbd, PageHeader, SearchField, KeyHints, Empty, ErrorState, Skeleton, Command,
  Toaster, Flashes, Scroller, Hover, SidePanel, AlertDialog, Avatar, Chip, Mono, Metric, StageHeading, Meter, Strip, SegBar,
  Tabs, Spinner, openMenu, alpha, Overline, Tiles,
} from "./ui.js";

const ICONS = {
  investigate: "scan-eye", open: "external", argocd: "rocket", pin: "pin", mute: "volume-x", refresh: "refresh-cw", fix: "wrench", bug: "bug",
  roll_back: "rotate-ccw", talk: "message-circle",
};
const STATE = {
  failed: ["Failed", "circle-x", (k) => k.destructive, "destructive"],
  moved: ["Moved", "triangle-alert", (k) => k.warning, "warning"],
  alert: ["Alert", "bell", (k) => k.destructive, "destructive"],
  watching: ["Watching", "hourglass", (k) => k.info, "info"],
  ok: ["Ok", "circle-check", (k) => k.success, "success"],
  idle: ["Not watched", "circle-dashed", (k) => k.mutedFg, "outline"],
  unknown: ["Unknown", "circle-help", (k) => k.mutedFg, "outline"],
};
const state = (s) => STATE[s] ?? STATE.unknown;
const tone = (s, k) => state(s)[2](k);
const SIDE_AT = 980;

/** A model action's key ("A") as gpui spells the keystroke ("shift-a"), and back. */
const stroke = (key) => (key && key !== key.toLowerCase() ? `shift-${key.toLowerCase()}` : key);
const unstroke = (s) => (/^shift-[a-z]$/.test(s) ? s.slice(6).toUpperCase() : s);

/** Every row the model has, by key: { kind, app, item }. */
function index(model) {
  const all = new Map();
  for (const app of model?.apps ?? []) {
    all.set(app.key, { kind: "app", app, item: app });
    for (const m of app.alerting) all.set(m.key, { kind: "monitor", app, item: m });
    for (const i of app.issues) all.set(i.key, { kind: "issue", app, item: i });
    for (const r of app.releases) all.set(r.key, { kind: "release", app, item: r });
  }
  return all;
}

/** Runs a row's action through the hub: `armed` once a confirm said yes. */
async function send(toasts, cx, a, armed) {
  try {
    const r = await hub.request("act", { app: "monitor", action: a.action, args: a.args, armed });
    if (r.error) toasts.push(cx, r.error, { kind: "error" });
    else if (r.note) toasts.push(cx, r.note, { kind: "success" });
  } catch (e) {
    toasts.push(cx, "The hub did not answer", { kind: "error", description: String(e?.message ?? e) });
  }
  cx.notify();
}

/** Opens the release (or any row) as a window that stays, and comes back after a restart until closed. */
export function popOut(key, title) {
  openOverlay({ mod: "cockpit", kind: "window", frame: "none", key: `monitor:${key}`, keep: true, width: 460, height: 620, props: { app: "monitor-window", key, title } });
}

// ---------------------------------------------------------------------------------------------------------------------
// Pieces both the dashboard and its windows draw

function verdictBadge(s, { id } = {}) {
  const [label, icon, , variant] = state(s);
  return Badge(label, variant, { icon, id });
}

function hist(id, label, s, { w = 3, h = 22, wide = false } = {}) {
  const k = tokens();
  if (!s?.values?.length) return div();
  const t = s.level >= 2 ? k.destructive : s.level === 1 ? k.warning : k.foreground;
  return h_flex()
    .id(id)
    .flex_none()
    .gap(8)
    .items_end()
    .tooltip(`${label} · the last ${s.values.length} samples · usual ${Math.round(s.usual * 100) / 100} · now ${s.text}`)
    .child(Strip(s.values, { levels: s.levels, h, w: wide ? w + 1 : w }))
    .child(v_flex().w(wide ? 72 : 56).gap(0).child(div().text_size(10).text_color(k.dim).child(label.toUpperCase())).child(Mono(s.text, { size: TEXT.sm, color: t })));
}

function monitorsMeter(id, m, { total = 18, segW = 5 } = {}) {
  const k = tokens();
  if (!m || !m.total) return div();
  const scale = (n) => (m.total <= total ? n : Math.round((n / m.total) * total));
  const parts = [
    { n: scale(m.alert), color: k.destructive },
    { n: scale(m.warn), color: k.warning },
    { n: scale(m.nodata), color: alpha(k.mutedFg, 0.6) },
    { n: Math.max(0, Math.min(total, m.total) - scale(m.alert) - scale(m.warn) - scale(m.nodata)), color: k.success },
  ];
  return Meter(parts, { total: Math.min(total, m.total), segW, id, tooltip: `${m.ok} ok · ${m.warn} warn · ${m.alert} alert · ${m.nodata} no data` });
}

/** Before and after a release, one measure: two bars on one scale, the change as a badge. */
function compare(id, label, before, after, text, { lowerIsBetter = true } = {}) {
  const k = tokens();
  const max = Math.max(before ?? 0, after ?? 0, 1e-9);
  const ratio = before > 0 ? after / before : after > 0 ? Infinity : 1;
  const worse = lowerIsBetter ? ratio > 1.25 : ratio < 0.8;
  const better = lowerIsBetter ? ratio < 0.8 : ratio > 1.25;
  const c = worse ? (ratio > 2 ? k.destructive : k.warning) : better ? k.success : k.mutedFg;
  const pct = Number.isFinite(ratio) ? `${ratio >= 1 ? "+" : "−"}${Math.abs(Math.round((ratio - 1) * 100))}%` : "new";
  const bar = (v, color) => div().h(6).rounded(3).bg(alpha(k.border, 0.35)).w_full().child(div().h_full().rounded(3).bg(color).w(`${Math.max(2, (v / max) * 100)}%`));
  return v_flex()
    .id(id)
    .gap(6)
    .child(
      h_flex()
        .gap(8)
        .items_center()
        .child(div().flex_1().text_size(TEXT.sm).font_medium().text_color(k.foreground).child(label))
        .child(Mono(text(before), { size: TEXT.sm }))
        .child(Icon("chevron-right", { size: 12, color: k.dim }))
        .child(Mono(text(after), { size: TEXT.sm, color: worse ? c : k.foreground }))
        .child(Badge(pct, worse ? (ratio > 2 ? "destructive" : "warning") : better ? "success" : "secondary")),
    )
    .child(v_flex().gap(3).child(bar(before ?? 0, alpha(k.mutedFg, 0.85))).child(bar(after ?? 0, c)));
}

function factRow(label, value) {
  const k = tokens();
  return h_flex()
    .gap(12)
    .items_start()
    .child(div().w(92).flex_none().text_size(TEXT.sm).text_color(k.mutedFg).child(label))
    .child(typeof value === "string" ? div().flex_1().min_w(0).text_size(TEXT.sm).text_color(k.foreground).child(value) : h_flex().flex_1().min_w(0).gap(6).flex_wrap().child(value));
}

/** The actions as buttons: the main one solid, each with its key, roll back in red. */
function actionButtons(prefix, acts, run, { keys = true } = {}) {
  return h_flex()
    .gap(6)
    .flex_wrap()
    .children(
      (acts ?? []).map((a) =>
        Button(`${prefix}-${a.id}`, {
          label: a.label[0].toUpperCase() + a.label.slice(1),
          icon: ICONS[a.id],
          kbd: keys ? a.key : undefined,
          size: "sm",
          variant: a.main ? "default" : a.id === "roll_back" ? "destructive" : "outline",
          onClick: (_e, cx) => run(a, cx),
        }),
      ),
    );
}

/** The details of one row (an app, an alerting monitor, a new error or a release), as the side panel and windows show. */
export function details(entry, { run, open, compact = false }) {
  const k = tokens();
  const { kind, app, item } = entry;
  const out = [];
  const section = (title) => Overline(title).pt(4);
  if (kind === "release") {
    const r = item;
    const t = tone(r.state, k);
    out.push(
      v_flex()
        .gap(6)
        .p(12)
        .rounded(RADIUS.lg)
        .border_1()
        .border_color(alpha(t, 0.4))
        .bg(alpha(t, 0.08))
        .child(h_flex().gap(8).items_center().child(verdictBadge(r.state)).child(div().flex_1()).when(r.watchingFor !== null && r.watchingFor !== undefined, (e) => e.child(div().text_size(TEXT.xs).text_color(k.mutedFg).child(`judged in ${r.watchingFor} min`))))
        .child(div().whitespace_normal().text_size(TEXT.base).text_color(k.foreground).child(r.summary || "No verdict yet")),
    );
    out.push(factRow("Shipped", h_flex().gap(6).items_center().child(Avatar(r.who, { size: 18 })).child(div().text_size(TEXT.sm).child(`${r.who} · ${r.hm ?? ""} · ${r.ago ?? ""}`))));
    out.push(factRow("Phase", r.phase || "—"));
    if (r.previous) out.push(factRow("Rolls back to", Mono(r.previous, { size: TEXT.sm, color: k.foreground })));
    if (r.before && r.after) {
      out.push(section("Before → after"));
      out.push(compare(`cmp-err-${r.key}`, "Error rate", r.before.rate, r.after.rate, (v) => `${(v ?? 0).toFixed(2)}%`));
      out.push(compare(`cmp-p95-${r.key}`, "p95 latency", r.before.p95, r.after.p95, (v) => `${Math.round(v ?? 0)} ms`));
      out.push(compare(`cmp-hits-${r.key}`, "Requests", r.before.hits, r.after.hits, (v) => String(Math.round(v ?? 0)), { lowerIsBetter: false }));
    } else if (r.before) {
      out.push(section("Before it"));
      out.push(factRow("Error rate", r.before.rateText));
      out.push(factRow("p95", r.before.p95Text));
    }
    const fresh = app.issues.filter((i) => r.newIssues.includes(i.id));
    if (fresh.length) {
      out.push(section(`New errors · ${fresh.length}`));
      for (const i of fresh)
        out.push(
          h_flex()
            .id(`d-issue-${i.key}`)
            .gap(8)
            .items_center()
            .p(8)
            .rounded(RADIUS.md)
            .border_1()
            .border_color(k.border)
            .child(Icon("bug", { size: 14, color: k.destructive }))
            .child(v_flex().flex_1().min_w(0).child(Mono(i.kind, { size: TEXT.sm, color: k.foreground })).child(div().truncate().text_size(TEXT.xs).text_color(k.mutedFg).child(i.message)))
            .child(Chip(`${i.count}×`, { mono: true })),
        );
    }
    if (r.tickets.length || r.prs.length) {
      out.push(section("Shipping"));
      out.push(
        h_flex()
          .gap(6)
          .flex_wrap()
          .children([
            ...r.tickets.map((t) => Chip(t.id, { id: `d-t-${r.key}-${t.id}`, icon: "ticket", mono: true, tooltip: t.url, onClick: (_e, cx) => open(t.url, cx) })),
            ...r.prs.map((p) => Chip(p.name, { id: `d-p-${r.key}-${p.name}`, icon: "git-pull-request", mono: true, tooltip: p.url, onClick: (_e, cx) => open(p.url, cx) })),
          ]),
      );
    }
  } else if (kind === "app") {
    const a = item;
    out.push(factRow("Running", h_flex().gap(6).child(Mono(a.tag, { size: TEXT.sm, color: k.foreground })).child(Badge(a.health || "?", a.health === "Healthy" ? "success" : a.health === "Degraded" ? "destructive" : "secondary"))));
    out.push(factRow("Deployed", `${a.deployedAgo ?? "—"}${a.behind ? ` · ${a.behind} behind` : ""}`));
    if (a.monitors) {
      out.push(section("Monitors"));
      out.push(h_flex().gap(10).items_center().child(monitorsMeter(`d-meter-${a.key}`, a.monitors, { total: 24, segW: 8 })).child(div().text_size(TEXT.sm).text_color(k.mutedFg).child(a.counts ?? "")));
    }
    if (a.errors || a.p95) {
      out.push(section("History"));
      out.push(v_flex().gap(10).child(hist(`d-err-${a.key}`, "errors", a.errors, { h: 36, w: 5, wide: true })).child(hist(`d-p95-${a.key}`, "p95", a.p95, { h: 36, w: 5, wide: true })));
    }
    if (a.releases.length) {
      out.push(section(`Releases · ${a.releases.length}`));
      for (const r of a.releases)
        out.push(h_flex().gap(8).items_center().child(verdictBadge(r.state)).child(Mono(r.tag, { size: TEXT.sm, color: k.foreground })).child(div().flex_1().truncate().text_size(TEXT.xs).text_color(k.mutedFg).child(`${r.who} · ${r.ago ?? ""}`)));
    }
    if (a.muted) out.push(div().text_size(TEXT.sm).text_color(k.mutedFg).child("Muted: not watched, nor announced, until you unmute it."));
    else if (!a.watched) out.push(div().text_size(TEXT.sm).text_color(k.mutedFg).child("Not watched: no release today and not pinned. Pin it to watch it anyway."));
  } else if (kind === "issue") {
    const i = item;
    out.push(div().whitespace_normal().text_size(TEXT.base).text_color(k.foreground).child(i.message));
    out.push(factRow("Events", `${i.count} in the last day`));
    out.push(factRow("First seen", `${i.firstSeenAgo ?? "—"} in ${i.firstVersion || "?"}`));
    if (i.release) out.push(factRow("Came with", Mono(i.release, { size: TEXT.sm, color: k.foreground })));
  } else if (kind === "monitor") {
    const m = item;
    out.push(h_flex().gap(8).child(Badge(m.status, m.status === "Alert" ? "destructive" : "warning", { icon: "bell" })));
    out.push(div().whitespace_normal().text_size(TEXT.base).text_color(k.foreground).child(m.name));
  }
  const acts = item.acts ?? [];
  if (acts.length) {
    out.push(section("Actions"));
    out.push(actionButtons(`d-${compact ? "w" : "p"}-${entry.item.key}`, acts, run, { keys: !compact }));
  }
  return out;
}

function titleOf(entry) {
  const { kind, app, item } = entry;
  if (kind === "release") return [`${app.name} ${item.tag}`, `release · ${item.who} · ${item.ago ?? ""}`];
  if (kind === "issue") return [`${app.name} · ${item.kind}`, "new error"];
  if (kind === "monitor") return [item.name, `${app.name} · monitor`];
  return [app.name, `${app.service} · ${app.repo}`];
}

// ---------------------------------------------------------------------------------------------------------------------
// The dashboard

export default class Monitor {
  init(_props, cx) {
    this.model = null;
    this.byKey = new Map();
    this.status = "connecting";
    this.sel = null;
    this.timeline = false;
    this.opened = new Set();
    this.closed = new Set();
    this.filter = "";
    this.filtering = false;
    this.palette = null;
    this.confirm = null;
    this.fresh = new Map(); // key → newest event time, since you last looked
    this.cleared = new Map();
    this.visible = false;
    this.width = 1200;
    this.hover = new Hover();
    this.toasts = new Toaster();
    this.flashes = new Flashes();
    this.scroller = new Scroller();
    this.handle = cx.focus_handle();
    this.search = InputState.new({ placeholder: "Filter apps, tags, errors…" });
    this.search.on("change", (_e, cx) => {
      this.filter = this.search.value();
      cx.notify();
    });
    this.search.on("submit", (_e, cx) => {
      this.filtering = false;
      this.handle.focus();
      cx.notify();
    });
    this.query = InputState.new({ placeholder: "Type a command or search…" });
    this.query.on("change", (_e, cx) => {
      if (this.palette) this.palette.at = 0;
      cx.notify();
    });
    this.query.on("submit", (_e, cx) => this.runPalette(cx));
    this.sub = hub.subscribe(
      cx,
      { views: ["monitor"] },
      (m, cx) => {
        if (m.view === "monitor") this.take(m.value, cx);
        if (m.focus === "monitor") this.goTo(m.target?.key, cx);
        cx.notify();
      },
      (s, cx) => {
        this.status = s;
        cx.notify();
      },
    );
  }

  close() {
    this.sub?.stop();
  }

  command(c, cx) {
    if (c === "focus" || c === "show:monitor") {
      this.visible = true;
      this.handle.focus();
    } else if (c.startsWith("show:") && this.visible) {
      this.leave(cx);
    }
    const [scope, what, id, ...rest] = c.split(":");
    if (scope === "monitor" && what === "act") {
      const key = rest.join(":");
      const a = this.byKey.get(key)?.item.acts?.find((x) => x.id === id);
      if (a) {
        this.sel = key;
        this.run(a, cx);
      }
    } else if (scope === "monitor" && what === "window") popOut(rest.length ? `${id}:${rest.join(":")}` : id);
    cx.notify();
  }

  /** You looked away: the hub keeps the time, so the next visit marks what changed after it. */
  leave(cx) {
    this.visible = false;
    this.fresh.clear();
    this.cleared.clear();
    cx.spawn(async () => hub.act("monitor", "looked", []).catch(() => {}));
  }

  take(model, cx) {
    const before = this.byKey;
    this.model = model;
    this.byKey = index(model);
    const k = tokens();
    for (const [key, e] of this.byKey) {
      const was = before.get(key);
      if (e.kind === "release" && was && was.item.state !== e.item.state) this.flashes.start(cx, key, tone(e.item.state, k));
      if (e.kind !== "app" && !was && before.size) this.flashes.start(cx, key, k.ring);
    }
    if (!this.sel || !this.byKey.has(this.sel)) this.sel = this.rows()[0]?.key ?? null;
    if (model?.ready && model.since) this.fetchEvents(model.since, cx);
  }

  fetchEvents(since, cx) {
    cx.spawn(async (cx) => {
      const events = await hub.events("monitor", since).catch(() => []);
      this.fresh = new Map();
      for (const e of events) if ((e.at ?? 0) > (this.fresh.get(e.key) ?? 0)) this.fresh.set(e.key, e.at ?? 0);
      this.events = events;
      cx.notify();
    });
  }

  /** Whether the row (or, folded, a row under it) changed since you last looked. */
  marked(row) {
    const keys = [row.key, ...(row.kind === "app" && row.fold === false ? row.kids.map((x) => x.key) : [])];
    return keys.some((key) => this.fresh.has(key) && this.fresh.get(key) > (this.cleared.get(key) ?? 0));
  }

  select(key) {
    this.sel = key;
    const row = this.rows().find((r) => r.key === key);
    if (row) for (const k of [row.key, ...(row.kind === "app" && row.fold === false ? row.kids.map((x) => x.key) : [])]) if (this.fresh.has(k)) this.cleared.set(k, this.fresh.get(k));
  }

  isOpen(app) {
    return this.opened.has(app.key) || (app.troubled && !this.closed.has(app.key));
  }

  toggle(key) {
    const app = this.byKey.get(key)?.app;
    if (!app) return;
    if (this.isOpen(app)) {
      this.opened.delete(app.key);
      this.closed.add(app.key);
    } else {
      this.closed.delete(app.key);
      this.opened.add(app.key);
    }
  }

  textOf(e) {
    const { kind, app, item } = e;
    const parts = [app.name, app.tag];
    if (kind === "release") parts.push(item.tag, item.who, item.summary, item.state);
    if (kind === "issue") parts.push(item.kind, item.message);
    if (kind === "monitor") parts.push(item.name, item.status);
    return parts.join(" ").toLowerCase();
  }

  hits(e) {
    const words = this.filter.toLowerCase().split(/\s+/).filter(Boolean);
    const text = this.textOf(e);
    return words.every((w) => text.includes(w));
  }

  /** The rows as they show: by app (troubled first, unfolded onto their rows) or the releases newest first. */
  rows() {
    const m = this.model;
    if (!m?.ready) return [];
    const filtering = this.filter.trim() !== "";
    if (this.timeline) return m.timeline.map((key) => ({ key, ...this.byKey.get(key), depth: 0 })).filter((r) => r.item && (!filtering || this.hits(r)));
    const out = [];
    for (const app of m.apps) {
      const kids = [
        ...app.alerting.map((item) => ({ key: item.key, kind: "monitor", app, item })),
        ...app.issues.filter((i) => i.isNew).map((item) => ({ key: item.key, kind: "issue", app, item })),
        ...app.releases.map((item) => ({ key: item.key, kind: "release", app, item })),
      ];
      const shownKids = filtering ? kids.filter((x) => this.hits(x)) : kids;
      const appHit = !filtering || this.hits({ kind: "app", app, item: app });
      if (filtering && !appHit && !shownKids.length) continue;
      const open = filtering ? shownKids.length > 0 : this.isOpen(app);
      out.push({ key: app.key, kind: "app", app, item: app, depth: 0, kids, fold: kids.length ? open : null });
      if (open) shownKids.forEach((x, i) => out.push({ ...x, depth: 1, last: i === shownKids.length - 1 }));
    }
    return out;
  }

  selected() {
    return this.sel ? this.byKey.get(this.sel) ?? null : null;
  }

  goTo(key, cx) {
    if (!key || !this.model?.ready) return;
    const [appKey] = key.split("/");
    const app = this.model.apps.find((a) => a.key === appKey || a.name === appKey);
    if (!app) return this.toasts.push(cx, `${appKey} is not in the monitor`);
    const target = key.includes("/") ? key : app.key;
    this.timeline = false;
    this.closed.delete(app.key);
    this.opened.add(app.key);
    if (this.filter) {
      this.filter = "";
      this.search.set_value("");
    }
    this.select(this.byKey.has(target) ? target : app.key);
    this.handle.focus();
  }

  move(by) {
    const rows = this.rows();
    if (!rows.length) return;
    const at = Math.max(0, rows.findIndex((r) => r.key === this.sel));
    this.select(rows[Math.max(0, Math.min(rows.length - 1, at + by))].key);
  }

  moveGroup(by) {
    const rows = this.rows();
    const at = Math.max(0, rows.findIndex((r) => r.key === this.sel));
    const heads = rows.map((r, i) => [r, i]).filter(([r]) => r.depth === 0).map(([, i]) => i);
    const next = by > 0 ? heads.find((i) => i > at) : [...heads].reverse().find((i) => i < at);
    if (next !== undefined) this.select(rows[next].key);
  }

  fold() {
    const e = this.selected();
    if (!e || this.timeline) return;
    if (e.kind !== "app") this.select(e.app.key);
    this.toggle(e.app.key);
  }

  run(a, cx) {
    if (a.confirm) {
      this.confirm = a;
      cx.notify();
      return;
    }
    cx.spawn(async (cx) => send(this.toasts, cx, a));
  }

  open(url, cx) {
    if (url) cx.spawn(async (cx) => send(this.toasts, cx, { action: "open_url", args: [url] }));
  }

  refresh(cx) {
    cx.spawn(async (cx) => send(this.toasts, cx, { action: "request_refresh", args: [] }));
  }

  menu(e) {
    const acts = e.item.acts ?? [];
    openMenu([
      ...acts.map((a) => ({ label: a.label[0].toUpperCase() + a.label.slice(1), icon: ICONS[a.id], kbd: a.key, destructive: a.id === "roll_back", command: `monitor:act:${a.id}:${e.item.key}` })),
      "separator",
      { label: "Open in a window", icon: "picture-in-picture", kbd: "W", command: `monitor:window:${e.item.key}` },
    ]);
  }

  // -------------------------------------------------------------------------------------------------------------------
  // The command list (ctrl+p): the view's own, then every row's actions and a jump to each

  paletteGroups(cx) {
    const q = this.query.value().toLowerCase().split(/\s+/).filter(Boolean);
    const hit = (t) => q.every((w) => t.toLowerCase().includes(w));
    const own = [
      { id: "p-view", label: this.timeline ? "Show the apps" : "Show the releases, newest first", shortcut: "w", icon: "history", run: () => ((this.timeline = !this.timeline), cx.notify()) },
      { id: "p-refresh", label: "Refresh the monitor", shortcut: "R", icon: "refresh-cw", run: () => this.refresh(cx) },
    ];
    const acts = [];
    const jumps = [];
    for (const [key, e] of this.byKey) {
      const [name, help] = titleOf(e);
      for (const a of e.item.acts ?? []) acts.push({ id: `p-${a.id}-${key}`, label: `${a.label[0].toUpperCase() + a.label.slice(1)}  ${name}`, hint: help, icon: ICONS[a.id], run: () => ((this.sel = key), this.run(a, cx)) });
      jumps.push({ id: `p-go-${key}`, label: `Go to  ${name}`, hint: help, icon: "chevron-right", run: () => (this.goTo(key, cx), cx.notify()) });
    }
    const main = acts.filter((a) => {
      const e = this.byKey.get(a.id.split("-").slice(2).join("-"));
      return e?.kind !== "app" && e?.item.acts?.find((x) => x.main && a.id === `p-${x.id}-${e.item.key}`);
    });
    return [
      { heading: "Monitor", items: own.filter((i) => hit(i.label)) },
      { heading: q.length ? "Actions" : "Suggested", items: (q.length ? acts : main).filter((i) => hit(`${i.label} ${i.hint ?? ""}`)) },
      { heading: "Go to", items: q.length ? jumps.filter((i) => hit(`${i.label} ${i.hint ?? ""}`)) : [] },
    ];
  }

  openPalette(cx) {
    this.palette = { at: 0 };
    this.query.set_value("");
    this.query.focus();
    cx.notify();
  }

  closePalette(cx) {
    this.palette = null;
    this.handle.focus();
    cx.notify();
  }

  runPalette(cx, picked) {
    const all = this.paletteGroups(cx).flatMap((g) => g.items);
    const item = picked ?? all[this.palette?.at ?? 0];
    this.closePalette(cx);
    if (item) item.run();
  }

  // -------------------------------------------------------------------------------------------------------------------
  // Keys

  key(ev, cx) {
    const s = ev.keystroke;
    if (this.confirm) {
      if (s === "escape") this.confirm = null;
      else if (s === "enter") {
        const a = this.confirm;
        this.confirm = null;
        cx.spawn(async (cx) => send(this.toasts, cx, a, a.confirm));
      } else return false;
      cx.notify();
      return true;
    }
    if (this.palette) {
      const n = this.paletteGroups(cx).flatMap((g) => g.items).length;
      if (s === "escape") this.closePalette(cx);
      else if (s === "down" || s === "ctrl-n") this.palette.at = Math.min(n - 1, this.palette.at + 1);
      else if (s === "up" || s === "ctrl-p") this.palette.at = Math.max(0, this.palette.at - 1);
      else return false;
      cx.notify();
      return true;
    }
    if (this.filtering) {
      if (s !== "escape") return false;
      this.filtering = false;
      this.filter = "";
      this.search.set_value("");
      this.handle.focus();
      cx.notify();
      return true;
    }
    const e = this.selected();
    const acts = e?.item.acts ?? [];
    const own = acts.find((a) => a.key && (a.key === unstroke(s) || stroke(a.key) === s));
    if (own) {
      this.run(own, cx);
      cx.notify();
      return true;
    }
    switch (s) {
      case "ctrl-p":
      case "cmd-shift-p":
        this.openPalette(cx);
        break;
      case "j":
      case "down":
        this.move(1);
        break;
      case "k":
      case "up":
        this.move(-1);
        break;
      case "pagedown":
      case "ctrl-d":
        this.move(10);
        break;
      case "pageup":
      case "ctrl-u":
        this.move(-10);
        break;
      case "g":
      case "home":
        this.move(-1e6);
        break;
      case "shift-g":
      case "end":
        this.move(1e6);
        break;
      case "]":
        this.moveGroup(1);
        break;
      case "[":
        this.moveGroup(-1);
        break;
      case "w":
        this.timeline = !this.timeline;
        break;
      case "shift-r":
        this.refresh(cx);
        break;
      case "space":
        this.fold();
        break;
      case "enter": {
        const main = acts.find((a) => a.main);
        if (main) this.run(main, cx);
        else this.fold();
        break;
      }
      case "shift-w":
        if (e) popOut(e.item.key, titleOf(e)[0]);
        break;
      case "/":
        this.filtering = true;
        this.search.focus();
        break;
      case "escape":
        this.filter = "";
        this.search.set_value("");
        break;
      case "m":
      case "shift-f10":
        if (e) this.menu(e);
        break;
      default:
        return false;
    }
    cx.notify();
    return true;
  }

  // -------------------------------------------------------------------------------------------------------------------
  // Drawing

  tiles(k, cx) {
    const m = this.model;
    const t = m.tiles;
    const v = m.verdict;
    const word = v === "idle" || v === "ok" ? "All good" : state(v)[0];
    const bad = ["failed", "moved", "alert"].includes(v);
    const segs = [
      { value: t.failed, color: k.destructive, label: `${t.failed} failed` },
      { value: t.moved, color: k.warning, label: `${t.moved} moved` },
      { value: t.watching, color: k.info, label: `${t.watching} watching` },
      { value: t.ok, color: k.success, label: `${t.ok} ok` },
    ];
    const firstOf = (pred) => () => {
      const row = this.rows().find(pred) ?? this.rows(true).find(pred);
      if (row) this.goTo(row.key, cx);
    };
    return Tiles([
      Metric("tile-verdict", {
        label: "Production",
        value: word,
        tone: bad ? tone(v, k) : k.success,
        tint: bad,
        icon: state(bad ? v : "ok")[1],
        hint: m.headline || (m.env ? `${m.env} · ${m.ago ?? ""}` : ""),
        onClick: (_e, cx) => (firstOf((r) => r.depth === 0 && r.item.troubled)(), cx.notify()),
      }),
      v_flex()
        .id("tile-releases")
        .flex_1()
        .min_w(132)
        .px(16)
        .py(14)
        .gap(6)
        .rounded(RADIUS.lg)
        .border_1()
        .border_color(k.border)
        .bg(k.card)
        .child(h_flex().gap(6).items_center().child(Icon("rocket", { size: 13, color: k.mutedFg })).child(div().text_size(TEXT.xs).font_semibold().text_color(k.mutedFg).child("RELEASES TODAY")))
        .child(div().text_size(26).font_semibold().text_color(k.foreground).child(String(t.today)))
        .child(SegBar("tile-releases-bar", segs)),
      v_flex()
        .id("tile-alerts")
        .flex_1()
        .min_w(132)
        .px(16)
        .py(14)
        .gap(6)
        .rounded(RADIUS.lg)
        .border_1()
        .border_color(t.alerts ? alpha(k.destructive, 0.35) : k.border)
        .bg(t.alerts ? alpha(k.destructive, 0.06) : k.card)
        .cursor_pointer()
        .on_click((_e, cx) => (firstOf((r) => r.kind === "monitor")(), cx.notify()))
        .child(h_flex().gap(6).items_center().child(Icon("bell", { size: 13, color: k.mutedFg })).child(div().text_size(TEXT.xs).font_semibold().text_color(k.mutedFg).child("ALERTS")))
        .child(h_flex().gap(8).items_end().child(div().text_size(26).font_semibold().text_color(t.alerts ? k.destructive : k.foreground).child(String(t.alerts))).child(div().pb(5).text_size(TEXT.xs).text_color(k.mutedFg).child(`of ${t.monitors} monitors`)))
        .child(monitorsMeter("tile-alerts-meter", { ok: t.monitors - t.alerts - t.warns, warn: t.warns, alert: t.alerts, nodata: 0, total: t.monitors }, { total: 20, segW: 6 })),
      Metric("tile-errors", {
        label: "New errors",
        value: t.newErrors,
        tone: t.newErrors ? k.destructive : k.foreground,
        icon: "bug",
        hint: t.newErrors ? `${t.newErrorEvents} events in the last day` : "none since the last release",
        onClick: (_e, cx) => (firstOf((r) => r.kind === "issue")(), cx.notify()),
      }),
      this.left() >= 820 ? Metric("tile-watched", { label: "Watched", value: `${t.watched}/${t.apps}`, icon: "eye", hint: "apps released today or pinned" }) : null,
    ]);
  }

  appRow(row, k, sel) {
    const a = row.item;
    const t = tone(a.state, k);
    const room = this.left();
    const wide = room >= 640;
    const glyph =
      a.state === "watching"
        ? Spinner({ size: 16, color: t })
        : Icon(state(a.state)[1], { size: 16, color: t });
    return h_flex()
      .id(`row-${row.key}`)
      .h(56)
      .px(12)
      .gap(12)
      .items_center()
      .rounded(RADIUS.md)
      .relative()
      .cursor_pointer()
      .border_1()
      .border_color(this.marked(row) ? k.ring : sel ? k.border : alpha(k.border, 0))
      .bg(sel ? k.selected : k.background)
      .when(!sel, (e) => e.hover((s) => s.bg(k.accent)))
      .child(
        div()
          .w(14)
          .flex_none()
          .child(row.fold === null ? "" : Icon(row.fold ? "chevron-down" : "chevron-right", { size: 14, color: k.dim })),
      )
      .child(glyph)
      .child(
        v_flex()
          .flex_1()
          .min_w(0)
          .gap(3)
          .child(
            h_flex()
              .gap(8)
              .items_center()
              .child(div().text_size(TEXT.base).font_semibold().text_color(a.watched ? k.foreground : k.mutedFg).child(a.name))
              .child(Mono(a.tag))
              .when(Boolean(a.pinned), (e) => e.child(Icon("pin", { size: 12, color: k.mutedFg })))
              .when(Boolean(a.muted), (e) => e.child(Icon("volume-x", { size: 12, color: k.mutedFg })))
              .when(a.health && a.health !== "Healthy", (e) => e.child(Badge(a.health, a.health === "Degraded" ? "destructive" : "secondary"))),
          )
          .child(div().truncate().text_size(TEXT.xs).text_color(k.mutedFg).child(a.watched ? `deployed ${a.deployedAgo ?? "?"}${a.counts ? ` · ${a.counts}` : ""}` : `deployed ${a.deployedAgo ?? "?"} · ${a.muted ? "muted" : "not watched"}`)),
      )
      .when(room >= 860 && Boolean(a.monitors), (e) => e.child(monitorsMeter(`meter-${row.key}`, a.monitors)))
      .when(wide, (e) => e.child(hist(`err-${row.key}`, "errors", a.errors)))
      .when(room >= 480, (e) => e.child(hist(`p95-${row.key}`, "p95", a.p95)))
      .when(Boolean(this.flashes.of(row.key)), (e) => e.child(div().id(`fl-${row.key}`).absolute().inset_0().rounded(RADIUS.md).bg(alpha(this.flashes.of(row.key)?.color ?? k.ring, 0.25)).opacity(this.flashes.of(row.key)?.on ? 1 : 0).transition("opacity", { duration: 1100, easing: "ease-out" })));
  }

  /** The list's width, beside the details panel when there is one. */
  left() {
    return this.width - (this.width >= SIDE_AT ? 400 : 0) - 48;
  }

  childRow(row, k, sel) {
    const { kind, item } = row;
    const flash = this.flashes.of(row.key);
    let lead;
    let title;
    let meta;
    if (kind === "release") {
      lead = verdictBadge(item.state);
      title = h_flex()
        .gap(8)
        .items_center()
        .min_w(0)
        .child(Mono(item.tag, { size: TEXT.sm, color: k.foreground }))
        .child(Avatar(item.who, { size: 16 }))
        .child(div().flex_none().text_size(TEXT.sm).text_color(k.mutedFg).child(item.who))
        .child(div().flex_1().min_w(0).truncate().text_size(TEXT.sm).text_color(item.state === "ok" ? k.mutedFg : k.foreground).child(item.summary));
      meta = div().flex_none().text_size(TEXT.xs).text_color(k.mutedFg).child(item.hm ?? item.ago ?? "");
    } else if (kind === "issue") {
      lead = Icon("bug", { size: 14, color: k.destructive });
      title = h_flex().gap(8).items_center().min_w(0).child(Mono(item.kind, { size: TEXT.sm, color: k.foreground })).child(div().flex_1().min_w(0).truncate().text_size(TEXT.sm).text_color(k.mutedFg).child(item.message));
      meta = Chip(`${item.count}×`, { mono: true, color: k.destructive });
    } else {
      lead = Icon("bell", { size: 14, color: k.destructive });
      title = div().truncate().text_size(TEXT.sm).child(item.name);
      meta = Badge(item.status, "destructive");
    }
    return h_flex()
      .id(`row-${row.key}`)
      .h(40)
      .pl(this.timeline ? 12 : 38)
      .pr(12)
      .gap(10)
      .items_center()
      .rounded(RADIUS.md)
      .relative()
      .cursor_pointer()
      .border_1()
      .border_color(this.marked(row) ? k.ring : alpha(k.border, 0))
      .bg(sel ? k.selected : k.background)
      .when(!sel, (e) => e.hover((s) => s.bg(k.accent)))
      .when(!this.timeline, (e) => e.child(div().absolute().left(20).top(0).w(1).h(row.last ? 20 : 40).bg(k.border)).child(div().absolute().left(20).top(20).w(10).h(1).bg(k.border)))
      .when(this.timeline, (e) => e.child(div().w(84).flex_none().truncate().text_size(TEXT.sm).font_medium().child(row.app.name)))
      .child(lead)
      .child(div().flex_1().min_w(0).child(title))
      .child(meta)
      .when(Boolean(flash), (e) => e.child(div().id(`fl-${row.key}`).absolute().inset_0().rounded(RADIUS.md).bg(alpha(flash?.color ?? k.ring, 0.25)).opacity(flash?.on ? 1 : 0).transition("opacity", { duration: 1100, easing: "ease-out" })));
  }

  list(k, cx) {
    const rows = this.rows();
    if (!rows.length) return Empty({ icon: "search", title: "Nothing matches", description: `No app, release or error has every word of “${this.filter}”.` });
    const at = rows.findIndex((r) => r.key === this.sel);
    const items = rows.map((row) => {
      const sel = row.key === this.sel;
      const el = (row.kind === "app" ? this.appRow(row, k, sel) : this.childRow(row, k, sel))
        .on_click((e, cx) => {
          const already = this.sel === row.key;
          this.select(row.key);
          this.handle.focus();
          if (row.kind === "app" && (already || e.click_count > 1)) this.toggle(row.key);
          else if (row.kind === "release" && e.click_count > 1) popOut(row.key, titleOf(row)[0]);
          cx.notify();
        })
        .on_mouse_down("right", (_e, cx) => {
          this.select(row.key);
          this.menu(row);
          cx.notify();
        });
      return { height: row.kind === "app" ? 56 : 40, el };
    });
    return this.scroller.view("monitor-list", items, at, { gap: 2, padding: 4 });
  }

  side(k, cx) {
    const e = this.selected();
    if (!e) return null;
    const [title, description] = titleOf(e);
    return SidePanel("monitor-side", {
      title,
      description,
      width: 400,
      children: details(e, { run: (a, cx) => this.run(a, cx), open: (u, cx) => this.open(u, cx) }),
      footer: h_flex()
        .w_full()
        .gap(8)
        .child(Button("side-window", { label: "Open in a window", icon: "picture-in-picture", variant: "ghost", size: "sm", kbd: "W", onClick: () => popOut(e.item.key, title) }))
        .child(div().flex_1())
        .when(Boolean(e.app.datadogUrl), (x) => x.child(Button("side-dd", { label: "Datadog", icon: "external", variant: "outline", size: "sm", onClick: (_e, cx) => this.open(e.item.windowUrl ?? e.app.datadogUrl, cx) }))),
    });
  }

  body(k, cx) {
    const m = this.model;
    if (!m) {
      if (this.status.startsWith("error")) return ErrorState({ title: "Cannot reach the cockpit hub", message: this.status.slice(7) });
      return v_flex().flex_1().p(24).gap(16).child(h_flex().gap(12).children([0, 1, 2, 3].map(() => Skeleton({ h: 96 })))).children([0, 1, 2, 3, 4].map(() => Skeleton({ h: 48 })));
    }
    if (!m.ready)
      return Empty({
        icon: "activity",
        title: "No monitor data yet",
        description: m?.connected
          ? "The hub reads your deploy platform and monitoring, then this fills in."
          : "No deploy platform or monitoring is connected. cockpit setup adds them.",
        action: Button("empty-refresh", { label: "Refresh", icon: "refresh-cw", kbd: "R", onClick: (_e, cx) => this.refresh(cx) }),
      });
    const wide = this.width >= SIDE_AT;
    const changes = [...this.fresh.entries()].filter(([key, at]) => at > (this.cleared.get(key) ?? 0)).length;
    return h_flex()
      .flex_1()
      .min_h(0)
      .child(
        v_flex()
          .flex_1()
          .h_full()
          .min_w(0)
          .min_h(0)
          .child(v_flex().px(24).pt(16).pb(12).gap(12).flex_none().child(this.tiles(k, cx)).when(m.errors.length > 0, (e) => e.child(div().text_size(TEXT.sm).text_color(k.destructive).child(m.errors.join(" · ")))))
          .child(
            h_flex()
              .px(24)
              .pb(8)
              .gap(8)
              .flex_none()
              .items_center()
              .child(Tabs("monitor-view", { items: [{ value: "apps", label: "Apps", icon: "server", count: m.apps.length }, { value: "releases", label: "Releases", icon: "history", count: m.timeline.length }], value: this.timeline ? "releases" : "apps", onChange: (v, cx) => ((this.timeline = v === "releases"), cx.notify()) }))
              .child(
                h_flex()
                  .id("monitor-filter")
                  .flex_1()
                  .on_mouse_down("left", (_e, cx) => ((this.filtering = true), cx.notify()))
                  .child(SearchField(this.search, { active: this.filtering, kbd: "/" })),
              )
              .when(changes > 0, (e) => e.child(Badge(`${changes} new since you looked`, "info", { dot: k.ring }))),
          )
          .child(v_flex().flex_1().min_h(0).px(20).child(this.list(k, cx))),
      )
      .when(wide, (e) => e.child(this.side(k, cx) ?? div()));
  }

  render(cx) {
    const k = tokens();
    const m = this.model;
    const e = this.selected();
    const acts = e?.item.acts ?? [];
    const description = m?.ready ? `${m.env} · read ${m.ago ?? "just now"}` : "Production after each release";
    const root = v_flex()
      .id("monitor")
      .track_focus(this.handle)
      .size_full()
      .relative()
      .bg(k.background)
      .text_color(k.foreground)
      .text_size(TEXT.base)
      .on_size_change((size, cx) => {
        if (Math.abs(size.width - this.width) > 4) {
          this.width = size.width;
          cx.notify();
        }
      })
      .on_mouse_down("left", () => {
        this.visible = true;
        if (!this.filtering && !this.palette) this.handle.focus();
      })
      .on_key_down((ev, cx) => {
        if (this.key(ev, cx)) cx.stop_propagation();
      })
      .child(
        PageHeader({
          icon: "activity",
          title: "Monitor",
          description,
          aside: m?.ready ? (m.bad ? Badge(m.headline, ["failed", "alert"].includes(m.verdict) ? "destructive" : "warning", { dot: tone(m.verdict, k) }) : Badge(m.headline || "all good", "success", { dot: k.success })) : null,
          actions: [
            Button("monitor-commands", { icon: "command", variant: "ghost", size: "icon", tooltip: "Commands  ctrl+p", onClick: (_e, cx) => this.openPalette(cx) }),
            Button("monitor-refresh", { label: "Refresh", icon: "refresh-cw", variant: "outline", size: "sm", kbd: "R", onClick: (_e, cx) => this.refresh(cx) }),
          ],
        }),
      )
      .child(this.body(k, cx))
      .when(this.width < SIDE_AT && acts.length > 0, (x) =>
        x.child(
          h_flex()
            .px(24)
            .py(10)
            .gap(8)
            .flex_none()
            .items_center()
            .border_t_1()
            .border_color(k.border)
            .bg(alpha(k.muted, 0.5))
            .child(div().flex_1().min_w(0).child(actionButtons("bar", acts, (a, cx) => this.run(a, cx))))
            .child(Button("bar-window", { icon: "picture-in-picture", variant: "ghost", size: "icon-sm", tooltip: "Open in a window  W", onClick: () => popOut(e.item.key, titleOf(e)[0]) })),
        ),
      )
      .child(KeyHints([["j k", "move"], ["⏎", "main action"], ["space", "fold"], ["w", this.timeline ? "apps" : "releases"], ["W", "window"], ["m", "menu"], ["/", "filter"], ["ctrl p", "commands"]]))
      .when(this.status !== "live" && this.status !== "connecting", (x) =>
        x.child(div().absolute().top(20).right(140).child(Badge("offline", "destructive", { dot: k.destructive }))),
      );
    let out = root;
    const toasts = this.toasts.render();
    if (toasts) out = out.child(toasts);
    if (this.confirm) {
      const a = this.confirm;
      out = out.child(
        AlertDialog("monitor-confirm", {
          title: `${a.label[0].toUpperCase() + a.label.slice(1)}?`,
          description: a.question,
          confirmLabel: a.label[0].toUpperCase() + a.label.slice(1),
          destructive: a.id === "roll_back",
          onCancel: (_e, cx) => ((this.confirm = null), cx.notify()),
          onConfirm: (_e, cx) => {
            this.confirm = null;
            cx.spawn(async (cx) => send(this.toasts, cx, a, a.confirm));
            cx.notify();
          },
        }),
      );
    }
    if (this.palette) {
      const groups = this.paletteGroups(cx);
      const all = groups.flatMap((g) => g.items);
      this.palette.at = Math.min(this.palette.at, Math.max(0, all.length - 1));
      const wrapped = groups.map((g) => ({ ...g, items: g.items.map((i) => ({ ...i, onSelect: (_e, cx) => this.runPalette(cx, i) })) }));
      out = out.child(
        div()
          .id("monitor-palette-backdrop")
          .absolute()
          .inset_0()
          .bg("#00000055")
          .on_mouse_down("left", (_e, cx) => this.closePalette(cx))
          .child(
            v_flex()
              .absolute()
              .top(72)
              .left(0)
              .right(0)
              .items_center()
              .child(
                v_flex()
                  .id("monitor-palette")
                  .w(580)
                  .max_h(440)
                  .rounded(RADIUS.lg)
                  .border_1()
                  .border_color(k.border)
                  .bg(k.popover)
                  .shadow_lg()
                  .overflow_hidden()
                  .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
                  .child(Command("monitor-commands-list", { state: this.query, groups: wrapped, selected: all[this.palette.at]?.id })),
              ),
          ),
      );
    }
    return out;
  }
}

// ---------------------------------------------------------------------------------------------------------------------
// One row as a window of its own (`props.key`): kept across restarts until closed, following the hub's monitor view

export class MonitorWindow {
  init(_props, cx) {
    this.key = props()?.key;
    this.title = props()?.title ?? "Release";
    this.model = null;
    this.byKey = new Map();
    this.confirm = null;
    this.toasts = new Toaster();
    this.handle = cx.focus_handle();
    this.sub = hub.subscribe(cx, { views: ["monitor"] }, (m, cx) => {
      if (m.view !== "monitor") return;
      this.model = m.value;
      this.byKey = index(m.value);
      cx.notify();
    });
  }

  close() {
    this.sub?.stop();
  }

  /** `cockpit.monitor:close-windows` closes every release window, `monitor:close:<key>` this one. */
  command(c) {
    if (c === "monitor:close-windows" || c === `monitor:close:${this.key}`) closeOverlay(props().overlay);
  }

  run(a, cx) {
    if (a.confirm) this.confirm = a;
    else cx.spawn(async (cx) => send(this.toasts, cx, a));
    cx.notify();
  }

  render(cx) {
    const k = tokens();
    const e = this.byKey.get(this.key);
    const [title, description] = e ? titleOf(e) : [this.title, this.model ? "no longer in the monitor" : "loading…"];
    const stop = (el) => el.on_mouse_down("left", (_e, cx) => cx.stop_propagation());
    let out = v_flex()
      .id("monitor-window")
      .track_focus(this.handle)
      .size_full()
      .relative()
      .rounded(RADIUS.xl)
      .border_1()
      .border_color(k.border)
      .bg(k.background)
      .text_color(k.foreground)
      .shadow_lg()
      .overflow_hidden()
      .on_key_down((ev, cx) => {
        if (ev.keystroke !== "escape") return;
        if (this.confirm) this.confirm = null;
        else closeOverlay(props().overlay);
        cx.stop_propagation();
        cx.notify();
      })
      .child(
        h_flex()
          .h(48)
          .px(14)
          .gap(10)
          .flex_none()
          .items_center()
          .border_b_1()
          .border_color(k.border)
          .bg(k.chrome)
          .child(e ? Icon(state(e.kind === "app" ? e.item.state : e.item.state ?? "alert")[1], { size: 16, color: tone(e.item.state ?? "alert", k) }) : Icon("activity", { size: 16, color: k.mutedFg }))
          .child(v_flex().flex_1().min_w(0).child(div().truncate().text_size(TEXT.base).font_semibold().child(title)).child(div().truncate().text_size(TEXT.xs).text_color(k.mutedFg).child(description)))
          .child(stop(Button("window-close", { icon: "x", variant: "ghost", size: "icon-sm", tooltip: "Close", onClick: () => closeOverlay(props().overlay) }))),
      )
      .child(
        stop(
          v_flex()
            .id("monitor-window-body")
            .flex_1()
            .min_h(0)
            .p(16)
            .gap(12)
            .overflow_y_scrollbar()
            .children(e ? details(e, { run: (a, cx) => this.run(a, cx), open: (u, cx) => cx.spawn(async (cx) => send(this.toasts, cx, { action: "open_url", args: [u] })), compact: true }) : [Skeleton({ h: 64 }), Skeleton({ h: 120 })]),
        ),
      );
    const toasts = this.toasts.render();
    if (toasts) out = out.child(toasts);
    if (this.confirm) {
      const a = this.confirm;
      out = out.child(
        AlertDialog("window-confirm", {
          title: `${a.label[0].toUpperCase() + a.label.slice(1)}?`,
          description: a.question,
          confirmLabel: a.label[0].toUpperCase() + a.label.slice(1),
          destructive: a.id === "roll_back",
          onCancel: (_e, cx) => ((this.confirm = null), cx.notify()),
          onConfirm: (_e, cx) => {
            this.confirm = null;
            cx.spawn(async (cx) => send(this.toasts, cx, a, a.confirm));
            cx.notify();
          },
        }),
      );
    }
    return out;
  }
}
