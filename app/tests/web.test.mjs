// The Chromium side browser's slot: where it says the browser's window goes, when it shows, whose browser it is, and
// how the side panel follows the session in front, in both layouts.
import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import { createContext, SourceTextModule, SyntheticModule } from "node:vm";
import { test } from "node:test";

const MODS = new URL("../mods/cockpit/", import.meta.url);

async function load(layoutName) {
  const tree = JSON.parse(await readFile(new URL(`../layouts/${layoutName}.json`, import.meta.url), "utf8"));
  const state = new Map(Object.entries({ layout: layoutName }));
  const ran = [];
  const sent = [];
  const world = { focus: null, viewport: { width: 1280, height: 820 } };
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
        if (name === "layout.resize") find(tree, args.id).size = { px: args.px };
        return null;
      },
      layout: () => tree,
      shared: (key) => state.get(key) ?? null,
      setShared: (key, value) => state.set(key, value),
      focused: () => world.focus,
      appHome: () => "/home/maya/.config/herdr-app",
      theme: () => ({ background: "#101114", foreground: "#e8e8e6", chrome: "#16171b", border: "#2a2b30", muted: "#8a8b90", accent: "#a3b3e0", hover: "rgba(255,255,255,0.06)" }),
    },
    "./hub.js": {
      subscribe: () => ({ stop() {} }),
      act: async (app, action, args) => (sent.push([app, action, args?.[0] ?? null]), {}),
      sleep: async () => {},
      remoteMachine: () => null,
      isNotInstalled: () => false,
      isStoppedOnPurpose: () => false,
      downWords: () => "",
    },
  };
  const context = createContext({ console, setTimeout, window: { viewport_size: () => world.viewport } });
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
  const module = new SourceTextModule('export * as web from "./web.js"; export { default as Web } from "./web.js";', { context });
  await module.link(linker);
  await module.evaluate();
  return { web: module.namespace.web, Web: module.namespace.Web, tree, state, ran, sent, world, context, node: (id) => find(tree, id) };
}

/** A value made in the mod's own realm, as plain data. */
const plain = (v) => JSON.parse(JSON.stringify(v));

test("both layouts have the slot in their side panel", async () => {
  for (const name of ["cockpit", "cockpit-chat"]) {
    const { node, web } = await load(name);
    assert.equal(node("panel-body").children.some((c) => c.id === web.WEB), true, name);
    assert.deepEqual(node(web.WEB).content, { mod: "cockpit", props: { app: "web" } });
    assert.ok(web.INSETS[name], `${name} has its insets`);
  }
});

test("the slot is placed from the window's far edges, by layout", async () => {
  const { web } = await load("cockpit");
  const size = { width: 594, height: 726 };
  assert.deepEqual(plain(web.slotRect({ width: 1280, height: 820 }, size, web.INSETS.cockpit)), { x: 633, y: 51, width: 594, height: 726 });
  assert.deepEqual(plain(web.slotRect({ width: 1280, height: 820 }, { width: 594, height: 746 }, web.INSETS["cockpit-chat"])), { x: 633, y: 65, width: 594, height: 746 });
  assert.equal(web.slotRect({ width: 1280, height: 820 }, null, web.INSETS.cockpit), null, "no size yet, no slot");
  assert.deepEqual(plain(web.insetsOf({ width: 1280, height: 820 }, { x: 631, y: 51, width: 594, height: 726 })), { right: 55, bottom: 43 }, "a pointer over it measures them");
});

test("the slot shows only with the panel open on it and the agent's screen in front", async () => {
  const { web, tree, node } = await load("cockpit");
  assert.equal(web.slotShown(tree, "cockpit"), false, "the panel starts hidden");
  node("panel").hidden = false;
  node("panel-body").active = web.WEB;
  node("center").active = "work";
  assert.equal(web.slotShown(tree, "cockpit"), true);
  node("center").active = "inbox";
  assert.equal(web.slotShown(tree, "cockpit"), false, "another screen in front");
  node("center").active = "work";
  node("panel-body").active = "show";
  assert.equal(web.slotShown(tree, "cockpit"), false, "the panel on another child");
});

test("a page an agent opens brings the panel to the browser, for the session in front only", async () => {
  const { web } = await load("cockpit-chat");
  const f = { terminal: "t3", cwd: "/work/acme/web-eng-412/apps/site" };
  const opened = { at: 2000, owner: "app:t3" };
  assert.equal(web.wantsPanel({ opened, seen: 1000, f, showing: false }), true);
  assert.equal(web.wantsPanel({ opened, seen: 2000, f, showing: false }), false, "old news");
  assert.equal(web.wantsPanel({ opened, seen: 1000, f, showing: true }), false, "already showing");
  assert.equal(web.wantsPanel({ opened: { ...opened, owner: "app:t7" }, seen: 1000, f, showing: false }), false, "another session's page");
  assert.equal(web.wantsPanel({ opened: { at: 2000 }, seen: 1000, f, showing: false }), false, "a page with no session");
  assert.equal(web.wantsPanel({ opened, seen: 1000, f: null, showing: false }), false, "no session in front");
});

