// The Machines tab: each machine's CPU, memory, GPU and VRAM with the last two minutes, the tokens per second of its
// agents, and its oh-my-opencode-slim preset with the models each OMO agent uses; a preset is picked per machine here.

import { div } from "gpui-kit";
import { h_flex, v_flex, InputState } from "gpui-base";
import { notify } from "herdr";
import * as hub from "./hub.js";
import { tokens, TEXT, Icon, Badge, Button, Card, PageHeader, Empty, Mono, Progress, Sparkline, TextField } from "./ui.js";
import { PerfModel, OmoModel, tpsText, machineOf } from "./agentsui.js";

const gb = (mb) => `${(mb / 1024).toFixed(1)} GB`;

export default class Machines {
  init(_props, cx) {
    this.rows = null;
    this.perf = new PerfModel();
    this.perf.follow(cx, () => this.rows?.agents);
    this.omo = new OmoModel();
    this.omo.follow(cx);
    // Which preset of each machine is shown (default: the one in use), the one being edited, its fields, the delete asking twice.
    this.shown = new Map();
    this.edit = null;
    this.fields = new Map();
    this.confirmDelete = null;
    this.newName = InputState.new({ placeholder: "Name for a copy of the preset shown" });
  }

  close() {
    this.sub?.stop();
  }

  command() {}

  pick(machine, name, cx) {
    cx.spawn(async (cx) => {
      try {
        const r = await this.omo.set(machine, name);
        notify({ title: r.error ? "Could not change the OMO preset" : "OMO preset", body: r.error ?? r.note });
      } catch (e) {
        notify({ title: "Could not change the OMO preset", body: String(e?.message ?? e) });
      }
      cx.notify();
    });
  }

  async report(cx, r) {
    notify({ title: r.error ? "Could not do that" : "OMO presets", body: r.error ?? r.note });
    await this.omo.refresh?.(cx);
    cx.notify();
  }

  startEdit(key, preset, models) {
    for (const [agent, model] of Object.entries(models)) {
      const id = `${key}|${preset}|${agent}`;
      if (!this.fields.has(id)) this.fields.set(id, InputState.new({ placeholder: "model > fallback > fallback" }));
      this.fields.get(id).set_value(model);
    }
    this.edit = { key, preset };
  }

  /** A field's text as a model or, with ">" between, a fallback chain. */
  chainOf(text) {
    const parts = String(text).split(">").map((s) => s.trim()).filter(Boolean);
    return parts.length > 1 ? parts : parts[0] ?? null;
  }

  save(cx, machine, key, preset, models) {
    const out = {};
    for (const agent of Object.keys(models)) {
      const c = this.chainOf(this.fields.get(`${key}|${preset}|${agent}`)?.value() ?? models[agent]);
      if (c) out[agent] = c;
    }
    this.edit = null;
    cx.spawn(async (cx) => this.report(cx, await this.omo.edit(machine, preset, out).catch((e) => ({ error: String(e?.message ?? e) }))));
  }

  copy(cx, machine, from) {
    const name = this.newName.value().trim();
    if (!/^[A-Za-z0-9_.-]+$/.test(name)) return notify({ title: "Name the copy", body: "Letters, digits, dot, dash and underscore only." });
    this.newName.set_value("");
    cx.spawn(async (cx) => this.report(cx, await this.omo.edit(machine, name, {}, from).catch((e) => ({ error: String(e?.message ?? e) }))));
  }

  drop(cx, machine, key, preset) {
    const id = `${key}|${preset}`;
    if (this.confirmDelete !== id) {
      this.confirmDelete = id;
      cx.notify();
      return;
    }
    this.confirmDelete = null;
    this.shown.delete(key);
    cx.spawn(async (cx) => this.report(cx, await this.omo.remove(machine, preset).catch((e) => ({ error: String(e?.message ?? e) }))));
  }

  /** One meter: label, bar, %, and the last two minutes. */
  meter(k, id, label, p, history, tip) {
    const tone = p >= 90 ? k.destructive : p >= 70 ? k.warning : k.success;
    return h_flex()
      .id(id)
      .gap(10)
      .items_center()
      .tooltip(tip ?? label)
      .child(div().w(64).flex_none().text_size(TEXT.sm).text_color(k.mutedFg).child(label))
      .child(div().flex_1().child(Progress((p ?? 0) / 100, { color: tone, h: 6 })))
      .child(div().w(44).flex_none().text_right().child(Mono(`${Math.round(p ?? 0)}%`, { color: k.foreground })))
      .child(div().w(56).flex_none().child((history?.length ?? 0) > 1 ? Sparkline(history, { w: 56, h: 14, color: tone }) : div()));
  }

  machineCard(k, m, s) {
    const h = this.perf.sysHistory.get(m) ?? {};
    const mine = Object.entries(this.perf.value?.agents ?? {}).filter(([address]) => (machineOf(address) ?? null) === m);
    const tps = this.perf.tpsOf(m);
    const key = m ?? "here";
    return Card({
      id: `machine-${key}`,
      padding: 16,
      gap: 12,
      title: m ?? "This Mac",
      action: h_flex()
        .gap(8)
        .items_center()
        .child(Mono(`${mine.length} agent${mine.length === 1 ? "" : "s"}`, { color: k.mutedFg }))
        .when(tps > 0, (e) => e.child(Badge(tpsText(tps), "success"))),
      children: [
        this.meter(k, `mc-cpu-${key}`, "CPU", s.cpu, h.cpu, `${s.cores ?? "?"} cores`),
        s.mem_total ? this.meter(k, `mc-mem-${key}`, "Memory", (s.mem_used / s.mem_total) * 100, h.mem, `${gb(s.mem_used)} of ${gb(s.mem_total)}`) : null,
        s.gpu != null ? this.meter(k, `mc-gpu-${key}`, "GPU", s.gpu, h.gpu, "GPU busy") : null,
        s.vram_total ? this.meter(k, `mc-vram-${key}`, "VRAM", (s.vram_used / s.vram_total) * 100, h.vram, `${gb(s.vram_used)} of ${gb(s.vram_total)}`) : null,
      ].filter(Boolean),
    });
  }

