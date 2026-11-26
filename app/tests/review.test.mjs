import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import { createContext, SourceTextModule, SyntheticModule } from "node:vm";
import { test } from "node:test";

import { agentsIn, menuItems, nameOf, newAgentRequest, plainError, recipient, recipientLabel } from "../mods/cockpit/sendto.js";

// The acme agents: a blocked terminal, a working one and a driven session in the api checkout; one in web; one in a
// worktree nested inside the api checkout, which git calls a repository of its own.
const API = "/work/acme/api";
const ROWS = [
  { terminal: "app:t41", address: "t41", title: "Enforce daily quotas", program: "claude", status: "blocked", since: 4, cwd: API },
  { terminal: "app:t45", address: "t45", title: "Quota e2e tests", program: "claude", status: "working", since: 2, cwd: `${API}/e2e` },
  { terminal: "claude:demo-421", address: "claude:demo-421", title: "Per-plan limits", program: "claude", status: "idle", since: 9, cwd: API, driven: true },
  { terminal: "app:t42", address: "t42", title: "Quota banner", program: "claude", status: "working", since: 7, cwd: "/work/acme/web" },
  { terminal: "app:t48", address: "t48", title: "Nested worktree", program: "codex", status: "working", since: 1, cwd: `${API}/.worktrees/eng-417`, space: { name: "eng-417" } },
];
// What the review view lists as working in the api repository: by each folder's git toplevel, never by path prefix.
const HERE = ["app:t41", "app:t45", "claude:demo-421"];

test("the agents of a repository are the ones git puts in it, driven sessions among them, the working ones first", () => {
  assert.deepEqual(agentsIn(ROWS, HERE).map((r) => r.terminal), ["app:t45", "app:t41", "claude:demo-421"], "the nested worktree is not counted");
  assert.deepEqual(agentsIn(ROWS, []), []);
});

test("the review goes to the choice kept for the repo, else the followed agent, else a new one", () => {
  assert.equal(recipient(ROWS, HERE, { followed: "t41" }).row.terminal, "app:t41", "the panel's agent by default");
  assert.equal(recipient(ROWS, HERE, {}).row.terminal, "app:t45", "nothing followed: the most recently active");
  assert.equal(recipient(ROWS, HERE, { chosen: { handle: "claude:demo-421", name: "Per-plan limits" }, followed: "t41" }).row.driven, true);
  assert.deepEqual(recipient(ROWS, HERE, { chosen: "new", followed: "t41" }), { kind: "new" });
  const gone = recipient(ROWS, HERE, { chosen: { handle: "app:t40", name: "Old run" } });
  assert.deepEqual(gone, { kind: "gone", name: "Old run" }, "a choice that stopped is said gone, not swapped");
  assert.equal(recipientLabel(gone), "Old run (gone)");
  const elsewhere = recipient(ROWS, HERE, { chosen: { handle: "app:t42", name: "Quota banner" } });
  assert.deepEqual(elsewhere, { kind: "gone", name: "Quota banner" }, "a choice that works in another repo now is gone here");
  assert.deepEqual(recipient(ROWS, [], { followed: "t41" }), { kind: "new" }, "no agent there: a new one");
  // The agent a review just started, found by its space's label once it runs.
  const fresh = [...ROWS, { terminal: "app:t50", title: "Review agent", status: "working", cwd: API, space: { name: "review-7" } }];
  assert.equal(recipient(fresh, [...HERE, "app:t50"], { chosen: { space: "review-7", name: "the review agent" } }).row.terminal, "app:t50");
  assert.deepEqual(recipient(ROWS, HERE, { chosen: { space: "review-7", name: "the review agent" } }), { kind: "gone", name: "the review agent" });
  // Until it shows up, a couple of minutes at most, it is starting, not gone.
  const starting = recipient(ROWS, HERE, { chosen: { space: "review-7", name: "the review agent", at: 1000 }, now: 31_000 });
  assert.deepEqual(starting, { kind: "starting", name: "the review agent" });
  assert.equal(recipientLabel(starting), "the review agent (starting)");
  assert.equal(recipient(ROWS, HERE, { chosen: { space: "review-7", name: "the review agent", at: 1000 }, now: 200_000 }).kind, "gone");
});

