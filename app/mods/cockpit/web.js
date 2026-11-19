// The Chromium side browser's slot in the side panel. cockpit's own browser is a separate window: this says where it
// goes (its rectangle inside the Herdr window, whether it shows, whether something of cockpit's covers it, and which
// session is in front) and the browser lays itself exactly over it. A browser belongs to the session it was opened
// for: it shows with that session only, and the panel follows. While the browser is hidden the slot draws its last
// frame, so nothing blinks.

import { div, image } from "gpui-kit";
import { v_flex } from "gpui-base";
import { run, layout, focused, shared, setShared, appHome } from "herdr";
import * as hub from "./hub.js";
import { tokens, TEXT, RADIUS, Icon, Button } from "./ui.js";
import { openPanel, closePanel, panelOpen } from "./commands.js";
import { layoutName, nodeOf } from "./screens.js";

/** The side panel's child that is the browser's slot, in both layouts. */
export const WEB = "web";

/** How wide the panel opens for a page, when it is narrower. */
const WIDTH = 640;

/** What lies between the slot and the window's right and bottom edges in each layout: padding, the panel's border, the activity bar, the status bar. */
export const INSETS = {
  cockpit: { right: 8 + 1 + 44, bottom: 34 + 8 + 1 },
  "cockpit-chat": { right: 8 + 1 + 44, bottom: 8 + 1 },
};

/** The insets a pointer over the slot measured, by layout: they correct the table above to the pixel. */
const MEASURED = "cockpit.web.insets";

/** Set while cockpit closed the panel because the session in front has no browser: it opens again with one that has. */
export const AWAY = "cockpit.web.away";

/** The slot's size as its view last drew it. The view and the follower are separate copies of the mod: this is how one tells the other. */
const SIZE = "cockpit.web.size";

/** What the follower keeps about the slot: what was last told, the hub's view, and who is in front. It starts afresh with each load of the mod. */
const state = { viewport: null, sent: "", next: null, sending: 0, seq: 0, since: Date.now(), model: null, terminal: undefined, unseen: new Set() };
const slot = () => state;

/** The follower's state, for the tests that walk it. */
export const followState = () => state;

/** How long a report may stay unanswered before the next one goes anyway: a request made as the window reloads the mod never comes back. */
const STUCK_MS = 3000;

/** The browser's owner for a Herdr app terminal, as the hub names it. */
export const ownerOf = (terminal) => (terminal ? `app:${terminal}` : null);

/** How many tabs the browser of a terminal has, in the hub's view. */
export const tabsOf = (model, terminal) => model?.tabs?.[ownerOf(terminal)] ?? 0;

/** The slot's rectangle inside the window, from the window's drawable size, the slot's own size and its insets. */
export function slotRect(viewport, size, insets) {
  if (!viewport || !size || !insets || !(size.width > 0) || !(size.height > 0)) return null;
  return { x: Math.round(viewport.width - insets.right - size.width), y: Math.round(viewport.height - insets.bottom - size.height), width: Math.round(size.width), height: Math.round(size.height) };
}

/** The insets an element's bounds give, as a pointer event reports them in window coordinates. */
export const insetsOf = (viewport, bounds) => ({ right: Math.round(viewport.width - bounds.x - bounds.width), bottom: Math.round(viewport.height - bounds.y - bounds.height) });

/** Whether the slot is what the window shows: the side panel open on it, with the agent's screen in front. */
export function slotShown(tree, name = layoutName()) {
  const stack = name === "cockpit-chat" ? "screen" : "center";
  return panelOpen(tree) && nodeOf("panel-body", tree)?.active === WEB && nodeOf(stack, tree)?.active === "work";
}

/** Whether a popup, a dialog or the palette of cockpit's is open over the window: the browser steps aside for it. */
export function covered(now = Date.now(), copies = globalThis.cockpitCopies ?? []) {
  for (const copy of copies) if (copy.overlay && now - (copy.beat ?? copy.at ?? 0) < 3000) return true;
  return false;
}

