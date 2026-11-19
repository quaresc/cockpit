// The browser itself: each agent session's tabs, the one session shown in the window, and what each command does to
// them. A browser belongs to the session it was opened for (its `owner`) and shows only with that session. A tab is a
// WebContentsView in its profile's session; only the shown browser's active tab is in the window.

import fs from "node:fs";
import path from "node:path";
import { app, WebContentsView, nativeTheme, shell as desktop } from "electron";
import * as paths from "./paths.js";
import { emptyBrowser, withTab, withoutTab, withActive, withTabFields, withTabMoved, tabShowing, typedUrl, allowedUrl, newTabId, saved, restored } from "./state.js";
import * as profiles from "./profiles.js";
import { sessionFor } from "./sessions.js";
import { Answers } from "./permissions.js";
import { readHistory, writeHistory, visited, suggest } from "./history.js";
import { viewportOf } from "./viewport.js";
import { toSleep, idleMsOf } from "./discard.js";
import { Window } from "./window.js";
import { Dock } from "./dock.js";

/** How many tabs a page may open by itself in one browser before the next is refused. */
const POPUP_BUDGET = 24;
const NEW_TAB = "cockpit://newtab";

export class Shell {
  constructor({ options = {}, token = "", log = () => {} } = {}) {
    this.options = { dock: true, profileScope: "shared", popups: "tab", downloads: "", idleMinutes: 60, ...options };
    this.token = token;
    this.log = log;
    this.browsers = new Map();
    this.live = new Map();
    this.current = null;
    this.registry = profiles.read(paths.profilesFile());
    this.history = readHistory(paths.historyFile());
    this.answers = new Answers(path.join(paths.dataDir(), "permissions.json"));
    this.listeners = new Set();
    this.downloads = [];
    this.servers = [];
    this.theme = null;
    this.slot = null;
    this.grab = null;
    this.notes = new Map();
    this.suggestions = [];
    this.find = null;
    this.saving = new Map();
    this.cdpPort = 0;
    this.window = new Window({ docked: this.options.dock, onUi: (m) => this.onUi(m), onKey: (event, input) => this.key(event, input, null), onClosed: () => this.emit("window", { open: false }) });
    this.dock = new Dock({
      hide: () => this.hideDocked(),
      place: (rect, host) => {
        this.window.place(rect, host);
        this.emit("docked", { shown: true, rect });
      },
      raise: (host) => this.window.raise(host),
    });
    this.sweeper = setInterval(() => this.sweep(), 60_000);
    this.sweeper.unref?.();
  }

  // -------------------------------------------------------------------------------------------------------------------
  // Events

  /** Tells every subscriber of an event; a listener is `(event, payload) => void`. */
  emit(event, payload) {
    for (const listener of this.listeners) listener(event, payload);
  }

  // -------------------------------------------------------------------------------------------------------------------
  // Browsers and their files

  /** A session's browser, read from disk the first time it is asked for; `root` and `repo` name the worktree it works in. */
  browser(owner, { root = "", repo = "" } = {}) {
    if (!owner) throw new Error("a browser belongs to a session: no owner was given");
    let wt = this.browsers.get(owner);
    if (!wt) {
      let json = null;
      try {
        json = JSON.parse(fs.readFileSync(paths.browserFile(owner), "utf8"));
      } catch {}
      wt = json ? restored(json, owner) : emptyBrowser(owner, root, repo || root);
      this.browsers.set(owner, wt);
    }
    if (root && (wt.root !== root || (repo && wt.repo !== repo))) this.put({ ...wt, root, repo: repo || root });
    return this.browsers.get(owner);
  }

  put(wt) {
    this.browsers.set(wt.owner, wt);
    clearTimeout(this.saving.get(wt.owner));
    this.saving.set(wt.owner, setTimeout(() => this.save(wt.owner), 400));
    return wt;
  }

  save(owner) {
    clearTimeout(this.saving.get(owner));
    this.saving.delete(owner);
    const wt = this.browsers.get(owner);
    if (!wt) return;
    try {
      const file = paths.browserFile(owner);
      fs.mkdirSync(path.dirname(file), { recursive: true });
      const tmp = `${file}.${process.pid}.tmp`;
      fs.writeFileSync(tmp, JSON.stringify(saved(wt), null, 2));
      fs.renameSync(tmp, file);
    } catch (error) {
      this.log(`saving ${owner}: ${error.message}`);
    }
  }

  /** Closes a session's browser for good: its tabs, its notes, its file and what it downloaded for an agent. */
  forget(owner) {
    const wt = this.browsers.get(owner);
    for (const tab of wt?.tabs || []) this.destroyView(tab.id);
    this.browsers.delete(owner);
    this.notes.delete(owner);
    fs.rmSync(paths.browserFile(owner), { force: true });
    fs.rmSync(paths.downloadsDir(owner), { recursive: true, force: true });
    if (this.current === owner) this.show(null);
    this.emit("tabs", { owner, tabs: [] });
  }

