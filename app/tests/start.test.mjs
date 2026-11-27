import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { createContext, SourceTextModule, SyntheticModule } from "node:vm";
import { test } from "node:test";

import { emptyState, requestOf, applyFix, edited, sourceOf, phase, resolved, stepMark, shortPath, choicesOf, agentOf, takesModel, accountsOf } from "../mods/cockpit/startform.js";

test("the form becomes the request the hub plans", () => {
  const s = edited(edited(emptyState("ENG-412  ", "g1"), "mode", "plan"), "model", " opus ");
  assert.deepEqual(requestOf(s), { id: "g1", text: "ENG-412", worktree: "new", base: "", branch: "", model: "opus", effort: "", repo: "", agent: "", mode: "plan", account: "" });
  const picked = edited(edited(s, "effort", "high"), "fast", "off");
  assert.deepEqual([requestOf(picked).effort, requestOf(picked).fast], ["high", false], "fast mode goes only once picked");
  const other = edited(picked, "agent", "codex");
  assert.deepEqual([other.model, other.effort, other.fast], ["", "", ""], "another agent starts from its own defaults");
  assert.equal(requestOf(edited(s, "here", true)).worktree, "here");
});

test("a fix fills its field or sets its flag, and a new source drops the old fixes", () => {
  let s = emptyState("ENG-412", "g2");
  s = applyFix(s, { branch: "me/eng-412-store-and-enforce-daily-quotas-2" });
  s = applyFix(s, { trust: "1f2e" });
  s = applyFix(s, { repo: "api" });
  const r = requestOf(s);
  assert.deepEqual([r.branch, r.trust, r.repo], ["me/eng-412-store-and-enforce-daily-quotas-2", "1f2e", "api"]);
  assert.equal(requestOf(applyFix(s, { worktree: "new" })).worktree, "new");
  assert.equal(requestOf(edited(s, "source", "ENG-415")).trust, undefined, "another source asks again");
  assert.equal(requestOf(edited(applyFix(s, { dirty: true }), "here", false)).dirty, undefined, "the worktree changed: the checkout's state no longer counts");
});

test("the account goes with the request, and its fix sets it back to the default", () => {
  const work = edited(emptyState("ENG-412", "g4"), "account", "/home/me/.claude-work");
  assert.equal(requestOf(work).account, "/home/me/.claude-work");
  const back = applyFix(work, { account: "" });
  assert.deepEqual([back.account, requestOf(back).account], [null, ""]);
  const claude = { id: "claude", accounts: [{ dir: "/home/me/.claude", name: "Personal", default: true }, { dir: "/home/me/.claude-work", name: "Work", default: false }] };
  assert.deepEqual(accountsOf(claude), [["/home/me/.claude", "Personal (default)"], ["/home/me/.claude-work", "Work"]]);
  assert.deepEqual(accountsOf({ id: "codex", accounts: [] }), []);
  assert.deepEqual(accountsOf(null), []);
});

test("an inbox row starts work on its ticket's key or its PR's URL", () => {
  assert.equal(sourceOf({ kind: "ticket", key: "ENG-412" }), "ENG-412");
  assert.equal(sourceOf({ kind: "pr", key: "x", item: { url: "https://github.com/acme/api/pull/2198" } }), "https://github.com/acme/api/pull/2198");
  assert.equal(sourceOf({ kind: "alert", item: { url: "https://ci.example.com/run/1" } }), "");
  assert.equal(sourceOf(null), "");
});

test("where the form stands, from the plan and the run", () => {
  assert.equal(phase(null, null), "checking");
  assert.equal(phase({ ok: true, place: "worktree" }, null, { checking: true }), "checking");
  assert.equal(phase({ ok: true, place: "worktree" }, null), "ready");
  assert.equal(phase({ ok: true, place: "focus" }, null), "focus");
  assert.equal(phase({ ok: false, blockers: [{ step: "branch" }] }, null), "blocked");
  assert.equal(phase({ ok: true }, null, { started: true }), "running");
  assert.equal(phase({ ok: true }, { state: "failed" }), "failed");
  assert.equal(phase({ ok: true }, { state: "done" }), "done");
});