test("the menu lists the repo's agents, then a new agent, each command naming the repo", () => {
  const items = menuItems(ROWS, HERE, { kind: "agent", row: ROWS[0] }, API);
  assert.deepEqual(items.map((i) => (i === "separator" ? "—" : i.label)), [
    "Quota e2e tests · claude, working",
    "Enforce daily quotas · claude, blocked",
    "Per-plan limits · in chat, idle",
    "—",
    "A new agent, started on the review",
  ]);
  assert.equal(items[1].icon, "check");
  assert.equal(items[2].command, `review:to:claude:demo-421|${API}`);
  assert.deepEqual(menuItems([], [], { kind: "new" }, API).map((i) => i.command), [`review:to:new|${API}`]);
  const long = [{ ...ROWS[2], title: "Make the daily quota configurable per plan, and run the quota tests" }];
  assert.equal(menuItems(long, HERE, { kind: "new" }, API)[0].label, "Make the daily quota configur… · in chat, idle", "the name gives way, not what it is");
});

test("a new agent starts in a space of its own and leaves Start work's choices alone", () => {
  assert.deepEqual(newAgentRequest(API, "My review of …", 7), {
    id: "review-7", source: "prompt", text: "My review of …", repo: API, worktree: "here", dirty: true, label: "review-7", remember: false,
  });
  assert.equal(plainError("send: Enforce daily quotas is waiting on a question: answer it first"), "Enforce daily quotas is waiting on a question: answer it first");
  assert.equal(plainError("work: the repository is gone"), "the repository is gone");
  assert.equal(nameOf({ tab: { name: "quotas" } }), "quotas");
});

/**
 * An element as gpui builds it, every style chained; like a virtual list's item renderer, it refuses a click or a
 * tooltip handler, which rows rebuilt each frame would pile up.
 */
function element() {
  return new Proxy(function () {}, {
    get: (_t, p) => {
      if (p === "then") return undefined;
      if (p === "on_click" || p === "tooltip") return () => { throw new Error(`\`${p}\` registered from a list row`); };
      return (...args) => (p === "when" ? (args[0] ? args[1](element()) : element()) : element());
    },
    apply: () => element(),
  });
}

// diff.js in a VM with Herdr's natives stubbed: its rows are plain data, and with `draw` its elements are chained stubs.
async function diffModule({ draw = false } = {}) {
  const context = createContext({ console, performance });
  // The hub client stays out: diff.js never calls it, and it reaches for sockets.
  const natives = { herdr: { theme: () => ({}), settings: () => ({}) }, "gpui-base": { VirtualListScrollHandle: { new: () => ({ scroll_to_item() {} }) } }, "./hub.js": {} };
  if (draw) {
    for (const name of ["div", "svg"]) (natives["gpui-kit"] ??= {})[name] = () => element();
    for (const name of ["h_flex", "v_flex"]) natives["gpui-base"][name] = () => element();
  }
  const sources = new Map();
  const folder = new URL("../mods/cockpit/", import.meta.url);
  for (const name of await readdir(folder)) {
    if (!name.endsWith(".js")) continue;
    const text = await readFile(new URL(name, folder), "utf8");
    sources.set(`./${name}`, text);
    for (const match of text.matchAll(/import\s*\{([^}]+)\}\s*from\s*"(herdr|gpui-kit|gpui-base)"/g)) {
      const exports = (natives[match[2]] ??= {});
      for (const word of match[1].split(",")) {
        const key = word.trim().split(/\s+as\s+/)[0];
        exports[key] ??= () => { throw new Error(`unexpected native call: ${key}`); };
      }
    }
  }
  const modules = new Map();
  const load = (name) => {
    if (modules.has(name)) return modules.get(name);
    const native = natives[name];
    const module = native
      ? new SyntheticModule(Object.keys(native), function () { for (const [k, v] of Object.entries(native)) this.setExport(k, v); }, { context })
      : new SourceTextModule(sources.get(name), { context, identifier: name });
    modules.set(name, module);
    return module;
  };
  const entry = new SourceTextModule('export { DiffView, held, bytes } from "./diff.js";', { context });
  await entry.link(load);
  await entry.evaluate();
  return entry.namespace;
}

