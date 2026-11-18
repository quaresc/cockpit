import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import net from "node:net";
import { once } from "node:events";
import { createContext, SourceTextModule, SyntheticModule } from "node:vm";
import { test } from "node:test";

async function fixture(t, remote = true, globals = {}, host = "127.0.0.1") {
  const calls = [], clients = new Set(), messages = [];
  const behavior = { token: "test-token", silent: false, bytewise: false, hangs: 0, tickets: [{ id: "ENG-401" }] };
  const server = net.createServer(socket => {
    clients.add(socket);
    socket.on("close", () => clients.delete(socket));
    let pending = "";
    socket.on("data", data => {
      pending += data;
      while (pending.includes("\n")) {
        const end = pending.indexOf("\n");
        const request = JSON.parse(pending.slice(0, end));
        pending = pending.slice(end + 1);
        messages.push(request);
        if (behavior.silent) continue;
        if (request.token !== behavior.token) socket.end('{"error":"a request on TCP needs hub.json\'s token"}\n');
        else if (request.op === "subscribe") socket.write(JSON.stringify({ view: typeof request.views[0] === "string" ? request.views[0] : request.views[0].name, value: { tickets: behavior.tickets } }) + "\n");
        else socket.write(JSON.stringify(request.op === "ping" ? { ok: true } : { value: { tickets: behavior.tickets } }) + "\n");
      }
    });
  });
  server.listen(0, host);
  await once(server, "listening");
  t.after(() => { for (const socket of clients) socket.destroy(); server.close(); });
  const info = { host: remote ? "10.99.99.99" : host, port: server.address().port, token: "test-token" };
  const install = { bin: "/runner/cockpit", home: "/runner", cache: "/runner/cache", ...(remote ? { machine: "runner", host: "127.0.0.1" } : {}) };
  const context = createContext({ console, ...globals });
  const imports = {
    herdr: {
      appHome: () => "C:/Herdr",
      window: () => ({ platform: "windows", active: true }),
      run: (...args) => calls.push(["run", ...args]),
      readTextFile: async (path, options) => {
        calls.push(["read", path, options]);
        if (path === "C:/Herdr/cockpit.json") return JSON.stringify(install);
        if (path.endsWith("/hub.stopped")) return behavior.stopped ? "" : null;
        if (behavior.noInfo) return null;
        assert.equal(options?.on, install.machine);
        return JSON.stringify(info);
      },
      exec: async (...args) => {
        calls.push(["exec", ...args]);
        if (behavior.startHangs && args[1]?.includes?.("start")) return new Promise(() => {});
        return behavior.exec ?? { code: 0, stdout: JSON.stringify({ ...info, token: behavior.token }), stderr: "" }; },
    },
    net: { default: { connect: async (host, port) => {
      if (behavior.hangs > 0) {
        behavior.hangs -= 1;
        calls.push(["hang", host, port]);
        return new Promise(() => {});
      }
      calls.push(["connect", host, port]);
      const socket = net.createConnection({ host, port });
      await once(socket, "connect");
      const chunks = (async function* () {
        for await (const chunk of socket) {
          if (behavior.bytewise) for (const byte of chunk) yield Uint8Array.of(byte);
          else yield chunk;
        }
      })();
      return {
        write: data => new Promise((resolve, reject) => socket.write(data, error => error ? reject(error) : resolve())),
        read: async () => { const chunk = await chunks.next(); return chunk.done ? null : chunk.value; },
        close: () => socket.destroy(),
      };
    } } },
  };
  const module = new SourceTextModule(await readFile(new URL("../mods/cockpit/hub.js", import.meta.url), "utf8"), { context });
  await module.link(name => new SyntheticModule(Object.keys(imports[name]), function () {
    for (const [key, value] of Object.entries(imports[name])) this.setExport(key, value);
  }, { context }));
  await module.evaluate();
  return { hub: module.namespace, calls, info, install, messages, clients, behavior, server, host, context, imports };
}

