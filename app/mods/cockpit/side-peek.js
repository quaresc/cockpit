// The drawer on the window's left edge, standing in for the pinned sidebar while it is away: the palette and the apps,
// the agents by whose move it is, and the way to Needs you. It follows the agents only while that sidebar is away.

import { div } from "gpui-kit";
import { h_flex, v_flex } from "gpui-base";
import { shared, focused, run, spaces } from "herdr";
import * as hub from "./hub.js";
import { keyed } from "./person.js";
import { sharing } from "./feed.js";
import { showSidebar, sidebarShown, show, screens, CHAT_SUBJECT } from "./screens.js";
import { rowsOf, goToAgent, needing } from "./fleet.js";
import { herdrRows, stageColor, STAGES as AGENT_STAGES } from "./agentsui.js";
import { statusMark } from "./chat-top.js";
import { hint, lend } from "./commands.js";
import { tokens, TEXT, RADIUS, Icon, CountPill, dur, AgentLogo } from "./ui.js";
import { SettingsLink } from "./doctor.js";
import { QuotaGroups } from "./usage.js";
import { UpdateTag, pending as pendingUpdate } from "./update.js";

// The agents, Home and queue views, followed once per window whichever copies ask (feed.js).
const following = sharing(hub.subscribe);

const WIDTH = 222;
/** The places the drawer lists before it leaves the rest to the palette. */
const PLACES = 6;
/** A drawer sizes to what it holds; past this it scrolls instead of running off the window. */
export const MAX_H = 620;
/** What peeks at the edge: a bar of colour, no content. */
export const BAR = 10;
/** The hub's stages in Cockpit's order and words (agentsui.js), each dot in its stage's colour. */
const STAGES = Object.entries(AGENT_STAGES).map(([id, s]) => ({ id, label: s.label }));

/** The bar on the edge side of a drawer. */
export function peekBar(k) {
  return div().w(BAR).h_full().flex_none().bg(k.selected);
}

/** The head both drawers wear: what it is on the left, a count, what else it leads to, then the button that pins it open. */
export function peekHead(k, label, count, { tooltip, onPin, extra = null }) {
  const pin = div()
    .id(`peek-pin-${label}`)
    .size(24)
    .flex()
    .items_center()
    .justify_center()
    .rounded(RADIUS.md)
    .cursor_pointer()
    .text_color(k.dim)
    .hover((s) => s.bg(k.accent).text_color(k.foreground))
    .tooltip(tooltip)
    .on_click((_e, cx) => {
      onPin();
      cx.notify();
    })
    .child(Icon("panel-right", { size: 13 }));
  return h_flex()
    .h(38)
    .px(10)
    .gap(6)
    .flex_none()
    .items_center()
    .child(div().flex_1().min_w(0).truncate().text_size(11.5).font_semibold().text_color(k.mutedFg).child(label))
    .when(count !== null && count !== undefined, (e) => e.child(CountPill(count)))
    .when(Boolean(extra), (e) => e.child(extra))
    .child(pin);
}

export default class SidePeek {
  init(_props, cx) {
    this.rows = null;
    this.update = null;
    this.unlend = lend(() => (this.sub ? this.rows : null));
    cx.spawn(async (cx) => {
      for (;;) {
        this.listen(cx);
        await hub.sleep(cx, 500);
      }
    });
    // A newer cockpit shows here as it does in the sidebar this drawer stands in for (update.json, every five minutes).
    cx.spawn(async (cx) => {
      for (;;) {
        try {
          this.update = await pendingUpdate();
        } catch {
          this.update = null;
        }
        cx.notify();
        await hub.sleep(cx, 5 * 60 * 1000);
      }
    });
  }

  close() {
    this.unlend?.();
    this.sub?.stop();
  }

  /** Follows the agents only while the pinned sidebar is away, so the two never subscribe at once. */
  listen(cx) {
    const wanted = !sidebarShown();
    if (wanted === Boolean(this.sub)) return;
    if (!wanted) {
      this.sub?.stop();
      this.sub = null;
      this.rows = null;
      return;
    }
    this.sub = following(
      cx,
      { views: [{ name: "agents", args: { frontend: "app" } }] },
      (m, cx) => {
        if (m.view === "agents") this.rows = m.value;
        cx.notify();
      },
      (state, cx) => {
        // Down from the first failure until the hub answers again: the drawer shows Herdr's own agents meanwhile.
        if (state === "live") this.down = false;
        else if (state.startsWith("error")) this.down = true;
        cx.notify();
      },
    );
  }