/** A file of the review model: `n` added lines in hunks of 40. */
function file(path, n, extra = {}) {
  const lines = [];
  for (let i = 0; i < n; i++) {
    if (i % 40 === 0) lines.push(["@", null, null, [[`@@ -${i + 1},0 +${i + 1},40 @@`, "", false]]]);
    lines.push(["+", null, i + 1, [[`row_${i} = compute_daily_limit(plan, used, grace=${i})`, "", false]]]);
  }
  return { path, added: n, removed: 0, hunks: [], lines, more: 0, total: lines.length, ...extra };
}

test("a file with comments is always read whole, so its comments keep their lines when it grows large", async () => {
  const { DiffView } = await diffModule();
  const view = new DiffView("review", { value: () => "" });
  view.comments = [{ id: "c1", path: "lib/quota.ex", line: 3, old: false, code: "x", text: "why 1500?" }];
  assert.deepEqual([...view.whole(new Set(["web/app.ts"]))], ["web/app.ts", "lib/quota.ex"]);
  assert.deepEqual([...view.whole(new Set(["lib/quota.ex"]))], ["lib/quota.ex"], "each file once");
});

test("a large file waits as one row, a binary file and a rename say what they are", async () => {
  const { DiffView, bytes } = await diffModule();
  const view = new DiffView("review", {});
  const loads = [];
  view.onLoad = (path) => loads.push(path);
  view.set({ files: [
    { path: "docs/limits.md", old_path: "docs/quotas.md", added: 0, removed: 0, hunks: [], lines: [], more: 0 },
    { path: "assets/logo.png", binary: true, size: 12008, added: 0, removed: 0, hunks: [], lines: [], more: 0 },
    { path: "src/plans_table.py", large: true, total: 3004, added: 3003, removed: 0, hunks: [], lines: [], more: 0 },
  ] });
  assert.deepEqual(JSON.parse(JSON.stringify(view.rows().map((r) => r.kind))), ["file", "file", "binary", "file", "large"]);
  assert.equal(bytes(12008), "11 KB");
  view.cursor = { f: 2, l: null };
  assert.equal(view.key({ keystroke: "enter" }), true);
  assert.deepEqual([...loads], ["src/plans_table.py"], "Enter on a large file's header loads it");
  view.click("g:2", { notify() {} });
  assert.equal(loads.length, 2, "and so does its row");
});

test("every row of the list draws without a click or tooltip of its own: the list's item click takes them", async () => {
  const { DiffView } = await diffModule({ draw: true });
  const view = new DiffView("review", element());
  view.set({ files: [
    { path: "docs/limits.md", old_path: "docs/quotas.md", added: 0, removed: 0, hunks: [], lines: [], more: 0 },
    { path: "assets/logo.png", binary: true, size: 12008, added: 0, removed: 0, hunks: [], lines: [], more: 0 },
    { path: "src/plans_table.py", large: true, total: 3004, added: 3003, removed: 0, hunks: [], lines: [], more: 0 },
    file("src/quotas.py", 50, { more: 3 }),
  ] });
  const k = new Proxy({}, { get: () => "#888888" });
  const draw = { file: "fileRow", comment: "commentRow", more: "moreRow", binary: "binaryRow", large: "largeRow" };
  for (const r of view.rows()) assert.doesNotThrow(() => view[draw[r.kind] ?? "lineRow"](r, k), `${r.kind} row ${r.key}`);
});

test("a 1 MB diff builds its rows in a few frames, faster with its large file held back", async () => {
  const { DiffView } = await diffModule();
  const files = Array.from({ length: 30 }, (_, i) => file(`src/mod_${i}.py`, 400)).concat([file("src/big.py", 9000)]);
  const time = (view) => {
    const t = performance.now();
    const n = view.rows().length;
    view.stops();
    return [n, performance.now() - t];
  };
  const all = new DiffView("review", {});
  all.set({ files });
  const guarded = new DiffView("review", {});
  guarded.set({ files: files.map((f) => (f.path === "src/big.py" ? { ...f, lines: [], large: true } : f)) });
  for (let i = 0; i < 3; i++) time(all), time(guarded);
  const [n1, ms1] = time(all);
  const [n2, ms2] = time(guarded);
  if (process.env.BENCH) console.log(`rows+stops: ${n1} rows ${ms1.toFixed(1)} ms; held back: ${n2} rows ${ms2.toFixed(1)} ms`);
  assert.ok(n1 - n2 > 9000, "the large file's 9000 lines are one row");
  assert.ok(ms1 < 250, `every row of 21k lines in ${ms1.toFixed(1)} ms`);
});

