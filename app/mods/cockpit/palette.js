// The palette (⌘⇧P in both cockpit layouts), a superset of Herdr's on its geometry: the agents in the hub's order, the
// shown layout's screens and commands, places, Herdr's commands and what the hub finds. As the keys sheet (⌘/) it
// lists the layout's keys instead.

import { div } from "gpui-kit";
import { h_flex, v_flex, InputState, Input } from "gpui-base";
import { commands, layouts, keys, spaces, agents, focused, terminals, run, props, closeOverlay, openOverlay, shared, setShared, window as win } from "herdr";
import * as hub from "./hub.js";
import { keyed, testKeys } from "./person.js";
import { statusColor } from "./agentsui.js";
import { tokens, alpha, TEXT, RADIUS, Icon, Kbd, Spinner, Button, AgentLogo, iconOf } from "./ui.js";
import { openInEditor, editorLabel } from "./editor.js";
import { goToAgent, agentStatus } from "./fleet.js";
import { screens, layoutName } from "./screens.js";
import { keyCaps, keyText, keyFor, NAMED, CATALOGUE, available, paletteAction, titleOf, sheet, runAction, lent, ANSWERED } from "./commands.js";

export { keyCaps, togglePalette } from "./commands.js";
import { openStart } from "./start.js";

// Herdr's palette: 620 wide, 60 from the top, 10 rows of 32 under a 36 search line, group titles 26.
export const WIDTH = 620;
const TOP = 60;
// Herdr keeps a popup 8px inside the window: the full-window popup starts there.
const INSET = 8;
const SHOWN = 10;
const ROW_H = 32;
const HEAD_H = 26;
const LIST_H = SHOWN * ROW_H;
const PAGE_H = LIST_H + HEAD_H;
const FOUND_HEAD = "Found in cockpit";

const GROUPS = ["Session", "Here", "Cockpit", "Places", "Tab", "Pane", "Space", "Layout", "Terminal", "Agent", "Window", "App"];

// Commands that act on something pointed at (ids, terminals): not for a list.
const HIDDEN = new Set([
  "space.move", "tab.move", "space.select", "tab.select", "pane.focus", "terminal.keep", "terminal.park", "terminal.kill",
  "terminal.show", "agent.focus", "layout.set", "layout.toggle", "layout.show", "layout.hide", "layout.resize",
  "layout.select", "mod.open", "overlay.close", "tab.settle", "tab.unsettle", "app.palette", "app.hide",
  "app.hide_others", "app.show_all", "app.theme", "app.settings",
]);

const FOUND = {
  pr: ["PR", "git-pull-request"], review: ["Review", "git-pull-request"], ticket: ["Ticket", "ticket"], step: ["Step", "book"],
  runbook: ["Runbook", "book"], block: ["Documents", "layers"], view: ["Documents", "layers"], todo: ["To-do", "list-todo"],
  meeting: ["Meeting", "calendar"], release: ["Release", "rocket"], app: ["Monitor", "activity"], routine: ["Routine", "clock"],
  changed: ["Changed", "history"], session: ["Session", "message"],
};

const STATUS = new Set(["working", "blocked", "failed", "done", "idle"]);

// The runner of the shown layout shows one of its screens on `cockpit.go:<child>`.
const show = (child) => run(`cockpit.go:${child}`);

/**
 * The rows before anything is typed: the agents the hub lists first (`listed`, ids in its order: waiting on you, then
 * the latest), the entries picked lately, then every group in order.
 */
/** Where the selection goes once the rows change under it: the same entry, else as close to where it was. */
export function keepSelection(rows, id, at) {
  const i = id ? rows.findIndex((r) => r.e.id === id) : -1;
  return i >= 0 ? i : Math.min(at, Math.max(0, rows.length - 1));
}

/** The fields of the agents view's rows the hub orders them by: what the palette sends instead of having it rebuilt. */
function slim(rows) {
  return rows.map((r) => ({
    terminal: r.terminal, address: r.address, frontend: r.frontend, title: r.title, stage: r.stage, status: r.status, since: r.since,
    waiting_since: r.waiting_since, space: r.space ? { name: r.space.name } : null, tab: r.tab ? { name: r.tab.name } : null, repo: r.repo,
    interrupted: r.interrupted, snoozed: r.snoozed ? { label: r.snoozed.label } : null, driven: r.driven, icon: r.icon, agent: r.agent, program: r.program,
  }));
}

export function emptyRows(all, recent, listed) {
  const byId = (id) => all.find((e) => e.id === id);
  const first = listed.map(byId).filter(Boolean);
  const used = new Set(first);
  const lately = recent.map(byId).filter((e) => e && !used.has(e)).slice(0, 5);
  for (const e of lately) used.add(e);
  const rest = GROUPS.flatMap((g) => all.filter((e) => e.group === g && !used.has(e)));
  const others = all.filter((e) => !GROUPS.includes(e.group) && !used.has(e));
  return [
    ...first.map((e) => ({ e, head: e.waiting ? "Waiting on you" : "Agents" })),
    ...lately.map((e) => ({ e, head: "Recent" })),
    ...[...rest, ...others].map((e) => ({ e, head: e.group })),
  ];
}

