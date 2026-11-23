// The window's commands for both cockpit layouts: what keys.json binds, what the palette lists and what the keys sheet
// shows. The one copy each layout always shows runs them (the status bar in Cockpit, the top bar in Cockpit Chat).

import { run, shared, setShared, keys, commands, focused, openOverlay, spaces, notify, terminals } from "herdr";
import * as hub from "./hub.js";
import { fleet, goToAgent } from "./fleet.js";
import { show, showWork, screens, screensOf, layoutName, shownLayout, toggleSidebar, nodeOf, agentInView, CHAT_SUBJECT, CHAT_PIN, pinChat } from "./screens.js";
import { shownScreen } from "./front.js";
import { follow } from "./notify.js";
import { openUpdate } from "./update.js";
import { openCode } from "./code.js";
import { openSettings } from "./doctor.js";
import { openUsage } from "./usage.js";
import { openStart } from "./start.js";
import { keyed, take } from "./person.js";
import { openMenu } from "./ui.js";
import { undoToast } from "./agentsui.js";
import { projectPanels } from "./projects.js";

/** The view that runs the window's commands in each layout: always shown there, and only once. */
export const RUNNER = { cockpit: "status", "cockpit-chat": "chat-top" };

// ---------------------------------------------------------------------------------------------------------------------
// Keys

const SYMBOL = { cmd: "⌘", shift: "⇧", alt: "⌥", ctrl: "⌃", enter: "↵", escape: "Esc", backspace: "⌫", space: "Space", tab: "⇥", up: "↑", down: "↓", left: "←", right: "→" };

/** "cmd-shift-p" as the caps ⌘ ⇧ P. */
export function keyCaps(k) {
  const parts = k.endsWith("--") ? [...k.slice(0, -2).split("-"), "-"] : k.split("-");
  return parts.map((p) => SYMBOL[p] ?? (p.length === 1 ? p.toUpperCase() : p.charAt(0).toUpperCase() + p.slice(1)));
}

/** A key as one string of caps: "⌘⌥0". */
export const keyText = (k) => keyCaps(k).join("");

/** The keys that work in a layout: its own section's, then Herdr's global ones it does not take over. */
export function bindings(layout = shownLayout(), bound = keys()) {
  const own = bound.filter((b) => b.section === `layout:${layout}`);
  const taken = new Set(own.map((b) => b.keys));
  return [...own, ...bound.filter((b) => b.section === "global" && !taken.has(b.keys))];
}

/** The key an action is bound to in a layout, or null. */
export function keyFor(action, layout = shownLayout(), bound = keys()) {
  return bindings(layout, bound).find((b) => b.action === action)?.keys ?? null;
}

/** A tooltip with its key after two spaces when the shown layout binds one: "Home  ⌘⌥0". */
export function hint(label, action) {
  const k = action ? keyFor(action) : null;
  return k ? `${label}  ${keyText(k)}` : label;
}

// ---------------------------------------------------------------------------------------------------------------------
// What the commands are called

/** A command of cockpit's besides its screens: what the palette and the keys sheet call it, and where it works. */
const entry = (command, title, icon, group, words, more = {}) => ({ command, title, icon, group, words, ...more });

/**
 * cockpit's commands besides its screens. `layouts` names the layouts where a view handles one (the queue drawer, the
 * chat), `screen` the screen whose view that is, and `driven` the ones for a session cockpit drives.
 */
