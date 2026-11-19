// The browser's native menus and dialogs. What only a person may decide (which profile, deleting one, importing
// cookies, sending notes to an agent) is offered here and nowhere a page or a debugging tool can click.

import { Menu, dialog, clipboard, shell as desktop } from "electron";
import * as profiles from "./profiles.js";
import { PRESETS } from "./viewport.js";
import { clearCookies, clearProfile } from "./sessions.js";

export class Menus {
  constructor(shell) {
    this.shell = shell;
  }

  popup(template) {
    Menu.buildFromTemplate(template).popup({ window: this.shell.window.win });
  }

  /** Opens the menu a toolbar button asks for. */
  open(name, message = {}) {
    const owner = this.shell.current;
    if (name === "profile") return owner && this.profile(owner);
    if (name === "viewport") return owner && this.viewport(owner);
    if (name === "more") return this.more(owner);
    if (name === "tab") return owner && this.tab(owner, String(message.id));
  }

  /** The profiles, the one this browser uses ticked, and what can be done with them. */
  async profile(owner) {
    const { shell } = this;
    const wt = shell.browser(owner);
    const using = shell.profileOf(wt);
    const pick = (id) => () => shell.useProfile(owner, id);
    const items = shell.registry.profiles.map((p) => ({ label: p.owner ? `${p.name} (this ${p.owner.split(":")[0]}'s)` : p.name, type: "radio", checked: p.id === using.id, click: pick(p.id) }));
    this.popup([
      ...items,
      { type: "separator" },
      { label: "New profile", click: () => this.newProfile(owner) },
      { label: `Rename "${using.name}"…`, enabled: using.id !== profiles.DEFAULT, click: () => shell.window.send({ ...shell.uiState(), rename: { id: using.id, name: using.name } }) },
      { label: `Import cookies into "${using.name}"`, submenu: await this.importItems(using.id) },
      { type: "separator" },
      { label: `Clear "${using.name}" cookies…`, click: () => this.clear(using, "cookies") },
      { label: `Clear "${using.name}" browsing data…`, click: () => this.clear(using, "all") },
      { label: `Delete "${using.name}"…`, enabled: using.id !== profiles.DEFAULT, click: () => this.remove(using) },
    ]);
  }

  newProfile(owner) {
    const { shell } = this;
    for (let n = shell.registry.profiles.length; n < 1000; n++) {
      try {
        const [registry, profile] = profiles.created(shell.registry, `Profile ${n}`);
        shell.setRegistry(registry);
        shell.useProfile(owner, profile.id);
        return shell.window.send({ ...shell.uiState(), rename: { id: profile.id, name: profile.name } });
      } catch {}
    }
  }

  async clear(profile, what) {
    const { shell } = this;
    const all = what === "all";
    const { response } = await dialog.showMessageBox(shell.window.win, {
      type: "warning",
      message: all ? `Clear everything "${profile.name}" stored?` : `Clear "${profile.name}" cookies?`,
      detail: all ? "Cookies, local storage and cache go: every site in this profile is signed out." : "Every site in this profile is signed out.",
      buttons: ["Clear", "Cancel"],
      defaultId: 1,
      cancelId: 1,
    });
    if (response !== 0) return;
    await (all ? clearProfile(profile.id) : clearCookies(profile.id));
    for (const record of shell.live.values()) if (record.profile === profile.id) record.view.webContents.reload();
  }

  async remove(profile) {
    const { response } = await dialog.showMessageBox(this.shell.window.win, { type: "warning", message: `Delete the profile "${profile.name}"?`, detail: "Its cookies, storage and cache are deleted with it.", buttons: ["Delete", "Cancel"], defaultId: 1, cancelId: 1 });
    if (response === 0) await this.shell.deleteProfile(profile.id);
  }

  /** The sizes a page can be shown at. */
  viewport(owner) {
    const { shell } = this;
    const tab = shell.tabOf(owner);
    const now = tab?.viewport?.preset || (tab?.viewport ? "custom" : null);
    this.popup([
      { label: "Fit the panel", type: "radio", checked: !now, click: () => shell.setViewport(owner, null, null) },
      { type: "separator" },
      ...PRESETS.map((p) => ({ label: p.label, type: "radio", checked: now === p.id, click: () => shell.setViewport(owner, null, p.id) })),
    ]);
  }