  /** Every browser kept on disk, awake or not: its owner and the worktree it works in. */
  kept() {
    const found = new Map([...this.browsers.values()].map((wt) => [wt.owner, wt.root]));
    try {
      const dir = path.dirname(paths.browserFile("x"));
      for (const name of fs.readdirSync(dir).filter((n) => n.endsWith(".json"))) {
        const json = JSON.parse(fs.readFileSync(path.join(dir, name), "utf8"));
        if (typeof json?.owner === "string" && !found.has(json.owner)) found.set(json.owner, typeof json.root === "string" ? json.root : "");
      }
    } catch {}
    return found;
  }

  /** Forgets the browsers of every session that worked in a removed worktree, and the profile that worktree alone owned. */
  async forgetWorktree(root) {
    for (const [owner, at] of this.kept()) if (at === root) this.forget(owner);
    for (const profile of profiles.ownedBy(this.registry, root)) await this.deleteProfile(profile.id);
  }

  // -------------------------------------------------------------------------------------------------------------------
  // Profiles

  /** The profile a browser's tabs use: the one picked for it, else the one its worktree's scope gives it. */
  profileOf(wt) {
    const [registry, profile] = profiles.chosen(this.registry, wt, this.options.profileScope);
    if (registry !== this.registry) this.setRegistry(registry);
    return profile;
  }

  setRegistry(registry) {
    this.registry = registry;
    profiles.write(paths.profilesFile(), registry);
  }

  sessionOf(profileId) {
    return sessionFor(profileId, {
      answers: this.answers,
      window: () => this.window.win,
      downloadDir: (wc) => this.downloadDirFor(wc),
      onDownload: (item, info) => this.trackDownload(item, info),
    });
  }

  /** Gives a browser another profile: its tabs load again in that profile's session. */
  useProfile(owner, profileId) {
    const wt = this.browser(owner);
    if (profileId && !this.registry.profiles.some((p) => p.id === profileId)) throw new Error("no such profile");
    for (const tab of wt.tabs) this.destroyView(tab.id, { keep: true });
    this.put({ ...wt, profile: profileId || null, tabs: wt.tabs.map((t) => ({ ...t, asleep: true })) });
    if (this.current === owner) this.show(owner);
    return this.profileOf(this.browser(owner));
  }

  async deleteProfile(id) {
    const registry = profiles.removed(this.registry, id);
    for (const wt of [...this.browsers.values()]) if (this.profileOf(wt).id === id) this.useProfile(wt.owner, null);
    this.setRegistry(registry);
    this.answers.forget(id);
    const { clearProfile } = await import("./sessions.js");
    await clearProfile(id);
    this.push();
  }

  /**
   * Runs `work(session, sendCommand, onCommandTimeout)` with a hidden page of a profile's session and its debugger:
   * cookies are written through it, since only the debugging protocol keeps a cookie's partition.
   */
  async withCookieTarget(profileId, work) {
    const ses = this.sessionOf(profileId);
    const helper = new WebContentsView({ webPreferences: { session: ses, sandbox: true, contextIsolation: true, nodeIntegration: false } });
    const wc = helper.webContents;
    const end = () => {
      try {
        if (wc.debugger.isAttached()) wc.debugger.detach();
        wc.close({ waitForBeforeUnload: false });
      } catch {}
    };
    try {
      await wc.loadURL("data:text/html,<title>cookies</title>");
      wc.debugger.attach("1.3");
      return await work(ses, (method, params) => wc.debugger.sendCommand(method, params), end);
    } finally {
      end();
    }
  }

  // -------------------------------------------------------------------------------------------------------------------
  // Tabs

  tabOf(owner, id) {
    const wt = this.browser(owner);
    return wt.tabs.find((t) => t.id === (id || wt.active)) || null;
  }

  rootOf(tabId) {
    for (const wt of this.browsers.values()) if (wt.tabs.some((t) => t.id === tabId)) return wt.owner;
    return null;
  }

  /** Opens a page in a session's browser: the tab that shows it already comes forward, else a new tab. */
  open({ owner, worktree = "", repo = "", url, reload = false, background = false, fresh = false, show = true }) {
    const wt = this.browser(owner, { root: worktree, repo });
    const target = url ? typedUrl(url) : NEW_TAB;
    if (target !== NEW_TAB && !allowedUrl(target)) throw new Error(`cockpit's browser does not open ${target.split(":")[0]}: pages`);
    const found = target !== NEW_TAB && !fresh ? tabShowing(wt, target) : null;
    let id = found?.id;
    if (found) {
      this.put(background ? wt : withActive(wt, found.id));
      if (reload) this.live.get(found.id)?.view.webContents.reload();
    } else {
      id = newTabId();
      this.put(withTab(wt, { id, url: target, title: "", asleep: true }, { background }));
    }
    // Docked, the slot says which session's browser is shown; a window of its own shows the one a page was just opened for.
    if (this.current === owner || (show && (!this.current || !this.slot))) this.show(owner);
    this.emitTabs(owner);
    return { tab: id, owner, reused: !!found };
  }