export const CATALOGUE = [
  entry("cockpit.palette", "Search or run a command", "search", "Window", "palette find"),
  entry("cockpit.keys", "Keys", "keyboard", "Window", "shortcuts keybindings help sheet"),
  entry("cockpit.settings", "Cockpit settings", "sliders-horizontal", "Window", "options preferences config version update check switch toggle"),
  entry("cockpit.settings:health", "Cockpit settings: Health", "shield-check", "Window", "doctor diagnostics checks report setup getting started hub fix"),
  entry("cockpit.settings:agents", "Cockpit settings: Agents", "bot", "Window", "installed providers can start resume fork tools"),
  entry("cockpit.settings:layout", "Cockpit settings: Layout", "layout", "Window", "cockpit chat reading look switch screenshot"),
  entry("cockpit.next-waiting", "Next agent waiting for you", "bell", "Agents", "needs you question jump"),
  entry("cockpit.next:blocked", "Next blocked agent", "circle-dot", "Agents", "needs you"),
  entry("cockpit.next:done", "Next finished agent", "circle-check", "Agents", "done"),
  entry("cockpit.recap", "Recap the agents", "sparkles", "Agents", "whose move stages summary"),
  entry("cockpit.usage", "Usage and limits", "gauge", "Agents", "quota limits tokens spend estimate account plan"),
  entry("cockpit.start", "Start work…", "play", "Agents", "new agent worktree ticket pr prompt task branch"),
  entry("cockpit.resume", "Resume stopped agents", "play", "Agents", "restart reboot resume session"),
  entry("cockpit.queue", "Prompt queue", "list", "Agents", "follow-ups send later", { layouts: ["cockpit"] }),
  entry("cockpit.session", "Start a session cockpit drives", "bot", "Agents", "claude driven new chat"),
  entry("cockpit.agent-view", "The focused agent alone", "bot", "This agent", "focus lone terminal tab toggle", { layouts: ["cockpit"] }),
  entry("cockpit.terminal", "Its terminal", "terminal", "This agent", "pane shell real resume"),
  entry("cockpit.chat:mode", "What it may do without asking: the next mode", "shield-check", "This agent", "permission mode cycle", { layouts: ["cockpit-chat"], screen: "chat", driven: true }),
  entry("cockpit.chat:stop", "Stop its turn", "circle-stop", "This agent", "interrupt escape", { layouts: ["cockpit-chat"], screen: "chat", driven: true }),
  entry("cockpit.dismiss", "Dismiss this agent until it asks again", "x", "This agent", "needs you set aside hide waiting card"),
  entry("cockpit.dismiss:back", "Bring this agent back to Needs you", "undo", "This agent", "dismissed restore undo waiting card"),
  entry("cockpit.pr", "This branch's PR", "git-pull-request", "This agent", "pull request github"),
  entry("cockpit.ticket", "This branch's ticket", "ticket", "This agent", "linear jira issue"),
  entry("cockpit.panel", "Side panel", "panel-right", "Side panel", "show review runbook files changes toggle"),
  entry("cockpit.panel:show", "Side panel: Show", "layout", "Side panel", "charts tables diagrams"),
  entry("cockpit.panel:review", "Side panel: Review", "git-pull-request", "Side panel", "diff comments"),
  entry("cockpit.panel:runbook", "Side panel: Runbook", "book", "Side panel", "steps ticket"),
  entry("cockpit.panel:files", "Side panel: Files", "folder", "Side panel", "tree"),
  entry("cockpit.panel:changes", "Side panel: Changes", "git-fork", "Side panel", "git diff"),
  entry("cockpit.sidebar", "Sidebar", "panel-right", "Window", "pin drawer open close", { layouts: ["cockpit-chat"] }),
  entry("cockpit.new", "Start something new", "plus", "Window", "terminal agent space worktree tab", { layouts: ["cockpit-chat"] }),
  entry("cockpit.tabs", "Every tab, the ones that do not fit too", "app-window", "Window", "tabs more overflow switch", { layouts: ["cockpit-chat"] }),
];

/** Older names of commands, which keys bound to them still run: the keys sheet names each as what it opens now. */
export const ALIASES = { "cockpit.diagnostics": "cockpit.settings:health" };

/** What the palette runs for an entry: a view's command goes through the runner, which shows that view's screen first. */
export function paletteAction(e) {
  return e.screen ? `cockpit.via:${e.screen}|${e.command.slice(8)}` : e.command;
}