/** The theme's colors the browser's toolbar takes, so it reads as part of the window. */
export function toolbarTheme(k) {
  const hex = (color) => (/^#[0-9a-f]{6}([0-9a-f]{2})?$/i.test(String(color)) ? String(color) : undefined);
  return { bg: hex(k.chrome), panel: hex(k.background), fg: hex(k.foreground), muted: hex(k.mutedFg), line: hex(k.border), accent: hex(k.ring), hover: hex(k.accent) };
}

/**
 * Whether a page an agent just opened should bring the panel to the browser: the page is news (`opened.at` after
 * `seen`), it was opened for the session in front, and the panel shows something else.
 */
export function wantsPanel({ opened, seen, f, showing }) {
  if (!opened?.at || opened.at <= seen || showing) return false;
  return Boolean(opened.owner) && opened.owner === ownerOf(f?.terminal);
}

/**
 * What the panel does when another session comes in front: it closes when that session has no browser (and remembers
 * it did), and opens on the browser of one that has, when cockpit had closed it or that session's agent opened a page
 * meanwhile (`unseen`). Nothing while the same session stays in front.
 */
export function panelSteps({ terminal, last, showing, away, count, unseen = false }) {
  if (terminal === last) return [];
  if (showing && count === 0) return [["close"], ["away", true]];
  if (!showing && (away || unseen) && count > 0) return [["open"], ["away", false]];
  return [];
}

/** Opens the panel on the browser's slot, wide enough for a page. */
export function openWeb(tree = layout()) {
  if ((nodeOf("panel", tree)?.size?.px ?? 0) < WIDTH) run("layout.resize", { id: "panel", px: WIDTH });
  openPanel(WEB);
}

/** The slot as the browser needs it now, or null while its size is unknown. */
export function currentSlot() {
  const st = slot();
  const name = layoutName();
  try {
    st.viewport = window.viewport_size();
  } catch {}
  const rect = slotRect(st.viewport, shared(SIZE), shared(MEASURED)?.[name] ?? INSETS[name]);
  if (!rect) return null;
  const f = focused();
  return { rect, viewport: { width: st.viewport.width, height: st.viewport.height }, visible: slotShown(layout(), name), cover: covered(), terminal: f?.terminal ?? null, cwd: f?.cwd ?? null, theme: toolbarTheme(tokens()), layout: name };
}

/** Tells the hub the slot when it changed, from the follower alone; the hub hands it to the browser with what only it knows. */
export function reportSlot() {
  const st = slot();
  if (!st.model?.enabled || !st.model.running) return;
  const now = currentSlot();
  const text = JSON.stringify(now);
  if (!now || text === st.sent) return;
  st.sent = text;
  st.next = now;
  flush();
}

/** Sends the newest slot, one request at a time: a window that animates reports dozens, and the last must land last. */
function flush() {
  const st = slot();
  if (!st.next || (st.sending && Date.now() - st.sending < STUCK_MS)) return;
  const now = { ...st.next, seq: ++st.seq, since: st.since };
  const mine = (st.sending = Date.now());
  st.next = null;
  hub
    .act("browser", "slot", [now])
    .catch(() => (st.sent = ""))
    .finally(() => {
      if (st.sending === mine) st.sending = 0;
      flush();
    });
}

/** Takes the hub's view in: a browser that just started knows no slot yet, so the next report goes whatever was told. */
function took(model) {
  const st = slot();
  if (model?.run !== st.model?.run) st.sent = "";
  st.model = model;
  setShared("cockpit.web.model", model);
}

/**
 * Follows the browser from a view that is always there (the sidebar): the slot is told again whenever the panel, the
 * screen or the session in front changes, a page an agent opens brings the panel to the browser, and the panel leaves
 * with a session that has no browser.
 */
export function followBrowser(cx) {
  let seen = Date.now();
  const sub = hub.subscribe(cx, { views: ["browser"] }, (m, cx) => {
    if (m.view !== "browser" || !m.value) return;
    took(m.value);
    const tree = layout();
    const opened = m.value.opened;
    if (m.value.enabled && nodeOf(WEB, tree) && wantsPanel({ opened, seen, f: focused(), showing: slotShown(tree) })) openWeb(tree);
    // A page opened for a session that is not in front waits for it: the panel opens when that session comes forward.
    else if (opened?.at > seen && opened.owner && opened.owner !== ownerOf(focused()?.terminal)) slot().unseen.add(opened.owner);
    seen = Math.max(seen, opened?.at ?? 0);
    reportSlot();
    cx.notify();
  });
  let stopped = false;
  cx.spawn(async (cx) => {
    while (!stopped) {
      await hub.sleep(cx, 100);
      try {
        if (followPanel()) cx.notify();
        reportSlot();
        // A newer slot that waited behind a report that never came back goes now.
        flush();
      } catch {}
    }
  });
  return { stop: () => ((stopped = true), sub.stop()) };
}

/** Runs `panelSteps` on the window as it is; true when the panel moved. */
export function followPanel() {
  const st = slot();
  const tree = layout();
  const terminal = focused()?.terminal ?? null;
  const last = st.terminal;
  st.terminal = terminal;
  if (last === undefined || !st.model?.enabled || !nodeOf(WEB, tree)) return false;
  const steps = panelSteps({ terminal, last, showing: slotShown(tree), away: shared(AWAY) === true, count: tabsOf(st.model, terminal), unseen: st.unseen.has(ownerOf(terminal)) });
  if (terminal !== last) st.unseen.delete(ownerOf(terminal));
  for (const [step, arg] of steps) {
    if (step === "close") closePanel();
    else if (step === "open") openWeb(tree);
    else if (step === "away") setShared(AWAY, arg);
  }
  return steps.length > 0;
}

export default class Web {
  init(_props, cx) {
    this.model = shared("cockpit.web.model") ?? null;
    this.sub = hub.subscribe(cx, { views: ["browser"] }, (m, cx) => {
      if (m.view !== "browser") return;
      this.model = m.value;
      cx.notify();
    });
  }

  async close() {
    this.sub?.stop();
  }

  /** A pointer over the slot gives its exact place in the window, which corrects the insets from then on. */
  measured(event) {
    const name = layoutName();
    const found = insetsOf(window.viewport_size(), event.bounds);
    const known = shared(MEASURED)?.[name] ?? INSETS[name];
    if (known && found.right === known.right && found.bottom === known.bottom) return;
    setShared(MEASURED, { ...(shared(MEASURED) ?? {}), [name]: found });
  }

  /** A new tab in the browser of the session in front. */
  open(url = "") {
    const f = focused();
    hub.act("browser", "open", [{ url, terminal: f?.terminal ?? null, cwd: f?.cwd ?? null }]).catch(() => {});
  }

  render() {
    const k = tokens();
    const m = this.model;
    const f = focused();
    const message = (icon, title, text, action) =>
      v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap(8)
        .px(24)
        .child(Icon(icon, { size: 22, color: k.mutedFg }))
        .child(div().text_size(TEXT.base).text_color(k.foreground).child(title))
        .child(div().text_size(TEXT.sm).text_color(k.mutedFg).text_center().child(text))
        .when(Boolean(action), (e) => e.child(action));
    const mine = Boolean(m?.current) && m.current === ownerOf(f?.terminal);
    let body;
    if (!m) body = message("globe", "The browser", "Waiting for cockpit's hub.");
    else if (!m.enabled) body = message("globe", "The Chromium side browser is off", "Turn it on in Cockpit settings: Side browser, Chromium.");
    else if (!m.installed) body = message("arrow-down", "The Chromium side browser is not installed", "Run cockpit browser install in a terminal: it downloads Electron and agent-browser once.");
    else if (!f?.terminal) body = message("globe", "No session in front", "A browser belongs to an agent session: put one in front.");
    else if (!m.docked) body = message("globe", "The browser is a window of its own", "Docking is off in Cockpit settings.", Button("web-open", { label: "Open a tab", onClick: () => this.open() }));
    else if (!m.running || tabsOf(m, f.terminal) === 0) {
      body = message("globe", "No page open for this session", "Its agent opens one beside its work, or start with a new tab.", Button("web-new", { label: "New tab", onClick: () => this.open() }));
    } else if (!m.shown && mine && m.frame?.page) {
      // The browser's window is hidden (the window moves, something covers it): its last frame stands in for it.
      const rel = (file) => String(file).replace(`${appHome()}/`, "");
      const height = shared(SIZE)?.height ?? 0;
      body = v_flex()
        .size_full()
        .child(image(rel(m.frame.toolbar)).w_full().h(76).flex_none())
        .child(image(rel(m.frame.page)).w_full().h(Math.max(0, height - 76)));
    } else body = div().size_full();
    return div()
      .id("web-slot")
      .size_full()
      .bg(k.background)
      .rounded(RADIUS.md)
      .overflow_hidden()
      .on_size_change((size, cx) => {
        // Written once the frame is drawn: the shared store is not changed from inside a layout pass.
        const now = { width: size.width, height: size.height };
        cx.spawn(async () => setShared(SIZE, now));
        cx.notify();
      })
      .on_mouse_move((event, cx) => event.bounds && (this.measured(event), cx.notify()))
      .child(body);
  }
}