  /** Makes a tab's page (waking it when it sleeps) in its browser's profile; `adopt` takes a page a pop-up made. */
  ensureView(owner, id, adopt = null) {
    const had = this.live.get(id);
    if (had) return had;
    const wt = this.browser(owner);
    const tab = wt.tabs.find((t) => t.id === id);
    if (!tab) return null;
    const profile = this.profileOf(wt);
    const prefs = { session: this.sessionOf(profile.id), sandbox: true, contextIsolation: true, nodeIntegration: false, webSecurity: true, backgroundThrottling: true };
    const view = adopt ? new WebContentsView({ webContents: adopt }) : new WebContentsView({ webPreferences: prefs });
    const record = { view, owner, id, profile: profile.id, target: null, hiddenAt: Date.now(), loading: false, audible: false, devtools: null, edited: false, agentAt: 0, waking: !adopt && Boolean(tab.scroll) };
    this.live.set(id, record);
    this.wire(record);
    // The page's debugging target is asked before anything loads: a debugger attached while a load swaps the page's
    // process gets no answer, and holds the load.
    if (adopt) view.webContents.once("did-stop-loading", () => this.identify(record));
    else this.identify(record).finally(() => this.live.get(id) === record && this.load(record, tab.url));
    this.put(withTabFields(this.browser(owner), id, { asleep: false }));
    return record;
  }

  load(record, url) {
    const wc = record.view.webContents;
    if (url === NEW_TAB) {
      const root = this.browsers.get(record.owner)?.root || "";
      const servers = JSON.stringify(this.servers.filter((s) => s.worktree === root).map(({ port, command }) => ({ port, command })));
      wc.loadFile(path.join(path.dirname(new URL(import.meta.url).pathname), "..", "ui", "newtab.html"), { query: { worktree: root, servers } }).catch(() => {});
    } else {
      // A load cut short by the next one, or by its tab closing, is no news.
      wc.loadURL(url).catch((error) => this.live.get(record.id) === record && error.code !== "ERR_ABORTED" && this.log(`loading ${url}: ${error.message}`));
    }
  }

  /** Wires a tab's page to the shell: its address and title, what it may open, its keys and its menus. */
  wire(record) {
    const wc = record.view.webContents;
    const { owner, id } = record;
    const set = (fields) => {
      if (!this.browsers.get(owner)?.tabs.some((t) => t.id === id)) return;
      this.put(withTabFields(this.browsers.get(owner), id, fields));
      this.emitTabs(owner);
      this.push();
    };
    const address = () => (wc.getURL().startsWith("file://") && wc.getURL().includes("/ui/newtab.html") ? NEW_TAB : wc.getURL());
    wc.on("did-start-loading", () => ((record.loading = true), this.push()));
    wc.on("did-stop-loading", () => ((record.loading = false), this.push()));
    wc.on("did-navigate", () => {
      record.edited = false;
      if (this.grab?.tab === id) this.design?.stop();
      // The first page of a tab that wakes keeps the scroll it slept with; any later one starts at its top.
      const woke = record.waking;
      record.waking = false;
      set(woke ? { url: address(), title: wc.getTitle() } : { url: address(), title: wc.getTitle(), scroll: null });
      this.visit(record);
    });
    wc.on("did-navigate-in-page", (_e, _url, isMain) => isMain && set({ url: address(), title: wc.getTitle() }));
    wc.on("page-title-updated", (_e, title) => (set({ title }), this.visit(record)));
    wc.on("page-favicon-updated", (_e, icons) => set({ favicon: icons.find((i) => /^https?:|^data:/.test(i)) || null }));
    wc.on("did-finish-load", () => this.afterLoad(record));
    wc.on("did-fail-load", (_e, code, why, url, isMain) => isMain && code !== -3 && this.log(`${url}: ${why} (${code})`));
    wc.on("audio-state-changed", (event) => ((record.audible = !!event.audible), this.push()));
    wc.on("render-process-gone", (_e, details) => {
      this.log(`tab ${id}: renderer ${details.reason}`);
      if (details.reason !== "clean-exit") this.sleep(id);
    });
    wc.on("will-navigate", (event) => {
      if (!allowedUrl(event.url)) event.preventDefault();
    });
    wc.on("will-redirect", (event) => {
      if (!allowedUrl(event.url)) event.preventDefault();
    });
    wc.setWindowOpenHandler((details) => this.popup(record, details));
    wc.on("before-input-event", (event, input) => this.key(event, input, record));
    wc.on("context-menu", (_e, params) => this.menus?.page(record, params));
    wc.on("found-in-page", (_e, result) => {
      if (this.find && result.finalUpdate) ((this.find = { ...this.find, matches: result.matches, at: result.activeMatchOrdinal }), this.push());
    });
    wc.on("devtools-closed", () => this.closeDevtools(id));
    wc.on("focus", () => this.emit("focus", { owner, tab: id }));
    wc.on("destroyed", () => this.live.get(id)?.view === record.view && this.live.delete(id));
  }

