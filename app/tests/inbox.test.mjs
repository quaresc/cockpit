import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import { createContext, SourceTextModule, SyntheticModule } from "node:vm";
import { test } from "node:test";

/** inbox.js with every native import stubbed: only the cleanup's own logic runs. */
async function load() {
  const context = createContext({ console });
  const imports = { herdr: { shared: () => "cockpit", settings: () => ({}) }, net: { default: {} } };
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
        if (key) exports[key] ??= () => { throw new Error(`unexpected native call: ${key}`); };
      }
    }
  }
  function link(name) {
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
  const module = new SourceTextModule('export { default } from "./inbox.js";', { context });
  await module.link(link);
  await module.evaluate();
  return module.namespace.default;
}

/** Values made in the module's own realm, as plain ones to compare. */
const plain = (v) => JSON.parse(JSON.stringify(v));

const CLEAR = { path: "/w/api/eng-412", repo: "api", branch: "me/eng-412-quota-model", pr: "https://github.com/acme/api/pull/2190", blockers: [] };
const KEPT = {
  path: "/w/api/eng-413", repo: "api", branch: "me/eng-413-quota-ui", pr: "https://github.com/acme/api/pull/2191",
  blockers: [{ id: "uncommitted", words: "1 uncommitted change" }, { id: "running", words: "Claude Code still runs in it" }, { id: "not_merged", words: "not merged into main" }],
};
const row = (tree) => ({
  key: `worktree:${tree.path}`, kind: "worktree", name: `${tree.repo} ${tree.branch}`, item: tree,
  acts: [
    { label: "remove worktree", key: "D", confirm: `remove-${tree.path}`, call: { fn: "remove_worktree", args: [tree.path] } },
    { label: "go", key: "o", call: { fn: "open_worktree", args: [tree.path] } },
  ],
});

/** An inbox on these worktrees, its hub calls and toasts kept. */
async function inbox(trees, refuse = []) {
  const Inbox = await load();
  const box = Object.create(Inbox.prototype);
  const calls = [], toasts = [];
  Object.assign(box, {
    model: { worktrees: trees }, picked: new Set(), dialog: null,
    toasts: { push: (_cx, text) => toasts.push(text) },
    all: () => trees.map(row),
    call: async (_cx, call, opts) => {
      calls.push([call.fn, call.args, opts?.armed ?? null]);
      return { error: refuse.includes(call.args?.[0]) ? `kept ${call.args[0]}: 1 uncommitted change` : null };
    },
  });
  return { box, calls, toasts };
}

const cx = () => ({ notify: () => {}, spawn: (f) => f(cx()) });

test("a worktree's remove dialog says it goes, or what keeps it and offers nothing to run", async () => {
  const { box, calls } = await inbox([CLEAR, KEPT]);
  box.run(cx(), row(CLEAR), row(CLEAR).acts[0]);
  assert.equal(box.dialog.title, "Remove this worktree?");
  assert.deepEqual(plain(box.dialog.lines), [{ text: "Goes: api me/eng-412-quota-model, its folder deleted; its branch too once git counts it merged" }]);
  assert.equal(box.dialog.confirm, "remove");
  await box.dialog.run(cx());
  assert.deepEqual(plain(calls), [["remove_worktree", ["/w/api/eng-412"], "remove-/w/api/eng-412"]]);
  box.run(cx(), row(KEPT), row(KEPT).acts[0]);
  assert.deepEqual([box.dialog.title, box.dialog.confirm], ["This worktree is kept", null]);
  assert.deepEqual(plain(box.dialog.lines), [{ text: "Stays: api me/eng-413-quota-ui (1 uncommitted change, Claude Code still runs in it, not merged into main)", tone: "warning" }]);
});

