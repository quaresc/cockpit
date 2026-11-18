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
  const earlier = [{ role: "user", text: "Look at the export", at: "2026-01-01T09:00:00Z" }, { role: "assistant", text: "Done.", at: "2026-01-01T09:01:00Z" }];
  assert.deepEqual(chat.rowsOf([...earlier, ...TURN]).slice(-3).map(r => r.key), rows.map(r => r.key), "a row keeps its key when older ones load above it");
  const twice = chat.rowsOf([{ role: "user", text: "continue", at: "" }, { role: "user", text: "continue", at: "" }]);
  assert.notEqual(twice[0].key, twice[1].key, "the same words twice are two rows");
});

test("a turn still running keeps its words in the fold and counts up to now", async () => {
  const { chat } = await fixture();
  const rows = chat.rowsOf(TURN.slice(0, 4), { working: true, now: "2026-01-01T10:00:42Z" });
  assert.equal(rows.map(r => r.kind).join(" "), "said work", "nothing is the answer yet");
  assert.equal(rows[1].done, false);
  assert.equal(rows[1].notes.length, 1);
  assert.equal(chat.workedFor(rows[1].secs, rows[1].done), "Working for 40s");
});

test("a turn you stopped keeps its words in the fold and ends on a stopped mark, not a bubble of yours", async () => {
  const { chat } = await fixture();
  const stop = { role: "user", text: "[Request interrupted by user for tool use]", at: "2026-01-01T10:00:05Z" };
  const rows = chat.rowsOf([...TURN.slice(0, 4), stop, { role: "user", text: "Try again", at: "2026-01-01T10:01:00Z" }]);
  assert.equal(rows.map(r => r.kind).join(" "), "said work stopped said");
  assert.equal(rows[1].done, true);
  assert.equal(rows[1].notes.map(n => n.text).join(" "), "Looking at it.", "nothing it said reads as its answer");
  assert.equal(rows[2].text, undefined, "the marker's words are never drawn");
  assert.equal(chat.rowsOf([{ ...stop, text: "[Request interrupted by user]" }])[0].kind, "stopped");
  assert.equal(chat.rowsOf([{ ...stop, text: "[Request interrupted by user] but go on" }])[0].kind, "said", "words of the person's own stay theirs");
});