  /** Asks a page for its debugging target, the id agents' tools are pointed at; a call that gets no answer is tried again. */
  async identify(record) {
    const wc = record.view.webContents;
    for (let attempt = 0; attempt < 3 && !record.target && !wc.isDestroyed(); attempt++) {
      try {
        wc.debugger.attach("1.3");
        const late = new Promise((_, reject) => setTimeout(() => reject(new Error("no answer")), 1500));
        const { targetInfo } = await Promise.race([wc.debugger.sendCommand("Target.getTargetInfo"), late]);
        record.target = targetInfo?.targetId || null;
      } catch (error) {
        if (attempt === 2) this.log(`tab ${record.id}: no debugging target (${error.message})`);
      } finally {
        try {
          wc.debugger.detach();
        } catch {}
      }
      if (!record.target) await new Promise((resolve) => setTimeout(resolve, 300));
    }
  }

  /** After a page loads: its scroll comes back, its viewport is applied, and edits are watched so it is not put to sleep. */
  afterLoad(record) {
    const wc = record.view.webContents;
    const tab = this.browsers.get(record.owner)?.tabs.find((t) => t.id === record.id);
    if (!tab) return;
    if (tab.scroll && (tab.scroll.x || tab.scroll.y)) {
      wc.executeJavaScript(`scrollTo(${Number(tab.scroll.x) || 0}, ${Number(tab.scroll.y) || 0})`, true).catch(() => {});
    }
    this.applyViewport(record, tab.viewport);
    this.design?.mark(record);
    wc.executeJavaScriptInIsolatedWorld(991, [{ code: "addEventListener('input', () => { window.__cockpitEdited = true }, { capture: true, passive: true })" }]).catch(() => {});
  }

  visit(record) {
    const wc = record.view.webContents;
    const before = this.history;
    this.history = visited(this.history, { url: wc.getURL(), title: wc.getTitle(), profile: record.profile });
    if (this.history !== before) {
      clearTimeout(this.historyTimer);
      this.historyTimer = setTimeout(() => writeHistory(paths.historyFile(), this.history), 2000);
    }
  }

  /** What a page's own `window.open` or link in a new tab becomes: a tab of the same browser, or nothing. */
  popup(record, details) {
    if (this.options.popups === "block" || !allowedUrl(details.url || "about:blank")) return { action: "deny" };
    const wt = this.browser(record.owner);
    if (wt.tabs.length >= POPUP_BUDGET) return { action: "deny" };
    const background = details.disposition === "background-tab";
    return {
      action: "allow",
      // The new page keeps its opener, so a sign-in window can still answer the page that opened it.
      createWindow: (options) => {
        const id = newTabId();
        this.put(withTab(this.browser(record.owner), { id, url: details.url, title: "" }, { background, after: record.id }));
        const made = this.ensureView(record.owner, id, options.webContents);
        if (this.current === record.owner) this.show(record.owner);
        this.emitTabs(record.owner);
        return made.view.webContents;
      },
    };
  }

  destroyView(id, { keep = false } = {}) {
    const record = this.live.get(id);
    if (!record) return;
    this.live.delete(id);
    if (this.window.page === record.view) this.window.setPage(null);
    if (record.devtools) this.closeDevtools(id, record);
    if (!keep && this.grab?.tab === id) this.grab = null;
    try {
      record.view.webContents.close({ waitForBeforeUnload: false });
    } catch {}
  }

  /** Reads a page's scroll, so the tab opens there again. */
  async scrollOf(record) {
    try {
      return await record.view.webContents.executeJavaScript("({ x: scrollX, y: scrollY })", true);
    } catch {
      return null;
    }
  }

  /** Puts a tab to sleep: its page is closed, its address, title and scroll are kept. */
  async sleep(id) {
    const record = this.live.get(id);
    if (!record) return;
    const scroll = await this.scrollOf(record);
    const wt = this.browsers.get(record.owner);
    if (wt?.tabs.some((t) => t.id === id)) this.put(withTabFields(wt, id, { asleep: true, scroll }));
    this.destroyView(id, { keep: true });
    this.push();
  }

