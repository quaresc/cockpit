// Cockpit's settings, in a window of their own (the gear in the top bar): which tabs the sidebar shows, which of its sections
// start open, the AI tab titles, and the config file itself. The tabs are `spaces` in ~/.config/cockpit/config.json.

import { div } from "gpui-kit";
import { h_flex, v_flex } from "gpui-base";
import { notify, openOverlay, closeOverlay, props, readTextFile, writeTextFile } from "herdr";
import * as hub from "./hub.js";
import { openCode } from "./code.js";
import { tokens, TEXT, Button, Checkbox, Toggle, Card } from "./ui.js";
import { APPS, DEFAULT_SPACES } from "./sidebar.js";

/** Opens the settings window over whatever is showing. */
export function openSettings() {
  openOverlay({ mod: "cockpit", props: { app: "settings" }, kind: "dialog", width: 0.42, height: 0.78 });
}

const SECTIONS = [
  ["usage", "Usage limits"],
  ["machines", "Machines"],
  ["omo", "OMO presets"],
];

export default class Settings {
  init(_p, cx) {
    this.spaces = DEFAULT_SPACES;
    this.titles = true;
    this.header = ["usage", "machines", "omo"];
    const read = (cx) =>
      cx.spawn(async (cx) => {
        try {
          const { home } = await hub.install();
          const cfg = JSON.parse((await readTextFile(`${home}/.config/cockpit/config.json`)) ?? "{}");
          if (Array.isArray(cfg.spaces)) this.spaces = cfg.spaces.map((s) => (typeof s === "string" ? s : s.name));
          this.titles = cfg.titles?.enabled !== false;
          this.header = (cfg.sidebar?.header ?? this.header).filter((x) => SECTIONS.some(([n]) => n === x));
          cx.notify();
        } catch {}
      });
    read(cx);
  }

  command() {}

  /** Writes the cockpit config with one change (everything else in it is kept). */
  async write(change) {
    const { home } = await hub.install();
    const path = `${home}/.config/cockpit/config.json`;
    const before = await readTextFile(path);
    const cfg = before ? JSON.parse(before) : {};
    await writeTextFile(path, JSON.stringify(change(cfg), null, 2) + "\n", before);
  }

  fail(e) {
    notify({ title: "Could not save the setting", body: String(e?.message ?? e) });
  }

  setTab(cx, name, on) {
    const next = on ? [...this.spaces, name] : this.spaces.filter((s) => s !== name);
    this.spaces = next;
    cx.notify();
    cx.spawn(async () => this.write((cfg) => ({ ...cfg, spaces: next })).catch((e) => this.fail(e)));
  }

  setTitles(cx, on) {
    this.titles = on;
    cx.notify();
    cx.spawn(async () => this.write((cfg) => ({ ...cfg, titles: { ...(cfg.titles ?? {}), enabled: on } })).catch((e) => this.fail(e)));
  }

  moveHeader(cx, name, by) {
    const i = this.header.indexOf(name);
    const j = i + by;
    if (i < 0 || j < 0 || j >= this.header.length) return;
    const next = [...this.header];
    [next[i], next[j]] = [next[j], next[i]];
    this.header = next;
    this.saveHeader(cx);
  }

  saveHeader(cx) {
    const header = this.header;
    cx.notify();
    cx.spawn(async () => this.write((cfg) => ({ ...cfg, sidebar: { ...(cfg.sidebar ?? {}), header } })).catch((e) => this.fail(e)));
  }