/** Herdr's own names for its commands: [command, args, group, title, keywords]. */
export const NAMED = [
  ["tab.new", {}, "Tab", "New tab", "tab agent new task"],
  ["mod.open", { name: "picker" }, "Tab", "Go to space or tab…", "switch jump find"],
  ["tab.rename", {}, "Tab", "Rename tab", "tab"],
  ["tab.close", {}, "Tab", "Close tab", "tab kill"],
  ["tab.next", {}, "Tab", "Next tab", "tab"],
  ["tab.prev", {}, "Tab", "Previous tab", "tab"],
  ["pane.split", { direction: "right" }, "Pane", "Split right", "vertical"],
  ["pane.split", { direction: "down" }, "Pane", "Split down", "horizontal"],
  ["pane.close", {}, "Pane", "Close pane", "kill"],
  ["pane.zoom", {}, "Pane", "Zoom pane", "maximize fullscreen"],
  ["pane.rename", {}, "Pane", "Rename pane", "name title"],
  ...["left", "right", "up", "down"].map((d) => ["pane.resize", { direction: d }, "Pane", `Move pane edge ${d}`, "resize size wider narrower taller"]),
  ...["left", "right", "up", "down"].map((d) => ["pane.go", { direction: d }, "Pane", `Focus pane ${d}`, "move go"]),
  ...["left", "right", "up", "down"].map((d) => ["pane.swap", { direction: d }, "Pane", `Swap pane ${d}`, "move"]),
  ["space.new", {}, "Space", "New space", "workspace"],
  ["space.rename", {}, "Space", "Rename space", "workspace"],
  ["space.close", {}, "Space", "Close space", "workspace kill"],
  ["space.next", {}, "Space", "Next space", "workspace"],
  ["space.prev", {}, "Space", "Previous space", "workspace"],
  ["layout.switch", {}, "Layout", "Next layout", "look switch"],
  ["layout.reset", {}, "Layout", "Reset layout", "new version default restore herdr's"],
  ["layout.clone", {}, "Layout", "Duplicate layout", "clone copy customize"],
  ["mod.clone", {}, "Layout", "Duplicate mod", "clone copy customize"],
  ["terminal.open", {}, "Terminal", "New terminal", "shell"],
  ["terminal.copy", {}, "Terminal", "Copy", "clipboard"],
  ["terminal.paste", {}, "Terminal", "Paste", "clipboard"],
  ["agent.next", {}, "Agent", "Next agent", ""],
  ["agent.prev", {}, "Agent", "Previous agent", ""],
  ["window.minimize", {}, "Window", "Minimize window", ""],
  ["window.maximize", {}, "Window", "Maximize window", "restore"],
  ["window.close", {}, "Window", "Close window", "quit"],
  ["app.settings", {}, "App", "Settings", "preferences options config look font theme sound"],
  ["app.help", {}, "App", "Herdr's keys", "keybindings shortcuts help"],
  ["app.welcome", {}, "App", "Welcome", "intro start onboarding"],
  ["app.whats_new", {}, "App", "What's new", "release changes news update"],
  ["app.reset", {}, "App", "Reset to the default screen", "escape hatch broken restore"],
  ["app.zoom_in", {}, "App", "Zoom in", "bigger font size"],
  ["app.zoom_out", {}, "App", "Zoom out", "smaller font size"],
  ["app.zoom_reset", {}, "App", "Reset zoom", "font size"],
  ["app.quit", {}, "App", "Quit Herdr", "exit"],
  ...["graphite", "ink", "moss", "atom-one-dark"].map((n) => ["app.theme", { name: n }, "App", `Theme: ${n.replace(/-/g, " ")}`, "colors look"]),
];

/** A Herdr command's name in words: "tab.rename" reads "Rename tab". */
export function titleOf(name) {
  const [noun, verb] = name.split(".");
  return { group: noun.charAt(0).toUpperCase() + noun.slice(1), title: `${(verb ?? "").replace(/_/g, " ")} ${noun}`.replace(/^./, (c) => c.toUpperCase()) };
}

/** keys.json's `name:value` as [name, value]; value is undefined without one. */
function splitAction(action) {
  const at = action.indexOf(":");
  return at < 0 ? [action, undefined] : [action.slice(0, at), action.slice(at + 1)];
}

/** Keys a view takes itself, the same in every app that has them: a page header's source chips. */
export const VIEW_KEYS = [
  ["shift-l", "Sign in to a source again"],
  ["shift-r", "Read the sources again"],
];

/** Keys the reading layout's chat takes itself, while the keyboard is in it: [keys, title, layout]. */
export const CHAT_KEYS = [
  ["shift-tab", "What it may do without asking: the next mode", "cockpit-chat"],
  ["escape", "Stop its turn", "cockpit-chat"],
  ["tab", "Complete a / or @, or put the keyboard on the question", "cockpit-chat"],
];

/** The keys sheet's groups, in order. */
const SHEET = ["Screens", "Agents", "This agent", "Side panel", "Window", "In the apps", "Herdr"];

/** What a keys.json action does, in words and in which group of the sheet: {title, group, icon}. */
export function describe(action, layout = layoutName()) {
  const s = screensOf(layout)?.list.find((x) => x.command === action);
  if (s) return { title: s.label, group: "Screens", icon: s.icon };
  const c = CATALOGUE.find((e) => e.command === (ALIASES[action] ?? action));
  if (c) return { title: c.title, group: c.group, icon: c.icon };
  const [name, value] = splitAction(action);
  const named = NAMED.find(([n, args]) => n === name && (value === undefined ? Object.keys(args).length === 0 : Object.values(args)[0] === value));
  if (named) return { title: named[3], group: "Herdr", icon: null };
  return { title: `${titleOf(name).title}${value === undefined ? "" : ` ${value}`}`, group: "Herdr", icon: null };
}

/**
 * The keys sheet of a layout: its bindings grouped, Herdr's global ones it keeps last, cockpit's in the order of its
 * screens and commands whatever order keys() gives. [{title, rows: [{keys, action, title, group, icon}]}].
 */