test("how a source and a step read", () => {
  assert.equal(resolved({ kind: "ticket", title: "ENG-412 Store and enforce daily quotas" }), "Ticket · ENG-412 Store and enforce daily quotas");
  assert.equal(resolved({ kind: "pr", title: "api#2198 Daily quotas" }), "PR · api#2198 Daily quotas");
  assert.equal(resolved({ kind: "prompt", title: "Fix the login redirect" }), null);
  assert.deepEqual(["done", "running", "failed", "skipped", "todo"].map((s) => stepMark(s).tone), ["success", "info", "destructive", "dim", "dim"]);
  assert.equal(shortPath("add a worktree at /home/me/.herdr/worktrees/api/x", "/home/me"), "add a worktree at ~/.herdr/worktrees/api/x");
  assert.equal(shortPath("/home/me/x", ""), "/home/me/x");
});

test("the options and fixes in the order they are drawn and 1–9 pick them, and the agent the form starts", () => {
  const plan = {
    agent: "claude",
    blockers: [{ step: "branch", fixes: [{ label: "Use it" }, { label: "Rename" }] }, { step: "github", fixes: [{ label: "Try again" }] }],
    options: [{ label: "Use as a prompt", set: { source: "prompt" } }],
    choices: { agents: [{ id: "claude", modes: [{ id: "plan" }] }, { id: "codex", modes: [] }] },
  };
  assert.deepEqual(choicesOf(plan).map((c) => c.label), ["Use as a prompt", "Use it", "Rename", "Try again"]);
  assert.deepEqual(choicesOf({ ...plan, place: "focus" }).map((c) => c.label), ["Use it", "Rename", "Try again"], "a source brought into view draws no options");
  assert.equal(requestOf(applyFix(emptyState("ENG-412", "g3"), choicesOf(plan)[0].set)).source, "prompt", "a form sends the source as a prompt once chosen");
  assert.deepEqual(agentOf(plan, emptyState("x")).modes, [{ id: "plan" }]);
  assert.deepEqual(agentOf(plan, { agent: "codex" }).modes, [], "an agent whose row takes no mode: the dialog shows none");
  assert.deepEqual([takesModel({ id: "claude", models: true }), takesModel({ id: "amp", models: false }), takesModel(null)], [true, false, true], "no model field for an agent that takes none");
  assert.equal(emptyState("", "run-7").id, "run-7", "opened on a run, the dialog follows it");
});

// The dialog itself, its natives stubbed: one window-wide focus, a field told of its focus and blur, and keys handed to
// Start.key as the root's on_key_down hands them.
async function dialog(planFor) {
  const world = { focused: null, acts: [], dismissed: 0 };
  const focus = (target) => {
    const was = world.focused;
    world.focused = target;
    if (was !== target) was?.fire?.("blur");
    if (was !== target) target?.fire?.("focus");
  };
  const handle = (name) => ({ name, focus() { focus(this); }, is_focused() { return world.focused === this; } });
  const InputState = {
    new: ({ value = "" } = {}) => {
      const listeners = {};
      const field = {
        ...handle("field"),
        on: (event, fn) => (listeners[event] = fn),
        fire: (event) => listeners[event]?.(null, cx),
        value: () => value,
        set_value: (v) => (value = v),
      };
      return field;
    },
  };
  const cx = {
    spawn: (f) => f(cx),
    timer: { after: (ms, fn) => setTimeout(() => fn(cx), Math.min(ms, 20)), every: () => ({ cancel() {} }) },
    notify() {},
    focus_handle: () => handle("root"),
    bind_keys: (bindings) => (world.bound ??= []).push(...bindings),
  };
  const stub = () => new Proxy(function () {}, { get: () => stub(), apply: () => stub() });
  const imports = {
    "gpui-kit": { div: stub() },
    "gpui-base": { h_flex: stub(), v_flex: stub(), InputState },
    herdr: { closeOverlay: () => world.dismissed++, openOverlay: () => {}, props: () => ({ overlay: "o1" }), shared: () => null },
    "./hub.js": {
      install: async () => ({ home: "/home/me" }),
      fetchDetails: async (_n, { request }) => planFor(request),
      act: async (...a) => (world.acts.push(a), {}),
      subscribe: (_cx, _opts, onMessage) => ((world.push = (run) => onMessage({ view: "start", value: { run } }, cx)), { stop() {} }),
    },
    "./screens.js": { readInChat: (...a) => (world.read ??= []).push(a) },
    "./ui.js": Object.fromEntries(["tokens", "TEXT", "RADIUS", "Icon", "Spinner", "Button", "Kbd", "TextField", "Pills", "AlertDialog", "HubDown", "alpha", "AgentLogo"].map((k) => [k, stub()])),
  };
  const context = createContext({ console, setTimeout, Promise, Date, JSON, Math, String, Number, Boolean, Object, Array, RegExp, Error, Set, Map });
  const mods = new URL("../mods/cockpit/", import.meta.url);
  const loaded = new Map();
  const load = async (file) => {
    if (loaded.has(file)) return loaded.get(file);
    const m = new SourceTextModule(await readFile(new URL(file, mods), "utf8"), { context, identifier: file });
    loaded.set(file, m);
    await m.link(async (name) =>
      name === "./startform.js" || name === "./picks.js"
        ? load(name.slice(2))
        : new SyntheticModule(Object.keys(imports[name]), function () {
            for (const [k, v] of Object.entries(imports[name])) this.setExport(k, v);
          }, { context }),
    );
    return m;
  };
  const m = await load("start.js");
  await m.evaluate();
  const s = new m.namespace.default("start");
  s.init({ source: "" }, cx);
  const key = (k) => s.key({ keystroke: k, key: k, modifiers: {} }, cx);
  const type = (text) => (s.source.set_value(text), s.set(cx, "source", text));
  return { s, key, type, world, cx };
}

