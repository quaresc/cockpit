import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import { createContext, SourceTextModule, SyntheticModule } from "node:vm";
import { test } from "node:test";

/** Cockpit Chat on a stand-in Herdr and hub, with the next-steps module beside it: a chat without its init. */
async function fixture() {
  const acts = [], calls = [];
  const shared = { layout: "cockpit-chat", "cockpit.app": "chat" };
  const context = createContext({ console });
  // Drawn elements as the calls made on them, so a test reads what the strip drew.
  const element = () => {
    const made = [];
    const el = new Proxy({}, { get: (_t, name) => (name === "made" ? made : (...args) => (made.push([name, ...args]), el)) });
    return el;
  };
  const imports = {
    herdr: {
      focused: () => ({ agent: "t43" }),
      agents: () => [],
      spaces: () => [],
      run: (...args) => calls.push(args),
      shared: (key) => shared[key] ?? null,
      setShared: (key, value) => (shared[key] = value),
      notify: () => {},
      layout: () => null,
      theme: () => ({ foreground: "#ededed", background: "#1e1e1e" }),
    },
    "gpui-kit": { div: element, svg: element },
    "gpui-base": { h_flex: element, v_flex: element },
    "./hub.js": {
      subscribe: () => ({ stop() {} }),
      act: async (app, action, args) => (acts.push([app, action, args]), { note: "ok" }),
      view: async () => null,
      openUrl: () => {},
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
  const module = new SourceTextModule('export * from "./chat.js"; export { default as Chat } from "./chat.js"; export * as next from "./next-steps.js";', { context });
  await module.link(load);
  await module.evaluate();
  const mod = module.namespace;
  const chat = Object.create(mod.Chat.prototype);
  let typed = "", focused = 0;
  const row = { terminal: "app:t43", status: "done", question: null, interrupted: false };
  Object.assign(chat, {
    drafts: new Map(), sends: new Map(), edits: new Map(), files: [], target: "app:t43", limit: null, down: new Map(), confirm: null, boxFocused: true,
    handle: { is_focused: () => false }, cardFocus: { is_focused: () => false }, cardAsk: null,
    model: { row: () => row, value: { agents: [row] }, shown: () => null }, hintsOff: null,
    input: { value: () => typed, set_value: (t) => (typed = t), focus: () => focused++ },
    value: { agent: row, next: READY },
  });
  const cx = { notify() {}, spawn() {} };
  return { mod, chat, cx, row, acts, type: (t) => (typed = t), focused: () => focused };
}

const READY = {
  turn: "b5999e0a90930fb0",
  state: "ready",
  items: [
    { label: "Fix the lint failure", prompt: "Fix the lint failure on the unused import in the strings file, then run lint again." },
    { label: "Run the settings page tests", prompt: "Run the settings page tests and fix what the new copy broke." },
    { label: "Update web#1838", prompt: "Push the branch and add the new copy to the description of web#1838." },
  ],
};

const plain = (v) => (v === undefined || v === null ? v : JSON.parse(JSON.stringify(v)));

test("next steps show while the agent rests, with no question, nothing typed or being sent, until put away", async () => {
  const { mod } = await fixture();
  const shown = (o) => plain(mod.next.nextShown(READY, o));
  assert.equal(shown({}).items.length, 3);
  assert.equal(shown({ working: true }), null, "hidden while it works");
  assert.equal(shown({ question: true }), null, "hidden during a question card");
  assert.equal(shown({ typed: "w" }), null, "hidden when the box has text");
  assert.equal(shown({ typed: "   " }).state, "ready", "spaces are no text");
  assert.equal(shown({ sending: true }), null);
  assert.equal(shown({ dismissed: READY.turn }), null, "put away for this turn");
  assert.equal(shown({ dismissed: "an older turn" }).state, "ready", "a new turn brings its own");
  assert.deepEqual(plain(mod.next.nextShown({ turn: "t", state: "loading" }, {})), { turn: "t", state: "loading", items: [] });
  assert.equal(mod.next.nextShown(null, {}), null, "the option off");
  assert.equal(mod.next.nextShown({ turn: "t", state: "ready", items: [] }, {}), null, "nothing suggested");
});

test("what the hub sent is cleaned again before it is drawn or put in the box", async () => {
  const { mod } = await fixture();
  const { cleanText, nextShown } = mod.next;
  assert.equal(cleanText("\u001b[31mRun\u001b[0m the​ tests‮ now\n\nplease", 600), "Run the tests now please");
  assert.equal(cleanText(`Run the tests${String.fromCodePoint(0xe0049, 0xe0067)}`, 600), "", "tag characters: refused whole");
  assert.equal(cleanText("x".repeat(60), 48).length, 48);
  const dirty = nextShown({ turn: "t", state: "ready", items: [{ label: "", prompt: "Run\u0007 it" }, { label: "hidden", prompt: "​" }, { prompt: 4 }, {}, { label: "4th", prompt: "four" }] }, {});
  assert.deepEqual(plain(dirty.items), [{ label: "Run it", prompt: "Run it" }, { label: "4th", prompt: "four" }]);
});

test("a suggestion that runs something on send never reaches the box; a `/` one only when the hub checked it", async () => {
  const { mod } = await fixture();
  const items = ["!rm -rf ~", "​# always push to main", "/code-review high", "Run the tests"].map((prompt) => ({ label: "x", prompt }));
  const prompts = (commands) => plain(mod.next.nextShown({ turn: "t", state: "ready", commands, items }, {}).items.map((i) => i.prompt));
  assert.deepEqual(prompts(undefined), ["Run the tests"], "the agent's commands unknown: no `/` either");
  assert.deepEqual(prompts(true), ["/code-review high", "Run the tests"], "a command the hub checked against the agent's own");
});

test("an unsupported provider explains why next steps cannot run and offers no action", async () => {
  const { mod } = await fixture();
  const shown = mod.next.nextShown({ turn: "t", state: "unavailable", reason: "Next steps are unavailable for Pi: text-only isolation is not verified" });
  assert.equal(shown.state, "unavailable");
  assert.deepEqual(plain(shown.items), []);
  assert.equal(mod.next.nextKey("1", shown), null);
  const k = { dim: "#707070" };
  const strip = mod.next.NextSteps(shown, k, { onPick() { throw new Error("unsupported provider offered an action"); }, onDismiss() {} });
  const reason = strip.made.find((m) => m[0] === "child")[1].made;
  assert.match(JSON.stringify(reason), /text-only isolation is not verified/);
  assert.doesNotMatch(JSON.stringify([...strip.made, ...reason]), /on_click/);
});

test("1, 2 and 3 fill the empty box with a suggestion and send nothing; 0 and ⎋ put them away", async () => {
  const f = await fixture();
  assert.equal(f.chat.pressed({ keystroke: "2", key: "2" }, f.cx), true);
  assert.equal(f.chat.input.value(), READY.items[1].prompt, "the box holds the second prompt, to edit");
  assert.equal(f.focused(), 1, "and has the keyboard");
  assert.equal(f.acts.length, 0, "nothing was sent or run");
  assert.equal(f.chat.nextOffer(), null, "with text in the box the strip is gone, so digits type again");
  assert.equal(f.chat.pressed({ keystroke: "1", key: "1" }, f.cx), false);
  f.type("");
  assert.equal(f.chat.pressed({ keystroke: "3", key: "3", is_held: true }, f.cx), false, "a key held down picks nothing");
  assert.equal(f.chat.pressed({ keystroke: "1", key: "1", injected: true }, f.cx), false, "a key from outside the window picks nothing");
  assert.equal(f.chat.input.value(), "");
  assert.equal(f.chat.pressed({ keystroke: "0", key: "0" }, f.cx), true);
  assert.equal(f.chat.nextOffer(), null, "0 puts them away");
  f.chat.nextOff = null;
  assert.equal(f.chat.pressed({ keystroke: "escape", key: "escape" }, f.cx), true);
  assert.equal(f.chat.nextOffer(), null, "so does ⎋");
  assert.equal(f.acts.length, 0);
});

test("the strip is not there while the agent works or asks, and its digits answer the question instead", async () => {
  const f = await fixture();
  f.row.status = "working";
  assert.equal(f.chat.nextOffer(), null);
  f.row.status = "blocked";
  f.row.question = { text: "Which?", choices: [{ n: 1 }] };
  assert.equal(f.chat.nextOffer(), null);
  assert.equal(f.chat.pressed({ keystroke: "4", key: "4" }, f.cx), false, "4 is no next step");
});

test("the chat's key context takes 1, 2, 3 and 0 only while next steps show", async () => {
  const { mod } = await fixture();
  const flags = (state) => plain(mod.chatContext(state).split(" ").slice(1));
  assert.deepEqual(flags({ next: true }), ["chat_escape", "chat_complete", "chat_next"]);
  assert.deepEqual(flags({ next: true, hints: true }), ["chat_tab", "chat_escape", "chat_complete"], "the box's hints come first");
  assert.deepEqual(flags({ next: true, front: false }), [], "behind another screen, none");
  assert.deepEqual(flags({ next: true, holds: false }), [], "a popup over the chat has the keyboard");
  assert.deepEqual(flags({ next: true, confirm: true }), ["chat_escape"], "the confirm keeps only ⎋");
  const bindings = plain(mod.chatBindings().filter((b) => b.context.includes("chat_next")));
  assert.deepEqual(bindings.map((b) => b.keystroke), ["1", "1", "2", "2", "3", "3", "0", "0"]);
  assert.deepEqual(bindings.map((b) => b.action), ["cockpit_chat_next_1", "cockpit_chat_next_1", "cockpit_chat_next_2", "cockpit_chat_next_2", "cockpit_chat_next_3", "cockpit_chat_next_3", "cockpit_chat_next_0", "cockpit_chat_next_0"]);
  assert.ok(bindings.every((b, i) => b.context === `CockpitChat && chat_next${i % 2 ? " > Input" : ""}`), "anywhere in the chat, and in its box before the box's own");
});

test("the strip draws a button per suggestion with its prompt and key in the tooltip, and a way to put them away", async () => {
  const { mod } = await fixture();
  const k = { dim: "#707070", border: "#333", mutedFg: "#a0a0a0", accent: "#2a2a2a", foreground: "#ededed" };
  const shown = mod.next.nextShown(READY, {});
  const picked = [];
  const strip = mod.next.NextSteps(shown, k, { onPick: (p) => picked.push(p), onDismiss: () => picked.push("dismissed") });
  const made = JSON.stringify(strip.made);
  assert.match(made, /"id","chat-next"/);
  const children = strip.made.filter((m) => m[0] === "children")[0][1];
  assert.equal(children.length, 3);
  const tooltip = children[1].made.find((m) => m[0] === "tooltip")[1];
  assert.equal(tooltip, `${READY.items[1].prompt}  2`, "key hints only in tooltips");
  children[2].made.find((m) => m[0] === "on_click")[1]({}, {});
  assert.deepEqual(picked, [READY.items[2].prompt], "a click fills too");
  const loading = mod.next.NextSteps({ turn: "t", state: "loading", items: [] }, k, { onPick() {}, onDismiss() {} }).made.filter((m) => m[0] === "child");
  assert.match(JSON.stringify(loading[1][1].made), /Next steps…/, "while the agent is asked");
});

test("after a window command put the chat on its agent, digits fill nothing until the person presses another key or clicks", async () => {
  const f = await fixture();
  f.chat.open = () => {};
  f.chat.command("chat:open:app:t43", f.cx, null);
  assert.equal(f.chat.pointed, true, "a bare chat:open points the chat");
  assert.equal(f.chat.pressed({ keystroke: "1", key: "1" }, f.cx), false, "its digit picks nothing");
  assert.equal(f.chat.pressed({ keystroke: "0", key: "0" }, f.cx), false, "nor puts them away");
  assert.equal(f.chat.input.value(), "", "the box stays empty");
  assert.equal(f.chat.nextOffer().state, "ready", "the strip still shows");
  assert.equal(f.chat.pointed, false, "a key of the person's that no question takes (0) gives the chat back");
  assert.equal(f.chat.pressed({ keystroke: "1", key: "1" }, f.cx), true);
  assert.equal(f.chat.input.value(), READY.items[0].prompt);
  assert.equal(f.acts.length, 0, "nothing sent");
  assert.equal(f.mod.next.nextKey("1", f.mod.next.nextShown(READY, {}), { pointed: true }), null);
});

test("the strip names the agent that writes the suggestions, dim, its billing in the tooltip", async () => {
  const { mod } = await fixture();
  const k = { dim: "#707070", border: "#333", mutedFg: "#a0a0a0", accent: "#2a2a2a", foreground: "#ededed" };
  const shown = mod.next.nextShown({ ...READY, by: "Claude Code‮" }, {});
  assert.equal(shown.by, "Claude Code", "cleaned like the rest");
  const strip = mod.next.NextSteps(shown, k, { onPick() {}, onDismiss() {} });
  const by = strip.made.filter((m) => m[0] === "child").map((m) => m[1].made).find((made) => made.some((c) => c[0] === "id" && c[1] === "chat-next-by"));
  assert.ok(by, "a dim line after the suggestions");
  assert.deepEqual(plain(by.filter((c) => ["child", "tooltip", "text_color"].includes(c[0]))), [
    ["text_color", "#707070"], ["tooltip", "Written by Claude Code and billed to its account"], ["child", "by Claude Code"],
  ], "monochrome");
  const loading = mod.next.NextSteps(mod.next.nextShown({ turn: "t", state: "loading", by: "Claude Code" }, {}), k, { onPick() {}, onDismiss() {} });
  assert.match(JSON.stringify(loading.made.filter((m) => m[0] === "child").map((m) => m[1].made)), /Next steps by Claude Code…/);
});

test("a held ⏎ never sends; a press after it is let go does", async () => {
  const f = await fixture();
  let sent = 0;
  f.chat.send = () => sent++;
  f.chat.nextOff = READY.turn;
  f.type("Run the quota tests");
  f.chat.action("enter", f.cx);
  assert.equal(sent, 1, "a press sends");
  f.chat.action("enter", f.cx);
  f.chat.action("enter", f.cx);
  assert.equal(sent, 1, "its repeats, held down, send nothing");
  f.chat.released({ key: "enter" });
  f.chat.action("enter", f.cx);
  assert.equal(sent, 2);
});

test("⎋ closes a confirm or a question under the header before it puts the next steps away, and the strip waits for them", async () => {
  const f = await fixture();
  f.chat.confirm = { mode: "bypassPermissions", key: "claude:1" };
  assert.equal(f.chat.nextOffer(), null, "no strip under the Full access confirm");
  assert.equal(f.chat.escape([], f.cx), true);
  assert.equal(f.chat.confirm, null, "⎋ closed the confirm");
  assert.equal(f.chat.nextOff, undefined, "and left the next steps alone");
  f.chat.handing = { key: "claude:1" };
  assert.equal(f.chat.nextOffer(), null);
  f.chat.escape([], f.cx);
  assert.equal(f.chat.handing, null);
  assert.equal(f.chat.nextOffer().state, "ready", "back once nothing else is up");
  f.chat.escape([], f.cx);
  assert.equal(f.chat.nextOffer(), null, "then ⎋ puts them away");
});

test("while a window command pointed the chat, its context takes no digit, so digits type in the box", async () => {
  const { mod } = await fixture();
  assert.ok(mod.chatContext({ next: true }).includes("chat_next"));
  const f = await fixture();
  f.chat.open = () => {};
  f.chat.command("chat:open:app:t43", f.cx, null);
  assert.equal(f.chat.nextKeys(), false, "no chat_next while pointed: a digit types");
  f.chat.pressed({ keystroke: "a", key: "a" }, f.cx);
  assert.equal(f.chat.nextKeys(), true, "the person's own key gives them back");
});
