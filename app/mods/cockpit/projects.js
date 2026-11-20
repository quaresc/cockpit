// Where the side panel's Files and Changes look: the folder of the agent the chat reads and the git repositories it
// works in, the latest first, so an agent started at a root of several projects shows the one it changed.

import { exec, focused, panes, readTextFile, shared, setShared, spaces } from "herdr";
import * as hub from "./hub.js";

/** The shared key the chat writes its subject's place to: {cwd, repos: [{path, name}]}, or null for none. */
export const WHERE = "cockpit.panel.where";

/** Git reads that write nothing under .git (no index refresh or lock, no fsmonitor cookie). */
export const READ = ["--no-optional-locks", "-c", "core.fsmonitor=false", "-c", "diff.autoRefreshIndex=false"];

/** `chat.projectPanels` as last read, and when: cockpit's Files and Changes stand in for Herdr's unless it is false. */
let panels = { on: true, at: 0 };

/** Whether cockpit's Files and Changes show: read from the config again at most every 10 s, true until it says no. */
export function projectPanels() {
  if (Date.now() - panels.at > 10_000) {
    panels.at = Date.now();
    readPanels().catch(() => {});
  }
  return panels.on;
}

async function readPanels() {
  const { home, cache } = await (async () => hub.install())().catch(() => ({}));
  for (const path of [cache && `${cache}/config.json`, home && `${home}/.config/cockpit/config.json`].filter(Boolean)) {
    try {
      const config = JSON.parse((await readTextFile(path)) ?? "null");
      if (config) return void (panels = { on: config.chat?.projectPanels !== false, at: panels.at });
    } catch {}
  }
}

/** Tells the panels where the chat's agent works; null hands them back to the focused pane. */
export function showWhere(where) {
  const now = shared(WHERE);
  const next = where?.cwd ? { cwd: where.cwd, repos: (where.repos ?? []).filter((r) => r?.path) } : null;
  if (JSON.stringify(now ?? null) !== JSON.stringify(next)) setShared(WHERE, next);
}

/** The folder and repositories the panels show: the chat's, else the focused pane's folder and no repositories. */
export function where() {
  const w = shared(WHERE);
  if (w?.cwd) return { cwd: w.cwd, repos: w.repos ?? [] };
  const space = spaces().find((s) => s.active);
  const f = focused();
  const pane = f?.pane ? panes().find((p) => p.pane === f.pane) : null;
  return { cwd: pane?.cwd || f?.cwd || space?.folder || null, repos: [] };
}

/** A place as one string, to tell when it moved. */
export const whereKey = (w) => JSON.stringify([w.cwd, (w.repos ?? []).map((r) => r.path)]);

/** The repositories to show changes of: the ones the agent works in, else the one its folder is in; [] outside git. */
export async function reposOf(w) {
  if (w.repos?.length) return w.repos;
  if (!w.cwd) return [];
  const top = await exec("git", ["rev-parse", "--show-toplevel"], { cwd: w.cwd }).catch(() => null);
  const path = top?.code === 0 ? top.stdout.trim() : null;
  return path ? [{ path, name: path.split("/").pop() }] : [];
}

/** A status entry's letter: U a conflict, D deleted, A added or new, else M. */
export function statusLetter(xy) {
  if (xy.includes("U") || xy === "AA" || xy === "DD") return "U";
  if (xy.includes("D")) return "D";
  if (xy === "??" || xy.includes("A")) return "A";
  return "M";
}

/** What changed in a repository, `[{path, status, fresh}]` from `git status`, capped at `max`; null when git fails. */
export async function statusOf(root, max = 1000) {
  const r = await exec("git", [...READ, "status", "--porcelain=v1", "-z", "--no-renames", "--untracked-files=all"], { cwd: root }).catch(() => null);
  if (!r || r.code !== 0) return null;
  const entries = r.stdout.split("\0").filter(Boolean);
  const files = entries.slice(0, max).map((e) => ({ path: e.slice(3), status: statusLetter(e.slice(0, 2)), fresh: e.slice(0, 2) === "??" }));
  return { files, limited: entries.length > max };
}