test("a desktop reads the runner's discovery file and authenticates before requesting its inbox", async t => {
  const f = await fixture(t);
  assert.equal((await f.hub.view("inbox")).tickets[0].id, "ENG-401");
  assert.equal(f.calls.find(call => call[0] === "connect")[1], "127.0.0.1");
  assert.deepEqual(f.messages.map(message => message.op), ["ping", "view"]);
});

for (const [name, globals] of [["TextDecoder", { TextDecoder }], ["the percent fallback", {}]]) test(`inbox reads and live updates preserve Unicode split across socket reads with ${name}`, async t => {
  const f = await fixture(t, true, globals), r = runtime(t);
  f.behavior.bytewise = true;
  f.behavior.tickets = [{ id: "ENG-412", title: "Café € 漢字 🚀" }];
  assert.equal((await f.hub.view("inbox")).tickets[0].title, "Café € 漢字 🚀");
  const sub = f.hub.subscribe(r.cx, { views: ["inbox"] }, r.message);
  t.after(() => sub.stop());
  await until(() => r.received.length === 1);
  assert.equal(r.received[0].value.tickets[0].title, "Café € 漢字 🚀");
  sub.stop();
  await Promise.all(r.tasks);
});

test("remote actions and commands cannot operate the runner's desktop", async t => {
  const f = await fixture(t);
  await assert.rejects(f.hub.act("inbox", "review", [{}]), /read-only/);
  await assert.rejects(f.hub.cockpit(["resume", "all"]), /read-only/);
  await assert.rejects(f.hub.act("review", "open_reviewr", ["/runner/acme/api"]), /read-only/);
  await assert.rejects(f.hub.cockpit(["hub", "act", "inbox", "review", "[]"]), /read-only/);
  await assert.rejects(f.hub.cockpit(["hub", "act", "review", "send"]), /read-only/);
  await assert.rejects(f.hub.go({ app: "session", key: "past" }), /read-only/);
  assert.equal(f.messages.length, 0);
  assert.equal(f.calls.filter(call => call[0] === "exec").length, 0);
});

test("remote review actions run as a cockpit command on the runner, through its machine connection", async t => {
  const f = await fixture(t);
  const review = { scope: "branch", repo: "/runner/acme/api", comments: [{ path: "src/a.rs", line: 12, text: `say "hi"; $(rm -rf ~) 'x'\nnext` }] };
  await f.hub.act("review", "walkthrough", ["/runner/acme/api", "branch", "haiku"]);
  await f.hub.act("review", "stop_walkthrough", ["/runner/acme/api", "branch"]);
  await f.hub.act("review", "send", ["runner/t7", review]);
  const runs = f.calls.filter(call => call[0] === "exec").map(([, , args, options]) => [JSON.parse(JSON.stringify(args.slice(args.indexOf("/runner/cockpit")))), options?.on]);
  assert.deepEqual(runs, [
    [["/runner/cockpit", "hub", "act", "review", "walkthrough", '["/runner/acme/api","branch","haiku"]'], "runner"],
    [["/runner/cockpit", "hub", "act", "review", "stop_walkthrough", '["/runner/acme/api","branch"]'], "runner"],
    [["/runner/cockpit", "hub", "act", "review", "send", JSON.stringify(["t7", review])], "runner"],
  ]);
  assert.equal(f.messages.length, 0, "no write travels over TCP");
});

