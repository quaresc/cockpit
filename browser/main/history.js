// The pages visited, for the address bar's suggestions: kept per profile, newest first, in one small JSON file.

import fs from "node:fs";
import path from "node:path";

const MAX = 4000;

export const emptyHistory = () => ({ entries: [] });

export function readHistory(file) {
  try {
    const json = JSON.parse(fs.readFileSync(file, "utf8"));
    const entries = Array.isArray(json?.entries) ? json.entries.filter((e) => e && typeof e.url === "string") : [];
    return { entries };
  } catch {
    return emptyHistory();
  }
}

export function writeHistory(file, history) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const tmp = `${file}.${process.pid}.tmp`;
  fs.writeFileSync(tmp, JSON.stringify(history));
  fs.renameSync(tmp, file);
}

/** The history with a visit noted: the page moves to the front and counts one more. */
export function visited(history, { url, title, profile }, now = Date.now()) {
  if (!/^https?:/i.test(url || "")) return history;
  const before = history.entries.find((e) => e.url === url && e.profile === profile);
  const entry = { url, title: title || before?.title || "", profile, at: now, count: (before?.count || 0) + 1 };
  return { entries: [entry, ...history.entries.filter((e) => e !== before)].slice(0, MAX) };
}

/** How well an entry answers what was typed: every word must be in its address or title; starts of hosts count most. */
function score(entry, words) {
  const url = entry.url.toLowerCase();
  const title = entry.title.toLowerCase();
  const host = url.replace(/^https?:\/\/(www\.)?/, "");
  let total = 0;
  for (const word of words) {
    if (host.startsWith(word)) total += 6;
    else if (url.includes(word)) total += 3;
    else if (title.includes(word)) total += 2;
    else return 0;
  }
  return total + Math.min(entry.count, 20) / 10;
}

/** The pages that answer what was typed, best first; a profile's own pages before the others'. */
export function suggest(history, typed, { profile = null, limit = 6 } = {}) {
  const words = String(typed || "").toLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length) return [];
  const seen = new Set();
  return history.entries
    .map((entry) => ({ entry, points: score(entry, words) + (entry.profile === profile ? 1 : 0) }))
    .filter((s) => s.points > 0)
    .sort((a, b) => b.points - a.points || b.entry.at - a.entry.at)
    .filter((s) => !seen.has(s.entry.url) && seen.add(s.entry.url))
    .slice(0, limit)
    .map(({ entry }) => ({ url: entry.url, title: entry.title }));
}

/** The history without a profile's pages. */
export const withoutProfile = (history, profile) => ({ entries: history.entries.filter((e) => e.profile !== profile) });
