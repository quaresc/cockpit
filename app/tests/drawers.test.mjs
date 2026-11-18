// Cockpit Chat's two edges: the sidebar and the side panel, each open or put away behind its drawer, on a stand-in
// Herdr that applies the layout commands to the shipped layout file.
import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import { createContext, SourceTextModule, SyntheticModule } from "node:vm";
import { test } from "node:test";

const MODS = new URL("../mods/cockpit/", import.meta.url);

/** A node of a layout tree by id (the first one: the reading layout names two nodes `work`). */
function find(node, id) {
  if (!node || typeof node !== "object") return null;
  if (node.id === id) return node;
  for (const c of [...(node.children ?? []), ...(node.floating ?? [])]) {
    const found = find(c, id);
    if (found) return found;
  }
  return null;
}

async function fixture({ layoutName = "cockpit-chat", shared: given = {} } = {}) {
  const tree = JSON.parse(await readFile(new URL(`../layouts/${layoutName}.json`, import.meta.url), "utf8"));
  const calls = [];
  const state = new Map(Object.entries({ layout: layoutName, "cockpit.app": "chat", ...given }));
  // Herdr's layout commands, as the window applies them to its tree; a resize keeps the size, whatever the node's min.
  const run = (name, args = {}) => {
    calls.push([name, args]);
    const node = args.id ? find(tree, args.id) : null;
    if (name === "layout.hide" && node) node.hidden = true;
    if (name === "layout.show" && node) node.hidden = false;
    if (name === "layout.resize" && node) node.size = { px: args.px };
    if (name === "layout.select") {
      const stack = find(tree, args.stack);
      if (stack) stack.active = args.child;
    }
    return null;
  };
  const element = () => {
    const made = [];
    const el = new Proxy({}, { get: (_t, name) => (name === "made" ? made : (...a) => (made.push([name, ...a]), el)) });
    return el;
  };
  const context = createContext({ console });
  const imports = {
    herdr: {
      run,
      layout: () => tree,
      shared: (key) => state.get(key) ?? null,
      setShared: (key, value) => state.set(key, value),
      spaces: () => [],
      agents: () => [],
      focused: () => null,
      panes: () => [],
      notify: () => null,
      keys: () => [],
      window: () => ({ platform: "macos", titlebar: "none", fullscreen: false }),
      theme: () => ({ foreground: "#ededed", background: "#1e1e1e", hover: "#2a2a2a", muted: "#a0a0a0", dim: "#707070", border: "#333333" }),
    },
    "gpui-kit": { div: element, svg: element },
    "gpui-base": { h_flex: element, v_flex: element },
    "./hub.js": { subscribe: () => ({ stop() {}, park() {} }), sleep: async () => {}, act: async () => ({}), view: async () => null, remoteMachine: () => null },
    // The drawer's own `show`, standing for the real view: only whether it is held matters here.
    "./show.js": { default: class { init() {} close() {} tick() {} render() { return element(); } } },
  };
  const sources = new Map(), modules = new Map();
  for (const name of await readdir(MODS)) {
    if (!name.endsWith(".js")) continue;
    const text = await readFile(new URL(name, MODS), "utf8");
    sources.set(`./${name}`, text);
    for (const match of text.matchAll(/import\s*\{([^}]+)\}\s*from\s*"(herdr|gpui-kit|gpui-base)"/g)) {
      const exports = (imports[match[2]] ??= {});
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
  const module = new SourceTextModule('export * as screens from "./screens.js"; export * as top from "./chat-top.js"; export * as commands from "./commands.js"; export { default as SidePeek } from "./side-peek.js"; export { default as PanelPeek } from "./panel-peek.js"; export { default as Code, openCode } from "./code.js";', { context });
  await module.link(load);
  await module.evaluate();
  return { ...module.namespace, tree, state, calls, node: (id) => find(tree, id) };
}

test("closing a file restores the panel the layout actually showed, even with a stale shared flag", async () => {
  for (const [open, flag] of [[true, false], [false, true]]) {
    const f = await fixture({ shared: { "cockpit.panel.open": flag } });
    f.node("panel").hidden = !open;
    f.node("panel-body").active = "show";
    await f.openCode("/acme/api/README.md");
    await f.openCode("/acme/api/notes.md");
    const code = Object.create(f.Code.prototype);
    code.panel = true;
    code.closePane();
    assert.equal(!f.node("panel").hidden, open, "Close restores the actual original visibility across repeated files");
    if (open) assert.equal(f.node("panel-body").active, "show");
  }
});

test("Herdr's brand row folding the sidebar to a rail puts it away behind its drawer, at its width", async () => {
  const f = await fixture();
  // The brand row's button: the width it had kept, then a rail (on macOS wide enough for the traffic lights).
  f.state.set("sidebar.width", 300);
  f.state.set("sidebar.collapsed", true);
  f.node("sidebar").size = { px: 76 };
  assert.equal(f.screens.settleSidebar(), "folded");
  assert.equal(f.node("sidebar").hidden, true, "put away: the chat takes the whole window");
  assert.equal(f.node("sidebar").size.px, 300, "it comes back at the width it had");
  assert.equal(f.state.get("cockpit.chat.sidebar"), false, "the drawer on the edge stands in for it");
  assert.equal(f.state.get("sidebar.collapsed"), false);
  assert.equal(f.screens.settleSidebar(), null, "settled: nothing more to do");
  // Pinned again from the drawer, it opens at that width.
  f.screens.showSidebar(true);
  assert.equal(f.node("sidebar").hidden, false);
  assert.equal(f.node("sidebar").size.px, 300);
});

test("a fold to a rail never leaves a sidebar narrower than this layout's minimum, whatever width was kept", async () => {
  for (const [kept, back] of [[null, 268], [90, 220], [900, 480]]) {
    const f = await fixture();
    f.state.set("sidebar.width", kept);
    f.node("sidebar").size = { px: 76 };
    f.screens.settleSidebar();
    assert.equal(f.node("sidebar").size.px, back, `kept ${kept}`);
  }
});

test("the flag follows the layout: hidden or shown from elsewhere, the drawer and the bar read the layout", async () => {
  const f = await fixture({ shared: { "cockpit.chat.sidebar": true } });
  f.node("sidebar").hidden = true;
  assert.equal(f.screens.sidebarShown(), false);
  assert.equal(f.screens.settleSidebar(), "flag");
  assert.equal(f.state.get("cockpit.chat.sidebar"), false);
  assert.ok(f.top.lightsRoom() > 12, "the traffic lights get room on the bar once the sidebar is away");
  f.node("sidebar").hidden = false;
  assert.equal(f.screens.settleSidebar(), "flag");
  assert.equal(f.state.get("cockpit.chat.sidebar"), true);
  assert.equal(f.top.lightsRoom(), 12);
});

test("⌘B toggles what the window shows, even when the flag says otherwise", async () => {
  const f = await fixture({ shared: { "cockpit.chat.sidebar": false } });
  // The layout shows the sidebar although the flag says it is away (a click on the layout, a restart).
  f.node("sidebar").hidden = false;
  f.screens.toggleSidebar();
  assert.equal(f.node("sidebar").hidden, true);
  assert.equal(f.state.get("cockpit.chat.sidebar"), false);
  f.screens.toggleSidebar();
  assert.equal(f.node("sidebar").hidden, false);
  assert.equal(f.state.get("cockpit.chat.sidebar"), true);
});

test("the left drawer draws nothing while the sidebar is in the window, whatever the flag says", async () => {
  const f = await fixture({ shared: { "cockpit.chat.sidebar": false } });
  const peek = Object.create(f.SidePeek.prototype);
  f.node("sidebar").hidden = false;
  assert.equal(peek.render().made.length, 0, "a stale flag never draws the drawer over an open sidebar");
  f.screens.settleSidebar();
  assert.equal(f.state.get("cockpit.chat.sidebar"), true, "and the flag follows the layout");
});

test("the Cockpit layout's sidebar is never settled: it folds to its own rail", async () => {
  const f = await fixture({ layoutName: "cockpit" });
  f.node("sidebar").size = { px: 76 };
  assert.equal(f.screens.settleSidebar(), null);
  assert.equal(f.node("sidebar").hidden, undefined);
});

test("the right drawer is there from the first frame when this layout's panel is closed, whatever the flag says", async () => {
  // The Cockpit layout left its own panel open: the flag says open, the reading layout's panel is closed.
  const f = await fixture({ shared: { "cockpit.panel.open": true, "cockpit.panel.child": "show" } });
  assert.equal(f.node("panel").hidden, true);
  const peek = Object.create(f.PanelPeek.prototype);
  peek.show = null;
  peek.sync({ notify() {} });
  assert.ok(peek.show, "the drawer holds its view from the start");
  assert.equal(f.commands.settlePanel(), true);
  assert.equal(f.state.get("cockpit.panel.open"), false, "the flag follows the layout");
  // Pinned open from the drawer, the drawer stands down.
  f.commands.openPanel(null);
  assert.equal(f.node("panel").hidden, false);
  peek.sync({ notify() {} });
  assert.equal(peek.show, null);
});

test("an open panel the flag calls closed has no drawer over it, and ⌘⌥B closes it at the first press", async () => {
  const f = await fixture({ shared: { "cockpit.panel.open": false } });
  f.node("panel").hidden = false;
  const peek = Object.create(f.PanelPeek.prototype);
  peek.show = null;
  peek.sync({ notify() {} });
  assert.equal(peek.show, null);
  f.commands.togglePanel(null);
  assert.equal(f.node("panel").hidden, true);
  assert.equal(f.state.get("cockpit.panel.open"), false);
});

test("⌘⌥B opens a closed panel at the first press, even when the other layout left the flag open", async () => {
  const f = await fixture({ shared: { "cockpit.panel.open": true, "cockpit.panel.child": "review" } });
  f.commands.togglePanel(null);
  assert.equal(f.node("panel").hidden, false, "opened, not closed again");
  // On another child it switches to it; on the one it shows it closes.
  f.commands.togglePanel("files");
  assert.equal(f.node("panel-body").active, "files");
  assert.equal(f.node("panel").hidden, false);
  f.commands.togglePanel("files");
  assert.equal(f.node("panel").hidden, true);
});

test("the agent drawer lists working agents before a long idle list, after agents that need attention", async () => {
  const f = await fixture();
  f.node("sidebar").hidden = true;
  const drawer = Object.create(f.SidePeek.prototype);
  const row = (id, stage, status = stage) => ({ terminal: `app:${id}`, address: id, title: id, stage, status });
  drawer.rows = { agents: [
    ...Array.from({ length: 10 }, (_, i) => row(`idle-${i}`, "idle")),
    row("done-1", "done"), row("working-1", "working"), row("waiting-1", "needs_you", "blocked"),
  ] };
  drawer.update = null;
  const ids = [];
  const visit = (element) => {
    for (const [method, ...args] of element?.made ?? []) {
      if (method === "id") ids.push(args[0]);
      for (const child of args.flat()) if (child?.made) visit(child);
    }
  };
  visit(drawer.render());
  const groups = ids.filter((id) => id.startsWith("peek-stage-"));
  assert.deepEqual(groups, ["peek-stage-needs_you", "peek-stage-working", "peek-stage-done", "peek-stage-idle"]);
  for (const agent of drawer.rows.agents) assert.ok(ids.includes(`peek-${agent.terminal}`), `${agent.address} stays listed`);
});