test("remote runbook edits and runs go to the runner with their armed key, and nothing else of the runbook does", async t => {
  const f = await fixture(t);
  const step = { n: "2", step: "Deploy", where: "prod", status: "to do", command: "make deploy" };
  await f.hub.act("runbook", "done", [[["eng-412", "2"]]]);
  await f.hub.act("runbook", "set", ["eng-412", "2", "command", "make ship"]);
  await f.hub.act("runbook", "add", ["eng-412", null, "Smoke", "prod", "make smoke"]);
  await f.hub.act("runbook", "undo", []);
  await f.hub.request("act", { app: "runbook", action: "run_step", args: [{ $book: "eng-412" }, step], armed: "run-eng-412-2" });
  const runs = f.calls.filter(call => call[0] === "exec").map(([, , args]) => JSON.parse(JSON.stringify(args.slice(args.indexOf("/runner/cockpit") + 1))));
  assert.deepEqual(runs, [
    ["hub", "act", "runbook", "done", '[[["eng-412","2"]]]'],
    ["hub", "act", "runbook", "set", '["eng-412","2","command","make ship"]'],
    ["hub", "act", "runbook", "add", '["eng-412",null,"Smoke","prod","make smoke"]'],
    ["hub", "act", "runbook", "undo", "[]"],
    ["hub", "act", "runbook", "run_step", JSON.stringify([{ $book: "eng-412" }, step]), "run-eng-412-2"],
  ]);
  assert.equal(f.messages.length, 0, "no write travels over TCP");
  for (const action of ["delete", "update", "talk", "focus_session", "open_pr"]) await assert.rejects(f.hub.act("runbook", action, [{ $book: "eng-412" }]), /read-only/);
  await assert.rejects(f.hub.cockpit(["hub", "act", "runbook", "delete", "[]"]), /read-only/);
  assert.equal(f.calls.filter(call => call[0] === "exec").length, runs.length);
});

test("Copy on a remote desk writes the clipboard and never calls the hub", async t => {
  const f = await fixture(t);
  const source = (await readFile(new URL("../mods/cockpit/runbook.js", import.meta.url), "utf8"));
  const stubs = new Proxy({}, { get: () => () => new Proxy(function () {}, { get: () => () => ({}) }) });
  const toasts = [];
  const modules = {
    "./hub.js": f.hub,
    "./ui.js": { ...Object.fromEntries([...source.matchAll(/import \{([^}]*)\} from "\.\/ui\.js"/g)].flatMap(m => m[1].split(",")).map(n => n.trim()).filter(Boolean).map(n => [n, stubs[n]])), Toaster: class { push(_cx, text) { toasts.push(text); } } },
    "gpui-kit": { div: stubs.div },
    "gpui-base": { h_flex: stubs.h_flex, v_flex: stubs.v_flex, InputState: stubs.InputState },
    herdr: { props: () => ({}), focused: () => null, panes: () => [], shared: () => ({}), settings: () => ({}) },
  };
  // Every other module it imports by name is stubbed whole: the copy needs none of them.
  for (const m of source.matchAll(/import\s*\{([^}]*)\}\s*from\s*"(\.\/[^"]+)"/g)) {
    modules[m[2]] ??= Object.fromEntries(m[1].split(",").map(n => n.trim().split(/\s+as\s+/)[0]).filter(Boolean).map(n => [n, stubs[n]]));
  }
  const module = new SourceTextModule(source, { context: f.context });
  await module.link(name => new SyntheticModule(Object.keys(modules[name]), function () {
    for (const [key, value] of Object.entries(modules[name])) this.setExport(key, value);
  }, { context: f.context }));
  await module.evaluate();
  const runbook = Object.create(module.namespace.default.prototype);
  runbook.toasts = new modules["./ui.js"].Toaster();
  const written = [];
  runbook.copy({ write_to_clipboard: text => written.push(text) }, ["make deploy", "", "make smoke"]);
  assert.deepEqual(written, ["make deploy\nmake smoke"]);
  assert.deepEqual(toasts, ["Copied 2 commands"]);
  assert.equal(f.messages.length, 0);
  assert.equal(f.calls.filter(call => call[0] === "exec").length, 0);
});

