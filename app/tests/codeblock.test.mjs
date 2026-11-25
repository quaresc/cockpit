import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { createContext, SourceTextModule, SyntheticModule } from "node:vm";
import { test } from "node:test";

/** codeblock.js on stubbed natives: elements record their calls, editors record how they were made and released. */
async function load() {
  const editors = [];
  const element = (kind) => {
    const calls = [];
    const el = new Proxy({ kind, calls }, { get: (t, key) => (key in t ? t[key] : (...args) => (calls.push([key, ...args]), el)) });
    return el;
  };
  const natives = {
    "gpui-kit": { div: () => element("div") },
    "gpui-base": {
      v_flex: () => element("v_flex"),
      Editor: { new: (state) => element(`editor:${state.options.language}`) },
      EditorState: {
        new: (options) => {
          const state = { options, value: options.value, released: false, set_value: (v) => (state.value = v), release: () => (state.released = true) };
          editors.push(state);
          return state;
        },
      },
    },
    herdr: { theme: () => ({ ansi1: "#1", ansi2: "#2", ansi3: "#3", ansi4: "#4", ansi5: "#5", ansi6: "#6", dim: "#d" }), settings: () => ({ font_family: "Mono" }) },
  };
  const context = createContext({ console });
  const module = new SourceTextModule(await readFile(new URL("../mods/cockpit/codeblock.js", import.meta.url), "utf8"), { context });
  await module.link((name) => new SyntheticModule(Object.keys(natives[name]), function () {
    for (const [key, value] of Object.entries(natives[name])) this.setExport(key, value);
  }, { context }));
  await module.evaluate();
  return { m: module.namespace, editors };
}

const k = { foreground: "#fff" };

test("a fence's tag names the editor's language", async () => {
  const { m } = await load();
  assert.equal(m.languageOf("ts"), "js");
  assert.equal(m.languageOf("TypeScript"), "js");
  assert.equal(m.languageOf("elixir"), "elixir");
  assert.equal(m.languageOf("sh {.copy}"), "bash");
  assert.equal(m.languageOf(""), "text");
});

test("a block shows plain lines first, then its editor, kept while drawn and released once it is not", async () => {
  const { m, editors } = await load();
  const codes = new m.CodeBlocks();
  codes.sweep();
  assert.equal(codes.view("b1", "ts", "const a = 1;", k).kind, "div");
  assert.equal(editors.length, 0, "no editor is made while a view is drawn");
  assert.equal(codes.make(), true);
  assert.equal(editors.length, 1);
  assert.deepEqual({ ...editors[0].options, syntax: undefined }, { language: "js", value: "const a = 1;", readonly: true, line_numbers: false, folding: false, soft_wrap: false, font_family: "Mono", font_size: 12, syntax: undefined });

  codes.sweep();
  const drawn = codes.view("b1", "ts", "const a = 2;", k);
  assert.ok(drawn.calls.some(([key, child]) => key === "child" && child.kind === "editor:js"), "the editor draws the block");
  assert.equal(editors[0].value, "const a = 2;", "a block still being written follows its words");
  assert.equal(codes.make(), false, "nothing more to make");

  codes.sweep();
  codes.sweep();
  assert.equal(editors[0].released, true, "a block no frame drew lets its editor go");
});

test("code fences split a message, a fence still being written included", async () => {
  const { m } = await load();
  const parts = JSON.parse(JSON.stringify(m.segments("Before\n```ts\nconst a = 1;\n```\nAfter\n```sh\nmix test")));
  assert.deepEqual(parts, [
    { code: false, text: "Before\n" },
    { code: true, lang: "ts", text: "const a = 1;\n" },
    { code: false, text: "\nAfter\n" },
    { code: true, lang: "sh", text: "mix test" },
  ]);
});
