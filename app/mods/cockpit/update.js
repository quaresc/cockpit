// The update tag's dialog, on the hub's update.json: what the published cockpit changes, and Install, which runs
// `cockpit focus update` and follows update.log until cockpit.json names the new version. Once installed, by it or any
// other way (`askAfterUpdate`), it shows what changed, the options it brought (saved only by the person's click or key)
// and the new layout to try.

import { div } from "gpui-kit";
import { h_flex, v_flex } from "gpui-base";
import { closeOverlay, notify, openOverlay, props, readTextFile, run, setShared, shared } from "herdr";
import * as hub from "./hub.js";
import { take } from "./person.js";
import { layoutName } from "./screens.js";
import { ago } from "./fresh.js";
import { tokens, alpha, TEXT, RADIUS, Icon, Button, Spinner, Mono, Toggle, Badge } from "./ui.js";

const RELEASES = "https://github.com/quaresc/cockpit/releases/tag/v";

/** An install this window started, followed again by the dialog the mod's reload in the middle of it makes anew. */
const FLOW = "cockpit.update.flow";

/** How long an install is followed before its output is pointed to instead. */
const INSTALL_MS = 10 * 60 * 1000;

/** This load of the module: the start check runs once per load, so the mod an update reloads asks again. */
const LOAD = {};

/** The layout's name in words. */
const LAYOUT_TITLES = { "cockpit-chat": "Cockpit Chat" };

/** Whether `a` is a later version than `b`, number by number, as the hub compares them. */
export function isNewer(a, b) {
  const parts = (v) => String(v ?? "").replace(/^v/, "").split(".").map((p) => Number(p) || 0);
  const [x, y] = [parts(a), parts(b)];
  for (let i = 0; i < Math.max(x.length, y.length); i++) if ((x[i] ?? 0) !== (y[i] ?? 0)) return (x[i] ?? 0) > (y[i] ?? 0);
  return false;
}

/** The published version worth installing: {current, latest, notes, cache}, or null when this install is current,
 *  builds itself, or is the read-only client of a cockpit on another machine. */
export async function pending() {
  const install = await hub.install();
  if (install.machine) return null;
  const state = JSON.parse((await readTextFile(`${install.cache}/update.json`)) ?? "{}");
  const current = install.version ?? state.current;
  if (state.dev || !state.latest || !current || !isNewer(state.latest, current)) return null;
  return { current, latest: state.latest, notes: (state.notes ?? []).filter((n) => isNewer(n.version, current)), cache: install.cache };
}

/**
 * What runs here and what is published, from cockpit.json and the update.json the hub's checks write: {current, latest,
 * at, dev, newer, error}. `at` is the last look (`looked_at` for one asked for) and `error` why it failed when it did.
 */
export async function versions() {
  const install = await hub.install();
  const state = JSON.parse((await readTextFile(`${install.cache}/update.json`)) ?? "{}");
  const current = install.version ?? state.current ?? null;
  const latest = typeof state.latest === "string" && state.latest ? state.latest : null;
  // The last look, the six-hourly one or one asked for; its error is the one to say.
  const last = Math.max(Number(state.at) || 0, Number(state.looked_at) || 0);
  const error = state.error && Number(state.error.at) === last ? String(state.error.message ?? "") || null : null;
  return { current, latest, at: last || null, dev: Boolean(state.dev), newer: Boolean(latest && current && isNewer(latest, current)), error };
}

/** A check asked for now, through the hub: the versions after it, `error` saying why when it failed. */
export async function checkNow(cx) {
  let failed = null;
  try {
    const r = await hub.act("update", "check", [], cx);
    if (r?.unknown) failed = "this cockpit's hub cannot check on demand yet: update it first";
    else if (r?.error) failed = String(r.error).replace(/^check: /, "");
  } catch (e) {
    failed = String(e?.message ?? e);
  }
  const found = await versions().catch(() => ({ current: null, latest: null, at: null, dev: false, newer: false, error: null }));
  return failed ? { ...found, error: failed } : found;
}

/**
 * The versions in one line, and what the line offers: {text, state: checking | failed | never | out | current, update}.
 * `update` is true when the published version installs from here (not on an install that builds itself).
 */
