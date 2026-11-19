// The agent the Cockpit layout's status bar describes, its documents at the bar's right end: the agent in view, never
// the first agent Herdr names for its agent blocks when the tab in front runs none.
import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import { createContext, SourceTextModule, SyntheticModule } from "node:vm";
import { test } from "node:test";

const MODS = new URL("../mods/cockpit/", import.meta.url);

function find(node, id) {
  if (!node || typeof node !== "object") return null;
  if (node.id === id) return node;
  for (const c of node.children ?? []) {
    const found = find(c, id);
    if (found) return found;
  }
  return null;
}

/** Every id a drawn element and its children were given. */
function ids(el, out = []) {
  for (const [name, ...args] of el?.made ?? []) {
    if (name === "id") out.push(args[0]);
    for (const a of args.flat()) if (a?.made) ids(a, out);
  }
  return out;
}

// Herdr's own agents: Enforce quotas in tab A, Flaky e2e in tab B; tab C runs a shell (t6), tab D an agent and a shell.
const LIVE = [
  { address: "t1", tab: "tA", status: "blocked", title: "claude", focused: false },
  { address: "t4", tab: "tB", status: "working", title: "claude", focused: false },
  { address: "t9", tab: "tD", status: "idle", title: "claude", focused: false },
];
const ROWS = {
  agents: [
    { terminal: "app:t1", address: "t1", title: "Enforce daily quotas", branch: "me/eng-412-daily-quotas", documents: [{ name: "quota-plan", title: "Quota rollout plan" }], links: [{ name: "GitHub", label: "GitHub", url: "https://github.com/acme/api/pull/2198" }] },
    { terminal: "app:t4", address: "t4", title: "Flaky checkout e2e test", branch: "me/eng-405-flaky-checkout", documents: [{ name: "flaky-runs", title: "Checkout e2e runs" }] },
    { terminal: "app:t9", address: "t9", title: "Quota model", documents: [] },
  ],
};