const settle = (ms = 60) => new Promise((resolve) => setTimeout(resolve, ms));
const option = { label: "Use as a prompt", set: { source: "prompt" } };
const readyPlan = (request) => ({ ok: true, kind: "ticket", title: request.text, options: [option], blockers: [], steps: [], notes: [], choices: {} });

test("changing agents clears the model in both the request and the text field", async () => {
  const { s, cx } = await dialog(readyPlan);
  s.model.set_value("acme/large");
  s.set(cx, "model", "acme/large");
  s.pick(cx, "agent", "opencode");
  assert.equal(s.model.value(), "");
  assert.equal(requestOf(s.s).model, "");
  s.close();
});

test("an open dialog refreshes a delayed model catalog and stops after it completes", async () => {
  let ready = false, reads = 0;
  const { s } = await dialog((r) => {
    reads++;
    return { ...readyPlan(r), choices: { agents: [{ id: "codex", picks: { loading: !ready, models: ready ? [{ id: "gpt-acme-1" }] : [] } }] } };
  });
  await settle();
  assert.equal(s.plan.choices.agents[0].picks.loading, true);
  ready = true;
  await settle();
  assert.equal(s.plan.choices.agents[0].picks.models[0].id, "gpt-acme-1");
  const completed = reads;
  await settle();
  assert.equal(reads, completed, "completion stops polling");
  s.close();
});

test("while the plan is checked again ↓ and the digits wait, and ⏎ on a lit choice never starts the work", async () => {
  const { s, key, type, world, cx } = await dialog(readyPlan);
  type("ENG-41");
  await settle();
  type("ENG-412");
  assert.equal(key("down"), false, "the plan of ENG-412 is not in yet");
  assert.equal(s.cursor, null);
  await settle();
  assert.equal(key("down"), true);
  assert.equal(world.focused.name, "root");
  s.replan(cx, 0);
  assert.equal(key("1"), false, "no digit picks from a plan being checked");
  assert.equal(key("enter"), true, "a lit choice waits for its plan");
  await settle();
  assert.equal(world.acts.length, 0);
  assert.equal(key("enter"), true);
  assert.equal(requestOf(s.s).source, "prompt", "⏎ applied the lit option");
  assert.equal(world.acts.length, 0, "and started nothing");
});

test("the plan of what was typed comes back: the timer's own context asks for it, not the keystroke's, gone by then", async () => {
  const { s, cx } = await dialog(readyPlan);
  await settle();
  let live = true;
  const errors = [];
  const stale = (what) => (...args) => {
    if (!live) throw new Error(`${what} on a context whose call is over`);
    return cx[what](...args);
  };
  // A keystroke's context: good during its own call only, as Herdr's are; its timers fire with a context of their own.
  const keystroke = { spawn: stale("spawn"), notify: stale("notify"), timer: { after: (ms, fn) => setTimeout(() => { try { fn(cx); } catch (e) { errors.push(e); } }, Math.min(ms, 20)) } };
  s.source.set_value("ENG-412");
  s.set(keystroke, "source", "ENG-412");
  live = false;
  await settle();
  assert.deepEqual(errors.map(String), []);
  assert.equal(s.checking, false, "planned");
  assert.equal(s.plan?.title, "ENG-412");
});

