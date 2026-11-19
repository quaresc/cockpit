import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import { createContext, SourceTextModule, SyntheticModule } from "node:vm";
import { test } from "node:test";

// usage.js (or another mod, `entry`) with Herdr's natives stubbed: only its words and figures run here, never a drawing.
async function usage(entry = "./usage.js", natives = {}, withDefault = false) {
  const context = createContext({ console });
  const imports = { ...natives, "./hub.js": { act: () => { throw new Error("no hub action expected"); }, subscribe: () => ({ stop() {} }), ...(natives["./hub.js"] ?? {}) } };
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
  const module = new SourceTextModule(`export * from "${entry}"; ${withDefault ? `export { default } from "${entry}";` : ""}`, { context });
  await module.link(load);
  await module.evaluate();
  return module.namespace;
}

// The acme quota rows as the hub sends them: Claude fresh, Codex stale until its known reset, and a verdict with no
// figure.
const CLAUDE_5H = { agent: "claude", name: "5h", label: "Claude 5h", percent: 64, figure: "64%", floor: false, state: "fresh", resets_at: 2000, resets_words: "resets in 1 h 35 min", as_of_words: "as of 12:04", warn: false };
const CLAUDE_7D = { agent: "claude", name: "7d", label: "Claude 7d", percent: 38, figure: "38%", floor: false, state: "fresh", resets_at: 9000, resets_words: "resets in 4 d", as_of_words: "as of 12:04", warn: false };
const CODEX_7D = { agent: "codex", name: "7d", label: "Codex 7d", percent: 12, figure: "≥12%", floor: true, state: "stale", resets_at: 5000, resets_words: "resets in 3 d", as_of_words: "as of 09:01", warn: false };
const NEAR = { agent: "claude", name: "5h", label: "Claude 5h", percent: null, figure: "near", status: "allowed_warning", state: "fresh", warn: true };
// A newer "rejected" verdict over an older 64 %: the limit is reached whatever the percent said.
const REACHED = { ...CLAUDE_5H, status: "rejected", figure: "full", warn: true };

test("a figure is the hub's own, a stale one a floor only until its known reset, and a verdict stays a word", async () => {
  const u = await usage();
  assert.equal(u.valueWords(CLAUDE_5H), "64%");
  assert.equal(u.valueWords(CODEX_7D), "≥12%");
  assert.equal(u.valueWords(NEAR), "near");
  assert.equal(u.valueWords(REACHED), "full");
  const bare = ({ figure, ...q }) => q;
  assert.equal(u.valueWords(bare(CODEX_7D)), "≥12%", "without a figure, the floor mark comes from `floor`");
  assert.equal(u.valueWords(bare({ ...CODEX_7D, floor: false, resets_at: null })), "12%", "a stale reading with no known reset is no floor");
  assert.equal(u.valueWords(bare(REACHED)), "full", "a newer rejected verdict wins over an older percent");
});

test("bars warn past 80 %, ahead of pace or on a warning verdict, and go red near the end", async () => {
  const u = await usage();
  assert.equal(u.barTone(CLAUDE_5H), "success");
  assert.equal(u.barTone({ ...CLAUDE_5H, percent: 81 }), "warning");
  assert.equal(u.barTone({ ...CLAUDE_5H, ahead: true }), "warning");
  assert.equal(u.barTone(NEAR), "warning");
  assert.equal(u.barTone({ ...CLAUDE_5H, percent: 96 }), "destructive");
  assert.equal(u.barTone(REACHED), "destructive", "a reached limit is red at any percent");
});

test("the tightest window is the most used, the soonest reset breaking a tie", async () => {
  const u = await usage();
  assert.equal(u.tightestOf([CLAUDE_7D, CODEX_7D, CLAUDE_5H]).label, "Claude 5h");
  assert.equal(u.tightestOf([{ ...CLAUDE_7D, percent: 64 }, CLAUDE_5H]).name, "5h");
  assert.equal(u.tightestOf([]), null);
  assert.equal(u.tightestOf([{ ...CLAUDE_7D, percent: 90 }, REACHED]).figure, "full", "a reached limit is the tightest");
});