test("the panel leaves with a session that has no browser and comes back with one that has", async () => {
  const { web } = await load("cockpit");
  const plain = (v) => JSON.parse(JSON.stringify(v));
  const model = { tabs: { "app:t3": 2 } };
  assert.equal(web.tabsOf(model, "t3"), 2);
  assert.equal(web.tabsOf(model, "t7"), 0);
  assert.equal(web.tabsOf(model, null), 0);
  assert.deepEqual(plain(web.panelSteps({ terminal: "t3", last: "t3", showing: true, away: false, count: 0 })), [], "the same session stays in front");
  assert.deepEqual(plain(web.panelSteps({ terminal: "t7", last: "t3", showing: true, away: false, count: 0 })), [["close"], ["away", true]]);
  assert.deepEqual(plain(web.panelSteps({ terminal: "t3", last: "t7", showing: false, away: true, count: 2 })), [["open"], ["away", false]]);
  assert.deepEqual(plain(web.panelSteps({ terminal: "t3", last: "t7", showing: false, away: false, count: 2 })), [], "a panel the person closed stays closed");
  assert.deepEqual(plain(web.panelSteps({ terminal: "t9", last: "t3", showing: true, away: false, count: 1 })), [], "from one browser to another, it stays open");
  assert.deepEqual(plain(web.panelSteps({ terminal: "t3", last: "t7", showing: false, away: false, count: 1, unseen: true })), [["open"], ["away", false]], "its agent opened a page while it was away");
  assert.deepEqual(plain(web.panelSteps({ terminal: "t7", last: "t3", showing: false, away: false, count: 0 })), []);
});

test("opening the panel on the browser widens it for a page", async () => {
  const { web, ran, node } = await load("cockpit");
  web.openWeb();
  assert.deepEqual(plain(ran[0]), ["layout.resize", { id: "panel", px: 640 }]);
  assert.equal(node("panel-body").active, web.WEB);
  assert.equal(node("panel").hidden, false);
});

test("a popup of cockpit's covers the slot while it lives", async () => {
  const { web } = await load("cockpit");
  const now = 100_000;
  assert.equal(web.covered(now, [{ app: "web", overlay: false, beat: now }]), false, "the slot itself is no cover");
  assert.equal(web.covered(now, [{ app: "palette", overlay: true, beat: now - 500 }]), true);
  assert.equal(web.covered(now, [{ app: "palette", overlay: true, beat: now - 5000 }]), false, "a popup that stopped beating is gone");
  assert.equal(web.covered(now), false, "nothing open");
});

test("the toolbar takes the theme's colours that are plain hex", async () => {
  const { web } = await load("cockpit");
  const theme = web.toolbarTheme({ chrome: "#16171b", background: "#101114", foreground: "#e8e8e6", mutedFg: "#8a8b90", border: "#2a2b30", ring: "#a3b3e0", accent: "rgba(255,255,255,0.06)" });
  assert.deepEqual(plain(theme), { bg: "#16171b", panel: "#101114", fg: "#e8e8e6", muted: "#8a8b90", line: "#2a2b30", accent: "#a3b3e0" });
});

test("the slot is told once per change, one request at a time, with the session in front", async () => {
  const { web, Web, sent, node, state, world, context } = await load("cockpit");
  const view = new Web();
  view.init({}, { notify() {} });
  const shared = web.followState();
  shared.model = { enabled: true, running: true, run: 1 };
  state.set("cockpit.web.size", { width: 594, height: 726 });
  world.focus = { terminal: "t3", cwd: "/work/acme/web-eng-412" };
  node("panel").hidden = false;
  node("panel-body").active = "web";
  node("center").active = "work";
  web.reportSlot();
  web.reportSlot();
  assert.equal(sent.length, 1, "nothing changed, nothing sent");
  assert.equal(sent[0][1], "slot");
  assert.deepEqual(plain(sent[0][2].rect), { x: 633, y: 51, width: 594, height: 726 });
  assert.deepEqual([sent[0][2].visible, sent[0][2].terminal, sent[0][2].seq], [true, "t3", 1]);
  state.set("cockpit.web.size", { width: 594, height: 700 });
  web.reportSlot();
  assert.equal(sent.length, 1, "the first request is still out");
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(sent.length, 2, "the newest slot goes once the first has landed");
  assert.equal(sent[1][2].seq, 2);
  // The panel closing draws no frame of the slot: it is told all the same, from whoever looks.
  node("panel").hidden = true;
  web.reportSlot();
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(sent.at(-1)[2].visible, false);
  node("panel").hidden = false;
  world.focus = { terminal: "t7", cwd: "/work/acme/api" };
  web.reportSlot();
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.deepEqual([sent.at(-1)[2].visible, sent.at(-1)[2].terminal], [true, "t7"], "another session in front is told too");
  state.set("cockpit.web.insets", { cockpit: { right: 55, bottom: 43 } });
  web.reportSlot();
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(plain(sent.at(-1)[2].rect).x, 631, "measured insets correct the table");
  const told = sent.length;
  shared.model = { enabled: false, running: true };
  state.set("cockpit.web.size", { width: 500, height: 500 });
  web.reportSlot();
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(sent.length, told, "off, it tells nothing");
});

test("the panel follows the session in front", async () => {
  const { web, node, state, world, context } = await load("cockpit");
  const shared = web.followState();
  shared.model = { enabled: true, running: true, tabs: { "app:t3": 1 } };
  node("panel").hidden = false;
  node("panel-body").active = "web";
  node("center").active = "work";
  world.focus = { terminal: "t3" };
  assert.equal(web.followPanel(), false, "the first look only notes who is in front");
  world.focus = { terminal: "t7" };
  assert.equal(web.followPanel(), true);
  assert.equal(node("panel").hidden, true, "a session with no browser takes the panel away");
  assert.equal(state.get("cockpit.web.away"), true);
  world.focus = { terminal: "t3" };
  assert.equal(web.followPanel(), true);
  assert.deepEqual([node("panel").hidden, node("panel-body").active, state.get("cockpit.web.away")], [false, "web", false]);
});