  render() {
    const k = tokens();
    const head = (text, hint) => v_flex().gap(2).child(div().text_size(TEXT.base).font_semibold().text_color(k.foreground).child(text)).child(div().text_size(TEXT.xs).text_color(k.dim).child(hint));
    const row = (id, label, hint, control) =>
      h_flex().id(id).gap(10).items_center().child(control).child(div().text_size(TEXT.sm).text_color(k.foreground).child(label)).child(div().flex_1().text_size(TEXT.xs).text_color(k.dim).child(hint ?? ""));
    return v_flex()
      .id("settings")
      .size_full()
      .bg(k.background)
      .text_color(k.foreground)
      .text_size(TEXT.base)
      .child(
        h_flex()
          .px(20)
          .py(14)
          .items_center()
          .border_b_1()
          .border_color(k.border)
          .child(div().flex_1().text_size(TEXT.lg ?? 16).font_semibold().child("Cockpit settings"))
          .child(Button("settings-done", { label: "Done", variant: "outline", size: "sm", kbd: "esc", onClick: () => closeOverlay(props().overlay) })),
      )
      .child(
        v_flex()
          .id("settings-scroll")
          .flex_1()
          .min_h(0)
          .px(20)
          .py(16)
          .gap(18)
          .overflow_y_scrollbar()
          .child(
            Card({
              id: "settings-tabs",
              padding: 16,
              gap: 10,
              children: [
                head("Tabs", "Which apps the sidebar's tab strip shows (spaces in the cockpit config; applied within seconds)"),
                v_flex().gap(6).children(Object.entries(APPS).map(([name, a]) => row(`set-tab-${name}`, a.label, a.key, Checkbox(`set-tab-box-${name}`, { checked: this.spaces.includes(name), onClick: (_e, cx) => this.setTab(cx, name, !this.spaces.includes(name)) })))),
              ],
            }),
          )
          .child(
            Card({
              id: "settings-sections",
              padding: 16,
              gap: 10,
              children: [
                head("Sidebar sections", "Under \"Search or run\": a section that is off is gone from the sidebar, not folded; the arrows set the order"),
                v_flex()
                  .gap(6)
                  .children(
                    [...this.header, ...SECTIONS.map(([n]) => n).filter((n) => !this.header.includes(n))].map((name) => {
                      const on = this.header.includes(name);
                      const i = this.header.indexOf(name);
                      return h_flex()
                        .id(`set-sec-${name}`)
                        .gap(10)
                        .items_center()
                        .child(Toggle(`set-sec-toggle-${name}`, on, (cx) => this.setHeader(cx, name, !on)))
                        .child(div().flex_1().text_size(TEXT.sm).text_color(on ? k.foreground : k.dim).child(SECTIONS.find(([n]) => n === name)[1]))
                        .when(on, (e) =>
                          e
                            .child(Button(`set-sec-up-${name}`, { icon: "chevron-up", variant: "ghost", size: "icon-sm", disabled: i === 0, tooltip: "Move up", onClick: (_e, cx) => this.moveHeader(cx, name, -1) }))
                            .child(Button(`set-sec-down-${name}`, { icon: "chevron-down", variant: "ghost", size: "icon-sm", disabled: i === this.header.length - 1, tooltip: "Move down", onClick: (_e, cx) => this.moveHeader(cx, name, 1) })),
                        );
                    }),
                  ),
              ],
            }),
          )
          .child(
            Card({
              id: "settings-titles",
              padding: 16,
              gap: 10,
              children: [
                head("AI tab titles", "An unnamed agent tab gets a few words for its task; a name you typed is never touched"),
                row("set-titles", "Name agent tabs", this.titles ? "on" : "off", Toggle("set-titles-toggle", this.titles, (cx) => this.setTitles(cx, !this.titles))),
              ],
            }),
          )
          .child(
            Card({
              id: "settings-file",
              padding: 16,
              gap: 10,
              children: [
                head("Everything else", "The recap and title models, the gateway, context limits and machines to watch live in the config file"),
                Button("settings-open-config", { label: "Open config.json", icon: "file-text", variant: "outline", size: "sm", onClick: (_e, cx) => cx.spawn(async () => openCode(`${(await hub.install()).home}/.config/cockpit/config.json`)) }),
              ],
            }),
          ),
      );
  }
}