test("an agent's windows read as a tooltip, stale ones marked", async () => {
  const u = await usage();
  assert.equal(u.windowsWords([CLAUDE_5H, CLAUDE_7D, CODEX_7D], "codex"), "Codex 7d ≥12% · resets in 3 d · stale, as of 09:01");
  // With the reading's time, staleness reads as every freshness chip says it (fresh.js).
  assert.equal(u.windowsWords([{ ...CODEX_7D, as_of: Date.now() - 40 * 60_000 }], "codex"), "Codex 7d ≥12% · resets in 3 d · stale, 40 min ago");
  assert.equal(u.windowsWords([CLAUDE_5H, CLAUDE_7D], "claude").split("\n").length, 2);
  assert.equal(u.windowsWords([CLAUDE_5H], "opencode"), "", "an agent that reports nothing gets no line");
});

test("spend is always an estimate, and none is shown before the count", async () => {
  const u = await usage();
  assert.deepEqual({ ...u.spendTile({ today: 0.8, week: 3.49, estimated: true, unpriced: [], covers: null }) }, { value: "~$0.80", hint: "~$3.49 these 7 days, estimated" });
  assert.equal(
    u.spendTile({ today: 0.8, week: 3.49, estimated: true, unpriced: ["gpt-5-codex"], covers: "Claude only" }).hint,
    "~$3.49 these 7 days, estimated · Claude only",
    "an agent with no price is named out of the estimate",
  );
  assert.equal(
    u.spendTile({ today: 0.8, week: 3.49, estimated: true, unpriced: ["claude-opus-4-5"], covers: "Claude, some models without a price" }).hint,
    "~$3.49 these 7 days, estimated · Claude, some models without a price",
  );
  assert.equal(u.spendTile(null), null, "no tile when no token has a price, rather than a $0");
  assert.deepEqual(
    [u.price({ total: 3300, unpriced: 3300, cost: 0 }), u.price({ total: 4000, unpriced: 3300, cost: 0.04 }), u.price({ total: 0, unpriced: 0, cost: 0 })],
    ["no price", "~$0.04", "·"],
  );
  assert.equal(u.estimate(0.004), "~$0.004");
  assert.equal(u.estimate(null), "–");
  assert.deepEqual([u.count(950), u.count(34_200), u.count(1_240_000)], ["950", "34K", "1.2M"]);
});

test("the windows group by provider with its icon, and a running agent that reports nothing says so", async () => {
  const u = await usage();
  const as = (q, provider) => ({ ...q, provider, icon: q.agent });
  const running = [{ agent: "claude", icon: "claude" }, { agent: "", program: "gemini", icon: "gemini", agent_name: "Gemini", reports_limits: false }, { agent: "codex", icon: "codex" }];
  const groups = u.byProvider([as(CLAUDE_5H, "Claude"), as(CODEX_7D, "Codex"), as(CLAUDE_7D, "Claude")], running);
  const plain = (v) => JSON.parse(JSON.stringify(v));
  assert.deepEqual(
    plain(groups.map((g) => [g.agent, g.icon, g.name, g.windows.map((w) => w.name), g.unreported === true])),
    [["claude", "claude", "Claude", ["5h", "7d"], false], ["codex", "codex", "Codex", ["7d"], false], ["gemini", "gemini", "Gemini", [], true]],
    "one group per provider, in the order its windows come, then the agents that report none",
  );
  assert.equal(groups[0].tightest.name, "5h", "each provider's tightest window");
  assert.equal(groups[2].tightest, null);
  assert.equal(u.byProvider([CLAUDE_5H])[0].icon, "agent", "an older hub never makes an icon filename from its agent id");
  assert.deepEqual(plain(u.byProvider([], running).map((g) => g.unreported)), [false, true, false], "a known unsupported source differs from a reading not received yet");
});