export function sheet(layout = layoutName(), bound = keys(), section = shownLayout()) {
  const order = [...(screensOf(layout)?.list ?? []).map((s) => s.command), ...CATALOGUE.map((e) => e.command)];
  const at = (r) => (r.group === "Herdr" ? 0 : order.indexOf(r.action) + 1 || order.length + 1);
  const rows = bindings(section, bound).map((b) => ({ keys: b.keys, action: b.action, ...describe(b.action, layout) }));
  rows.push(...VIEW_KEYS.map(([k, title]) => ({ keys: k, action: null, title, group: "In the apps", icon: null })));
  rows.push(...CHAT_KEYS.filter(([, , l]) => l === layout).map(([k, title]) => ({ keys: k, action: null, title, group: "This agent", icon: null })));
  // Herdr's commands a layout binds keep NAMED's order (left, right, up, down), whatever order keys() lists them in.
  const named = (r) => {
    const [name, value] = splitAction(r.action ?? "");
    const n = NAMED.findIndex(([c, args]) => c === name && (value === undefined || Object.values(args)[0] === value));
    return n < 0 ? NAMED.length : n;
  };
  const sorted = rows.map((r, i) => [r, i]).sort(([a, i], [b, j]) => at(a) - at(b) || named(a) - named(b) || i - j).map(([r]) => r);
  return SHEET.map((title) => ({ title, rows: sorted.filter((r) => r.group === title) })).filter((g) => g.rows.length > 0);
}

/** keys.json's value for an argument: a number when it is written as one (`tab.select:1`). */
const argValue = (value) => (/^-?\d+$/.test(value) ? Number(value) : value);

/** Runs a keys.json action: a cockpit command, or Herdr's `name:value` with the value as its first argument. */
export function runAction(action) {
  if (action.startsWith("cockpit.")) return run(action);
  const [name, value] = splitAction(action);
  if (value === undefined) return run(name, {});
  const arg = commands().find((c) => c.name === name)?.args?.[0]?.name;
  return run(name, arg ? { [arg]: argValue(value) } : {});
}

// ---------------------------------------------------------------------------------------------------------------------
// The side panel

/** The side panel's children by the names keys and the activity bar use. */
export const PANELS = { show: "show", review: "review", runbook: "runbook-panel", files: "files", changes: "changes" };

/**
 * The panel a name shows: cockpit's Files and Changes, which follow the projects the chat's agent works in, where the
 * layout has them and `chat.projectPanels` is on, else Herdr's.
 */
export function panelChild(name, tree) {
  const child = PANELS[name] ?? name;
  const ours = `project-${child}`;
  return (child === "files" || child === "changes") && projectPanels() && nodeOf(ours, tree) ? ours : child;
}

/** Whether the side panel is open, and on what, as last set; `panelOpen` reads what the shown layout really does. */
export const PANELED = "cockpit.panel.open";
export const PANEL_CHILD = "cockpit.panel.child";

/** Whether the shown layout's side panel is open: each layout keeps its own, so a flag set in the other never decides. */
export function panelOpen(tree) {
  const node = nodeOf("panel", tree);
  return Boolean(node && !node.hidden);
}

/** Puts the flag back on what the shown layout does; true when it was off. */
export function settlePanel(tree) {
  const open = panelOpen(tree);
  if ((shared(PANELED) === true) === open) return false;
  setShared(PANELED, open);
  return true;
}

/** Opens the side panel on that child, left open when it already shows it. */
export function openPanel(name) {
  const child = (name && panelChild(name)) ?? shared(PANEL_CHILD) ?? "show";
  setShared(PANELED, true);
  setShared(PANEL_CHILD, child);
  run("layout.select", { stack: "panel-body", child });
  run("layout.show", { id: "panel" });
  // The panel only shows beside the agent, so opening it brings that screen up in whichever layout is on.
  if (shared("cockpit.app") !== screens().work) showWork();
}

/** Closes the side panel, and says so: the drawer on the edge takes over from the flag. */
export function closePanel() {
  setShared(PANELED, false);
  run("layout.hide", { id: "panel" });
}

/** Opens the side panel on that child, or closes it when it already shows it. */
export function togglePanel(name) {
  const child = name && panelChild(name);
  const showing = nodeOf("panel-body")?.active ?? shared(PANEL_CHILD);
  if (panelOpen() && (!child || showing === child)) closePanel();
  else openPanel(child);
}

// ---------------------------------------------------------------------------------------------------------------------
// Overlays

const PALETTE = { mod: "cockpit", kind: "popup", frame: "none", width: 1, height: 1, at: { x: 0, y: 0 } };

/** When an open palette last answered `cockpit.palette` or `cockpit.keys` itself, so the runner opens none then. */
export const ANSWERED = "cockpit.palette.closed";

