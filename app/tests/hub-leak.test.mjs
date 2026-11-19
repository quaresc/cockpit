// request() gives up on a connection that is too slow, and closes it when it comes later.
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import net from "node:net";
import { once } from "node:events";
import { createContext, SourceTextModule, SyntheticModule } from "node:vm";
import { test } from "node:test";

const HUB = new URL("../mods/cockpit/hub.js", import.meta.url);

/**
 * hub.js's source for one new context, told apart from every other context's by a comment: Node 20 and 22 reuse code
 * compiled for the same source text across vm contexts, which crashes the process (SIGSEGV) when one test's context
 * compiles what another's already did.
 */
let compiled = 0;
const hubSource = async () => `${await readFile(HUB, "utf8")}\n// context ${++compiled}\n`;

async function fixture(t, { connectDelay = 0, stopped = false } = {}) {
  const clients = new Set(), calls = [];
  const server = net.createServer((socket) => {
    clients.add(socket);
    socket.on("close", () => clients.delete(socket));
    let pending = "";
    socket.on("data", (d) => {
      pending += d;
      while (pending.includes("\n")) {
        const end = pending.indexOf("\n");
        const req = JSON.parse(pending.slice(0, end));
        pending = pending.slice(end + 1);
        socket.write(JSON.stringify(req.op === "ping" ? { ok: true } : { value: {} }) + "\n");
      }
    });
  });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  t.after(() => { for (const s of clients) s.destroy(); server.close(); });
  const info = { host: "127.0.0.1", port: server.address().port, token: "tok" };
  const install = { bin: "/x/cockpit", home: "/x", cache: "/x/cache" };
  const context = createContext({ console });
  const imports = {
    herdr: {
      appHome: () => "/app",
      window: () => ({ platform: "macos" }),
      run: () => {},
      readTextFile: async (path) => {
        if (path === "/app/cockpit.json") return JSON.stringify(install);
        if (path.endsWith("/hub.stopped")) return stopped ? "" : null;
        return JSON.stringify(info);
      },
      exec: async (...a) => { calls.push(a); return { code: 0, stdout: JSON.stringify(info), stderr: "" }; },
    },
    net: { default: { connect: async (host, port) => {
      if (connectDelay) await new Promise((r) => setTimeout(r, connectDelay));
      const socket = net.createConnection({ host, port });
      await once(socket, "connect");
      const chunks = (async function* () { for await (const c of socket) yield c; })();
      return {
        write: (d) => new Promise((res, rej) => socket.write(d, (e) => (e ? rej(e) : res()))),
        read: async () => { const c = await chunks.next(); return c.done ? null : c.value; },
        close: () => socket.destroy(),
      };
    } } },
  };
  const module = new SourceTextModule(await hubSource(), { context });
  await module.link((name) => new SyntheticModule(Object.keys(imports[name]), function () {
    for (const [k, v] of Object.entries(imports[name])) this.setExport(k, v);
  }, { context }));
  await module.evaluate();
  return { hub: module.namespace, clients, calls };
}

test("request(): a connection that comes after its 20 s bound gave up is closed, not left open", async (t) => {
  const f = await fixture(t, { connectDelay: 50 });
  // A cx whose 20 s timer (request's outer bound) fires at once and whose 10 s timers (open's own steps) never do.
  const cx = { timer: { after: (ms, fn) => { const id = ms === 20_000 ? setTimeout(fn, 1) : null; return { cancel: () => id && clearTimeout(id) }; } } };
  await assert.rejects(f.hub.act("inbox", "set_muted", ["api", true], cx), /did not answer/);
  // Let the late connect and its ping finish.
  await new Promise((r) => setTimeout(r, 300));
  assert.equal(f.clients.size, 0, `a connection given up on should be closed, ${f.clients.size} still open`);
});