  /** The way to Needs you: every waiting agent's question on one screen, with how many wait. */
  needsButton(k, waiting) {
    return h_flex()
      .id("peek-needs")
      .h(24)
      .px(6)
      .gap(4)
      .flex_none()
      .items_center()
      .rounded(RADIUS.md)
      .cursor_pointer()
      .text_size(TEXT.xs)
      .font_medium()
      .text_color(waiting ? k.blocked : k.dim)
      .hover((s) => s.bg(k.accent).text_color(waiting ? k.blocked : k.foreground))
      .tooltip(hint("Needs you", "cockpit.go:needs"))
      .on_click((_e, cx) => {
        show("needs");
        cx.notify();
      })
      .child(Icon("hand", { size: 13 }))
      .when(waiting > 0, (e) => e.child(String(waiting)));
  }

  /** What the sidebar's search box and app strip lead to, as one row of icons: the palette, then each app's screen. */
  apps(k) {
    const here = shared("cockpit.app");
    const items = [
      { child: null, label: "Search or run a command", icon: "search" },
      ...screens().list.filter((s) => !["chat", "needs", "tab"].includes(s.child)),
    ];
    return h_flex()
      .id("peek-apps")
      .px(3)
      .pb(4)
      .gap(1)
      .items_center()
      .children(
        items.map((s) =>
          div()
            .id(`peek-app-${s.child ?? "search"}`)
            .size(22)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded(RADIUS.md)
            .cursor_pointer()
            .text_color(s.child && s.child === here ? k.foreground : k.mutedFg)
            .when(Boolean(s.child) && s.child === here, (e) => e.bg(k.selected))
            .hover((st) => st.bg(k.accent).text_color(k.foreground))
            .tooltip(hint(s.label, s.command ?? "cockpit.palette"))
            .on_click((_e, cx) => {
              if (s.child) show(s.child);
              else run("cockpit.palette");
              cx.notify();
            })
            .child(Icon(s.icon, { size: 13 })),
        ),
      );
  }

  row(a, here, k) {
    const meta = [a.ticket?.key, a.branch].filter(Boolean)[0] ?? null;
    return h_flex()
      .id(`peek-${a.terminal}`)
      .min_h(meta ? 34 : 28)
      .px(8)
      .gap(8)
      .items_center()
      .rounded(RADIUS.md)
      .cursor_pointer()
      .text_size(TEXT.sm)
      .when(a.address === here, (e) => e.bg(k.selected))
      .when(a.address !== here, (e) => e.hover((s) => s.bg(k.accent)))
      .on_click((_e, cx) => {
        // A session cockpit drives has no terminal to focus: the chat opens on it.
        if (a.driven) run(keyed(`cockpit.via:chat|chat:open:${a.terminal}`));
        else {
          goToAgent({ address: a.address });
          show("chat");
        }
        cx.notify();
      })
      .child(div().w(14).flex_none().flex().justify_center().child(statusMark(a.status, k)))
      .child(AgentLogo(a, { size: 13, tooltip: a.agent_name, id: `peek-${a.terminal}-logo` }))
      .child(
        v_flex()
          .flex_1()
          .min_w(0)
          .gap(1)
          .child(div().w_full().truncate().text_color(k.foreground).child(a.title ?? a.terminal))
          .when(Boolean(meta), (e) => e.child(div().w_full().truncate().text_size(TEXT.xs).text_color(k.dim).child(meta))),
      )
      .when(Boolean(a.since) && a.stage === "needs_you", (e) =>
        e.child(div().flex_none().text_size(TEXT.xs).text_color(k.blocked).child(dur(Date.now() - a.since))),
      );
  }

  /**
   * Places: the window's spaces (a repo's checkout or one of its worktrees), the one in front marked; a click brings it
   * in front and the chat follows its agent. The palette lists the same places for the keyboard.
   */
  places(k) {
    const all = spaces();
    if (!all.length) return [];
    const shown = all.slice(0, PLACES);
    return [
      h_flex()
        .id("peek-places-head")
        .px(7)
        .pt(9)
        .pb(2)
        .gap(6)
        .items_center()
        .tooltip(hint("Every place, by name: the palette", "cockpit.palette"))
        .child(div().flex_1().text_size(11.5).font_medium().text_color(k.mutedFg).child("Places"))
        .child(div().text_size(TEXT.xs).text_color(k.dim).child(String(all.length))),
      ...shown.map((sp) =>
        h_flex()
          .id(`peek-place-${sp.id}`)
          .min_h(28)
          .px(8)
          .gap(8)
          .items_center()
          .rounded(RADIUS.md)
          .cursor_pointer()
          .text_size(TEXT.sm)
          .when(Boolean(sp.active), (e) => e.bg(k.selected))
          .when(!sp.active, (e) => e.hover((s) => s.bg(k.accent)))
          .tooltip(sp.folder || sp.name)
          .on_click((_e, cx) => {
            run("space.select", { id: sp.id });
            show("chat");
            cx.notify();
          })
          .child(div().w(14).flex_none().flex().justify_center().child(Icon(sp.machine ? "server" : "folder", { size: 12, color: k.mutedFg })))
          .child(div().flex_1().min_w(0).truncate().text_color(k.foreground).child(sp.name))
          .when(Boolean(sp.branch), (e) => e.child(div().flex_none().max_w(90).truncate().text_size(TEXT.xs).text_color(k.dim).child(sp.branch))),
      ),
      all.length > shown.length
        ? div()
            .id("peek-places-more")
            .px(8)
            .py(4)
            .cursor_pointer()
            .text_size(TEXT.xs)
            .text_color(k.dim)
            .hover((s) => s.text_color(k.foreground))
            .tooltip(hint("Search them in the palette", "cockpit.palette"))
            .on_click((_e, cx) => (run("cockpit.palette"), cx.notify()))
            .child(`${all.length - shown.length} more in the palette`)
        : null,
    ].filter(Boolean);
  }