test("⏎ starts from a text field only: on the dialog or a button with nothing lit it does nothing", async () => {
  const { s, key, type, world } = await dialog(readyPlan);
  type("ENG-412");
  await settle();
  s.handle.focus();
  assert.equal(key("enter"), false);
  key("down");
  // The keyboard moves on to a button: the lit choice goes with it, and its ⏎ is the button's.
  world.focused = { name: "button", is_focused: () => true };
  assert.equal(key("enter"), false);
  assert.equal(s.cursor, null);
  await settle();
  assert.equal(world.acts.length, 0);
  s.source.focus();
  assert.equal(key("enter"), true);
  await settle();
  assert.deepEqual(world.acts.map((a) => a.slice(0, 2)), [["start", "work"]], "from the source field it starts");
});

test("a new plan keeps a lit choice on its label, and one it dropped gives the source the keyboard", async () => {
  let plan = readyPlan;
  const { s, key, type, world, cx } = await dialog((r) => plan(r));
  type("ENG-412");
  await settle();
  key("down");
  plan = (r) => ({ ...readyPlan(r), ok: false, blockers: [{ step: "branch", text: "exists", fixes: [{ label: "Use it", set: { existing: true } }] }] });
  s.replan(cx, 0);
  await settle();
  assert.equal(choicesOf(s.plan)[s.cursor]?.label, "Use as a prompt", "still lit");
  plan = (r) => ({ ...readyPlan(r), options: [] });
  s.replan(cx, 0);
  await settle();
  assert.equal(s.cursor, null);
  assert.equal(world.focused, s.source, "the source has the keyboard again");
});

test("digits pick only while the dialog holds the keyboard, and Esc on a lit choice goes back before it leaves", async () => {
  const { s, key, type, world } = await dialog(readyPlan);
  type("ENG-412");
  await settle();
  s.source.focus();
  assert.equal(key("1"), false, "a digit typed in the source is the field's");
  assert.equal(key("down"), true);
  assert.equal(key("escape"), true);
  assert.equal(world.dismissed, 0, "Esc only lets go of the lit choice");
  assert.equal(world.focused, s.source);
  s.handle.focus();
  assert.equal(key("1"), true);
  assert.equal(requestOf(s.s).source, "prompt");
  assert.equal(key("escape"), true);
  assert.equal(world.dismissed, 1, "a second Esc leaves");
});

test("Esc is bound in the dialog's own context, from a field too, so the overlay's Esc does not close it before its keys hear it", async () => {
  const { s, key, type, world } = await dialog(readyPlan);
  assert.deepEqual(JSON.parse(JSON.stringify(world.bound)), [
    { keystroke: "escape", action: "cockpit_start_escape", context: "CockpitStart && start_holds" },
    { keystroke: "escape", action: "cockpit_start_escape", context: "CockpitStart && start_holds > Input" },
  ]);
  type("ENG-412");
  await settle();
  key("down");
  assert.equal(s.cursor, 0, "↓ lights the first option");
  s.key({ keystroke: "escape", key: "escape" }, { notify() {} });
  assert.equal(s.cursor, null, "the action's Esc lets go of it");
  assert.equal(world.dismissed, 0);
  s.key({ keystroke: "escape", key: "escape" }, { notify() {} });
  assert.equal(world.dismissed, 1, "a second Esc closes the dialog");
});

test("a second Start on a ticket whose driven session is open brings that session up in Cockpit Chat", async () => {
  const sessionPlan = (request) => ({ ...readyPlan(request), place: "focus", session: true, space: "claude:1793611000000" });
  const { s, type, world, cx } = await dialog(sessionPlan);
  type("ENG-412");
  await settle();
  s.start(cx);
  world.push({ state: "done", steps: [], made: { pane: "claude:1793611000000" } });
  world.push({ state: "done", steps: [], made: { pane: "claude:1793611000000" } });
  assert.deepEqual(world.read, [["claude:1793611000000"]], "read in the chat, once");
});