/** Opens the palette over the window (or its keys sheet), unless an open one answers the same command by itself. */
export function togglePalette(cx, sheet = null) {
  // `popup: true` tells front.js the copy is a popup, so keys pressed from outside reach it first.
  const open = () => openOverlay({ ...PALETTE, props: sheet ? { app: "palette", sheet, popup: true } : { app: "palette", popup: true } });
  if (!cx) return open();
  const asked = Date.now();
  cx.timer.after(80, () => {
    if ((shared(ANSWERED) ?? 0) < asked) open();
  });
}

/** The recap of every agent, on one of its tabs. */
export function openRecap(tab) {
  openOverlay({ mod: "cockpit", props: { app: "recap", popup: true, ...(tab ? { tab } : {}) }, kind: "dialog", width: 0.8, height: 0.8 });
}

/** Herdr's own new menu: a terminal, an agent, a space, a worktree. */
export function openNew() {
  openOverlay({ mod: "new-menu", kind: "popup", width: 250, height: 512 });
}

// ---------------------------------------------------------------------------------------------------------------------
// Agents

/**
 * Copies of the window that follow the agents view and lend it, each as a `() => value | null` with the load of hub.js
 * it runs: a load that stopped running (a reload left its views behind) lends nothing.
 */
const lenders = () => (globalThis.cockpitAgentLent ??= new Map());

/** Lends a copy's agents view to the window's commands until the returned function is called. */
export function lend(fn) {
  lenders().set(fn, hub.LOAD);
  return () => lenders().delete(fn);
}

/** The agents view some running copy follows now, the newest load's first, else null. */
export function lent() {
  for (const [fn, load] of [...lenders()].sort((x, y) => y[1] - x[1])) {
    if (!hub.liveLoad(load)) continue;
    try {
      const value = fn();
      if (value) return value;
    } catch {}
  }
  return null;
}

/** The agents now, to `then(list, cx)`: the runner's own or another copy's when one follows them, else the hub's. */
function withAgents(cx, ctx, then) {
  const rows = ctx?.rows ?? lent();
  if (rows) return then(fleet(rows, ctx?.home ?? null), cx);
  cx.spawn(async (cx) => {
    let rows = null;
    try {
      rows = await hub.view("agents", { frontend: "app" });
    } catch {}
    then(fleet(rows, null), cx);
    cx.notify();
  });
}

/**
 * The agent the person looks at: in Cockpit Chat the one a `chat:open` pinned while the same pane is in front, else the
 * chat's subject; in Cockpit the focused pane's.
 */
function looking() {
  const f = focused();
  const front = f?.agent ?? f?.terminal ?? null;
  if (layoutName() !== "cockpit-chat") return front;
  const pin = shared(CHAT_PIN);
  return (pin?.key && pin.focus === front ? pin.key : null) ?? shared(CHAT_SUBJECT) ?? front;
}

/** The agents at a stage of Home's (null: needs you), the one waiting longest first. */
function atStage(list, stage) {
  return list.filter((a) => (stage ? a.stage === stage || a.status === stage : a.stage === "needs")).sort((a, b) => (a.since ?? 0) - (b.since ?? 0));
}

/**
 * The first agent after `here` at a stage that `reachable` says can be brought up, going round once, and the ones it
 * passed on the way. Needs you falls back to done when nobody needs you.
 */
export function nextOf(list, stage, here, reachable = () => true) {
  const ring = atStage(list, stage);
  const at = ring.length || stage !== null ? ring : atStage(list, "done");
  const i = at.findIndex((a) => a.address === here || a.handle === here);
  const passed = [];
  for (let k = 1; k <= at.length; k++) {
    const a = at[(i + k) % at.length];
    if (reachable(a)) return { agent: a, passed };
    passed.push(a);
  }
  return { agent: null, passed };
}

/** Whether an agent can be brought up here: a session cockpit drives is read in the chat, a terminal must be running. */
function reachableNow() {
  let live = null;
  try {
    const all = terminals();
    if (Array.isArray(all) && all.length) live = new Set(all.filter((t) => t?.state === "live").map((t) => t.address));
  } catch {}
  return (a) => a.driven === true || !live || live.has(a.address);
}

/** Where the last jump went from and to: a press soon after, while the focus never left `from`, goes on from `to`. */
let jumped = null;

/** The agent in view, as the status bar names it. */
const current = (list) => agentInView(list);

/**
 * Brings the next agent at a stage into view: a session cockpit drives is read in the chat, which has no pane for it. An
 * agent with no running terminal is passed, and so is one the last press could not bring up, and the person is told.
 */
