import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import { createContext, SourceTextModule, SyntheticModule } from "node:vm";
import { test } from "node:test";

/** Cockpit Chat's top bar, its menus and its side panel's file, on a stand-in Herdr: 12 tabs in one space by default. */
async function fixture({ tabs = 12, active = 1, settled = [], layoutName = "cockpit-chat", panelBody = "review", panelOpen = false, window = null, agents = null, live = [], focus = { tab: "t1", terminal: "t7" } } = {}) {
  const calls = [], notices = [], overlays = [], acts = [];
  // Drawn elements as the calls made on them, so a test reads what a component drew.
  const element = () => {
    const made = [];
    const el = new Proxy({}, { get: (_t, name) => (name === "made" ? made : (...args) => (made.push([name, ...args]), el)) });
    return el;
  };
  const state = new Map([["layout", layoutName], ["cockpit.app", "chat"]]);
  const space = {
    id: "s1",
    active: true,
    tabs: Array.from({ length: tabs }, (_, i) => ({ id: `t${i + 1}`, name: `feature-branch-${i + 1}`, active: i + 1 === active, status: i === 3 ? "blocked" : "idle", settled: settled.includes(`t${i + 1}`) })),
  };
  const tree = { id: "root", children: [{ id: "sidebar" }, { id: "panel", hidden: !panelOpen, children: [{ id: "panel-body", active: panelBody, children: [{ id: "code" }] }] }] };
  const context = createContext({ console, ...(window ? { window } : {}) });
  const imports = {
    herdr: {
      spaces: () => [space],
      // Herdr's own agents: the terminals that report a status, each in its tab.
      agents: () => live,
      run: (...args) => calls.push(args),
      shared: (key) => state.get(key) ?? null,
      setShared: (key, value) => state.set(key, value),
      notify: (n) => notices.push(n),
      openOverlay: (spec) => overlays.push(spec),
      window: () => ({ platform: "macos", titlebar: "none", fullscreen: false }),
      layout: () => ({ ...tree, traffic_lights: [12, 16] }),
      panes: () => [{ pane: "p1", tab: "t1", terminal: "t7", focused: true }],
      focused: () => focus,
      keys: () => [{ section: "layout:cockpit-chat", keys: "cmd-shift-a", action: "cockpit.tabs" }],
      exec: async () => ({ code: 1, stdout: "", stderr: "" }),
      appHome: () => "/app",
      home: () => "/home",
      theme: () => ({ foreground: "#ededed", background: "#1e1e1e", hover: "#2a2a2a", muted: "#a0a0a0", dim: "#707070", border: "#333333" }),
    },
    "gpui-base": { InputState: { new: () => ({ value: () => "", set_value() {}, focus() {} }) }, h_flex: element, v_flex: element },
    "gpui-kit": { div: element },
    "./hub.js": {
      act: async (...a) => (acts.push(a), agents ? { note: "review sent to Fix the login redirect (1 comment)" } : (() => { throw new Error("no hub action expected"); })()),
      view: async () => (agents ? { agents } : null),
      openUrl: () => {},
      subscribe: () => ({ stop() {}, park() {} }),
      isStoppedOnPurpose: (e) => String(e).includes("stopped on purpose"),
      downWords: (e, short) => (String(e).includes("stopped on purpose") ? (short ? "Hub stopped" : "The cockpit hub was stopped") : short ? "Hub not running" : "The cockpit hub isn't running"),
      downTip: (e) => (String(e).includes("stopped on purpose") ? "The cockpit hub was stopped on purpose, with cockpit hub stop" : `The cockpit hub isn't running: ${String(e).replace(/^error:\s*/, "")}`),
      isNotInstalled: (e) => String(e).includes("not installed"),
      remoteMachine: () => null,
      startHub: async () => {},
    },
  };
  const sources = new Map(), modules = new Map();
  const folder = new URL("../mods/cockpit/", import.meta.url);
  for (const name of await readdir(folder)) {
    if (!name.endsWith(".js")) continue;
    const text = await readFile(new URL(name, folder), "utf8");
    sources.set(`./${name}`, text);
    for (const match of text.matchAll(/import\s*\{([^}]+)\}\s*from\s*"(herdr|gpui-kit|gpui-base)"/g)) {
      const exports = imports[match[2]] ??= {};
      for (const word of match[1].split(",")) {
        const key = word.trim().split(/\s+as\s+/)[0];
        exports[key] ??= () => { throw new Error(`unexpected native call: ${key}`); };
      }
    }
  }
  function load(name) {
    if (modules.has(name)) return modules.get(name);
    const native = imports[name];
    const module = native
      ? new SyntheticModule(Object.keys(native), function () {
        for (const [key, value] of Object.entries(native)) this.setExport(key, value);
      }, { context })
      : new SourceTextModule(sources.get(name), { context, identifier: name });
    modules.set(name, module);
    return module;
  }
  const module = new SourceTextModule('export * as top from "./chat-top.js"; export * as ui from "./ui.js"; export * as code from "./code.js"; export * as commands from "./commands.js"; export * as screens from "./screens.js";', { context });
  await module.link(load);
  await module.evaluate();
  const { top, ui, code, commands, screens } = module.namespace;
  // The bar as a copy of it stands after its first layout: the window 1280 px wide, the sidebar open.
  const bar = Object.create(top.default.prototype);
  bar.sizes = new Map([["bar", 1280 - 8 - 268 - 8 - 8], ["right", 104]]);
  bar.sessions = [];
  bar.hub = "live";
  return { top, ui, code, commands, screens, bar, space, state, calls, notices, overlays, acts, menus: () => overlays.filter((o) => o.props?.app === "menu") };
}

