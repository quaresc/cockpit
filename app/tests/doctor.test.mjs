import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import { createContext, SourceTextModule, SyntheticModule } from "node:vm";
import { test } from "node:test";

const REPORT = {
  version: 1, at: 1793611507779, cockpit: "0.40.3", summary: { pass: 1, warn: 1, fail: 1 },
  checks: [
    { id: "tools.gh", category: "tools", status: "fail", message: "gh not signed in", fix: { label: "gh auth login", command: "gh auth login" } },
    { id: "sources.cache.inbox", category: "sources", status: "warn", message: "inbox: no data yet", fix: { label: "open the inbox", act: { app: "sources", action: "refresh", args: [] } } },
    { id: "claude.plugin", category: "claude", status: "pass", message: "Claude plugin cockpit 0.40.3", fix: null },
  ],
  getting_started: [
    { id: "setup", label: "Set up cockpit", done: true, check: "tools.config", fix: null },
    { id: "github", label: "Sign in to GitHub", done: false, check: "tools.gh", fix: { label: "gh auth login", command: "gh auth login" } },
    { id: "tools", label: "Give Claude Code cockpit's tools", done: false, check: "claude.plugin", fix: { label: "/plugin install cockpit" } },
    { id: "tickets", label: "Connect your tickets", done: false, check: null, fix: { label: "cockpit setup", command: "cockpit setup" } },
  ],
  agents: [],
};

/** The hub's `options` view: a switch seen, one an update brought, a choice, a field, a number and one set by hand. */
const OPTIONS = [
  { key: "queue.drain", section: "Queue and CI", since: "0.41.0", title: "Send queued prompts on their own", description: "A prompt goes once the agent rests.", default: true, kind: "toggle", value: true, seen: true },
  { key: "chat.nextSteps", section: "Chat", since: "0.41.0", title: "Suggest next prompts", description: "A few prompts after each turn.", default: false, kind: "toggle", value: false, seen: false },
  { key: "agents.pace", section: "Usage", since: null, title: "Pace of the 7-day limit", description: "Where the tick stands.", default: "week", kind: "choice", value: "week", seen: true,
    choices: [{ value: "week", label: "Whole week" }, { value: "weekdays", label: "Weekdays" }] },
  { key: "brief.at", section: "Briefs and recaps", since: null, title: "Morning brief time", description: "When the brief is written.", default: "08:30", kind: "text", value: "08:30", blank: null, seen: true },
  { key: "refresh.github", section: "Refresh", since: null, title: "GitHub", description: "Seconds between two reads.", default: 30, kind: "number", value: 30, min: 15, max: 86400, unit: "s", seen: true },
  { key: "meetingBrief.model", section: "Briefs and recaps", since: null, title: "Model for meeting briefs", description: "The model.", default: "sonnet", kind: "text", value: "sonnet", by_hand: true, blank: null, seen: true },
];
const SECTIONS = ["Agents", "Queue and CI", "Chat", "Usage", "Refresh", "Briefs and recaps"];
const IN_FILE = [{ key: "usage.commands", description: "A command per agent id that prints its limits." }];

/** A text field as Herdr's InputState, with what a person does to it: `type`, `submit`, `click` (focus), `blur`. */
function input({ value = "", placeholder = "" } = {}) {
  const on = {};
  const state = {
    text: value, placeholder,
    value: () => state.text,
    set_value: (v) => ((state.text = v), on.change?.({}, null)),
    on: (name, fn) => (on[name] = fn),
    focus: () => on.focus?.({}, null),
    type: (v, cx) => ((state.text = v), on.change?.({}, cx)),
    submit: (cx) => on.submit?.({}, cx),
    blur: () => on.blur?.({}, null),
  };
  return state;
}

/** Elements that only keep what was put in them, so a test reads the words a view draws. */
function drawing() {
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
  const flat = (el) => (typeof el === "string" ? [el] : [el, ...(el?.kids ?? []).flatMap(flat)]);
  /** Every word drawn under an element, and the tooltips on the way. */
  const words = (el) => flat(el).flatMap((x) => (typeof x === "string" ? [x] : (x?.made ?? []).filter(([name]) => name === "tooltip").map(([, tip]) => tip)));
  return { primitives, flat, words, element };
}

/**
 * A mod (`entry`, doctor.js by default) on a fake hub: `cockpit` answers with `stdout`, or each run with the next of
 * `answers` (a function, so a test can hold one back); `act` records its calls, saves an option as the hub would
 * (`refuse[key]` says why not) and checks for updates as `published` says; `subscribe` gives the options view and the
 * link's state, which `link(state)` changes.
 */
