import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import { createContext, SourceTextModule, SyntheticModule } from "node:vm";
import { test } from "node:test";

async function fixture(layout = "cockpit-chat") {
  const calls = [], notices = [];
  const context = createContext({ console });
  const imports = {
    herdr: {
      agents: () => [{ address: "t7", tab: "tab7", space: "s1", status: "working", title: "Local agent" }],
      spaces: () => [{ id: "s1", tabs: [{ id: "tab7" }] }],
      run: (...args) => calls.push(args),
      setShared: (...args) => calls.push(["shared", ...args]),
      shared: key => (key === "layout" ? layout : null),
      notify: notice => notices.push(notice),
    },
    "./hub.js": { act: () => { throw new Error("no hub action expected"); }, openUrl: () => {} },
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
  const module = new SourceTextModule('export * from "./chat.js";', { context });
  await module.link(load);
  await module.evaluate();
  return { chat: module.namespace, calls, notices };
}

const TURN = [
  { role: "user", text: "Fix the rounding", at: "2026-01-01T10:00:00Z" },
  { role: "assistant", text: "Looking at it.", at: "2026-01-01T10:00:02Z" },
  { role: "assistant", text: "", at: "2026-01-01T10:00:03Z", tool: { name: "Read", summary: "lib/pricing.ex", ok: true } },
  { role: "assistant", text: "", at: "2026-01-01T10:00:04Z", tool: { name: "Bash", summary: "mix test", ok: false } },
  { role: "assistant", text: "Found it.", at: "2026-01-01T10:02:16Z" },
  { role: "assistant", text: "   ", at: "2026-01-01T10:02:17Z" },
];

test("a turn is what you said, one folded row of work, then the answer", async () => {
  const { chat } = await fixture();
  const rows = chat.rowsOf(TURN);
  assert.equal(rows.map(r => r.kind).join(" "), "said work answered", "blank words are left out");
  assert.equal(rows[1].calls.map(c => c.name).join(" "), "Read Bash");
  assert.equal(rows[1].notes.map(n => n.text).join(" "), "Looking at it.", "what it said on the way stays in the fold");
  assert.equal(rows[2].text, "Found it.", "the last thing it says is the answer");
  assert.equal(chat.workedFor(rows[1].secs, rows[1].done), "Worked for 2m 14s");
  assert.equal(rows.map(r => r.key).join(" "), "r0 r1 r2");
});

test("a turn still running keeps its words in the fold and counts up to now", async () => {
  const { chat } = await fixture();
  const rows = chat.rowsOf(TURN.slice(0, 4), { working: true, now: "2026-01-01T10:00:42Z" });
  assert.equal(rows.map(r => r.kind).join(" "), "said work", "nothing is the answer yet");
  assert.equal(rows[1].done, false);
  assert.equal(rows[1].notes.length, 1);
  assert.equal(chat.workedFor(rows[1].secs, rows[1].done), "Working for 40s");
});

test("the model is named the way the picker says it", async () => {
  const { chat } = await fixture();
  assert.equal(chat.modelName("claude-opus-5-5"), "Opus 5.5");
  assert.equal(chat.modelName("claude-haiku-4-5-20251001"), "Haiku 4.5");
  assert.equal(chat.modelName(""), "");
});

test("fenced code is split out of the words around it", async () => {
  const { chat } = await fixture();
  const parts = chat.segments("Before\n\n```elixir\ndef f, do: 1\n```\n\nAfter");
  assert.equal(parts.map(p => p.code).join(" "), "false true false");
  assert.equal(parts[1].lang, "elixir");
  assert.equal(parts[1].text.trim(), "def f, do: 1");
  assert.equal(chat.segments("   ").length, 0);
});

test("the box offers the agent's commands after a / and the folder's files after an @", async () => {
  const { chat } = await fixture();
  const commands = [{ name: "review", description: "Review it" }, { name: "ship", description: "" }];
  const files = ["lib/pricing.ex", "test/pricing_test.exs", "README.md"];

  const slash = chat.hintsFor("/re", { commands, files });
  assert.equal(slash.kind, "command");
  assert.equal(slash.items.map((i) => i.put).join(" "), "/review ");

  const none = chat.hintsFor("/review it", { commands, files });
  assert.equal(none.items.length, 0, "a command with arguments is already chosen");

  const at = chat.hintsFor("look at @pricing", { commands, files });
  assert.equal(at.kind, "file");
  assert.equal(at.items.map((i) => i.label).join(" "), "pricing.ex pricing_test.exs");
  assert.equal(at.items[0].put, "look at @lib/pricing.ex ", "what was typed before the @ is kept");

  assert.equal(chat.hintsFor("nothing special", { commands, files }).items.length, 0);
});