  more(owner) {
    const { shell } = this;
    const tab = owner ? shell.tabOf(owner) : null;
    const act = (type, extra = {}) => () => owner && shell.act(owner, { type, ...extra });
    const downloads = shell.downloadList().map((d) => ({
      label: `${d.name}${d.state === "progressing" ? ` — ${Math.round((100 * d.received) / (d.total || 1))} %` : d.state === "completed" ? "" : ` — ${d.state}`}`,
      submenu: [
        { label: "Open", enabled: d.state === "completed", click: () => desktop.openPath(d.file) },
        { label: "Show in Finder", enabled: d.state === "completed", click: () => desktop.showItemInFolder(d.file) },
        { label: "Cancel", enabled: d.state === "progressing", click: () => shell.downloads.find((x) => x.id === d.id)?.item?.cancel() },
      ],
    }));
    this.popup([
      { label: "New tab", accelerator: "Cmd+T", enabled: !!owner, click: act("newTab") },
      { label: "Reopen closed tab", accelerator: "Cmd+Shift+T", enabled: !!owner && shell.browser(owner).closed.length > 0, click: act("reopen") },
      { type: "separator" },
      { label: "Find in page…", accelerator: "Cmd+F", enabled: !!tab, click: () => shell.window.send({ ...shell.uiState(), focusFind: true }) },
      { label: "Zoom in", accelerator: "Cmd+Plus", enabled: !!tab, click: act("zoom", { by: 1 }) },
      { label: "Zoom out", accelerator: "Cmd+-", enabled: !!tab, click: act("zoom", { by: -1 }) },
      { label: "Actual size", accelerator: "Cmd+0", enabled: !!tab, click: act("zoom", { by: 0 }) },
      { type: "separator" },
      { label: "Hard reload", accelerator: "Cmd+Shift+R", enabled: !!tab, click: act("hard") },
      { label: "Open in your default browser", enabled: !!tab, click: act("external") },
      { label: "Copy address", enabled: !!tab?.url, click: () => clipboard.writeText(tab.url) },
      { type: "separator" },
      { label: "DevTools", accelerator: "Cmd+Alt+I", enabled: !!tab, click: act("devtools") },
      {
        label: "DevTools at",
        submenu: ["bottom", "right"].map((side) => ({ label: side === "bottom" ? "The bottom" : "The right", type: "radio", checked: (shell.devtoolsSide || "bottom") === side, click: () => ((shell.devtoolsSide = side), shell.window.devtools && shell.window.setDevtools(shell.window.devtools, side)) })),
      },
      { type: "separator" },
      { label: "Downloads", enabled: downloads.length > 0, submenu: downloads.length ? downloads : [{ label: "None", enabled: false }] },
    ]);
  }

  /** A tab's own menu, from a right click on it. */
  tab(owner, id) {
    const { shell } = this;
    const wt = shell.browser(owner);
    const tab = wt.tabs.find((t) => t.id === id);
    if (!tab) return;
    this.popup([
      { label: "Reload", click: () => shell.nav(owner, id, "reload") },
      { label: "Duplicate", click: () => shell.open({ owner, url: tab.url, background: false }) && null },
      { label: "Put to sleep", enabled: shell.live.has(id) && wt.active !== id, click: () => shell.sleep(id) },
      { type: "separator" },
      { label: "Close", click: () => shell.close(owner, id) },
      { label: "Close the other tabs", enabled: wt.tabs.length > 1, click: () => wt.tabs.filter((t) => t.id !== id).forEach((t) => shell.close(owner, t.id)) },
    ]);
  }