const next = (stage) => (_arg, cx, ctx) =>
  withAgents(cx, ctx, (list) => {
    const here = looking();
    const from = jumped && jumped.stage === stage && jumped.from === here && Date.now() - jumped.at < 30_000 ? jumped.to : here;
    const { agent, passed } = nextOf(list, stage, from, reachableNow());
    const names = passed.map((a) => a.title || a.address).join(", ");
    const stays = agent && (agent.handle === here || agent.address === here);
    const onTo = !agent ? "Nobody else waits." : stays ? "The one in front is the only one waiting you can reach." : `On to ${agent.title || agent.address}.`;
    if (passed.length) notify({ title: passed.length === 1 ? `${names} has no running terminal` : `${passed.length} agents have no running terminal`, body: `${onTo}${passed.length > 1 ? ` Passed: ${names}.` : ""}`, level: "info", timeout: 5000 });
    else if (from !== here && agent) notify({ title: "That agent did not come up", body: `On to ${agent.title || agent.address}.`, level: "info", timeout: 4000 });
    if (!agent) return;
    jumped = { stage, from: here, to: agent.handle ?? agent.address, at: Date.now() };
    goToAgent(agent, ctx?.person);
  });

const openLink = (pick) => (_arg, cx, ctx) =>
  withAgents(cx, ctx, (list, cx) => {
    const url = pick(current(list));
    if (url) cx.spawn(async () => hub.openUrl(url));
  });

// ---------------------------------------------------------------------------------------------------------------------
// The commands

/**
 * Shows a screen, then hands its view a command once it shows: a hidden view hears none. The person's command (`person`)
 * goes on with a key of its own; a window command's goes on without one.
 */
function handOn(screen, command, cx, person = false) {
  // The chat's subject is set here, at once: the chat follows it whenever its copy comes up.
  if (command.startsWith("chat:open:")) pinChat(command.slice("chat:open:".length));
  show(screen);
  cx.timer.after(150, () => run(person ? keyed(`cockpit.${command}`) : `cockpit.${command}`));
}

/** The chat's subject, else the agent in front. */
function subject() {
  const f = focused();
  return shared(CHAT_SUBJECT) ?? f?.agent ?? f?.terminal ?? null;
}

/** The agents view's row for this key, null when it lists none, undefined when the view cannot be read. */
async function rowOf(key) {
  const rows = lent()?.agents ?? (await hub.view("agents", { frontend: "app" }).catch(() => null))?.agents;
  return rows ? (rows.find((r) => r.terminal === key || r.address === key || r.terminal === `app:${key}`) ?? null) : undefined;
}

/** Says what went wrong with a hub action the person asked for. */
function said(r, title) {
  if (r?.error) notify({ title, body: String(r.error).replace(/^[a-z_]+: /, ""), level: "warning", timeout: 5000 });
}

/**
 * The chat's agent in its real terminal, as a window over the reading layout: what a reader cannot show. A session
 * cockpit drives has no terminal: the chat asks before handing it over to one of its own.
 */
function openTerminal(cx) {
  const key = subject();
  if (!key) return;
  cx.spawn(async (cx) => {
    const row = await rowOf(key);
    // Nothing known of it: no terminal is opened on a guess.
    if (!row) return notify({ title: "No terminal to open", body: row === undefined ? "Cockpit's hub is not answering." : "No agent by that name runs now.", level: "info" });
    if (!row.driven) return windowFor(key);
    if (hub.remoteMachine()) return notify({ title: "It runs on the runner", body: "A remote desk cannot hand it over to a terminal.", level: "info" });
    handOn("chat", "chat:handover", cx);
  });
}

/** The terminal of an agent Herdr runs, in a window over the reading layout with its own head bar. */
function windowFor(key) {
  const address = key.replace(/^app:/, "");
  openOverlay({
    kind: "window",
    key: "cockpit-chat-terminal",
    width: 0.7,
    height: 0.7,
    frame: "none",
    layout: {
      type: "column",
      background: "background",
      radius: 12,
      border: { width: 1, color: "border" },
      shadow: "lg",
      children: [
        { type: "block", size: { px: 34 }, content: { mod: "cockpit", props: { app: "agent-head", place: "overlay", agent: address } } },
        { type: "block", content: { terminal: address } },
      ],
    },
  });
}

/**
 * Starts a session cockpit drives in the folder of the space in front (else the chat agent's), and opens its chat.
 * Asked by a window command, a key binding included, it opens a menu naming it, which the person picks from.
 */