test("an agent's icon is the hub's name, else Herdr's program's, else the neutral one, and never a path", async () => {
  const ui = await usage("./ui.js");
  assert.equal(ui.iconOf({ icon: "antigravity" }), "antigravity");
  assert.equal(ui.iconOf({ program: { icon: "icons/claude.svg", tint: false } }), "claude", "Herdr's own agent, while the hub is away");
  assert.equal(ui.iconOf("codex"), "codex");
  for (const bad of [null, {}, "", "../../secrets", "icons/x.svg", { icon: "a/b" }]) assert.equal(ui.iconOf(bad), "agent", JSON.stringify(bad));
  assert.deepEqual(
    JSON.parse(JSON.stringify(ui.providersOf([{ icon: "claude", agent_name: "Claude Code" }, { icon: "codex" }, { icon: "claude" }]).map((p) => [p.icon, p.name]))),
    [["claude", "Claude Code"], ["codex", null]],
    "each provider once, in order",
  );
});


test("generic agents are skipped and a useful Herdr program replaces the neutral hub icon", async () => {
  const u = await usage();
  const ui = await usage("./ui.js");
  assert.equal(ui.iconOf({ icon: "agent", program: { key: "pig", icon: "icons/pig.svg", tint: false } }), "pig");
  assert.equal(u.byProvider([], [{ agent: "agent", icon: "agent" }]).length, 0);
  const groups = u.byProvider([], [{ agent: "claude", icon: "claude", reports_limits: true }, { agent: "gemini", icon: "gemini", reports_limits: false }]);
  assert.deepEqual(JSON.parse(JSON.stringify(groups.map((g) => [g.agent, g.unreported]))), [["claude", false], ["gemini", true]]);
});

test("Copilot with no reading says why: not reported without gh, unlimited on a plan with no limit, never waiting forever", async () => {
  const u = await usage();
  const running = [
    { agent: "codex", icon: "codex", agent_name: "Codex", reports_limits: true, limits: "reports" },
    { agent: "copilot", icon: "githubcopilot", agent_name: "Copilot", reports_limits: true, limits: "unlimited" },
    { agent: "kiro", icon: "kiro", agent_name: "Kiro", reports_limits: false, limits: "unreported" },
  ];
  const kinds = u.byProvider([], running).map((g) => [g.agent, g.unreported, g.unlimited]);
  assert.deepEqual(JSON.parse(JSON.stringify(kinds)), [["codex", false, false], ["copilot", false, true], ["kiro", true, false]]);
});

test("logo drawing takes picture metadata from Herdr, brand colours in both layouts, mono marks when tinted", async () => {
  const ui = await usage("./ui.js");
  const programs = [{ key: "codex", icon: "icons/codex.svg", tint: false }, { key: "pi", icon: "icons/pi.svg", tint: true }];
  const plain = (v) => JSON.parse(JSON.stringify(v));
  assert.deepEqual(plain(ui.agentLogoSpec("codex", { mono: false, programs })), { name: "codex", path: "icons/codex.svg", picture: true });
  assert.deepEqual(plain(ui.agentLogoSpec("codex", { mono: true, programs })), { name: "codex", path: "mods/cockpit/icons/agents/codex.svg", picture: false });
  assert.equal(ui.agentLogoSpec("pi", { mono: false, programs }).picture, false);
  assert.equal(ui.CHAT_LOGO_STYLE, "brand");
});