  /** A page's menu, from a right click in it. */
  page(record, params) {
    const { shell } = this;
    const wc = record.view.webContents;
    const items = [];
    if (params.linkURL) {
      items.push(
        { label: "Open link in a new tab", click: () => shell.open({ owner: record.owner, url: params.linkURL, background: true }) },
        { label: "Open link in your default browser", click: () => desktop.openExternal(params.linkURL) },
        { label: "Copy link", click: () => clipboard.writeText(params.linkURL) },
        { type: "separator" },
      );
    }
    if (params.hasImageContents && params.srcURL) items.push({ label: "Copy image address", click: () => clipboard.writeText(params.srcURL) }, { type: "separator" });
    if (params.isEditable) items.push({ role: "cut" }, { role: "copy" }, { role: "paste" }, { role: "selectAll" }, { type: "separator" });
    else if (params.selectionText) items.push({ role: "copy" }, { type: "separator" });
    items.push(
      { label: "Back", enabled: wc.navigationHistory.canGoBack(), click: () => wc.navigationHistory.goBack() },
      { label: "Forward", enabled: wc.navigationHistory.canGoForward(), click: () => wc.navigationHistory.goForward() },
      { label: "Reload", click: () => wc.reload() },
      { type: "separator" },
      { label: "Pick an element for the agent", click: () => shell.design.pickAt(record) },
      { label: "Inspect", click: () => (shell.toggleDevtools(record.owner, record.id, true), wc.inspectElement(params.x, params.y)) },
    );
    this.popup(items);
  }

  /** The browsers on this Mac whose cookies can be imported, each with its profiles. */
  async sources() {
    const cookies = await import("./cookies/index.js");
    return cookies.sources({});
  }

  /** The Import submenu: every browser found with its profiles, and a cookie file. */
  async importItems(profileId) {
    const run = (from) => () => this.importCookies(profileId, from).catch((error) => this.failed(error));
    const found = await this.sources().catch(() => []);
    const browsers = found.map((source) =>
      source.profiles.length === 1
        ? { label: source.label, click: run({ source: source.id, profile: source.profiles[0].directory }) }
        : { label: source.label, submenu: source.profiles.map((p) => ({ label: p.name, click: run({ source: source.id, profile: p.directory }) })) },
    );
    const file = async () => {
      const picked = await dialog.showOpenDialog(this.shell.window.win, { title: "Import cookies from a file", properties: ["openFile"], filters: [{ name: "Cookies (JSON)", extensions: ["json"] }] });
      if (!picked.canceled && picked.filePaths[0]) await run({ file: picked.filePaths[0] })();
    };
    return [...(browsers.length ? browsers : [{ label: "No other browser found", enabled: false }]), { type: "separator" }, { label: "From a cookie file…", click: file }];
  }

  failed(error) {
    if (/did not allow/.test(error.message)) return;
    dialog.showMessageBox(this.shell.window.win, { type: "warning", message: "The cookies were not imported", detail: error.message, buttons: ["OK"] });
  }

  /**
   * Imports cookies into a profile. The person confirms before anything is read (the Keychain then asks for the source
   * browser's key) unless the hub vouches a person asked; the answer says how many went in.
   */
  async importCookies(profileId, from, { ask = true } = {}) {
    const { shell } = this;
    const cookies = await import("./cookies/index.js");
    const target = shell.registry.profiles.find((p) => p.id === profileId);
    if (!target) throw new Error("no such profile");
    const source = from.file ? null : (await this.sources()).find((s) => s.id === from.source);
    if (!from.file && !source) throw new Error(`no browser ${from.source} with cookies was found on this Mac`);
    if (ask) {
      const what = from.file ? `the file ${from.file}` : `${source.label}${from.profile ? ` (${from.profile})` : ""}`;
      const key = source?.needsKey ? ` macOS may ask for your password so that "security" can read "${source.keychain?.service}": that is this import.` : "";
      const { response } = await dialog.showMessageBox(shell.window.win, {
        type: "question",
        message: `Import cookies from ${what} into "${target.name}"?`,
        detail: `Cookies "${target.name}" already has for the same sites are replaced. Google's sign-in cookies are left out: sign in to Google here.${key}`,
        buttons: ["Import", "Cancel"],
        defaultId: 0,
        cancelId: 1,
      });
      if (response !== 0) throw new Error("the person did not allow it");
    }
    const read = from.file ? await cookies.readFile(from.file) : await cookies.read(source, { profile: from.profile });
    if (!read.ok) throw new Error(read.message || read.reason);
    const result = await shell.withCookieTarget(profileId, (session, sendCommand, onCommandTimeout) => cookies.write(read.cookies, { session, sendCommand, onCommandTimeout, log: shell.log }));
    if (result.error) throw new Error(result.error);
    for (const record of shell.live.values()) if (record.profile === profileId) record.view.webContents.reload();
    return { imported: result.imported, failed: result.failed, sites: result.domains.length, replaced: result.replaced, excluded: result.excluded };
  }
}