async function fixture(stdout = JSON.stringify(REPORT), answers = null, { entry = "./doctor.js", layout = "cockpit" } = {}) {
  const acts = [], ran = [], opened = [], closed = [], started = [], subs = [], executed = [], commands = [], notices = [];
  const d = drawing();
  const state = { options: OPTIONS.map((o) => ({ ...o })), refuse: {}, hub: "live", failure: null, files: {}, published: { latest: "0.40.4" }, startFails: null, stops: 0, viewFails: false };
  const view = () => ({ next: "0.41.0", sections: SECTIONS, options: state.options.map((o) => ({ ...o })), in_file: IN_FILE, path: "/Users/me/.config/cockpit/config.json", update: null, layout: null });
  const context = createContext({ console });
  const shared = new Map([["layout", layout]]);
  const imports = {
    herdr: {
      shared: (key) => shared.get(key), setShared: (key, value) => shared.set(key, value), props: () => ({ overlay: "o1" }),
      openOverlay: (o) => opened.push(o), closeOverlay: (o) => closed.push(o),
      run: (name, args) => (commands.push([name, args]), null), notify: (n) => notices.push(n),
      readTextFile: async (path) => state.files[path] ?? null,
      exec: async (...args) => (executed.push(args), { code: 0, stdout: "", stderr: "" }),
      window: () => ({ platform: "macos" }),
      theme: () => ({ foreground: "#eeeeee", muted: "#aaaaaa", dim: "#888888", background: "#101010", border: "#333333", hover: "#222222", accent: "#5555ff", ansi1: "#ff0000", ansi2: "#00ff00" }),
    },
    "gpui-kit": { ...d.primitives },
    "gpui-base": { ...d.primitives, InputState: { new: (o) => input(o) }, Input: { new: (s) => Object.assign(d.element("input"), { state: s }) } },
    "./hub.js": {
      cockpit: async (args) => (ran.push(args), answers ? answers.shift()(args) : { code: 0, stdout, stderr: "" }),
      act: async (...args) => {
        acts.push(args);
        const [app, action, [arg] = []] = args;
        if (app === "options" && action === "set") {
          const [[key, value]] = Object.entries(arg);
          if (state.refuse[key]) return { note: "", error: state.refuse[key] };
          // The hub writes a time of day with two digits each.
          const saved = key === "brief.at" ? String(value).padStart(5, "0") : value;
          state.options = state.options.map((o) => (o.key === key ? { ...o, value: saved, seen: true } : o));
          return { note: `${key} saved` };
        }
        if (app === "update" && action === "check") {
          const at = 1793611600000;
          const path = "/Users/me/.cache/cockpit/update.json";
          if (state.published.error) {
            state.files[path] = JSON.stringify({ ...JSON.parse(state.files[path] ?? "{}"), at, error: { message: state.published.error, at } });
            return { note: "", error: `check: ${state.published.error}` };
          }
          state.files[path] = JSON.stringify({ at, current: "0.40.4", latest: state.published.latest, dev: Boolean(state.published.dev), notes: [] });
          return { note: "ok" };
        }
        if (app === "doctor" && action === "fix_all") return state.fixAll ?? { note: "Fixed 2\nrunning gh auth login in the actions space\nrefresh requested, it lands within a minute" };
        return { note: "running gh auth login in the actions space" };
      },
      view: async () => {
        if (state.viewFails) throw new Error("the cockpit hub did not answer");
        return view();
      },
      subscribe: (cx, _what, onMessage, onState) => {
        const sub = { onMessage, onState, cx, stop: () => (state.stops += 1) };
        subs.push(sub);
        onState(state.hub, cx);
        if (state.hub === "live") onMessage({ view: "options", value: view() }, cx);
        return sub;
      },
      install: async () => ({ home: "/Users/me", cache: "/Users/me/.cache/cockpit", version: "0.40.4" }),
      startHub: async (o) => {
        started.push(o);
        if (state.startFails) throw new Error(state.startFails);
      },
      remoteMachine: () => null,
      // As hub.js bounds a wait: `state.gaveUp` (ms) gives a hung read up, else the promise is waited for.
      within: (_cx, promise, _ms, what) => (state.gaveUp ? Promise.race([promise, new Promise((_, reject) => setTimeout(() => reject(new Error(`${what} did not answer`)), state.gaveUp))]) : promise),
      lastFailure: () => state.failure,
      isNotInstalled: (e) => String(e).includes("not installed in this Herdr home"),
      isStoppedOnPurpose: (e) => String(e).includes("stopped on purpose"),
      downWords: (e, short = false) => (String(e).includes("stopped on purpose") ? (short ? "Hub stopped" : "The cockpit hub was stopped") : short ? "Hub not running" : "The cockpit hub isn't running"),
      downReason: (e) => String(e).replace(/^error:\s*/, ""),
      downTip: (e) => `The cockpit hub isn't running: ${String(e).replace(/^error:\s*/, "")}`,
      openUrl: async () => ({ code: 0 }),
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
  const withDefault = /export default/.test(sources.get(entry) ?? "");
  const module = new SourceTextModule(`export * from "${entry}"; ${withDefault ? `export { default } from "${entry}";` : ""}`, { context });
  await module.link(load);
  await module.evaluate();
  /** The link to the hub changes for every sheet that follows it. */
  const link = (now) => {
    state.hub = now;
    for (const s of subs) {
      s.onState(now, s.cx);
      if (now === "live") s.onMessage({ view: "options", value: view() }, s.cx);
    }
  };
  return { doctor: module.namespace, mod: module.namespace, acts, ran, opened, closed, started, subs, executed, commands, notices, state, link, shared, ...d };
}

/** Values made in the module's own realm, as plain ones to compare. */
const plain = (v) => JSON.parse(JSON.stringify(v));

/** A context whose spawned work runs at once, its clipboard and toasts kept. */
function cx() {
  const copied = [], tasks = [];
  return { copied, tasks, write_to_clipboard: (t) => copied.push(t), spawn: (f) => tasks.push(f(cx())), notify: () => {} };
}

/** A sheet opened on a fixture, its toasts kept as `[title, action label, description]`, and its tasks to wait for. */
async function sheet(f, p = {}) {
  const tasks = [], toasts = [];
  const cx = { spawn: (fn) => tasks.push(fn(cx)), notify: () => {}, focus_handle: () => ({ focus() { cx.focused = (cx.focused ?? 0) + 1; } }), write_to_clipboard() {}, timer: { after: () => ({ cancel() {} }) } };
  const s = new f.doctor.default();
  s.init(p, cx);
  s.toasts = { push: (_cx, title, o) => toasts.push([title, o?.action?.label, o?.description, o?.action]), render: () => null };
  const settled = async () => {
    for (let n = -1; n !== tasks.length; ) {
      n = tasks.length;
      await Promise.all(tasks);
    }
  };
  await settled();
  const sets = () => plain(f.acts.filter(([app, action]) => app === "options" && action === "set").map(([, , args]) => args[0]));
  const option = (key) => s.options.find((o) => o.key === key);
  return { s, cx, toasts, settled, sets, option };
}

test("the report is read from cockpit doctor --json, cached when asked", async () => {
  const f = await fixture();
  const report = await f.doctor.loadDoctor({ cached: true });
  assert.deepEqual(plain(f.ran), [["doctor", "--json", "--cached"]]);
  assert.equal(report.summary.fail, 1);
  assert.deepEqual(plain(f.doctor.toDo(report).map((s) => s.id)), ["github", "tools", "tickets"]);
  const broken = await fixture("");
  await assert.rejects(broken.doctor.loadDoctor(), /cockpit doctor did not answer/);
});

test("a fix runs through the hub on the report it was shown in, and words are copied", async () => {
  const f = await fixture();
  assert.equal(f.doctor.fixVerb(REPORT.checks[0].fix), "Run");
  assert.equal(f.doctor.fixVerb(REPORT.checks[1].fix), "Fix");
  assert.equal(f.doctor.fixVerb(REPORT.getting_started[2].fix), null);
  const toasts = { pushed: [], push: (_cx, title) => toasts.pushed.push(title) };
  const c = cx();
  let done = 0;
  f.doctor.runFix(REPORT.getting_started[1], REPORT, c, toasts, () => done++);
  await Promise.all(c.tasks);
  assert.deepEqual(plain(f.acts), [["doctor", "fix", ["tools.gh", REPORT.at, "gh auth login"]]], "a step fixes through its check, naming the report and the fix shown");
  assert.deepEqual(toasts.pushed, ["running gh auth login in the actions space"]);
  assert.equal(done, 1, "the caller hears the hub took it, to check again");
  f.doctor.runFix(REPORT.getting_started[2], REPORT, c, toasts);
  assert.deepEqual(c.copied, ["/plugin install cockpit"]);
  assert.equal(f.acts.length, 1, "nothing runs a fix said in words");
});

test("a step no check says is copied, never sent as a check", async () => {
  const f = await fixture();
  const step = REPORT.getting_started[3];
  assert.equal(f.doctor.runnable(step), false);
  assert.equal(f.doctor.runnable(REPORT.getting_started[1]), true);
  const c = cx();
  f.doctor.runFix(step, REPORT, c, { push: () => {} });
  await Promise.all(c.tasks);
  assert.deepEqual(plain(c.copied), ["cockpit setup"]);
  assert.equal(f.acts.length, 0);
});

test("a fix says what it will do before it runs", async () => {
  const f = await fixture();
  assert.deepEqual(plain(f.doctor.fixQuestion(REPORT.checks[0])), {
    title: "Run this command?", description: "gh auth login runs in a tab of the actions space, where you can watch it.", confirm: "Run",
  });
  assert.deepEqual(plain(f.doctor.fixQuestion(REPORT.checks[1])), { title: "Fix it?", description: "Reads every source again now.", confirm: "Fix" });
  assert.equal(f.doctor.shownFix(REPORT.checks[1].fix), 'sources refresh []', "an action as the hub compares it");
});

test("a confirm asked from one report closes when another lands, and never runs that one's fix", async () => {
  const A = { ...REPORT, at: 1000, checks: [{ id: "install.link", category: "install", status: "fail", message: "no link", fix: { label: "ln -sfn /a ~/.config/cockpit/herdr", command: "ln -sfn /a ~/.config/cockpit/herdr" } }] };
  const B = { ...A, at: 2000, checks: [{ ...A.checks[0], fix: { label: "sh /b/ensure-spaces.sh", command: "sh /b/ensure-spaces.sh" } }] };
  let release;
  const held = new Promise((r) => (release = r));
  const f = await fixture(undefined, [async () => ({ code: 0, stdout: JSON.stringify(A), stderr: "" }), async () => (await held, { code: 0, stdout: JSON.stringify(B), stderr: "" })]);
  const tasks = [];
  const cx = { spawn: (fn) => tasks.push(fn(cx)), notify: () => {}, focus_handle: () => ({ focus() {} }), write_to_clipboard() {} };
  const sheet = new f.doctor.default();
  const toasts = [];
  sheet.init({ tab: "health" }, cx);
  sheet.toasts = { push: (_cx, title) => toasts.push(title), render: () => null };
  await new Promise((r) => setTimeout(r, 10));
  assert.equal(sheet.report.at, 1000, "the kept report shows first");
  sheet.ask(sheet.report.checks[0], cx);
  assert.equal(f.doctor.fixQuestion(sheet.asking.item).description, "ln -sfn /a ~/.config/cockpit/herdr runs in a tab of the actions space, where you can watch it.");
  release();
  await Promise.all(tasks);
  assert.equal(sheet.report.at, 2000);
  assert.equal(sheet.asking, null, "the question asked from the old report is gone");
  assert.deepEqual(toasts, ["The checks ran again: look again"]);
  sheet.confirmed(cx);
  await Promise.all(tasks);
  assert.deepEqual(plain(f.acts.filter(([app]) => app === "doctor")), [], "nothing runs");
  // Asked and confirmed from the report shown, the hub is told that report and that fix.
  sheet.ask(sheet.report.checks[0], cx);
  sheet.asking.opened = 0;
  sheet.confirmed(cx);
  await Promise.all(tasks);
  assert.deepEqual(plain(f.acts.filter(([app]) => app === "doctor")), [["doctor", "fix", ["install.link", 2000, "sh /b/ensure-spaces.sh"]]]);
});

test("the sheet opens on Options, on Health for the name it had, and its link opens on Health while a check fails", async () => {
  const f = await fixture();
  f.doctor.openSettings();
  f.doctor.openSettings({ tab: "health" });
  assert.deepEqual(plain(f.opened.map((o) => [o.kind, o.key, o.props])), [["window", "cockpit-settings", { app: "settings", tab: null, person: false }], ["window", "cockpit-settings", { app: "settings", tab: "health", person: false }]]);
  assert.deepEqual([f.doctor.sheetWidth({ width: 1600 }), f.doctor.sheetWidth({ width: 760 }), f.doctor.sheetWidth({ width: 200 }), f.doctor.sheetWidth(null)], [860, 728, 320, 860], "it fits a narrow window");
  assert.equal((await sheet(f)).s.tab, "options");
  assert.equal((await sheet(f, { tab: "agents" })).s.tab, "agents");
  // The two tabs the checks had before are Health, the second with every check.
  const problems = (await sheet(f, { tab: "problems" })).s;
  assert.deepEqual([problems.tab, problems.all], ["health", false]);
  const all = (await sheet(f, { tab: "all" })).s;
  assert.deepEqual([all.tab, all.all], ["health", true]);
  assert.equal((await sheet(f, { tab: "made-up" })).s.tab, "options");
  // The sidebar's entry: its name, its failing count, and where a click lands.
  f.shared.set("cockpit.doctor.failing", 0);
  const quiet = f.doctor.SettingsLink("sidebar-settings");
  assert.ok(f.words(quiet).includes("Cockpit settings") && f.words(quiet).includes("Cockpit settings  ⌘⇧P, then Cockpit settings"));
  assert.ok(!f.words(quiet).some((w) => /failing/.test(w)));
  quiet.made.find(([name]) => name === "on_click")[1]();
  assert.equal(f.opened.at(-1).props.tab, null);
  f.shared.set("cockpit.doctor.failing", 2);
  const failing = f.doctor.SettingsLink("sidebar-settings");
  assert.ok(f.words(failing).includes("2 failing") && f.words(failing).includes("Cockpit settings: 2 failing  ⌘⇧P, then Cockpit settings"));
  failing.made.find(([name]) => name === "on_click")[1]();
  assert.deepEqual([f.opened.at(-1).props.tab, f.opened.at(-1).props.person], ["health", true], "the person's click");
  // Cockpit Chat keeps the key out of sight: its tooltip names no key.
  f.shared.set("layout", "cockpit-chat");
  assert.ok(f.words(f.doctor.SettingsLink("peek-settings")).includes("Cockpit settings: 2 failing"));
});

test("the options tab notes what it shows as seen and sets an option from the person's own key only", async () => {
  const f = await fixture();
  const { s, cx, toasts, settled, sets } = await sheet(f, { person: true });
  assert.equal(s.tab, "options", "the sheet opens on its options");
  const optionActs = () => plain(f.acts.filter(([app]) => app === "options").map(([app, action, args]) => [app, action, args]));
  assert.deepEqual(optionActs(), [["options", "shown", [{ keys: ["chat.nextSteps"] }]]], "the one not seen yet, once");
  s.key({ keystroke: "j" }, cx);
  s.key({ keystroke: "space", injected: true }, cx);
  await settled();
  assert.equal(optionActs().length, 1, "a key from outside the window sets nothing");
  s.key({ keystroke: "space" }, cx);
  await settled();
  assert.deepEqual(sets(), [{ "chat.nextSteps": true }]);
  assert.deepEqual(toasts.map((t) => t.slice(0, 2)), [["Suggest next prompts: on", "Undo"]]);
  assert.equal(s.options.find((o) => o.key === "chat.nextSteps").value, true);
  // Undo sets it back, and its own toast offers no second Undo.
  toasts[0][3].onClick({}, cx);
  await settled();
  assert.deepEqual(sets().at(-1), { "chat.nextSteps": false });
  assert.deepEqual(toasts.at(-1).slice(0, 2), ["Suggest next prompts: off", undefined]);
});

test("a sheet a bare command opened notes nothing as seen until the person's own key or click", async () => {
  const f = await fixture();
  const { s, cx, settled } = await sheet(f);
  const shown = () => plain(f.acts.filter(([app, action]) => app === "options" && action === "shown").map(([, , args]) => args[0]));
  assert.deepEqual(shown(), [], "opened by a bare command");
  s.key({ keystroke: "j", injected: true }, cx);
  s.key({ keystroke: "j", is_held: true }, cx);
  await settled();
  assert.deepEqual(shown(), [], "nor an injected or held key");
  s.key({ keystroke: "k" }, cx);
  await settled();
  assert.deepEqual(shown(), [{ keys: ["chat.nextSteps"] }], "the person's own key");
});

test("a fix's question is answered by a click only, and not by one landing as it opens", async () => {
  const report = { ...REPORT, at: 1000, checks: [{ id: "install.link", category: "install", status: "fail", message: "no link", fix: { label: "ln", command: "ln -sfn /a /b" } }] };
  for (const layout of ["cockpit", "cockpit-chat"]) {
    const f = await fixture(JSON.stringify(report), null, { layout });
    const { s, cx, settled } = await sheet(f, { tab: "health" });
    const fixes = () => plain(f.acts.filter(([app]) => app === "doctor"));
    s.pick_check("install.link", cx);
    s.key({ keystroke: "enter" }, cx);
    assert.equal(s.asking?.item.id, "install.link", `${layout}: ⏎ on the clicked check asks`);
    for (const e of [{ keystroke: "enter" }, { keystroke: "enter", is_held: true }, { keystroke: "space" }, { keystroke: "y" }]) s.key(e, cx);
    await settled();
    assert.deepEqual(fixes(), [], `${layout}: no key answers it`);
    assert.notEqual(s.asking, null);
    s.confirmed(cx);
    await settled();
    assert.deepEqual(fixes(), [], `${layout}: a click as it opens is not one on it`);
    s.asking.opened = Date.now() - 500;
    s.confirmed(cx);
    await settled();
    assert.deepEqual(fixes().map((a) => a[1]), ["fix"], `${layout}: a click on Run runs it`);
  }
  // Its dialog draws no ⏎ for a key that does nothing.
  const f = await fixture(JSON.stringify(report));
  const { s, cx } = await sheet(f, { tab: "health" });
  s.pick_check("install.link", cx);
  s.ask(s.report.checks[0], cx);
  const dialog = f.flat(s.render(cx)).find((el) => el?.made?.some(([name, id]) => name === "id" && id === "diag-confirm"));
  assert.ok(dialog && f.words(dialog).includes("Run") && !f.words(dialog).includes("⏎"), "no Enter key on Run");
});

test("⏎ on a check needs a move or a click on Health itself: arming one tab arms no other", async () => {
  const report = { ...REPORT, at: 1000, checks: [{ id: "install.link", category: "install", status: "fail", message: "no link", fix: { label: "ln", command: "ln -sfn /a /b" } }] };
  const f = await fixture(JSON.stringify(report));
  const { s, cx, toasts } = await sheet(f);
  // Moved in Options, the person changes tab: Health waits for its own move.
  s.key({ keystroke: "j" }, cx);
  assert.equal(s.pointed, false);
  s.key({ keystroke: "2" }, cx);
  assert.deepEqual([s.tab, s.pointed], ["health", true]);
  s.key({ keystroke: "enter" }, cx);
  assert.equal(s.asking, null, "⏎ asks about nothing never picked");
  assert.equal(toasts.at(-1)[0], "Move to it first (j/k), or click it");
  // A click anywhere in the sheet is not a pick of a check.
  s.acted();
  s.key({ keystroke: "enter" }, cx);
  assert.equal(s.asking, null);
  // Showing every check changes the rows: it waits again.
  s.key({ keystroke: "j" }, cx);
  s.key({ keystroke: "a" }, cx);
  assert.equal(s.pointed, true);
  s.key({ keystroke: "a" }, cx);
  s.pick_check("install.link", cx);
  assert.equal(s.pointed, false);
  s.key({ keystroke: "enter" }, cx);
  assert.equal(s.asking?.item.id, "install.link");
  // O opens the file only after a move or a click, as space does.
  const g = await fixture();
  const o = await sheet(g);
  o.s.key({ keystroke: "o" }, o.cx);
  await o.settled();
  assert.deepEqual(g.executed, []);
  o.s.key({ keystroke: "j" }, o.cx);
  o.s.key({ keystroke: "o" }, o.cx);
  await o.settled();
  assert.equal(g.executed.length, 1);
});

test("a sheet any process opened sets and asks nothing on the person's next keys until they move or click in it", async () => {
  const f = await fixture();
  const { s, cx, toasts, settled, sets } = await sheet(f, { tab: "health" });
  s.key({ keystroke: "1" }, cx);
  assert.equal(s.tab, "options", "1 still shows the tab");
  s.key({ keystroke: "space" }, cx);
  s.key({ keystroke: "enter" }, cx);
  await settled();
  assert.deepEqual(sets(), [], "space and ⏎ set nothing in a sheet the person has not moved in");
  assert.deepEqual(toasts.map((t) => t[0]), ["Move to it first (j/k), or click it", "Move to it first (j/k), or click it"]);
  s.key({ keystroke: "j", is_held: true }, cx);
  s.key({ keystroke: "space" }, cx);
  await settled();
  assert.deepEqual(sets(), [], "a held j arms nothing");
  s.key({ keystroke: "k" }, cx);
  s.key({ keystroke: "space", is_held: true }, cx);
  s.key({ keystroke: "enter", is_held: true }, cx);
  await settled();
  assert.deepEqual(sets(), [], "a held space or ⏎ never sets");
  s.key({ keystroke: "space" }, cx);
  await settled();
  assert.equal(sets().length, 1, "moved in, the person's space sets the option");
  const fixes = await fixture(undefined, [async () => ({ code: 0, stdout: JSON.stringify({ ...REPORT, at: 1000, checks: [{ id: "install.link", category: "install", status: "fail", message: "no link", fix: { label: "ln", command: "ln -sfn /a /b" } }] }), stderr: "" })]);
  const other = new fixes.doctor.default();
  other.init({ tab: "health" }, cx);
  other.toasts = { push: () => {}, render: () => null };
  await new Promise((r) => setTimeout(r, 10));
  await settled();
  other.key({ keystroke: "enter" }, cx);
  other.key({ keystroke: "enter" }, cx);
  assert.equal(other.asking, null, "⏎ asks about no fix before a move or a click");
  other.pointed = false;
  other.key({ keystroke: "enter", is_held: true }, cx);
  assert.equal(other.asking, null, "nor a held ⏎");
  other.key({ keystroke: "enter" }, cx);
  assert.equal(other.asking.item.id, "install.link");
  other.key({ keystroke: "enter" }, cx);
  other.key({ keystroke: "enter", is_held: true }, cx);
  await settled();
  assert.notEqual(other.asking, null, "no ⏎ confirms it");
  assert.deepEqual(plain(fixes.acts.filter(([app]) => app === "doctor")), []);
});

test("a choice, a field and a number change nothing from a bare command, an injected key or a held one", async () => {
  const f = await fixture();
  const { s, cx, settled, sets } = await sheet(f);
  const at = (key) => s.options.findIndex((o) => o.key === key);
  // On the choice, before any move: its arrows, space and ⏎ only say to move first.
  s.optionAt = at("agents.pace");
  for (const keystroke of ["right", "left", "l", "h", "space", "enter"]) s.key({ keystroke }, cx);
  await settled();
  assert.deepEqual(sets(), []);
  // Moved to by the person, an injected or held arrow still turns nothing.
  s.key({ keystroke: "j" }, cx);
  s.key({ keystroke: "k" }, cx);
  assert.equal(s.options[s.optionAt].key, "agents.pace");
  s.key({ keystroke: "right", injected: true }, cx);
  s.key({ keystroke: "right", is_held: true }, cx);
  s.key({ keystroke: "space", injected: true }, cx);
  await settled();
  assert.deepEqual(sets(), []);
  // ⏎ from outside puts the keyboard in no field, so nothing typed from outside lands in one.
  s.optionAt = at("brief.at");
  s.key({ keystroke: "enter", injected: true }, cx);
  s.key({ keystroke: "enter", is_held: true }, cx);
  assert.equal(s.field, null);
  // `o` opens the file from the person's own key only, once they moved.
  s.key({ keystroke: "o", injected: true }, cx);
  s.key({ keystroke: "o", is_held: true }, cx);
  await settled();
  assert.deepEqual(f.executed, []);
  s.key({ keystroke: "o" }, cx);
  await settled();
  assert.deepEqual(plain(f.executed), [["code", ["/Users/me/.config/cockpit/config.json"]]], "the file in the person's editor");
});

test("a choice takes its next value from space or an arrow, and its pill by a click", async () => {
  const f = await fixture();
  const { s, cx, toasts, settled, sets, option } = await sheet(f);
  s.pointed = false;
  s.optionAt = s.options.findIndex((o) => o.key === "agents.pace");
  s.key({ keystroke: "space" }, cx);
  await settled();
  assert.deepEqual(sets(), [{ "agents.pace": "weekdays" }]);
  assert.deepEqual(toasts.at(-1).slice(0, 2), ["Pace of the 7-day limit: Weekdays", "Undo"]);
  s.key({ keystroke: "right" }, cx);
  await settled();
  assert.deepEqual(sets().at(-1), { "agents.pace": "week" }, "past the last value it goes round");
  s.key({ keystroke: "left" }, cx);
  await settled();
  assert.deepEqual(sets().at(-1), { "agents.pace": "weekdays" });
  assert.equal(option("agents.pace").value, "weekdays");
  // The pills: the one picked is the value, a click on the other sets it, a click on the picked one nothing.
  const pills = f.flat(s.control({ ring: "#55f", border: "#333", selected: "#222", background: "#111", foreground: "#eee", mutedFg: "#aaa", accent: "#222" }, option("agents.pace"), s.optionAt));
  const pill = (label) => pills.find((el) => el?.kids?.includes(label));
  const before = sets().length;
  pill("Weekdays").made.find(([name]) => name === "on_click")[1]({}, cx);
  await settled();
  assert.equal(sets().length, before);
  pill("Whole week").made.find(([name]) => name === "on_click")[1]({}, cx);
  await settled();
  assert.deepEqual(sets().at(-1), { "agents.pace": "week" });
  assert.equal(f.doctor.turned({ value: "x", choices: [] }), "x");
});

test("a field is typed in after ⏎ or a click, saved by ⏎ as the hub wrote it, and esc gives the keyboard back", async () => {
  const f = await fixture();
  const { s, cx, toasts, settled, sets, option } = await sheet(f);
  const at = (key) => s.options.findIndex((o) => o.key === key);
  const brief = s.fields.get("brief.at");
  assert.equal(brief.value(), "08:30");
  s.key({ keystroke: "j" }, cx);
  s.optionAt = at("brief.at");
  s.key({ keystroke: "enter" }, cx);
  assert.equal(s.field, "brief.at", "⏎ on its row puts the keyboard in the field");
  // While the field has the keyboard, the sheet's own keys are the field's: no tab changes, nothing is set.
  for (const keystroke of ["2", "j", "space", "r", "u"]) assert.equal(s.key({ keystroke }, cx), false, keystroke);
  assert.equal(s.tab, "options");
  brief.type("9:05", cx);
  assert.equal(s.dirty(option("brief.at")), true);
  brief.submit(cx);
  await settled();
  assert.deepEqual(sets(), [{ "brief.at": "9:05" }]);
  assert.equal(option("brief.at").value, "09:05");
  assert.equal(brief.value(), "09:05", "the field shows the time as the hub wrote it");
  assert.equal(s.dirty(option("brief.at")), false);
  assert.deepEqual(toasts.at(-1).slice(0, 2), ["Morning brief time: 09:05", "Undo"]);
  // ⏎ again with nothing changed saves nothing.
  brief.submit(cx);
  await settled();
  assert.equal(sets().length, 1);
  // Esc drops what was typed and gives the keyboard back to the sheet, which stays open.
  brief.type("midnight", cx);
  const focused = cx.focused;
  assert.equal(s.key({ keystroke: "escape" }, cx), true);
  assert.deepEqual([s.field, brief.value(), f.closed.length, cx.focused], [null, "09:05", 0, focused + 1]);
  assert.equal(s.key({ keystroke: "escape" }, cx), true);
  assert.equal(f.closed.length, 1, "esc on the sheet itself closes it");
  // A number goes as a number; Undo puts the value before back, in the field too.
  const every = s.fields.get("refresh.github");
  every.focus();
  every.type("45", cx);
  every.submit(cx);
  await settled();
  assert.deepEqual(sets().at(-1), { "refresh.github": 45 });
  assert.deepEqual(toasts.at(-1).slice(0, 2), ["GitHub: 45 s", "Undo"]);
  toasts.at(-1)[3].onClick({}, cx);
  await settled();
  assert.deepEqual(sets().at(-1), { "refresh.github": 30 });
  assert.equal(every.value(), "30");
  assert.equal(f.doctor.typedValue({ kind: "number" }, " 45 "), 45);
  assert.equal(f.doctor.typedValue({ kind: "number" }, "soon"), "soon", "what is no number goes as typed, for the hub to refuse in its words");
  assert.equal(f.doctor.typedValue({ kind: "text" }, "45"), "45");
});

test("a save the hub refuses puts the switch back and says why under it, and an invalid value stays in its field", async () => {
  const f = await fixture();
  const { s, cx, toasts, settled, sets, option } = await sheet(f);
  s.pointed = false;
  const k = { ring: "#55f", border: "#333", selected: "#222", background: "#111", foreground: "#eee", mutedFg: "#aaa", accent: "#222", destructive: "#f00", dim: "#888", input: "#444", primary: "#fff", primaryFg: "#000" };
  // A switch: shown on at once, back off when the hub refuses, with its reason.
  f.state.refuse["queue.drain"] = "config.json not written: Permission denied (os error 13)";
  s.optionAt = 0;
  s.key({ keystroke: "space" }, cx);
  assert.equal(option("queue.drain").value, false, "it moves at once");
  await settled();
  assert.equal(option("queue.drain").value, true, "and goes back");
  assert.equal(s.errors["queue.drain"], "config.json not written: Permission denied (os error 13)");
  assert.equal(toasts.length, 0, "no toast says saved");
  assert.ok(f.words(s.optionRow(k, option("queue.drain"), 0)).includes("Not saved: config.json not written: Permission denied (os error 13)"));
  // Trying again clears it once it saves.
  delete f.state.refuse["queue.drain"];
  s.key({ keystroke: "space" }, cx);
  await settled();
  assert.equal(s.errors["queue.drain"], undefined);
  assert.equal(option("queue.drain").value, false);
  // A value its option does not take: the hub's own words under the field, the text left to fix.
  f.state.refuse["brief.at"] = "Morning brief time takes a time of day, such as 08:30";
  const brief = s.fields.get("brief.at");
  brief.focus();
  brief.type("8h30", cx);
  brief.submit(cx);
  await settled();
  assert.deepEqual(sets().at(-1), { "brief.at": "8h30" });
  assert.equal(option("brief.at").value, "08:30", "the value is the one before");
  assert.equal(brief.value(), "8h30", "what was typed stays to be fixed");
  const row = f.words(s.optionRow(k, option("brief.at"), s.optionAt));
  assert.ok(row.includes("Not saved: Morning brief time takes a time of day, such as 08:30"), row.join(" | "));
  assert.ok(row.includes("Save"), "Save stays while the field differs");
  // Typing again takes the reason away.
  brief.type("8:3", cx);
  assert.equal(s.errors["brief.at"], undefined);
  // A hub that does not answer: the same, in its words.
  f.state.refuse["refresh.github"] = "the cockpit hub did not answer";
  const every = s.fields.get("refresh.github");
  every.focus();
  every.type("soon", cx);
  every.submit(cx);
  await settled();
  assert.deepEqual(sets().at(-1), { "refresh.github": "soon" });
  assert.equal(s.errors["refresh.github"], "the cockpit hub did not answer");
});

test("an option config.json sets by hand shows no field and takes no key", async () => {
  const f = await fixture();
  const { s, cx, settled, sets, option } = await sheet(f);
  s.pointed = false;
  s.optionAt = s.options.findIndex((o) => o.key === "meetingBrief.model");
  s.key({ keystroke: "enter" }, cx);
  s.key({ keystroke: "space" }, cx);
  await settled();
  assert.deepEqual([sets(), s.field], [[], null]);
  const drawn = f.words(s.control({ border: "#333", mutedFg: "#aaa", foreground: "#eee" }, option("meetingBrief.model"), s.optionAt));
  assert.ok(drawn.includes("set in config.json"), drawn.join(" | "));
});

test("the options are drawn section by section, each with its default, then what only config.json holds", async () => {
  const f = await fixture();
  const { s } = await sheet(f);
  assert.deepEqual(plain(f.doctor.sectioned(s.view).map((x) => [x.title, x.rows.map(([o]) => o.key)])), [
    ["Queue and CI", ["queue.drain"]], ["Chat", ["chat.nextSteps"]], ["Usage", ["agents.pace"]], ["Refresh", ["refresh.github"]], ["Briefs and recaps", ["brief.at", "meetingBrief.model"]],
  ]);
  const k = { ring: "#55f", border: "#333", selected: "#222", background: "#111", foreground: "#eee", mutedFg: "#aaa", accent: "#222", destructive: "#f00", dim: "#888", input: "#444", primary: "#fff", primaryFg: "#000", muted: "#222" };
  s.rowAt = {};
  const words = s.optionRows(k).flatMap((row) => f.words(row));
  const order = ["QUEUE AND CI", "Send queued prompts on their own", "CHAT", "USAGE", "REFRESH", "BRIEFS AND RECAPS", "IN CONFIG.JSON", "usage.commands"].map((w) => words.indexOf(w));
  assert.ok(order.every((at, i) => at >= 0 && (i === 0 || at > order[i - 1])), `${order}`);
  assert.ok(words.includes("Default: on · since 0.41.0 · queue.drain"));
  assert.ok(words.includes("Default: Whole week · agents.pace"), "an option cockpit always read names no version");
  assert.ok(words.includes("Default: 30 s · refresh.github") && words.includes("Default: 08:30 · brief.at"));
  assert.ok(words.includes("new"), "the one an update brought and not seen yet");
  assert.ok(words.includes("Open config.json in VS Code") && words.includes("A command per agent id that prints its limits."));
  assert.equal(f.doctor.optionWords({ kind: "text", blank: "the conversation's own model", value: "" }), "the conversation's own model");
  assert.equal(f.doctor.optionWords({ kind: "toggle", value: false }), "off");
  // Every option's row is where j/k scroll to.
  assert.deepEqual(Object.keys(s.rowAt).filter((key) => key !== "end").sort(), s.options.map((o) => `option:${o.key}`).sort());
  // The last option brings the file's list into view with it.
  const scrolled = [];
  s.scroll = { scroll_to_item: (row) => scrolled.push(row) };
  s.pick(0);
  s.pick(s.options.length - 1);
  assert.deepEqual(scrolled, [s.rowAt[`option:${s.options[0].key}`], s.rowAt.end]);
});

test("the header says the installed and published versions, and one click checks now", async () => {
  const f = await fixture();
  const path = "/Users/me/.cache/cockpit/update.json";
  const now = 1793611600000 + 4 * 60_000;
  const { versionLine } = await (await fixture(undefined, null, { entry: "./update.js" })).mod;
  assert.deepEqual(plain(versionLine(null)), { text: "Cockpit", state: "never", update: false });
  assert.equal(versionLine({ current: "0.40.4", latest: null, at: null }).text, "Cockpit 0.40.4 · updates not checked yet");
  assert.deepEqual(plain(versionLine({ current: "0.40.4", latest: "0.40.4", at: 1793611600000, newer: false }, { now })), { text: "Cockpit 0.40.4 · up to date · checked 4 min ago", state: "current", update: false });
  assert.deepEqual(plain(versionLine({ current: "0.40.4", latest: "0.41.0", at: 1793611600000, newer: true }, { now })), { text: "Cockpit 0.40.4 · 0.41.0 is out · checked 4 min ago", state: "out", update: true });
  assert.deepEqual(plain(versionLine({ current: "0.40.4", latest: "0.41.0", at: 1793611600000, newer: true, dev: true }, { now })), { text: "Cockpit 0.40.4, your own build · 0.41.0 is published · checked 4 min ago", state: "out", update: false });
  assert.equal(versionLine({ current: "0.40.4", latest: "0.41.0", newer: true }, { checking: true }).text, "Cockpit 0.40.4 · checking for updates…");
  assert.deepEqual(plain(versionLine({ current: "0.40.4", latest: "0.40.4", at: 5, error: "GitHub could not be reached (io: timeout)" })), { text: "Cockpit 0.40.4 · the check failed: GitHub could not be reached (io: timeout)", state: "failed", update: false });

  // Opening the sheet reads the files and asks nobody.
  const { s, cx, settled } = await sheet(f);
  assert.deepEqual(plain(s.v), { current: "0.40.4", latest: null, at: null, dev: false, newer: false, error: null });
  assert.equal(f.acts.filter(([app]) => app === "update").length, 0);
  // One click: checking… while the hub looks, then what it found.
  f.state.published = { latest: "0.41.0" };
  s.checkUpdates(cx);
  assert.equal(s.checking, true);
  s.checkUpdates(cx);
  await settled();
  assert.deepEqual(plain(f.acts.filter(([app]) => app === "update").map((a) => a.slice(0, 3))), [["update", "check", []]], "a second click while it checks asks once");
  assert.deepEqual([s.checking, s.v.latest, s.v.newer, s.v.error], [false, "0.41.0", true, null]);
  assert.equal(versionLine(s.v, { now: s.v.at }).text, "Cockpit 0.40.4 · 0.41.0 is out · checked just now");
  // `u` asks again, from any key: it only reads. Up to date.
  f.state.published = { latest: "0.40.4" };
  s.key({ keystroke: "u", injected: true }, cx);
  await settled();
  assert.equal(versionLine(s.v, { now: s.v.at }).text, "Cockpit 0.40.4 · up to date · checked just now");
  // A check that fails says why, keeps the last version read, and the next one that works clears it.
  f.state.published = { error: "GitHub could not be reached (io: Connection refused)" };
  s.checkUpdates(cx);
  await settled();
  assert.equal(s.v.error, "GitHub could not be reached (io: Connection refused)");
  assert.equal(versionLine(s.v).text, "Cockpit 0.40.4 · the check failed: GitHub could not be reached (io: Connection refused)");
  assert.equal(JSON.parse(f.state.files[path]).latest, "0.40.4");
  f.state.published = { latest: "0.40.4" };
  s.checkUpdates(cx);
  await settled();
  assert.equal(s.v.error, null);
  // Update opens the update dialog as the person's own click, and installs nothing by itself.
  f.state.published = { latest: "0.41.0" };
  s.checkUpdates(cx);
  await settled();
  const drawn = f.flat(s.render(cx));
  const button = (label) => drawn.find((el) => el?.kids?.includes(label));
  assert.ok(f.words(s.render(cx)).some((w) => w.startsWith("Cockpit 0.40.4 · 0.41.0 is out · checked ")), "the header's line");
  assert.ok(button("Check for updates"), "the check stays one click away");
  button("Update to 0.41.0").made.find(([name]) => name === "on_click")[1]({}, cx);
  assert.deepEqual(plain(f.opened.at(-1).props), { app: "update", after: false, person: true });
  assert.equal(f.acts.filter(([app]) => app !== "options" && app !== "update").length, 0, "nothing installs from the sheet");
});

test("with the hub down the sheet still opens, says why in the error's own words, and offers Start it", async () => {
  const f = await fixture();
  f.state.hub = "error: cockpit hub: address already in use (port 7410)";
  const { s, cx, toasts, settled } = await sheet(f);
  assert.equal(s.options, null, "no option was read");
  const k = { ring: "#55f", border: "#333", selected: "#222", background: "#111", foreground: "#eee", mutedFg: "#aaa", accent: "#222", destructive: "#f00", dim: "#888", input: "#444", primary: "#fff", primaryFg: "#000", muted: "#222" };
  const options = s.optionRows(k).flatMap((row) => f.words(row));
  assert.ok(options.includes("The cockpit hub isn't running: this window cannot reach it"), options.join(" | "));
  assert.ok(options.includes("Why: cockpit hub: address already in use (port 7410)"));
  assert.ok(options.includes("Start it") && options.includes("Open config.json in VS Code"), "the file still opens without the hub");
  // Health says the same above its checks, which run without the hub, and counts it.
  s.show("health");
  const health = s.healthRows(k, cx).flatMap((row) => f.words(row));
  assert.ok(health.includes("Why: cockpit hub: address already in use (port 7410)") && health.includes("gh not signed in"), health.join(" | "));
  const tabs = f.flat(s.render(cx)).find((el) => el?.kids?.includes("Health"));
  assert.ok(f.words(tabs).includes("3"), "two checks to look at and the hub");
  // Start it is the person's click: an explicit start, and why when it fails.
  f.state.startFails = "cockpit hub: address already in use (port 7410)";
  const row = f.flat(s.hubRow(k, "health-hub"));
  row.find((el) => el?.kids?.includes("Start it")).made.find(([name]) => name === "on_click")[1]({}, cx);
  assert.equal(s.starting, true);
  await settled();
  assert.deepEqual(plain(f.started.map((o) => o.explicit)), [true]);
  assert.deepEqual(toasts.at(-1).slice(0, 3), ["The hub did not start", undefined, "cockpit hub: address already in use (port 7410)"]);
  // A hub stopped on purpose says so; one that answers again brings the options, and the row goes.
  f.link("error: the cockpit hub was stopped on purpose");
  assert.ok(f.words(s.hubRow(k, "x")).includes("It was stopped with cockpit hub stop, and no window starts it again until you do."));
  f.state.failure = { message: "connect ECONNREFUSED 127.0.0.1:7410", at: 1 };
  f.link("connecting");
  const trying = f.words(s.hubRow(k, "x"));
  assert.ok(trying.includes("Reaching the cockpit hub…") && trying.includes("The last try failed: connect ECONNREFUSED 127.0.0.1:7410") && !trying.includes("Start it"));
  f.link("live");
  assert.equal(s.hubRow(k, "x"), null);
  assert.equal(s.options.length, OPTIONS.length);
  // Closing the sheet stops following the hub.
  s.close();
  assert.equal(f.state.stops, 1);
});

test("a read the window never answers is given up: the sheet says so and Check again runs the checks", async () => {
  const never = new Promise(() => {});
  const f = await fixture(undefined, [async () => never, async () => never, async () => ({ code: 0, stdout: JSON.stringify(REPORT), stderr: "" })]);
  f.state.gaveUp = 5;
  const tasks = [];
  const cx = { spawn: (fn) => tasks.push(fn(cx)), notify: () => {}, focus_handle: () => ({ focus() {} }), write_to_clipboard() {} };
  const s = new f.doctor.default();
  s.init({ tab: "health" }, cx);
  await Promise.all(tasks);
  assert.deepEqual([s.running, s.report, s.error], [false, null, "cockpit doctor did not answer"]);
  assert.equal(s.options.length, OPTIONS.length, "the options came all the same");
  s.key({ keystroke: "r" }, cx);
  await Promise.all(tasks);
  assert.deepEqual([s.running, s.report?.summary?.fail, s.error], [false, 1, null]);
});

test("Open in editor reads the editor from the resolved copy, and never takes a switch for one", async () => {
  const f = await fixture(undefined, null, { entry: "./editor.js" });
  const raw = "/Users/me/.config/cockpit/config.json", copy = "/Users/me/.cache/cockpit/config.json";
  assert.equal(await f.mod.editor(), "code", "nothing written: VS Code");
  f.state.files[raw] = '{\n  // mine\n  "editor": "cursor"\n}\n';
  assert.equal(await f.mod.editor(), "code", "a file with comments is not read as JSON here");
  f.state.files[copy] = JSON.stringify({ editor: "cursor" });
  assert.equal(await f.mod.editor(), "cursor", "its resolved copy is");
  f.state.files[copy] = JSON.stringify({ editor: "--wait" });
  f.state.files[raw] = JSON.stringify({ editor: "zed" });
  assert.equal(await f.mod.editor(), "zed", "a switch is no editor");
});

test("the versions read the later of the six-hourly look and one asked for, and that look's error", async () => {
  const f = await fixture(undefined, null, { entry: "./update.js" });
  const path = "/Users/me/.cache/cockpit/update.json";
  f.state.files[path] = JSON.stringify({ at: 100, looked_at: 5000, latest: "0.41.0", error: { message: "GitHub could not be reached", at: 5000 } });
  assert.deepEqual(plain(await f.mod.versions()), { current: "0.40.4", latest: "0.41.0", at: 5000, dev: false, newer: true, error: "GitHub could not be reached" });
  f.state.files[path] = JSON.stringify({ at: 9000, looked_at: 5000, latest: "0.41.0", error: { message: "old", at: 5000 } });
  assert.deepEqual([(await f.mod.versions()).at, (await f.mod.versions()).error], [9000, null], "a later look that worked clears it");
});

test("a press on a toast stays the toast's: its Undo does not also press what is beneath", async () => {
  const f = await fixture(undefined, null, { entry: "./ui.js" });
  const toaster = new f.mod.Toaster();
  let undone = 0;
  const cx = { timer: { after: () => ({ cancel() {} }) }, notify() {} };
  toaster.push(cx, "Name agents after their work: off", { action: { label: "Undo", onClick: () => undone++ } });
  const drawn = f.flat(toaster.render());
  const toast = drawn.find((el) => el?.made?.some(([name, id]) => name === "id" && String(id).startsWith("toast-") && !String(id).endsWith("-action")));
  const stops = [];
  const press = { stop_propagation: () => stops.push("down") };
  toast.made.find(([name, button]) => name === "on_mouse_down" && button === "left")[2]({}, press);
  assert.deepEqual(stops, ["down"], "the press stops at the toast");
  const undo = drawn.find((el) => el?.kids?.includes("Undo"));
  const click = { stop_propagation: () => stops.push("click"), notify() {} };
  undo.made.find(([name]) => name === "on_click")[1]({}, click);
  assert.deepEqual([undone, stops], [1, ["down", "click"]]);
});

test("the status bar's hub item says why the window last failed to reach the hub", async () => {
  const f = await fixture(undefined, null, { entry: "./status.js" });
  assert.equal(f.mod.hubTip("error: cockpit hub: address already in use (port 7410)", true),
    "The cockpit hub isn't running: cockpit hub: address already in use (port 7410). Click to start it; Cockpit settings, under Health, says more");
  assert.equal(f.mod.hubTip("error: connect ECONNREFUSED 127.0.0.1:7410", false), "The cockpit hub isn't running: connect ECONNREFUSED 127.0.0.1:7410");
  assert.equal(f.mod.hubTip("connecting"), "Reaching the cockpit hub…");
  f.state.failure = { message: "hub.json did not answer", at: 1 };
  assert.equal(f.mod.hubTip("connecting"), "Reaching the cockpit hub… The last try failed: hub.json did not answer");
});

test("Fix all asks once with a line per fix, counts only what the hub can run, and runs them in one act", async () => {
  const report = {
    ...REPORT, at: 3000,
    checks: [
      ...REPORT.checks,
      { id: "claude.hooks", category: "claude", status: "fail", message: "hooks missing", fix: { label: "/plugin install cockpit" } },
      { id: "spaces.pinned", category: "spaces", status: "warn", message: "no pinned spaces", fix: null },
    ],
  };
  const f = await fixture(JSON.stringify(report));
  const { items, left } = f.doctor.fixAll(report);
  assert.deepEqual([plain(items.map((c) => c.id)), left], [["tools.gh", "sources.cache.inbox"], 2], "a fix said in words, or none, is the person's");
  assert.deepEqual(plain(f.doctor.fixAllLines(items)), [{ command: "gh auth login", text: "runs in a tab of the actions space" }, { text: "Reads every source again now." }]);
  assert.equal(f.doctor.fixAll({ checks: [REPORT.checks[0], REPORT.checks[2]] }).items.length, 1);
  for (const layout of ["cockpit", "cockpit-chat"]) {
    const f = await fixture(JSON.stringify(report), null, { layout });
    const { s, cx, toasts, settled } = await sheet(f, { tab: "health" });
    const drawn = () => f.flat(s.render(cx));
    const button = drawn().find((el) => el?.kids?.includes("Fix all (2)"));
    assert.ok(button, `${layout}: Fix all counts the two the hub can run`);
    const tip = button.made.find(([name]) => name === "tooltip")[1];
    assert.equal(tip, layout === "cockpit" ? "Run the 2 fixes cockpit can run, after one confirm" : "Run the 2 fixes cockpit can run, after one confirm  F", `${layout}: its key`);
    button.made.find(([name]) => name === "on_click")[1]({}, cx);
    assert.equal(s.asking?.items.length, 2);
    const dialog = drawn().find((el) => el?.made?.some(([name, id]) => name === "id" && id === "diag-confirm"));
    const words = f.words(dialog);
    for (const w of ["Run these 2 fixes?", "gh auth login", "runs in a tab of the actions space", "Reads every source again now.", "2 more need you: see below", "Fix all"]) assert.ok(words.includes(w), `${layout}: ${w} in ${words.join(" | ")}`);
    assert.ok(!words.includes("⏎"), `${layout}: no key answers it`);
    const fixes = () => plain(f.acts.filter(([app]) => app === "doctor"));
    for (const e of [{ keystroke: "enter" }, { keystroke: "space" }, { keystroke: "f" }, { keystroke: "y" }]) s.key(e, cx);
    s.confirmed(cx);
    await settled();
    assert.deepEqual(fixes(), [], `${layout}: neither a key nor a click as it opens runs it`);
    s.asking.opened = Date.now() - 500;
    s.confirmed(cx);
    await settled();
    assert.deepEqual(fixes(), [["doctor", "fix_all", [3000, [["tools.gh", "gh auth login"], ["sources.cache.inbox", "sources refresh []"]]]]], `${layout}: one act names the report and each fix as shown`);
    assert.deepEqual(toasts.at(-1).slice(0, 3), ["Fixed 2", undefined, "running gh auth login in the actions space\nrefresh requested, it lands within a minute"]);
    assert.equal(s.fixing, "A command runs in its tab: Check again once done", `${layout}: a command runs on, so the sheet waits`);
  }
  // `F` asks too, from the person's own key once they moved in Health; the hub refusing says why and runs nothing.
  const g = await fixture(JSON.stringify(report));
  const { s, cx, toasts, settled } = await sheet(g, { tab: "health" });
  s.key({ keystroke: "f", injected: true }, cx);
  s.key({ keystroke: "f" }, cx);
  assert.equal(s.asking, null);
  assert.equal(toasts.at(-1)[0], "Move to it first (j/k), or click it");
  s.key({ keystroke: "j" }, cx);
  s.key({ keystroke: "f", is_held: true }, cx);
  assert.equal(s.asking, null, "nor a held F");
  s.key({ keystroke: "f" }, cx);
  assert.equal(s.asking?.items.length, 2);
  g.state.fixAll = { note: "", error: "The checks changed: look again" };
  s.asking.opened = 0;
  s.confirmed(cx);
  await settled();
  assert.deepEqual(toasts.at(-1).slice(0, 1), ["The checks changed: look again"]);
  assert.equal(s.fixing, null);
  // One fix the hub can run is its own row's Fix: no Fix all, and F asks nothing.
  const one = await fixture(JSON.stringify({ ...report, checks: report.checks.filter((c) => c.id !== "sources.cache.inbox") }));
  const o = await sheet(one, { tab: "health" });
  assert.ok(!one.words(o.s.render(o.cx)).some((w) => String(w).startsWith("Fix all")));
  o.s.key({ keystroke: "j" }, o.cx);
  o.s.key({ keystroke: "f" }, o.cx);
  assert.equal(o.s.asking, null);
});

test("the Agents tab says in chat for a session Cockpit Chat runs itself, and its tooltip says what that means", async () => {
  const report = { ...REPORT, agents: [{ id: "claude", name: "Claude Code", icon: "claude", installed: true, default: true, path: "/opt/acme/bin/claude", can: { start: true, resume: true, fork: true, headless: true, driven: true }, tools: "plugin" }] };
  const f = await fixture(JSON.stringify(report));
  const { s } = await sheet(f, { tab: "agents" });
  const words = f.words(s.agentsEl({ foreground: "#eee", mutedFg: "#aaa", dim: "#888", border: "#333" }));
  assert.ok(words.includes("in chat") && words.includes("Cockpit Chat runs it itself, without a terminal"), words.join(" | "));
  assert.ok(!words.includes("driven"));
});

test("the Layout tab shows both layouts in the window's theme, In use on the one shown, and switches to the other on a click", async () => {
  const dark = { background: "#101010", foreground: "#eee", mutedFg: "#aaa", dim: "#888", border: "#333", ring: "#55f" };
  const light = { ...dark, background: "#fafaf8", foreground: "#111" };
  const f = await fixture();
  f.doctor.openSettings({ tab: "layout" });
  assert.equal(f.opened.at(-1).props.tab, "layout", "cockpit.settings:layout opens it");
  const { s } = await sheet(f, { tab: "layout" });
  assert.equal(s.tab, "layout");
  const cards = (k) => f.flat(s.layoutEl(k)).filter((el) => el?.made?.some(([name, id]) => name === "id" && /^layout-cockpit/.test(id)));
  const pictures = (k) => f.flat(s.layoutEl(k)).filter((el) => el?.kind === "image").map((el) => el.path);
  assert.deepEqual(pictures(dark), ["mods/cockpit/layouts/cockpit-dark.png", "mods/cockpit/layouts/cockpit-chat-dark.png"]);
  assert.deepEqual(pictures(light), ["mods/cockpit/layouts/cockpit-light.png", "mods/cockpit/layouts/cockpit-chat-light.png"]);
  assert.equal(cards(dark).length, 2);
  const words = f.words(s.layoutEl(dark));
  for (const w of ["Cockpit", "Cockpit Chat", "In use", "Use this layout", ...f.doctor.LAYOUT_CHOICES.flatMap((l) => l.points)]) assert.ok(words.includes(w), w);
  assert.equal(words.filter((w) => w === "In use").length, 1, "only the layout shown is in use");
  // The button is on the other layout's card, and its click switches the window as the update dialog's Try does.
  const button = f.flat(s.layoutEl(dark)).find((el) => el?.made?.some(([name, id]) => name === "id" && id === "layout-use-cockpit-chat"));
  assert.ok(button, "Use this layout sits on Cockpit Chat's card while Cockpit is shown");
  assert.ok(!f.flat(s.layoutEl(dark)).some((el) => el?.made?.some(([name, id]) => name === "id" && id === "layout-use-cockpit")));
  button.made.find(([name]) => name === "on_click")[1]({}, {});
  assert.deepEqual(plain(f.commands), [["layout.switch", { name: "cockpit-chat" }]]);
  assert.deepEqual(plain(f.closed), ["o1"], "the sheet closes first");
  assert.match(f.notices[0].title, /This is Cockpit Chat/);
  // The layout shown changes nothing, and a copy of a layout counts as that layout.
  f.shared.set("layout", "cockpit-chat-copy");
  s.useLayout("cockpit-chat");
  s.useLayout("made-up");
  assert.equal(f.commands.length, 1);
  assert.ok(f.flat(s.layoutEl(dark)).some((el) => el?.made?.some(([name, id]) => name === "id" && id === "layout-use-cockpit")));
  // Narrow, the cards stack and their pictures take the width.
  s.width = 400;
  const narrow = f.flat(s.layoutEl(dark)).find((el) => el?.kind === "image");
  assert.equal(narrow.made.find(([name]) => name === "w")[1], 360 - 26);
});

test("the Layout tab is the fourth: its number key shows it, and Esc still closes", async () => {
  const f = await fixture();
  const { s, cx } = await sheet(f);
  s.key({ keystroke: "4" }, cx);
  assert.equal(s.tab, "layout");
  s.key({ keystroke: "1" }, cx);
  assert.equal(s.tab, "options");
  assert.equal(f.doctor.isDark("#101010"), true);
  assert.equal(f.doctor.isDark("#fafaf8ff"), false);
  assert.equal(f.doctor.isDark(undefined), true);
});
