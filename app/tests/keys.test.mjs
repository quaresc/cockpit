import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import { createContext, SourceTextModule, SyntheticModule } from "node:vm";
import { test } from "node:test";

const app = new URL("../", import.meta.url);
const folder = new URL("mods/cockpit/", app);
const read = (url) => readFile(url, "utf8");
const json = async (url) => JSON.parse(await read(url));

/** A layout tree whose stacks show these screens, as Herdr's layout() reads (front.js's shownScreen). */
const tree = (stack, active, inner = null) => ({ id: "root", children: [{ id: stack, active, children: [] }, ...(inner ? [{ id: "view", active: inner, children: [] }] : [])] });

// The mod loaded in a VM with Herdr's natives stubbed: `shared` holds the layout, `bound` is what keys() returns, `shown`
// the layout tree layout() returns, `hub` the views the hub answers; `calls` records every command run and hub act.
async function fixture({ layout = "cockpit", bound = [], agents = [], focused = {}, hub = {}, shown = null, app: appShown = null, terms = null } = {}) {
  const calls = [];
  const views = [];
  const shared = { layout, "cockpit.app": appShown };
  const context = createContext({ console });
  const imports = {
    herdr: {
      keys: () => bound,
      commands: () => [{ name: "pane.split", args: [{ name: "direction" }] }, { name: "tab.select", args: [{ name: "index" }] }],
      agents: () => agents,
      spaces: () => [],
      layouts: () => [],
      window: () => ({ platform: "macos" }),
      layout: () => shown,
      focused: () => focused,
      run: (...args) => calls.push(args),
      openOverlay: (spec) => calls.push(["overlay", spec]),
      shared: (key) => shared[key] ?? null,
      setShared: (key, value) => (shared[key] = value),
      notify: (n) => calls.push(["notify", n]),
      ...(terms ? { terminals: () => terms } : {}),
    },
    "./hub.js": {
      view: async (name, args) => (views.push([name, args]), hub[name] ?? null),
      act: async (...args) => (calls.push(["act", ...args]), {}),
      openUrl: () => {},
      install: async () => ({}),
      sleep: async () => {},
      subscribe: () => ({ stop() {} }),
      remoteMachine: () => null,
      LOAD: 1,
      liveLoad: () => true,
    },
  };
  const sources = new Map(), modules = new Map();
  for (const name of await readdir(folder)) {
    if (!name.endsWith(".js")) continue;
    const text = await read(new URL(name, folder));
    sources.set(`./${name}`, text);
    for (const match of text.matchAll(/import\s*\{([^}]+)\}\s*from\s*"(herdr|gpui-kit|gpui-base)"/g)) {
      const exports = (imports[match[2]] ??= {});
      for (const word of match[1].split(",")) {
        const key = word.trim().split(/\s+as\s+/)[0];
        exports[key] ??= () => {
          throw new Error(`unexpected native call: ${key}`);
        };
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
  const entry = [
    'export * from "./commands.js";',
    'export { emptyRows, keepSelection, PALETTE_KEYS, default as Palette } from "./palette.js";',
    'export { default as Sidebar, takesAgentCommands } from "./sidebar.js";',
    'export { baseOf, screens, screensOf, layoutName } from "./screens.js";',
  ].join(" ");
  const module = new SourceTextModule(entry, { context });
  await module.link(load);
  await module.evaluate();
  return { mod: module.namespace, calls, shared, views };
}

// The VM's objects as plain data, so they compare with this realm's.
const plain = (value) => JSON.parse(JSON.stringify(value));

/** A context whose timers run at once and whose spawned work can be awaited. */
function context() {
  const work = [];
  const cx = { notify() {}, stop_propagation() {}, spawn: (f) => work.push(f(cx)), timer: { after: (_ms, f) => f(cx), every: () => {} } };
  return { cx, settled: () => Promise.all(work) };
}

/** keys.json as Herdr's keys() lists it, with Herdr's global keys this test leans on. */
async function bound() {
  const keys = await json(new URL("keys.json", app));
  const global = { "cmd-/": "app.help", "cmd-b": "layout.toggle:sidebar", "cmd-n": "space.new", "cmd-shift-p": "app.palette" };
  return [...Object.entries({ global, ...keys }).flatMap(([section, map]) => Object.entries(map).map(([k, action]) => ({ section, keys: k, action })))];
}

// ---------------------------------------------------------------------------------------------------------------------
// What each view handles, read from its source

const sources = async () => {
  const out = new Map();
  for (const name of await readdir(folder)) if (name.endsWith(".js")) out.set(name, await read(new URL(name, folder)));
  return out;
};

/** The body of every `command(` method in a source, braces matched. */
function commandBodies(source) {
  const out = [];
  for (const m of source.matchAll(/\n\s+command\(c\b[^)]*\)\s*\{/g)) {
    let depth = 1;
    let i = m.index + m[0].length;
    while (depth > 0 && i < source.length) {
      if (source[i] === "{") depth++;
      else if (source[i] === "}") depth--;
      i++;
    }
    out.push(source.slice(m.index, i));
  }
  return out;
}

/** The commands a view's `command()` names exactly (`c === "x"`), and whether it hands them to commands.js. */
function handledBy(file, files) {
  const exact = new Set();
  let runner = false;
  for (const body of commandBodies(files.get(file))) {
    for (const m of body.matchAll(/\bc === "([^"]+)"/g)) exact.add(m[1]);
    if (/\brunCommand\(c\b/.test(body)) runner = true;
  }
  return { exact, runner };
}

/** Each view of main.js's VIEWS, by name, as the source file that defines it. */
async function viewFiles() {
  const main = await read(new URL("main.js", folder));
  const fileOf = new Map();
  for (const m of main.matchAll(/import\s+([\w\s,{}]+?)\s+from\s+"\.\/([\w-]+\.js)"/g)) for (const id of m[1].match(/\w+/g)) fileOf.set(id, m[2]);
  const views = main.match(/const VIEWS = \{([^}]+)\}/)[1];
  return new Map([...views.matchAll(/"?([\w-]+)"?\s*:\s*(\w+)/g)].map(([, name, cls]) => [name, fileOf.get(cls)]));
}

/** Nodes a key hides with the views in them: Herdr's own ⌘B toggles the sidebar in every layout. */
const HIDEABLE = new Set(["sidebar"]);

/**
 * The cockpit views a layout shows, one per block that holds one (floating blocks too). A hidden copy takes no
 * commands, so views a key can hide are left out unless `all`.
 */
function appsOf(node, out = [], all = false) {
  if (!node || typeof node !== "object" || (!all && HIDEABLE.has(node.id))) return out;
  if (node.content?.mod === "cockpit" && node.content.props?.app) out.push(node.content.props.app);
  for (const c of [...(node.children ?? []), ...(node.floating ?? [])]) appsOf(c, out, all);
  return out;
}

/** The views of a layout with what each handles. */
async function layoutViews(layout) {
  const files = await sources();
  const fileOf = await viewFiles();
  return appsOf(await json(new URL(`layouts/${layout}.json`, app))).map((a) => ({ app: a, ...handledBy(fileOf.get(a), files) }));
}

/** Who runs `cockpit.<c>` in a layout: its runner when commands.js takes it, and each view that names it exactly. */
function handlersOf(c, layout, views, mod) {
  const out = [];
  for (const v of views) {
    if (v.runner && v.app === mod.RUNNER[layout] && mod.handles(layout, c)) out.push(`${v.app} (commands.js)`);
    if (v.exact.has(c)) out.push(v.app);
  }
  return out;
}

/** Every `layout:<name>` binding no handler or more than one runs in its layout: [{layout, keys, action, handlers}]. */
async function check(keys, mod) {
  const problems = [];
  for (const [section, map] of Object.entries(keys)) {
    if (!section.startsWith("layout:")) continue;
    const layout = section.slice(7);
    const views = await layoutViews(layout);
    for (const [k, action] of Object.entries(map)) {
      if (!action.startsWith("cockpit.")) continue;
      const handlers = handlersOf(action.slice(8), layout, views, mod);
      if (handlers.length !== 1) problems.push({ layout, keys: k, action, handlers });
    }
  }
  return problems;
}

/** A source without its comments, URLs kept: `//` after a colon is part of a string. */
const code = (text) => text.replace(/\/\*[\s\S]*?\*\//g, "").replace(/(^|[^:\\])\/\/.*$/gm, "$1");

// ---------------------------------------------------------------------------------------------------------------------
// Tests

test("every layout binding reaches exactly one handler in its layout", async () => {
  const { mod } = await fixture();
  assert.deepEqual(await check(await json(new URL("keys.json", app)), mod), []);
});

test("a binding no view of its layout handles fails the check, and so does a screen the layout does not have", async () => {
  const { mod } = await fixture();
  const keys = await json(new URL("keys.json", app));
  keys["layout:cockpit-chat"]["cmd-alt-a"] = "cockpit.agent-view";
  keys["layout:cockpit"]["cmd-alt-9"] = "cockpit.go:needs";
  keys["layout:cockpit"]["cmd-alt-x"] = "cockpit.nothing-here";
  keys["layout:cockpit-chat"]["cmd-alt-y"] = "cockpit.panel:nowhere";
  keys["layout:cockpit-chat"]["cmd-alt-q"] = "cockpit.queue";
  const problems = (await check(keys, mod)).map((p) => `${p.layout} ${p.action} ${p.handlers.length}`).sort();
  assert.deepEqual(problems, ["cockpit cockpit.go:needs 0", "cockpit cockpit.nothing-here 0", "cockpit-chat cockpit.agent-view 0", "cockpit-chat cockpit.panel:nowhere 0", "cockpit-chat cockpit.queue 0"]);
});

test("each layout has its runner, and every palette entry reaches exactly one handler there", async () => {
  const { mod } = await fixture();
  for (const layout of ["cockpit", "cockpit-chat"]) {
    const file = await json(new URL(`layouts/${layout}.json`, app));
    assert.equal(appsOf(file).filter((a) => a === mod.RUNNER[layout]).length, 1, `${layout} shows its runner once`);
    assert.equal(appsOf(file, [], true).filter((a) => a === mod.RUNNER[layout]).length, 1, "and nowhere a key hides it");
    assert.deepEqual(await paletteProblems(mod, layout), [], `${layout}: every palette entry runs once`);
  }
});

/** The palette entries of a layout that reach no handler there, or more than one, as `<action> <handlers>`. */
async function paletteProblems(mod, layout) {
  const views = await layoutViews(layout);
  const commands = [
    ...mod.screensOf(layout).list.map((s) => `cockpit.go:${s.child}`),
    ...mod.CATALOGUE.filter((e) => !["cockpit.palette", "cockpit.keys"].includes(e.command) && mod.available(layout, e.command.slice(8), { driven: true })).map((e) => mod.paletteAction(e)),
  ];
  const out = [];
  for (const action of commands) {
    const c = action.slice(8);
    if (c.startsWith("via:")) {
      const [screen, inner] = [c.slice(4, c.indexOf("|")), c.slice(c.indexOf("|") + 1)];
      if (!mod.handles(layout, c) || !views.some((v) => v.app === screen && v.exact.has(inner))) out.push(`${action} 0`);
    } else if (handlersOf(c, layout, views, mod).length !== 1) out.push(`${action} ${handlersOf(c, layout, views, mod).length}`);
  }
  return out;
}

test("a palette entry listed in a layout whose views do nothing with it fails the check", async () => {
  const { mod } = await fixture();
  const queue = mod.CATALOGUE.find((e) => e.command === "cockpit.queue");
  const was = queue.layouts;
  queue.layouts = ["cockpit", "cockpit-chat"];
  try {
    assert.deepEqual(await paletteProblems(mod, "cockpit-chat"), ["cockpit.queue 0"]);
  } finally {
    queue.layouts = was;
  }
});

test("every window command a view of a layout runs by name reaches a handler there, or the view asks first", async () => {
  const { mod } = await fixture();
  const fileOf = await viewFiles();
  const files = await sources();
  for (const layout of ["cockpit", "cockpit-chat"]) {
    const views = await layoutViews(layout);
    // The views a layout shows, with what opens over it in either layout (the palette, the terminal window's head).
    const shown = new Set([...appsOf(await json(new URL(`layouts/${layout}.json`, app)), [], true).map((a) => fileOf.get(a)), "palette.js", "agent-head.js", "agentsui.js"]);
    const dead = [];
    for (const name of shown) {
      const text = code(files.get(name));
      for (const [, c] of text.matchAll(/run\((?:keyed\()?"cockpit\.([^"]+)"/g)) {
        const asks = text.includes(`available(layoutName(), "${c}")`);
        const reached = c.endsWith(":") ? mod.runnerCommands(layout).includes(c) : handlersOf(c, layout, views, mod).length > 0;
        if (!reached && !asks) dead.push(`${name} cockpit.${c}`);
      }
    }
    assert.deepEqual(dead, [], `${layout}: commands its views run that nothing there handles`);
  }
});

test("the views' own keys stay free of window bindings", async () => {
  const { mod } = await fixture();
  const keys = await json(new URL("keys.json", app));
  const ui = await read(new URL("ui.js", folder));
  const taken = new Set([...ui.matchAll(/e\.keystroke === "([^"]+)"/g)].map((m) => m[1]));
  for (const [k] of mod.VIEW_KEYS) assert.ok(taken.has(k), `${k} is a key the page header takes`);
  for (const [section, map] of Object.entries(keys)) for (const k of Object.keys(map)) assert.ok(!mod.VIEW_KEYS.some(([v]) => v === k), `${section} binds ${k}, a view's own key`);
});

test("the keys sheet lists the shown layout's keys, grouped, Herdr's last", async () => {
  const keys = await bound();
  const chat = await fixture({ layout: "cockpit-chat", bound: keys });
  const groups = plain(chat.mod.sheet("cockpit-chat", keys));
  const titles = groups.map((g) => g.title);
  assert.deepEqual(titles.slice(0, 2), ["Screens", "Agents"]);
  assert.equal(titles.at(-1), "Herdr");
  const screens = groups[0].rows.map((r) => `${r.keys} ${r.title}`);
  assert.deepEqual(screens.slice(0, 4), ["cmd-alt-0 Chat", "cmd-alt-9 Needs you", "cmd-alt-8 Home", "cmd-alt-t Tab"]);
  // keys() lists keys sorted (as an install writes keys.json): the sheet keeps the layout's own order.
  const sorted = [...keys].sort((a, b) => (a.keys < b.keys ? -1 : 1));
  assert.deepEqual(plain(chat.mod.sheet("cockpit-chat", sorted)), groups);
  const all = groups.flatMap((g) => g.rows);
  assert.ok(!all.some((r) => r.action === "cockpit.agent-view"), "the lone agent is the cockpit layout's");
  assert.ok(all.some((r) => r.keys === "cmd-j" && r.title === "Next agent waiting for you"));
  assert.ok(all.some((r) => r.keys === "shift-l" && r.group === "In the apps"));
  // ⇧⇥ and ⎋ are the chat's own while the keyboard is in it, never the window's: a menu or the palette keeps them.
  assert.ok(!keys.some((b) => b.section === "layout:cockpit-chat" && ["shift-tab", "escape"].includes(b.keys)));
  assert.ok(all.some((r) => r.keys === "shift-tab" && r.group === "This agent" && r.action === null));
  assert.ok(all.some((r) => r.keys === "escape" && r.group === "This agent"));
  const herdr = groups.at(-1).rows.map((r) => r.keys);
  assert.ok(herdr.includes("cmd-n"), "Herdr's own keys stay listed");
  assert.ok(!herdr.includes("cmd-b") && !herdr.includes("cmd-/"), "a key the layout takes over is listed once, as the layout's");
  const cockpit = await fixture({ layout: "cockpit", bound: keys });
  const home = plain(cockpit.mod.sheet("cockpit", keys))[0].rows[0];
  assert.deepEqual([home.keys, home.title, home.action], ["cmd-alt-0", "Home", "cockpit.home"]);
});

test("tooltips name the shown layout's keys, and nothing Cockpit Chat shows writes a Cockpit key by hand", async () => {
  const keys = await bound();
  const chat = await fixture({ layout: "cockpit-chat", bound: keys });
  assert.equal(chat.mod.hint("Home", "cockpit.go:home"), "Home  ⌘⌥8");
  assert.equal(chat.mod.hint("Home", "cockpit.home"), "Home", "Cockpit's Home key is not Cockpit Chat's");
  assert.equal(chat.mod.hint("Keys", "cockpit.keys"), "Keys  ⌘/");
  const cockpit = await fixture({ layout: "cockpit", bound: keys });
  assert.equal(cockpit.mod.hint("Home", "cockpit.home"), "Home  ⌘⌥0");
  assert.equal(cockpit.mod.hint("Its terminal", "cockpit.terminal"), "Its terminal");
  // A key the Cockpit layout binds to something else, or not at all, in Cockpit Chat.
  const file = await json(new URL("keys.json", app));
  const own = file["layout:cockpit-chat"];
  const foreign = Object.entries(file["layout:cockpit"]).filter(([k, a]) => own[k] !== a).map(([k]) => cockpit.mod.keyText(k));
  const fileOf = await viewFiles();
  const shown = new Set([...appsOf(await json(new URL("layouts/cockpit-chat.json", app)), [], true).map((a) => fileOf.get(a)), "palette.js", "recap.js", "agent-head.js", "agentsui.js", "update.js", "code.js", "reader.js"]);
  const found = {};
  for (const name of shown) {
    const text = code(await read(new URL(name, folder)));
    const keys = foreign.filter((k) => text.includes(k));
    if (keys.length) found[name] = keys;
  }
  assert.deepEqual(found, {});
});

test("⌘J runs once, in the layout shown, on the agents a copy already follows, and lands on the agent waiting longest", async () => {
  const agents = [
    { address: "t1", status: "blocked", title: "Quota banner", tab: "b", space: "s" },
    { address: "t2", status: "blocked", title: "Enforce daily quotas", tab: "a", space: "s" },
  ];
  const rows = { agents: [{ address: "t1", terminal: "app:t1", stage: "needs_you", since: 200 }, { address: "t2", terminal: "app:t2", stage: "needs_you", since: 100 }] };
  const { mod, calls, shared, views } = await fixture({ layout: "cockpit-chat", agents });
  const { cx } = context();
  assert.equal(mod.runCommand("next-waiting", cx, "cockpit", {}), false, "the cockpit layout's runner stands down in Cockpit Chat");
  assert.equal(calls.length, 0);
  const unlend = mod.lend(() => rows);
  assert.equal(mod.runCommand("next-waiting", cx, "cockpit-chat"), true);
  unlend();
  assert.equal(views.length, 0, "the hub builds nothing while a copy follows the agents");
  assert.deepEqual(plain(calls.slice(0, 3)), [["agent.focus", { terminal: "t2" }], ["layout.select", { stack: "screen", child: "work" }], ["layout.select", { stack: "view", child: "chat" }]]);
  assert.equal(shared["cockpit.app"], "chat");
  calls.length = 0;
  assert.equal(mod.runCommand("go:needs", cx, "cockpit-chat"), true);
  assert.deepEqual(plain(calls[0]), ["layout.select", { stack: "screen", child: "needs" }]);
  assert.equal(mod.runCommand("go:agent", cx, "cockpit-chat"), false, "a screen the layout does not have");
});

test("⌘J reads a driven session in the chat, in either layout, and the next press moves on from it", async () => {
  const agents = [{ address: "t1", status: "blocked", title: "Quota banner", tab: "b", space: "s" }];
  const key = "claude:1793611000000";
  const rows = { agents: [
    { address: key, terminal: key, frontend: "app", driven: true, stage: "needs_you", status: "blocked", since: 50 },
    { address: "t1", terminal: "app:t1", stage: "needs_you", since: 200 },
  ] };
  const opened = (calls) => calls.filter((c) => c[0] === "agent.focus" || /chat:open:/.test(String(c[0])) || c[0] === "layout.switch").map((c) => (/chat:open:/.test(String(c[0])) ? String(c[0]).replace(/~[0-9a-f]{32}$/, "") : c));
  const chat = await fixture({ layout: "cockpit-chat", agents, focused: { terminal: "t9" } });
  const { cx } = context();
  const unlend = chat.mod.lend(() => rows);
  chat.mod.runCommand("next-waiting", cx, "cockpit-chat");
  assert.deepEqual(plain(opened(chat.calls)), ["cockpit.via:chat|chat:open:claude:1793611000000"], "the session waiting longest opens in the chat");
  assert.equal(chat.shared["cockpit.chat.pinned"].key, key);
  chat.mod.runCommand("next-waiting", cx, "cockpit-chat");
  assert.deepEqual(plain(opened(chat.calls).at(-1)), ["agent.focus", { terminal: "t1" }], "the chat's subject is where the next press starts from");
  // The chat follows t1 now; the next press comes back round to the session.
  chat.shared["cockpit.chat.subject"] = "t1";
  chat.calls.length = 0;
  chat.mod.runCommand("next-waiting", cx, "cockpit-chat");
  assert.deepEqual(plain(opened(chat.calls)), ["cockpit.via:chat|chat:open:claude:1793611000000"]);
  unlend();

  const cockpit = await fixture({ layout: "cockpit", agents, focused: { terminal: "t9" } });
  const back = cockpit.mod.lend(() => rows);
  cockpit.mod.runCommand("next-waiting", cx, "cockpit");
  assert.deepEqual(plain(opened(cockpit.calls)), [["layout.switch", { name: "cockpit-chat" }], "cockpit.via:chat|chat:open:claude:1793611000000"], "Cockpit reads it in Cockpit Chat");
  assert.ok(!cockpit.calls.some((c) => c[0] === "agent.focus"), "never a focus on a terminal it does not have");
  // The layout switched to Cockpit Chat: the next press there moves on from the session it reads.
  cockpit.shared.layout = "cockpit-chat";
  cockpit.mod.runCommand("next-waiting", cx, "cockpit-chat");
  assert.deepEqual(plain(opened(cockpit.calls).at(-1)), ["agent.focus", { terminal: "t1" }], "the next press moves on");
  back();
});

test("⌘J passes an agent with no running terminal and says so, and moves past one the last press could not bring up", async () => {
  const rows = { agents: [
    { address: "t41", terminal: "app:t41", title: "Enforce daily quotas", stage: "needs_you", since: 100 },
    { address: "t42", terminal: "app:t42", title: "Quota banner", stage: "needs_you", since: 200 },
    { address: "t43", terminal: "app:t43", title: "Retry failed CSV exports", stage: "needs_you", since: 300 },
  ] };
  const focusOf = (calls) => calls.filter((c) => c[0] === "agent.focus").map((c) => c[1].terminal);
  const noted = (calls) => calls.filter((c) => c[0] === "notify").map((c) => c[1].title);
  const f = await fixture({ layout: "cockpit", focused: { terminal: "t9" }, terms: [{ address: "t42", state: "live" }, { address: "t43", state: "live" }, { address: "t9", state: "live" }] });
  const { cx } = context();
  const unlend = f.mod.lend(() => rows);
  f.mod.runCommand("next-waiting", cx, "cockpit");
  assert.deepEqual(plain(focusOf(f.calls)), ["t42"], "the one waiting longest has no running terminal: on to the next");
  assert.deepEqual(plain(noted(f.calls)), ["Enforce daily quotas has no running terminal"]);
  // The focus never came: the window still shows t9, and the next press goes on from t42, not back to it.
  f.calls.length = 0;
  f.mod.runCommand("next-waiting", cx, "cockpit");
  assert.deepEqual(plain(focusOf(f.calls)), ["t43"]);
  assert.deepEqual(plain(noted(f.calls)), ["That agent did not come up"]);
  unlend();
  assert.deepEqual(plain(f.mod.nextOf(rows.agents.map((a) => ({ ...a, stage: "needs" })), null, "t9", () => false)), { agent: null, passed: rows.agents.map((a) => ({ ...a, stage: "needs" })) }, "nobody reachable: nothing to go to");
});

test("Tab and ⇧⇥ are the palette's own, from its search line too: the keyboard never leaves it for the window", async () => {
  const { mod } = await fixture({ layout: "cockpit-chat" });
  const bound = JSON.parse(JSON.stringify(mod.PALETTE_KEYS));
  assert.deepEqual(bound.map((b) => [b.keystroke, b.context]), [
    ["tab", "CockpitPalette"],
    ["tab", "CockpitPalette > Input"],
    ["shift-tab", "CockpitPalette"],
    ["shift-tab", "CockpitPalette > Input"],
  ]);
  assert.deepEqual([...new Set(bound.map((b) => b.action))], ["cockpit_palette_next", "cockpit_palette_prev"]);
});

test("⌃` opens the chat's subject: its terminal over the window, or the chat's question before a driven session is handed over", async () => {
  const { cx, settled } = context();
  const watched = await fixture({ layout: "cockpit-chat", shown: tree("screen", "home"), hub: { agents: { agents: [{ terminal: "app:t41", address: "t41" }] } } });
  watched.shared["cockpit.chat.subject"] = "t41";
  assert.equal(watched.mod.runCommand("terminal", cx, "cockpit-chat"), true, "from any screen: the runner has it");
  await settled();
  const window = plain(watched.calls.find((c) => c[0] === "overlay")[1]);
  assert.deepEqual([window.kind, window.layout.children[1].content], ["window", { terminal: "t41" }]);
  const key = "claude:1793611000000-0";
  const driven = await fixture({ layout: "cockpit-chat", hub: { agents: { agents: [{ terminal: key, driven: true }] } } });
  driven.shared["cockpit.chat.subject"] = key;
  driven.mod.runCommand("terminal", cx, "cockpit-chat");
  await settled();
  assert.ok(!driven.calls.some((c) => c[0] === "act"), "nothing is handed over before the person says so");
  assert.deepEqual(plain(driven.calls.at(-1)), ["cockpit.chat:handover"], "the chat asks first");
  const unknown = await fixture({ layout: "cockpit-chat" });
  unknown.shared["cockpit.chat.subject"] = key;
  unknown.mod.runCommand("terminal", cx, "cockpit-chat");
  await settled();
  assert.ok(!unknown.calls.some((c) => c[0] === "overlay"), "nothing opened on a guess when the hub says nothing of it");
  const here = await fixture({ layout: "cockpit-chat", shown: tree("screen", "work", "chat") });
  here.mod.runCommand("via:chat|chat:mode", cx, "cockpit-chat");
  assert.deepEqual(plain(here.calls.at(-1)), ["cockpit.chat:mode"]);
});

/** A palette with no Herdr places, over nothing, with these agents from the hub. */
function palette(mod, jumps, driven = false) {
  const p = Object.create(mod.Palette.prototype);
  Object.assign(p, { over: null, place: null, editor: "VS Code", jumps, listed: jumps.length, driven, recent: [], sheet: null });
  return p;
}

const JUMPS = [
  { terminal: "claude:1793611000000", address: "claude:1793611000000", title: "Make the quota configurable", status: "blocked", stage: "needs_you", waiting: true, driven: true, help: "needs you" },
  { terminal: "app:t41", address: "t41", title: "Enforce daily quotas", status: "blocked", stage: "needs_you", waiting: true, driven: false, help: "needs you" },
];

test("a driven session opens its chat from the palette, and Cockpit leaves it out", async () => {
  const chat = await fixture({ layout: "cockpit-chat" });
  const entries = palette(chat.mod, JUMPS).entries();
  const driven = entries.find((e) => e.id === "agent:claude:1793611000000");
  driven.act();
  const [opened] = plain(chat.calls.at(-1));
  assert.match(opened, /^cockpit\.via:chat\|chat:open:claude:1793611000000~[0-9a-f]{32}$/, "the person's pick goes with its one-time key");
  await entries.find((e) => e.id === "agent:t41").act();
  assert.deepEqual(plain(chat.calls.find((c) => c[0] === "agent.focus")), ["agent.focus", { terminal: "t41" }]);
  assert.deepEqual(plain(chat.calls.find((c) => c[0] === "act")), ["act", "agents", "seen", [{ terminal: "app:t41" }]], "a jump marks the agent seen");
  const cockpit = await fixture({ layout: "cockpit" });
  assert.ok(!palette(cockpit.mod, JUMPS).entries().some((e) => e.id === "agent:claude:1793611000000"), "Cockpit has no chat to open it in");
});

test("the palette lists what the shown layout has, a driven session's commands only while the chat follows one", async () => {
  const chat = await fixture({ layout: "cockpit-chat" });
  assert.deepEqual(plain(chat.mod.screens().list).map((s) => s.child).slice(0, 4), ["chat", "needs", "home", "tab"]);
  const ids = (p) => p.entries().map((e) => e.id);
  assert.ok(!ids(palette(chat.mod, [])).includes("cockpit.chat:mode"));
  assert.ok(ids(palette(chat.mod, [], true)).includes("cockpit.chat:mode"));
  assert.ok(!ids(palette(chat.mod, [])).includes("cockpit.queue") && !ids(palette(chat.mod, [])).includes("cockpit.agent-view"));
  assert.ok(ids(palette(chat.mod, [])).includes("cockpit.terminal") && ids(palette(chat.mod, [])).includes("cockpit.resume"));
  const cockpit = await fixture({ layout: "cockpit" });
  assert.ok(cockpit.mod.available("cockpit", "queue") && !cockpit.mod.available("cockpit", "terminal") && !cockpit.mod.available("cockpit", "sidebar"));
});

test("a query nothing matches offers Start work on it: the dialog in Cockpit, the chat's own box in Cockpit Chat", async () => {
  for (const layout of ["cockpit", "cockpit-chat"]) {
    const f = await fixture({ layout });
    const p = palette(f.mod, []);
    Object.assign(p, { input: { value: () => "Fix the zz-qq redirect" }, found: [] });
    const rows = p.rows();
    assert.equal(rows.length, 1);
    assert.deepEqual([rows[0].head, rows[0].e.title], ["Nothing matches", "Start work: Fix the zz-qq redirect"]);
    rows[0].e.act();
    if (layout === "cockpit") {
      const [, spec] = plain(f.calls.find((c) => c[0] === "overlay"));
      assert.deepEqual([spec.props.app, spec.props.source], ["start", "Fix the zz-qq redirect"], "the Start dialog, prefilled");
    } else assert.match(String(f.calls.at(-1)[0]), /^cockpit\.via:chat\|chat:start:new:Fix the zz-qq redirect~[0-9a-f]{32}$/, "the chat's box, with the pick's key");
  }
  const f = await fixture();
  const p = palette(f.mod, []);
  Object.assign(p, { input: { value: () => "recap" }, found: [] });
  assert.ok(!p.rows().some((r) => r.e.fallback), "a command matches: no Start work row");
  Object.assign(p, { input: { value: () => "zz-qq redirect" }, found: [{ kind: "session", title: "Daily quotas for ENG-412", target: {} }] });
  assert.deepEqual(plain(p.rows().map((r) => (r.e.fallback ? "start" : "found"))), ["found", "start"], "past sessions the hub found, then Start work");
});

test("an empty query lists the hub's agents first, in its order, then what was picked lately; the selection keeps its entry", async () => {
  const { mod } = await fixture();
  const e = (id, group, extra = {}) => ({ id, group, title: id, ...extra });
  const all = [
    e("fork-tab", "Session"),
    e("app:home", "Cockpit"),
    e("cockpit.recap", "Cockpit"),
    e("agent:t3", "Places", { waiting: false }),
    e("agent:t1", "Places", { waiting: true }),
    e("agent:t2", "Places", { waiting: true }),
    e("agent:t9", "Places", { waiting: false }),
    e("tab.new", "Tab"),
  ];
  const rows = plain(mod.emptyRows(all, ["cockpit.recap", "agent:t1", "gone"], ["agent:t1", "agent:t2", "agent:t3"]));
  assert.deepEqual(
    rows.map((r) => `${r.head}: ${r.e.id}`),
    ["Waiting on you: agent:t1", "Waiting on you: agent:t2", "Agents: agent:t3", "Recent: cockpit.recap", "Session: fork-tab", "Cockpit: app:home", "Places: agent:t9", "Tab: tab.new"],
  );
  assert.equal(mod.keepSelection(rows, "fork-tab", 1), 4, "the agents arrived above it: it moved, so did the selection");
  assert.equal(mod.keepSelection(rows, "gone", 20), rows.length - 1);
});

test("the agents menu runs once: the screen in front runs it when it can, the sidebar otherwise", async () => {
  const run = async (shown) => {
    const { mod } = await fixture({ layout: "cockpit-chat", shown });
    const sidebar = Object.create(mod.Sidebar.prototype);
    let ran = 0;
    sidebar.agents = { command: () => (ran++, true) };
    sidebar.command("agents|fork|app:t41|tab", context().cx);
    return [mod.takesAgentCommands(), ran];
  };
  assert.deepEqual(await run(tree("screen", "needs")), [false, 0], "Needs in front runs it with its own model");
  assert.deepEqual(await run(tree("screen", "home")), [true, 1]);
});

test("a copy of a cockpit layout is that layout, and a screen carried over from the other one is put back", async () => {
  const { mod, shared } = await fixture({ layout: "cockpit", app: "needs" });
  assert.equal(mod.baseOf("cockpit-chat-copy", () => null), "cockpit-chat");
  assert.equal(mod.baseOf("reading", () => ({ cloned_from: { name: "cockpit-chat" } })), "cockpit-chat");
  assert.equal(mod.baseOf("main", () => null), "cockpit");
  mod.settle();
  assert.equal(shared["cockpit.app"], "home");
});

test("a keys.json value written as a number reaches Herdr as one", async () => {
  const { mod, calls } = await fixture();
  mod.runAction("tab.select:1");
  mod.runAction("pane.split:right");
  assert.deepEqual(plain(calls), [["tab.select", { index: 1 }], ["pane.split", { direction: "right" }]]);
});

test("every node of a cockpit layout has an id of its own, so Herdr finds the one a drag or a command names", async () => {
  for (const name of ["cockpit", "cockpit-chat"]) {
    const ids = [];
    const walk = (node) => {
      if (node?.id) ids.push(node.id);
      for (const child of [...(node?.children ?? []), ...(node?.floating ?? [])]) walk(child);
    };
    walk(await json(new URL(`layouts/${name}.json`, app)));
    assert.deepEqual(ids.filter((id, i) => ids.indexOf(id) !== i), [], name);
  }
});
