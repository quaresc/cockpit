// The chat's reading position, window and keys, over conversations long enough to need them.
import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import { createContext, SourceTextModule, SyntheticModule } from "node:vm";
import { test } from "node:test";

const MODS = new URL("../mods/cockpit/", import.meta.url);

async function subjectFixture({ focusedOn = "t41", answers = {}, views = {} } = {}) {
  const calls = [], acts = [];
  const shared = { layout: "cockpit-chat", "cockpit.app": "chat" };
  // Herdr's focus as it is with an agent in front: its pane's terminal, its tab, and the agent its blocks would show.
  const focus = focusedOn ? { agent: focusedOn, terminal: focusedOn, tab: `tab-${focusedOn}` } : {};
  const context = createContext({ console });
  const imports = {
    herdr: {
      focused: () => focus,
      agents: () => ["t41", "t42", "t45"].map((address) => ({ address, tab: `tab-${address}` })),
      spaces: () => [],
      run: (...args) => calls.push(args),
      shared: (key) => shared[key] ?? null,
      setShared: (key, value) => (shared[key] = value),
      notify: () => {},
      layout: () => null,
    },
    "./hub.js": {
      subscribe: (_cx, spec) => (calls.push(["subscribe", spec]), { stop() {} }),
      act: async (app, action, args) => (acts.push([app, action, args]), answers[`${app} ${action}`] ?? { note: "ok" }),
      view: async (name, args) => (calls.push(["view", name, args]), typeof views[name] === "function" ? views[name](args) : (views[name] ?? null)),
      openUrl: () => {},
      sleep: async () => {},
      install: async () => ({}),
      cockpit: async () => ({ code: 0, stdout: "", stderr: "" }),
      isUnconfirmed: (e) => Boolean(e?.unconfirmed),
      remoteMachine: () => null,
    },
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
  const module = new SourceTextModule('export * from "./chat.js"; export { default as Chat } from "./chat.js";', { context });
  await module.link(load);
  await module.evaluate();
  const mod = module.namespace;
  const chat = Object.create(mod.Chat.prototype);
  let typed = "";
  Object.assign(chat, { drafts: new Map(), sends: new Map(), edits: new Map(), files: [], target: undefined, limit: null, model: { row: () => null, value: null }, input: { value: () => typed, set_value: (t) => (typed = t), focus() {} } });
  const work = [];
  const cx = { notify() {}, spawn: (f) => work.push(f(cx)), timer: { after: () => {} } };
  return { mod, chat, cx, calls, acts, shared, focus, type: (t) => (typed = t), settled: () => Promise.all(work) };
}

/** A cx whose timers run in order when flushed, and a scroll handle that remembers where it was sent. */
function reading(f) {
  const timers = [], scrolled = [];
  const cx = { notify() {}, spawn: f.cx.spawn, timer: { after: (ms, fn) => (timers.push([ms, fn]), { cancel() {} }) } };
  const flush = () => {
    for (const [, fn] of timers.splice(0).sort((a, b) => a[0] - b[0])) fn();
  };
  Object.assign(f.chat, { scroll: { scroll_to_item: (i) => scrolled.push(i) }, pinned: true, top: 0, viewH: 300, heights: new Map(), seenMsg: null, rows: [], all: [], windowTop: null, windowAround: null, restoring: null, loadingEarlier: false, older: null, kept: null });
  return { cx, flush, scrolled };
}

const said = (n) => ({ role: "user", text: `Message ${n}`, at: `2026-01-01T10:${String(Math.floor(n / 60)).padStart(2, "0")}:${String(n % 60).padStart(2, "0")}Z` });
const at = (n) => `2026-01-01T${String(10 + Math.floor(n / 3600)).padStart(2, "0")}:${String(Math.floor(n / 60) % 60).padStart(2, "0")}:${String(n % 60).padStart(2, "0")}Z`;

/** Turns of: what you said, five tool calls, the answer (7 messages each). */
function turns(count) {
  const out = [];
  let t = 0;
  for (let i = 0; i < count; i++) {
    out.push({ role: "user", text: `Turn ${i}: do the thing`, at: at(t++) });
    for (let c = 0; c < 5; c++) out.push({ role: "assistant", text: "", at: at(t++), tool: { name: "Bash", summary: `step ${i}.${c}`, ok: true } });
    out.push({ role: "assistant", text: `Turn ${i} done.`, at: at(t++) });
  }
  return out;
}

function draw(f, cx, messages, all = f.mod.rowsOf(messages)) {
  const rows = f.chat.drawn(all);
  f.chat.settle(rows, cx);
  f.chat.rows = rows;
  f.chat.all = all;
  f.chat.messagesNow = messages;
  for (const r of rows) if (!f.chat.heights.has(r.key)) f.chat.heights.set(r.key, 100);
  return rows;
}

test("reading further back keeps the message at the top in place even when the window cut its turn in two", async () => {
  const f = await subjectFixture();
  const { cx, flush, scrolled } = reading(f);
  const all = turns(20);
  const window = all.slice(all.length - 40 + 2);
  f.chat.target = "t41";
  f.chat.value = { agent: { terminal: "app:t41" }, messages: window, more: true };
  const before = draw(f, cx, window);
  assert.equal(before[0].kind, "work", "the first row is the tail of a turn");
  flush();
  scrolled.length = 0;
  f.chat.earlier(cx);
  const anchor = f.chat.restoring?.anchor;
  assert.equal(anchor, before[0].ids[0], "the anchor is the first message shown, not a row");
  f.chat.value = { agent: { terminal: "app:t41" }, messages: all, more: false };
  const after = draw(f, cx, all);
  const holder = after.findIndex((r) => r.ids.includes(anchor));
  assert.ok(holder > 0, "the grown row still holds it");
  flush();
  assert.equal(scrolled.at(-1), holder, "the view goes back to the row holding that message");
});

test("reading back while rows keep coming: the drawn window stays, nothing above the reader goes", async () => {
  const f = await subjectFixture();
  const { cx, flush, scrolled } = reading(f);
  const msgs = (n) => Array.from({ length: n }, (_, i) => said(i));
  const rows = draw(f, cx, msgs(350));
  flush();
  scrolled.length = 0;
  f.chat.wheeled({ delta: { y: 5000 } }, cx);
  assert.equal(f.chat.pinned, false, "reading back");
  assert.ok(f.chat.windowTop, "the window froze where it was");
  const next = draw(f, cx, msgs(351));
  flush();
  assert.equal(next[0].key, rows[0].key, "the row at the top of the content is still there");
  assert.deepEqual(scrolled, [], "and nothing moved the reader");
  assert.equal(f.chat.unseen(), 1, "the new one waits, counted");
});

test("a conversation's latest page sliding on keeps the messages the reader holds", async () => {
  const f = await subjectFixture();
  const page = (from, to) => Array.from({ length: to - from }, (_, i) => said(from + i));
  const held = f.mod.joinPages(page(0, 400), page(1, 401));
  assert.equal(held.length, 401, "the one that slid out stays");
  assert.equal(held[0].text, "Message 0");
  assert.equal(f.mod.joinPages(page(0, 400), page(500, 900)).length, 400, "a page past what is held starts over");
});

test("a trackpad's momentum at the top slides the window once, not once per tick", async () => {
  const f = await subjectFixture();
  const { cx } = reading(f);
  const messages = Array.from({ length: 1000 }, (_, i) => said(i));
  draw(f, cx, messages);
  f.chat.pinned = false;
  f.chat.top = 0;
  f.chat.wheeled({ delta: { y: 40 } }, cx);
  const first = f.chat.restoring.anchor;
  const top = f.chat.windowTop;
  draw(f, cx, messages);
  f.chat.wheeled({ delta: { y: 40 } }, cx);
  assert.equal(f.chat.restoring.anchor, first, "the second tick waits for the first slide to settle");
  assert.equal(f.chat.windowTop, top);
});

test("the live page moving again while a gap is filled leaves no hole", async () => {
  const key = "claude:1793611000000-0";
  const all = Array.from({ length: 22 }, (_, i) => said(i));
  const session = (args) => ({ session: { key, messages: all.slice(args.before - args.limit, args.before), cursor: { from: args.before - args.limit, to: args.before, total: 22, more: true } } });
  const f = await subjectFixture({ views: { session } });
  f.chat.open(key, f.cx);
  const { cx } = reading(f);
  f.chat.older = { key, messages: all.slice(0, 8), from: 0, to: 8, more: false };
  f.chat.sessions = { session: { key, messages: all.slice(10), cursor: { from: 10, to: 20, total: 20, more: true } } };
  f.chat.fillGap(cx);
  // Another message comes before the fill is back: the live page now starts at 12.
  f.chat.sessions = { session: { key, messages: all.slice(12), cursor: { from: 12, to: 22, total: 22, more: true } } };
  await f.settled();
  await f.settled();
  const texts = JSON.parse(JSON.stringify(f.chat.messagesOf(f.chat.session()).map((m) => m.text)));
  assert.deepEqual(texts, all.map((m) => m.text), "every message once, in order");
});

test("the new-messages count follows messages: a turn that ends and turns its fold into an answer keeps counting", async () => {
  const f = await subjectFixture();
  const { cx } = reading(f);
  const base = [{ role: "user", text: "Explain it", at: at(0) }, { role: "assistant", text: "Looking…", at: at(5) }];
  draw(f, cx, base, f.mod.rowsOf(base, { working: true }));
  f.chat.pinned = false;
  f.chat.seenMsg = f.chat.lastMessage();
  const more = [...base, { role: "assistant", text: "Here is how it works.", at: at(9) }];
  draw(f, cx, more, f.mod.rowsOf(more, { working: false }));
  assert.equal(f.chat.unseen(), 1, "the answer that came is new, whichever row now holds it");
});

test("⎋ closes the hints before anything else, and stops the turn only when nothing is open", async () => {
  const key = "claude:1793611000000-0";
  const f = await subjectFixture();
  f.chat.open(key, f.cx);
  Object.assign(f.chat, { confirm: null, handle: { is_focused: () => false }, cardFocus: { is_focused: () => false, focus() {} }, boxFocused: true });
  f.chat.model.shown = () => null;
  f.chat.sessions = { session: { key, working: true, messages: [], commands: [{ name: "review" }] } };
  f.type("/re");
  assert.equal(f.chat.hints().items.length, 1, "the hints show");
  assert.equal(f.chat.pressed({ keystroke: "escape" }, f.cx), true);
  assert.equal(f.chat.hints().items.length, 0, "⎋ put them away");
  f.chat.ending = { key };
  assert.equal(f.chat.pressed({ keystroke: "escape" }, f.cx), true);
  assert.equal(f.chat.ending, null, "then the question under the header");
  await f.settled();
  assert.deepEqual(f.acts, [], "nothing was stopped yet");
  assert.equal(f.chat.pressed({ keystroke: "escape" }, f.cx), true);
  await f.settled();
  assert.deepEqual(JSON.parse(JSON.stringify(f.acts.map((a) => a[1]))), ["interrupt"], "with nothing open, the turn");
});

test("⏎ with a hint showing completes it, and never sends the half-typed command", async () => {
  const key = "claude:1793611000000-0";
  const f = await subjectFixture();
  f.chat.open(key, f.cx);
  f.chat.sessions = { session: { key, working: false, messages: [], commands: [{ name: "review" }] } };
  f.chat.model.row = () => ({ terminal: key, driven: true, status: "idle" });
  f.type("/re");
  f.chat.send(f.cx, "key");
  await f.settled();
  assert.deepEqual(f.acts, [], "nothing sent");
  assert.match(f.chat.input.value(), /^\/review/, "the command is completed in the box");
});

test("the question card's keyboard belongs to its question: a newer one starts without it", async () => {
  const f = await subjectFixture();
  const answered = [];
  let q = { ask: "a1", permission: true, choices: [{ n: 1 }, { n: 2 }] };
  const row = { terminal: "app:t41", status: "blocked" };
  f.chat.watch("t41", f.cx);
  f.chat.value = { agent: row };
  Object.assign(f.chat, { confirm: null, handle: { is_focused: () => false }, cardFocus: { is_focused: () => true, focus() {} }, boxFocused: false });
  f.chat.model = { row: () => row, value: {}, shown: () => q, answer: (_cx, _row, n) => answered.push(n) };
  assert.equal(f.chat.pressed({ keystroke: "tab" }, f.cx), true, "Tab puts the keyboard on the card");
  assert.equal(f.chat.pressed({ keystroke: "1" }, f.cx), true);
  assert.deepEqual(answered, [1], "its digit answers the question it was put on");
  q = { ask: "a2", permission: true, choices: [{ n: 1 }, { n: 2 }] };
  assert.equal(f.chat.pressed({ keystroke: "2" }, f.cx), false, "a newer question is not answered by the old card focus");
  assert.deepEqual(answered, [1]);
});