  /** Forgets the browsers whose worktree is gone: it was removed while the shell was away. */
  async prune() {
    const gone = new Set([...this.kept().values()].filter((root) => root && !fs.existsSync(root)));
    for (const root of gone) await this.forgetWorktree(root);
  }

  /** Puts to sleep the tabs that stayed out of view past the delay, unless something keeps them awake. */
  async sweep(now = Date.now()) {
    await this.prune();
    const tabs = [];
    for (const record of this.live.values()) {
      const edited = await record.view.webContents.executeJavaScriptInIsolatedWorld(991, [{ code: "!!window.__cockpitEdited" }]).catch(() => false);
      tabs.push({ id: record.id, shown: this.window.page === record.view, hiddenAt: record.hiddenAt, audible: record.audible, devtools: !!record.devtools, edited, loading: record.loading, agentAt: record.agentAt });
    }
    for (const id of toSleep(tabs, now, idleMsOf(this.options.idleMinutes))) await this.sleep(id);
  }

  /** Shows a session's browser in the window: its active tab's page, its row of tabs, its profile; null shows none. */
  show(owner) {
    const before = this.window.page;
    this.current = owner;
    const wt = owner ? this.browser(owner) : null;
    const tab = wt?.tabs.find((t) => t.id === wt.active) || null;
    const record = tab ? this.ensureView(owner, tab.id) : null;
    for (const other of this.live.values()) {
      if (other.view === before && other !== record) {
        other.hiddenAt = Date.now();
        this.scrollOf(other).then((scroll) => {
          const theirs = this.browsers.get(other.owner);
          if (scroll && theirs?.tabs.some((t) => t.id === other.id)) this.put(withTabFields(theirs, other.id, { scroll }));
        });
      }
    }
    this.window.setPage(record?.view || null, record ? viewportOf(tab.viewport) : null);
    this.window.setDevtools(record?.devtools || null, this.devtoolsSide || "bottom");
    this.find = null;
    this.push();
    this.emit("current", { owner });
    if (!this.options.dock && owner && !this.window.win.isVisible()) this.window.show();
  }

  activate(owner, id) {
    const wt = this.browser(owner);
    if (!wt.tabs.some((t) => t.id === id)) throw new Error("no such tab");
    this.put(withActive(wt, id));
    if (this.current === owner) this.show(owner);
    this.emitTabs(owner);
  }

  close(owner, id) {
    const wt = this.browser(owner);
    const tab = wt.tabs.find((t) => t.id === (id || wt.active));
    if (!tab) throw new Error("no such tab");
    this.destroyView(tab.id);
    this.put(withoutTab(wt, tab.id));
    if (this.current === owner) this.show(owner);
    this.emitTabs(owner);
  }

  reopen(owner) {
    const wt = this.browser(owner);
    const [last, ...rest] = wt.closed;
    if (!last) return null;
    this.put({ ...wt, closed: rest });
    return this.open({ owner, url: last.url });
  }

  /** Loads another address in a tab. */
  navigate(owner, id, text) {
    const tab = this.tabOf(owner, id);
    if (!tab) throw new Error("no such tab");
    const url = typedUrl(text);
    if (!allowedUrl(url)) throw new Error(`cockpit's browser does not open ${url.split(":")[0]}: pages`);
    const record = this.ensureView(owner, tab.id);
    this.put(withTabFields(this.browser(owner), tab.id, { url, scroll: null }));
    this.load(record, url);
    if (this.current === owner) this.window.focusPage();
  }

  nav(owner, id, what) {
    const tab = this.tabOf(owner, id);
    const wc = tab && this.ensureView(owner, tab.id)?.view.webContents;
    if (!wc) throw new Error("no such tab");
    if (what === "back") wc.navigationHistory.goBack();
    else if (what === "forward") wc.navigationHistory.goForward();
    else if (what === "stop") wc.stop();
    else if (what === "hard") wc.reloadIgnoringCache();
    else wc.reload();
  }

  setViewport(owner, id, asked) {
    const tab = this.tabOf(owner, id);
    if (!tab) throw new Error("no such tab");
    const viewport = viewportOf(asked);
    if (asked && !viewport) throw new Error("a viewport is a preset's name or a width and height from 200 to 7680");
    this.put(withTabFields(this.browser(owner), tab.id, { viewport }));
    const record = this.live.get(tab.id);
    if (record) this.applyViewport(record, viewport);
    if (record && this.window.page === record.view) ((this.window.viewport = viewport), this.window.layout());
    this.push();
    return viewport;
  }

  /** Shows a page at a viewport: its size and scale, and a mobile screen for a mobile one. */
  applyViewport(record, viewport) {
    const wc = record.view.webContents;
    const v = viewportOf(viewport);
    if (!v) return wc.disableDeviceEmulation();
    const scale = this.window.page === record.view ? this.window.pageScale || 1 : 1;
    wc.enableDeviceEmulation({ screenPosition: v.mobile ? "mobile" : "desktop", screenSize: { width: v.width, height: v.height }, viewPosition: { x: 0, y: 0 }, deviceScaleFactor: v.scale, viewSize: { width: v.width, height: v.height }, scale });
  }