function startSession(cx, person) {
  const here = spaces().find((s) => s.active) ?? spaces()[0];
  const known = (lent()?.agents ?? []).find((r) => r.address === subject() || r.terminal === subject());
  const cwd = here?.folder || known?.cwd;
  if (!cwd) return notify({ title: "Nowhere to start it", body: "The space in front has no folder.", level: "info" });
  if (!person) return openMenu([{ label: `Start a session in ${cwd.split("/").pop() || cwd}`, icon: "bot", command: "session" }], { width: 280, clickOnly: true });
  cx.spawn(async (cx) => {
    // The first agent of the table cockpit can drive: the hub picks it.
    const r = await hub.act("session", "start", [{ cwd, mode: "default" }]).catch((e) => ({ error: String(e?.message ?? e) }));
    said(r, "Could not start it");
    const key = /^started (\S+)/.exec(r?.note ?? "")?.[1];
    if (key) handOn("chat", `chat:open:${key}`, cx, true);
  });
}

/**
 * Sets an agent aside until it asks something new, or brings it back (`back`): the one `named`, else the one the person
 * looks at, `wait` the wait a menu showed. Asked by a window command, a key binding or a toast's click included, it
 * opens a menu naming the agent, which the person picks from.
 */
function setAside(cx, person, back, named = null, wait = null) {
  const key = named || looking();
  if (!key) return notify({ title: "No agent in front", level: "info", timeout: 3000 });
  const terminal = String(key).includes(":") ? String(key) : `app:${key}`;
  cx.spawn(async () => {
    const row = await rowOf(key);
    const title = row?.title ?? "the agent";
    if (!person) {
      const item = back
        ? { label: `Bring ${title} back to Needs you`, icon: "undo", command: `dismiss:back:${terminal}` }
        : { label: `Dismiss ${title} until it asks again`, icon: "x", command: `dismiss:at:${terminal}|${row?.wait ?? ""}` };
      return openMenu([item], { width: 340, clickOnly: true });
    }
    const named = wait ?? (back ? null : row?.wait ?? null);
    const r = await hub.act("agents", "dismiss", [{ terminal, undo: back, ...(named ? { wait: named } : {}) }]).catch((e) => ({ error: String(e?.message ?? e) }));
    said(r, back ? "Could not bring it back" : "Could not dismiss it");
    if (r?.error) return;
    if (back) notify({ title: `${row?.title ?? "The agent"} is back in Needs you`, level: "success", timeout: 3000 });
    else undoToast(terminal, row?.title ?? "the agent");
  });
}

/** The window's commands in both layouts; a name ending in ":" takes the rest of the command as its argument. */
const WINDOW = {
  palette: (_a, cx) => togglePalette(cx),
  keys: (_a, cx) => togglePalette(cx, "keys"),
  recap: () => openRecap(),
  "recap:": (tab) => openRecap(tab),
  "next-waiting": next(null),
  "next:blocked": next("needs"),
  "next:done": next("done"),
  panel: () => togglePanel(null),
  "panel:": (name) => togglePanel(name),
  "doc:": (name, _cx, ctx) => {
    openPanel("show");
    run(ctx.person ? keyed(`cockpit.show:go:${name}`) : `cockpit.show:go:${name}`);
  },
  // `go:work` is where you deal with an agent in either layout: the panes in one, the chat in the other.
  "go:": (child) => show(child === "work" ? screens().work : child),
  // `via:<screen>|<command>`: that screen first, then the command to its view (the palette's chat entries).
  "via:": (arg, cx, ctx) => handOn(arg.slice(0, arg.indexOf("|")), arg.slice(arg.indexOf("|") + 1), cx, ctx.person),
  // The agent in front leaves what needs you until it asks something new; `dismiss:back[:<terminal>]` brings it back.
  dismiss: (_a, cx, ctx) => setAside(cx, ctx.person, false),
  // `dismiss:at:<terminal>|<wait>` from its menu, `dismiss:back[:<terminal>]`, and a toast's `dismiss:undo:<terminal>`,
  // which only opens the menu that brings it back.
  "dismiss:": (arg, cx, ctx) => {
    const [what, ...rest] = arg.split(":");
    const named = rest.join(":");
    if (what === "back") setAside(cx, ctx.person, true, named || null);
    else if (what === "undo" && named) setAside(cx, false, true, named);
    else if (what === "at" && named.includes("|")) setAside(cx, ctx.person, false, named.slice(0, named.indexOf("|")), named.slice(named.indexOf("|") + 1) || null);
  },
  pr: openLink((me) => me?.pr?.url),
  ticket: openLink((me) => me?.ticketUrl),
  resume: (_a, cx) => cx.spawn(async () => hub.act("home", "resume", [])),
  // A web page only: `open` given a file or an app would run it.
  "link:": (url, cx) => /^https?:\/\//.test(url) && cx.spawn(async () => hub.openUrl(url)),
  // A notification's row, a URL or a space: it only ever navigates (notify.js).
  "target:": (target, cx) => follow(target, cx),
  update: (_a, _cx, ctx) => openUpdate({ person: ctx.person }),
  // Opened by a menu, the palette or a key of the window's, the sheet counts what it shows as seen; by a bare command, not.
  settings: (_a, _cx, ctx) => openSettings({ person: ctx.person }),
  "settings:": (tab, _cx, ctx) => openSettings({ tab, person: ctx.person }),
  // The name the sheet had: a key bound to it still opens it, on the checks it showed.
  diagnostics: (_a, _cx, ctx) => openSettings({ tab: "health", person: ctx.person }),
  usage: () => openUsage(),
  start: () => openStart(),
  kit: () => openOverlay({ mod: "cockpit", props: { app: "kit" }, kind: "window", key: "cockpit-kit", width: 0.85, height: 0.9 }),
  "kit:": (tab) => openOverlay({ mod: "cockpit", props: { app: "kit", tab }, kind: "window", key: "cockpit-kit", width: 0.85, height: 0.9 }),
  "code:open:": (path) => openCode(path),
};

