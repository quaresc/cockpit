// What the person picked in this window, told apart from a window command any local process can send (`herdr-app do`):
// a menu, the palette or a click sends `<command>~<key>` with a key minted here, and a command without one only navigates.

import { exec } from "herdr";

/** A received command's key, at its end: `~` and 32 hex digits. */
const KEYED = /~([0-9a-f]{32})$/;

/** How long a minted key stays good: a menu or the palette left open a while still works. */
const KEY_FOR_MS = 10 * 60_000;

/** The keys minted and not used yet, shared by every copy of the window: key -> {commands, at}. */
const minted = () => (globalThis.cockpitPersonKeys ??= new Map());

/** A key nobody outside this window can guess: the system's random source where the runtime has one. */
export function oneTimeKey() {
  const bytes = new Uint8Array(16);
  if (globalThis.crypto?.getRandomValues) globalThis.crypto.getRandomValues(bytes);
  else for (let i = 0; i < bytes.length; i++) bytes[i] = Math.floor(Math.random() * 256);
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

const bare = (command) => (command.startsWith("cockpit.") ? command.slice(8) : command);

/** A new key, good once for any of these commands (one opening of a menu); old keys are forgotten. */
function mint(commands) {
  const keys = minted();
  const now = Date.now();
  for (const [key, found] of keys) if (now - found.at >= KEY_FOR_MS) keys.delete(key);
  while (keys.size >= 64) keys.delete(keys.keys().next().value);
  const key = oneTimeKey();
  keys.set(key, { commands: new Set(commands.map(bare)), at: now });
  return key;
}

/** The command as the person's own, `cockpit.` kept when it has it: sent from a click, a real key or a menu pick. */
export function keyed(command) {
  return `${command}~${mint([command])}`;
}

/** Each item of a menu with its command keyed, one key for the whole opening of the menu. */
export function keyedItems(items) {
  const commands = items.filter((i) => i && typeof i === "object" && i.command).map((i) => i.command);
  if (!commands.length) return items;
  const key = mint(commands);
  return items.map((i) => (i && typeof i === "object" && i.command ? { ...i, command: `${i.command}~${key}` } : i));
}

/** A received command split into `{command, key}`; `key` is null when it names none. */
export function unkey(raw) {
  const found = KEYED.exec(raw);
  return found ? { command: raw.slice(0, found.index), key: found[1] } : { command: raw, key: null };
}

/** Whether the person sent this command: its key was minted for it, is recent, and was not used. Used up either way. */
export function take(command, key) {
  if (!key) return false;
  const found = minted().get(key);
  if (!found) return false;
  minted().delete(key);
  return Date.now() - found.at < KEY_FOR_MS && found.commands.has(bare(command));
}

let testing = null;

/**
 * Whether this window takes keys and typing from a shell (`cockpit.key:`, `palette:type:`): only a sandbox's, whose
 * own process was started with COCKPIT_SANDBOX and COCKPIT_TEST_KEYS=1. Read once; no file anyone can write says so.
 */
export function testKeys() {
  testing ??= exec("/usr/bin/env", [])
    .then((r) => {
      const env = new Map(String(r?.stdout ?? "").split("\n").map((line) => [line.slice(0, line.indexOf("=")), line.slice(line.indexOf("=") + 1)]));
      return r?.code === 0 && Boolean(env.get("COCKPIT_SANDBOX")) && env.get("COCKPIT_TEST_KEYS") === "1";
    })
    .catch(() => false);
  return testing;
}

/** What a view says to a key that waits until the person moves to a row a command pointed at. */
export const MOVE_FIRST = "Move to it first (j/k), or click it";

/** The keys that move the selection: one makes the pointed row the person's. */
const MOVES = new Set(["j", "k", "up", "down", "pageup", "pagedown", "ctrl-d", "ctrl-u", "g", "home", "shift-g", "end", "[", "]"]);

/** The keys that only look, open or close something, whatever row is selected. */
const LOOKS = new Set(["escape", "tab", "shift-tab", "space", "/", "?", "shift-/", "ctrl-p", "cmd-shift-p", "shift-r", "shift-l", "shift-f10", "w", "q"]);

/**
 * Whether a key waits while `view.pointed` (a row a hub focus or a window command selected): true for one that would act
 * on that row; a move makes it the person's, and keys that only look go on unless the view acts with them (`acts`).
 */
export function holds(view, event, acts = []) {
  if (!view.pointed) return false;
  const key = String(event?.keystroke ?? "");
  if (MOVES.has(key)) return (view.pointed = false);
  return !LOOKS.has(key) || acts.includes(key);
}
