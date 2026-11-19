// The browser beside the agent in Cockpit Chat: a terminal-browser split right of the agent's terminal shows in the side
// panel, its tab zoomed on it, while the chat stays in front; the Cockpit layout keeps the tab's panes as they are.
import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import { createContext, SourceTextModule, SyntheticModule } from "node:vm";
import { test } from "node:test";

const MODS = new URL("../mods/cockpit/", import.meta.url);

const program = (key, name = key) => ({ key, name, icon: null, tint: false, tile: null, color: null, shell: false, agent: key === "claude" });
const AGENT = { pane: "p62d2", terminal: "t2", tab: "tb1", title: "claude", program: program("claude", "Claude Code"), zoomed: false };
const BROWSER_PANE = { pane: "pe496", terminal: "t3", tab: "tb1", title: "Acme preview", program: program("program", "terminal-browser"), zoomed: false };

async function load(layoutName) {
  const tree = JSON.parse(await readFile(new URL(`../layouts/${layoutName}.json`, import.meta.url), "utf8"));
  const state = new Map(Object.entries({ layout: layoutName }));
  const ran = [];
  const world = { panes: [], focus: null };
  const find = (node, id) => {
    if (!node || typeof node !== "object") return null;
    if (node.id === id) return node;
    for (const c of [...(node.children ?? []), ...(node.floating ?? [])]) {
      const found = find(c, id);
      if (found) return found;
    }
    return null;
  };
  const imports = {
    herdr: {
      run: (name, args) => {
        ran.push([name, args]);
        if (name === "layout.select") find(tree, args.stack).active = args.child;
        if (name === "layout.show" || name === "layout.hide") find(tree, args.id).hidden = name === "layout.hide";
        return null;
      },
      layout: () => tree,
      shared: (key) => state.get(key) ?? null,
      setShared: (key, value) => state.set(key, value),
      panes: () => world.panes,
      focused: () => world.focus,
    },
    "./hub.js": { subscribe: () => ({ stop() {} }), act: async () => ({}), sleep: async () => {}, remoteMachine: () => null, isNotInstalled: () => false, isStoppedOnPurpose: () => false, downWords: () => "" },
  };
  const context = createContext({ console });
  const sources = new Map();
  const modules = new Map();
  for (const name of await readdir(MODS)) {
    if (!name.endsWith(".js")) continue;
    const text = await readFile(new URL(name, MODS), "utf8");
    sources.set(`./${name}`, text);
    for (const match of text.matchAll(/import\s*\{([^}]+)\}\s*from\s*"(herdr|gpui-kit|gpui-base|gpui-components)"/g)) {
      const exports = (imports[match[2]] ??= {});
      for (const word of match[1].split(",")) exports[word.trim().split(/\s+as\s+/)[0]] ??= () => null;
    }
    for (const match of text.matchAll(/import\s+(\w+)\s*,?\s*(?:\{[^}]*\})?\s*from\s*"(herdr|gpui-kit|gpui-base|gpui-components)"/g)) {
      (imports[match[2]] ??= {}).default ??= class {};
    }
  }
  const linker = (name) => {
    if (modules.has(name)) return modules.get(name);
    const native = name.startsWith("./") && !imports[name] ? null : imports[name];
    const module = native
      ? new SyntheticModule(Object.keys(native), function () {
        for (const [key, value] of Object.entries(native)) this.setExport(key, value);
      }, { context })
      : new SourceTextModule(sources.get(name), { context, identifier: name });
    modules.set(name, module);
    return module;
  };
  const module = new SourceTextModule('export * as browser from "./browser.js";', { context });
  await module.link(linker);
  await module.evaluate();
  return { browser: module.namespace.browser, tree, state, ran, world, node: (id) => find(tree, id) };
}

/** A value made in the mod's own realm, as plain data. */
const plain = (v) => JSON.parse(JSON.stringify(v));

const base = { inChat: true, onTab: false, list: [AGENT, BROWSER_PANE], tab: "tb1", focusedPane: "p62d2", panel: false, child: "show", zoomed: null, away: false, seen: [] };

test("a browser split beside the agent opens the panel on it and zooms its tab on it", async () => {
  const { browser } = await load("cockpit-chat");
  assert.deepEqual(plain(browser.browserSteps(base)), [["seen", "pe496"], ["open"], ["zoom", "pe496"], ["track", "pe496"]]);
  assert.equal(browser.isBrowserPane(AGENT), false);
  assert.equal(browser.browserIn([AGENT, BROWSER_PANE], "tb2"), null, "another tab's browser is not this agent's");
});