  omoCard(k, m, v) {
    const key = m ?? "here";
    const shown = v.presets.includes(this.shown.get(key)) ? this.shown.get(key) : v.preset;
    const models = v.all?.[shown] ?? v.models ?? {};
    const editing = this.edit?.key === key && this.edit.preset === shown;
    const inUse = shown === v.preset;
    const asked = this.confirmDelete === `${key}|${shown}`;
    return Card({
      id: `omo-${key}`,
      padding: 16,
      gap: 12,
      title: `OMO presets · ${m ?? "This Mac"}`,
      children: [
        div().text_size(TEXT.xs).text_color(k.dim).child("Use (starts new sessions with it)"),
        h_flex()
          .gap(6)
          .flex_wrap()
          .children(v.presets.map((p) => Button(`omo-${key}-${p}`, { label: p, size: "sm", variant: p === v.preset ? "default" : "outline", tooltip: p === v.preset ? "In use here" : `Use ${p} on ${m ?? "this computer"}`, onClick: (_e, cx) => p !== v.preset && this.pick(m, p, cx) }))),
        div().text_size(TEXT.xs).text_color(k.dim).child("Models of"),
        h_flex()
          .gap(6)
          .flex_wrap()
          .children(v.presets.map((p) => Button(`omo-${key}-show-${p}`, { label: p, size: "sm", variant: p === shown ? "secondary" : "ghost", onClick: (_e, cx) => (this.edit = null, this.confirmDelete = null, this.shown.set(key, p), cx.notify()) }))),
        v_flex()
          .gap(editing ? 6 : 3)
          .children(
            Object.entries(models).map(([agent, model]) => {
              const label = div().w(96).flex_none().text_size(TEXT.sm).text_color(k.mutedFg).child(agent);
              if (editing) {
                const id = `${key}|${shown}|${agent}`;
                if (!this.fields.has(id)) this.startEdit(key, shown, models);
                return h_flex().id(`omo-${key}-f-${agent}`).gap(10).items_center().child(label).child(TextField(this.fields.get(id)));
              }
              return h_flex().id(`omo-${key}-m-${agent}`).gap(10).child(label).child(div().flex_1().min_w(0).truncate().text_size(TEXT.sm).font_family("JetBrains Mono").text_color(k.foreground).child(model));
            }),
          ),
        editing ? div().text_size(TEXT.xs).text_color(k.dim).child("A fallback chain: models separated by >, tried in order. Saving backs the file up as .bak-cockpit.") : null,
        h_flex()
          .gap(6)
          .flex_wrap()
          .children(
            [
              editing
                ? Button(`omo-${key}-save`, { label: "Save", size: "sm", onClick: (_e, cx) => this.save(cx, m, key, shown, models) })
                : Button(`omo-${key}-edit`, { label: "Edit models", icon: "pencil", size: "sm", variant: "outline", onClick: (_e, cx) => (this.startEdit(key, shown, models), cx.notify()) }),
              editing ? Button(`omo-${key}-cancel`, { label: "Cancel", size: "sm", variant: "ghost", onClick: (_e, cx) => (this.edit = null, cx.notify()) }) : null,
              Button(`omo-${key}-delete`, { label: asked ? "Really delete?" : "Delete", icon: "trash", size: "sm", variant: asked ? "destructive" : "ghost", disabled: inUse, tooltip: inUse ? "In use here: use another preset first" : `Delete ${shown} from ${m ?? "this computer"}'s config`, onClick: (_e, cx) => this.drop(cx, m, key, shown) }),
            ].filter(Boolean),
          ),
        h_flex().gap(8).items_center().child(TextField(this.newName)).child(Button(`omo-${key}-copy`, { label: `Copy ${shown}`, size: "sm", variant: "outline", onClick: (_e, cx) => this.copy(cx, m, shown) })),
        div().text_size(TEXT.xs).text_color(v.env ? k.warning : k.dim).child(v.env ? `OH_MY_OPENCODE_SLIM_PRESET=${v.env} is set there and wins over this.` : "Sessions already running keep the preset they started with; new ones use it."),
      ].filter(Boolean),
    });
  }

  render() {
    const k = tokens();
    const sys = [...this.perf.sys];
    const omos = [...this.omo.by];
    return v_flex()
      .id("machines")
      .size_full()
      .bg(k.background)
      .text_color(k.foreground)
      .text_size(TEXT.base)
      .child(PageHeader({ icon: "layers", title: "Machines & presets", description: "Load on each machine herdr-core is connected to, and its oh-my-opencode-slim presets: use, edit, copy, delete" }))
      .child(
        v_flex()
          .id("machines-scroll")
          .flex_1()
          .min_h(0)
          .px(24)
          .py(20)
          .gap(16)
          .overflow_y_scrollbar()
          .child(
            sys.length || omos.length
              ? v_flex()
                  .gap(16)
                  .child(div().w_full().flex().flex_row().flex_wrap().gap(16).children(sys.map(([m, s]) => div().min_w(340).flex_1().child(this.machineCard(k, m, s)))))
                  .child(div().w_full().flex().flex_row().flex_wrap().gap(16).children(omos.map(([m, v]) => div().min_w(340).flex_1().child(this.omoCard(k, m, v)))))
              : Empty({ icon: "layers", title: "Asking the machines…", description: "Each machine answers within a few seconds." }),
          ),
      );
  }
}
