import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import { createContext, SourceTextModule, SyntheticModule } from "node:vm";
import { test } from "node:test";

/** needs.js loaded on stubbed natives: its exports, the Herdr commands it ran and its global object. */
async function fixture() {
  const calls = [];
  const context = createContext({ console });
  const hub = () => { throw new Error("no hub call expected"); };
  const imports = { herdr: { run: (...args) => calls.push(args) }, "./hub.js": { act: hub, go: hub, subscribe: hub, openUrl: hub } };
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
  const load = (name) => {
    if (modules.has(name)) return modules.get(name);
    const native = imports[name];
    const module = native
      ? new SyntheticModule(Object.keys(native), function () { for (const [key, value] of Object.entries(native)) this.setExport(key, value); }, { context })
      : new SourceTextModule(sources.get(name), { context, identifier: name });
    modules.set(name, module);
    return module;
  };
  const module = new SourceTextModule('export * from "./needs.js"; export { default as Routines, bodyShown, bodySaved } from "./routines.js";', { context });
  await module.link(load);
  await module.evaluate();
  return { needs: module.namespace, calls, context, imports };
}

test("on the one-line field, a script of several lines is never joined into one on save", async () => {
  const { needs } = await fixture();
  const script = "git -C ~/notes pull\ngit -C ~/notes push";
  const shown = needs.bodyShown(script, false);
  assert.equal(shown.kept, script);
  assert.ok(!shown.shown.includes("\n") && shown.shown.startsWith("git -C ~/notes pull"), shown.shown);
  assert.equal(needs.bodySaved(shown.shown, shown).body, script, "untouched: saved as it was, its lines kept");
  assert.ok(needs.bodySaved(`${shown.shown} --rebase`, shown).error, "edited there: refused");
  assert.equal(needs.bodySaved(" Sum up ENG-412 ", needs.bodyShown("Sum up ENG-412", false)).body, "Sum up ENG-412", "one line is edited there");
  const whole = needs.bodyShown(script, true);
  assert.equal(whole.shown, script, "the multi-line field takes it whole");
  assert.equal(whole.kept, null);
});

test("Routines loads on a Herdr without the multi-line field, whose natives it never imports by name", async () => {
  const { needs, imports } = await fixture();
  assert.equal(typeof needs.Routines, "function", "the mod evaluated");
  for (const optional of ["TextareaState", "Textarea"]) {
    assert.equal(optional in imports["gpui-base"], false, `no mod imports ${optional} by name: a Herdr without it would fail to load them all`);
  }
});

test("a routine run that needs a look opens in Routines in the layout you are in", async () => {
  const { needs, calls, context } = await fixture();
  const heard = [];
  context.cockpitRemoteFocus = { targets: new Map(), listeners: new Set([(target) => (heard.push(target.key), true)]) };
  needs.openRoutine("standup/run/1794424000285");
  assert.deepEqual(calls, [["cockpit.go:routines"]], "no layout switch: the routines screen of this layout");
  assert.equal(context.cockpitRemoteFocus.targets.get("routines").key, "standup/run/1794424000285", "kept for a view that has no model yet");
  assert.deepEqual(heard, ["standup/run/1794424000285"], "a view that is ready selects it at once");
});


test("template Undo restores the typed model field and the submitted selection together", async () => {
  const { needs } = await fixture();
  const view = Object.create(needs.Routines.prototype);
  view.form = { slug: null, model: "Acme/Large@stable", agent: "opencode" };
  view.model = { templates: [{ id: "sweep", name: "Sweep", body: "Summarize the quota" }] };
  const values = { name: "", when: "weekdays 09:00", cwd: "", tz: "", precheck: "", body: "", model: "Acme/Large@stable" };
  view.fields = Object.fromEntries(Object.keys(values).map((name) => [name, { value: () => values[name], set_value: (value) => (values[name] = value) }]));
  view.preview = () => {};
  let undo;
  view.toasts = { push: (_cx, _text, options) => (undo = options.action) };
  const cx = { notify() {} };
  view.pickTemplate("sweep", cx);
  assert.equal(view.fields.model.value(), "");
  assert.equal(view.form.model, "");
  assert.ok(undo, "a typed model alone makes Undo available");
  undo.onClick(null, cx);
  assert.equal(view.fields.model.value(), "Acme/Large@stable");
  assert.equal(view.form.model, "Acme/Large@stable");
  assert.equal(view.form.agent, "opencode");
});