export function versionLine(v, { checking = false, now = Date.now() } = {}) {
  const runs = v?.current ? `Cockpit ${v.current}${v.dev ? ", your own build" : ""}` : "Cockpit";
  if (checking) return { text: `${runs} · checking for updates…`, state: "checking", update: false };
  if (!v) return { text: runs, state: "never", update: false };
  const out = v.newer ? `${v.latest} is ${v.dev ? "published" : "out"}` : null;
  if (v.error) return { text: `${runs} · the check failed: ${v.error}${out ? ` · ${out}` : ""}`, state: "failed", update: Boolean(out) && !v.dev };
  if (!v.at || !v.latest) return { text: `${runs} · updates not checked yet`, state: "never", update: false };
  const checked = `checked ${ago(v.at, now)}`;
  if (out) return { text: `${runs} · ${out} · ${checked}`, state: "out", update: !v.dev };
  return { text: `${runs} · up to date · ${checked}`, state: "current", update: false };
}

/**
 * The dialog; `after` opens it on what the last update brought, `person` when the person's click, key or palette pick
 * opened it, so what it shows counts as seen at once.
 */
export function openUpdate({ after = false, person = false } = {}) {
  openOverlay({ mod: "cockpit", props: { app: "update", after, person: Boolean(person) }, kind: "dialog", width: 620, height: 580 });
}

/** Whether the update the hub noted has something to say in this window: notes, options it brought, a layout to try. */
export function worthAsking(view, layout) {
  const u = view?.update;
  if (!u || u.shown !== false || view.error) return false;
  return Boolean(u.notes?.length) || (view.options ?? []).some((o) => o.new) || Boolean(view.layout && view.layout !== layout);
}

/**
 * What the steps after an install show, {from, to, notes, options, layout}: the hub's `options` view, or the install this
 * dialog followed when the hub noted none for it (a hub not restarted yet).
 */
export function afterOf(view, flow) {
  const u = view && !view.error ? view.update : null;
  const noted = u && (!flow || u.to === flow.to) ? u : null;
  const from = noted?.from ?? flow?.from ?? null;
  return {
    from,
    to: noted?.to ?? flow?.to ?? null,
    notes: noted ? (noted.notes ?? []) : (flow?.notes ?? []),
    // The dialog asks about switches; an option of another kind an update brings is set in Cockpit settings.
    options: (view && !view.error ? view.options ?? [] : []).filter((o) => (o.kind ?? "toggle") === "toggle" && (noted ? o.new : Boolean(from) && Boolean(o.since) && isNewer(o.since, from))),
    layout: noted ? (view.layout ?? null) : null,
  };
}

/** The steps after an install: what's new when there are notes, the options it brought, the layout to try when the window shows another. */
export function afterSteps(after, layout) {
  const steps = [];
  if (after.notes.length) steps.push("news");
  if (after.options.length) steps.push("options");
  if (after.layout && after.layout !== layout) steps.push("layout");
  return steps.length ? steps : ["news"];
}

/** The options a person changed, as the hub's `options set` takes them: {key: on}. */
export function changed(options, picks) {
  return Object.fromEntries(options.filter((o) => picked(picks, o) !== Boolean(o.value)).map((o) => [o.key, picked(picks, o)]));
}

/** An option's value as picked in the dialog, else as it is. */
const picked = (picks, o) => (o.key in picks ? Boolean(picks[o.key]) : Boolean(o.value));

/** The install this window started, while it is recent enough to follow. */
function followed() {
  const flow = shared(FLOW);
  return flow && Date.now() - (flow.at ?? 0) < INSTALL_MS ? flow : null;
}

/** Whether an update dialog beats in this window. */
const dialogOpen = () => Date.now() - (globalThis.cockpitUpdateBeat ?? 0) < 2500;

/**
 * Once per load of the mod, a little after the window starts: when an update not shown yet has something to say and no
 * dialog shows it, the dialog opens on it. A remote desk's updates are its runner's.
 */
export function askAfterUpdate(cx) {
  const g = (globalThis.cockpitAfterUpdate ??= {});
  if (g.load === LOAD && (g.done || Date.now() - g.at < 30_000)) return;
  Object.assign(g, { load: LOAD, at: Date.now(), done: false });
  cx.spawn(async (cx) => {
    try {
      await hub.sleep(cx, 2500);
      const install = await hub.install();
      // A dialog already open (one the reload made anew mid-install) takes the steps itself.
      if (install.machine || dialogOpen()) return;
      let view = null;
      for (let i = 0; i < 6 && !view; i++) {
        view = await hub.view("options").catch(() => null);
        if (!view) await hub.sleep(cx, 5000);
      }
      if (worthAsking(view, layoutName()) && !dialogOpen()) openUpdate({ after: true });
    } catch {
    } finally {
      g.done = true;
    }
  });
}

