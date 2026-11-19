import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import { createContext, SourceTextModule, SyntheticModule } from "node:vm";
import { test } from "node:test";

// Copies of one window lend the agents view they follow to the window's commands (⌘J, the palette). Herdr 0.1.12
// evaluates the mod once per copy, so every running load lends; a load that stopped running (a reload) lends nothing.

const folder = new URL("../mods/cockpit/", import.meta.url);

/** The mod's sources, and the natives each one imports, stubbed to throw: commands.js loads without a window. */
async function sources() {
  const out = new Map(), natives = {};
  for (const name of await readdir(folder)) {
    if (!name.endsWith(".js")) continue;
    const text = await readFile(new URL(name, folder), "utf8");
    out.set(`./${name}`, text);
    for (const m of text.matchAll(/import\s*\{([^}]+)\}\s*from\s*"(herdr|gpui-kit|gpui-base)"/g)) {
      const ex = (natives[m[2]] ??= {});
      for (const w of m[1].split(",")) {
        const key = w.trim().split(/\s+as\s+/)[0];
        if (key) ex[key] ??= () => { throw new Error(`unexpected native call: ${key}`); };
      }
    }
  }
  return { texts: out, natives };
}

/** One load of the mod in `context`: its own module instances, the window's globals shared with every other load. */
async function load(context, { texts, natives }, n, running) {
  const modules = new Map();
  const imports = { ...natives, "./hub.js": { LOAD: n, liveLoad: (l) => running.has(l), view: async () => null, act: async () => ({}), subscribe: () => ({ stop() {} }), remoteMachine: () => null } };
  const link = (name) => {
    if (modules.has(name)) return modules.get(name);
    const lib = imports[name];
    const mod = lib
      ? new SyntheticModule(Object.keys(lib), function () { for (const [k, v] of Object.entries(lib)) this.setExport(k, v); }, { context })
      : new SourceTextModule(texts.get(name), { context, identifier: name });
    modules.set(name, mod);
    return mod;
  };
  const entry = new SourceTextModule('export { lend, lent } from "./commands.js";', { context });
  await entry.link(link);
  await entry.evaluate();
  return entry.namespace;
}

test("every running load lends, the newest first, and a load that stopped running lends nothing", async () => {
  const src = await sources();
  const context = createContext({ console });
  const running = new Set([1, 2]);
  const first = await load(context, src, 1, running);
  const second = await load(context, src, 2, running);
  const asked = [];
  first.lend(() => (asked.push("first"), { agents: [{ address: "t1" }] }));
  assert.equal(second.lent().agents[0].address, "t1", "a copy of another running load lends to this one");
  assert.equal(first.lent().agents[0].address, "t1");
  second.lend(() => (asked.push("second"), { agents: [{ address: "t2" }] }));
  asked.length = 0;
  assert.equal(first.lent().agents[0].address, "t2", "the newest load is asked first");
  assert.deepEqual(asked, ["second"]);

  // A reload: the first load's views stay behind, frozen, and its load stops running.
  running.delete(1);
  const unlend = second.lend(() => null);
  asked.length = 0;
  running.delete(2);
  assert.equal(second.lent(), null, "no running load: nothing is lent");
  assert.deepEqual(asked, [], "a stopped load's lender is not even asked");
  running.add(2);
  unlend();
  assert.equal(second.lent().agents[0].address, "t2");
  assert.ok(!asked.includes("first"), "the stopped load's lender is never asked");
});
