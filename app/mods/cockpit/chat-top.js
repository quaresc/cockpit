// The reading layout's top bar: a capsule with a pill per tab of the space in front and per session cockpit drives, each
// with the mark of what its agent is doing, and a "+N" pill for those that do not fit. Always shown,
// it runs the window's commands in this layout (commands.js).

import { div } from "gpui-kit";
import { h_flex } from "gpui-base";
import { run, spaces, shared, notify, agents, focused, window as win, layout } from "herdr";
import { DOCKED, showSidebar, settleSidebar, sidebarShown, showTab, frontAgent, CHAT_SUBJECT, CHAT_PIN, pinChat } from "./screens.js";
import * as hub from "./hub.js";
import { keyed, take } from "./person.js";
import { sharing } from "./feed.js";
import { runCommand, openNew, hint, settle, settlePanel } from "./commands.js";
import { syncBrowser, releaseBrowser } from "./browser.js";
import { tokens, TEXT, Icon, Spinner, dur, openMenu, viewport, AgentLogo } from "./ui.js";
import { agentStatus } from "./fleet.js";

const BAR_H = 56;
const CAPSULE_H = 38;
/** The layout's padding and gap: the bar sits at the window's top right, that far from its edges. */
const EDGE = 8;
const GAP = 8;
/** Between two pills, and inside the capsule (padding and border) on each side. */
const PILL_GAP = 2;
const CAPSULE_PAD = 5;
/** The room the "+N" pill takes once anything overflows. */
const MORE_W = 52;

/**
 * How much room the traffic lights need on the bar's left, at this zoom: 12 when Herdr draws its own title bar, or when
 * the sidebar is open and they sit over it.
 */
export function lightsRoom(settings_zoom = 1) {
  const root = layout();
  if (sidebarShown(root)) return 12;
  const w = win();
  const lights = root?.traffic_lights;
  return w?.platform === "macos" && w?.titlebar === "none" && !w?.fullscreen && lights ? (lights[0] + 60) / settings_zoom : 12;
}

/** What a tab's agent is doing, as a shape so it reads without colour. */
export function statusMark(raw, k) {
  const status = agentStatus(raw);
  if (status === "failed") return Icon("circle-x", { size: 13, color: k.destructive });
  if (status === "working") return Spinner({ size: 13, color: k.working });
  if (status === "done") return Icon("check", { size: 13, color: k.done });
  if (status === "blocked") return div().size(9).m(2).rounded(5).bg(k.blocked);
  return div().size(12).m(0.5).rounded(6).border_1().border_color(k.dim);
}

/** The same, as a menu's icon name. */
function statusIcon(raw) {
  return { failed: "circle-x", working: "loader", done: "check", blocked: "hand" }[agentStatus(raw)] ?? "app-window";
}

/** How long a close asked of a tab whose agent is at work waits for the second click that confirms it. */
const CONFIRM_MS = 4000;

/** A pill's width before it was ever drawn: its mark, its agent's logo, its name (cut at 170 px), and the close button
 *  of the one on. */
export function guessWidth(name, on = false, logo = false) {
  return 13 + 13 + 8 + (logo ? 13 + 8 : 0) + Math.min(170, Math.ceil(String(name ?? "").length * 7.2)) + (on ? 6 + 8 + 18 : 14);
}

/**
 * Which pills fit in `room` px, in their order, the one on always among them: {shown, hidden} as lists of keys. Pills
 * are {key, width, on}; once one overflows, room is kept for the "+N" pill.
 */
export function fitPills(pills, room, { gap = PILL_GAP, more = MORE_W } = {}) {
  const total = pills.reduce((w, p, i) => w + p.width + (i ? gap : 0), 0);
  if (room == null || total <= room) return { shown: pills.map((p) => p.key), hidden: [] };
  const on = pills.find((p) => p.on);
  const kept = new Set(on ? [on.key] : []);
  // Each pill kept is followed by a gap: the one on and its gap, then "+N".
  let used = on ? on.width + gap + more : more;
  for (const p of pills) {
    if (p === on) continue;
    if (used + p.width + gap > room) break;
    used += p.width + gap;
    kept.add(p.key);
  }
  return { shown: pills.filter((p) => kept.has(p.key)).map((p) => p.key), hidden: pills.filter((p) => !kept.has(p.key)).map((p) => p.key) };
}

/**
 * Where the "+N" menu opens: under its pill, else under the capsule's start. The bar sits at the window's top right,
 * so its place follows from the window's width and the widths measured in it; null before they are known.
 */
