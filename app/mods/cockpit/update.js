// The update tag's modal: what the published cockpit changes, and a button that installs it. The hub's six-hourly
// check writes the version and the changelog's sections above the running one to update.json; Install runs
// `cockpit focus update` (update.sh in the background) and the modal follows update.log until cockpit.json names
// the new version.

import { div } from "gpui-kit";
import { h_flex, v_flex } from "gpui-base";
import { closeOverlay, openOverlay, props, readTextFile } from "herdr";
import * as hub from "./hub.js";
import { tokens, alpha, TEXT, RADIUS, Icon, Button, Spinner, Mono } from "./ui.js";

const RELEASES = "https://github.com/quaresc/cockpit/releases/tag/v";

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

export function openUpdate() {
  openOverlay({ mod: "cockpit", props: { app: "update" }, kind: "dialog", width: 620, height: 560 });
}

export default class Update {
  init(_props, cx) {
    this.state = null;
    this.loaded = false;
    this.installing = false;
    this.step = null;
    this.done = null;
    this.error = null;
    this.handle = cx.focus_handle();
    this.handle.focus();
    cx.spawn(async (cx) => {
      try {
        this.state = await pending();
      } catch (e) {
        this.error = String(e?.message ?? e);
      }
      this.loaded = true;
      cx.notify();
    });
  }

  dismiss() {
    closeOverlay(props().overlay);
  }

  key(e) {
    if (e.key !== "escape") return false;
    this.dismiss();
    return true;
  }

  /** update.sh in the background, its output followed until cockpit.json names the published version. */
  start(cx) {
    this.installing = true;
    this.error = null;
    cx.notify();
    cx.spawn(async (cx) => {
      try {
        const r = await hub.cockpit(["focus", "update"]);
        if (r.code !== 0) throw new Error(r.stderr.trim() || "the update did not start");
      } catch (e) {
        this.installing = false;
        this.error = String(e?.message ?? e);
        cx.notify();
        return;
      }
      const until = Date.now() + 10 * 60 * 1000;
      while (this.installing) {
        await hub.sleep(cx, 1500);
        const log = (await readTextFile(`${this.state.cache}/update.log`)) ?? "";
        this.step = log.split("\n").map((l) => l.trim()).filter(Boolean).pop() ?? null;
        try {
          const { version } = await hub.install();
          if (version && !isNewer(this.state.latest, version)) {
            this.installing = false;
            this.done = version;
          }
        } catch {}
        if (this.installing && Date.now() > until) {
          this.installing = false;
          this.error = "the update is still running; its output is in update.log";
        }
        cx.notify();
      }
    });
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

  render(cx) {
    const k = tokens();
    const s = this.state;
    const waiting = this.error ?? (this.loaded ? "You are on the latest cockpit." : "Reading what is published…");
    const body = !s
      ? [h_flex().h(160).justify_center().items_center().gap(8).text_color(k.mutedFg).when(!this.loaded && !this.error, (e) => e.child(Spinner({ size: 14 }))).when(Boolean(this.error), (e) => e.child(Icon("triangle-alert", { size: 14 }))).child(waiting)]
      : s.notes.length
        ? s.notes.map((n) => this.section(k, n))
        : [div().text_size(TEXT.base).text_color(k.mutedFg).child("No notes were published for this version.")];
    return v_flex()
      .id("update")
      .track_focus(this.handle)
      .size_full()
      .bg(k.popover)
      .text_color(k.foreground)
      .on_key_down((e, cx) => {
        if (this.key(e)) {
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
          .child(h_flex().size(30).flex_none().items_center().justify_center().rounded(RADIUS.md).bg(alpha(k.done, 0.16)).child(Icon("arrow-up", { size: 16, color: k.done })))
          .child(
            v_flex()
              .flex_1()
              .min_w(0)
              .gap(2)
              .child(div().text_size(TEXT.xl).font_semibold().child(`Cockpit ${s?.latest ?? ""}`.trim()))
              .when(Boolean(s), (e) => e.child(h_flex().gap(6).items_center().text_size(TEXT.sm).text_color(k.mutedFg).child("You run").child(Mono(s.current)))),
          )
          .child(Button("update-close", { icon: "x", variant: "ghost", size: "icon-sm", tooltip: "Close  esc", onClick: () => this.dismiss() })),
      )
      .child(div().h(1).bg(k.border))
      .child(v_flex().id("update-notes").flex_1().min_h(0).px(20).py(16).gap(18).overflow_y_scrollbar().children(body))
      .child(div().h(1).bg(k.border))
      .child(
        h_flex()
          .px(20)
          .py(14)
          .gap(10)
          .items_center()
          .child(
            div()
              .flex_1()
              .min_w(0)
              .truncate()
              .text_size(TEXT.sm)
              .text_color(this.error ? k.destructive : k.mutedFg)
              .child(this.error ?? this.step ?? (this.done ? `Now on ${this.done}.` : "Updates the Herdr app and TUI, then restarts them.")),
          )
          .when(this.installing, (e) => e.child(h_flex().gap(8).items_center().text_size(TEXT.sm).text_color(k.mutedFg).child(Spinner({ size: 13 })).child("Updating…")))
          .when(Boolean(s) && !this.installing && !this.done, (e) =>
            e
              .child(Button("update-notes-open", { label: "Release notes", icon: "external", variant: "ghost", size: "sm", onClick: () => hub.openUrl(`${RELEASES}${s.latest}`) }))
              .child(Button("update-install", { label: `Install ${s.latest}`, icon: "arrow-up", size: "sm", onClick: (_e, cx) => this.start(cx) })),
          )
          .when(Boolean(this.done), (e) => e.child(Button("update-done", { label: "Done", size: "sm", onClick: () => this.dismiss() }))),
      );
  }
}