async function fixture({ layoutName = "cockpit" } = {}) {
  const tree = JSON.parse(await readFile(new URL(`../layouts/${layoutName}.json`, import.meta.url), "utf8"));
  const state = new Map(Object.entries({ layout: layoutName, "cockpit.app": "work" }));
  const focus = { value: null };
  const element = () => {
    const made = [];
    const el = new Proxy({}, { get: (_t, name) => (name === "made" ? made : (...a) => (made.push([name, ...a]), el)) });
    return el;
  };
  const context = createContext({ console });
  const subscriptions = [];
  const imports = {
    herdr: {
      run: () => null,
      layout: () => tree,
      shared: (key) => state.get(key) ?? null,
      setShared: (key, value) => state.set(key, value),
      focused: () => focus.value,
      agents: () => LIVE,
      spaces: () => [{ id: "s1", active: true, tabs: ["tA", "tB", "tC", "tD"].map((id, i) => ({ id, name: String(i + 1), active: false })) }],
      keys: () => [],
      notify: () => null,
      settings: () => ({}),
      theme: () => ({ foreground: "#ededed", background: "#1e1e1e", hover: "#2a2a2a", muted: "#a0a0a0", dim: "#707070", border: "#333333" }),
    },
    "gpui-kit": { div: element, svg: element },
    "gpui-base": { h_flex: element, v_flex: element },
    "./hub.js": { subscribe: (_cx, request, receive, status) => {
      const sub = { request, receive, status, stopped: false, stop() { this.stopped = true; }, park() {} };
      subscriptions.push(sub);
      return sub;
    }, sleep: async () => {}, isStoppedOnPurpose: () => false, isNotInstalled: () => false, remoteMachine: () => null, downWords: () => "" },
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
  const module = new SourceTextModule('export * as screens from "./screens.js"; export * as fleet from "./fleet.js"; export { default as Status } from "./status.js"; export { default as Show, followed } from "./show.js";', { context });
  await module.link(load);
  await module.evaluate();
  const { screens, fleet, Status, Show, followed } = module.namespace;
  const bar = Object.create(Status.prototype);
  Object.assign(bar, { rows: ROWS, home: null, queue: null, state: "live", width: 1280 });
  const list = () => fleet.fleet(ROWS, null);
  // What the bar draws at its right end: the agent's documents, or none.
  const docs = () => ids(bar.render()).filter((id) => id === "st-docs" || id === "st-branch");
  return { screens, list, bar, docs, tree, focus, state, Show, followed, subscriptions, node: (id) => find(tree, id) };
}

test("the document panel clears on a subject change and ignores a late reply from the previous session", async () => {
  const f = await fixture();
  f.focus.value = { tab: "tA", terminal: "t1" };
  const panel = Object.create(f.Show.prototype);
  Object.assign(panel, { panel: true, terminal: "t1", pick: null, data: { view: { name: "quota-plan" } } });
  const timers = [];
  const cx = { notify() {}, timer: { after: (_delay, fn) => timers.push(fn) } };
  panel.subscribe(cx);
  const old = f.subscriptions[0];
  f.focus.value = null;
  panel.follow(cx);
  assert.equal(panel.view(), null, "the old session disappears immediately, before the next subscription replies");
  assert.equal(f.state.get("badge.show"), null);
  assert.equal(old.stopped, true);
  old.receive({ view: "show", value: { view: { name: "quota-plan" } } }, cx);
  old.status("live", cx);
  assert.equal(panel.view(), null, "a stopped subscription cannot restore another session's document");
  timers.shift()(cx);
  assert.equal(f.subscriptions[1].request.views[0].args.terminal, "", "no focus is an empty scope, never all documents");
  f.focus.value = { tab: "tC", terminal: "t6" };
  panel.follow(cx);
  timers.shift()(cx);
  assert.equal(f.subscriptions[2].request.views[0].args.terminal, "t6", "plain shell documents stay reachable");
});

test("closing a document drawer cancels the subscription queued for its new subject", async () => {
  const f = await fixture();
  const panel = Object.create(f.Show.prototype);
  Object.assign(panel, { panel: true, terminal: "t1", pick: null, data: null });
  const timers = [];
  const cx = { notify() {}, timer: { after: (_delay, fn) => timers.push(fn) } };
  panel.subscribe(cx);
  f.focus.value = { tab: "tB", terminal: "t4" };
  panel.follow(cx);
  panel.close();
  timers.shift()(cx);
  assert.equal(f.subscriptions.length, 1, "a closed drawer cannot start a background subscription");
  assert.equal(f.subscriptions[0].stopped, true);
});

test("a tab running no agent shows no agent's documents, though Herdr names its first agent for the blocks", async () => {
  const f = await fixture();
  f.focus.value = { tab: "tC", pane: "p6", terminal: "t6", agent: "t1", shown: null };
  assert.equal(f.screens.agentInView(f.list()), null);
  assert.deepEqual(f.docs(), [], "no documents, no branch: nothing of Enforce quotas on the shell's tab");
});

test("the documents follow the agent in view from tab to tab", async () => {
  const f = await fixture();
  f.focus.value = { tab: "tA", pane: "p1", terminal: "t1", agent: "t1", shown: null };
  assert.equal(f.screens.agentInView(f.list()).address, "t1");
  assert.deepEqual(f.docs(), ["st-branch", "st-docs"]);
  // Herdr's agent block still names the agent picked before; the tab in front runs Flaky e2e.
  f.focus.value = { tab: "tB", pane: "p4", terminal: "t4", agent: "t1", shown: null };
  assert.equal(f.screens.agentInView(f.list()).address, "t4");
  // A tab with an agent and a shell, the shell focused: the tab's agent is the one in view.
  f.focus.value = { tab: "tD", pane: "p10", terminal: "t10", agent: "t1", shown: null };
  assert.equal(f.screens.agentInView(f.list()).address, "t9");
  assert.deepEqual(f.docs(), ["st-branch"], "Quota model showed nothing beside itself");
});

test("an agent block on screen shows its agent: the lone agent view, and Home's agent column", async () => {
  const f = await fixture();
  // The work screen on its panes: the agent block is behind, and a shell's tab is in front.
  f.node("center").active = "work";
  f.focus.value = { tab: "tC", pane: "p6", terminal: "t6", agent: "t4", shown: null };
  assert.equal(f.screens.agentInView(f.list()), null);
  f.node("view").active = "agent";
  assert.equal(f.screens.agentInView(f.list()).address, "t4", "the lone agent view shows that agent");
  f.node("view").active = "main";
  f.node("center").active = "home";
  f.node("home-agent").hidden = false;
  assert.equal(f.screens.agentInView(f.list()).address, "t4", "Home's agent column too");
  f.node("home-agent").hidden = true;
  assert.equal(f.screens.agentInView(f.list()), null);
});

test("the side panel lists the documents of the session in view: its agent's, a plain shell's own, or the chat's subject's", async () => {
  const f = await fixture();
  f.focus.value = { tab: "tA", pane: "p1", terminal: "t1", agent: "t1", shown: null };
  assert.equal(f.followed(), "t1");
  f.focus.value = { tab: "tD", pane: "p10", terminal: "t10", agent: "t1", shown: null };
  assert.equal(f.followed(), "t9", "the agent of a tab whose shell is focused, as the status bar names it");
  f.focus.value = { tab: "tC", pane: "p6", terminal: "t6", agent: "t1", shown: null };
  assert.equal(f.followed(), "t6", "a tab no agent runs in: the shell's own views, never Herdr's first agent's");
  const chat = await fixture({ layoutName: "cockpit-chat" });
  chat.focus.value = { tab: "tC", pane: "p6", terminal: "t6", agent: "t1", shown: null };
  chat.state.set("cockpit.chat.subject", "t4");
  assert.equal(chat.followed(), "t4", "Cockpit Chat: the documents of the agent the chat reads");
  chat.state.set("cockpit.chat.subject", null);
  assert.equal(chat.followed(), "t6");
});
