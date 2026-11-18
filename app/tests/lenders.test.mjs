import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import { createContext, SourceTextModule, SyntheticModule } from "node:vm";
import { test } from "node:test";

// An update reloads the mod in the same window: the views of the load before may never close, so the agents view
// they lend to the window's commands (⌘J, the palette) must stop counting once a newer load starts.

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
async function load(context, { texts, natives }) {
  const modules = new Map();
  const imports = { ...natives, "./hub.js": { view: async () => null, act: async () => ({}), subscribe: () => ({ stop() {} }), remoteMachine: () => null } };
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

test("a newer load of the mod drops the lenders an earlier load left, and only its own are asked", async () => {
  const src = await sources();
  const context = createContext({ console });
  const before = await load(context, src);
  const asked = [];
  before.lend(() => (asked.push("old"), { agents: [] }));
  assert.equal(before.lent().agents.length, 0, "one load: its lender answers");

  // The update: the same window loads the mod again, while the old views stay alive and never close.
  const after = await load(context, src);
  asked.length = 0;
  assert.equal(after.lent(), null, "the old load's empty list is not taken for the agents now");
  assert.deepEqual(asked, [], "the old lender is not even asked");
  after.lend(() => (asked.push("new"), { agents: [{ address: "t41" }] }));
  assert.equal(after.lent().agents[0].address, "t41");
  assert.deepEqual(asked, ["new"]);
  asked.length = 0;
  const unlend = before.lend(() => (asked.push("late old"), { agents: [] }));
  assert.equal(typeof unlend, "function");
  assert.equal(after.lent().agents[0].address, "t41");
  assert.ok(!asked.includes("late old"), "an old view lending again lends nothing");
  assert.equal(before.lent(), null, "and the old load's own commands fall back to the hub");
});