  // -------------------------------------------------------------------------------------------------------------------
  // DevTools

  /** Opens DevTools beside the page, in the window; a second call closes it. */
  toggleDevtools(owner, id, open) {
    const tab = this.tabOf(owner, id);
    const record = tab && this.ensureView(owner, tab.id);
    if (!record) throw new Error("no such tab");
    const want = open ?? !record.devtools;
    if (!want) return this.closeDevtools(record.id);
    if (record.devtools) return;
    // A fresh view each time: DevTools only moves into a page that never loaded anything.
    const tools = new WebContentsView();
    record.devtools = tools;
    record.view.webContents.setDevToolsWebContents(tools.webContents);
    record.view.webContents.openDevTools({ mode: "detach", activate: false });
    if (this.window.page === record.view) this.window.setDevtools(tools, this.devtoolsSide || "bottom");
    this.push();
  }

  closeDevtools(id, known = null) {
    const record = known || this.live.get(id);
    const tools = record?.devtools;
    if (!tools) return;
    record.devtools = null;
    if (this.window.devtools === tools) this.window.setDevtools(null);
    try {
      if (!record.view.webContents.isDestroyed() && record.view.webContents.isDevToolsOpened()) record.view.webContents.closeDevTools();
      tools.webContents.close();
    } catch {}
    this.push();
  }

  // -------------------------------------------------------------------------------------------------------------------
  // Downloads

  downloadDirFor(wc) {
    const record = [...this.live.values()].find((r) => r.view.webContents === wc);
    // What an agent downloads stays out of the person's folder.
    if (record && Date.now() - record.agentAt < 30_000) return paths.downloadsDir(record.owner);
    const set = this.options.downloads;
    if (set) return set.startsWith("~") ? path.join(process.env.HOME || "", set.slice(1)) : set;
    return app.getPath("downloads");
  }

  trackDownload(item, { file }) {
    const entry = { id: newTabId(), file, name: path.basename(file), state: "progressing", received: 0, total: item.getTotalBytes(), item };
    this.downloads = [entry, ...this.downloads].slice(0, 20);
    const tell = () => (this.emit("downloads", { downloads: this.downloadList() }), this.push());
    item.on("updated", (_e, state) => ((entry.state = state), (entry.received = item.getReceivedBytes()), tell()));
    item.once("done", (_e, state) => ((entry.state = state), (entry.item = null), tell()));
    tell();
  }

  downloadList() {
    return this.downloads.map(({ id, file, name, state, received, total }) => ({ id, file, name, state, received, total }));
  }

  // -------------------------------------------------------------------------------------------------------------------
  // The slot, and agents

  /** The slot the mod reports for the Herdr window, and the session in front: its browser shows there, when it has one. */
  setSlot(slot) {
    // A report older than the one in hand, from the same copy of the mod, is a late arrival.
    if (slot?.seq && this.slot?.since === slot.since && slot.seq < this.slot.seq) return;
    this.slot = slot && slot.rect ? slot : null;
    if (slot?.theme) this.theme = slot.theme;
    if (Array.isArray(slot?.servers)) this.servers = slot.servers;
    if (slot?.frames && slot.frames !== this.frames) {
      this.frames = slot.frames;
      // The frames of the run before are of no use to this one.
      let old = [];
      try {
        old = fs.readdirSync(this.frames).filter((name) => /^(page|toolbar)-\d+\.jpg$/.test(name));
      } catch {}
      for (const name of old) fs.rmSync(path.join(this.frames, name), { force: true });
    }
    this.applySlot();
  }

  /** Whether a session has a browser, awake or on disk. */
  known(owner) {
    return this.browsers.has(owner) || fs.existsSync(paths.browserFile(owner));
  }

  /** The session in front decides whose browser shows; one that never opened a page has none, and the window hides. */
  applySlot() {
    const slot = this.slot;
    if (!slot) return this.options.dock ? this.dock.setSlot(null) : undefined;
    const owner = slot.owner && this.known(slot.owner) ? slot.owner : null;
    const has = Boolean(owner && this.browser(owner, { root: slot.worktree, repo: slot.repo }).tabs.length);
    if ((has ? owner : null) !== this.current) this.show(has ? owner : null);
    else this.push();
    if (this.options.dock) this.dock.setSlot({ ...slot, visible: Boolean(slot.visible) && has });
  }

  /** Hides the docked window, leaving its last frame for the mod to draw in its place. */
  hideDocked() {
    this.snapshot().finally(() => {
      this.window.hide();
      this.emit("docked", { shown: false, frame: this.frame || null });
    });
  }

