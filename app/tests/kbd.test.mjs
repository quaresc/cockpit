import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import { createContext, SourceTextModule, SyntheticModule } from "node:vm";
import { test } from "node:test";

// Cockpit Chat keeps key hints to its tooltips: the views it shares with Cockpit draw no Kbd chip there, and Cockpit
// keeps them. The views render on a stand-in GPUI whose elements record every call made on them.

/** An element that records the calls made on it; a call's arguments hold its children. */
function recorder() {
  const made = [];
  const el = new Proxy(function () {}, {
    get: (_t, name) =>
      name === "made" ? made
      : name === "then" ? undefined
      // `.when(cond, f)` draws what `f` adds only when `cond` holds, as GPUI's does.
      : name === "when" ? (cond, f) => (made.push(["when", Boolean(cond)]), cond ? f(el) : el)
      : (...args) => (made.push([name, ...args]), el),
    apply: () => el,
  });
  return el;
}

/** A GPUI export: called it draws an element, and its statics (`Input.new`, `Scrollbar.vertical`) do too. */
const native = () => new Proxy(function () {}, { get: () => native(), apply: () => recorder() });

async function fixture(layout) {
  const context = createContext({ console, setTimeout, Date, Math, JSON, Map, Set, Array, Object, String, Number, Boolean, RegExp, Error, Promise });
  const theme = { foreground: "#ededed", background: "#1e1e1e", hover: "#2a2a2a", muted: "#a0a0a0", dim: "#707070", border: "#333333", accent: "#a9b4ff", ansi1: "#ff8080", ansi2: "#99ffe4", ansi3: "#ffc799" };
  const imports = {
    herdr: {
      shared: (key) => (key === "layout" ? layout : null),
      setShared: () => {},
      theme: () => theme,
      settings: () => ({ font_family: "JetBrains Mono" }),
      layout: () => ({ id: "root", children: [] }),
      layouts: () => [],
      keys: () => [],
      spaces: () => [],
      agents: () => [],
      focused: () => ({}),
      window: () => ({ platform: "macos" }),
    },
    "./hub.js": { subscribe: () => ({ stop() {} }), act: async () => ({}), remoteMachine: () => null, isNotInstalled: () => false, isStoppedOnPurpose: () => false, downWords: () => "", startHub: async () => {} },
  };
  const sources = new Map(), modules = new Map();
  const folder = new URL("../mods/cockpit/", import.meta.url);
  for (const name of await readdir(folder)) {
    if (!name.endsWith(".js")) continue;
    const text = await readFile(new URL(name, folder), "utf8");
    sources.set(`./${name}`, text);
    for (const m of text.matchAll(/import\s*\{([^}]+)\}\s*from\s*"(herdr|gpui-kit|gpui-base)"/g)) {
      const ex = (imports[m[2]] ??= {});
      for (const w of m[1].split(",")) {
        const key = w.trim().split(/\s+as\s+/)[0];
        if (!key) continue;
        ex[key] ??= m[2] === "herdr" ? () => null : native();
      }
    }
  }
  const load = (name) => {
    if (modules.has(name)) return modules.get(name);
    const lib = name.startsWith("./") && !imports[name] ? null : imports[name];
    const mod = lib
      ? new SyntheticModule(Object.keys(lib), function () { for (const [k, v] of Object.entries(lib)) this.setExport(k, v); }, { context })
      : new SourceTextModule(sources.get(name), { context, identifier: name });
    modules.set(name, mod);
    return mod;
  };
  const entry = new SourceTextModule(
    'export * as ui from "./ui.js"; export { default as Todo } from "./todo.js"; export { default as Runbook } from "./runbook.js"; export { default as Show } from "./show.js"; export { QuestionCard } from "./agentsui.js"; export { stopButton } from "./agent-head.js";',
    { context },
  );
  await entry.link(load);
  await entry.evaluate();
  return entry.namespace;
}

/** Every Kbd chip under an element: a 20 px high, at least 20 px wide, bordered key cap. */
function chips(el, seen = new Set()) {
  if (!el || (typeof el !== "function" && typeof el !== "object") || seen.has(el)) return 0;
  seen.add(el);
  const made = el.made;
  if (!Array.isArray(made)) return Array.isArray(el) ? el.reduce((n, x) => n + chips(x, seen), 0) : 0;
  const cap = made.some(([n, v]) => n === "h" && v === 20) && made.some(([n, v]) => n === "min_w" && v === 20) && made.some(([n]) => n === "border_1");
  return (cap ? 1 : 0) + made.reduce((n, [, ...args]) => n + args.reduce((m, a) => m + chips(a, seen), 0), 0);
}