/** Letters of `q` in order in `text`: a score (word starts and runs count more) and where they matched; null: none. */
export function fuzzy(q, text) {
  const t = text.toLowerCase();
  let score = 0;
  let at = 0;
  let streak = 0;
  const hits = [];
  for (const ch of q) {
    const i = t.indexOf(ch, at);
    if (i < 0) return null;
    const start = i === 0 || /[\s:.\-_/›]/.test(t[i - 1]);
    streak = i === at && hits.length ? streak + 1 : 0;
    score += 1 + (start ? 6 : 0) + streak * 3 - Math.min(i - at, 6) * 0.3;
    hits.push(i);
    at = i + 1;
  }
  return { score: score - text.length * 0.02, hits };
}

/**
 * The entry a query nothing matches becomes: Start work on it, in the Start work dialog in Cockpit and in the chat's own
 * box in Cockpit Chat, prefilled either way and started by the person from there.
 */
export function startEntry(text, layout) {
  const chat = layout === "cockpit-chat";
  return {
    id: "start-work",
    group: "Start work",
    icon: "play",
    title: `Start work: ${text}`,
    hint: chat ? "In the chat's box: it gets a worktree and an agent" : "In a worktree of its own, with an agent",
    words: "",
    fallback: true,
    act: () => (chat ? run(keyed(`cockpit.via:chat|chat:start:new:${text}`)) : openStart(text), null),
  };
}

/** Tab and ⇧⇥ in the palette, from its search line too: the next and previous row. */
export const PALETTE_KEYS = [
  { keystroke: "tab", action: "cockpit_palette_next", context: "CockpitPalette" },
  { keystroke: "tab", action: "cockpit_palette_next", context: "CockpitPalette > Input" },
  { keystroke: "shift-tab", action: "cockpit_palette_prev", context: "CockpitPalette" },
  { keystroke: "shift-tab", action: "cockpit_palette_prev", context: "CockpitPalette > Input" },
];

export default class Palette {
  init(_props, cx) {
    this.at = 0;
    this.scroll = 0;
    this.found = [];
    this.searching = false;
    this.asked = 0;
    this.page = null;
    this.busy = null;
    this.error = null;
    this.editor = "VS Code";
    this.recent = Array.isArray(shared("cockpit.palette.recent")) ? shared("cockpit.palette.recent") : [];
    this.over = this.agentBelow();
    this.place = this.placeBelow();
    // The agents in the order every palette lists them (the hub's), and how many an empty query shows.
    this.jumps = [];
    this.listed = 0;
    // Whether the chat follows a session cockpit drives, whose own commands then show.
    this.driven = false;
    this.loaded = false;
    this.sheet = props()?.sheet === "keys" ? "keys" : null;
    this.handle = cx.focus_handle();
    // Tab and ⇧⇥ move through the rows: left to the window, they would take the keyboard out of the palette.
    try {
      cx.bind_keys(PALETTE_KEYS);
    } catch {}
    this.makeInput();
    cx.timer.after(400, () => this.input.focus());
    if (!this.sheet) this.load(cx);
  }

  /** The search line, its hint saying what it searches. */
  makeInput() {
    this.input = InputState.new({ placeholder: this.sheet ? "Search the keys…" : "Search cockpit or type a command…" });
    this.input.on("change", (_e, cx) => this.changed(cx));
    this.input.on("submit", (_e, cx) => this.pick(this.rows()[this.at]?.e, cx));
    this.input.focus();
  }

  /**
   * What the palette lists beyond Herdr's: the editor's name, the agents in the hub's order (from the agents view a
   * copy of the window already follows, rebuilt by the hub only without one) and, in the chat, its driven session.
   */
  load(cx) {
    this.loaded = true;
    cx.spawn(async (cx) => {
      this.editor = await editorLabel();
      cx.notify();
    });
    cx.spawn(async (cx) => {
      try {
        const known = lent();
        const args = known ? { agents: "app", rows: slim(known.agents ?? []) } : { agents: "app" };
        const [view, install] = await Promise.all([hub.view("palette", args), hub.install()]);
        // A remote desk's agents are the runner's: their addresses carry its machine, as the agents view's rows do.
        const at = (a) => (install?.machine ? `${install.machine}/${a}` : a);
        // A selection the person moved stays on its entry; one left at the top stays at the top.
        const id = this.at > 0 ? this.rows()[this.at]?.e?.id : null;
        this.jumps = (view?.agents ?? []).map((j) => ({ ...j, address: at(j.address) }));
        this.listed = view?.listed ?? 0;
        this.at = keepSelection(this.rows(), id, this.at);
      } catch {}
      cx.notify();
    });
    if (layoutName() === "cockpit-chat")
      cx.spawn(async (cx) => {
        try {
          this.driven = Boolean((await hub.view("session"))?.session);
        } catch {}
        cx.notify();
      });
  }