// hub.js evaluated again and again in one window's context: a save or an update reloads the mod (the older load's
// tasks left waiting on their reads), and Herdr 0.1.12 evaluates it once per copy, every load running at once.
async function loads(t, n, { connectDelay = () => 0, together = false } = {}) {
  const clients = new Set();
  const server = net.createServer((socket) => {
    clients.add(socket);
    socket.on("close", () => clients.delete(socket));
    let pending = "";
    socket.on("data", (d) => {
      pending += d;
      while (pending.includes("\n")) {
        const end = pending.indexOf("\n");
        const req = JSON.parse(pending.slice(0, end));
        pending = pending.slice(end + 1);
        // A subscription gets its first value and then waits, as one does between changes.
        socket.write(JSON.stringify(req.op === "ping" ? { ok: true } : { view: "agents", value: { agents: [] } }) + "\n");
      }
    });
  });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  t.after(() => { for (const s of clients) s.destroy(); server.close(); });
  const info = { host: "127.0.0.1", port: server.address().port, token: "tok" };
  const install = { bin: "/x/cockpit", home: "/x", cache: "/x/cache" };
  const context = createContext({ console });
  // The loads share one context, as a window's do: one source for all of them.
  const source = await hubSource();
  const namespaces = [];
  const evaluated = [];
  for (let i = 0; i < n; i++) {
    const delay = connectDelay(i);
    const imports = {
      herdr: {
        appHome: () => "/app",
        window: () => ({ platform: "macos" }),
        run: () => {},
        readTextFile: async (path) => (path === "/app/cockpit.json" ? JSON.stringify(install) : path.endsWith("/hub.stopped") ? null : JSON.stringify(info)),
        exec: async () => ({ code: 0, stdout: JSON.stringify(info), stderr: "" }),
      },
      net: { default: { connect: async (host, port) => {
        if (delay) await new Promise((r) => setTimeout(r, delay));
        const socket = net.createConnection({ host, port });
        await once(socket, "connect");
        const chunks = (async function* () { for await (const c of socket) yield c; })();
        return {
          write: (d) => new Promise((res, rej) => socket.write(d, (e) => (e ? rej(e) : res()))),
          read: async () => { const c = await chunks.next(); return c.done ? null : c.value; },
          close: () => socket.destroy(),
        };
      } } },
    };
    const module = new SourceTextModule(source, { context, identifier: `hub.js#${i + 1}` });
    await module.link((name) => new SyntheticModule(Object.keys(imports[name]), function () {
      for (const [k, v] of Object.entries(imports[name])) this.setExport(k, v);
    }, { context }));
    await module.evaluate();
    evaluated.push(module.namespace);
    if (!together) namespaces.push(follow(module.namespace));
  }
  // Every copy starts once every load is evaluated, as a window starting on Herdr 0.1.12 does.
  if (together) namespaces.push(...evaluated.map(follow));
  return { clients, namespaces };
}

/** A copy following two views on `hub`: tasks run, timers never fire (a reload stops the older load's), no beat. */
function follow(hub) {
  const cx = { spawn: (f) => f(cx), notify() {}, timer: { after: () => ({ cancel() {} }) } };
  let live = false;
  const states = [];
  hub.subscribe(cx, { views: ["agents"] }, () => (live = true), (state) => states.push(state));
  hub.subscribe(cx, { views: ["queue"] }, () => {}, (state) => states.push(state));
  return { hub, live: () => live, states };
}

const settle = async (clients, want, ms = 2000) => {
  const until = Date.now() + ms;
  while (clients.size !== want && Date.now() < until) await new Promise((r) => setTimeout(r, 20));
};

test("loads evaluated together, one per copy, all follow the hub: none takes another's connections", async (t) => {
  const { clients, namespaces } = await loads(t, 6, { together: true });
  await settle(clients, 12);
  await new Promise((r) => setTimeout(r, 100));
  assert.equal(clients.size, 12, `each load's two subscriptions should stay open, ${clients.size} are`);
  for (const [i, n] of namespaces.entries()) {
    assert.equal(n.live(), true, `load ${i + 1} should get its values`);
    assert.deepEqual(n.states.filter((s) => s !== "connecting" && s !== "live"), [], `load ${i + 1} should never be down`);
  }
  // A sweep while every load still runs closes nothing.
  namespaces.at(-1).hub.sweep?.();
  await new Promise((r) => setTimeout(r, 100));
  assert.equal(clients.size, 12);
});

test("the connections of loads that stopped running are closed once past every bound, even one that came late", async (t) => {
  // The first load's connects take 300 ms; nine loads come after it, then only the newest runs.
  const { clients, namespaces } = await loads(t, 10, { connectDelay: (i) => (i === 0 ? 300 : 0) });
  await new Promise((r) => setTimeout(r, 600));
  await settle(clients, 20);
  namespaces.at(-1).hub.sweep(Date.now() + 91_000);
  await settle(clients, 2);
  assert.equal(clients.size, 2, `the newest load's two subscriptions alone should be open, ${clients.size} are`);
  assert.equal(namespaces.at(-1).live(), true, "and the newest one is live");
});

// Herdr's socket read gives up after 30 s with nothing to read, and rejects with the system's "would block": an idle
// subscription, and a ping the hub is slow to answer, read again on the same connection instead of dropping it.
const WOULD_BLOCK = "socket read failed: Resource temporarily unavailable (os error 35)";

