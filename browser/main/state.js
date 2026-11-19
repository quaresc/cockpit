// One session's browser as plain data: who it belongs to, the worktree that session works in, its tabs in order, the
// active one, and the profile its tabs use.

/** A browser that has no tab yet: `owner` names the session it belongs to, `root` and `repo` the worktree it works in. */
export const emptyBrowser = (owner, root = "", repo = root) => ({ owner, root, repo, tabs: [], active: null, profile: null, closed: [] });

/** The URL as two pages are compared: a local path as a file URL, no trailing slash. */
export function pageKey(url) {
  const text = String(url || "");
  const full = text.startsWith("/") ? `file://${text}` : text;
  return full.replace(/\/+$/, "");
}

/** The address a person typed, as the URL to load: a URL, a local path, a host, else a search. */
export function typedUrl(text, search = "https://duckduckgo.com/?q=%s") {
  const typed = String(text || "").trim();
  if (!typed) return "";
  if (typed.startsWith("?")) return search.replace("%s", encodeURIComponent(typed.slice(1).trim()));
  if (/^(https?|file|about|data|view-source):/i.test(typed)) return typed;
  // Another scheme is kept as typed, for the shell to refuse: it is never searched for.
  if (/^[a-z][a-z0-9+.-]*:(?!\d)\S*$/i.test(typed)) return typed;
  if (typed.startsWith("/")) return `file://${typed}`;
  if (/^(localhost|\d{1,3}(\.\d{1,3}){3}|\[[0-9a-f:]+\])(:\d+)?(\/.*)?$/i.test(typed)) return `http://${typed}`;
  if (!/\s/.test(typed) && /^[^/]+\.[a-z]{2,}(:\d+)?(\/.*)?$/i.test(typed)) return `https://${typed}`;
  return search.replace("%s", encodeURIComponent(typed));
}

/** Whether a page may load at all: the web, local files, and blank. */
export const allowedUrl = (url) => /^(https?:|file:|about:blank$|data:|view-source:)/i.test(String(url || ""));

let serial = 0;
/** A tab id that no other tab of this run has. */
export const newTabId = () => `t${Date.now().toString(36)}${(serial++).toString(36)}`;

/** The browser with one more tab, active unless opened in the background. */
export function withTab(wt, tab, { background = false, after = null } = {}) {
  const at = after ? wt.tabs.findIndex((t) => t.id === after) + 1 : wt.tabs.length;
  const tabs = [...wt.tabs.slice(0, at), tab, ...wt.tabs.slice(at)];
  return { ...wt, tabs, active: background && wt.active ? wt.active : tab.id };
}

/** The browser without a tab: its right neighbour becomes active, else its left one. */
export function withoutTab(wt, id) {
  const at = wt.tabs.findIndex((t) => t.id === id);
  if (at < 0) return wt;
  const tabs = wt.tabs.filter((t) => t.id !== id);
  const next = wt.active === id ? (tabs[at] || tabs[at - 1] || null)?.id ?? null : wt.active;
  const gone = wt.tabs[at];
  const closed = gone.url ? [{ url: gone.url, title: gone.title || "" }, ...wt.closed].slice(0, 10) : wt.closed;
  return { ...wt, tabs, active: next, closed };
}

/** The browser with another tab active. */
export const withActive = (wt, id) => (wt.tabs.some((t) => t.id === id) ? { ...wt, active: id } : wt);

/** The browser with one tab's fields changed. */
export const withTabFields = (wt, id, fields) => ({ ...wt, tabs: wt.tabs.map((t) => (t.id === id ? { ...t, ...fields } : t)) });

/** The tab that shows this page already. */
export const tabShowing = (wt, url) => wt.tabs.find((t) => pageKey(t.url) === pageKey(url)) || null;

/** The browser with a tab moved to another place in the row. */
export function withTabMoved(wt, id, to) {
  const from = wt.tabs.findIndex((t) => t.id === id);
  if (from < 0) return wt;
  const tabs = wt.tabs.filter((t) => t.id !== id);
  tabs.splice(Math.max(0, Math.min(to, tabs.length)), 0, wt.tabs[from]);
  return { ...wt, tabs };
}

/** What is written to disk for a browser: enough to open its tabs again where they were. */
export function saved(wt) {
  const tabs = wt.tabs.map(({ id, url, title, scroll, viewport, favicon }) => ({ id, url, title: title || "", scroll: scroll || null, viewport: viewport || null, favicon: favicon || null }));
  return { owner: wt.owner, root: wt.root, repo: wt.repo, active: wt.active, profile: wt.profile, tabs, closed: wt.closed };
}

/** A browser read back from disk: every tab asleep until it is shown. */
export function restored(json, owner) {
  const root = typeof json?.root === "string" ? json.root : "";
  const wt = emptyBrowser(owner, root, typeof json?.repo === "string" ? json.repo : root);
  const tabs = Array.isArray(json?.tabs) ? json.tabs.filter((t) => t && typeof t.url === "string" && allowedUrl(t.url)) : [];
  wt.tabs = tabs.map((t) => ({ id: typeof t.id === "string" ? t.id : newTabId(), url: t.url, title: String(t.title || ""), scroll: t.scroll || null, viewport: t.viewport || null, favicon: t.favicon || null, asleep: true }));
  wt.active = wt.tabs.some((t) => t.id === json?.active) ? json.active : wt.tabs[0]?.id ?? null;
  wt.profile = typeof json?.profile === "string" ? json.profile : null;
  wt.closed = Array.isArray(json?.closed) ? json.closed.filter((c) => c && typeof c.url === "string").slice(0, 10) : [];
  return wt;
}
