// The chat's sends, edits and answers from the box, against a hub whose answers the test releases by hand.
import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import { createContext, SourceTextModule, SyntheticModule } from "node:vm";
import { test } from "node:test";

const MODS = new URL("../mods/cockpit/", import.meta.url);

async function subjectFixture({ focusedOn = "t41" } = {}) {
  const calls = [], acts = [];
  const shared = { layout: "cockpit-chat" };
  // Herdr's focus as it is with an agent in front: its pane's terminal, its tab, and the agent its blocks would show.
  const focus = focusedOn ? { agent: focusedOn, terminal: focusedOn, tab: `tab-${focusedOn}` } : {};
  // Each hub act waits until the test answers it.
  const waiting = [];
  const context = createContext({ console });
  const imports = {
    herdr: {
      focused: () => focus,
      agents: () => ["t41", "t42", "t45"].map((address) => ({ address, tab: `tab-${address}` })),
      spaces: () => [],
      run: (...args) => calls.push(args),
      shared: (key) => shared[key] ?? null,
      setShared: (key, value) => (shared[key] = value),
      notify: (n) => calls.push(["notify", n]),
      layout: () => null,
    },
    "./hub.js": {
      subscribe: (_cx, spec) => (calls.push(["subscribe", spec]), { stop() {} }),
      act: (app, action, args) => new Promise((resolve, reject) => { acts.push([app, action, args]); waiting.push({ resolve, reject }); }),
      view: async () => null,
      openUrl: () => {},
      sleep: async () => {},
      install: async () => ({}),
      cockpit: async () => ({ code: 0, stdout: "", stderr: "" }),
      isUnconfirmed: (e) => Boolean(e?.unconfirmed),
      remoteMachine: () => null,
    },
  };
  const sources = new Map(), modules = new Map();
  const folder = new URL(MODS);
  for (const name of await readdir(folder)) {
    if (!name.endsWith(".js")) continue;
    const text = await readFile(new URL(name, folder), "utf8");
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
  const module = new SourceTextModule('export * from "./chat.js"; export { default as Chat } from "./chat.js"; export { AgentsModel } from "./agentsui.js"; export { keyed, unkey } from "./person.js";', { context });
  await module.link(load);
  await module.evaluate();
  const mod = module.namespace;
  const chat = Object.create(mod.Chat.prototype);
  let typed = "";
  Object.assign(chat, { drafts: new Map(), sends: new Map(), edits: new Map(), typedPicks: new Map(), files: [], target: undefined, limit: null, model: { row: () => null, value: null }, input: { value: () => typed, set_value: (t) => (typed = t), focus() {} } });
  const work = [];
  const cx = { notify() {}, spawn: (f) => work.push(f(cx)), timer: { after: () => {} } };
  const tick = () => new Promise((r) => setTimeout(r, 5));
  return { mod, chat, cx, calls, acts, waiting, shared, focus, type: (t) => (typed = t), settled: () => Promise.all(work), tick };
}

test("a keyed pick from another subject's menu cannot change the newly opened agent", async () => {
  for (const driven of [true, false]) {
    const f = await subjectFixture();
    const [a, b] = driven ? ["claude:1", "claude:2"] : ["t41", "t42"];
    f.chat.picker = (_id, opts) => opts;
    const show = (target) => {
      f.chat.command(`chat:open:${target}`, f.cx);
      if (driven) f.chat.sessions = { session: { key: target, model: "opus", working: false } };
      else f.chat.value = { agent: { terminal: `app:${target}`, status: "idle" } };
    };
    const menu = () => f.chat.pickChip({ id: "chat-pick-effort", items: [{ label: "High", command: "pick:effort:high" }] }, {});
    const choose = (raw) => {
      const { command, key } = f.mod.unkey(raw);
      f.chat.command(command, f.cx, key);
    };
    show(a);
    const stale = f.mod.keyed(menu().items[0].command);
    show(b);
    choose(stale);
    await f.settled();
    assert.equal(f.acts.length, 0, "the still-valid menu key cannot apply A's choice to B");
    choose(f.mod.keyed(menu().items[0].command));
    assert.equal(f.acts.length, 1, "a fresh menu for B still works");
    const args = f.acts[0][2][0];
    assert.equal(driven ? args.key : args.terminal, driven ? b : `app:${b}`);
    f.waiting[0].resolve({ note: "set" });
    await f.settled();
  }
});

test("an edit's send leaves words typed into the box while it was on its way", async () => {
  const key = "claude:1793611000000-0";
  const f = await subjectFixture();
  f.chat.open(key, f.cx);
  f.chat.sessions = { session: { key, working: true, messages: [] } };
  f.chat.model.row = () => ({ terminal: key, driven: true, status: "working" });
  f.type("Half a thought");
  f.chat.edits.set(f.chat.target, { id: "q1", draft: { text: f.chat.input.value(), files: [] } });
  f.type("Then the docs, and the changelog");
  f.chat.send(f.cx);
  // While it is on its way, the person types something new.
  f.type("A new idea typed meanwhile");
  f.waiting[0].resolve({ note: "" });
  await f.settled();
  assert.equal(f.chat.input.value(), "A new idea typed meanwhile", "words typed meanwhile stay (send's own comment says so)");
});

test("an answer from the box whose hub reply never came is 'Not confirmed', not 'Not sent'", async () => {
  const f = await subjectFixture();
  // A terminal agent waiting on a free-text question.
  let row = { terminal: "app:t41", status: "blocked", question: { kind: "text", text: "Which branch?", asked: "fp1", choices: [] } };
  const model = Object.create(f.mod.AgentsModel.prototype); Object.assign(model, { view: "chat", busy: new Map(), picks: new Map(), drawn: new Map(), opened: new Set(), replying: null });
  model.value = { agents: [row] };
  model.row = () => row;
  model.shown = () => row.question;
  model.reply = { value: () => model._reply ?? "", set_value: (t) => (model._reply = t), focus() {} };
  f.chat.model = model;
  f.chat.watch("t41", f.cx);
  f.chat.value = { agent: row };
  f.type("main");
  f.chat.send(f.cx);
  await f.tick();
  assert.deepEqual(JSON.parse(JSON.stringify(f.acts[0].slice(0, 2))), ["agents", "answer"]);
  // The hub took the answer, then the connection dropped before its reply (hub.js marks this unconfirmed).
  f.waiting[0].reject(Object.assign(new Error("the cockpit hub closed the connection"), { unconfirmed: true }));
  await f.settled();
  await f.tick();
  assert.equal(f.chat.sent?.state, "unconfirmed", "the answer may have gone: it is not 'Not sent'");
});

test("…and sending after that does not send the answer's words again as a prompt", async () => {
  const f = await subjectFixture();
  let row = { terminal: "app:t41", status: "blocked", question: { kind: "text", text: "Which branch?", asked: "fp1", choices: [] } };
  const model = Object.create(f.mod.AgentsModel.prototype); Object.assign(model, { view: "chat", busy: new Map(), picks: new Map(), drawn: new Map(), opened: new Set(), replying: null });
  model.value = { agents: [row] };
  model.row = () => row;
  model.shown = () => row?.question ?? null;
  model.reply = { value: () => model._reply ?? "", set_value: (t) => (model._reply = t), focus() {} };
  f.chat.model = model;
  f.chat.watch("t41", f.cx);
  f.chat.value = { agent: row };
  f.type("main");
  f.chat.send(f.cx);
  await f.tick();
  f.waiting[0].reject(Object.assign(new Error("the cockpit hub closed the connection"), { unconfirmed: true }));
  await f.settled();
  await f.tick();
  // It had gone: the agent took the answer and the question is gone.
  row = { terminal: "app:t41", status: "working", question: null };
  f.chat.value = { agent: row };
  f.chat.send(f.cx); // Retry
  await f.tick();
  assert.equal(f.acts.length, 1, "the words already went once");
});

test("an answer from the box that lands after the reader left for B leaves nothing in A's box to send again", async () => {
  const f = await subjectFixture();
  let row = { terminal: "app:t41", status: "blocked", question: { kind: "text", text: "Which branch?", asked: "fp1", choices: [] } };
  const other = { terminal: "app:t42", status: "idle" };
  const model = Object.create(f.mod.AgentsModel.prototype);
  Object.assign(model, { view: "chat", busy: new Map(), picks: new Map(), drawn: new Map(), opened: new Set(), replying: null });
  model.value = { agents: [row, other] };
  model.row = (t) => (String(t).endsWith("t41") ? row : other);
  model.shown = (r) => r?.question ?? null;
  model.reply = { value: () => model._reply ?? "", set_value: (t) => (model._reply = t), focus() {} };
  f.chat.model = model;
  f.chat.watch("t41", f.cx);
  f.chat.value = { agent: row };
  f.type("main");
  f.chat.send(f.cx);
  await f.tick();
  // The reader moves on to B before the hub answers.
  f.chat.watch("t42", f.cx);
  f.chat.value = { agent: other };
  f.waiting[0].resolve({ note: "Answered: main" });
  await f.settled();
  await f.tick();
  row = { terminal: "app:t41", status: "working", question: null };
  f.chat.watch("t41", f.cx);
  assert.equal(f.chat.input.value(), "", "back on A, the answer that went is not offered again");
});
