// Which copy of the mod is in front, so a key pressed from outside the window (`cockpit.key:<k>`) reaches that view and
// no other, and a hidden view's keys stay with it. Every copy of the window shares the list, on globalThis.

import { shared, layout } from "herdr";
import { screens } from "./screens.js";

/** The apps that are screens of a layout's stack: one of them is in front at a time. */
const SCREENS = new Set(["home", "inbox", "runbook", "todo", "meetings", "monitor", "routines", "mail", "chat", "needs"]);

const copies = () => (globalThis.cockpitCopies ??= new Set());

/** How long a copy counts as alive after its last beat: a switched layout or a reloaded mod drops copies unannounced. */
const ALIVE_MS = 3000;

/** A copy as `front` weighs it: {app, overlay, scoped, keys, at, beat}; its owner refreshes `beat` every second. */
export function enter(copy) {
  copies().add(copy);
}

/** The copies still alive, the dead ones forgotten. */
function alive(now = Date.now()) {
  const out = [];
  for (const c of copies()) {
    if (now - (c.beat ?? c.at ?? 0) < ALIVE_MS) out.push(c);
    else if (now - (c.beat ?? c.at ?? 0) > 10 * ALIVE_MS) copies().delete(c);
  }
  return out;
}

export function leave(copy) {
  copies().delete(copy);
}

/** The layout node with this id, anywhere in the tree. */
function node(tree, id) {
  if (!tree || typeof tree !== "object") return null;
  if (tree.id === id) return tree;
  for (const child of tree.children ?? []) {
    const found = node(child, id);
    if (found) return found;
  }
  return null;
}

/** The child a stack shows: its `active`, else its first. */
const active = (stack) => stack?.active ?? stack?.children?.[0]?.id ?? null;

/**
 * The screen the window shows, read off the shown layout (its stack's active child; in the reading layout, the agent
 * screen's own stack), so a layout switch or a click is never missed. Without a layout to read: the last `show()`.
 */
export function shownScreen() {
  const s = screens();
  const tree = layout();
  let shown = active(node(tree, s.stack));
  if (shown && s.inner && shown === s.inner.screen) shown = active(node(tree, s.inner.stack)) ?? s.inner.children[0];
  return shown ?? shared("cockpit.app") ?? s.first;
}

/** Whether the screen `app` is the one shown, so its keys are the person's. */
export function inFront(app) {
  return shownScreen() === app;
}

/**
 * The copies a key from outside reaches: the newest popup that takes keys, else the screen in front, else (when the
 * screen in front is an agent's terminal) the panes and the panel beside it, none of which answers an agent.
 */
export function front(list = alive(), shown = shownScreen()) {
  const keyed = list.filter((c) => c.keys);
  const popup = keyed.filter((c) => c.overlay).sort((a, b) => b.at - a.at)[0];
  if (popup) return [popup];
  const screen = keyed.find((c) => SCREENS.has(c.app) && !c.scoped && c.app === shown);
  if (screen) return [screen];
  if (SCREENS.has(shown)) return [];
  return keyed.filter((c) => !c.overlay && (!SCREENS.has(c.app) || c.scoped));
}