test("a driven turn that was stopped or failed ends on the same mark, drawn once", async () => {
  const { chat } = await fixture();
  const stopped = chat.rowsOf(TURN.slice(0, 4), { stopped: true });
  assert.equal(stopped.map(r => r.kind).join(" "), "said work stopped");
  assert.equal(stopped[2].failed, undefined);
  const failed = chat.rowsOf(TURN.slice(0, 4), { failed: true, error: "credit balance too low" });
  assert.deepEqual([failed[2].kind, failed[2].failed, failed[2].error], ["stopped", true, "credit balance too low"]);
  const marked = [...TURN.slice(0, 4), { role: "user", text: "[Request interrupted by user]", at: "2026-01-01T10:00:05Z" }];
  assert.equal(chat.rowsOf(marked, { stopped: true }).filter(r => r.kind === "stopped").length, 1);
  assert.equal(chat.rowsOf(TURN.slice(0, 4), { stopped: true, working: true }).at(-1).kind, "work", "a turn running again is not stopped");
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

// The chat with one subject: a VM whose shared state, focus and hub can be set, and an input that holds what is typed.
async function subjectFixture({ focusedOn = "t41", answers = {}, views = {} } = {}) {
  const calls = [], acts = [];
  const subscriptions = [];
  const shared = { layout: "cockpit-chat" };
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
      subscribe: (_cx, spec, receive, status) => {
        calls.push(["subscribe", spec]);
        const sub = { receive, status, stopped: false, stop() { this.stopped = true; } };
        subscriptions.push(sub);
        return sub;
      },
      act: async (app, action, args) => (acts.push([app, action, args]), answers[`${app} ${action}`] ?? { note: "ok" }),
      view: async (name, args) => (calls.push(["view", name, args]), views[name] ?? null),
      openUrl: () => {},
      sleep: async () => {},
      install: async () => ({}),
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
  const module = new SourceTextModule('export * from "./chat.js"; export { default as Chat } from "./chat.js"; export { pinChat } from "./screens.js"; export { goToAgent } from "./fleet.js";', { context });
  await module.link(load);
  await module.evaluate();
  const mod = module.namespace;
  // A chat without its init: what the tests need of it, an input that keeps its text.
  const chat = Object.create(mod.Chat.prototype);
  let typed = "";
  Object.assign(chat, { drafts: new Map(), sends: new Map(), edits: new Map(), files: [], target: undefined, limit: null, model: { row: () => null, value: null }, input: { value: () => typed, set_value: (t) => (typed = t), focus() {} } });
  const work = [];
  const cx = { notify() {}, spawn: (f) => work.push(f(cx)), timer: { after: () => {} } };
  return { mod, chat, cx, calls, acts, shared, focus, subscriptions, type: (t) => (typed = t), settled: () => Promise.all(work) };
}

test("a late transcript or connection status from the previous subject cannot replace the current conversation", async () => {
  const f = await subjectFixture();
  f.chat.watch("t41", f.cx);
  const old = f.subscriptions[0];
  f.chat.watch("t42", f.cx);
  const value = { agent: { terminal: "app:t42" }, messages: [{ text: "Current conversation" }] };
  f.subscriptions[1].receive({ view: "chat", value }, f.cx);
  f.chat.hubState = "live";
  old.receive({ view: "chat", value: { messages: [{ text: "Other session" }] } }, f.cx);
  old.status("error: disconnected", f.cx);
  assert.equal(f.chat.value, value);
  assert.equal(f.chat.hubState, "live");
  assert.equal(old.stopped, true);
});

test("a driven session is known by the agents view's row, not by the shape of its key", async () => {
  const f = await subjectFixture();
  f.chat.model.value = { agents: [] };
  f.chat.model.row = (k) => (k === "codex:17936-2" ? { terminal: k, driven: true } : k === "w1:p2" ? { terminal: k } : null);
  assert.deepEqual(["codex:17936-2", "w1:p2", "claude:unknown"].map((k) => f.chat.driven(k)), [true, false, false]);
  f.chat.model.value = null;
  assert.equal(f.chat.driven("claude:unknown"), null, "before the agents view says, nobody knows");
});

test("the chat follows the agent chat:open named until another one comes in front", async () => {
  const f = await subjectFixture({ focusedOn: "t41" });
  f.chat.model.row = (k) => (String(k).startsWith("claude:") ? { terminal: k, driven: true } : null);
  assert.equal(f.chat.wanted(), "t41", "the agent in front, by default");
  f.chat.open("claude:1793611000000-0", f.cx);
  assert.equal(f.chat.wanted(), "claude:1793611000000-0");
  assert.equal(f.shared["cockpit.chat.subject"], "claude:1793611000000-0", "the runner and the review panel read it here");
  const views = f.calls.filter((c) => c[0] === "subscribe").at(-1)[1].views;
  assert.deepEqual(JSON.parse(JSON.stringify(views)).map((v) => v.name ?? v), ["chat", "queue", "session"], "a driven subject has its session followed, by its key");
  assert.equal(views[2].args.key, "claude:1793611000000-0");
  // Once the focus had time to settle with the pin, another agent coming in front takes over.
  f.shared["cockpit.chat.pinned"].at -= 3_000;
  Object.assign(f.focus, { agent: "t42", terminal: "t42", tab: "tab-t42" });
  assert.equal(f.chat.wanted(), "t42", "another agent in front takes over");
});

test("on a tab no agent runs in, the chat follows none, never the first agent Herdr names for its blocks", async () => {
  const f = await subjectFixture({ focusedOn: "t41" });
  assert.equal(f.chat.wanted(), "t41");
  // Tab "3" runs a shell: Herdr's focused().agent still names an agent (the one picked before, else the first).
  Object.assign(f.focus, { agent: "t41", terminal: "t11", tab: "tab-3" });
  assert.equal(f.chat.wanted(), null, "the empty chat, as the lit pill of a tab with no agent says");
  // A tab with an agent and a shell beside it, the shell focused: the tab's agent.
  Object.assign(f.focus, { agent: "t41", terminal: "t12", tab: "tab-t42" });
  assert.equal(f.chat.wanted(), "t42");
  // A driven chat opened from a shell tab stays through focus changes inside that tab.
  const key = "claude:1793611000000-0";
  f.chat.model.row = (k) => (k === key ? { terminal: k, driven: true } : null);
  Object.assign(f.focus, { agent: "t41", terminal: "t11", tab: "tab-3" });
  f.chat.open(key, f.cx);
  assert.equal(f.shared["cockpit.chat.pinned"].focus, null);
  f.shared["cockpit.chat.pinned"].at -= 3_000;
  Object.assign(f.focus, { agent: "t41", terminal: "t13", tab: "tab-3" });
  assert.equal(f.chat.wanted(), key, "another shell pane in the same tab: kept");
  Object.assign(f.focus, { tab: "tab-web" });
  assert.equal(f.chat.wanted(), null, "native navigation to another shell tab ends the pin");
  Object.assign(f.focus, { agent: "t45", terminal: "t45", tab: "tab-t45" });
  assert.equal(f.chat.wanted(), "t45", "an agent in front takes over");
});

test("native navigation to a shell tab or another space ends even a freshly pinned watched conversation", async () => {
  for (const change of [{ tab: "tab-3" }, { space: "s2" }]) {
    const f = await subjectFixture();
    f.focus.space = "s1";
    f.chat.open("t41", f.cx);
    Object.assign(f.focus, { agent: "t41", terminal: "t11", tab: "tab-t41", ...change });
    if (change.space) Object.assign(f.focus, { tab: "tab-3" });
    assert.equal(f.chat.wanted(), null);
    assert.equal(f.shared["cockpit.chat.pinned"], null);
  }
});

test("the chat keeps the session the start box followed through a hub restart, whatever terminal the focus lands on", async () => {
  const key = "claude:1794648772681-0";
  const f = await subjectFixture({ focusedOn: null });
  f.focus.terminal = "t1";
  let listed = false;
  const rows = () => [{ terminal: "app:t41", address: "t41" }, ...(listed ? [{ terminal: key, address: key, driven: true }] : [])];
  f.chat.model = { value: { agents: [] }, down: false, get rows() { return rows(); }, row: (k) => rows().find((r) => r.terminal === k) ?? null };
  // The start box's follow, once the start made the session: the same open a pill, ⌘J, Needs or the palette ends in.
  f.chat.open(key, f.cx);
  assert.equal(f.chat.wanted(), key, "not listed yet, just started: followed");
  listed = true;
  assert.equal(f.chat.wanted(), key);
  f.shared["cockpit.chat.pinned"].at -= 3_000;
  // cockpit hub stop: Herdr's own rows, no session; the focus moves to another shell.
  listed = false;
  f.chat.model.down = true;
  f.focus.terminal = "t9";
  assert.equal(f.chat.wanted(), key, "the hub down and a shell in front: kept");
  // Start it: the new hub lists the session paused a moment later.
  f.chat.model.down = false;
  assert.equal(f.chat.wanted(), key, "back, not listed yet: kept");
  listed = true;
  assert.equal(f.chat.wanted(), key, "listed again, paused: followed");
  // An agent coming in front, as Herdr's own tab keys do, still takes over.
  f.focus.terminal = "t41";
  assert.equal(f.chat.wanted(), "t41");
});

test("each subject keeps what was typed for it", async () => {
  const f = await subjectFixture();
  f.chat.watch("t41", f.cx);
  f.type("Make the limit configurable");
  f.chat.watch("t42", f.cx);
  assert.equal(f.chat.input.value(), "", "B starts empty");
  f.type("Rename the flag");
  f.chat.watch("t41", f.cx);
  assert.equal(f.chat.input.value(), "Make the limit configurable", "back on A, its draft is intact");
  f.chat.watch("t42", f.cx);
  assert.equal(f.chat.input.value(), "Rename the flag");
});

test("a prompt goes through the queue, and one the hub refuses stays in the box with Retry", async () => {
  const f = await subjectFixture({ answers: { "queue tell": { error: "tell: Daily quotas waits on a question" } } });
  f.chat.watch("t41", f.cx);
  f.chat.value = { agent: { terminal: "app:t41", status: "idle" } };
  f.chat.model.row = () => ({ terminal: "app:t41", status: "idle" });
  f.type("Run the tests");
  f.chat.send(f.cx);
  assert.equal(f.chat.sent.state, "sending");
  await f.settled();
  assert.deepEqual(JSON.parse(JSON.stringify(f.acts[0])), ["queue", "tell", ["app:t41", "Run the tests"]], "told, never flushed with send all");
  assert.equal(f.chat.sent.state, "failed");
  assert.equal(f.chat.sent.why, "Daily quotas waits on a question");
  assert.equal(f.chat.input.value(), "Run the tests", "the words stay to be sent again");
});

test("a prompt the queue kept leaves the box and says where it stands", async () => {
  const f = await subjectFixture({ answers: { "queue tell": { note: "queued: its queue is paused (the last turn failed), resume to send" } } });
  f.chat.watch("t41", f.cx);
  f.chat.value = { agent: { terminal: "app:t41", status: "idle" } };
  f.chat.model.row = () => ({ terminal: "app:t41", status: "idle" });
  f.type("Run the tests");
  f.chat.send(f.cx);
  await f.settled();
  assert.equal(f.chat.input.value(), "", "queued, it is not offered again");
  assert.equal(f.chat.sent.state, "queued");
  assert.match(f.chat.sent.note, /paused/);
});

test("words sent to a driven session wait for its replay", async () => {
  const { chat } = await fixture();
  const rows = chat.rowsOf([{ role: "user", text: "Make it configurable", at: "2026-01-01T10:00:00Z", echo: true }]);
  assert.equal(rows[0].pending, true);
  assert.equal(chat.rowsOf([{ role: "user", text: "Make it configurable", at: "2026-01-01T10:00:00Z" }])[0].pending, undefined);
});

/** A cx whose timers wait to be run in order, and a scroll handle that remembers where it was sent. */
function reading(f) {
  const timers = [], scrolled = [];
  const cx = { notify() {}, spawn: f.cx.spawn, timer: { after: (ms, fn) => (timers.push([ms, fn]), { cancel() {} }) } };
  const flush = () => {
    for (const [, fn] of timers.splice(0).sort((a, b) => a[0] - b[0])) fn();
  };
  Object.assign(f.chat, { scroll: { scroll_to_item: (i) => scrolled.push(i) }, pinned: true, top: 0, viewH: 300, heights: new Map(), seenKey: null, rows: [], all: [], windowTop: null, windowAround: null, restoring: null, loadingEarlier: false, older: null });
  return { cx, flush, scrolled };
}

const said = (n) => ({ role: "user", text: `Message ${n}`, at: `2026-01-01T10:${String(Math.floor(n / 60)).padStart(2, "0")}:${String(n % 60).padStart(2, "0")}Z` });

test("reading back is never pulled down by new rows, which are counted until the reader goes back", async () => {
  const f = await subjectFixture();
  const { cx, flush, scrolled } = reading(f);
  const ten = Array.from({ length: 10 }, (_, i) => said(i));
  const rows = f.mod.rowsOf(ten);
  f.chat.settle(rows, cx);
  f.chat.rows = f.chat.all = rows;
  f.chat.messagesNow = ten;
  for (const r of rows) f.chat.heights.set(r.key, 100);
  flush();
  assert.equal(scrolled.at(-1), 10, "pinned, the view goes to the end");
  scrolled.length = 0;
  f.chat.wheeled({ delta: { y: 250 } }, cx);
  assert.equal(f.chat.pinned, false, "the wheel up leaves the end");
  const twelve = Array.from({ length: 12 }, (_, i) => said(i));
  const more = f.mod.rowsOf(twelve);
  f.chat.settle(more, cx);
  f.chat.rows = f.chat.all = more;
  f.chat.messagesNow = twelve;
  flush();
  assert.deepEqual(scrolled, [], "nothing moves the reader");
  assert.equal(f.chat.unseen(), 2, "two new messages wait");
  f.chat.toEnd(cx);
  flush();
  assert.equal(f.chat.pinned, true);
  assert.equal(scrolled.at(-1), 12, "back to the end");
  f.chat.wheeled({ delta: { y: -50 } }, cx);
  assert.equal(f.chat.pinned, true, "down at the end it stays pinned");
});

test("earlier pages of a driven session come in above, once at a time, the first row kept where it was", async () => {
  const key = "claude:1793611000000-0";
  const all = Array.from({ length: 12 }, (_, i) => said(i));
  const page = { session: { key, messages: all.slice(0, 8), cursor: { from: 0, to: 8, total: 12, more: false } } };
  const f = await subjectFixture({ views: { session: page } });
  f.chat.open(key, f.cx);
  const { cx, flush, scrolled } = reading(f);
  f.chat.sessions = { session: { key, messages: all.slice(8), cursor: { from: 8, to: 12, total: 12, more: true } } };
  const live = f.mod.rowsOf(f.chat.messagesOf(f.chat.session()));
  f.chat.settle(live, cx);
  f.chat.rows = f.chat.all = live;
  flush();
  scrolled.length = 0;
  assert.equal(f.chat.hasEarlier(), true);
  f.chat.earlier(cx);
  f.chat.earlier(cx);
  await f.settled();
  const asked = f.calls.filter((c) => c[0] === "view");
  assert.deepEqual(JSON.parse(JSON.stringify(asked)), [["view", "session", { key, before: 8, limit: 400 }]], "one page, from before the oldest shown");
  const rows = f.mod.rowsOf(f.chat.messagesOf(f.chat.session()));
  assert.equal(rows.length, 12, "the older page comes first");
  assert.equal(f.chat.hasEarlier(), false);
  f.chat.settle(rows, cx);
  f.chat.rows = f.chat.all = rows;
  flush();
  assert.deepEqual(scrolled.slice(0, 2), [12, 8], "to the end, then back up to the row that was first");
});

test("the live page moving on leaves no gap after the older pages", async () => {
  const key = "claude:1793611000000-0";
  const all = Array.from({ length: 14 }, (_, i) => said(i));
  const f = await subjectFixture({ views: { session: { session: { key, messages: all.slice(8, 10), cursor: { from: 8, to: 10, total: 14, more: true } } } } });
  f.chat.open(key, f.cx);
  const { cx } = reading(f);
  f.chat.older = { key, messages: all.slice(0, 8), from: 0, to: 8, more: false };
  f.chat.sessions = { session: { key, messages: all.slice(10), cursor: { from: 10, to: 14, total: 14, more: true } } };
  f.chat.fillGap(cx);
  await f.settled();
  assert.deepEqual(JSON.parse(JSON.stringify(f.calls.filter((c) => c[0] === "view"))), [["view", "session", { key, before: 10, limit: 2 }]]);
  assert.deepEqual(JSON.parse(JSON.stringify(f.chat.messagesOf(f.chat.session()).map((m) => m.text))), all.map((m) => m.text));
});

test("the chat's own keys: ⇧⇥ cycles a driven session's mode, ⎋ stops a turn or closes the confirm, keys from outside do nothing", async () => {
  const key = "claude:1793611000000-0";
  const f = await subjectFixture();
  f.chat.open(key, f.cx);
  Object.assign(f.chat, { confirm: null, handle: { is_focused: () => true }, cardFocus: { is_focused: () => false, focus() {} }, boxFocused: true });
  f.chat.model.shown = () => null;
  f.chat.sessions = { session: { key, working: true, messages: [] } };
  f.shared["cockpit.app"] = "chat";
  assert.equal(f.chat.pressed({ keystroke: "shift-tab", injected: true }, f.cx), false, "a key from outside");
  assert.equal(f.chat.pressed({ keystroke: "shift-tab" }, f.cx), true);
  assert.equal(f.chat.pressed({ keystroke: "escape" }, f.cx), true);
  await f.settled();
  assert.deepEqual(JSON.parse(JSON.stringify(f.acts.map((a) => [a[1], a[2][0]]))), [["mode", { key }], ["interrupt", { key }]]);
  f.chat.confirm = { mode: "bypassPermissions", key };
  assert.equal(f.chat.pressed({ keystroke: "escape" }, f.cx), true);
  assert.equal(f.chat.confirm, null, "⎋ closes the confirm and stops nothing");
  assert.equal(f.acts.length, 2);
  f.shared["cockpit.app"] = "tab";
  assert.equal(f.chat.pressed({ keystroke: "shift-tab" }, f.cx), false, "behind another screen, the chat takes no keys");
});

test("the chat's keys are bindings of its own context, so the box's Tab, ⏎ and ⎋ and the window's focus moves come after them", async () => {
  const f = await subjectFixture();
  const bindings = f.mod.chatBindings();
  for (const b of bindings) assert.match(b.context, /^CockpitChat && chat_[a-z]+( > Input)?$/, `${b.keystroke} stays in the chat: ${b.context}`);
  const where = (keystroke) => JSON.parse(JSON.stringify(bindings.filter((b) => b.keystroke === keystroke).map((b) => b.context.endsWith("> Input"))));
  assert.deepEqual(where("tab"), [false, true], "Tab: anywhere in the chat, and in its box before the box's own");
  assert.deepEqual(where("shift-tab"), [false, true]);
  assert.deepEqual(where("escape"), [false, true]);
  assert.deepEqual(where("enter"), [true], "⏎ is the chat's only in its box");
  const flags = (state) => JSON.parse(JSON.stringify(f.mod.chatContext(state).split(" ").slice(1)));
  assert.deepEqual(flags({}), ["chat_complete"], "⏎ sends from the box, ⇧⏎ stays the box's new line");
  assert.deepEqual(flags({ hints: true }), ["chat_tab", "chat_escape", "chat_complete"], "hints: Tab and ⏎ complete, ⎋ closes them");
  assert.deepEqual(flags({ question: true, live: true }), ["chat_tab", "chat_cycle", "chat_complete"], "a question takes Tab, a driven session ⇧⇥");
  assert.deepEqual(flags({ closes: true }), ["chat_escape", "chat_complete"], "a working turn or a question under the header takes ⎋");
  assert.deepEqual(flags({ confirm: true, hints: true, live: true }), ["chat_escape"], "the confirm keeps only ⎋");
  assert.deepEqual(flags({ front: false, hints: true, question: true, live: true, closes: true }), [], "behind another screen, none");
  assert.deepEqual(flags({ front: false, confirm: true }), ["chat_escape"]);
  assert.deepEqual(flags({ holds: false, hints: true, question: true, live: true, closes: true }), [], "the palette or a menu over it has the keyboard: none");
});

test("a chat key pressed for real takes the chat's path: Tab to the card, ⇧⇥ once per press, ⏎ completing and never sending", async () => {
  const key = "claude:1793611000000-0";
  const f = await subjectFixture();
  f.chat.open(key, f.cx);
  let focusedCard = false;
  Object.assign(f.chat, { confirm: null, down: new Map(), handle: { is_focused: () => false }, cardFocus: { is_focused: () => focusedCard, focus: () => (focusedCard = true) }, boxFocused: true });
  const q = { ask: "a1", permission: true, choices: [{ n: 1 }, { n: 2 }] };
  const answered = [];
  f.chat.model = { row: () => ({ terminal: key, driven: true, status: "blocked" }), value: {}, shown: () => q, answer: (_cx, _row, n) => answered.push(n) };
  f.chat.sessions = { session: { key, working: false, messages: [], commands: [{ name: "review" }] } };
  f.shared["cockpit.app"] = "chat";
  f.chat.action("tab", f.cx);
  assert.equal(focusedCard, true, "Tab puts the keyboard on the question card");
  assert.equal(f.chat.pressed({ keystroke: "2", key: "2" }, f.cx), true, "and its digit answers");
  assert.deepEqual(answered, [2]);
  f.chat.action("shift-tab", f.cx);
  f.chat.action("shift-tab", f.cx);
  await f.settled();
  assert.equal(f.acts.filter((a) => a[1] === "mode").length, 1, "a repeat before the key is let go is held down: one cycle");
  f.chat.released({ key: "tab" });
  f.chat.action("shift-tab", f.cx);
  await f.settled();
  assert.equal(f.acts.filter((a) => a[1] === "mode").length, 2, "let go, the next press cycles again");
  f.type("/re");
  f.chat.action("enter", f.cx);
  await f.settled();
  assert.match(f.chat.input.value(), /^\/review/, "⏎ with the hints showing completes");
  assert.equal(f.acts.filter((a) => a[0] === "queue").length, 0, "and sends nothing");
});

test("the chat keeps the session it follows through a hub restart, and one just started until the hub lists it", async () => {
  const key = "claude:1793611000000-0";
  const f = await subjectFixture({ focusedOn: "t41" });
  let listed = true;
  f.chat.model = { value: { agents: [] }, down: false, row: (k) => (listed && k === key ? { terminal: k, driven: true } : null) };
  f.chat.open(key, f.cx);
  assert.equal(f.chat.wanted(), key, "listed: followed");
  // The hub stops: the agents view falls back to Herdr's own rows, which know no session.
  listed = false;
  f.chat.model.down = true;
  assert.equal(f.chat.wanted(), key, "the hub is down: the pin stays");
  // Back, and not listing it yet: the pin stays a few seconds, then goes when the hub still does not list it.
  f.chat.model.down = false;
  const t = Date.now();
  assert.equal(f.chat.pinGone(key, t), false);
  assert.equal(f.chat.pinGone(key, t + 4_000), false, "back, the session may be read again from its registry");
  listed = true;
  assert.equal(f.chat.wanted(), key, "listed again (paused): followed again");
  listed = false;
  assert.equal(f.chat.pinGone(key, t + 10_000), false);
  assert.equal(f.chat.pinGone(key, t + 16_000), true, "a session the hub stopped listing is gone");
  // A session just started is not listed yet: it gets longer to show up.
  const fresh = "claude:1793611000999-0";
  assert.equal(f.chat.pinGone(fresh, t), false);
  assert.equal(f.chat.pinGone(fresh, t + 20_000), false);
  assert.equal(f.chat.pinGone(fresh, t + 31_000), true);
});

test("a fence still being written is drawn as code already", async () => {
  const { mod } = await subjectFixture();
  assert.deepEqual(JSON.parse(JSON.stringify(mod.segments("Run this:\n```sh\nmix test"))), [{ code: false, text: "Run this:\n" }, { code: true, lang: "sh", text: "mix test" }]);
  assert.deepEqual(JSON.parse(JSON.stringify(mod.segments("a\n```js\nx()\n```\nb"))).map((s) => s.code), [false, true, false]);
});

test("a send lands on the subject it went to, and leaves no copy to send twice after A, B, then A", async () => {
  const f = await subjectFixture();
  const rows = { "app:t41": { terminal: "app:t41", status: "idle" }, "app:t42": { terminal: "app:t42", status: "idle" } };
  f.chat.model.row = (t) => rows[t] ?? rows[`app:${t}`] ?? null;
  f.chat.watch("t41", f.cx);
  f.chat.value = { agent: rows["app:t41"] };
  f.type("Run the tests");
  // The hub answers after the reader moved on to B.
  f.chat.send(f.cx);
  assert.equal(f.chat.sent.state, "sending");
  f.chat.watch("t42", f.cx);
  assert.equal(f.chat.sent, null, "B shows nothing of A's send");
  await f.settled();
  assert.equal(f.chat.sends.get("t41"), undefined, "A's send went");
  assert.equal(f.chat.drafts.has("t41"), false, "and A keeps no copy of it");
  f.chat.watch("t41", f.cx);
  assert.equal(f.chat.input.value(), "", "back on A, nothing to send again");
});

test("an edit of a queued prompt keeps the unsent words, Cancel gives them back, and a driven session takes it as an edit", async () => {
  const key = "claude:1793611000000-0";
  const f = await subjectFixture();
  f.chat.open(key, f.cx);
  f.chat.sessions = { session: { key, working: true, messages: [] } };
  f.chat.model.row = () => ({ terminal: key, driven: true, status: "working" });
  f.chat.queues = { queues: { [key]: [{ id: "q1", text: "Then the docs" }] } };
  f.type("Half a thought");
  f.chat.edits.set(f.chat.target, { id: "q1", draft: { text: f.chat.input.value(), files: [] } });
  f.type("Then the docs, and the changelog");
  f.chat.send(f.cx);
  await f.settled();
  assert.deepEqual(JSON.parse(JSON.stringify(f.acts.at(-1))), ["queue", "edit", [key, "q1", "Then the docs, and the changelog"]], "an edit, not a new prompt");
  assert.equal(f.chat.input.value(), "Half a thought", "the words from before the edit come back");
  assert.equal(f.chat.editing, null);
  f.chat.edits.set(f.chat.target, { id: "q1", draft: { text: "Kept", files: [] } });
  f.chat.watch("t41", f.cx);
  f.chat.watch(key, f.cx);
  assert.equal(f.chat.input.value(), "Kept", "leaving mid-edit keeps what was there before it");
  assert.equal(f.chat.editing, null, "and the edit is over");
});

test("words sent while a driven session works wait in its queue behind the others", async () => {
  const key = "claude:1793611000000-0";
  const f = await subjectFixture();
  f.chat.open(key, f.cx);
  f.chat.sessions = { session: { key, working: true, messages: [] } };
  f.chat.model.row = () => ({ terminal: key, driven: true, status: "working" });
  f.type("And the docs");
  f.chat.send(f.cx);
  await f.settled();
  assert.deepEqual(JSON.parse(JSON.stringify(f.acts.at(-1))), ["queue", "tell", [key, "And the docs"]]);
});

test("going to an agent on purpose, even the one in front, unpins the chat; so does a pinned session that ended", async () => {
  const key = "claude:1793611000000-0";
  const f = await subjectFixture({ focusedOn: "t41" });
  let rows = { [key]: { terminal: key, driven: true } };
  f.chat.model.value = { agents: [] };
  f.chat.model.row = (k) => rows[k] ?? null;
  f.chat.open(key, f.cx);
  assert.equal(f.chat.wanted(), key);
  f.mod.goToAgent({ address: "t41" });
  assert.equal(f.shared["cockpit.chat.pinned"], null, "the drawer, Needs, the palette and a tab pill all go through here");
  assert.equal(f.chat.wanted(), "t41");
  f.chat.open(key, f.cx);
  f.chat.driven(key);
  rows = {};
  assert.equal(f.chat.wanted(), key, "unlisted a moment, as while a hub comes back: kept");
  f.chat.pinMissing.since -= 6_000;
  assert.equal(f.chat.wanted(), "t41", "the session it was pinned to is gone: the agent in front again");
  assert.equal(f.shared["cockpit.chat.pinned"], null);
});

test("reading further back keeps the row at the top of the view, wherever the reader is", async () => {
  const f = await subjectFixture();
  const { cx } = reading(f);
  const rows = f.mod.rowsOf(Array.from({ length: 10 }, (_, i) => said(i)));
  f.chat.rows = f.chat.all = rows;
  for (const r of rows) f.chat.heights.set(r.key, 100);
  f.chat.wheeled({ delta: { y: 450 } }, cx);
  assert.equal(f.chat.top, 250, "700 down to the end, then 450 back up");
  assert.equal(f.chat.topMessage(), rows[2].ids[0], "the third row's message is the one at the top");
  f.chat.pinned = true;
  assert.equal(f.chat.topMessage(), rows[0].ids[0], "at the end with everything in view, the first");
});

test("no more than a window of rows is drawn, and it slides over the rest as the reader reads back and comes down", async () => {
  const f = await subjectFixture();
  const { cx } = reading(f);
  const all = f.mod.rowsOf(Array.from({ length: 1000 }, (_, i) => said(i)));
  const draw = (rows) => ((f.chat.rows = f.chat.drawn(rows)), (f.chat.all = rows), f.chat.rows);
  let rows = draw(all);
  assert.equal(rows.length, f.mod.DRAW, "the latest rows only");
  assert.equal(rows[0].key, all[1000 - f.mod.DRAW].key);
  assert.equal(new Set(rows.map((r) => r.slot)).size, f.mod.DRAW, "every drawn row has text views of its own");
  const slotOf = new Map(rows.map((r) => [r.key, r.slot]));
  for (const r of rows) f.chat.heights.set(r.key, 100);
  f.chat.pinned = false;
  f.chat.top = 0;
  f.chat.earlier(cx);
  assert.equal(f.calls.filter((c) => c[0] === "view").length, 0, "rows read already: no trip to the hub");
  assert.equal(f.chat.restoring.anchor, all[1000 - f.mod.DRAW].ids[0], "the message at the top stays there");
  rows = draw(all);
  assert.equal(rows[0].key, all[1000 - f.mod.DRAW - f.mod.DRAW / 2].key, "the window slid up by half");
  assert.ok(rows.filter((r) => slotOf.has(r.key)).every((r) => r.slot === slotOf.get(r.key)), "a row still drawn keeps its text views");
  f.chat.restoring = null;
  f.chat.later(cx);
  rows = draw(all);
  assert.equal(rows[0].key, all[1000 - f.mod.DRAW].key, "and down again");
  f.chat.toEnd(cx);
  assert.equal(f.chat.windowTop, null);
});

test("focusing a floating terminal selects the tab that owns its conversation", async () => {
  const f = await subjectFixture();
  f.focus.tab = "tab-t42";
  f.focus.terminal = "t41";
  assert.equal(f.chat.inView(), "t41");
  assert.deepEqual(JSON.parse(JSON.stringify(f.calls.at(-1))), ["tab.select", { id: "tab-t41" }]);
});