/**
 * The published cockpit as a tag, `u` being what pending() found: a click opens what it changes. `rail` is the icon
 * alone, and `mono` draws it without colour, where the layout keeps colour for what the agents do.
 */
export function UpdateTag(id, u, { rail = false, mono = false } = {}) {
  const k = tokens();
  const row = h_flex()
    .id(id)
    .mt(6)
    .flex_none()
    .items_center()
    .rounded(RADIUS.md)
    .bg(mono ? k.accent : alpha(k.done, 0.14))
    .cursor_pointer()
    .hover((s) => s.bg(mono ? k.selected : alpha(k.done, 0.24)))
    .on_click(() => openUpdate({ person: true }))
    .child(Icon("arrow-up", { size: 14, color: mono ? k.foreground : k.done }));
  if (rail) return row.h(28).justify_center().tooltip(`Cockpit ${u.latest} is out — you run ${u.current}`);
  return row
    .h(30)
    .px(8)
    .gap(8)
    .tooltip(`You run ${u.current}`)
    .child(div().flex_1().truncate().text_size(TEXT.sm).font_medium().text_color(k.foreground).child(`Update cockpit to ${u.latest}`))
    .child(Icon("chevron-right", { size: 14, color: k.mutedFg }));
}

export default class Update {
  init(p, cx) {
    this.p = p ?? props() ?? {};
    /** loading | notes | installing | reading | after | latest */
    this.phase = "loading";
    this.state = null;
    this.after = null;
    this.steps = [];
    this.at = 0;
    this.picks = {};
    this.row = 0;
    this.saving = false;
    this.step = null;
    this.done = null;
    this.error = null;
    // The dialog opens on its own after an update, or from any local process (`cockpit.update`): space and ⏎ change
    // nothing until the person moves in it (j/k) or clicks in it, and what it shows counts as seen only once the person
    // opened it or acted in it.
    this.pointed = true;
    this.hint = null;
    this.person = Boolean(this.p.person);
    this.noted = new Set();
    this.handle = cx.focus_handle();
    this.handle.focus();
    this.beat();
    cx.spawn(async (cx) => this.load(cx));
  }

  /** main.js's clock, every second: an open dialog says so, and the start check leaves the steps to it. */
  tick() {
    this.beat();
  }

  beat() {
    globalThis.cockpitUpdateBeat = Date.now();
  }

  dismiss() {
    closeOverlay(props().overlay);
  }

  async load(cx) {
    try {
      if (this.p.after) return await this.showAfter(cx, null);
      const flow = followed();
      this.state = await pending();
      // An install this window started before the mod reloaded: followed on, never started again.
      if (flow) return await this.follow(cx, flow);
      if (this.state) {
        this.phase = "notes";
        cx.notify();
        return await this.watch(cx);
      }
      const view = await hub.view("options").catch(() => null);
      if (worthAsking(view, layoutName())) return await this.showAfter(cx, null, view);
      this.phase = "latest";
    } catch (e) {
      this.error = String(e?.message ?? e);
      this.phase = "latest";
    }
    cx.notify();
  }

  /** While the notes show, an install finished from elsewhere moves the dialog on to what it brought. */
  async watch(cx) {
    while (this.phase === "notes") {
      await hub.sleep(cx, 3000);
      if (this.phase !== "notes") return;
      const now = await pending().catch(() => this.state);
      if (now) continue;
      const view = await hub.view("options").catch(() => null);
      if (worthAsking(view, layoutName())) return await this.showAfter(cx, null, view);
      this.phase = "latest";
      cx.notify();
    }
  }