  /** Turns into the keys sheet or back into the palette, keeping the window and the keyboard. */
  setSheet(on, cx) {
    this.sheet = on ? "keys" : null;
    this.page = null;
    this.error = null;
    this.found = [];
    this.searching = false;
    this.asked++;
    this.at = 0;
    this.scroll = 0;
    this.makeInput();
    if (!on && !this.loaded) this.load(cx);
    cx.notify();
  }

  dismiss() {
    closeOverlay(props().overlay);
  }

  /** The agent focused when the palette opened: where found items can go, with its program as the hub names it. */
  agentBelow() {
    const f = focused();
    const address = f?.agent ?? f?.terminal;
    const a = address && agents().find((x) => x.address === address);
    if (!a) return null;
    // Herdr's program key, else what the agent reported ("claude-code", "codex"): the hub knows both spellings.
    const agent = a.program?.key ?? a.app ?? "";
    const t = terminals().find((x) => x.address === a.address);
    const labels = t?.labels ?? {};
    const session = Object.entries(labels).find(([k]) => /session/.test(k))?.[1] ?? null;
    // A terminal titled after its program says less than the place it runs in.
    const title = a.title && a.title !== a.program?.name ? a.title : `${a.space_name} › ${a.tab_name}`;
    return { terminal: a.address, title, cwd: t?.cwd ?? f.cwd ?? "", session, agent, name: a.program?.name ?? agent };
  }

  /** The folder of the focused terminal or space, and its machine, for Open in editor. */
  placeBelow() {
    const f = focused();
    const space = spaces().find((s) => s.id === f?.space);
    const folder = f?.cwd || space?.folder || null;
    return folder ? { folder, machine: space?.machine ?? null, name: folder.split("/").filter(Boolean).pop() ?? folder } : null;
  }

  changed(cx) {
    this.at = 0;
    this.scroll = 0;
    this.error = null;
    const query = this.input.value().trim();
    const asked = ++this.asked;
    // The keys sheet searches its own rows: the hub has nothing to add.
    if (query.length < 2 || this.sheet) {
      this.found = [];
      this.searching = false;
      cx.notify();
      return;
    }
    this.searching = true;
    cx.notify();
    cx.spawn(async (cx) => {
      let found = [];
      try {
        found = await hub.search(query);
      } catch {}
      if (asked !== this.asked) return;
      this.found = found;
      this.searching = false;
      cx.notify();
    });
  }

  // ---------------------------------------------------------------------------------------------------------------
  // Entries