// review.js in a VM: Herdr's natives and the hub stubbed, `answers` what the hub's review view gives, in turn, each
// call's arguments kept in `calls`; `stubs` stand in for the natives a test draws with.
async function reviewModule(answers, { calls = [], stubs = {} } = {}) {
  const context = createContext({ console, performance });
  const natives = { ...stubs, "./hub.js": { view: async (_name, args) => { calls.push(args); const a = answers.shift(); if (a instanceof Error) throw a; return a; } } };
  const sources = new Map();
  const folder = new URL("../mods/cockpit/", import.meta.url);
  for (const name of await readdir(folder)) {
    if (!name.endsWith(".js")) continue;
    const text = await readFile(new URL(name, folder), "utf8");
    sources.set(`./${name}`, text);
    for (const match of text.matchAll(/import\s*\{([^}]+)\}\s*from\s*"(herdr|gpui-kit|gpui-base|\.\/hub\.js)"/g)) {
      const exports = (natives[match[2]] ??= {});
      for (const word of match[1].split(",")) {
        const key = word.trim().split(/\s+as\s+/)[0];
        exports[key] ??= () => { throw new Error(`unexpected native call: ${key}`); };
      }
    }
  }
  const modules = new Map();
  const load = (name) => {
    if (modules.has(name)) return modules.get(name);
    const native = natives[name];
    const module = native
      ? new SyntheticModule(Object.keys(native), function () { for (const [k, v] of Object.entries(native)) this.setExport(k, v); }, { context })
      : new SourceTextModule(sources.get(name), { context, identifier: name });
    modules.set(name, module);
    return module;
  };
  const entry = new SourceTextModule('export { default as Review } from "./review.js";', { context });
  await entry.link(load);
  await entry.evaluate();
  return entry.namespace;
}

test("the review reads the changes again by itself once the hub is back, and the error goes even when nothing changed", async () => {
  const model = { scope: "uncommitted", hash: "h1", files: [] };
  const answers = [model, new Error("the cockpit hub was stopped on purpose"), { same: true, hash: "h1" }];
  const { Review } = await reviewModule(answers);
  const r = Object.create(Review.prototype);
  Object.assign(r, { repo: API, scope: "uncommitted", model: null, error: null, loading: false, next: 0, fresh: false, rows: [], loaded: new Set(), step: 0, diff: { whole: () => [], set() {} }, applyStep() {} });
  const work = [];
  const cx = { spawn: (f) => work.push(f(cx)), notify() {} };
  const settle = async () => { while (work.length) await work.shift(); };
  r.load(cx);
  await settle();
  assert.equal(r.error, null);
  r.load(cx);
  await settle();
  assert.match(r.error, /stopped on purpose/, "the hub stopped: said");
  r.load(cx);
  await settle();
  assert.equal(r.error, null, "back with the same changes: the error goes");
  assert.equal(r.model, model);
});

/** Elements as gpui chains them, each keeping what was set on it, so a test finds the ids a view drew. */
function drawing(state) {
  const element = (kind = "div", path = null) => {
    const value = { kind, path, kids: [], made: [] };
    const el = new Proxy(value, { get: (t, name) => {
      if (name in t) return t[name];
      if (name === "when") return (yes, draw) => (yes ? draw(el) : el);
      if (name === "child") return (child) => (value.kids.push(child), el);
      if (name === "children") return (children) => (value.kids.push(...children), el);
      return (...args) => (value.made.push([name, ...args]), el);
    } });
    return el;
  };
  const primitives = Object.fromEntries(["div", "svg", "image", "h_flex", "v_flex"].map((name) => [name, (path) => element(name, path)]));
  const stubs = {
    herdr: { shared: (key) => state.get(key), setShared: (key, value) => state.set(key, value), theme: () => ({ foreground: "#eeeeee", muted: "#aaaaaa", dim: "#888888", background: "#101010", border: "#333333" }) },
    "gpui-kit": primitives,
    "gpui-base": { ...primitives, VirtualListScrollHandle: { new: () => ({ scroll_to_item() {} }) } },
  };
  const flat = (el) => (typeof el === "string" ? [el] : [el, ...(el?.kids ?? []).flatMap(flat)]);
  const ids = (el) => flat(el).flatMap((e) => (e?.made ?? []).filter(([name]) => name === "id").map(([, id]) => id));
  return { stubs, flat, ids };
}

