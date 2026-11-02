// A pane's header in the cockpit layout (`pane.header.content` = {mod: "cockpit", props: {app: "pane"}}): the program's
// icon, its name or title, the folder and branch, the agent's status; on hover, Open in editor, Fork (a Claude
// session) and Herdr's split, zoom and close. Herdr gives each copy `props().pane` and `props().terminal`.

import { div, svg, image } from "gpui-kit";
import { h_flex } from "gpui-base";
import { panes, props, run, spaces, home, notify } from "herdr";
import { tokens, alpha, Button, Spinner, RADIUS, TEXT } from "./ui.js";
import { openInEditor, editorLabel } from "./editor.js";
import * as hub from "./hub.js";

function programIcon(p, size, color) {
  if (!p?.icon) return null;
  return p.tint ? svg(p.icon).size(size).flex_none().text_color(color) : image(p.icon).w(size).h(size).flex_none();
}

function folder(cwd) {
  if (!cwd) return "";
  const h = home();
  return cwd === h ? "~" : cwd.startsWith(h + "/") ? "~" + cwd.slice(h.length) : cwd;
}

const isClaude = (p) => p?.program?.key === "claude" || /claude/.test(p?.app ?? "");

export default class PaneHeader {
  init() {
    this.hovered = false;
    this.editor = "editor";
    editorLabel().then((l) => (this.editor = l)).catch(() => {});
  }

  me() {
    return panes().find((p) => p.pane === props()?.pane);
  }

  async openEditor(p) {
    const machine = spaces().find((s) => s.tabs?.some((t) => t.id === p.tab))?.machine ?? null;
    const failed = await openInEditor(p.cwd, machine).catch((e) => e?.message ?? String(e));
    if (failed) notify({ title: `Could not open ${this.editor}`, body: failed, level: "error" });
  }

  async fork(p) {
    const done = await hub.act("agents", "fork", [{ terminal: `app:${p.terminal}`, place: "tab" }]).catch((e) => ({ error: e?.message ?? String(e) }));
    if (done?.error) notify({ title: "Could not fork the session", body: done.error, level: "error" });
  }

  status(k, st) {
    if (st === "working") return Spinner({ size: 12, color: k.working });
    if (st === "blocked")
      return div().flex_none().px(6).h(18).flex().items_center().rounded(RADIUS.sm).bg(alpha(k.blocked, 0.15)).text_size(TEXT.xs).font_semibold().text_color(k.blocked).child("Needs you");
    if (st === "done") return div().size(7).flex_none().rounded(4).bg(k.done);
    return null;
  }

  render() {
    const k = tokens();
    const p = this.me();
    if (!p) return div();
    const pane = p.pane;
    const name = (p.program?.agent || p.program?.shell ? p.program.name : null) || p.title || p.program?.name || p.mod || "shell";
    const pic = programIcon(p.program, 14, k.dim);
    const st = this.status(k, p.status);
    const act = (id, icon, tooltip, onClick) => Button(`ph-${id}-${pane}`, { icon, size: "icon-sm", variant: "ghost", tooltip, onClick: (e, cx) => { cx.stop_propagation(); onClick(); } });
    const actions = [
      p.cwd && act("ed", "code", `Open in ${this.editor}`, () => this.openEditor(p)),
      p.terminal && isClaude(p) && act("fk", "git-fork", "Fork this session to a new tab", () => this.fork(p)),
      act("sr", "columns", "Split right", () => run("pane.split", { direction: "right", pane })),
      act("sd", "square-split", "Split down", () => run("pane.split", { direction: "down", pane })),
      act("zm", p.zoomed ? "minimize" : "maximize", p.zoomed ? "Unzoom" : "Zoom", () => run("pane.zoom", { pane })),
      act("cl", "x", "Close", () => run("pane.close", { pane })),
    ].filter(Boolean);
    return h_flex()
      .id(`pane-head-${pane}`)
      .size_full()
      .px(10)
      .gap(8)
      .items_center()
      .bg(k.background)
      .border_b(1)
      .border_color(k.border)
      .text_size(TEXT.sm)
      .on_hover((hovered, cx) => { this.hovered = hovered; cx.notify(); })
      .on_click(() => run("pane.focus", { pane }))
      .when(Boolean(pic), (el) => el.child(pic))
      .child(div().flex_none().truncate().max_w(260).font_medium().text_color(k.foreground).child(name))
      .child(div().flex_1().min_w(0).truncate().text_color(k.dim).child(folder(p.cwd)))
      .when(Boolean(p.branch) && !this.hovered, (el) => el.child(div().flex_none().truncate().max_w(180).text_size(TEXT.xs).text_color(k.mutedFg).child(p.branch)))
      .when(Boolean(st), (el) => el.child(st))
      .when(this.hovered, (el) => el.child(h_flex().flex_none().gap(2).items_center().children(actions)));
  }
}