  /** update.sh in the background, its output followed until cockpit.json names the published version. */
  start(cx) {
    if (this.phase !== "notes" || !this.state) return;
    const flow = { from: this.state.current, to: this.state.latest, notes: this.state.notes, at: Date.now() };
    setShared(FLOW, flow);
    this.phase = "installing";
    this.error = null;
    cx.notify();
    cx.spawn(async (cx) => {
      try {
        const r = await hub.cockpit(["focus", "update"]);
        if (r.code !== 0) throw new Error(r.stderr.trim() || "the update did not start");
      } catch (e) {
        setShared(FLOW, null);
        this.phase = "notes";
        this.error = String(e?.message ?? e);
        cx.notify();
        return;
      }
      await this.follow(cx, flow);
    });
  }

  async follow(cx, flow) {
    this.phase = "installing";
    cx.notify();
    while (this.phase === "installing") {
      try {
        const install = await hub.install();
        const log = (await readTextFile(`${install.cache}/update.log`)) ?? "";
        this.step = log.split("\n").map((l) => l.trim()).filter(Boolean).pop() ?? null;
        if (install.version && !isNewer(flow.to, install.version)) {
          setShared(FLOW, null);
          return await this.showAfter(cx, flow);
        }
      } catch {}
      if (Date.now() - flow.at > INSTALL_MS) {
        setShared(FLOW, null);
        this.phase = "notes";
        this.error = "the update is still running; its output is in update.log";
      }
      cx.notify();
      await hub.sleep(cx, 1500);
    }
  }

  /** What the update brought, from the hub (asked again while it restarts with the update); noted as shown at once. */
  async showAfter(cx, flow, view = null) {
    this.phase = "reading";
    cx.notify();
    for (let i = 0; !view && i < 8; i++) {
      view = await hub.view("options").catch(() => null);
      if (!view) await hub.sleep(cx, 2500);
    }
    this.after = afterOf(view, flow);
    if (!this.after.to) {
      this.phase = "latest";
      cx.notify();
      return;
    }
    this.picks = Object.fromEntries(this.after.options.map((o) => [o.key, Boolean(o.value)]));
    this.steps = afterSteps(this.after, layoutName());
    this.at = 0;
    this.phase = "after";
    cx.notify();
    this.noteShown();
  }

  /** The person's own key or click in the dialog: from then on what it shows counts as seen. */
  acted() {
    this.person = true;
    this.noteShown();
  }

  /** The update, and the options of the step in front, noted as seen once each, when the person opened or acted in it. */
  noteShown() {
    if (!this.person || this.phase !== "after" || !this.after?.to) return;
    const note = (what, args) => !this.noted.has(what) && (this.noted.add(what), hub.act("options", "shown", [args]).catch(() => {}));
    note("update", { update: this.after.to });
    if (this.steps[this.at] === "options") note("options", { keys: this.after.options.map((o) => o.key) });
  }

  /** The next step, or the dialog closed after the last one, `said` as a toast. */
  next(cx, said = null) {
    this.error = null;
    if (this.at + 1 >= this.steps.length) {
      // Cockpit Chat keeps colour for what the agents do.
      if (said) notify({ title: said, level: layoutName() === "cockpit-chat" ? "info" : "success", timeout: 5000 });
      this.dismiss();
      return;
    }
    this.at += 1;
    this.done = said;
    this.noteShown();
    cx.notify();
  }

  /** Save: the options the person changed, through the hub, then the next step. Skip keeps them as they are. */
  save(cx) {
    if (this.saving || this.phase !== "after" || this.steps[this.at] !== "options") return;
    const values = changed(this.after.options, this.picks);
    if (!Object.keys(values).length) return this.next(cx, "Options kept as they were");
    this.saving = true;
    this.error = null;
    cx.notify();
    cx.spawn(async (cx) => {
      try {
        const r = await hub.act("options", "set", [values], cx);
        if (r?.error) throw new Error(r.error);
        if (r?.unknown) throw new Error("this cockpit cannot save options: update it first");
        this.saving = false;
        this.after.options = this.after.options.map((o) => ({ ...o, value: picked(this.picks, o) }));
        this.next(cx, "Options saved");
      } catch (e) {
        this.saving = false;
        this.error = String(e?.message ?? e);
        cx.notify();
      }
    });
  }

  /** Skip: the options left as they are, and the next step. */
  skip(cx) {
    if (this.saving || this.phase !== "after" || this.steps[this.at] !== "options") return;
    this.picks = Object.fromEntries(this.after.options.map((o) => [o.key, Boolean(o.value)]));
    this.next(cx);
  }