/** The pieces of the shared views that draw keys: to-do rows, a runbook's adder, a show pane's lines, a question card, the footers. */
function render(mod) {
  const k = mod.ui.tokens();
  const els = [];
  const todo = Object.create(mod.Todo.prototype);
  Object.assign(todo, {
    sel: { kind: "adder" }, filter: "", showDone: false, marks: new Set(), editing: { key: "k1" }, editor: recorder(),
    flashes: { of: () => null }, hover: { is: () => false, on: () => {} }, reorder: { grip: () => recorder(), track: () => {} },
    shown: () => ({ open: [{ key: "k1", text: "Write the quota docs" }], done: [{ key: "k2", text: "Ship ENG-412", done: true }] }),
  });
  els.push(...todo.rows(k, { notify() {} }).map((r) => r.el));
  const runbook = Object.create(mod.Runbook.prototype);
  Object.assign(runbook, { panel: false, filter: "", adding: false, sel: null, scope: "runbook", shown: () => [] });
  els.push(...runbook.rows(k).map((r) => r.el));
  const show = Object.create(mod.Show.prototype);
  Object.assign(show, { focus: null, asking: false, name: "deploy", activeDiff: () => null });
  els.push(show.footer(k, { name: "deploy", url: "https://acme.example/deploy", blocks: [] }));
  const model = { busy: new Map(), picksOf: () => new Set(), drawn: new Map(), opened: new Set() };
  const row = { terminal: "app:t41", question: { text: "Which limit should existing accounts start with?", kind: "single", choices: [{ n: 1, label: "The plan's default" }, { n: 2, label: "Their last 30 days' peak" }], cursor: -1 } };
  els.push(mod.QuestionCard("needs-app:t41", row, model, { wide: true }));
  els.push(mod.ui.KeyHints([["j k", "move"], ["⏎", "run"], ["a", "new"]]));
  els.push(mod.ui.Kbd("esc"));
  return els;
}

test("Cockpit Chat draws no Kbd chip in the views it shares, and Cockpit keeps them", async () => {
  const chat = await fixture("cockpit-chat");
  assert.equal(render(chat).reduce((n, el) => n + chips(el), 0), 0, "no key cap anywhere in Cockpit Chat");
  const cockpit = await fixture("cockpit");
  const each = render(cockpit).map((el) => chips(el));
  assert.ok(each.every((n) => n > 0), `every piece keeps its key caps in Cockpit: ${each.join(", ")}`);
});

test("a choice's number stays readable in Cockpit Chat, as text, and its key is in the tooltip", async () => {
  const chat = await fixture("cockpit-chat");
  const made = chat.ui.ChoiceKey(2).made;
  assert.ok(made.some(([n, v]) => n === "child" && v === "2"), "the number, as quiet text");
  const model = { busy: new Map(), picksOf: () => new Set(), drawn: new Map(), opened: new Set() };
  const row = { terminal: "app:t41", question: { text: "Run it?", kind: "single", choices: [{ n: 1, label: "Yes" }], cursor: -1 } };
  const tips = [];
  const walk = (el, seen = new Set()) => {
    if (!el?.made || seen.has(el)) return;
    seen.add(el);
    for (const [n, ...args] of el.made) {
      if (n === "tooltip") tips.push(args[0]);
      for (const a of args) (Array.isArray(a) ? a : [a]).forEach((x) => walk(x, seen));
    }
  };
  walk(chat.QuestionCard("card", row, model, { wide: true }));
  assert.ok(tips.includes("Answer “Yes”  1"));
});

test("the agent head's Stop names the keys its row says stop a turn: a Kbd in Cockpit, the tooltip in Cockpit Chat", async () => {
  const tipsOf = (el) => el.made.filter(([n]) => n === "tooltip").map(([, t]) => t);
  const a = { handle: "app:t44" };
  const cockpit = await fixture("cockpit");
  const kept = cockpit.stopButton(a, "esc");
  assert.equal(chips(kept), 1, "its key on the button");
  assert.deepEqual(tipsOf(kept), ["Stop its turn, also in its terminal"]);
  const chat = await fixture("cockpit-chat");
  const quiet = chat.stopButton(a, "esc");
  assert.equal(chips(quiet), 0);
  assert.deepEqual(tipsOf(quiet), ["Stop its turn, also in its terminal  esc"], "the key in the tooltip");
  const none = cockpit.stopButton(a, null);
  assert.equal(chips(none), 0, "no key measured: none shown");
  assert.deepEqual(tipsOf(none), ["Stop its turn"]);
});