  /** Writes the window's last frame where the mod can draw it, and forgets the one before. */
  async snapshot() {
    const view = this.window.page;
    if (!this.frames || !view || !this.window.win.isVisible()) return;
    try {
      const [toolbar, page] = await Promise.all([this.window.toolbar.webContents.capturePage(), view.webContents.capturePage()]);
      fs.mkdirSync(this.frames, { recursive: true });
      const n = (this.frameSeq = (this.frameSeq || 0) + 1);
      const files = { toolbar: path.join(this.frames, `toolbar-${n}.jpg`), page: path.join(this.frames, `page-${n}.jpg`) };
      fs.writeFileSync(files.toolbar, toolbar.toJPEG(85));
      fs.writeFileSync(files.page, page.toJPEG(85));
      for (const old of Object.values(this.frame || {})) fs.rmSync(old, { force: true });
      this.frame = files;
    } catch (error) {
      this.log(`frame: ${error.message}`);
    }
  }

  /** Where an agent's tool reaches a tab of its session's browser: the debugging port and that page's target. */
  async agentEndpoint({ owner, tab }) {
    const found = this.tabOf(owner, tab);
    if (!found) throw new Error("this session's browser has no tab; open a page first");
    const record = this.ensureView(owner, found.id);
    for (let i = 0; i < 40 && !record.target; i++) await new Promise((r) => setTimeout(r, 50));
    if (!record.target || !this.cdpPort) throw new Error("the page has no debugging target yet");
    record.agentAt = Date.now();
    return { port: this.cdpPort, target: record.target, cdp: `ws://127.0.0.1:${this.cdpPort}/devtools/page/${record.target}`, session: `ck-${paths.keyOf(owner)}`, tab: found.id, url: record.view.webContents.getURL() };
  }

  touch(owner, tab) {
    const record = this.live.get(this.tabOf(owner, tab)?.id);
    if (record) record.agentAt = Date.now();
  }

  // -------------------------------------------------------------------------------------------------------------------
  // What the toolbar draws

  tabsOf(owner) {
    const wt = this.browsers.get(owner);
    if (!wt) return [];
    return wt.tabs.map((t) => {
      const live = this.live.get(t.id);
      return { id: t.id, url: t.url === NEW_TAB ? "" : t.url, title: t.title || "", active: t.id === wt.active, asleep: !live, loading: !!live?.loading, audible: !!live?.audible, favicon: t.favicon || null };
    });
  }

  /** Tells what a browser's tabs are now; the slot is looked at again, since a first tab shows the window and a last one hides it. */
  emitTabs(owner) {
    this.emit("tabs", { owner, tabs: this.tabsOf(owner) });
    if (this.slot?.owner === owner) this.applySlot();
  }

  uiState() {
    const wt = this.current ? this.browsers.get(this.current) : null;
    const tab = wt?.tabs.find((t) => t.id === wt.active) || null;
    const record = tab ? this.live.get(tab.id) : null;
    const wc = record?.view.webContents;
    return {
      worktree: wt ? { root: wt.root, name: path.basename(wt.root) } : null,
      tabs: wt ? this.tabsOf(wt.owner) : [],
      nav: { back: !!wc?.navigationHistory.canGoBack(), forward: !!wc?.navigationHistory.canGoForward(), loading: !!record?.loading },
      profile: wt ? this.profileOf(wt) : null,
      viewport: tab?.viewport || null,
      devtools: !!record?.devtools,
      grab: this.grab && this.grab.tab === tab?.id ? { mode: this.grab.mode } : null,
      notes: wt && this.design ? this.design.summaries(wt.owner) : [],
      details: Boolean(this.design?.details),
      find: this.find,
      suggestions: this.suggestions,
      downloads: this.downloadList().slice(0, 5),
      theme: this.theme,
      dark: nativeTheme.shouldUseDarkColors,
      docked: this.options.dock,
    };
  }

  push() {
    clearTimeout(this.pushing);
    this.pushing = setTimeout(() => this.window.send(this.uiState()), 16);
  }

  // -------------------------------------------------------------------------------------------------------------------
  // What the toolbar and the keyboard ask

  onUi(message) {
    const owner = this.current;
    if (!message || typeof message.type !== "string") return;
    try {
      switch (message.type) {
        case "ready":
          return this.push();
        case "open":
          return this.window.setToolbarOpen(!!message.open);
        case "suggest":
          this.suggestions = suggest(this.history, message.text, { profile: owner ? this.profileOf(this.browser(owner)).id : null });
          return this.push();
        case "menu":
          return this.menus?.open(String(message.name), message);
        case "note":
          return this.design?.saveDraft(message.save ? message : {});
        case "notes":
          return owner && this.design?.recap(owner, message);
        case "rename":
          this.setRegistry(profiles.renamed(this.registry, String(message.id), message.name));
          return this.push();
        default:
          if (owner) return this.act(owner, message);
      }
    } catch (error) {
      this.log(`toolbar ${message.type}: ${error.message}`);
    }
  }