  /** The new layout, with a toast saying how to come back. */
  tryLayout() {
    const name = this.after?.layout;
    if (this.phase !== "after" || !name) return;
    notify({ title: `This is ${LAYOUT_TITLES[name] ?? name}`, body: "⌘⇧L switches back to Cockpit.", level: "info", timeout: 8000 });
    this.dismiss();
    run("layout.switch", { name });
  }

  /** The step's main action: Next, Save or Try. */
  primary(cx) {
    if (this.phase !== "after") return;
    const step = this.steps[this.at];
    if (step === "options") this.save(cx);
    else if (step === "layout") this.tryLayout();
    else this.next(cx);
  }

  flip(k, cx) {
    if (!k || this.saving) return;
    this.picks[k] = !this.picks[k];
    cx?.notify();
  }

  /**
   * Esc closes; j/k pick an option; space and ⏎ change something, from the person's own keys only, never held, and
   * once they moved in the dialog or clicked in it.
   */
  key(e, cx) {
    const k = e.keystroke;
    const own = !e.injected && !e.is_held;
    if (own) this.acted();
    if (k === "escape") return this.dismiss(), true;
    if (this.phase !== "after") return false;
    const rows = this.after.options;
    const onOptions = this.steps[this.at] === "options";
    const move = ["j", "k", "down", "up"].includes(k);
    if (move && own) this.pointed = false;
    if (onOptions && (k === "j" || k === "down")) return (this.row = Math.min(this.row + 1, rows.length - 1)), true;
    if (onOptions && (k === "k" || k === "up")) return (this.row = Math.max(this.row - 1, 0)), true;
    const acts = k === "enter" || (onOptions && k === "space");
    if (!acts) return false;
    if (!own) return true;
    if (this.pointed) return (this.hint = "Press j or k first, or click"), true;
    this.hint = null;
    if (k === "space") return this.flip(rows[this.row]?.key, cx), true;
    return this.primary(cx), true;
  }

  /** A click in the dialog: the person's, so its keys act from now on. */
  clicked(cx) {
    this.pointed = false;
    this.hint = null;
    this.acted();
    cx.notify();
  }

  /** `update:<next|save|skip|try>` from a menu or the palette, by its key: a window command alone does nothing here. */
  command(c, cx, key = null) {
    if (!c.startsWith("update:") || !take(c, key)) return;
    this.acted();
    const what = c.slice("update:".length);
    if (what === "save") this.save(cx);
    else if (what === "skip") this.skip(cx);
    else if (what === "try") this.tryLayout();
    else if (what === "next") this.primary(cx);
    cx.notify();
  }

  section(k, note) {
    return v_flex()
      .gap(6)
      .child(
        h_flex()
          .gap(8)
          .items_baseline()
          .child(div().text_size(TEXT.lg).font_semibold().text_color(k.foreground).child(note.version))
          .when(Boolean(note.date), (e) => e.child(div().text_size(TEXT.sm).text_color(k.dim).child(note.date))),
      )
      .children(
        (note.changes ?? []).map((change) =>
          h_flex()
            .gap(8)
            .items_start()
            .child(div().mt(6).size(4).flex_none().rounded(2).bg(k.dim))
            .child(div().flex_1().min_w(0).whitespace_normal().text_size(TEXT.base).text_color(k.mutedFg).child(change)),
        ),
      );
  }

  optionRow(k, o, i) {
    const on = picked(this.picks, o);
    const select = (cx) => {
      this.row = i;
      this.clicked(cx);
      this.flip(o.key, cx);
    };
    return h_flex()
      .id(`update-option-${o.key}`)
      .gap(12)
      .p(12)
      .flex_none()
      .items_start()
      .rounded(RADIUS.lg)
      .border_1()
      .border_color(i === this.row ? k.ring : k.border)
      .cursor_pointer()
      .hover((s) => s.bg(k.accent))
      .on_click((_e, cx) => select(cx))
      .child(
        v_flex()
          .flex_1()
          .min_w(0)
          .gap(3)
          .child(div().text_size(TEXT.base).font_medium().text_color(k.foreground).child(o.title))
          .child(div().whitespace_normal().text_size(TEXT.sm).text_color(k.mutedFg).child(o.description))
          .child(div().text_size(TEXT.xs).text_color(k.dim).child(`${o.default ? "On" : "Off"} by default · ${o.key}`)),
      )
      .child(div().pt(2).child(Toggle(`update-toggle-${o.key}`, on, select, { tooltip: `${on ? "Turn it off" : "Turn it on"}  space` })));
  }