  entries() {
    const known = new Set(commands().map((c) => c.name));
    const mac = win()?.platform === "macos";
    const list = [];
    const add = (e) => list.push({ args: {}, words: "", ...e, id: e.id ?? `${e.name}:${JSON.stringify(e.args ?? {})}` });

    if (this.over) {
      const o = this.over;
      add({ id: "fork-tab", group: "Session", icon: "git-fork", title: "Fork this session to a new tab", words: `${o.agent} ${o.title}`, act: () => this.agentsFork("tab") });
      add({ id: "fork-right", group: "Session", icon: "git-fork", title: "Fork this session to the right", words: `${o.agent} split ${o.title}`, act: () => this.agentsFork("right") });
    }
    if (this.place) {
      const p = this.place;
      add({ id: "editor", group: this.over ? "Session" : "Here", icon: "code", title: `Open ${p.name} in ${this.editor}`, words: `editor vscode code cursor ${p.folder}`, act: () => this.editorOpen() });
    }
    // The shown layout's screens, then its commands: only what does something here, each with the key it has here.
    const layout = layoutName();
    for (const s of screens().list) {
      add({ id: `app:${s.child}`, group: "Cockpit", icon: s.icon, title: s.label, name: s.command, words: "cockpit app go screen", act: () => (show(s.child), null) });
    }
    for (const e of CATALOGUE) {
      if (e.command === "cockpit.palette" || !available(layout, e.command.slice(8), { driven: this.driven })) continue;
      const base = { id: e.command, group: "Cockpit", icon: e.icon, title: e.title, name: e.command, words: e.words };
      if (e.command === "cockpit.keys") add({ ...base, sheet: true });
      else add({ ...base, act: () => (run(keyed(paletteAction(e))), null) });
    }
    add({ id: "refresh", group: "Cockpit", icon: "refresh", title: "Refresh the inbox", words: "fetch PRs reviews tickets CI now", act: () => hub.act("inbox", "request_refresh") });
    add({ id: "brief", group: "Cockpit", icon: "newspaper", title: "Today's brief", words: "morning meetings reviews CI tickets", page: "brief" });
    add({ id: "kit", group: "Cockpit", icon: "palette", title: "Design kit", words: "components gallery ui", name: "cockpit.kit", act: () => (run("cockpit.kit"), null) });

    for (const s of spaces()) {
      add({ id: `space:${s.id}`, group: "Places", icon: "folder", title: s.name, hint: s.branch ?? "", status: s.status, words: `space ${s.branch ?? ""}`, act: () => (run("space.select", { id: s.id }), show(screens().work), null) });
      if (s.tabs.length > 1)
        for (const t of s.tabs) add({ id: `tab:${t.id}`, group: "Places", icon: "app-window", title: `${s.name} › ${t.name}`, status: t.status, words: "tab", act: () => (run("tab.select", { id: t.id }), show(screens().work), null) });
    }
    // The hub's agents in its order, then any Herdr shows that the hub does not know yet. A session cockpit drives has
    // no terminal: the chat opens on it, and the cockpit layout, which has no chat, leaves it out.
    const listed = new Set();
    for (const j of this.jumps) {
      listed.add(j.address);
      if (j.driven && layout !== "cockpit-chat") continue;
      const act = j.driven ? () => (run(keyed(`cockpit.via:chat|chat:open:${j.terminal}`)), null) : () => this.jumpTo(j);
      add({ id: `agent:${j.address}`, group: "Places", icon: "bot", logo: j.icon ?? "agent", title: j.title, hint: j.help, status: j.status, waiting: j.waiting, words: `agent ${j.icon ?? ""} ${j.stage ?? ""}`, act });
    }
    for (const a of agents()) {
      if (listed.has(a.address)) continue;
      add({ id: `agent:${a.address}`, group: "Places", icon: "bot", logo: iconOf(a), title: a.title || a.program?.name || a.address, hint: `${a.space_name} › ${a.tab_name}`, status: a.status, words: `agent ${a.program?.name ?? ""} ${a.space_name}`, act: () => (goToAgent({ address: a.address }), null) });
    }

    for (const [name, args, group, title, words] of NAMED) {
      if (!known.has(name) || (mac && name.startsWith("window."))) continue;
      add({ name, args, group, title, words });
    }
    for (const l of layouts()) {
      const t = { main: "Herdr", terminals: "Terminals", win98: "Herdr 98", cockpit: "Cockpit" }[l] ?? l.charAt(0).toUpperCase() + l.slice(1);
      add({ name: "layout.switch", args: { name: l }, group: "Layout", icon: "layout", title: `Layout: ${t}`, words: `look ${l}` });
    }
    const named = new Set(NAMED.map(([n]) => n));
    for (const c of commands()) {
      if (named.has(c.name) || HIDDEN.has(c.name) || c.args.some((a) => a.required && a.name !== "name")) continue;
      add({ name: c.name, ...titleOf(c.name), words: c.about });
    }
    return list;
  }

  /** Brings an agent's terminal into view and marks it seen, as the herdr palette's jump does. */
  async jumpTo(j) {
    goToAgent({ address: j.address });
    try {
      await hub.act("agents", "seen", [{ terminal: j.terminal }]);
    } catch {}
    return null;
  }

  /** The key bound to an entry in the shown layout: `name`, or `name:value` for its first argument. */
  keyOf(e, bound) {
    if (e.keys) return e.keys;
    if (!e.name) return null;
    const v = Object.values(e.args ?? {})[0];
    return keyFor(v === undefined ? e.name : `${e.name}:${v}`, layoutName(), bound);
  }

  /** The keys sheet's rows: the shown layout's keys by group, filtered by what is typed. */
  sheetRows(q) {
    const rows = [];
    for (const g of sheet(layoutName(), keys())) {
      for (const [i, r] of g.rows.entries()) {
        // A key is found by its caps (⌘⇧J) as well as by its words (cmd shift j).
        const e = { id: `key:${g.title}:${i}:${r.keys}`, group: g.title, icon: r.icon, title: r.title, keys: r.keys, action: r.action, words: `${keyText(r.keys)} ${r.keys.replace(/-/g, " ")} ${r.action ?? ""}` };
        if (!q || fuzzy(q, `${g.title} ${r.title} ${e.words}`)) rows.push({ e, head: g.title, hits: q ? fuzzy(q, r.title)?.hits : null });
      }
    }
    return rows;
  }