/** Values from the module's own realm compared by their contents. */
const plain = (v) => (v === undefined ? v : JSON.parse(JSON.stringify(v)));
const eq = (actual, expected, message) => assert.deepEqual(plain(actual), plain(expected), message);

const keys = (list) => list.map((p) => p.tab?.id ?? p.session?.terminal);

/** A menu item's command without the one-time key its menu was opened with. */
const bare = (item) => (typeof item === "object" && item?.command ? { ...item, command: item.command.replace(/~[0-9a-f]{32}$/, "") } : item);

test("12 tabs at 1280 px: the bar shows what fits, the active one always, and the +N menu holds every other", async () => {
  for (const active of [1, 12]) {
    const f = await fixture({ active });
    const { shown, hidden } = f.bar.layoutNow();
    assert.ok(shown.length >= 3 && hidden.length > 0, `some fit, the rest overflow (${shown.length} shown)`);
    assert.ok(keys(shown).includes(`t${active}`), "the tab the chat reads stays in the bar");
    eq([...keys(shown), ...keys(hidden)].sort(), f.space.tabs.map((t) => t.id).sort(), "every tab is in the bar or the menu");
    const room = 1280 - 8 - 268 - 8 - 8 - f.top.lightsRoom() - 12 - 16 - 104 - 10;
    const used = shown.reduce((w, p) => w + p.width + 2, 0) + 52;
    assert.ok(used <= room + 2, "what shows fits beside the +N pill");
    f.bar.openTabs();
    const items = f.menus().at(-1).props.items;
    eq(items.map((i) => bare(i).command), keys(hidden).map((id) => `tabs:select:${id}`));
    f.bar.command(bare(items.at(-1)).command, { notify() {} });
    eq(f.calls.filter((c) => c[0] === "tab.select").at(-1), ["tab.select", { id: keys(hidden).at(-1) }], "a hidden tab is one pick away");
  }
});