function menuAnchor({ window, bar, lights, left, tabs, more }) {
  if (!window?.width || !bar) return null;
  const capsule = window.width - EDGE - bar + lights + (left ?? 0) + GAP;
  const x = more && tabs ? capsule + tabs - CAPSULE_PAD - more : capsule;
  return { x: Math.round(x), y: EDGE + (BAR_H + CAPSULE_H) / 2 + 4 };
}

/**
 * The hub's state as the bar's dot draws it, in monochrome: `live`, `connecting` or `down`; down on this machine it
 * says so and a click starts it.
 */
export function hubDot(state, { remote = false } = {}) {
  const s = String(state ?? "connecting");
  if (s === "live") return { kind: "live", label: null, tooltip: "Cockpit's hub is running", startable: false };
  if (s === "connecting") return { kind: "connecting", label: null, tooltip: "Reaching cockpit's hub…", startable: false };
  const startable = s.startsWith("error") && !hub.isNotInstalled(s) && !remote;
  const label = hub.downWords(s, true);
  const tooltip = startable ? `${hub.downTip(s)} · click to start it` : hub.downTip(s);
  return { kind: "down", label, tooltip, startable };
}

export default class ChatTop {
  init(_props, cx) {
    // The layout and the flag are made to agree on the way in; the sidebar is open until it is closed once.
    showSidebar(shared(DOCKED) !== false);
    settle();
    // Then the layout leads: a fold to a rail, or a hide from elsewhere, leaves the sidebar put away behind its drawer,
    // and the panel's flag says what this layout shows, whatever the other one left it at.
    settlePanel();
    cx.spawn(async (cx) => {
      for (;;) {
        await hub.sleep(cx, 400);
        if (settlePanel() | Boolean(settleSidebar()) | syncBrowser()) cx.notify();
      }
    });
    this.sizes = new Map();
    this.hub = "connecting";
    // The sessions cockpit drives have no tab: they get pills of their own, from the window's one agents feed.
    this.sessions = [];
    // Every agents row, for the logo of the agent in each tab and the name a tab still at Herdr's number shows.
    this.rows = [];
    this.sub = sharing(hub.subscribe)(
      cx,
      { views: ["agents"] },
      (m, cx) => {
        if (m.view === "agents") {
          this.rows = m.value?.agents ?? [];
          this.sessions = this.rows.filter((r) => r.driven === true);
        }
        cx.notify();
      },
      (state, cx) => {
        this.hub = state;
        cx.notify();
      },
    );
  }

  close() {
    this.sub?.stop();
    // The cockpit layout shows the tab's panes as they are, not zoomed on the browser.
    releaseBrowser();
  }

  /** Keeps a measured width; only a pill's, the bar's or its right capsule's changes what fits and redraws. */
  sized(key, width, cx) {
    if (Math.abs((this.sizes.get(key) ?? -1) - width) < 0.5) return;
    this.sizes.set(key, width);
    if (key === "bar" || key === "right" || key.startsWith("pill:")) cx.notify();
  }

  /** The agent a tab runs, as the hub's row (its `icon` and name) or else Herdr's own agent; null for a tab of shells. */
  tabAgent(t) {
    const live = this.nativeAgents ?? agents();
    return (this.rows ?? []).find((r) => !r.driven && r.tab?.id === t.id) ?? live.find((a) => a.tab === t.id) ?? null;
  }

  /**
   * The agent the chat reads (its subject, else the agent in front), as the hub's row or else Herdr's own agent; null
   * when there is none.
   */
  subjectAgent() {
    const list = [...(this.rows ?? []), ...(this.nativeAgents ?? agents())];
    const subject = shared(CHAT_SUBJECT);
    if (subject) {
      const address = String(subject).replace(/^app:/, "");
      return list.find((r) => r.terminal === subject || r.address === address) ?? null;
    }
    const front = frontAgent(focused(), (this.rows ?? []).map((r) => ({ ...r, tab: r.tab?.id ?? r.tab })));
    return front ? (list.find((r) => r.address === front) ?? null) : null;
  }

  /** The toggle to the agent's own terminal: its provider's logo, its tooltip saying whose terminal it shows. */
  terminalButton(k) {
    const a = this.subjectAgent();
    return div()
      .id("chat-terminal")
      .size(30)
      .flex()
      .items_center()
      .justify_center()
      .rounded(15)
      .cursor_pointer()
      .text_color(k.mutedFg)
      .hover((s) => s.bg(k.accent).text_color(k.foreground))
      .tooltip(hint(terminalWords(a), "cockpit.terminal"))
      .on_click((_e, cx) => {
        run("cockpit.terminal");
        cx.notify();
      })
      .child(a ? AgentLogo(a, { size: 16 }) : Icon("terminal", { size: 15 }));
  }