  /** The rows: the agents first when nothing is typed, ranked while typing, then what the hub found. */
  rows() {
    const q = this.input.value().trim().toLowerCase().replace(/\s+/g, "");
    if (this.sheet) return this.sheetRows(q);
    const all = this.entries();
    if (!q) return emptyRows(all, this.recent, this.jumps.slice(0, this.listed).map((j) => `agent:${j.address}`));
    const rank = (e) => {
      const r = this.recent.indexOf(e.id);
      return r < 0 ? 99 : r;
    };
    const local = all
      .map((e) => {
        const title = fuzzy(q, e.title);
        const more = fuzzy(q, `${e.group} ${e.title} ${e.hint ?? ""} ${e.name ?? ""} ${e.words}`);
        // Letters strewn across the keywords are noise: a keyword match must keep them close together.
        const tight = more && more.hits[more.hits.length - 1] - more.hits[0] < q.length * 3;
        const best = title ?? (tight && { score: more.score - 4, hits: [] });
        return best && { e, score: best.score, hits: best.hits, head: null };
      })
      .filter(Boolean)
      .sort((a, b) => b.score - a.score || rank(a.e) - rank(b.e));
    const found = this.found.map((f, i) => {
      const [group, icon] = FOUND[f.kind] ?? FOUND[f.app] ?? ["Found", "search"];
      return { e: { id: `found:${i}`, group, icon, title: f.title, hint: f.help, found: f }, head: FOUND_HEAD };
    });
    if (local.length) return [...local, ...found];
    // No command matches: after what the hub found in past sessions, what was typed can start work.
    return [...found, { e: startEntry(this.input.value().trim(), layoutName()), head: found.length ? "Start work" : "Nothing matches" }];
  }

  // ---------------------------------------------------------------------------------------------------------------
  // Doing

  remember(e) {
    if (e.found || e.keys || e.fallback) return;
    this.recent = [e.id, ...this.recent.filter((r) => r !== e.id)].slice(0, 20);
    setShared("cockpit.palette.recent", this.recent);
  }

  /** Runs an entry; one that waits on the hub keeps the palette open until it answers, with its error in place. */
  pick(e, cx) {
    if (!e || this.busy) return;
    // A row of the keys sheet runs what its key runs; a key a view takes itself only says so.
    if (e.keys) {
      if (!e.action) return;
      this.dismiss();
      return runAction(e.action.startsWith("cockpit.") ? keyed(e.action) : e.action);
    }
    this.remember(e);
    if (e.sheet) return this.setSheet(true, cx);
    if (e.page) return this.openPage(e.page, cx);
    if (e.found) return this.wait(e.id, () => hub.go(e.found.target), cx);
    if (e.act) return this.wait(e.id, e.act, cx);
    this.dismiss();
    if (e.name === "mod.clone")
      openOverlay({ mod: "prompt", kind: "dialog", width: 380, props: { title: "Duplicate mod", hint: "Mod to copy; creates <name>-copy", command: { name: "mod.clone", args: {} } } });
    else run(e.name, e.name === "layout.clone" ? { name: shared("layout") ?? "main" } : e.args);
  }

  wait(id, work, cx) {
    this.busy = id;
    this.error = null;
    cx.notify();
    cx.spawn(async (cx) => {
      let error = null;
      try {
        const answer = await work();
        error = typeof answer === "string" ? answer : (answer?.error ?? null);
      } catch (err) {
        error = err?.message ?? String(err);
      }
      this.busy = null;
      if (!error) return this.dismiss();
      this.error = error;
      cx.notify();
    });
  }

  /** ctrl-r, ctrl-a, ctrl-f: the selected found item into the agent below. */
  intoSession(action, cx) {
    const e = this.rows()[this.at]?.e;
    if (!this.over || !e?.found || this.busy) return false;
    this.wait(`${e.id}:${action}`, () => hub.act("palette", action, [this.over, e.found]), cx);
    return true;
  }

  agentsFork(place) {
    return hub.act("agents", "fork", [{ terminal: `app:${this.over.terminal}`, place }]);
  }

  editorOpen() {
    return openInEditor(this.place.folder, this.place.machine);
  }

  openPage(kind, cx) {
    this.page = { kind, title: kind === "doctor" ? "Cockpit doctor" : "Today's brief", rows: null, at: 0 };
    cx.notify();
    cx.spawn(async (cx) => {
      try {
        const page = await hub.view("palette", { report: kind });
        if (this.page?.kind !== kind) return;
        this.page = { ...this.page, title: page.title ?? this.page.title, rows: page.rows ?? [] };
      } catch (err) {
        if (this.page) this.page.rows = [{ mark: "✗", tone: "bad", text: err?.message ?? String(err) }];
      }
      cx.notify();
    });
  }