  /** One thing asked of the shown browser, by a click in the toolbar or a key. */
  act(owner, message) {
    const wt = this.browser(owner);
    switch (message.type) {
      case "navigate":
        this.suggestions = [];
        if (!wt.active) return this.open({ owner, url: message.text });
        return this.navigate(owner, wt.active, message.text);
      case "newTab":
        this.open({ owner });
        return this.window.focusToolbar(), this.window.send({ ...this.uiState(), focusAddress: true });
      case "closeTab":
        return this.close(owner, message.id);
      case "activate":
        return this.activate(owner, String(message.id));
      case "moveTab":
        return this.put(withTabMoved(wt, String(message.id), Number(message.to))), this.push();
      case "reopen":
        return this.reopen(owner);
      case "back":
      case "forward":
      case "reload":
      case "hard":
      case "stop":
        return wt.active && this.nav(owner, wt.active, message.type);
      case "devtools":
        return wt.active && this.toggleDevtools(owner, wt.active);
      case "find":
        return this.findInPage(String(message.text || ""), message.forward !== false, !!message.next);
      case "findClose":
        return this.findInPage("");
      case "zoom":
        return this.zoom(message.by);
      case "focusPage":
        return this.window.focusPage();
      case "grab":
        return this.design?.toggle(owner, wt.active, message.mode);
      case "external":
        return this.external(owner);
    }
  }

  external(owner) {
    const tab = this.tabOf(owner);
    if (tab && /^(https?|file):/.test(tab.url)) desktop.openExternal(tab.url);
  }

  findInPage(text, forward = true, next = false) {
    const wc = this.window.page?.webContents;
    if (!wc) return;
    if (!text) {
      wc.stopFindInPage("clearSelection");
      this.find = null;
    } else {
      this.find = { text, matches: this.find?.matches || 0, at: this.find?.at || 0 };
      wc.findInPage(text, { forward, findNext: next });
    }
    this.push();
  }

  zoom(by) {
    const wc = this.window.page?.webContents;
    if (!wc) return;
    wc.setZoomLevel(by === 0 ? 0 : Math.max(-3, Math.min(4, wc.getZoomLevel() + (by > 0 ? 0.5 : -0.5))));
  }

  /** The browser's own keys, read before the page gets them. */
  key(event, input, record) {
    if (input.type !== "keyDown") return;
    const owner = record?.owner || this.current;
    if (!owner) return;
    const cmd = input.meta && !input.control;
    const key = input.key.toLowerCase();
    const run = (message) => (event.preventDefault(), this.act(owner, message));
    if (this.grab && this.design?.key(event, input)) return;
    if (!cmd) return;
    if (key === "l") return event.preventDefault(), this.window.focusToolbar(), this.window.send({ ...this.uiState(), focusAddress: true });
    if (key === "r") return run({ type: input.shift ? "hard" : "reload" });
    if (key === "t") return run({ type: input.shift ? "reopen" : "newTab" });
    if (key === "w") return run({ type: "closeTab" });
    if (key === "f") return event.preventDefault(), this.window.focusToolbar(), this.window.send({ ...this.uiState(), focusFind: true });
    if (key === "[") return run({ type: "back" });
    if (key === "]") return run({ type: "forward" });
    if (key === "i" && input.alt) return run({ type: "devtools" });
    if (key === "c" && input.shift) return run({ type: "grab", mode: "copy" });
    if (key === "a" && input.shift) return run({ type: "grab", mode: "annotate" });
    if (key === "=" || key === "+") return run({ type: "zoom", by: 1 });
    if (key === "-") return run({ type: "zoom", by: -1 });
    if (key === "0") return run({ type: "zoom", by: 0 });
    if (/^[1-9]$/.test(key)) {
      const tab = this.browser(owner).tabs[key === "9" ? this.browser(owner).tabs.length - 1 : Number(key) - 1];
      if (tab) return run({ type: "activate", id: tab.id });
    }
  }

  // -------------------------------------------------------------------------------------------------------------------
  // The end

  /** Saves every browser's tabs with their scroll, then lets the app quit. */
  async shutdown() {
    clearInterval(this.sweeper);
    this.dock.stop();
    for (const record of this.live.values()) {
      const scroll = await this.scrollOf(record);
      const wt = this.browsers.get(record.owner);
      if (scroll && wt) this.browsers.set(wt.owner, withTabFields(wt, record.id, { scroll }));
    }
    for (const owner of this.browsers.keys()) this.save(owner);
    writeHistory(paths.historyFile(), this.history);
    this.window.destroy();
  }
}