test("at 760 px fewer fit, and nothing is lost", async () => {
  const f = await fixture();
  f.bar.sizes.set("bar", 760 - 16);
  const { shown, hidden } = f.bar.layoutNow();
  assert.ok(hidden.length > 6);
  assert.equal(shown.length + hidden.length, 12);
  const wide = await fixture();
  wide.bar.sizes.set("bar", 1600 - 8 - 268 - 8 - 8);
  assert.ok(wide.bar.layoutNow().shown.length > shown.length);
});

test("pills fit in order, the one on kept, room left for +N; all of them when the bar is not measured yet", async () => {
  const { top } = await fixture();
  const pills = [100, 100, 100, 100].map((width, i) => ({ key: `p${i}`, width, on: i === 3 }));
  eq(top.fitPills(pills, 500), { shown: ["p0", "p1", "p2", "p3"], hidden: [] });
  eq(top.fitPills(pills, 380), { shown: ["p0", "p1", "p3"], hidden: ["p2"] });
  eq(top.fitPills(pills, 320), { shown: ["p0", "p3"], hidden: ["p1", "p2"] }, "in order: one that does not fit ends the bar");
  eq(top.fitPills(pills, null).hidden, []);
  assert.equal(top.guessWidth("x".repeat(400)), top.guessWidth("y".repeat(300)), "a long name is cut");
});

test("closing the tab the chat reads archives it, the next one comes in front, and Undo brings it back", async () => {
  const f = await fixture({ active: 2 });
  const { shown } = f.bar.layoutNow();
  const t = shown.find((p) => p.on).tab;
  f.bar.archive(t, f.space.tabs.filter((x) => !x.settled));
  eq(f.calls.slice(-2), [["tab.select", { id: "t3" }], ["tab.settle", { id: "t2" }]]);
  const toast = f.notices.at(-1);
  assert.match(toast.title, /Archived feature-branch-2/);
  assert.match(toast.body, /undo/i);
  assert.equal(toast.action.name, "cockpit.tabs:restore:t2", "the toast's click runs the bar's restore");
  // Herdr settles it: the bar leaves it out and the +N menu offers it back.
  f.space.tabs[1].settled = true;
  f.space.tabs[1].active = false;
  f.space.tabs[2].active = true;
  const after = f.bar.layoutNow();
  assert.ok(!keys(after.shown).includes("t2") && !keys(after.hidden).includes("t2"));
  eq(after.archived.map((x) => x.id), ["t2"]);
  f.bar.openTabs();
  const items = f.menus().at(-1).props.items;
  eq(items.slice(-2).map(bare), ["separator", { label: "feature-branch-2", icon: "archive", command: "tabs:restore:t2" }]);
  f.bar.command(toast.action.name.slice("cockpit.".length), { notify() {} });
  eq(f.calls.filter((c) => c[0].startsWith("tab.")).slice(-2), [["tab.unsettle", { id: "t2" }], ["tab.select", { id: "t2" }]]);
});

test("the archive toast's Undo brings the tab back in the Cockpit layout too", async () => {
  const f = await fixture({ layoutName: "cockpit" });
  assert.equal(f.commands.runCommand("tabs:restore:t2", { notify() {}, spawn() {} }, "cockpit", {}), true, "Cockpit's runner has it");
  eq(f.calls.filter((c) => c[0].startsWith("tab.")), [["tab.unsettle", { id: "t2" }], ["tab.select", { id: "t2" }]]);
});

test("pills fit with one gap between each, none spent before +N when no pill is on", async () => {
  const { top } = await fixture();
  const pills = [100, 100, 100, 100].map((width, i) => ({ key: `p${i}`, width, on: false }));
  const room = 100 + 4 + 100 + 4 + 40;
  eq(top.fitPills(pills, room, { gap: 4, more: 40 }), { shown: ["p0", "p1"], hidden: ["p2", "p3"] });
  eq(top.fitPills(pills, room - 1, { gap: 4, more: 40 }), { shown: ["p0"], hidden: ["p1", "p2", "p3"] });
});