  layoutBody(k) {
    const line = (icon, text) =>
      h_flex()
        .gap(10)
        .items_start()
        .child(div().pt(1).child(Icon(icon, { size: 15, color: k.mutedFg })))
        .child(div().flex_1().min_w(0).whitespace_normal().text_size(TEXT.base).text_color(k.foreground).child(text));
    return v_flex()
      .gap(14)
      .child(div().whitespace_normal().text_size(TEXT.base).text_color(k.mutedFg).child("Cockpit Chat reads one agent at a time, like a conversation. Your Cockpit layout stays as it is."))
      .child(line("message-square", "Your prompt, one folded line for the work the agent did, then its answer."))
      .child(line("panel-right", "The sidebar and the side panel slide in when you need them, and stay when you pin them."))
      .child(line("keyboard", "⌘⇧L switches back to Cockpit at any time, as do the palette's Layout entries and Cockpit settings, under Layout."));
  }

  /** The header's title and the line under it, for the phase and step shown. */
  heading() {
    const a = this.after;
    if (this.phase === "after") {
      const step = this.steps[this.at];
      if (step === "options") return ["New options", a.from ? `New since ${a.from}: turn on what you want` : "Turn on what you want"];
      if (step === "layout") return [`Try ${LAYOUT_TITLES[a.layout] ?? a.layout}`, "The new layout, one agent at a time"];
      return [`Cockpit ${a.to} is installed`, a.from ? `What changed since ${a.from}` : "What changed"];
    }
    if (this.phase === "reading") return ["Cockpit is updated", "Reading what changed…"];
    return [`Cockpit ${this.state?.latest ?? ""}`.trim(), null];
  }

  body(k) {
    const s = this.state;
    if (this.phase === "after") {
      const a = this.after;
      const step = this.steps[this.at];
      if (step === "options") return a.options.map((o, i) => this.optionRow(k, o, i));
      if (step === "layout") return [this.layoutBody(k)];
      if (!a.notes.length) return [div().text_size(TEXT.base).text_color(k.mutedFg).child("No notes were published for this version: Release notes has them all.")];
      return a.notes.map((n) => this.section(k, n));
    }
    if (s && (this.phase === "notes" || this.phase === "installing")) {
      return s.notes.length ? s.notes.map((n) => this.section(k, n)) : [div().text_size(TEXT.base).text_color(k.mutedFg).child("No notes were published for this version.")];
    }
    const busy = !this.error && (this.phase === "loading" || this.phase === "reading");
    const said = this.error ?? (this.phase === "latest" ? "You are on the latest cockpit." : this.phase === "reading" ? "Reading what the update brought…" : "Reading what is published…");
    return [
      h_flex()
        .h(160)
        .justify_center()
        .items_center()
        .gap(8)
        .text_color(k.mutedFg)
        .when(busy, (e) => e.child(Spinner({ size: 14 })))
        .when(Boolean(this.error), (e) => e.child(Icon("triangle-alert", { size: 14 })))
        .child(said),
    ];
  }