  /** A driven session's pill, from its agents row: its title, what it is doing (or that it is paused), and its chat. */
  driven(s, k) {
    const name = drivenName(s);
    const status = s.status;
    const on = shared(CHAT_PIN)?.key === s.terminal && shared(CHAT_SUBJECT) === s.terminal;
    const waiting = agentStatus(status) === "blocked";
    return h_flex()
      .id(`driven-${s.terminal}`)
      .h(30)
      .px(13)
      .gap(8)
      .items_center()
      .flex_none()
      .rounded(15)
      .text_size(TEXT.base)
      .cursor_pointer()
      .text_color(waiting ? k.blocked : on ? k.foreground : k.mutedFg)
      .when(on, (e) => e.bg(k.selected).font_medium())
      .when(!on, (e) => e.hover((st) => st.text_color(waiting ? k.blocked : k.foreground)))
      .tooltip(s.paused ? `${name} · paused, picks up when you write` : `${name} · a session cockpit drives`)
      .on_size_change((size, cx) => this.sized(`pill:d:${s.terminal}`, size.width, cx))
      .on_click((_e, cx) => {
        run(keyed(`cockpit.via:chat|chat:open:${s.terminal}`));
        cx.notify();
      })
      .child(s.paused ? Icon("circle-pause", { size: 13, color: k.dim }) : statusMark(status, k))
      .child(AgentLogo(s, { size: 13 }))
      .child(div().max_w(170).truncate().child(name));
  }

  /** A tab's pill; the one the chat reads has a close button, which closes the tab and what runs in it. */
  pill(t, on, k, open) {
    const waiting = agentStatus(t.status) === "blocked";
    // A close asked while its agent works or waits takes a second click: the first only asks.
    const asking = this.closing?.id === t.id && Date.now() - this.closing.at < CONFIRM_MS;
    // A tab whose agent is waiting says so from here: its name in the blocked colour, and how long it has waited.
    const age = waiting && t.changed_at ? dur(Date.now() - t.changed_at) : null;
    const agent = this.tabAgent(t);
    return h_flex()
      .id(`tab-${t.id}`)
      .h(30)
      .pl(13)
      .pr(on ? 6 : 14)
      .gap(8)
      .items_center()
      .flex_none()
      .rounded(15)
      .text_size(TEXT.base)
      .cursor_pointer()
      .text_color(waiting ? k.blocked : on ? k.foreground : k.mutedFg)
      .when(on, (e) => e.bg(k.selected).font_medium())
      .when(!on && waiting, (e) => e.font_medium())
      .when(!on, (e) => e.hover((s) => s.text_color(waiting ? k.blocked : k.foreground)))
      .on_size_change((size, cx) => this.sized(`pill:t:${t.id}`, size.width, cx))
      .on_click((_e, cx) => {
        // A tab picked on purpose, the active one too: what it holds comes in front, its agent's chat or its panes.
        showTab(t.id);
        cx.notify();
      })
      .tooltip(agent?.agent_name || agent?.program?.name ? `${tabName(t, this.rows)} · ${agent.agent_name || agent.program.name}` : tabName(t, this.rows))
      .child(statusMark(t.status, k))
      .when(Boolean(agent), (e) => e.child(AgentLogo(agent, { size: 13 })))
      .child(div().max_w(170).truncate().child(tabName(t, this.rows)))
      .when(Boolean(age), (e) => e.child(div().flex_none().text_size(TEXT.xs).text_color(k.blocked).child(age)))
      .when(on, (e) =>
        e.child(
          div()
            .id(`tab-close-${t.id}`)
            .h(18)
            .min_w(18)
            .px(asking ? 6 : 0)
            .flex()
            .items_center()
            .justify_center()
            .rounded(9)
            .text_size(TEXT.xs)
            .text_color(asking ? k.destructive : k.dim)
            .hover((s) => s.bg(k.accent).text_color(asking ? k.destructive : k.foreground))
            .tooltip(asking ? "Click again to close it, its agent with it" : "Close the tab and what runs in it")
            .on_click((_e, cx) => {
              cx.stop_propagation();
              this.closeTab(t, open, cx);
              cx.notify();
            })
            .child(asking ? div().child("Close?") : Icon("x", { size: 12 })),
        ),
      );
  }

