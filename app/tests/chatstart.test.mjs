import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { createContext, SourceTextModule, SyntheticModule } from "node:vm";
import { test } from "node:test";

// The chat's start form with its natives stubbed: the hub plans from `planFor`, records its acts, and lets a test push
// the `start` view's run.
async function starter(planFor, actFor = () => ({ note: "started" })) {
  const hub = { asked: [], acts: [], pushes: [] };
  const cx = { spawn: (f) => f(cx), timer: { after: (_ms, fn) => setTimeout(() => fn(cx), 1) }, notify() {} };
  const stub = () => new Proxy(function () {}, { get: () => stub(), apply: () => stub() });
  const imports = {
    "gpui-kit": { div: stub() },
    "gpui-base": { h_flex: stub(), v_flex: stub() },
    "./hub.js": {
      install: async () => ({ home: "/home/me" }),
      fetchDetails: async (_n, { request }) => (hub.asked.push(request), planFor(request)),
      act: async (...a) => (hub.acts.push(a.slice(0, 3)), actFor(...a)),
      subscribe: (_cx, opts, onMessage) => {
        const s = { opts, stopped: false, push: (run) => s.stopped || onMessage({ view: "start", value: { run } }, cx) };
        hub.pushes.push(s);
        return { stop: () => (s.stopped = true) };
      },
    },
    "./ui.js": Object.fromEntries(["TEXT", "RADIUS", "Icon", "Spinner", "Button", "HubDown"].map((k) => [k, stub()])),
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
  const m = await load("chatstart.js");
  await m.evaluate();
  return { start: new m.namespace.ChatStart(), hub, cx };
}

const settle = (ms = 30) => new Promise((resolve) => setTimeout(resolve, ms));
// Typed in the box, then the chat's clock ticks once typing settled.
async function typed(start, cx, text) {
  start.sync(text, cx);
  start.sync(text, cx, Date.now() + 1000);
  await settle();
}
const AGENTS = [
  { id: "claude", icon: "claude", name: "Claude Code", installed: true, drives: true, modes: [] },
  { id: "codex", icon: "codex", name: "Codex", installed: true, drives: false, modes: [] },
];
function plan(request) {
  const kind = /^ENG-\d+/.test(request.text) ? "ticket" : request.text.includes("/pull/") ? "pr" : "prompt";
  const agent = request.agent || "claude";
  return {
    ok: Boolean(request.text), kind, title: request.text, driven: request.driven === true, agent, place: "worktree",
    steps: [{ id: "agent", text: request.driven ? "start as a session cockpit drives" : "open a space there" }],
    blockers: request.text ? [] : [{ step: "empty", text: "Type a prompt", fixes: [] }],
    choices: { repos: ["api", "web"], repo: "api", agent: "claude", agents: AGENTS },
  };
}
// A chip as the chat's own picker would build it, kept as data.
const picker = (id, o) => ({ id, ...o });

test("the chat's idle clock refreshes a delayed model catalog without another edit", async () => {
  let ready = false;
  const { start, hub, cx } = await starter((r) => ({ ...plan(r), choices: { ...plan(r).choices, agents: AGENTS.map((a) => ({ ...a, picks: { loading: !ready, models: ready ? [{ id: "gpt-acme-1" }] : [] } })) } }));
  await typed(start, cx, "Fix the login redirect");
  assert.equal(start.plan.choices.agents[0].picks.loading, true);
  ready = true;
  start.sync("Fix the login redirect", cx, Date.now() + 1000);
  await settle();
  assert.equal(start.plan.choices.agents[0].picks.models[0].id, "gpt-acme-1");
  const completed = hub.asked.length;
  start.sync("Fix the login redirect", cx, Date.now() + 2000);
  await settle();
  assert.equal(hub.asked.length, completed, "completion stops polling");
});

test("what the box holds is planned as the session Claude runs in this chat, another agent in a terminal", async () => {
  const { start, hub, cx } = await starter(plan);
  start.sync("ENG-412", cx);
  assert.equal(hub.asked.length, 0, "not while typing");
  start.sync("ENG-412", cx, Date.now() + 1000);
  await settle();
  assert.deepEqual(hub.asked.map((r) => [r.text, r.driven]), [["ENG-412", false], ["ENG-412", true]], "planned again once its agent is known");
  const [where, agent] = start.chips(picker);
  assert.equal(where.label, "api · new worktree");
  assert.deepEqual([agent.label, agent.icon, agent.logo], ["Claude Code", "message-square", "claude"], "the agent beside its logo");
  assert.deepEqual(JSON.parse(JSON.stringify(agent.items.map((i) => [i.logo, i.check]))), [["claude", true], ["codex", false]]);
  assert.deepEqual(agent.items.map((i) => i.label), ["Claude Code, in this chat", "Codex, in a terminal"]);
  start.command("chatstart:agent:codex", cx);
  start.command("chatstart:here:here", cx);
  await settle();
  assert.deepEqual([hub.asked.at(-1).agent, hub.asked.at(-1).driven, hub.asked.at(-1).worktree], ["codex", false, "here"]);
  assert.equal(start.chips(picker)[1].icon, "terminal");
});

test("a fix that starts Claude in a terminal wins over the way it runs", async () => {
  const { start, hub, cx } = await starter(plan);
  await typed(start, cx, "Fix the login redirect");
  start.fix({ driven: false }, cx);
  await settle();
  assert.equal(hub.asked.at(-1).driven, false);
  assert.equal(start.drives(), false);
});

test("⏎ starts it, the chat follows what it made, and what is typed next is a new form with the same picks", async () => {
  const { start, hub, cx } = await starter(plan);
  start.command("chatstart:repo:web", cx);
  start.sync("https://github.com/acme/api/pull/2", cx);
  const followed = [];
  // ⏎ right after typing: what the box holds is planned at once, and it starts once that plan says it can.
  assert.equal(start.start(cx, (key) => followed.push(key), "https://github.com/acme/api/pull/2198"), false);
  await settle();
  assert.deepEqual(hub.acts.map((a) => a.slice(0, 2)), [["start", "work"]]);
  const request = hub.acts[0][2][0];
  assert.deepEqual([request.text, request.repo, request.driven], ["https://github.com/acme/api/pull/2198", "web", true]);
  hub.pushes.at(-1).push({ id: request.id, state: "running", steps: [], made: {} });
  assert.deepEqual(followed, []);
  hub.pushes.at(-1).push({ id: request.id, state: "done", steps: [], made: { pane: "claude:s-41" } });
  hub.pushes.at(-1).push({ id: request.id, state: "done", steps: [], made: { pane: "claude:s-41" } });
  assert.deepEqual(followed, ["claude:s-41"], "followed once");
  start.sync("", cx);
  assert.equal(start.run?.state, "done", "on show while the box is empty");
  start.sync("Fix the login redirect", cx);
  assert.equal(start.run, null, "something new typed: a new form");
  assert.deepEqual([start.s.source, start.s.repo], ["Fix the login redirect", "web"]);
});

test("a start brought into view made nothing to follow: the box is cleared all the same", async () => {
  const { start, hub, cx } = await starter((r) => ({ ...plan(r), place: "focus" }));
  await typed(start, cx, "ENG-405");
  const followed = [];
  assert.equal(start.start(cx, (key) => followed.push(key)), true);
  hub.pushes.at(-1).push({ id: start.run.id, state: "done", steps: [], made: {} });
  assert.deepEqual(followed, [null]);
});

test("a plan that never came back is asked for again", async () => {
  let answer = false;
  const { start, hub, cx } = await starter((r) => (answer ? plan(r) : new Promise(() => {})));
  await typed(start, cx, "ENG-412");
  assert.equal(start.checking, true);
  answer = true;
  start.sync("ENG-412", cx, Date.now() + 16_000);
  await settle();
  assert.equal(start.checking, false);
  assert.equal(hub.asked.length >= 2, true);
});

// The VM's objects as plain data, so they compare with this realm's.
const plain = (value) => JSON.parse(JSON.stringify(value));
/** Pushes a run to every `start` subscription still open, as the hub's view does. */
const pushed = (hub, run) => hub.pushes.filter((p) => !p.stopped).forEach((p) => p.push(run));

test("Retry after a failed start follows the retried run to its end", async () => {
  const { start, hub, cx } = await starter(plan);
  await typed(start, cx, "Fix the login redirect");
  const followed = [];
  assert.equal(start.start(cx, (key) => followed.push(key)), true);
  const id = start.run.id;
  pushed(hub, { id, state: "failed", steps: [], made: {}, error: { step: "setup", text: "exit 1" } });
  assert.equal(start.run.state, "failed");
  assert.equal(hub.pushes.filter((p) => !p.stopped).length, 0, "a run over is followed no more");
  start.retry(cx);
  await settle();
  assert.deepEqual(plain(hub.acts.at(-1)), ["start", "retry", [id]]);
  assert.equal(hub.pushes.filter((p) => !p.stopped).length, 1, "Retry follows the run again");
  assert.deepEqual(plain(hub.pushes.at(-1).opts.views), [{ name: "start", args: { id } }]);
  pushed(hub, { id, state: "running", steps: [], made: {} });
  pushed(hub, { id, state: "done", steps: [], made: { pane: "claude:s-77" } });
  assert.equal(start.run.state, "done");
  assert.deepEqual(followed, ["claude:s-77"], "the chat follows what the retried run made");
});

test("Retry of a start the hub never recorded runs it again from the box, under the same id", async () => {
  let works = 0;
  const reply = (_app, action) => (action === "retry" ? { error: "no such start" } : action === "work" && ++works === 1 ? { error: "the cockpit hub was stopped" } : { note: "started" });
  const { start, hub, cx } = await starter(plan, reply);
  await typed(start, cx, "Fix the login redirect");
  start.start(cx, () => {});
  await settle();
  const id = start.run.id;
  assert.equal(start.error, "the cockpit hub was stopped");
  start.retry(cx);
  await settle();
  assert.deepEqual(hub.acts.slice(-2).map((a) => [a[1], a[2][0]?.id ?? a[2][0]]), [["retry", id], ["work", id]]);
  assert.equal(start.error, null);
  pushed(hub, { id, state: "done", steps: [], made: { pane: "claude:s-78" } });
  assert.equal(start.run.state, "done");
});

test("a failed start that made something offers Cancel and remove, asked first", async () => {
  const { start, hub, cx } = await starter(plan);
  await typed(start, cx, "Fix the login redirect");
  start.start(cx, () => {});
  const id = start.run.id;
  pushed(hub, { id, state: "failed", steps: [], made: { worktree: "/w/api-fix" }, error: { step: "setup", text: "exit 1" }, cancel: true });
  start.cancel(cx);
  assert.equal(start.removing, true, "nothing goes before a yes");
  assert.ok(!hub.acts.some((a) => a[1] === "cancel"));
  start.cancel(cx, true);
  await settle();
  assert.deepEqual(plain(hub.acts.at(-1)), ["start", "cancel", [id]]);
  assert.equal(start.run, null, "removed: the box is a box again");
  assert.equal(start.s.repo, null);
});

test("a failed start that made nothing has nothing to remove", async () => {
  const { start, hub, cx } = await starter(plan);
  await typed(start, cx, "Fix the login redirect");
  start.start(cx, () => {});
  pushed(hub, { id: start.run.id, state: "failed", steps: [], made: {}, error: { step: "repo", text: "not a git repo" }, cancel: false });
  start.cancel(cx, true);
  await settle();
  assert.ok(!hub.acts.some((a) => a[1] === "cancel"));
});