  render(_cx) {
    const k = tokens();
    // Herdr hides a floating block that draws nothing: the drawer stands down while the sidebar is in the window.
    if (sidebarShown()) return div();
    const rows = this.down && !this.rows?.remote ? herdrRows() : rowsOf(this.rows);
    // The row the chat reads is the one marked: its subject, else the agent in front.
    const f = focused();
    const subject = shared(CHAT_SUBJECT);
    const terminal = f?.shown ?? f?.terminal;
    const inView = rows.find((r) => r.address === terminal || r.terminal === terminal || r.terminal === `app:${terminal}`)
      ?? rows.find((r) => f?.tab && (r.tab?.id ?? r.tab) === f.tab);
    const here = String(subject ?? inView?.address ?? inView?.terminal ?? "").replace(/^app:/, "") || null;
    const groups = STAGES.map((s) => [s, rows.filter((r) => r.stage === s.id)]).filter(([, list]) => list.length > 0);
    // Places stay under the list, outside its scroll, which gives up their height so the drawer keeps its own.
    const places = this.places(k);
    const placesH = places.length ? 30 + 28 * Math.min(spaces().length, PLACES) + (spaces().length > PLACES ? 24 : 0) : 0;
    const body = v_flex()
      .id("side-peek-body")
      .w(WIDTH)
      .max_h(MAX_H - placesH)
      .overflow_y_scrollbar()
      .px(5)
      .pb(6)
      .gap(1)
      .text_color(k.foreground)
      .child(peekHead(k, "Agents", rows.length, { tooltip: hint("Keep the sidebar open", "cockpit.sidebar"), onPin: () => showSidebar(true), extra: this.needsButton(k, needing(rows).length) }))
      .child(this.apps(k))
      .when(rows.length === 0, (e) => e.child(div().px(8).py(6).text_size(TEXT.sm).text_color(k.dim).child("No agent yet.")))
      // Each provider's tightest limit beside its logo, in the drawer's text tint, its windows in the tooltip; a click
      // opens the Usage sheet.
      .when(Boolean(this.rows?.quota?.length || rows.length), (e) =>
        e.child(div().px(8).pt(2).pb(6).child(QuotaGroups(this.rows?.quota, { id: "peek-quota", rows, compact: true, mono: true, tightest: true }))),
      )
      .child(
        v_flex()
          .w_full()
          .gap(1)
          .children(
        groups.flatMap(([s, list]) => [
          h_flex()
            .id(`peek-stage-${s.id}`)
            .px(7)
            .pt(7)
            .pb(2)
            .gap(6)
            .items_center()
            // Needs you's heading opens the screen that answers them all.
            .when(s.id === "needs_you", (e) =>
              e
                .cursor_pointer()
                .tooltip(hint("Answer them on Needs you", "cockpit.go:needs"))
                .on_click((_e, cx) => {
                  show("needs");
                  cx.notify();
                }),
            )
            .child(div().size(6).flex_none().rounded(3).bg(stageColor(s.id)))
            .child(div().flex_1().text_size(11.5).font_medium().text_color(k.mutedFg).child(s.label))
            .child(div().text_size(TEXT.xs).text_color(k.dim).child(String(list.length))),
          ...list.map((a) => this.row(a, here, k)),
        ]),
          ),
      );
    // Places, the update tag and the link stay under the list, outside its scroll.
    const foot = v_flex()
      .px(5)
      .pb(6)
      .children(places)
      .when(Boolean(this.update), (e) => e.child(UpdateTag("peek-update", this.update, { mono: true })))
      .child(SettingsLink("peek-settings"));
    return h_flex().id("side-peek").child(peekBar(k)).child(v_flex().w(WIDTH).child(body).child(foot));
  }
}