/** A review copy on `repo`, as `init` leaves it but for the parts a test stands in for. */
function copy(Review, repo, extra = {}) {
  const r = Object.create(Review.prototype);
  Object.assign(r, { repo, picked: null, branch: null, scope: "uncommitted", mode: "diff", model: null, error: null, loading: false, next: 0, fresh: false, rows: [], loaded: new Set(), step: 0, width: 700,
    terminal: "app:t41", diff: { whole: () => [], set() {}, review: () => [], composer: null }, applyStep() {}, withOpen: (d) => d, handle: { focus() {} }, ...extra });
  return r;
}

test("with several repositories the review draws a pill per repository and a chip for the branch", async () => {
  const state = new Map([["layout", "cockpit"], ["cockpit.panel.where", { cwd: API, repos: [{ path: API, name: "api", branch: "eng-412-quotas", branches: ["main", "eng-412-quotas"] }, { path: "/work/acme/web", name: "web" }] }]]);
  const d = drawing(state);
  const { Review } = await reviewModule([], { stubs: d.stubs });
  const r = copy(Review, API, { model: { repo: API, name: "api", scope: "branch", files: [], about: { branch: "eng-412-quotas", head: "eng-412-quotas", base: "main", commits: 2, checked_out: true } } });
  r.scope = "branch";
  const ids = d.ids(r.header(new Proxy({}, { get: () => "#888888" }), false));
  assert.ok(ids.includes(`review-repo-${API}`) && ids.includes("review-repo-/work/acme/web"), `a pill per repository: ${ids}`);
  assert.ok(ids.includes("review-branch"), "the branch chip");
  assert.ok(d.flat(r.header(new Proxy({}, { get: () => "#888888" }), false)).includes("eng-412-quotas · → main · 2 commits"), "the header's facts");
  assert.deepEqual(JSON.parse(JSON.stringify(r.branches())), ["eng-412-quotas", "main"], "the branches to pick from: the head, then the repository's, each once");
  // One repository: no pills, still the chip.
  state.set("cockpit.panel.where", { cwd: API, repos: [{ path: API, name: "api" }] });
  const one = d.ids(r.header(new Proxy({}, { get: () => "#888888" }), false));
  assert.ok(!one.some((id) => String(id).startsWith("review-repo-")) && one.includes("review-branch"));
});

test("a branch picked from the menu is taken by the copy reviewing that repository alone, and goes with the next read", async () => {
  const calls = [];
  const model = (repo) => ({ repo, scope: "branch", hash: "h1", files: [], about: { branch: "feat", head: "main", base: "main", commits: 1, checked_out: false } });
  const { Review } = await reviewModule([model("/r/api"), model("/r/api")], { calls, stubs: drawing(new Map()).stubs });
  const api = copy(Review, "/r/api", { model: { repo: "/r/api", about: { branch: "main", head: "main", checked_out: true } } });
  const web = copy(Review, "/r/web", { model: { repo: "/r/web", about: { branch: "main", head: "main", checked_out: true } } });
  const work = [];
  const cx = { spawn: (f) => work.push(f(cx)), notify() {} };
  const settle = async () => { while (work.length) await work.shift(); };
  for (const r of [api, web]) r.command("review:branch:feat|/r/api", cx);
  assert.equal(api.branch, "feat");
  assert.equal(web.branch, null, "the other repository's copy leaves it");
  assert.equal(api.scope, "branch", "Uncommitted gives way: another branch has no working tree here");
  await settle();
  assert.equal(calls.at(-1).branch, "feat");
  assert.equal(calls.at(-1).repo, "/r/api");
  assert.equal(api.elsewhere(), true, "the hub says it is not checked out");
  // Uncommitted asked while the branch is not checked out reads the branch instead; Uncommitted picked on purpose forgets the branch.
  api.scope = "uncommitted";
  api.load(cx);
  await settle();
  assert.equal(calls.at(-1).scope, "branch", "coerced before the request");
  assert.equal(api.scope, "branch");
  api.setScope("uncommitted", cx);
  assert.equal(api.branch, null);
  await settle();
  assert.equal(calls.at(-1).branch, null);
});
