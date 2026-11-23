// Which screen the window shows, in whichever cockpit layout is on: `cockpit` stacks its screens under `center`,
// `cockpit-chat` under `screen`. `work` is where you deal with an agent: its panes in one, its conversation in the other.

import { run, shared, setShared, layout, focused, agents } from "herdr";
import { keyed } from "./person.js";

/** The cockpit apps both layouts stack after their own screens: [child, label, icon]. */
const APPS = [
  ["inbox", "Inbox", "inbox"],
  ["runbook", "Runbook", "book"],
  ["todo", "To-do", "list-todo"],
  ["meetings", "Meetings", "calendar"],
  ["monitor", "Monitor", "activity"],
  ["routines", "Routines", "clock"],
  ["mail", "Mail", "mail"],
];

/** A screen as the palette, the app strip and the keys sheet name it, with the command its key is bound to. */
const screen = ([child, label, icon], command = `cockpit.go:${child}`) => ({ child, label, icon, command });

const LAYOUTS = {
  cockpit: {
    stack: "center",
    work: "work",
    first: "home",
    list: [screen(["home", "Home", "layout-dashboard"], "cockpit.home"), screen(["work", "Work", "terminal"]), ...APPS.map((a) => screen(a))],
  },
  // The reading layout puts the conversation and the panes in a stack of their own, beside the side panel.
  "cockpit-chat": {
    stack: "screen",
    work: "chat",
    first: "chat",
    inner: { stack: "view", screen: "work", children: ["chat", "tab"] },
    list: [
      screen(["chat", "Chat", "message-square"]),
      screen(["needs", "Needs you", "hand"]),
      screen(["home", "Home", "layout-dashboard"]),
      screen(["tab", "Tab", "terminal"]),
      ...APPS.map((a) => screen(a)),
    ],
  },
};

/**
 * The cockpit layout a layout is, or is a copy of: its own name, the name its file says it was cloned from, else the
 * longest known name it starts with (`cockpit-chat-copy`); `cockpit` for any other layout.
 */
export function baseOf(name, readTree = () => layout()) {
  if (LAYOUTS[name]) return name;
  let from = null;
  try {
    from = readTree()?.cloned_from?.name ?? null;
  } catch {}
  if (LAYOUTS[from]) return from;
  return Object.keys(LAYOUTS).filter((n) => String(name ?? "").startsWith(`${n}-`)).sort((a, b) => b.length - a.length)[0] ?? "cockpit";
}

/** The cockpit layout shown, a copy of one counting as the one it copies. */
export function layoutName() {
  return baseOf(shared("layout"));
}

/** The shown layout's own name, whose `layout:<name>` section of keys.json is the one that applies. */
export function shownLayout() {
  return shared("layout") ?? "cockpit";
}

/** The shown layout's screens; the cockpit layout's when it is some other layout. */
export function screens() {
  return LAYOUTS[layoutName()];
}

/** A named layout's screens, or null when cockpit has no such layout. */
export function screensOf(name) {
  return LAYOUTS[name] ?? null;
}

/** Shows a screen of the shown layout and gives that app the keyboard. */
export function show(child) {
  const s = screens();
  if (s.inner?.children.includes(child)) {
    run("layout.select", { stack: s.stack, child: s.inner.screen });
    run("layout.select", { stack: s.inner.stack, child });
  } else {
    run("layout.select", { stack: s.stack, child });
  }
  setShared("cockpit.app", child);
  if (child !== s.work) run(`cockpit.show:${child}`);
}

/** A node of the shown layout by id, or null. */
export function nodeOf(id, tree = layout()) {
  if (!tree || typeof tree !== "object") return null;
  if (tree.id === id) return tree;
  for (const c of [...(tree.children ?? []), ...(tree.floating ?? [])]) {
    const found = nodeOf(id, c);
    if (found) return found;
  }
  return null;
}

/** Whether a block showing one agent alone (`{agent: "focused"}`) is on screen: not hidden, nor a stack's other child. */
function agentBlockShown(node, visible = true) {
  if (!visible || !node || typeof node !== "object" || node.hidden) return false;
  if (node.content?.agent) return true;
  const active = node.type === "stack" ? (node.active ?? node.children?.[0]?.id) : null;
  return (node.children ?? []).some((c) => agentBlockShown(c, node.type !== "stack" || c.id === active));
}

/**
 * The agent of `list` (rows with `address` and `tab`) the window shows: the agent block's while one is on screen, else
 * the focused pane's, else one in the tab in front. Herdr names a first agent for its blocks even when the tab in front
 * runs none; that one never counts.
 */
export function agentInView(list, f = focused(), tree = layout()) {
  if (!f || !list?.length) return null;
  const at = (address) => (address ? (list.find((a) => a.address === address) ?? null) : null);
  if (f.agent && agentBlockShown(tree)) return at(f.agent);
  return at(f.shown ?? f.terminal) ?? (f.tab ? (list.find((a) => a.tab === f.tab) ?? null) : null);
}