  footer(k) {
    const s = this.state;
    const row = h_flex().px(20).py(14).gap(10).items_center();
    const words = (text, color = k.mutedFg) => div().flex_1().min_w(0).truncate().text_size(TEXT.sm).text_color(color).child(text);
    if (this.phase === "after") {
      const a = this.after;
      const step = this.steps[this.at];
      const last = this.at + 1 >= this.steps.length;
      const said = (fallback) => (this.error ? words(this.error, k.destructive) : words(this.hint ?? (this.done ? `${this.done}. ${fallback}` : fallback)));
      if (step === "options") {
        return row
          .child(said("You can change them later in Cockpit settings, under Options."))
          .when(this.saving, (e) => e.child(Spinner({ size: 13 })))
          .child(Button("update-skip", { label: "Skip", variant: "ghost", size: "sm", disabled: this.saving, tooltip: "Keep them as they are", onClick: (_e, cx) => this.skip(cx) }))
          .child(Button("update-save", { label: last ? "Save" : "Save and continue", icon: "check", size: "sm", disabled: this.saving, kbd: "⏎", onClick: (_e, cx) => this.save(cx) }));
      }
      if (step === "layout") {
        return row
          .child(said("⌘⇧L brings Cockpit back."))
          .child(Button("update-keep", { label: "Keep Cockpit", variant: "ghost", size: "sm", onClick: () => this.dismiss() }))
          .child(Button("update-try", { label: `Try ${LAYOUT_TITLES[a.layout] ?? a.layout}`, iconRight: "chevron-right", size: "sm", kbd: "⏎", onClick: () => this.tryLayout() }));
      }
      return row
        .child(said(`Now on ${a.to}.`))
        .child(Button("update-notes-open", { label: "Release notes", icon: "external", variant: "ghost", size: "sm", onClick: () => hub.openUrl(`${RELEASES}${a.to}`) }))
        .child(Button("update-next", { label: last ? "Done" : "Next", iconRight: last ? undefined : "chevron-right", size: "sm", kbd: "⏎", onClick: (_e, cx) => this.next(cx) }));
    }
    const said = this.error ?? this.step ?? (this.phase === "notes" ? "Updates the Herdr app and TUI, then restarts them." : "");
    return row
      .child(words(said, this.error ? k.destructive : k.mutedFg))
      .when(this.phase === "installing", (e) => e.child(h_flex().gap(8).items_center().text_size(TEXT.sm).text_color(k.mutedFg).child(Spinner({ size: 13 })).child("Updating…")))
      .when(Boolean(s) && this.phase === "notes", (e) =>
        e
          .child(Button("update-notes-open", { label: "Release notes", icon: "external", variant: "ghost", size: "sm", onClick: () => hub.openUrl(`${RELEASES}${s.latest}`) }))
          .child(Button("update-install", { label: `Install ${s.latest}`, icon: "arrow-up", size: "sm", onClick: (_e, cx) => this.start(cx) })),
      )
      .when(this.phase === "latest", (e) => e.child(Button("update-done", { label: "Close", variant: "outline", size: "sm", onClick: () => this.dismiss() })));
  }

  render(cx) {
    const k = tokens();
    // Cockpit Chat keeps colour for what the agents do.
    const mono = layoutName() === "cockpit-chat";
    const [title, sub] = this.heading();
    const after = this.phase === "after";
    const icon = after ? ({ options: "square-check", layout: "layout" }[this.steps[this.at]] ?? "sparkles") : "arrow-up";
    return v_flex()
      .id("update")
      .track_focus(this.handle)
      .size_full()
      .bg(k.popover)
      .text_color(k.foreground)
      .on_mouse_down("left", (_e, cx) => this.clicked(cx))
      .on_key_down((e, cx) => {
        if (this.key(e, cx)) {
          cx.stop_propagation();
          cx.notify();
        }
      })
      .child(
        h_flex()
          .px(20)
          .pt(18)
          .pb(12)
          .gap(12)
          .items_center()
          .child(h_flex().size(30).flex_none().items_center().justify_center().rounded(RADIUS.md).bg(mono ? k.muted : alpha(k.done, 0.16)).child(Icon(icon, { size: 16, color: mono ? k.foreground : k.done })))
          .child(
            v_flex()
              .flex_1()
              .min_w(0)
              .gap(2)
              .child(div().text_size(TEXT.xl).font_semibold().child(title))
              .when(Boolean(sub), (e) => e.child(div().truncate().text_size(TEXT.sm).text_color(k.mutedFg).child(sub)))
              .when(!sub && Boolean(this.state), (e) => e.child(h_flex().gap(6).items_center().text_size(TEXT.sm).text_color(k.mutedFg).child("You run").child(Mono(this.state.current)))),
          )
          .when(after && this.steps.length > 1, (e) => e.child(Badge(`${this.at + 1} of ${this.steps.length}`, "outline")))
          .child(Button("update-close", { icon: "x", variant: "ghost", size: "icon-sm", tooltip: "Close  esc", onClick: () => this.dismiss() })),
      )
      .child(div().h(1).bg(k.border))
      .child(v_flex().id("update-body").flex_1().min_h(0).px(20).py(16).gap(after && this.steps[this.at] === "options" ? 10 : 18).overflow_y_scrollbar().children(this.body(k)))
      .child(div().h(1).bg(k.border))
      .child(this.footer(k));
  }
}
