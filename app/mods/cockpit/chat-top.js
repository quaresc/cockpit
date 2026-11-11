// The reading layout's top bar: one capsule holding a pill per tab of the space in front, each with the mark of what
// its agent is doing, then a tab to add and the terminal of the agent being read.

import { div } from "gpui-kit";
import { h_flex } from "gpui-base";
import { run, spaces, shared, openOverlay, window as win, layout } from "herdr";
import { show, DOCKED, toggleSidebar, showSidebar } from "./screens.js";
import { openPanel, togglePanel } from "./status.js";
import { tokens, TEXT, Icon, Spinner, dur } from "./ui.js";

const BAR_H = 56;
const SPIN_MS = 900;

/** How much room the traffic lights need on the left, at this zoom; 12 when Herdr draws its own title bar. */
export function lightsRoom(settings_zoom = 1) {
  const w = win();
  const root = layout();
  const lights = root?.traffic_lights;
  return w?.platform === "macos" && w?.titlebar === "none" && !w?.fullscreen && lights ? (lights[0] + 60) / settings_zoom : 12;
}

/** What a tab's agent is doing, as a shape so it reads without colour. */
export function statusMark(status, k) {
  if (status === "working") return Spinner({ size: 13, color: k.working });
  if (status === "done") return Icon("check", { size: 13, color: k.done });
  if (status === "blocked") return div().size(9).m(2).rounded(5).bg(k.blocked);
  return div().size(12).m(0.5).rounded(6).border_1().border_color(k.dim);
}

export default class ChatTop {
  init(_props, _cx) {
    // The layout and the flag are made to agree on the way in; the sidebar is open until it is closed once.
    showSidebar(shared(DOCKED) !== false);
  }

  /** Herdr's own new menu: a terminal, an agent, a space, a worktree. */
  openNew() {
    openOverlay({ mod: "new-menu", kind: "popup", width: 250, height: 512 });
  }

  /** The shown view of this layout, so it carries what the window's keys ask for. */
  command(c, cx) {
    if (c === "new") this.openNew();
    else if (c === "sidebar") toggleSidebar();
    else if (c === "panel") togglePanel(null);
    else if (c.startsWith("panel:")) togglePanel(c.slice(6));
    else if (c.startsWith("doc:")) {
      openPanel("show");
      run(`cockpit.show:go:${c.slice(4)}`);
    }
    else if (c.startsWith("go:")) show(c.slice(3));
    cx.notify();
  }

  render(_cx) {
    const k = tokens();
    const space = spaces().find((s) => s.active) ?? spaces()[0];
    const tabs = space?.tabs ?? [];
    // A tab whose agent is waiting says so from here: its name in the blocked colour, and how long it has waited.
    const pill = (t) => {
      const waiting = t.status === "blocked";
      const age = waiting && t.changed_at ? dur(Date.now() - t.changed_at) : null;
      return h_flex()
        .id(`tab-${t.id}`)
        .h(30)
        .pl(13)
        .pr(t.active ? 6 : 14)
        .gap(8)
        .items_center()
        .flex_none()
        .rounded(15)
        .text_size(TEXT.base)
        .cursor_pointer()
        .text_color(waiting ? k.blocked : t.active ? k.foreground : k.mutedFg)
        .when(t.active, (e) => e.bg(k.selected).font_medium())
        .when(!t.active && waiting, (e) => e.font_medium())
        .when(!t.active, (e) => e.hover((s) => s.text_color(waiting ? k.blocked : k.foreground)))
        .on_click((_e, cx) => {
          run("tab.select", { id: t.id });
          cx.notify();
        })
        .tooltip(t.name)
        .child(statusMark(t.status, k))
        .child(div().max_w(170).truncate().child(t.name))
        .when(Boolean(age), (e) => e.child(div().flex_none().text_size(TEXT.xs).text_color(k.blocked).child(age)))
        .when(t.active, (e) =>
          e.child(
            div()
              .id(`tab-close-${t.id}`)
              .size(18)
              .flex()
              .items_center()
              .justify_center()
              .rounded(9)
              .text_color(k.dim)
              .hover((s) => s.bg(k.accent).text_color(k.foreground))
              .tooltip("Close the tab")
              .on_click((e, cx) => {
                cx.stop_propagation();
                run("tab.close", { id: t.id });
                cx.notify();
              })
              .child(Icon("x", { size: 12 })),
          ),
        );
    };
    const capsule = (children) => h_flex().h(38).px(4).gap(2).items_center().flex_none().rounded(19).bg(k.chrome).border_1().border_color(k.border).shadow_sm().children(children);
    const button = (id, icon, tooltip, onClick) =>
      div()
        .id(id)
        .size(30)
        .flex()
        .items_center()
        .justify_center()
        .rounded(15)
        .cursor_pointer()
        .text_color(k.mutedFg)
        .hover((s) => s.bg(k.accent).text_color(k.foreground))
        .tooltip(tooltip)
        .on_click(onClick)
        .child(Icon(icon, { size: 15 }));
    return h_flex()
      .id("chat-top")
      .w_full()
      .h(BAR_H)
      .pl(lightsRoom())
      .pr(12)
      .gap(8)
      .items_center()
      .justify_center()
      .child(div().flex_1())
      .when(tabs.length > 0, (e) => e.child(capsule(tabs.map(pill))))
      .child(
        h_flex()
          .flex_1()
          .justify_end()
          .child(
            capsule([
              // Herdr's own new menu: a terminal, an agent, a space, a worktree.
              button("chat-new-tab", "plus", "Start something new", (_e, cx) => {
                this.openNew();
                cx.notify();
              }),
              button("chat-terminal", "terminal", "Its terminal  ⌃`", (_e, cx) => {
                run("cockpit.terminal");
                cx.notify();
              }),
            ]),
          ),
      );
  }
}