/**
 * The address of the agent in view among Herdr's agents and `more` (rows the hub lists, with `address` and `tab`): null
 * on a tab no agent runs in.
 */
export function frontAgent(f = focused(), more = []) {
  return agentInView([...agents(), ...more], f)?.address ?? null;
}

/** Whether the reading layout's sidebar is open; it remembers across restarts. */
export const DOCKED = "cockpit.chat.sidebar";

/** Narrower than this, Herdr's brand row has folded the sidebar to a rail, which the reading layout's sidebar never draws. */
const RAIL = 160;

/** Whether the sidebar is in the window: the layout says, so a flag left from elsewhere never hides it or its drawer. */
export function sidebarShown(tree = layout()) {
  const node = nodeOf("sidebar", tree);
  return Boolean(node && !node.hidden);
}

/** Opens or closes the sidebar. */
export function toggleSidebar() {
  showSidebar(!sidebarShown());
}

/** Puts the sidebar in a known state, so the flag and the layout never disagree; one folded to a rail gets its width back. */
export function showSidebar(on) {
  const node = nodeOf("sidebar");
  if (node && (node.size?.px ?? Infinity) < RAIL) {
    const width = Math.min(node.max ?? 480, Math.max(node.min ?? 220, Number(shared("sidebar.width")) || 268));
    run("layout.resize", { id: "sidebar", px: width });
    setShared("sidebar.collapsed", false);
  }
  setShared(DOCKED, on === true);
  run(on ? "layout.show" : "layout.hide", { id: "sidebar" });
}

/**
 * Keeps the reading layout's sidebar open or put away behind its drawer, never between: a fold to a rail (Herdr's brand
 * row button) puts it away, and the flag follows a layout changed from elsewhere. Says what it changed, else null.
 */
export function settleSidebar(tree = layout()) {
  if (layoutName() !== "cockpit-chat") return null;
  const node = nodeOf("sidebar", tree);
  if (!node) return null;
  if (!node.hidden && (node.size?.px ?? Infinity) < RAIL) {
    showSidebar(false);
    return "folded";
  }
  if ((shared(DOCKED) !== false) === !node.hidden) return null;
  setShared(DOCKED, !node.hidden);
  return "flag";
}

/** Shows where you deal with the agent: its panes in the cockpit layout, its conversation in the reading one. */
export function showWork() {
  show(screens().work);
}

/** Shows the tab's panes as they are: the work area in the cockpit layout, the Tab screen in the reading one. */
export function showPanes() {
  show(screens().inner ? "tab" : "work");
}

/** Where the reading layout's chat says which agent it follows: a terminal's address or a driven session's key. */
export const CHAT_SUBJECT = "cockpit.chat.subject";
/** A view the side panel opens on once it follows that view's terminal: `{terminal, name, at}`, for a few seconds. */
export const PENDING_VIEW = "cockpit.show.pending";

/** Where `chat:open` keeps the agent it named, and the agent in front then: the chat stays on it until that changes. */
export const CHAT_PIN = "cockpit.chat.pinned";

/** A question the Cockpit layout leaves the chat to ask once it is up on that session: {key, what}. */
export const CHAT_ASK = "cockpit.chat.ask";

/**
 * A session cockpit drives, read in Cockpit Chat: that layout in front with the chat on the session, and `ask` a
 * question the chat puts first ("handover"). `person` false: a window command asked, and the chat's keys wait for one.
 */
export function readInChat(key, ask = null, person = true) {
  pinChat(key);
  if (ask) setShared(CHAT_ASK, { key, what: ask, at: Date.now() });
  if (layoutName() !== "cockpit-chat") run("layout.switch", { name: "cockpit-chat" });
  const open = `cockpit.via:chat|chat:open:${key}`;
  run(person ? keyed(open) : open);
}

/** Pins the chat to an agent in the current tab and space; null follows the agent in front. */
export function pinChat(key) {
  if (!key) return shared(CHAT_PIN) && setShared(CHAT_PIN, null);
  const f = focused();
  setShared(CHAT_PIN, { key, focus: frontAgent(f), tab: f?.tab ?? null, space: f?.space ?? null, at: Date.now() });
}

/**
 * A tab in front with what it holds: in the reading layout its agent's conversation, or its panes on the Tab screen
 * when no agent runs there (a shell, a tab Herdr numbers "3"); in the cockpit layout its panes.
 */
export function showTab(id) {
  pinChat(null);
  run("tab.select", { id });
  const s = screens();
  if (!s.inner) return show(s.work);
  const here = agents().filter((a) => a.tab === id);
  const agent = here.find((a) => a.focused) ?? here[0];
  // Herdr's agent blocks name a first agent when the pane in front runs none: the chat is told which one this tab runs.
  if (agent) run("agent.focus", { terminal: agent.address });
  show(agent ? s.work : "tab");
}