  /**
   * Closes a tab, and what runs in it, for good. One whose agent works or waits asks for a second click first; the next
   * tab comes in front before, so the chat moves on with it.
   */
  closeTab(t, open, cx = null) {
    const busy = ["working", "blocked"].includes(agentStatus(t.status));
    const confirmed = this.closing?.id === t.id && Date.now() - this.closing.at < CONFIRM_MS;
    if (busy && !confirmed) {
      this.closing = { id: t.id, at: Date.now() };
      // The question goes once its time is up.
      cx?.timer?.after(CONFIRM_MS, (cx) => cx.notify());
      return;
    }
    this.closing = null;
    const i = open.findIndex((x) => x.id === t.id);
    const next = open[i + 1] ?? open[i - 1];
    if (t.active && next) {
      pinChat(null);
      run("tab.select", { id: next.id });
    }
    run("tab.close", { id: t.id });
  }

  /** The bar's tabs and sessions as they stand: what shows, and what the "+N" menu holds. */
  layoutNow() {
    const space = spaces().find((s) => s.active) ?? spaces()[0];
    const open = space?.tabs ?? [];
    // The chat on a session it was opened on: no tab is the one it reads, even the active one.
    const pinned = Boolean(shared(CHAT_PIN)?.key);
    const pills = [
      ...open.map((t) => {
        const on = t.active && !pinned;
        return { key: `t:${t.id}`, tab: t, on, width: this.sizes.get(`pill:t:${t.id}`) ?? guessWidth(tabName(t, this.rows), on, Boolean(this.tabAgent(t))) };
      }),
      ...this.sessions.map((s) => {
        const on = shared(CHAT_PIN)?.key === s.terminal && shared(CHAT_SUBJECT) === s.terminal;
        return { key: `d:${s.terminal}`, session: s, on, width: this.sizes.get(`pill:d:${s.terminal}`) ?? guessWidth(drivenName(s), false, true) };
      }),
    ];
    const bar = this.sizes.get("bar");
    const room = bar ? bar - lightsRoom() - 12 - 2 * GAP - (this.sizes.get("right") ?? 110) - 2 * CAPSULE_PAD : null;
    const fit = fitPills(pills, room);
    const shown = pills.filter((p) => fit.shown.includes(p.key));
    const hidden = pills.filter((p) => fit.hidden.includes(p.key));
    return { open, shown, hidden };
  }

  /**
   * The "+N" menu, under its pill: what does not fit; with nothing hidden, every tab. Opened by its key, its first item
   * is picked.
   */
  openTabs(byKey = false) {
    const { shown, hidden } = this.layoutNow();
    const list = hidden.length ? hidden : shown;
    const items = list.map((p) =>
      p.tab
        ? { label: tabName(p.tab, this.rows), icon: p.on ? "check" : statusIcon(p.tab.status), command: `tabs:select:${p.tab.id}` }
        : { label: drivenName(p.session), icon: "message-square", command: `tabs:open:${p.session.terminal}` },
    );
    if (!items.length) return;
    const at = menuAnchor({ window: viewport(), bar: this.sizes.get("bar"), lights: lightsRoom(), left: this.sizes.get("left"), tabs: this.sizes.get("tabs"), more: hidden.length ? this.sizes.get("more") : null });
    openMenu(items, { width: 260, at, byKey });
  }

  /**
   * The window's commands, as the copy of this layout that is always shown, and the bar's own: its tabs menu only
   * selects and opens tabs. `key` is the one a menu, the palette or a click sent the command with.
   */
  command(c, cx, key = null) {
    if (c === "tabs") this.openTabs(true);
    else if (c.startsWith("tabs:select:")) showTab(c.slice(12));
    else if (c.startsWith("tabs:open:")) {
      const open = `cockpit.via:chat|chat:open:${c.slice(10)}`;
      run(take(c, key) ? keyed(open) : open);
    }
    else runCommand(c, cx, "cockpit-chat", { key });
    cx.notify();
  }

