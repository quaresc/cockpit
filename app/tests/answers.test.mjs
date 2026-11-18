import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import { createContext, SourceTextModule, SyntheticModule } from "node:vm";
import { test } from "node:test";

// The mod loaded in a VM with Herdr's natives stubbed: `shown` is the screen in front, `acts` every hub action sent.
async function fixture({ layout = "cockpit-chat", shown = null, answers = {}, down = false } = {}) {
  const acts = [], notices = [], calls = [], overlays = [];
  const shared = { layout, "cockpit.app": shown };
  // The shown layout as Herdr gives it: the screens' stack with its active child (the reading layout's agent screen
  // holds a stack of its own).
  const tree = () => {
    const now = shared["cockpit.app"];
    if (shared.layout === "cockpit-chat") {
      const inner = now === "chat" || now === "tab";
      return { id: "root", children: [{ id: "screen", type: "stack", active: inner ? "work" : now, children: [{ id: "work", children: [{ id: "view", type: "stack", active: inner ? now : "chat", children: [] }] }] }] };
    }
    return { id: "root", children: [{ id: "center", type: "stack", active: now, children: [{ id: "home" }] }] };
  };
  const context = createContext({ console });
  const imports = {
    herdr: {
      agents: () => [],
      focused: () => ({ tab: "demo-tab", space: "demo-space" }),
      spaces: () => [],
      run: (...args) => calls.push(args),
      setShared: (key, value) => (shared[key] = value),
      shared: (key) => shared[key] ?? null,
      notify: (notice) => notices.push(notice),
      theme: () => ({}),
      layout: () => tree(),
      openOverlay: (overlay) => overlays.push(overlay),
    },
    "./hub.js": {
      act: async (app, action, args) => {
        acts.push({ app, action, args });
        if (down) throw new Error("connect ECONNREFUSED 127.0.0.1");
        return answers[`${app} ${action}`] ?? { note: "ok", error: null };
      },
      openUrl: () => {},
      isUnconfirmed: (e) => Boolean(e?.unconfirmed),
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
  const entry = 'export * from "./fleet.js"; export * from "./front.js"; export { AgentsModel } from "./agentsui.js"; export { default as Chat, digitAnswers } from "./chat.js"; export { default as Home } from "./home.js"; export { keyed, unkey } from "./person.js";';
  const module = new SourceTextModule(entry, { context });
  await module.link(load);
  await module.evaluate();
  return { mod: module.namespace, acts, notices, calls, shared, overlays };
}

// The VM's objects as plain data, so they compare with this realm's.
const plain = (value) => JSON.parse(JSON.stringify(value));

// A context whose spawned work runs at once and whose promises can be awaited.
function context() {
  const work = [];
  const cx = { notify() {}, stop_propagation() {}, spawn: (f) => work.push(f(cx)) };
  return { cx, settled: () => Promise.all(work) };
}

const driven = {
  terminal: "claude:demo-421", address: "claude:demo-421", frontend: "app", driven: true, stage: "needs_you", status: "blocked", since: 1,
  question: { text: "May it run Bash: mix test?", kind: "single", permission: true, cursor: null, ask: "toolu_demo_421", choices: [{ n: 1, label: "Allow once" }, { n: 2, label: "Deny" }] },
};
const terminal = {
  terminal: "app:t41", address: "t41", frontend: "app", stage: "needs_you", status: "blocked", since: 2,
  question: { text: "Which limit?", kind: "single", permission: false, cursor: 1, asked: "0123456789abcdef", choices: [{ n: 1, label: "The plan's default" }, { n: 2, label: "Their peak" }] },
};

test("a question keeps what names it, and a permission has nothing picked", async () => {
  const { mod } = await fixture();
  const q = mod.questionOf(driven);
  assert.equal(q.ask, "toolu_demo_421");
  assert.equal(q.cursor, -1, "no choice is drawn as the one under the cursor");
  const t = mod.questionOf(terminal);
  assert.equal(t.asked, "0123456789abcdef");
  assert.equal(t.cursor, 0, "a plain question keeps its cursor");
});

test("every answer names the question it answers and where it came from", async () => {
  const { mod } = await fixture();
  assert.deepEqual(plain(mod.answerPayload("claude:demo-421", driven.question, { choice: 1 }, "chat:click")), { terminal: "claude:demo-421", choice: 1, ask: "toolu_demo_421", source: "chat:click" });
  assert.deepEqual(plain(mod.answerPayload("app:t41", terminal.question, { choices: [1, 2] }, "needs:click")), { terminal: "app:t41", choices: [1, 2], asked: "0123456789abcdef", source: "needs:click" });
  assert.deepEqual(plain(mod.answerPayload("app:t41", null, { text: "later" }, "home:key")), { terminal: "app:t41", text: "later", source: "home:key" }, "nothing shown: nothing named, so the hub refuses it");
});

test("the question card's answer goes out with its ask, and a refusal says the question changed", async () => {
  const refused = { "agents answer": { note: "", error: "answer: The question changed — look again" } };
  const { mod, acts, notices } = await fixture({ answers: refused });
  const model = Object.create(mod.AgentsModel.prototype);
  Object.assign(model, { view: "needs", busy: new Map(), picks: new Map(), value: { agents: [driven] } });
  const { cx, settled } = context();
  model.answer(cx, driven, 1);
  model.answer(cx, driven, 7);
  await settled();
  assert.equal(acts.length, 1, "a number the question does not offer sends nothing");
  assert.deepEqual(plain(acts[0].args[0]), { terminal: "claude:demo-421", choice: 1, ask: "toolu_demo_421", source: "needs:click" });
  assert.equal(notices[0].title, "The question changed — look again");
});

test("a choice that runs every tool unasked is confirmed only from the step its menu opened, once", async () => {
  const { mod, acts, overlays, notices } = await fixture();
  const full = {
    terminal: "app:t42", address: "t42", frontend: "app", stage: "needs_you", status: "blocked", since: 3,
    question: { text: "Select Approval Mode", kind: "single", permission: true, cursor: null, asked: "fedcba9876543210", preview: "Select Approval Mode", unasked: [3],
      choices: [{ n: 1, label: "Read Only" }, { n: 2, label: "Ask" }, { n: 3, label: "Full Access" }] },
  };
  const model = Object.create(mod.AgentsModel.prototype);
  Object.assign(model, { view: "needs", busy: new Map(), picks: new Map(), value: { agents: [full] } });
  model.row = (t) => [full].find((r) => r.terminal === t);
  const { cx, settled } = context();
  model.answer(cx, full, 3);
  await settled();
  assert.equal(acts.length, 0, "the hub's unasked choice asks first");
  const items = overlays.at(-1).props.items;
  assert.equal(items[0].label, "Cancel", "Cancel first");
  // A window command from outside, in the shape the menu once used, answers nothing.
  model.command("agents|unasked|app:t42|3/fedcba9876543210", cx);
  await settled();
  assert.equal(acts.length, 0, "no key, no answer");
  model.answer(cx, full, 3);
  // As main.js hands it on: the menu's own key for its opening comes off, the confirm's stays in the command.
  const { command: confirm, key } = mod.unkey(overlays.at(-1).props.items[1].command);
  model.command(confirm, cx, key);
  model.command(confirm, cx, key);
  await settled();
  assert.equal(acts.length, 1, "the menu's own key, used once");
  assert.deepEqual(plain(acts[0].args[0]), { terminal: "app:t42", choice: 3, asked: "fedcba9876543210", confirm: true, source: "needs:confirm" });
  // A permission cockpit cannot show in full is answered in its terminal, in the hub's words.
  const cut = { ...full, terminal: "app:t43", question: { ...full.question, unasked: [], cut: true, terminal_only: "Part of what it would run is off its screen: answer it in its terminal" } };
  model.answer(cx, cut, 1);
  await settled();
  assert.equal(acts.length, 1, "nothing sent");
  assert.equal(notices.at(-1).title, cut.question.terminal_only);
});

test("a hub that does not answer allows nothing and says so", async () => {
  const { mod, acts, notices } = await fixture({ down: true });
  const model = Object.create(mod.AgentsModel.prototype);
  Object.assign(model, { view: "sidebar", busy: new Map(), picks: new Map(), value: { agents: [driven] } });
  const { cx, settled } = context();
  model.answer(cx, driven, 1);
  await settled();
  assert.equal(acts.length, 1, "one try, never repeated");
  assert.equal(notices.length, 1);
  assert.equal(notices[0].title, "The cockpit hub did not answer");
  assert.equal(model.busy.size, 0, "the card is free again, still asking");
});

test("a digit answers in the chat only from the person's own key, and a permission only from its card", async () => {
  const { mod } = await fixture();
  const q = terminal.question;
  assert.equal(mod.digitAnswers({ keystroke: "1" }, { question: q, focused: true }), 1, "a question: the empty box's digit answers");
  assert.equal(mod.digitAnswers({ keystroke: "1", injected: true }, { question: q, focused: true }), null, "a key sent from outside the window");
  assert.equal(mod.digitAnswers({ keystroke: "1" }, { question: q, focused: false }), null, "the chat does not hold the keyboard");
  assert.equal(mod.digitAnswers({ keystroke: "2" }, { question: q, focused: true, typed: "run it" }), null, "words in the box");
  assert.equal(mod.digitAnswers({ keystroke: "3" }, { question: q, focused: true }), null, "no such choice");
  assert.equal(mod.digitAnswers({ keystroke: "1" }, { question: null, focused: true }), null, "nothing asked");
  const p = driven.question;
  assert.equal(mod.digitAnswers({ keystroke: "1" }, { question: p, focused: true }), null, "a permission: a digit typed into the empty box allows nothing");
  assert.equal(mod.digitAnswers({ keystroke: "1" }, { question: p, focused: true, card: true }), 1, "its card holds the keyboard (Tab)");
  assert.equal(mod.digitAnswers({ keystroke: "alt-2" }, { question: p, focused: true }), null, "no chord answers: ⌥ types a character");
  assert.equal(mod.digitAnswers({ keystroke: "cmd-2" }, { question: p, focused: true }), null, "⌘ and a digit pick Herdr's tab");
  assert.equal(mod.digitAnswers({ keystroke: "1", is_held: true }, { question: p, card: true }), null, "a key held down repeats, it does not pick");
});

test("a key from outside reaches the view in front and no other", async () => {
  const { mod } = await fixture();
  const chat = { app: "chat", keys: true, at: 1 }, home = { app: "home", keys: true, at: 2 }, recap = { app: "recap", keys: true, overlay: true, at: 3 };
  const panel = { app: "runbook", keys: true, scoped: true, at: 4 }, show = { app: "show", keys: true, at: 5 }, sidebar = { app: "sidebar", keys: false, at: 0 };
  const all = [sidebar, chat, home, panel, show];
  const apps = (list) => list.map((c) => c.app).join(" ");
  assert.equal(apps(mod.front(all, "chat")), "chat", "the chat in front: Home, hidden, hears nothing");
  assert.equal(apps(mod.front([...all, recap], "chat")), "recap", "a popup over it takes the key alone");
  assert.equal(apps(mod.front(all, "home")), "home");
  assert.equal(apps(mod.front(all, "inbox")), "", "a screen with no copy here");
  assert.equal(apps(mod.front(all, "work")), "runbook show", "an agent's terminal in front: only the panes and the panel beside it");
});

test("a copy a layout switch dropped without a word stops counting once it stops beating", async () => {
  const { mod } = await fixture({ shown: "home" });
  const dropped = { app: "home", keys: true, at: 1, beat: Date.now() - 60_000 };
  const live = { app: "home", keys: true, at: 2, beat: Date.now() };
  mod.enter(dropped);
  mod.enter(live);
  const shown = mod.front();
  assert.equal(shown.length, 1);
  assert.equal(shown[0], live, "the live Home takes the key, not the one left behind");
});

test("Home's keys answer nothing while another screen is in front", async () => {
  const { mod } = await fixture({ shown: "chat" });
  const home = Object.create(mod.Home.prototype);
  Object.assign(home, { confirm: null, replying: null });
  assert.equal(home.key({ keystroke: "1" }, context().cx), false);
});

test("Home answers a driven session by its own handle", async () => {
  const { mod, acts } = await fixture({ layout: "cockpit", shown: "home" });
  const [agent] = mod.fleet({ agents: [driven] }, {});
  assert.equal(agent.handle, "claude:demo-421");
  const home = Object.create(mod.Home.prototype);
  Object.assign(home, { busy: new Map(), picks: new Map(), replying: null, toasts: { push() {} }, flashes: { start() {} } });
  const { cx, settled } = context();
  home.answer(cx, agent, { choice: 2 }, "key");
  await settled();
  await new Promise((r) => setTimeout(r, 0));
  assert.deepEqual(plain(acts[0].args[0]), { terminal: "claude:demo-421", choice: 2, ask: "toolu_demo_421", source: "home:key" });
});

test("a mode that runs tools without asking waits for a yes about its own session, and the layout's keys act only on the chat in front", async () => {
  const { mod, acts, shared } = await fixture({ shown: "chat" });
  const chat = Object.create(mod.Chat.prototype);
  const focus = { focus() {}, is_focused: () => true };
  const session = { key: "claude:demo-421", mode: "default", cwd: "/Users/me/code/api", messages: [{ role: "user", text: "Make the daily quota configurable per plan" }] };
  Object.assign(chat, { confirm: null, sessions: { session }, handle: focus, cancelFocus: focus });
  const { cx, settled } = context();
  chat.command("mode:bypassPermissions", cx);
  await settled();
  assert.equal(acts.length, 0, "nothing is set before the confirm");
  assert.deepEqual(plain(chat.confirm), { mode: "bypassPermissions", key: "claude:demo-421", title: "Make the daily quota configurable per plan", folder: "api" });
  chat.command("chat:stop", cx);
  assert.equal(chat.confirm, null, "⎋ cancels the confirm and stops nothing");
  chat.command("mode:auto", cx);
  assert.equal(chat.confirm?.mode, "auto", "auto runs tools without asking too");
  chat.confirmed(false, cx);
  chat.command("mode:bypassPermissions", cx);
  chat.sessions = { session: { ...session, key: "claude:other" } };
  chat.confirmed(true, cx);
  await settled();
  assert.deepEqual(plain(acts.map((a) => [a.action, a.args[0]])), [["mode", { key: "claude:demo-421", mode: "bypassPermissions", confirm: true }]], "the yes goes to the session it was asked about");
  chat.sessions = { session };
  shared["cockpit.app"] = "tab";
  chat.command("chat:mode", cx);
  chat.command("chat:stop", cx);
  await settled();
  assert.equal(acts.length, 1, "with a terminal in front, ⇧⇥ and ⎋ leave the session alone");
  shared["cockpit.app"] = "chat";
  chat.command("chat:mode", cx);
  chat.command("chat:stop", cx);
  await settled();
  assert.equal(acts.length, 1, "a bare window command cycles and stops nothing");
  assert.equal(chat.confirm, null);
  // The palette's entries come with the key it minted when the person picked them.
  const viaPalette = (c) => {
    const { command, key } = mod.unkey(mod.keyed(c));
    chat.command(command, cx, key);
  };
  viaPalette("chat:mode");
  await settled();
  assert.equal(acts.length, 1, "picked from the palette, a mode that lets it edit unasked is set only through the confirm");
  assert.equal(chat.confirm?.mode, "acceptEdits", "in front, ⇧⇥ moves to Edits freely through the confirm");
  chat.confirmed(true, cx);
  await settled();
  assert.deepEqual(plain(acts.at(-1).args[0]), { key: "claude:demo-421", mode: "acceptEdits", confirm: true });
  chat.sessions = { session: { ...session, mode: "acceptEdits" } };
  viaPalette("chat:mode");
  await settled();
  assert.deepEqual(plain(acts.at(-1).args[0]), { key: "claude:demo-421", mode: "plan" }, "a mode that asks as much goes at once");
  // The mode menu's items carry a key made when it opened: picked from it, Edits freely is set at once, once.
  const items = chat.modeItems();
  const edits = items.find((i) => i.command.startsWith("mode:acceptEdits:")).command;
  chat.command(edits, cx);
  await settled();
  assert.deepEqual(plain(acts.at(-1).args[0]), { key: "claude:demo-421", mode: "acceptEdits" });
  const before = acts.length;
  chat.command(edits, cx);
  chat.command("mode:acceptEdits:guessed", cx);
  await settled();
  assert.equal(acts.length, before, "a used or guessed key sets nothing");
  assert.equal(chat.confirm?.mode, "acceptEdits", "it asks instead");
});

test("a terminal-only permission or unreadable wait opens the floating terminal without replacing the chat", async () => {
  for (const row of [
    { ...terminal, question: { ...terminal.question, permission: true, terminal_only: "Review the full command in its terminal" } },
    { ...terminal, question: null, waits: { kind: "terminal", text: "Waiting at its terminal" } },
  ]) {
    const { mod, calls, acts, shared, overlays } = await fixture({ shown: "chat" });
    const model = Object.create(mod.AgentsModel.prototype);
    Object.assign(model, { view: "chat", busy: new Map(), drawn: new Map() });
    const { cx, settled } = context();
    model.go(cx, row);
    await settled();
    assert.equal(shared["cockpit.app"], "chat");
    assert.equal(overlays.at(-1).kind, "window");
    assert.equal(overlays.at(-1).key, "cockpit-chat-terminal");
    assert.equal(overlays.at(-1).layout.children.at(-1).content.terminal, row.address);
    assert.ok(calls.some(([name, arg]) => name === "agent.focus" && arg.terminal === row.address));
    assert.ok(!calls.some(([name, arg]) => name === "layout.select" && arg.child === "tab"));
    assert.ok(acts.every(({ action }) => action === "seen"), "navigation never answers or approves the request");
  }
});

test("readable questions and driven sessions keep their normal chat navigation", async () => {
  for (const row of [terminal, driven]) {
    const { mod, calls, acts, shared } = await fixture({ shown: "chat" });
    const model = Object.create(mod.AgentsModel.prototype);
    Object.assign(model, { view: "chat", busy: new Map(), drawn: new Map() });
    const { cx, settled } = context();
    model.go(cx, row);
    await settled();
    assert.equal(shared["cockpit.app"], "chat");
    assert.ok(!calls.some(([name, arg]) => name === "layout.select" && arg.child === "tab"));
    assert.ok(acts.every(({ action }) => action === "seen"));
  }
});

test("a terminal-only navigation from a window command marks nothing seen", async () => {
  const { mod, acts, shared, overlays } = await fixture({ shown: "chat" });
  const model = Object.create(mod.AgentsModel.prototype);
  Object.assign(model, { view: "chat", busy: new Map(), drawn: new Map() });
  const { cx, settled } = context();
  model.go(cx, { ...terminal, question: { ...terminal.question, terminal_only: "Open its terminal" } }, false);
  await settled();
  assert.equal(shared["cockpit.app"], "chat");
  assert.equal(overlays.at(-1).kind, "window");
  assert.equal(acts.length, 0);
});

test("a complete terminal permission is answered from its chat card with the shown question identity", async () => {
  const { mod, acts } = await fixture({ shown: "chat" });
  const row = { ...terminal, question: { ...terminal.question, permission: true, cut: false, terminal_only: null, preview: "$ mix test test/api/quota_test.exs" } };
  const model = Object.create(mod.AgentsModel.prototype);
  Object.assign(model, { view: "chat", busy: new Map(), picks: new Map(), drawn: new Map() });
  const { cx, settled } = context();
  model.answer(cx, row, 1);
  await settled();
  assert.equal(acts.length, 1);
  assert.equal(acts[0].action, "answer");
  assert.deepEqual(plain(acts[0].args[0]), { terminal: row.terminal, choice: 1, asked: row.question.asked, source: "chat:click" });
});