test("a panel the person closed stays closed, and the tab gets its panes and its focus back", async () => {
  const { browser } = await load("cockpit-chat");
  const list = [AGENT, { ...BROWSER_PANE, zoomed: true }];
  const closed = { ...base, list, focusedPane: "pe496", panel: false, child: "browser", zoomed: "pe496", seen: ["pe496"] };
  assert.deepEqual(plain(browser.browserSteps(closed)), [["unzoom", "pe496"], ["focus", "p62d2"], ["track", null]]);
  const settled = { ...closed, list: [AGENT, BROWSER_PANE], focusedPane: "p62d2", zoomed: null };
  assert.deepEqual(plain(browser.browserSteps(settled)), [], "nothing to do once it is put back");
  assert.deepEqual(plain(browser.browserSteps({ ...settled, panel: true })), [["zoom", "pe496"], ["track", "pe496"]], "reopened on the browser, it zooms again");
});

test("something focusing the agent in the zoomed tab hands the panel back to the browser", async () => {
  const { browser } = await load("cockpit-chat");
  const list = [{ ...AGENT, zoomed: true }, BROWSER_PANE];
  const state = { ...base, list, panel: true, child: "browser", zoomed: "pe496", seen: ["pe496"] };
  assert.deepEqual(plain(browser.browserSteps(state)), [["focus", "pe496"]]);
  assert.deepEqual(plain(browser.browserSteps({ ...state, focusedPane: "pe496" })), [], "steady while it shows");
});

test("the Tab screen takes the browser back from the panel, and the chat gives it to the panel again", async () => {
  const { browser } = await load("cockpit-chat");
  const list = [AGENT, { ...BROWSER_PANE, zoomed: true }];
  const onTab = { ...base, inChat: false, onTab: true, list, focusedPane: "pe496", panel: true, child: "browser", zoomed: "pe496", seen: ["pe496"] };
  assert.deepEqual(plain(browser.browserSteps(onTab)), [["close"], ["away", true], ["unzoom", "pe496"], ["focus", "p62d2"], ["track", null]]);
  const back = { ...base, panel: false, child: "browser", away: true, seen: ["pe496"] };
  assert.deepEqual(plain(browser.browserSteps(back)), [["open"], ["away", false], ["zoom", "pe496"], ["track", "pe496"]]);
});

test("the browser closing closes its panel and forgets the zoom", async () => {
  const { browser } = await load("cockpit-chat");
  const state = { ...base, list: [AGENT], panel: true, child: "browser", zoomed: "pe496", seen: ["pe496"] };
  assert.deepEqual(plain(browser.browserSteps(state)), [["close"], ["track", null]]);
  assert.deepEqual(plain(browser.browserSteps({ ...state, child: "show" })), [["track", null]], "another panel stays");
});

test("in the window: the panel opens wide enough on the browser, and leaving the layout unzooms", async () => {
  const f = await load("cockpit-chat");
  f.world.panes = [AGENT, BROWSER_PANE];
  f.world.focus = { tab: "tb1", pane: "p62d2", terminal: "t2" };
  f.node("screen").active = "work";
  f.node("view").active = "chat";
  assert.equal(f.browser.syncBrowser(1000), true);
  const names = f.ran.map(([name]) => name);
  assert.deepEqual(plain(f.ran.find(([n]) => n === "layout.resize")), ["layout.resize", { id: "panel", px: 600 }]);
  assert.ok(names.includes("layout.show") && f.node("panel-body").active === "browser", names.join(" "));
  assert.deepEqual(plain(f.ran.at(-1)), ["pane.zoom", { pane: "pe496" }]);
  assert.equal(f.state.get(f.browser.ZOOMED), "pe496");
  f.ran.length = 0;
  assert.equal(f.browser.syncBrowser(1200), false, "the window answers the zoom before the next step");
  f.world.panes = [{ ...AGENT, zoomed: true }, { ...BROWSER_PANE, zoomed: true }];
  f.world.focus = { tab: "tb1", pane: "pe496", terminal: "t3" };
  f.state.set("layout", "cockpit");
  f.browser.releaseBrowser();
  assert.deepEqual(plain(f.ran), [["pane.zoom", { pane: "pe496" }], ["pane.focus", { pane: "p62d2" }]]);
  assert.equal(f.state.get(f.browser.ZOOMED), null);
});

test("the cockpit layout has no browser panel and never zooms a browser", async () => {
  const f = await load("cockpit");
  f.state.set("layout", "cockpit");
  f.world.panes = [AGENT, BROWSER_PANE];
  f.world.focus = { tab: "tb1", pane: "p62d2", terminal: "t2" };
  assert.equal(f.node("browser"), null);
  assert.equal(f.browser.syncBrowser(5000), false);
  assert.deepEqual(f.ran, []);
});

test("the chat's box takes the keyboard back once the panel's move landed, once, and never long after", async () => {
  const { browser } = await load("cockpit-chat");
  const at = { took: 10_000, answered: null, now: 10_400 };
  assert.equal(browser.boxTakesBack(at), true);
  assert.equal(browser.boxTakesBack({ ...at, answered: 10_000 }), false, "answered already");
  assert.equal(browser.boxTakesBack({ ...at, now: 10_100 }), false, "the move has not landed yet");
  assert.equal(browser.boxTakesBack({ ...at, now: 13_000 }), false, "a later click of the person's stands");
  assert.equal(browser.boxTakesBack({ ...at, took: null }), false);
});