  /** A key, from the window or `cockpit.key:` (tests): true when the palette used it. */
  key(e, cx, typed = false) {
    const key = e.keystroke;
    if (this.page) {
      const rows = this.page.rows ?? [];
      if (key === "escape") this.page = null;
      else if (key === "down") this.page.at = Math.min(this.page.at + 1, rows.length - 1);
      else if (key === "up") this.page.at = Math.max(this.page.at - 1, 0);
      else if (key === "enter" && rows[this.page.at]?.url) cx.spawn(async () => hub.openUrl(rows[this.page.at].url));
      else return false;
      return true;
    }
    const rows = this.rows();
    if (key === "escape") this.dismiss();
    else if (key === "down" || key === "ctrl-n") this.at = Math.min(this.at + 1, rows.length - 1);
    else if (key === "up" || key === "ctrl-p") this.at = Math.max(this.at - 1, 0);
    else if (key === "pagedown") this.at = Math.min(this.at + SHOWN, rows.length - 1);
    else if (key === "pageup") this.at = Math.max(this.at - SHOWN, 0);
    // A press of Enter in the search line is its submit; only a driven one comes here.
    else if (key === "enter" && !typed) this.pick(rows[this.at]?.e, cx);
    else if (key === "ctrl-r") return this.intoSession("insert", cx);
    else if (key === "ctrl-a") return this.intoSession("ask", cx);
    else if (key === "ctrl-f") return this.intoSession("fork", cx);
    else return false;
    return true;
  }

  /**
   * `cockpit.palette` and `cockpit.keys` close what they opened, or turn the other one into it in place; either way the
   * palette says it answered, so the runner opens no second one. `cockpit.palette:type:<text>` types into the search, in a
   * sandbox only.
   */
  command(c, cx) {
    if (c === "palette" || c === "keys") {
      setShared(ANSWERED, Date.now());
      if (Boolean(this.sheet) === (c === "keys")) this.dismiss();
      else this.setSheet(c === "keys", cx);
    } else if (c.startsWith("palette:type:")) {
      // Typing from a shell picks what the person's next ⏎ runs: a sandbox's checks only.
      cx.spawn(async (cx) => {
        if (!(await testKeys())) return;
        this.input.set_value(c.slice(13));
        this.changed(cx);
        cx.notify();
      });
    }
  }

  // ---------------------------------------------------------------------------------------------------------------
  // Drawing

  title(k, text, hits) {
    if (!hits?.length) return div().truncate().text_color(k.foreground).child(text);
    const parts = [];
    let cur = "";
    let lit = false;
    [...text].forEach((ch, i) => {
      const h = hits.includes(i);
      if (h !== lit && cur) {
        parts.push([cur, lit]);
        cur = "";
      }
      lit = h;
      cur += ch;
    });
    if (cur) parts.push([cur, lit]);
    return h_flex().overflow_hidden().children(parts.map(([s, h]) => div().flex_none().text_color(h ? k.foreground : k.mutedFg).when(h, (d) => d.font_semibold()).child(s.replace(/ /g, " "))));
  }

  /**
   * A key as the shown layout draws one: a Kbd chip in Cockpit; in Cockpit Chat, which keeps keys to its tooltips and
   * this sheet, quiet text in the sheet and nothing on the palette's rows.
   */
  keyHint(k, key) {
    if (layoutName() === "cockpit-chat" && !this.sheet) return div();
    const caps = keyCaps(key);
    if (layoutName() === "cockpit-chat") return div().flex_none().text_size(TEXT.sm).text_color(k.mutedFg).child(caps.join(" "));
    return Kbd(caps.join(""));
  }

  sessionButtons(k) {
    const b = (id, label, key, action) =>
      h_flex()
        .id(`into-${id}`)
        .h(22)
        .px(6)
        .gap(4)
        .flex_none()
        .items_center()
        .rounded(RADIUS.sm)
        .border_1()
        .border_color(k.border)
        .bg(k.background)
        .text_size(TEXT.xs)
        .font_medium()
        .text_color(k.foreground)
        .cursor_pointer()
        .hover((s) => s.bg(k.accent))
        .on_click((_e, cx) => {
          cx.stop_propagation();
          this.intoSession(action, cx);
        })
        .tooltip(`${label}: into ${this.over.title}`)
        .child(label)
        .child(div().text_color(k.dim).child(key));
    return h_flex()
      .gap(4)
      .flex_none()
      .child(b("insert", "Insert", "⌃R", "insert"))
      .child(b("ask", "Ask", "⌃A", "ask"))
      .child(b("fork", "Fork", "⌃F", "fork"));
  }

  leading(k, e, on, busy) {
    let glyph = null;
    if (busy) glyph = Spinner({ size: 14, color: k.mutedFg });
    else if (STATUS.has(e.status) || e.status === "error") {
      const st = agentStatus(e.status);
      glyph = st === "working" ? Spinner({ size: 12, color: k.working }) : div().size(8).rounded(4).bg(statusColor(k, st));
    }
    else if (e.icon) glyph = Icon(e.icon, { size: 15, color: on ? k.foreground : k.mutedFg });
    return h_flex().size(16).flex_none().items_center().justify_center().when(Boolean(glyph), (el) => el.child(glyph));
  }