function drawing() {
  const state = new Map([["layout", "cockpit"]]);
  const element = (kind = "div", path = null) => {
    const value = { kind, path, kids: [], made: [] };
    const el = new Proxy(value, { get: (t, name) => {
      if (name in t) return t[name];
      if (name === "when") return (yes, draw) => yes ? draw(el) : el;
      if (name === "child") return (child) => (value.kids.push(child), el);
      if (name === "children") return (children) => (value.kids.push(...children), el);
      return (...args) => (value.made.push([name, ...args]), el);
    } });
    return el;
  };
  const primitives = Object.fromEntries(["div", "svg", "image", "h_flex", "v_flex"].map((name) => [name, (path) => element(name, path)]));
  const natives = {
    herdr: { shared: (key) => state.get(key), setShared: (key, value) => state.set(key, value), knownAgents: async () => [],
      theme: () => ({ foreground: "#eeeeee", muted: "#aaaaaa", dim: "#888888", background: "#101010", border: "#333333" }) },
    "gpui-kit": primitives, "gpui-base": primitives,
  };
  const flat = (el) => typeof el === "string" ? [el] : [el, ...(el?.kids ?? []).flatMap(flat)];
  return { natives, flat, state };
}

test("limits without readings still draw honest words, and long window names have room", async () => {
  const f = drawing();
  const u = await usage("./usage.js", f.natives);
  const rows = [{ agent: "claude", icon: "claude", agent_name: "Claude", reports_limits: true }, { agent: "gemini", icon: "gemini", agent_name: "Gemini", reports_limits: false }];
  const missing = f.flat(u.QuotaGroups([], { rows }));
  assert.ok(missing.includes("Claude · no reading yet"));
  assert.ok(missing.includes("Gemini · not reported"));
  const grouped = f.flat(u.QuotaGroups([{ ...CLAUDE_5H, icon: "claude", name: "7d opus" }], { compact: true }));
  const label = grouped.find((el) => el?.kids?.includes("7d opus"));
  assert.ok(label.made.find(([key, width]) => key === "w" && width >= 44));
});

test("Cockpit settings draws the hub-provided agent logo beside its name", async () => {
  const f = drawing();
  const d = await usage("./doctor.js", f.natives, true);
  const view = Object.create(d.default.prototype);
  view.report = { agents: [{ id: "acme-helper", name: "Acme helper", icon: "codex", installed: true }] };
  const all = f.flat(view.agentsEl({ foreground: "#eee", mutedFg: "#aaa", border: "#333", dim: "#888" }));
  assert.ok(all.some((el) => el?.path === "icons/codex.svg"));
  assert.ok(all.includes("Acme helper"));
});


test("the Chat drawer keeps its agents and no-reading row when the hub has never answered", async () => {
  const f = drawing();
  f.state.set("layout", "cockpit-chat");
  f.state.set("cockpit.chat.sidebar", false);
  Object.assign(f.natives.herdr, { agents: () => [{ address: "t1", tab: "tab-a", title: "Quota checks", status: "idle", program: { key: "codex", name: "Codex", icon: "icons/codex.svg", tint: false } }],
    focused: () => ({ terminal: "t1", tab: "tab-a" }), spaces: () => [], keys: () => [],
    layout: () => ({ id: "root", children: [{ id: "sidebar", hidden: true }] }) });
  f.natives["./hub.js"] = { remoteMachine: () => null };
  const u = await usage("./side-peek.js", f.natives, true);
  const view = Object.assign(Object.create(u.default.prototype), { rows: null, down: true, update: null });
  const all = f.flat(view.render());
  assert.ok(all.includes("no reading yet"));
  assert.ok(all.includes("Quota checks"));
});

test("space-tree terminal rows draw the agent logo as well as the status mark", async () => {
  const f = drawing();
  const u = await usage("./sidebar.js", f.natives, true);
  const view = Object.create(u.default.prototype);
  const p = { terminal: "t1", pane: "p1", tab: "tab-a", program: { key: "codex", name: "Codex", icon: "icons/codex.svg", tint: false, agent: true }, status: "idle" };
  const rows = view.termRows({ foreground: "#eee", mutedFg: "#aaa", dim: "#888", accent: "#222" }, { tabs: [{ id: "tab-a" }] }, [p], new Map(), false);
  const all = rows.flatMap(f.flat);
  assert.ok(all.some((el) => el?.path === "icons/codex.svg"));
  assert.ok(all.includes("Codex"));
});