test("the +N menu picks its first item only when its key opened it, not a click", async () => {
  const f = await fixture();
  f.bar.openTabs();
  assert.equal(f.menus().at(-1).props.picked, undefined, "a click picks nothing");
  f.bar.command("tabs", { notify() {} });
  assert.equal(f.menus().at(-1).props.picked, 0);
});

test("the last tab open is archived in place, with nothing to bring in front", async () => {
  const f = await fixture({ tabs: 2, active: 2, settled: ["t1"] });
  f.bar.archive(f.space.tabs[1], f.space.tabs.filter((x) => !x.settled));
  eq(f.calls.at(-1), ["tab.settle", { id: "t2" }]);
  assert.ok(!f.calls.some((c) => c[0] === "tab.select"));
});

test("a tab with no agent, like Herdr's numbered \"3\", shows its panes; a tab with one, its agent's chat", async () => {
  const live = [{ address: "t41", tab: "t2", focused: false }, { address: "t44", tab: "t5", focused: false }, { address: "t45", tab: "t5", focused: true }];
  const f = await fixture({ live });
  f.space.tabs[2].name = "3";
  const screens = () => f.calls.filter((c) => c[0] === "layout.select").map((c) => c[1].child);
  const pill = (id) => f.bar.pill(f.space.tabs.find((t) => t.id === id), false, f.ui.tokens(), f.space.tabs);
  const click = (el) => el.made.find((m) => m[0] === "on_click")[1]({}, { notify() {}, stop_propagation() {} });
  // The chat was pinned on a session cockpit drives: a tab picked on purpose ends that.
  f.state.set("cockpit.chat.pinned", { key: "claude:1", focus: null, at: 0 });
  click(pill("t3"));
  eq(f.calls.filter((c) => c[0] === "tab.select").at(-1), ["tab.select", { id: "t3" }]);
  eq(screens().slice(-2), ["work", "tab"], "the shell's panes on the Tab screen, not the chat of some other agent");
  assert.equal(f.state.get("cockpit.chat.pinned"), null);
  assert.ok(!f.calls.some((c) => c[0] === "agent.focus"), "no agent runs there to follow");
  click(pill("t2"));
  eq(f.calls.filter((c) => c[0] === "agent.focus").at(-1), ["agent.focus", { terminal: "t41" }], "the chat is told which agent this tab runs");
  eq(screens().slice(-2), ["work", "chat"]);
  // Two agents in one tab: the one in front there.
  click(pill("t5"));
  eq(f.calls.filter((c) => c[0] === "agent.focus").at(-1), ["agent.focus", { terminal: "t45" }]);
  // The +N menu's pick and an archived tab brought back show what their tab holds too.
  f.bar.command("tabs:select:t3", { notify() {} });
  eq(screens().at(-1), "tab");
  f.bar.command("tabs:restore:t2", { notify() {}, spawn() {} });
  eq(f.calls.filter((c) => c[0].startsWith("tab.")).slice(-2), [["tab.unsettle", { id: "t2" }], ["tab.select", { id: "t2" }]]);
  eq(screens().at(-1), "chat");
});

test("⌘⇧A opens the menu under the +N pill, its first item picked; with nothing hidden, every tab", async () => {
  const f = await fixture({ window: { viewport_size: () => ({ width: 1280, height: 800 }), mouse_position: () => ({ x: -40, y: -40 }) } });
  f.bar.sizes.set("left", 120);
  f.bar.sizes.set("tabs", 600);
  f.bar.sizes.set("more", 50);
  f.bar.command("tabs", { notify() {} });
  const menu = f.menus().at(-1);
  const x = 1280 - 8 - f.bar.sizes.get("bar") + f.top.lightsRoom() + 120 + 8 + 600 - 5 - 50;
  eq(menu.at, { x: Math.round(Math.min(x, 1280 - 260 - 8)), y: 8 + 47 + 4 });
  assert.equal(menu.props.picked, 0, "↓ and ⏎ start from the first item");
  const few = await fixture({ tabs: 3 });
  few.bar.command("tabs", { notify() {} });
  eq(few.menus().at(-1).props.items.map((i) => bare(i).command), ["tabs:select:t1", "tabs:select:t2", "tabs:select:t3"]);
  assert.equal(few.menus().at(-1).props.items[0].icon, "check", "the one in front is marked");
});