  list(k, rows) {
    const q = this.input.value().trim();
    const bound = keys();
    const tops = [];
    let y = 0;
    rows.forEach((r, i) => {
      const head = Boolean(r.head) && (!q || this.sheet || r.head === FOUND_HEAD) && (i === 0 || r.head !== rows[i - 1].head);
      tops.push({ head, title: y, row: y + (head ? HEAD_H : 0) });
      y += (head ? HEAD_H : 0) + ROW_H;
    });
    const fixed = y > LIST_H;
    this.maxScroll = Math.max(0, y - LIST_H);
    const sel = tops[this.at];
    if (sel) {
      const from = sel.head ? sel.title : sel.row;
      if (from < this.scroll) this.scroll = from;
      if (sel.row + ROW_H > this.scroll + LIST_H) this.scroll = sel.row + ROW_H - LIST_H;
    }
    this.scroll = Math.max(0, Math.min(this.scroll, this.maxScroll));
    // Each row sits at its edges from the model, not stacked: whole-pixel rounding would drift a long stack.
    const place = (el, start, end) => (fixed ? el.absolute().left(0).right(0).top(start - this.scroll).h(end - start) : el.flex_none().h(end - start));
    const visible = (start, end) => !fixed || (end > this.scroll && start < this.scroll + LIST_H);
    const out = [];
    rows.forEach((r, i) => {
      const { head, title, row } = tops[i];
      if (head && visible(title, row))
        out.push(
          place(h_flex(), title, row)
            .px(14)
            .pb(4)
            .gap(6)
            .items_end()
            .text_size(TEXT.xs)
            .font_medium()
            .text_color(k.dim)
            .child(r.head)
            .when(r.head === FOUND_HEAD && this.searching, (e) => e.child(Spinner({ size: 11, color: k.dim }))),
        );
      if (!visible(row, row + ROW_H)) return;
      const on = i === this.at;
      const e = r.e;
      const key = this.keyOf(e, bound);
      const busy = Boolean(this.busy) && (this.busy === e.id || this.busy.startsWith(`${e.id}:`));
      const into = on && Boolean(e.found) && Boolean(this.over);
      out.push(
        place(h_flex(), row, row + ROW_H)
          .id(`pl-${i}`)
          .mx(6)
          .px(10)
          .gap(10)
          .items_center()
          .rounded(RADIUS.md + 1)
          .cursor_pointer()
          .text_size(TEXT.base)
          .when(on, (el) => el.bg(k.selected))
          .when(!on, (el) => el.hover((s) => s.bg(k.accent)))
          .on_click((_ev, cx) => {
            this.at = i;
            this.pick(e, cx);
          })
          .child(this.leading(k, e, on, busy))
          .child(
            h_flex()
              .flex_1()
              .min_w(0)
              .gap(8)
              .items_center()
              .when(Boolean(e.logo), (el) => el.child(AgentLogo(e.logo, { size: 14, color: on ? k.foreground : k.mutedFg })))
              .child(div().flex_none().max_w_full().child(this.title(k, e.title, r.hits)))
              .when(Boolean(e.hint), (el) => el.child(div().flex_1().min_w(0).truncate().text_size(TEXT.sm).text_color(k.dim).child(e.hint))),
          )
          .when(into, (el) => el.child(this.sessionButtons(k)))
          .when(!into && Boolean(q) && !key, (el) => el.child(div().flex_none().text_size(TEXT.xs).text_color(k.dim).child(e.group)))
          .when(!into && Boolean(key), (el) => el.child(this.keyHint(k, key))),
      );
    });
    return div()
      .id("palette-rows")
      .relative()
      .when(fixed, (el) => el.h(LIST_H).overflow_hidden())
      .on_scroll_wheel((ev, cx) => {
        const d = Math.abs(ev.delta.y) < 4 ? ev.delta.y * ROW_H : ev.delta.y;
        this.scroll = Math.max(0, Math.min(this.scroll - d, this.maxScroll));
        while (this.at < rows.length - 1 && tops[this.at].row < this.scroll) this.at++;
        while (this.at > 0 && tops[this.at].row + ROW_H > this.scroll + LIST_H) this.at--;
        cx.notify();
      })
      .children(fixed ? out : [v_flex().children(out)]);
  }

