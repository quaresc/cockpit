// The browser beside the agent in the reading layout. terminal-browser opens in a pane split right of the agent's
// terminal, in the tab the chat stands in for; the side panel shows that tab zoomed on the browser, beside the chat.

import { run, panes, focused, layout, shared, setShared } from "herdr";
import { openPanel, closePanel, panelOpen } from "./commands.js";
import { layoutName, nodeOf } from "./screens.js";

/** The side panel's child that shows the browser (cockpit-chat.json). */
export const BROWSER = "browser";

/** The pane cockpit zoomed for the panel: it only ever unzooms its own. */
export const ZOOMED = "cockpit.browser.zoomed";

/** Set while the Tab screen holds the browser the panel showed, so the panel shows it again with the chat. */
export const AWAY = "cockpit.browser.away";

/** When cockpit last moved the pane focus with the chat in front: a move takes the keyboard, which the box takes back. */
export const TOOK = "cockpit.browser.took";

/** How wide the panel opens for a browser, when it is narrower. */
const WIDTH = 600;

/** Whether a pane runs terminal-browser. */
export function isBrowserPane(p) {
  return Boolean(p?.terminal) && [p.program?.key, p.program?.name, p.app].some((n) => String(n ?? "").includes("terminal-browser"));
}

/** The terminal-browser pane in a tab, or null. */
export function browserIn(list, tab) {
  return (tab && (list ?? []).find((p) => p.tab === tab && isBrowserPane(p))) || null;
}

/**
 * The steps that make the window show the tab's browser in the panel beside the chat, from what it shows now: `inChat`
 * the chat in front, `onTab` the Tab screen, `child` the panel's shown child, `zoomed` the pane cockpit zoomed, `seen`
 * the browsers already brought in (a panel the person closes stays closed). Pure, so tests can walk every state.
 */
export function browserSteps({ inChat, onTab, list, tab, focusedPane, panel, child, zoomed, away, seen }) {
  const steps = [];
  const browser = inChat ? browserIn(list, tab) : null;
  const showing = panel && child === BROWSER;
  const pane = (id) => (id ? (list ?? []).find((p) => p.pane === id) : null);
  const tabZoomed = (t) => (list ?? []).some((p) => p.tab === t && p.zoomed);
  // The agent's own pane gets the keyboard back when the browser stops filling the tab in front.
  const release = () => {
    const own = pane(zoomed);
    if (!own) return zoomed ? steps.push(["track", null]) : null;
    if (tabZoomed(own.tab)) steps.push(["unzoom", own.pane]);
    const agent = (list ?? []).find((p) => p.tab === own.tab && p.terminal && p.pane !== own.pane && !isBrowserPane(p));
    if (agent && own.tab === tab && focusedPane === own.pane) steps.push(["focus", agent.pane]);
    steps.push(["track", null]);
  };
  if (!browser) {
    if (showing && (onTab || inChat)) steps.push(["close"], ...(onTab ? [["away", true]] : []));
    release();
    return steps;
  }
  const newcomer = !seen.includes(browser.pane);
  if (newcomer) steps.push(["seen", browser.pane]);
  if ((newcomer || away) && !showing) steps.push(["open"]);
  if (away) steps.push(["away", false]);
  if (!showing && !newcomer && !away) {
    release();
    return steps;
  }
  if (zoomed && zoomed !== browser.pane) release();
  // A zoomed tab shows its focused pane: the browser gets the zoom, or the focus when the tab is zoomed already.
  if (!tabZoomed(tab)) steps.push(["zoom", browser.pane]);
  else if (focusedPane !== browser.pane) steps.push(["focus", browser.pane]);
  if (zoomed !== browser.pane) steps.push(["track", browser.pane]);
  return steps;
}

/** Browsers already brought in, and when the last zoom or focus was asked: the window answers it a moment later. */
const seen = [];
let settlingUntil = 0;

/** Runs `browserSteps` on the window as it is, in the reading layout; any other layout gets back the zoom it gave. */
export function syncBrowser(now = Date.now()) {
  if (now < settlingUntil) return false;
  if (layoutName() !== "cockpit-chat" || !nodeOf(BROWSER)) return releaseBrowser();
  const tree = layout();
  const onWork = nodeOf("screen", tree)?.active === "work";
  const view = nodeOf("view", tree)?.active;
  const f = focused();
  const inChat = onWork && view === "chat";
  const steps = browserSteps({
    inChat,
    onTab: onWork && view === "tab",
    list: panes(),
    tab: f?.tab ?? null,
    focusedPane: f?.pane ?? null,
    panel: panelOpen(tree),
    child: nodeOf("panel-body", tree)?.active ?? null,
    zoomed: shared(ZOOMED) ?? null,
    away: shared(AWAY) === true,
    seen,
  });
  for (const [step, arg] of steps) {
    if (step === "seen") seen.push(arg);
    else if (step === "open") open(tree);
    else if (step === "close") closePanel();
    else if (step === "away") setShared(AWAY, arg);
    else if (step === "track") setShared(ZOOMED, arg);
    else if (step === "zoom" || step === "unzoom") run("pane.zoom", { pane: arg });
    else if (step === "focus") run("pane.focus", { pane: arg });
  }
  if (steps.some(([step]) => step === "zoom" || step === "unzoom" || step === "focus")) {
    settlingUntil = now + 800;
    if (inChat) setShared(TOOK, now);
  }
  return steps.length > 0;
}

/** Opens the panel on the browser, wide enough for a page. */
function open(tree) {
  const width = nodeOf("panel", tree)?.size?.px ?? 0;
  if (width < WIDTH) run("layout.resize", { id: "panel", px: WIDTH });
  openPanel(BROWSER);
}

/** Unzooms the pane cockpit zoomed for the panel, if it still is, and gives the keyboard back to the agent's pane. */
export function releaseBrowser() {
  const zoomed = shared(ZOOMED);
  if (!zoomed) return false;
  const list = panes();
  const own = list.find((p) => p.pane === zoomed);
  if (own && list.some((p) => p.tab === own.tab && p.zoomed)) run("pane.zoom", { pane: zoomed });
  const agent = own && list.find((p) => p.tab === own.tab && p.terminal && p.pane !== own.pane && !isBrowserPane(p));
  if (agent && focused()?.pane === zoomed) run("pane.focus", { pane: agent.pane });
  setShared(ZOOMED, null);
  return true;
}

/**
 * Whether the chat's box should take the keyboard back after a move cockpit made with the chat in front (`took`): once
 * the move has landed, and not after another moment, so a later click of the person's stands.
 */
export function boxTakesBack({ took, answered, now }) {
  return Boolean(took) && took !== answered && now - took >= 300 && now - took <= 2000;
}