test("a remote review goes only to one of the runner's agents", async t => {
  const f = await fixture(t);
  for (const terminal of ["t7", "app:t7", "laptop/t7", "runner/laptop/t7", "runner/", ""]) await assert.rejects(f.hub.act("review", "send", [terminal, { comments: [] }]), /runner's agents/);
  assert.equal(f.calls.filter(call => call[0] === "exec").length, 0);
});

test("the runner's refusal of a review action comes back as its error", async t => {
  const f = await fixture(t);
  f.behavior.exec = { code: 1, stdout: "", stderr: "walkthrough: no changes to walk through\n" };
  await assert.rejects(f.hub.act("review", "walkthrough", ["/runner/acme/api", "uncommitted", "haiku"]), /no changes to walk through/);
});

test("links and navigation stay on the desktop", async t => {
  const f = await fixture(t);
  await f.hub.act("inbox", "open_url", ["https://github.com/acme/api/pull/4"]);
  await f.hub.go({ app: "inbox", key: "https://github.com/acme/api/pull/4" });
  const command = f.calls.find(call => call[0] === "exec");
  assert.equal(command[1], "cmd");
  assert.equal(command[3]?.on, undefined);
  assert.deepEqual(f.calls.find(call => call[0] === "run"), ["run", "cockpit.go:inbox"]);
  assert.equal(f.messages.length, 0);
});

test("runner startup never receives the desktop's app home", async t => {
  const f = await fixture(t);
  f.install.env = { HERDR_APP_HOME: "C:/wrong", COCKPIT_FRONTEND: "tui" };
  await f.hub.cockpit(["hub", "start"]);
  const [, command, args, options] = f.calls.find(call => call[0] === "exec");
  assert.equal(command, "env");
  assert.equal(options.on, "runner");
  assert.ok(args.includes("COCKPIT_HUB_LISTEN=127.0.0.1"));
  assert.ok(!args.some(arg => arg.startsWith("HERDR_APP_HOME=")));
  assert.ok(args.includes("COCKPIT_FRONTEND=app"));
});

test("local installations retain their actions and localhost connection", async t => {
  const f = await fixture(t, false);
  await f.hub.act("inbox", "set_muted", ["api", true]);
  assert.equal(f.messages.at(-1).action, "set_muted");
  assert.equal(f.calls.find(call => call[0] === "connect")[1], "127.0.0.1");
});

test("the runner's own desktop can use its advertised private interface", async t => {
  const f = await fixture(t, false, {}, "::1");
  assert.equal((await f.hub.view("inbox")).tickets[0].id, "ENG-401");
  assert.equal(f.calls.find(call => call[0] === "connect")[1], "::1", "hub.json's interface, not the loopback default");
});

function runtime(t) {
  const timers = new Set(), tasks = [], received = [], states = [];
  const cx = {
    spawn: work => { tasks.push(work(cx)); },
    timer: { after: (ms, callback) => {
      const timer = { ms, callback, cancel: () => { clearTimeout(timer.id); timers.delete(timer); } };
      timers.add(timer);
      if (ms < 10000) timer.id = setTimeout(() => { timer.cancel(); callback(); }, 1);
      return timer;
    } },
  };
  t.after(() => { for (const timer of timers) timer.cancel(); });
  return { cx, timers, tasks, received, states, message: m => received.push(m), state: s => states.push(s) };
}

async function until(predicate) {
  const started = Date.now();
  while (!predicate()) {
    assert.ok(Date.now() - started < 2000, "condition did not become true");
    await new Promise(resolve => setTimeout(resolve, 1));
  }
}

test("an expired discovery token is refreshed before sending an operation", async t => {
  const f = await fixture(t);
  f.behavior.token = "rotated-test-token";
  assert.equal((await f.hub.view("inbox")).tickets[0].id, "ENG-401");
  assert.deepEqual(f.messages.map(message => message.op), ["ping", "ping", "view"]);
  assert.equal(f.messages.at(-1).token, "rotated-test-token");
});

test("subscriptions deliver changes, reconnect after a silent link and stop cleanly", async t => {
  const f = await fixture(t), r = runtime(t);
  const subscription = f.hub.subscribe(r.cx, { views: ["inbox"], focus: "inbox" }, r.message, r.state);
  t.after(() => subscription.stop());
  await until(() => r.received.length === 1);
  for (const socket of f.clients) socket.write('{"view":"inbox","value":{"tickets":[{"id":"ENG-402"}]}}\n');
  await until(() => r.received.length === 2);
  assert.equal(r.received[1].value.tickets[0].id, "ENG-402");
  const watchdog = [...r.timers].find(timer => timer.ms === 65000);
  assert.ok(watchdog, "a silent connection must have a heartbeat deadline");
  watchdog.callback();
  await until(() => r.received.length === 3);
  assert.ok(r.states.some(state => state.includes("timed out")));
  assert.equal(r.states.at(-1), "live");
  await f.hub.go({ app: "inbox", key: "ENG-402" });
  assert.equal(r.received.at(-1).target.key, "ENG-402");
  subscription.stop();
  await Promise.all(r.tasks);
  await until(() => f.clients.size === 0);
});

test("subscriptions rediscover the port and token after a hub restart", async t => {
  const f = await fixture(t), r = runtime(t);
  const subscription = f.hub.subscribe(r.cx, { views: ["inbox"] }, r.message, r.state);
  t.after(() => subscription.stop());
  await until(() => r.received.length === 1);
  const closed = once(f.server, "close");
  f.server.close();
  for (const socket of f.clients) socket.destroy();
  await closed;
  f.server.listen(0, f.host);
  await once(f.server, "listening");
  f.info.port = f.server.address().port;
  f.info.token = f.behavior.token = "restarted-test-token";
  await until(() => r.received.length === 2);
  assert.equal(f.messages.at(-1).token, "restarted-test-token");
  subscription.stop();
  await Promise.all(r.tasks);
});

test("navigation to an unopened view waits for its rows before focusing", async t => {
  const f = await fixture(t), r = runtime(t);
  await f.hub.go({ app: "inbox", key: "ENG-401" });
  const subscription = f.hub.subscribe(r.cx, { views: ["inbox"], focus: "inbox" }, r.message, r.state);
  t.after(() => subscription.stop());
  await until(() => r.received.length === 2);
  assert.equal(r.received[0].view, "inbox");
  assert.equal(r.received[1].target.key, "ENG-401");
  subscription.stop();
  await Promise.all(r.tasks);
});

test("stopping during authentication closes the socket without starting the runner", async t => {
  const f = await fixture(t), r = runtime(t);
  f.behavior.silent = true;
  const sub = f.hub.subscribe(r.cx, { views: ["inbox"] }, r.message, r.state);
  t.after(() => sub.stop());
  await until(() => f.messages.length === 1);
  sub.stop();
  await until(() => f.clients.size === 0);
  await Promise.all(r.tasks);
  assert.equal(f.calls.filter(c => c[0] === "exec").length, 0);
});

test("navigation is consumed by its view once, not by a sidebar subscription", async t => {
  const f = await fixture(t), r = runtime(t);
  const sidebar = f.hub.subscribe(r.cx, { views: ["inbox"], notify: true }, () => {});
  t.after(() => sidebar.stop());
  await until(() => f.messages.some(m => m.op === "subscribe"));
  await f.hub.go({ app: "inbox", key: "ENG-401" });
  let sub = f.hub.subscribe(r.cx, { views: ["inbox"], focus: "inbox" }, r.message);
  t.after(() => sub.stop());
  await until(() => r.received.length === 2);
  assert.equal(r.received[1].target.key, "ENG-401");
  sub.stop();
  sub = f.hub.subscribe(r.cx, { views: ["inbox"], focus: "inbox" }, r.message);
  await until(() => r.received.length >= 3);
  assert.equal(r.received.filter(m => m.focus).length, 1);
  sub.stop();
  sidebar.stop();
  await Promise.all(r.tasks);
});

for (const remote of [true, false]) test(`${remote ? "remote" : "local"} desks ${remote ? "ignore" : "receive"} hub-side navigation`, async t => {
  const f = await fixture(t, remote), r = runtime(t);
  const sub = f.hub.subscribe(r.cx, { views: ["inbox"], focus: "inbox" }, r.message);
  t.after(() => sub.stop());
  await until(() => r.received.length === 1);
  for (const socket of f.clients) socket.write('{"focus":"inbox","target":{"app":"inbox","key":"ENG-401"}}\n{"view":"inbox","value":{"tickets":[{"id":"ENG-402"}]}}\n');
  await until(() => r.received.some(m => m.value?.tickets?.[0]?.id === "ENG-402"));
  assert.equal(r.received.filter(m => m.focus).length, remote ? 0 : 1);
  sub.stop();
  await Promise.all(r.tasks);
});

test("runbook notification navigation keeps the slug and step list", async t => {
  const f = await fixture(t), r = runtime(t);
  const sub = f.hub.subscribe(r.cx, { views: ["runbook"], focus: "runbook" }, r.message);
  t.after(() => sub.stop());
  await until(() => r.received.length === 1);
  await f.hub.cockpit(["focus", "herdr-app:runbook:release:1,2"]);
  assert.equal(r.received.at(-1).target.key, "release");
  assert.deepEqual(Array.from(r.received.at(-1).target.steps), ["1", "2"]);
  sub.stop();
  await Promise.all(r.tasks);
});

test("Show navigation reveals the work panel rather than a missing center child", async t => {
  const f = await fixture(t);
  await f.hub.go({ app: "show", key: "release#summary" });
  assert.deepEqual(JSON.parse(JSON.stringify(f.calls.filter(c => c[0] === "run"))), [
    ["run", "cockpit.go:work"],
    ["run", "layout.select", { stack: "panel-body", child: "show" }],
    ["run", "layout.show", { id: "panel" }],
  ]);
});

test("remote agent models carry their machine through reads and subscriptions", async t => {
  const f = await fixture(t), r = runtime(t);
  assert.equal((await f.hub.view("agents")).remote, "runner");
  const sub = f.hub.subscribe(r.cx, { views: ["agents"] }, r.message);
  t.after(() => sub.stop());
  await until(() => r.received.length === 1);
  assert.equal(r.received[0].value.remote, "runner");
  sub.stop();
  await Promise.all(r.tasks);
});

test("a notification that names a space opens it on the desktop", async t => {
  const f = await fixture(t);
  await f.hub.cockpit(["focus", "herdr-app:space:inbox"]);
  assert.deepEqual(f.calls.find(call => call[0] === "run"), ["run", "cockpit.go:inbox"]);
  assert.equal(f.messages.length, 0);
});

test("a notification that needs the runner's own terminals says so", async t => {
  const f = await fixture(t);
  await assert.rejects(f.hub.cockpit(["focus", "herdr-app:workspace:s1"]), /read-only/);
  await assert.rejects(f.hub.cockpit(["focus", "herdr-app:resume"]), /read-only/);
});

test("a task is given up when its own timer goes quiet or its steps outlast every bound, never while it waits to retry", async t => {
  const { hub } = await fixture(t);
  const now = 100_000;
  const at = (fields) => ({ connecting: true, parked: false, answered: true, moved: now - 1000, alive: now - 500, ...fields });
  assert.equal(hub.stuck(at({}), now), false, "a step under way, its context alive");
  assert.equal(hub.stuck(at({ alive: now - hub.SILENT_MS - 1 }), now), true, "its context went: its timer stopped");
  assert.equal(hub.stuck(at({ moved: now - 12_000 }), now), false, "a 10 s step that is still bounded");
  assert.equal(hub.stuck(at({ moved: now - hub.STUCK_MS - 1 }), now), true, "longer than every step together");
  assert.equal(hub.stuck(at({ connecting: false, alive: 0 }), now), false, "a live subscription is no task to give up");
  assert.ok(hub.STUCK_MS > 20_000, "longer than the largest bounded step, a hub start included");
  assert.equal(hub.stuck(at({ answered: false, moved: now - 9_000 }), now), false, "a first step still within its bound");
  assert.equal(hub.stuck(at({ answered: false, moved: now - hub.FIRST_MS - 1 }), now), true, "a first try the hub never answered");
  assert.ok(hub.FIRST_MS > 10_000 && hub.FIRST_MS < hub.STUCK_MS, "past the 10 s bound of a step, well short of every step together");
});

test("a first connect that never answers is given up after about 10 s, not the full 40", async t => {
  let jump = 0;
  class Clock extends Date {
    static now() { return Date.now() + jump; }
  }
  const f = await fixture(t, false, { Date: Clock });
  f.behavior.hangs = 1;
  const r = runtime(t), beats = [];
  r.cx.timer.every = (_ms, callback) => (beats.push(callback), { cancel: () => beats.splice(beats.indexOf(callback), 1) });
  const beat = () => [...beats].forEach((b) => b(r.cx));
  const subscription = f.hub.subscribe(r.cx, { views: ["inbox"] }, r.message, r.state);
  t.after(() => subscription.stop());
  await until(() => f.calls.some((c) => c[0] === "hang"));
  jump = 5_000;
  beat();
  await new Promise((resolve) => setTimeout(resolve, 30));
  assert.equal(r.received.length, 0, "5 s in, the first try keeps its chance");
  assert.ok(!r.states.includes("live"));
  jump = 5_000 + f.hub.FIRST_MS;
  beat();
  await until(() => r.received.length === 1);
  assert.equal(r.states.at(-1), "live", "a new task got there, long before STUCK_MS");
});

test("the watchdog parks copies that stopped beating and closes only sockets on another hub's port it knows", async t => {
  const { hub } = await fixture(t);
  const sub = (beat, port, socket = {}) => ({ beat, port, socket, parked: false, stopped: false });
  const a = sub(100_000, 5001), b = sub(100_000, 5000), gone = sub(80_000, 5001);
  const plain = (v) => JSON.parse(JSON.stringify({ park: v.park.map((s) => [a, b, gone].indexOf(s)), close: v.close.map((s) => [a, b, gone].indexOf(s)) }));
  assert.deepEqual(plain(hub.verdict([a, b, gone], 5001)), { park: [2], close: [1] }, "the gone copy parked, b on an old hub closed");
  assert.deepEqual(plain(hub.verdict([a, b, gone], null)), { park: [2], close: [] }, "a ping that failed closes nothing");
  gone.parked = true;
  assert.deepEqual(plain(hub.verdict([a, b, gone], 5001)), { park: [], close: [1] }, "a parked copy waits for its own beat");
});

test("a hub stopped on purpose is started again only by Start it, and a runner's hub never by a desk on its own", async t => {
  const local = await fixture(t, false);
  local.behavior.stopped = true;
  await assert.rejects(local.hub.startHub(), (e) => local.hub.isStoppedOnPurpose(e));
  assert.equal(local.calls.filter((c) => c[0] === "exec").length, 0, "nothing started");
  await local.hub.startHub({ explicit: true });
  const started = (calls) => calls.some((c) => c[0] === "exec" && JSON.stringify(c).includes('"hub","start"'));
  assert.ok(started(local.calls), "Start it starts it");
  const remote = await fixture(t, true);
  remote.behavior.exec = { code: 1, stdout: "", stderr: "" };
  await assert.rejects(remote.hub.startHub(), /runner's cockpit hub is not running/);
  assert.ok(!started(remote.calls), "the desk only asked whether it runs");
});

test("a machine that slept takes no copy for gone: all three keep their values after a 60 s jump of the clock", async t => {
  let jump = 0;
  class Clock extends Date {
    static now() { return Date.now() + jump; }
  }
  const f = await fixture(t, false, { Date: Clock });
  // Three copies, each with its own context whose beat the test fires by hand.
  const copies = [0, 1, 2].map(() => {
    const r = runtime(t), beats = [];
    r.cx.timer.every = (_ms, callback) => (beats.push(callback), { cancel: () => beats.splice(beats.indexOf(callback), 1) });
    return { ...r, beats, beat: () => beats.forEach((b) => b(r.cx)) };
  });
  const subs = copies.map((c) => f.hub.subscribe(c.cx, { views: ["inbox"] }, c.message, c.state));
  t.after(() => subs.forEach((s) => s.stop()));
  await until(() => copies.every((c) => c.received.length >= 1));
  for (const c of copies) c.beat();
  await new Promise((r) => setTimeout(r, 20));
  jump = 60_000;
  // The first copy to beat after the wake runs the watchdog, the others not beaten since before the sleep.
  copies[0].beat();
  await new Promise((r) => setTimeout(r, 50));
  // The other two have not beaten since: they still follow the hub, on the sockets they had.
  for (const socket of f.clients) socket.write('{"view":"inbox","value":{"tickets":[{"id":"ENG-402"}]}}\n');
  await until(() => copies.every((c) => c.received.some((m) => m.value?.tickets?.[0]?.id === "ENG-402")));
  assert.ok(copies.every((c) => c.states.at(-1) === "live"), "every copy is still live");
});

test("every surface words a hub that does not answer the same way: stopped on purpose, else not running", async t => {
  const { hub } = await fixture(t, false);
  assert.equal(hub.downWords("error: the cockpit hub was stopped on purpose"), "The cockpit hub was stopped");
  assert.equal(hub.downWords("error: connect ECONNREFUSED 127.0.0.1:7410"), "The cockpit hub isn't running");
  assert.equal(hub.downWords("error: the cockpit hub was stopped on purpose", true), "Hub stopped");
  assert.equal(hub.downWords("error: timed out", true), "Hub not running");
});

test("the window keeps why it last failed to reach the hub, in the error's own words", async t => {
  const f = await fixture(t, false), r = runtime(t);
  assert.equal(f.hub.lastFailure(), null);
  assert.equal(f.hub.downReason("connecting"), "", "nothing failed yet");
  // hub.json names a port nothing listens on, and starting a hub fails with what the binary said.
  f.info.port = 1;
  f.behavior.exec = { code: 1, stdout: "", stderr: "cockpit hub: address already in use (port 7410)\n" };
  const sub = f.hub.subscribe(r.cx, { views: ["inbox"] }, r.message, r.state);
  t.after(() => sub.stop());
  await until(() => r.states.some((s) => s.startsWith("error")));
  const state = r.states.find((s) => s.startsWith("error"));
  assert.equal(state, "error: cockpit hub: address already in use (port 7410)");
  assert.equal(f.hub.lastFailure().message, "cockpit hub: address already in use (port 7410)");
  assert.ok(f.hub.lastFailure().at > 0);
  assert.equal(f.hub.downReason(state), "cockpit hub: address already in use (port 7410)");
  assert.equal(f.hub.downTip(state), "The cockpit hub isn't running: cockpit hub: address already in use (port 7410)");
  // While it tries again, the reason of the try before is still the one to say.
  assert.equal(f.hub.downReason("connecting"), "cockpit hub: address already in use (port 7410)");
  assert.equal(f.hub.downTip("error: the cockpit hub was stopped on purpose"), "The cockpit hub was stopped on purpose, with cockpit hub stop");
  sub.stop();
  // A subscription its own copy stopped says nothing about the hub.
  assert.equal(f.hub.lastFailure().message, "cockpit hub: address already in use (port 7410)");
  // An old failure is no reason any more.
  assert.equal(f.hub.lastFailure(Date.now() + 11 * 60_000), null);
  // Reaching the hub again clears it.
  f.info.port = f.server.address().port;
  assert.equal((await f.hub.view("inbox")).tickets[0].id, "ENG-401");
  assert.equal(f.hub.lastFailure(), null);
  assert.equal(f.hub.downReason("connecting"), "");
});

test("a window that came up before its hub goes live as soon as hub.json names one, its own start still under way", async t => {
  const f = await fixture(t, false), r = runtime(t);
  f.behavior.stopped = true;
  f.behavior.noInfo = true;
  const sidebar = f.hub.subscribe(r.cx, { views: ["inbox"] }, r.message, r.state);
  t.after(() => sidebar.stop());
  await until(() => r.states.some((s) => f.hub.isStoppedOnPurpose(s)));
  // Let go, the window's own `cockpit hub start` never comes back; a hub started another way writes hub.json.
  f.behavior.startHangs = true;
  f.behavior.stopped = false;
  await until(() => f.calls.some((c) => c[0] === "exec" && c[2].includes("start")));
  f.behavior.noInfo = false;
  await until(() => r.received.length === 1);
  assert.equal(r.states.at(-1), "live");
  // Start it joins the start under way, and the views stay live.
  const started = f.hub.startHub({ explicit: true });
  started.catch(() => {});
  assert.equal(f.calls.filter((c) => c[0] === "exec" && c[2].includes("start")).length, 1);
  sidebar.stop();
});