test("a batch cleanup removes only what nothing keeps, and says why the rest stays", async () => {
  const { box, calls } = await inbox([CLEAR, KEPT]);
  box.picked = new Set([`worktree:${CLEAR.path}`, `worktree:${KEPT.path}`]);
  box.runShared(cx(), "remove worktree", "D");
  assert.equal(box.dialog.title, "Remove 1 worktree?");
  assert.deepEqual(plain(box.dialog.lines.map((l) => l.text)), [
    "Goes: api me/eng-412-quota-model, its folder deleted; its branch too once git counts it merged",
    "Stays: api me/eng-413-quota-ui (1 uncommitted change, Claude Code still runs in it, not merged into main)",
  ]);
  await box.dialog.run(cx());
  assert.deepEqual(plain(calls.map(([fn, args]) => [fn, args])), [["remove_worktree", ["/w/api/eng-412"]]]);
  box.picked = new Set([`worktree:${KEPT.path}`]);
  box.runShared(cx(), "remove worktree", "D");
  assert.deepEqual([box.dialog.title, box.dialog.description, box.dialog.confirm], ["Nothing to remove", "Every picked worktree is kept.", null]);
});

test("any other shared action runs on every picked worktree, kept ones included", async () => {
  const { box, calls } = await inbox([CLEAR, KEPT]);
  box.picked = new Set([`worktree:${CLEAR.path}`, `worktree:${KEPT.path}`]);
  box.runShared(cx(), "go", "o");
  await box.dialog.run(cx());
  assert.deepEqual(plain(calls.map(([fn, args]) => [fn, args[0]])), [["open_worktree", CLEAR.path], ["open_worktree", KEPT.path]]);
});

test("a batch says how many went and what refused the rest", async () => {
  const { box, toasts } = await inbox([CLEAR, KEPT], [CLEAR.path]);
  const acts = [CLEAR, { ...KEPT, blockers: [] }].map((t) => [row(t), row(t).acts[0]]);
  await box.runActs(cx(), "remove worktree", acts);
  assert.deepEqual(toasts, ["remove worktree: 1 done, 1 refused"]);
});

test("after a merge, the toast offers its worktree's removal, or says what keeps it besides the merge", async () => {
  const { box, toasts } = await inbox([CLEAR, KEPT]);
  assert.equal(box.cleanupAction({ fn: "merge", args: [CLEAR.pr] }, "api#2190 joined the merge queue"), undefined, "a place in the queue is no merge");
  const clear = box.cleanupAction({ fn: "merge", args: [CLEAR.pr] }, "merged api#2190 (squash): its worktree is in To clean up");
  assert.equal(clear.label, "Remove worktree");
  clear.onClick(null, cx());
  assert.equal(box.dialog.title, "Remove this worktree?");
  assert.equal(box.dialog.confirm, "remove");
  const kept = box.cleanupAction({ fn: "merge", args: [KEPT.pr] }, "merged api#2191 (squash): its worktree is in To clean up");
  assert.equal(kept.label, "Why kept");
  kept.onClick(null, cx());
  assert.deepEqual(toasts, ["Worktree kept: 1 uncommitted change, Claude Code still runs in it"]);
  assert.equal(box.cleanupAction({ fn: "rerun", args: [CLEAR.pr] }, "merged"), undefined);
});

test("a merged session's worktree is asked about as ending its session first", async () => {
  const { box, calls } = await inbox([CLEAR, KEPT]);
  const alert = { key: "alert:session:w7", kind: "alert", name: "session on merged api me/x" };
  const act = { label: "remove worktree", key: "D", confirm: "close-w7", call: { fn: "close_space", args: ["w7", "merged api me/x", true] } };
  box.run(cx(), alert, act);
  assert.deepEqual([box.dialog.title, box.dialog.description, box.dialog.confirm], ["Remove this worktree?", "session on merged api me/x: ends the session in it, then deletes its folder; its branch too once git counts it merged.", "remove"]);
  await box.dialog.run(cx());
  assert.deepEqual(plain(calls), [["close_space", ["w7", "merged api me/x", true], "close-w7"]]);
});
