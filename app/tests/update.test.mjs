import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import { createContext, SourceTextModule, SyntheticModule } from "node:vm";
import { test } from "node:test";

/** The hub's `options` view after an update from 0.40.3 to 0.41.0, in the acme world. */
const VIEW = {
  next: "0.41.0",
  layout: "cockpit-chat",
  update: { from: "0.40.3", to: "0.41.0", notes: [{ version: "0.41.0", date: "2026-11-20", changes: ["Quotas grouped by provider"] }], shown: false },
  options: [
    { key: "queue.drain", since: "0.41.0", title: "Send queued prompts on their own", description: "A prompt goes once the agent rests.", default: true, kind: "toggle", value: true, seen: false, new: true },
    { key: "chat.nextSteps", since: "0.41.0", title: "Suggest next prompts", description: "A few prompts after each turn.", default: false, kind: "toggle", value: false, seen: false, new: true },
    { key: "ci.tellAgents", since: "0.40.0", title: "Tell agents when their CI fails", description: "A failed run goes to its agent.", default: true, kind: "toggle", value: true, seen: true, new: false },
  ],
};

/** update.js on a fake hub and Herdr: `acts` every hub action, `ran` every Herdr command, `notices` the toasts. */
async function fixture({ layout = "cockpit", view = VIEW, saveWait = null } = {}) {
  const acts = [], ran = [], notices = [], overlays = [], closed = [];
  const shared = { layout };
  const context = createContext({ console });
  const imports = {
    herdr: {
      shared: (key) => shared[key] ?? null,
      setShared: (key, value) => (shared[key] = value),
      props: () => ({ overlay: "dialog-1" }),
      run: (...args) => ran.push(args),
      notify: (n) => notices.push(n),
      openOverlay: (o) => overlays.push(o),
      closeOverlay: (id) => closed.push(id),
      exec: async () => ({ code: 1, stdout: "" }),
    },
    "./hub.js": {
      view: async () => view,
      act: async (...args) => {
        acts.push(args);
        if (args[1] === "set" && saveWait) await saveWait;
        return { note: "ok", error: null };
      },
      install: async () => ({ cache: "/tmp/acme-cache", version: "0.41.0" }),
      sleep: async () => {},
      openUrl: () => {},
      cockpit: async () => ({ code: 0, stdout: "", stderr: "" }),
    },
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
  const module = new SourceTextModule('export * from "./update.js"; export { default } from "./update.js"; export { keyed, unkey } from "./person.js";', { context });
  await module.link(load);
  await module.evaluate();
  return { update: module.namespace, acts, ran, notices, overlays, closed, shared };
}

const plain = (v) => JSON.parse(JSON.stringify(v));

/** A context whose spawned work is kept to wait on. */
function cx() {
  const tasks = [];
  const c = { tasks, spawn: (f) => tasks.push(f(c)), notify: () => {}, focus_handle: () => ({ focus() {} }), timer: { after: (_ms, f) => f(c) } };
  return c;
}

/** The dialog opened on what the update brought, its first step shown; `person` when the person's click opened it. */
async function opened(f, { person = false } = {}) {
  const c = cx();
  const dialog = new f.update.default();
  dialog.init({ after: true, person }, c);
  await Promise.all(c.tasks);
  return { dialog, c };
}

test("the steps say what changed, the options the update brought, and the layout to try where another shows", async () => {
  const f = await fixture();
  const after = f.update.afterOf(VIEW, null);
  assert.deepEqual(plain(after.options.map((o) => o.key)), ["queue.drain", "chat.nextSteps"], "only what the update brought");
  assert.deepEqual(plain(f.update.afterSteps(after, "cockpit")), ["news", "options", "layout"]);
  assert.deepEqual(plain(f.update.afterSteps(after, "cockpit-chat")), ["news", "options"], "no layout to offer on that layout");
  assert.deepEqual(plain(f.update.afterSteps({ notes: [], options: [], layout: null }, "cockpit")), ["news"], "always one step");
  // A hub not restarted yet notes nothing: the install the dialog followed says from and to.
  const followed = f.update.afterOf({ update: null, options: VIEW.options }, { from: "0.40.3", to: "0.41.0", notes: [{ version: "0.41.0" }] });
  assert.deepEqual(plain([followed.from, followed.to, followed.notes.length, followed.options.length, followed.layout]), ["0.40.3", "0.41.0", 1, 2, null]);
  assert.equal(f.update.afterOf({ error: "no view \"options\"" }, null).to, null, "an older hub has no options view");
});

test("the window asks once, only while an update with something to say was not shown", async () => {
  const f = await fixture();
  assert.equal(f.update.worthAsking(VIEW, "cockpit"), true);
  assert.equal(f.update.worthAsking({ ...VIEW, update: { ...VIEW.update, shown: true } }, "cockpit"), false);
  const quiet = { ...VIEW, update: { ...VIEW.update, notes: [] }, options: VIEW.options.map((o) => ({ ...o, new: false })) };
  assert.equal(f.update.worthAsking(quiet, "cockpit"), true, "the layout to try is something to say");
  assert.equal(f.update.worthAsking(quiet, "cockpit-chat"), false, "nothing new on the layout it offers");
  assert.equal(f.update.worthAsking(null, "cockpit"), false);
});

test("opened by the person, the steps note the update shown and the options seen", async () => {
  const f = await fixture();
  const { dialog, c } = await opened(f, { person: true });
  assert.equal(dialog.phase, "after");
  assert.deepEqual(plain(f.acts), [["options", "shown", [{ update: "0.41.0" }]]]);
  dialog.next(c);
  assert.equal(dialog.steps[dialog.at], "options");
  assert.deepEqual(plain(f.acts.at(-1)), ["options", "shown", [{ keys: ["queue.drain", "chat.nextSteps"] }]]);
  dialog.next(c);
  assert.equal(f.acts.length, 2, "each noted once");
});

test("opened on its own or by a bare command, nothing counts as shown until the person acts in it", async () => {
  const f = await fixture();
  const { dialog, c } = await opened(f);
  dialog.key({ keystroke: "j", injected: true }, c);
  dialog.key({ keystroke: "enter", is_held: true }, c);
  assert.deepEqual(f.acts, [], "no `options shown` from a bare opening, an injected key or a held one");
  dialog.key({ keystroke: "escape" }, c);
  assert.deepEqual(plain(f.acts), [["options", "shown", [{ update: "0.41.0" }]]], "the person's own ⎋ closes it seen");
  const g = await fixture();
  const other = await opened(g);
  other.dialog.clicked(other.c);
  assert.deepEqual(plain(g.acts), [["options", "shown", [{ update: "0.41.0" }]]], "so does a click in it");
  const h = await fixture();
  h.update.openUpdate();
  h.update.openUpdate({ person: true });
  assert.deepEqual(plain(h.overlays.map((o) => o.props.person)), [false, true]);
});

test("options are saved only from a click or the person's own key, never an injected key or a bare command", async () => {
  const f = await fixture();
  const { dialog, c } = await opened(f);
  dialog.next(c);
  const sets = () => f.acts.filter(([, action]) => action === "set");
  // j moves to the second option; space flips it.
  assert.equal(dialog.key({ keystroke: "j", injected: true }, c), true, "an injected key may move");
  assert.equal(dialog.key({ keystroke: "space", injected: true }, c), true);
  assert.equal(dialog.key({ keystroke: "enter", injected: true }, c), true);
  dialog.command("update:save", c, null);
  await Promise.all(c.tasks);
  assert.deepEqual(sets(), [], "nothing saved from outside the person's hand");
  assert.equal(dialog.picks["chat.nextSteps"], false);
  dialog.key({ keystroke: "space" }, c);
  dialog.key({ keystroke: "enter" }, c);
  await Promise.all(c.tasks);
  assert.equal(dialog.picks["chat.nextSteps"], false, "the person's first space flips nothing: the dialog opened under their keys");
  assert.deepEqual(sets(), [], "nor does their first ⏎ save");
  assert.equal(dialog.hint, "Press j or k first, or click");
  assert.equal(dialog.steps[dialog.at], "options");
  dialog.key({ keystroke: "j" }, c);
  dialog.key({ keystroke: "space", is_held: true }, c);
  assert.equal(dialog.picks["chat.nextSteps"], false, "a held key flips nothing");
  dialog.key({ keystroke: "space" }, c);
  assert.equal(dialog.picks["chat.nextSteps"], true, "the person's own space flips the picked option");
  dialog.key({ keystroke: "enter" }, c);
  await Promise.all(c.tasks);
  assert.deepEqual(plain(sets().map(([app, action, args]) => [app, action, args])), [["options", "set", [{ "chat.nextSteps": true }]]], "only what changed");
  assert.equal(dialog.steps[dialog.at], "layout");
  assert.equal(dialog.done, "Options saved");
});

test("a command keyed by the window acts, a bare one does nothing", async () => {
  const f = await fixture();
  const { dialog, c } = await opened(f);
  dialog.next(c);
  dialog.flip("queue.drain", c);
  const { command, key } = f.update.unkey(f.update.keyed("update:save"));
  dialog.command(command, c, "0".repeat(32));
  await Promise.all(c.tasks);
  assert.equal(f.acts.filter(([, a]) => a === "set").length, 0, "a key the window never minted");
  dialog.command(command, c, key);
  await Promise.all(c.tasks);
  assert.deepEqual(plain(f.acts.filter(([, a]) => a === "set").map(([app, action, args]) => [app, action, args])), [["options", "set", [{ "queue.drain": false }]]]);
  dialog.command(command, c, key);
  await Promise.all(c.tasks);
  assert.equal(f.acts.filter(([, a]) => a === "set").length, 1, "a key is good once");
});

test("Skip keeps the options as they are; Try switches the layout and says how to come back", async () => {
  const f = await fixture();
  const { dialog, c } = await opened(f);
  dialog.next(c);
  dialog.flip("queue.drain", c);
  dialog.skip(c);
  assert.equal(f.acts.filter(([, a]) => a === "set").length, 0);
  assert.equal(dialog.steps[dialog.at], "layout");
  assert.equal(dialog.key({ keystroke: "enter", injected: true }, c), true);
  assert.deepEqual(f.ran, [], "an injected ⏎ switches nothing");
  dialog.key({ keystroke: "enter" }, c);
  assert.deepEqual(f.ran, [], "nor the person's ⏎ before they moved or clicked in the dialog");
  dialog.clicked(c);
  dialog.key({ keystroke: "enter", is_held: true }, c);
  assert.deepEqual(f.ran, [], "nor a held one");
  dialog.key({ keystroke: "enter" }, c);
  assert.deepEqual(plain(f.ran), [["layout.switch", { name: "cockpit-chat" }]]);
  assert.deepEqual(plain(f.closed), ["dialog-1"]);
  assert.match(f.notices[0].body, /⌘⇧L/);
});

test("on Cockpit Chat the last step is the options, and saving closes with a toast", async () => {
  const f = await fixture({ layout: "cockpit-chat" });
  const { dialog, c } = await opened(f);
  assert.deepEqual(plain(dialog.steps), ["news", "options"]);
  dialog.next(c);
  dialog.flip("chat.nextSteps", c);
  dialog.save(c);
  await Promise.all(c.tasks);
  assert.deepEqual(plain(f.closed), ["dialog-1"]);
  assert.equal(f.notices.at(-1).title, "Options saved");
  assert.deepEqual(f.ran, []);
});

test("Skip waits for an in-flight Save so the layout offer stays open", async () => {
  let finish;
  const saveWait = new Promise((resolve) => { finish = resolve; });
  const f = await fixture({ saveWait });
  const { dialog, c } = await opened(f);
  dialog.next(c);
  dialog.flip("chat.nextSteps", c);
  dialog.save(c);
  assert.equal(dialog.saving, true);
  dialog.skip(c);
  assert.equal(dialog.steps[dialog.at], "options", "Skip cannot advance during Save");
  finish();
  await Promise.all(c.tasks);
  assert.equal(dialog.saving, false);
  assert.equal(dialog.steps[dialog.at], "layout");
  assert.deepEqual(f.closed, [], "the person still gets the layout offer");
});