async function idleReads(t, { gaveUp = 2 } = {}) {
  const clients = [];
  const server = net.createServer((socket) => {
    clients.push(socket);
    let pending = "";
    socket.on("data", (d) => {
      pending += d;
      while (pending.includes("\n")) {
        const end = pending.indexOf("\n");
        const req = JSON.parse(pending.slice(0, end));
        pending = pending.slice(end + 1);
        socket.write(JSON.stringify(req.op === "ping" ? { ok: true } : req.op === "subscribe" ? { view: "agents", value: { agents: [] } } : { note: "done" }) + "\n");
      }
    });
  });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  t.after(() => { for (const s of clients) s.destroy(); server.close(); });
  const info = { host: "127.0.0.1", port: server.address().port, token: "tok" };
  const install = { bin: "/x/cockpit", home: "/x", cache: "/x/cache" };
  let refusals = 0;
  const imports = {
    herdr: {
      appHome: () => "/app",
      window: () => ({ platform: "macos" }),
      run: () => {},
      readTextFile: async (path) => (path === "/app/cockpit.json" ? JSON.stringify(install) : path.endsWith("/hub.stopped") ? null : JSON.stringify(info)),
      exec: async () => ({ code: 0, stdout: JSON.stringify(info), stderr: "" }),
    },
    net: { default: { connect: async (host, port) => {
      const socket = net.createConnection({ host, port });
      // The test's end resets what is left open: no error of its own.
      socket.on("error", () => {});
      await once(socket, "connect");
      const chunks = (async function* () { for await (const c of socket) yield c; })();
      // Each connection's first reads give up as Herdr's do when nothing came in time.
      let idle = gaveUp;
      return {
        write: (d) => new Promise((res, rej) => socket.write(d, (e) => (e ? rej(e) : res()))),
        read: async () => {
          if (idle > 0) {
            idle -= 1;
            refusals += 1;
            throw new Error(WOULD_BLOCK);
          }
          const c = await chunks.next();
          return c.done ? null : c.value;
        },
        close: () => socket.destroy(),
      };
    } } },
  };
  const context = createContext({ console });
  const module = new SourceTextModule(await hubSource(), { context, identifier: "hub.js#idle" });
  await module.link((name) => new SyntheticModule(Object.keys(imports[name]), function () {
    for (const [k, v] of Object.entries(imports[name])) this.setExport(k, v);
  }, { context }));
  await module.evaluate();
  return { hub: module.namespace, clients, refusals: () => refusals };
}

test("a read that gives up with nothing to read keeps the connection: the ping, an idle subscription, a request", async (t) => {
  const f = await idleReads(t);
  assert.equal(f.hub.idleRead(new Error(WOULD_BLOCK)), true);
  assert.equal(f.hub.idleRead(new Error("socket read failed: Resource temporarily unavailable (os error 11)")), true, "Linux's");
  assert.equal(f.hub.idleRead(new Error("socket read failed: Connection reset by peer (os error 54)")), false);
  // A context whose short timers run (the pause between reads) and whose deadlines never come.
  const cx = { spawn: (fn) => fn(cx), notify() {}, timer: { after: (ms, fn) => { const id = ms <= 1000 ? setTimeout(fn, ms) : null; return { cancel: () => id && clearTimeout(id) }; } } };
  const states = [];
  let value = null;
  const sub = f.hub.subscribe(cx, { views: ["agents"] }, (m) => (value = m.value), (s) => states.push(s));
  t.after(() => sub.stop());
  const until = Date.now() + 2000;
  while (value === null && Date.now() < until) await new Promise((r) => setTimeout(r, 10));
  assert.equal(JSON.stringify(value), JSON.stringify({ agents: [] }), "live on its first connection");
  assert.equal(f.clients.length, 1, `one connection, never dropped and opened again: ${f.clients.length}`);
  assert.ok(!states.some((s) => s.startsWith("error")), `no error said: ${states.join(", ")}`);
  assert.ok(f.refusals() >= 2, "its reads did give up first");
  // A one-off request with no timer to wait on reads again too.
  const answer = await f.hub.act("inbox", "set_muted", ["api", true]);
  assert.equal(answer.note, "done");
});

test("a read that keeps giving up with no timer to wait on is a broken socket in the end", async (t) => {
  const f = await idleReads(t, { gaveUp: Infinity });
  await assert.rejects(f.hub.act("inbox", "set_muted", ["api", true]), /Resource temporarily unavailable/);
});
