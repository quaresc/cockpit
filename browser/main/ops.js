// What each request on the control socket does, by op.

import { dialog } from "electron";
import { suggest } from "./history.js";
import * as profiles from "./profiles.js";

/** Asks the person to confirm what a request without the hub's token wants to do to their profiles. */
async function confirmed(shell, who, message, detail) {
  if (!who.guarded || who.trusted) return true;
  const { response } = await dialog.showMessageBox(shell.window.win, { type: "question", message, detail, buttons: ["Allow", "Cancel"], defaultId: 1, cancelId: 1 });
  return response === 0;
}

export function handlers(shell, { quit, tracker }) {
  const profileNamed = (id) => {
    const found = shell.registry.profiles.find((p) => p.id === id || p.name.toLowerCase() === String(id).toLowerCase());
    if (!found) throw new Error(`no profile ${id}`);
    return found;
  };
  return {
    ping: () => ({ ok: true }),
    status: () => ({
      ok: true,
      pid: process.pid,
      port: shell.cdpPort,
      docked: shell.options.dock,
      shown: shell.window.win.isVisible(),
      current: shell.current,
      browsers: [...shell.browsers.values()].map((wt) => ({ owner: wt.owner, worktree: wt.root, tabs: wt.tabs.length, awake: wt.tabs.filter((t) => shell.live.has(t.id)).length })),
      versions: { electron: process.versions.electron, chrome: process.versions.chrome },
    }),
    quit: () => (setTimeout(quit, 20), { ok: true }),
    configure: (r) => {
      const before = shell.options;
      shell.options = { ...before, ...(r.options || {}), dock: before.dock };
      if (Array.isArray(r.servers)) shell.servers = r.servers;
      shell.push();
      return { ok: true, restart: r.options?.dock !== undefined && r.options.dock !== before.dock };
    },
    open: (r) => shell.open({ owner: r.owner, worktree: r.worktree, repo: r.repo, url: r.url, reload: !!r.reload, background: !!r.background, fresh: !!r.fresh }),
    "tabs.list": (r) => (r.owner ? { owner: r.owner, tabs: shell.known(r.owner) ? shell.browser(r.owner) && shell.tabsOf(r.owner) : [] } : { browsers: [...shell.browsers.keys()].map((owner) => ({ owner, tabs: shell.tabsOf(owner) })) }),
    "tab.activate": (r) => shell.activate(r.owner, String(r.tab)),
    "tab.close": (r) => shell.close(r.owner, r.tab),
    "tab.reload": (r) => shell.nav(r.owner, r.tab, r.hard ? "hard" : "reload"),
    "tab.back": (r) => shell.nav(r.owner, r.tab, "back"),
    "tab.forward": (r) => shell.nav(r.owner, r.tab, "forward"),
    "tab.navigate": (r) => shell.navigate(r.owner, r.tab, r.url),
    "tab.viewport": (r) => ({ viewport: shell.setViewport(r.owner, r.tab, r.preset || (r.width ? { width: r.width, height: r.height, scale: r.scale, mobile: r.mobile } : null)) }),
    "tab.devtools": (r) => shell.toggleDevtools(r.owner, r.tab, r.open),
    "agent.endpoint": (r) => shell.agentEndpoint(r),
    "agent.touch": (r) => shell.touch(r.owner, r.tab),
    "slot.set": (r) => {
      shell.setSlot(r.slot || r);
      shell.emit("slot", { hostPid: shell.slot?.hostPid || 0 });
    },
    "slot.drop": () => {
      shell.setSlot(null);
      shell.emit("slot", { hostPid: 0 });
      tracker.host(0);
    },
    "profiles.list": () => ({ profiles: shell.registry.profiles, scope: shell.options.profileScope }),
    "profile.sources": async () => ({ sources: await shell.menus.sources() }),
    "profile.create": async (r, who) => {
      if (!(await confirmed(shell, who, `Create the browser profile "${r.name}"?`, "A command asked for it."))) throw new Error("the person did not allow it");
      const [registry, profile] = profiles.created(shell.registry, r.name);
      shell.setRegistry(registry);
      return { profile };
    },
    "profile.rename": (r) => (shell.setRegistry(profiles.renamed(shell.registry, profileNamed(r.profile).id, r.name)), shell.push()),
    "profile.delete": async (r, who) => {
      const profile = profileNamed(r.profile);
      if (!(await confirmed(shell, who, `Delete the browser profile "${profile.name}"?`, "Its cookies, storage and cache are deleted with it."))) throw new Error("the person did not allow it");
      await shell.deleteProfile(profile.id);
    },
    "profile.use": (r) => ({ profile: shell.useProfile(r.owner, r.profile ? profileNamed(r.profile).id : null) }),
    "profile.import": async (r, who) => shell.menus.importCookies(profileNamed(r.profile).id, { source: r.source, profile: r.from, file: r.file }, { ask: !who.trusted }),
    "grab.start": (r) => shell.design.start(r.owner, r.tab, r.mode === "annotate" ? "annotate" : "copy"),
    "grab.stop": () => shell.design.stop(),
    "tab.sleep": (r) => shell.sleep(String(r.tab)),
    "notes.send": (r) => {
      const notes = shell.notes.get(r.owner) || [];
      if (!notes.length) throw new Error("no note to send");
      shell.design.send(r.owner, notes, true);
      return { sent: notes.length };
    },
    "notes.clear": (r) => shell.design.clear(r.owner),
    "notes.sent": (r) => shell.design.sent(r.owner, r.ok === true, r.error),
    "notes.list": (r) => ({ notes: shell.notes.get(r.owner) || [] }),
    "owners.list": () => ({ owners: [...shell.kept()].map(([owner, worktree]) => ({ owner, worktree })) }),
    "owner.forget": (r) => shell.forget(r.owner),
    "worktree.forget": (r) => shell.forgetWorktree(r.worktree),
    "history.query": (r) => ({ entries: suggest(shell.history, r.q, { limit: Number(r.limit) || 10 }) }),
    "downloads.list": () => ({ downloads: shell.downloadList() }),
  };
}