test("a menu opened from a key goes under its control, kept inside the window; else at the pointer, or the top middle", async () => {
  const { ui } = await fixture();
  const now = (x, y) => ({ size: { width: 1000, height: 700 }, pointer: { x, y } });
  eq(ui.menuAt({ x: 300, y: 60 }, 220, 200, now(5, 5)), { x: 300, y: 60 });
  eq(ui.menuAt({ x: 950, y: 650 }, 220, 200, now(5, 5)), { x: 772, y: 492 }, "pushed back inside");
  assert.equal(ui.menuAt(null, 220, 200, now(500, 300)), "pointer");
  eq(ui.menuAt(null, 220, 200, now(-20, 300)), { x: 390, y: 64 }, "a pointer outside the window is no place for it");
  assert.equal(ui.menuAt(null, 220, 200, null), "pointer", "outside a host call");
});

test("the hub's dot says its state without colour, and a hub down on this machine starts from it", async () => {
  const { top } = await fixture();
  assert.equal(top.hubDot("live").kind, "live");
  assert.equal(top.hubDot("connecting").kind, "connecting");
  const stopped = top.hubDot("error: the cockpit hub was stopped on purpose");
  eq([stopped.kind, stopped.label, stopped.startable], ["down", "Hub stopped", true]);
  assert.equal(top.hubDot("error: connection refused").label, "Hub not running");
  // The tooltip says why the window could not reach it, in the error's own words.
  assert.equal(top.hubDot("error: connection refused").tooltip, "The cockpit hub isn't running: connection refused · click to start it");
  assert.equal(top.hubDot("error: connection refused", { remote: true }).tooltip, "The cockpit hub isn't running: connection refused");
  assert.equal(top.hubDot("error: connection refused", { remote: true }).startable, false, "a runner's hub is started there");
  assert.equal(top.hubDot("error: cockpit is not installed").startable, false);
});

test("in Cockpit Chat a file opens in the side panel beside the chat, and Close gives the panel back", async () => {
  const f = await fixture({ panelBody: "review", panelOpen: true });
  f.state.set("cockpit.panel.open", true);
  await f.code.openCode("/repo/lib/a.ex", { line: 12 });
  const spec = f.state.get(f.code.CODE_PANEL);
  eq([spec.file, spec.line, spec.terminal, spec.back], ["/repo/lib/a.ex", 12, "t7", { open: true, child: "review" }]);
  assert.ok(f.calls.some((c) => c[0] === "layout.select" && c[1].stack === "panel-body" && c[1].child === "code"));
  assert.ok(f.calls.some((c) => c[0] === "layout.show" && c[1].id === "panel"));
  assert.ok(!f.calls.some((c) => c[0] === "pane.split"), "no pane hidden behind the chat");
  const view = new f.code.default();
  view.init({ app: "code", panel: true });
  view.follow();
  eq([view.file, view.line, view.loaded], ["/repo/lib/a.ex", 12, false]);
  view.loaded = true;
  view.follow();
  assert.equal(view.loaded, true, "the same file is taken up once");
  f.calls.length = 0;
  view.closePane();
  eq(f.calls.filter((c) => c[0] === "layout.select"), [["layout.select", { stack: "panel-body", child: "review" }]]);
});