  /** The hub's dot: its state without colour, and on this machine a way to start it when it is down. */
  hubButton(k) {
    const dot = hubDot(this.hub, { remote: Boolean(hub.remoteMachine()) });
    const mark =
      dot.kind === "live"
        ? div().size(7).rounded(4).bg(k.mutedFg)
        : dot.kind === "connecting"
          ? div().size(8).rounded(4).border_1().border_color(k.dim)
          : div().size(8).rounded(4).border_1().border_color(k.foreground);
    return h_flex()
      .id("chat-hub")
      .h(30)
      .min_w(30)
      .px(dot.label ? 11 : 0)
      .gap(7)
      .items_center()
      .justify_center()
      .rounded(15)
      .text_size(TEXT.sm)
      .text_color(dot.kind === "down" ? k.foreground : k.mutedFg)
      .when(dot.startable, (e) => e.cursor_pointer().hover((s) => s.bg(k.accent)))
      .tooltip(dot.tooltip)
      .when(dot.startable, (e) =>
        e.on_click((_e, cx) => {
          cx.spawn(async (cx) => hub.startHub({ explicit: true, cx }).catch((err) => notify({ title: "The hub did not start", body: String(err?.message ?? err), level: "warning" })));
        }),
      )
      .child(mark)
      .when(Boolean(dot.label), (e) => e.child(div().child(dot.label)));
  }

  render(_cx) {
    this.nativeAgents = agents();
    const k = tokens();
    const { open, shown, hidden } = this.layoutNow();
    const more = hidden.length;
    const pills = shown.map((p) => (p.tab ? this.pill(p.tab, p.on, k, open) : this.driven(p.session, k)));
    if (more > 0) {
      const parts = `${hidden.length} more`;
      pills.push(
        h_flex()
          .id("chat-tabs-more")
          .h(30)
          .px(12)
          .gap(4)
          .items_center()
          .flex_none()
          .rounded(15)
          .text_size(TEXT.base)
          .text_color(k.mutedFg)
          .cursor_pointer()
          .hover((s) => s.bg(k.accent).text_color(k.foreground))
          .tooltip(hint(parts, "cockpit.tabs"))
          .on_size_change((size, cx) => this.sized("more", size.width, cx))
          .on_click((_e, cx) => {
            this.openTabs();
            cx.notify();
          })
          .child(`+${more}`)
          .child(Icon("chevron-down", { size: 12 })),
      );
    }
    const capsule = (id, children) =>
      h_flex().id(id).h(CAPSULE_H).px(4).gap(PILL_GAP).items_center().flex_none().rounded(19).bg(k.chrome).border_1().border_color(k.border).shadow_sm().children(children);
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
      .gap(GAP)
      .items_center()
      .justify_center()
      .on_size_change((size, cx) => this.sized("bar", size.width, cx))
      .child(div().id("chat-top-left").flex_1().on_size_change((size, cx) => this.sized("left", size.width, cx)))
      .when(pills.length > 0, (e) => e.child(capsule("chat-tabs", pills).on_size_change((size, cx) => this.sized("tabs", size.width, cx))))
      .child(
        h_flex()
          .flex_1()
          .justify_end()
          .child(
            capsule("chat-actions", [
              this.hubButton(k),
              // Herdr's own new menu: a terminal, an agent, a space, a worktree.
              button("chat-new-tab", "plus", hint("Start something new", "cockpit.new"), (_e, cx) => {
                openNew();
                cx.notify();
              }),
              // The agent's own terminal, shown by its provider's logo rather than a generic terminal glyph.
              this.terminalButton(k),
            ]).on_size_change((size, cx) => this.sized("right", size.width, cx)),
          ),
      );
  }
}

/** What the toggle to the chat agent's terminal does, naming its agent: "Show Claude Code's terminal". */
export function terminalWords(a) {
  const name = a?.agent_name || a?.program?.name || null;
  if (!a) return "Its terminal";
  if (a.driven) return name ? `Open ${name}'s session in a terminal of its own` : "Open this session in a terminal of its own";
  return name ? `Show ${name}'s terminal` : "Show its terminal";
}

/**
 * What a tab's pill says: the title of the work its agent does, while the tab's label is still Herdr's number or its
 * agent's own name ("Claude Code"); a label the person gave stays.
 */
export function tabName(t, rows = []) {
  const label = String(t?.name ?? "").trim();
  const row = rows.find((r) => r.tab?.id && r.tab.id === t?.id && r.title);
  const generic = !label || /^\d+$/.test(label) || [row?.agent_name, row?.program?.name, row?.program].some((n) => typeof n === "string" && n.toLowerCase() === label.toLowerCase());
  return generic ? row?.title || label : label;
}

/** What a driven session's pill and menu item say: its title, else its folder. */
function drivenName(s) {
  return s.title || String(s.cwd ?? "").split("/").pop() || "A session";
}