/** What only one layout has: Home and the lone agent in Cockpit, the sidebar and the new menu in Cockpit Chat. */
const OWN = {
  cockpit: {
    home: () => show((shared("cockpit.app") ?? "home") === "home" ? "work" : "home"),
    "agent-view": () => {
      const on = shared("cockpit.view") === "agent";
      run("layout.select", { stack: "view", child: on ? "main" : "agent" });
      setShared("cockpit.view", on ? "main" : "agent");
      if ((shared("cockpit.app") ?? "home") !== "work") show("work");
    },
  },
  "cockpit-chat": {
    sidebar: () => toggleSidebar(),
    new: () => openNew(),
    terminal: (_a, cx) => openTerminal(cx),
    session: (_a, cx, ctx) => startSession(cx, ctx.person),
  },
};

const has = (table, name) => Object.prototype.hasOwnProperty.call(table, name);

/** Whether a layout has this screen. */
const hasScreen = (layout, child) => Boolean(screensOf(layout)?.list.some((s) => s.child === child));

/** Whether an argument names something the layout has: a screen of its own for `go:` and `via:`, a panel for `panel:`. */
function argOk(name, arg, layout) {
  if (name === "go:") return arg === "work" || hasScreen(layout, arg);
  if (name === "via:") return arg.includes("|") && hasScreen(layout, arg.slice(0, arg.indexOf("|")));
  if (name === "panel:") return has(PANELS, arg);
  return true;
}

/** The handler a command reaches in a layout, with its argument; null when the runner has none. */
function lookup(c, layout) {
  const tables = [OWN[layout] ?? {}, WINDOW];
  for (const t of tables) if (has(t, c)) return { fn: t[c], arg: undefined };
  for (const t of tables) {
    for (const name of Object.keys(t)) {
      if (!name.endsWith(":") || !c.startsWith(name) || c.length <= name.length) continue;
      const arg = c.slice(name.length);
      return argOk(name, arg, layout) ? { fn: t[name], arg } : null;
    }
  }
  return null;
}

/** The names the runner of a layout handles; one ending in ":" takes the rest of the command as its argument. */
export const runnerCommands = (layout) => [...Object.keys(OWN[layout] ?? {}), ...Object.keys(WINDOW)];

/** Whether the runner of a layout handles `cockpit.<c>`. */
export const handles = (layout, c) => Boolean(lookup(c, layout));

/**
 * Whether `cockpit.<c>` does something in a layout: its runner handles it, or a view of that layout does; one for a
 * session cockpit drives only while the chat follows one (`driven`).
 */
export function available(layout, c, { driven = false } = {}) {
  const e = CATALOGUE.find((x) => x.command === `cockpit.${c}`);
  if (e?.driven && !driven) return false;
  return handles(layout, c) || Boolean(e?.layouts?.includes(layout));
}

/** Puts `cockpit.app` back on a screen of the shown layout: one carried over from the other layout names none of its. */
export function settle() {
  const s = screens();
  const valid = (child) => s.list.some((x) => x.child === child);
  if (valid(shared("cockpit.app"))) return;
  const shown = shownScreen();
  setShared("cockpit.app", valid(shown) ? shown : s.first);
}

/**
 * Runs a window command from the runner of `layout`, only while that layout shows; true when it was one. `ctx.key` is
 * the key a menu, the palette or a click sent it with: the handler gets `ctx.person`, true when it names this command.
 */
export function runCommand(c, cx, layout, ctx = {}) {
  if (layoutName() !== layout) return false;
  const found = lookup(c, layout);
  if (!found) return false;
  found.fn(found.arg, cx, { ...ctx, person: take(c, ctx.key ?? null) });
  return true;
}