  pageView(k) {
    const p = this.page;
    const tone = { good: k.success, bad: k.destructive, warn: k.warning, dim: k.dim };
    const icon = { good: "circle-check", bad: "circle-x", warn: "alert", dim: "circle" };
    const head = h_flex()
      .h(HEAD_H)
      .pr(8)
      .gap(6)
      .items_center()
      .child(Button("page-back", { icon: "chevron-left", variant: "ghost", size: "icon-sm", tooltip: "Back  Esc", onClick: (_e, cx) => ((this.page = null), cx.notify()) }))
      .child(div().flex_1().text_size(TEXT.sm).font_medium().text_color(k.foreground).child(p.title))
      .when(p.rows === null, (e) => e.child(Spinner({ size: 13, color: k.mutedFg })));
    const rows = p.rows ?? [];
    const body =
      p.rows === null
        ? div().px(14).py(12).text_size(TEXT.base).text_color(k.mutedFg).child(p.kind === "doctor" ? "Checking every part of cockpit…" : "Reading today's brief…")
        : v_flex()
            .id("page-rows")
            .h(LIST_H)
            .overflow_y_scrollbar()
            .children(
              rows.map((r, i) =>
                h_flex()
                  .id(`page-${i}`)
                  .mx(6)
                  .px(10)
                  .py(7)
                  .gap(10)
                  .items_start()
                  .rounded(RADIUS.md + 1)
                  .text_size(TEXT.base)
                  .when(i === p.at, (e) => e.bg(k.selected))
                  .when(Boolean(r.url), (e) => e.cursor_pointer().hover((s) => s.bg(k.accent)).on_click(() => hub.openUrl(r.url)))
                  .child(div().pt(1).child(Icon(icon[r.tone] ?? "circle", { size: 15, color: tone[r.tone] ?? k.mutedFg })))
                  .child(
                    v_flex()
                      .flex_1()
                      .min_w(0)
                      .child(div().text_color(k.foreground).child(r.text))
                      .when(Boolean(r.url), (e) => e.child(div().truncate().text_size(TEXT.xs).text_color(k.dim).child(r.url ?? ""))),
                  )
                  .when(Boolean(r.url), (e) => e.child(Icon("external", { size: 13, color: k.dim }))),
              ),
            );
    return v_flex().h(PAGE_H).child(head).child(body);
  }

  render(cx) {
    const k = tokens();
    const rows = this.page ? [] : this.rows();
    this.at = Math.min(this.at, Math.max(0, rows.length - 1));
    const q = this.input.value().trim();
    const over = this.over;
    const searchLine = h_flex()
      .h(36)
      .flex_none()
      .px(10)
      .gap(10)
      .items_center()
      .rounded(RADIUS.lg + 1)
      .border_1()
      .border_color(k.ring)
      .bg(k.background)
      .child(Icon(this.sheet ? "keyboard" : "command", { size: 16, color: k.dim }))
      .child(Input.new(this.input).flex_1().h_full().text_size(TEXT.base).text_color(k.foreground))
      .when(Boolean(over), (e) =>
        e.child(
          h_flex()
            .id("over")
            .h(22)
            .max_w(200)
            .px(7)
            .gap(5)
            .flex_none()
            .items_center()
            .rounded(RADIUS.sm)
            .bg(alpha(k.info, 0.12))
            .text_size(TEXT.xs)
            .font_medium()
            .text_color(k.info)
            .tooltip(`Found items go into this ${over?.name ?? "agent"} session: ⌃R insert, ⌃A ask, ⌃F fork`)
            .child(Icon("bot", { size: 12, color: k.info }))
            .child(div().truncate().child(over?.title ?? "")),
        ),
      )
      .when(!over, (e) => e.child(div().text_size(TEXT.xs).text_color(k.dim).child("Esc")));
    const empty = div()
      .px(16)
      .py(14)
      .text_size(TEXT.base)
      .text_color(k.dim)
      .child(this.searching ? "Searching cockpit…" : this.sheet ? `No key matches "${q}".` : `Nothing matches "${q}".`);
    const error = this.error
      ? h_flex()
          .mx(6)
          .px(10)
          .py(6)
          .gap(8)
          .items_center()
          .rounded(RADIUS.md)
          .bg(alpha(k.destructive, 0.12))
          .text_size(TEXT.sm)
          .text_color(k.destructive)
          .child(Icon("circle-x", { size: 14, color: k.destructive }))
          .child(div().flex_1().truncate().child(this.error))
      : null;
    const card = v_flex()
      .id("palette-card")
      .w(WIDTH)
      .max_w_full()
      .p(18)
      .gap(10)
      .rounded(RADIUS.xl)
      .border_1()
      .border_color(k.border)
      .bg(k.popover)
      .shadow_lg()
      .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
      .child(searchLine)
      .when(Boolean(error), (e) => e.child(error))
      .child(this.page ? this.pageView(k) : rows.length ? this.list(k, rows) : empty);
    return div()
      .id("cockpit-palette")
      .track_focus(this.handle)
      .key_context("CockpitPalette")
      .on_action("cockpit_palette_next", (_e, cx) => (this.key({ keystroke: "down", key: "down" }, cx, true), cx.notify()))
      .on_action("cockpit_palette_prev", (_e, cx) => (this.key({ keystroke: "up", key: "up" }, cx, true), cx.notify()))
      .size_full()
      .flex()
      .flex_col()
      .items_center()
      .pt(TOP - INSET)
      .pl(16 - INSET)
      .pr(16 + INSET)
      .on_mouse_down("left", () => this.dismiss())
      .on_key_down((e, cx) => {
        if (!this.key(e, cx, true)) return;
        cx.stop_propagation();
        cx.notify();
      })
      .child(card);
  }
}
