import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import net from "node:net";
import { once } from "node:events";
import { createContext, SourceTextModule, SyntheticModule } from "node:vm";
import { test } from "node:test";

async function fixture(t, remote = true, globals = {}, host = "127.0.0.1") {
  const calls = [], clients = new Set(), messages = [];
  const behavior = { token: "test-token", silent: false, bytewise: false, tickets: [{ id: "ENG-401" }], keepalives: 0 };
  const server = net.createServer(socket => {
    clients.add(socket);
    socket.on("close", () => clients.delete(socket));
    let pending = "";
    socket.on("data", data => {
      pending += data;
      while (pending.includes("\n")) {
        const end = pending.indexOf("\n");
        const line = pending.slice(0, end);
        pending = pending.slice(end + 1);
        if (!line.trim()) {
          behavior.keepalives += 1;
          continue;
        }
        const request = JSON.parse(line);
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
        assert.equal(options?.on, install.machine);
        return JSON.stringify(info);
      },
      exec: async (...args) => { calls.push(["exec", ...args]); return behavior.exec ?? { code: 0, stdout: JSON.stringify({ ...info, token: behavior.token }), stderr: "" }; },
    },
    net: { default: { connect: async (host, port) => {
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
  return { hub: module.namespace, calls, info, install, messages, clients, behavior, server, host };
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

test("subscriptions promise the hub a line every half minute and keep it", async t => {
  const clock = { offset: 0 };
  const f = await fixture(t, true, { Date: { now: () => Date.now() + clock.offset } }), r = runtime(t);
  const subscription = f.hub.subscribe(r.cx, { views: ["inbox"] }, r.message, r.state);
  t.after(() => subscription.stop());
  await until(() => r.received.length === 1);
  assert.equal(f.messages.find(m => m.op === "subscribe").keepalive, 30);
  for (const socket of f.clients) socket.write("\n");
  await new Promise(resolve => setTimeout(resolve, 20));
  assert.equal(f.behavior.keepalives, 0, "a line soon after the last write needs no answer");
  clock.offset = 26_000;
  for (const socket of f.clients) socket.write("\n");
  await until(() => f.behavior.keepalives === 1);
  subscription.stop();
  await Promise.all(r.tasks);
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