test("a file opened while the panel was closed closes it again on Close; the Cockpit layout still splits a pane", async () => {
  const f = await fixture({ panelBody: "show" });
  f.state.set("cockpit.panel.open", false);
  await f.code.openCode("/repo/README.md");
  const view = new f.code.default();
  view.init({ app: "code", panel: true });
  view.follow();
  view.closePane();
  eq(f.calls.at(-1), ["layout.hide", { id: "panel" }]);
  assert.equal(f.state.get("cockpit.panel.open"), false);
  const cockpit = await fixture({ layoutName: "cockpit" });
  await cockpit.code.openCode("/repo/README.md");
  assert.equal(cockpit.calls.at(-1)[0], "pane.split");
  assert.equal(cockpit.state.get(cockpit.code.CODE_PANEL), undefined);
});

test("a button's key is a Kbd chip in Cockpit, and goes into its tooltip in Cockpit Chat", async () => {
  const said = (el, name) => el.made.filter((c) => c[0] === name).map((c) => c[1]);
  const chat = await fixture();
  const send = chat.ui.Button("code-send", { label: "Send", kbd: "⏎", onClick() {} });
  assert.deepEqual(said(send, "tooltip"), ["Send  ⏎"]);
  assert.equal(said(send, "child").length, 1, "the label only: no Kbd chip");
  assert.deepEqual(said(chat.ui.Button("code-save", { label: "Save", kbd: "⌘S", tooltip: "Save  ⌘S" }), "tooltip"), ["Save  ⌘S"], "a tooltip that names its key keeps it once");
  assert.deepEqual(said(chat.ui.Button("form-ok", { label: "Add routine", kbd: "⌘⏎", tooltip: "Add it" }), "tooltip"), ["Add it  ⌘⏎"]);
  const cockpit = await fixture({ layoutName: "cockpit" });
  const kept = cockpit.ui.Button("code-send", { label: "Send", kbd: "⏎", onClick() {} });
  assert.deepEqual(said(kept, "tooltip"), [], "Cockpit keeps its Kbd hint on the button");
  assert.equal(said(kept, "child").length, 2, "its label, then its Kbd chip");
});

test("Ask the agent in Cockpit Chat goes to the chat's subject, a driven session included, and says who took it", async () => {
  const key = "claude:1793611000000";
  const f = await fixture({ agents: [{ terminal: key, address: key, driven: true, title: "Fix the login redirect" }, { terminal: "app:t7", address: "t7", title: "Quota banner" }] });
  f.state.set("cockpit.chat.subject", key);
  const view = Object.create(f.code.default.prototype);
  Object.assign(view, { file: "/home/acme/api/src/auth.ts", terminal: "t7", line: 3, code: { value: () => "a\nb\nc" }, ask: { value: () => "fix this", set_value() {} }, toasts: { msgs: [], push(_cx, m) { this.msgs.push(m); } } });
  assert.deepEqual({ ...(await view.askTarget()) }, { handle: key, name: "Fix the login redirect" });
  const work = [];
  view.sendAsk({ spawn: (fn) => work.push(fn({ notify() {} })), notify() {} });
  await Promise.all(work);
  const [app, action, args] = f.acts.at(-1);
  assert.deepEqual([app, action, args[0], args[1].to], ["review", "send", key, "Fix the login redirect"]);
  assert.equal(view.toasts.msgs.at(-1), "Sent to Fix the login redirect");
  const cockpit = await fixture({ layoutName: "cockpit", agents: [{ terminal: "app:t7", address: "t7", title: "Quota banner" }] });
  cockpit.state.set("cockpit.chat.subject", key);
  const beside = Object.assign(Object.create(cockpit.code.default.prototype), { terminal: "t7" });
  assert.deepEqual({ ...(await beside.askTarget()) }, { handle: "t7", name: "Quota banner" }, "Cockpit asks the agent beside the file");
});

test("the toggle to the chat agent's terminal shows its provider's logo and names whose terminal it shows", async () => {
  const f = await fixture();
  const claude = { terminal: "app:t41", address: "t41", icon: "claude", agent_name: "Claude Code", tab: { id: "t1" } };
  const driven = { terminal: "claude:demo-421", address: "claude:demo-421", icon: "claude", agent_name: "Claude Code", driven: true };
  f.bar.rows = [claude, driven];
  f.state.set("cockpit.chat.subject", "app:t41");
  eq(f.bar.subjectAgent()?.terminal, "app:t41", "the chat's subject");
  assert.equal(f.top.terminalWords(f.bar.subjectAgent()), "Show Claude Code's terminal");
  assert.equal(f.ui.iconOf(f.bar.subjectAgent()), "claude", "drawn with Claude Code's logo, not a terminal glyph");
  f.state.set("cockpit.chat.subject", "claude:demo-421");
  assert.equal(f.top.terminalWords(f.bar.subjectAgent()), "Open Claude Code's session in a terminal of its own", "a session cockpit drives has no terminal yet");
  f.state.delete("cockpit.chat.subject");
  f.bar.rows = [];
  assert.equal(f.top.terminalWords(f.bar.subjectAgent()), "Its terminal", "no agent known: the plain words and glyph");
});


test("the terminal logo follows the tab in view, never Herdr's unrelated agent fallback", async () => {
  const live = [{ address: "t41", tab: "agent-tab", program: { name: "Claude Code", icon: "icons/claude.svg", tint: false } }];
  const shell = await fixture({ live, focus: { tab: "shell-tab", terminal: "t9", agent: "t41" } });
  assert.equal(shell.bar.subjectAgent(), null);
  const split = await fixture({ live, focus: { tab: "agent-tab", terminal: "t9", agent: "unrelated" } });
  assert.equal(split.bar.subjectAgent().address, "t41");
});


test("a tab still at Herdr's number shows the name of the agent in it; a label anyone gave stays", async () => {
  const f = await fixture();
  const rows = [{ terminal: "app:t9", tab: { id: "t3" }, title: "Daily quota limiter" }];
  eq(f.top.tabName({ id: "t3", name: "3" }, rows), "Daily quota limiter");
  eq(f.top.tabName({ id: "t3", name: "" }, rows), "Daily quota limiter");
  eq(f.top.tabName({ id: "t3", name: "quotas" }, rows), "quotas", "a name the person typed");
  eq(f.top.tabName({ id: "t4", name: "4" }, rows), "4", "no agent in it: its number");
  eq(f.top.tabName({ id: "t3", name: "3" }), "3", "no rows yet");
});

test("the highlighted pill follows the chat's terminal, including a floating terminal in another tab", async () => {
  const f = await fixture({ tabs: 3, active: 1, live: [{ address: "a", tab: "t1" }, { address: "b", tab: "t2" }] });
  f.state.set("cockpit.chat.subject", "app:b");
  const selected = f.bar.layoutNow().shown.filter((pill) => pill.on);
  eq(selected.map((pill) => pill.tab.id), ["t2"]);
});

test("exited agents do not give shell tabs an agent icon or chat, and generic labels follow the current provider", async () => {
  const f = await fixture({ tabs: 3, live: [{ address: "stale", tab: "t1" }, { address: "current", tab: "t2" }] });
  const rows = [{ terminal: "app:current", address: "current", tab: { id: "t2" }, agent_name: "Codex" }];
  f.screens.setChatAgents(rows);
  f.bar.rows = rows;
  assert.equal(f.bar.tabAgent({ id: "t1" }), null);
  assert.equal(f.screens.frontAgent({ tab: "t1", terminal: "stale" }), null);
  eq(f.top.tabName({ id: "t1", name: "Claude Code" }, rows), "Terminals");
  eq(f.top.tabName({ id: "t2", name: "Claude Code" }, rows), "Codex");
  eq(f.top.tabName({ id: "t2", name: "Quota review" }, rows), "Quota review");
});
