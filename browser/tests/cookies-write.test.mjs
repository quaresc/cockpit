// Writing imported cookies into a session, against a fake Electron session and a fake debugger that share one jar.
// The hung-command, pending-flush and rollback cases are ported from Orca (https://github.com/stablyai/orca, commit 49498c3),
// src/main/browser/cookies/browser-cookie-clear-store-lifecycle.test.ts, browser-cookie-import-snapshot-ownership.test.ts and
// browser-cookie-import-partition-rollback.electron.test.ts. MIT licence, Copyright (c) 2026 Lovecast Inc.

import assert from "node:assert/strict";
import { test } from "node:test";
import { boundedCommandSender, cdpSetCookieParamsFromIdentity, cookieIdentitiesFromCdp } from "../main/cookies/cdp.js";
import { write } from "../main/cookies/index.js";
import { acquireCookieMutationLock, withCookieMutationLock } from "../main/cookies/lock.js";
import { importedDomainScope } from "../main/cookies/policy.js";
import { removeCookiesInScope } from "../main/cookies/replace.js";
import { importedCookieIdentity } from "../main/cookies/write.js";

const NOW = Date.UTC(2026, 0, 15);
const LATER = NOW / 1000 + 3600;
const CHIPS_KEY = { topLevelSite: "https://acme.example", hasCrossSiteAncestor: true };

const cookie = (fields = {}) => ({
  name: "session",
  value: "imported",
  domain: ".acme.example",
  path: "/",
  secure: true,
  httpOnly: true,
  sameSite: "lax",
  partition: { status: "unpartitioned" },
  ...fields,
});

/** A cookie already in the destination jar, as the fake keeps it. */
const existing = (fields = {}) => ({ name: "session", value: "stale", domain: ".acme.example", path: "/", secure: true, httpOnly: false, sameSite: "Lax", ...fields });

const tick = () => new Promise((resolve) => setImmediate(resolve));

/**
 * One jar seen two ways, as Electron shows it: `session.cookies` without partition keys, and a debugger's
 * `sendCommand` with them. `events` records every call in order.
 */
function fakeTarget(initial = [], hooks = {}) {
  const jar = initial.map((entry) => ({ ...entry }));
  const events = [];
  const host = (entry) => entry.domain.replace(/^\./, "");
  const session = {
    cookies: {
      get: async (filter) => {
        events.push(["get", filter]);
        return jar.map(({ partitionKey, sameSite, ...entry }) => ({ ...entry, hostOnly: !entry.domain.startsWith("."), sameSite: "unspecified" }));
      },
      remove: async (url, name) => {
        events.push(["remove", url, name]);
        await hooks.remove?.(url, name);
        const target = new URL(url);
        for (let index = jar.length - 1; index >= 0; index -= 1) {
          if (jar[index].name === name && host(jar[index]) === target.hostname && jar[index].path === target.pathname) jar.splice(index, 1);
        }
      },
      flushStore: async () => {
        events.push(["flush"]);
        await hooks.flush?.();
      },
    },
  };
  const sendCommand = async (method, params) => {
    events.push([method, params]);
    await tick();
    await hooks.command?.(method, params);
    if (method === "Network.getAllCookies") {
      return { cookies: jar.map((entry) => ({ ...entry, session: entry.expires === undefined })) };
    }
    if (method === "Network.setCookie") {
      const refused = hooks.refuse?.(params) === true;
      if (refused) return { success: false };
      const domain = params.domain ?? new URL(params.url).hostname;
      const same = (entry) => entry.name === params.name && entry.domain === domain && entry.path === (params.path ?? "/") && JSON.stringify(entry.partitionKey ?? null) === JSON.stringify(params.partitionKey ?? null);
      const index = jar.findIndex(same);
      if (index >= 0) jar.splice(index, 1);
      const { url, ...stored } = params;
      jar.push({ ...stored, domain, path: params.path ?? "/" });
      return { success: true };
    }
    throw new Error(`unexpected command ${method}`);
  };
  const sets = () => events.filter(([method]) => method === "Network.setCookie").map(([, params]) => params);
  const count = (kind) => events.filter(([method]) => method === kind).length;
  const values = () => jar.map((entry) => `${entry.name}@${entry.domain}=${entry.value}`).sort();
  return { session, sendCommand, jar, events, sets, count, values };
}

test("an imported cookie is written where the source held it", () => {
  assert.deepEqual(cdpSetCookieParamsFromIdentity(importedCookieIdentity(cookie({ expirationDate: LATER }))), {
    url: "https://acme.example/",
    name: "session",
    value: "imported",
    domain: ".acme.example",
    path: "/",
    secure: true,
    httpOnly: true,
    sameSite: "Lax",
    expires: LATER,
  });
  // A host-only cookie goes without a domain, so it is not widened to the subdomains.
  assert.deepEqual(cdpSetCookieParamsFromIdentity(importedCookieIdentity(cookie({ domain: "api.acme.example", path: "/v1", secure: false, httpOnly: false, sameSite: "unspecified" }))), {
    url: "http://api.acme.example/",
    name: "session",
    value: "imported",
    path: "/v1",
    secure: false,
    httpOnly: false,
  });
  // A __Host- cookie is host-only at the root path whatever the source says.
  const hostPrefixed = cdpSetCookieParamsFromIdentity(
    importedCookieIdentity(cookie({ name: "__Host-chips", domain: ".app.acme.example", path: "/deep", sameSite: "no_restriction", partition: { status: "partitioned", partitionKey: CHIPS_KEY } })),
  );
  assert.equal("domain" in hostPrefixed, false);
  assert.equal(hostPrefixed.path, "/");
  assert.equal(hostPrefixed.sameSite, "None");
  assert.deepEqual(hostPrefixed.partitionKey, CHIPS_KEY);
  assert.equal(cdpSetCookieParamsFromIdentity(importedCookieIdentity(cookie({ sameSite: "strict" }))).sameSite, "Strict");
});

test("write sends every kept cookie through Network.setCookie and flushes the store once", async () => {
  const target = fakeTarget();
  const cookies = [cookie({ expirationDate: LATER }), cookie({ name: "token", domain: "api.acme.example" }), cookie({ name: "chips", domain: "embed.acme.example", partition: { status: "partitioned", partitionKey: CHIPS_KEY } })];

  const result = await write(cookies, { ...target, now: NOW });

  assert.deepEqual(result, { imported: 3, failed: 0, domains: ["acme.example", "api.acme.example", "embed.acme.example"], replaced: 0, excluded: {} });
  assert.deepEqual(target.sets().map((params) => [params.url, params.name, params.domain, params.partitionKey]), [
    ["https://acme.example/", "session", ".acme.example", undefined],
    ["https://api.acme.example/", "token", undefined, undefined],
    ["https://embed.acme.example/", "chips", undefined, CHIPS_KEY],
  ]);
  assert.equal(target.count("flush"), 1);
  assert.deepEqual(target.events.at(-1), ["flush"]);
});

test("write replaces existing cookies only for the domains in the import", async () => {
  const target = fakeTarget([
    existing(),
    existing({ name: "csrf", domain: "acme.example" }),
    existing({ name: "token", domain: "api.acme.example" }),
    existing({ name: "deep", domain: "v2.api.acme.example" }),
    existing({ name: "other", domain: ".other.example" }),
    existing({ name: "lookalike", domain: "notacme.example" }),
    existing({ name: "SID", domain: "accounts.google.com" }),
  ]);

  const result = await write([cookie(), cookie({ name: "token", value: "imported", domain: "api.acme.example" })], { ...target, now: NOW });

  assert.equal(result.imported, 2);
  assert.equal(result.replaced, 4);
  assert.deepEqual(result.domains, ["acme.example", "api.acme.example"]);
  assert.deepEqual(target.values(), [
    "SID@accounts.google.com=stale",
    "lookalike@notacme.example=stale",
    "other@.other.example=stale",
    "session@.acme.example=imported",
    "token@api.acme.example=imported",
  ]);
  // Every removal comes before the first write.
  const kinds = target.events.map(([kind]) => kind);
  assert.equal(kinds.lastIndexOf("remove") < kinds.indexOf("Network.setCookie"), true);
});

test("write of a host-only cookie leaves the host's subdomains and a parent's host-only cookie alone", async () => {
  const target = fakeTarget([
    existing({ name: "token", domain: "api.acme.example" }),
    existing({ name: "deep", domain: "v2.api.acme.example" }),
    existing({ name: "csrf", domain: "acme.example" }),
    existing({ name: "session", domain: ".acme.example" }),
  ]);

  const result = await write([cookie({ name: "token", domain: "api.acme.example" })], { ...target, now: NOW });

  assert.equal(result.replaced, 2);
  assert.deepEqual(target.values(), ["csrf@acme.example=stale", "deep@v2.api.acme.example=stale", "token@api.acme.example=imported"]);
});

test("write never clears or writes a site that binds its session to the device", async () => {
  const target = fakeTarget([existing({ name: "SID", domain: ".google.com" }), existing({ name: "PREF", domain: ".youtube.com" })]);
  const cookies = [cookie({ name: "SID", domain: ".google.com" }), cookie({ name: "SIDCC", domain: ".google.com" }), cookie({ name: "PREF", domain: ".youtube.com" })];

  const result = await write(cookies, { ...target, now: NOW });

  assert.deepEqual(result, { imported: 1, failed: 0, domains: ["youtube.com"], replaced: 1, excluded: { googleSourceBound: 1, nonTransplantableDomain: 1 } });
  assert.deepEqual(target.values(), ["PREF@.youtube.com=imported", "SID@.google.com=stale"]);
});

test("the clear leaves a device-bound site alone even when the scope names it", async () => {
  const target = fakeTarget([existing(), existing({ name: "SID", domain: "accounts.google.com" }), existing({ name: "NID", domain: ".google.com" })]);
  const scope = importedDomainScope([".acme.example", "accounts.google.com", ".google.com"]);

  assert.equal(await removeCookiesInScope(target.session.cookies, target.sendCommand, new Set(), scope), 1);
  assert.deepEqual(target.values(), ["NID@.google.com=stale", "SID@accounts.google.com=stale"]);
  assert.equal(await removeCookiesInScope(target.session.cookies, target.sendCommand, new Set(), importedDomainScope([])), 0);
});

test("write with nothing to import touches neither the jar nor the debugger", async () => {
  const target = fakeTarget([existing()]);
  const cookies = [cookie({ expirationDate: NOW / 1000 - 1 }), cookie({ name: "SID", domain: ".google.com" }), cookie({ domain: ".co.uk" })];

  const result = await write(cookies, { ...target, now: NOW });

  assert.deepEqual(result, { imported: 0, failed: 0, domains: [], replaced: 0, excluded: { expired: 1, nonTransplantableDomain: 1, invalidDomain: 1 } });
  assert.deepEqual(target.events, []);
  assert.deepEqual(await write([], target), { imported: 0, failed: 0, domains: [], replaced: 0, excluded: {} });
});

test("write keeps a family's existing cookies when one of its imported cookies has an unreadable partition", async () => {
  const target = fakeTarget([existing(), existing({ name: "other", domain: ".other.example" })]);
  const cookies = [
    cookie(),
    cookie({ name: "chips", domain: "embed.acme.example", partition: { status: "unreadable", reason: "partition key was opaque" } }),
    cookie({ name: "other", domain: ".other.example" }),
  ];

  const result = await write(cookies, { ...target, now: NOW });

  assert.equal(result.imported, 2);
  assert.equal(result.replaced, 1);
  assert.deepEqual(result.excluded, { partitionUnreadable: 1 });
  assert.deepEqual(target.events.filter(([kind]) => kind === "remove").map(([, url, name]) => [url, name]), [["https://other.example/", "other"]]);
  // The readable cookie still lands on its own coordinate.
  assert.deepEqual(target.values(), ["other@.other.example=imported", "session@.acme.example=imported"]);
});

test("write counts a cookie the store refuses and still writes the rest", async () => {
  const logged = [];
  const target = fakeTarget([], {
    refuse: (params) => params.name === "bad",
    command: (method, params) => {
      if (params?.name === "broken") throw new Error("Invalid cookie fields");
    },
  });
  const cookies = [cookie({ name: "bad", value: "café" }), cookie({ name: "broken" }), cookie({ name: "good" })];

  const result = await write(cookies, { ...target, now: NOW, log: (message) => logged.push(message) });

  assert.deepEqual(result, { imported: 1, failed: 2, domains: ["acme.example"], replaced: 0, excluded: {} });
  assert.equal(target.count("flush"), 1);
  assert.equal(logged.length, 2);
  assert.match(logged[0], /cookie write rejected: domain=acme\.example valLen=4 badChar=pos=3 char=U\+00e9/);
  // The log names the domain and the value's length, never the value.
  assert.equal(logged.some((message) => message.includes("caf")), false);
});

test("write does not flush when no cookie went in", async () => {
  const target = fakeTarget([], { refuse: () => true });
  const result = await write([cookie()], { ...target, now: NOW });
  assert.deepEqual(result, { imported: 0, failed: 1, domains: [], replaced: 0, excluded: {} });
  assert.equal(target.count("flush"), 0);
});

test("write survives a flush that fails", async () => {
  const target = fakeTarget([], {
    flush: () => {
      throw new Error("disk full");
    },
  });
  const logged = [];
  const result = await write([cookie()], { ...target, now: NOW, log: (message) => logged.push(message) });
  assert.equal(result.imported, 1);
  assert.deepEqual(logged, ["cookie flush failed: Error: disk full"]);
});

test("write puts every removed cookie back, partition included, when one removal fails", async () => {
  const initial = [
    existing(),
    existing({ name: "chips", domain: "embed.acme.example", sameSite: "None", partitionKey: CHIPS_KEY, expires: LATER }),
    existing({ name: "victim", domain: "app.acme.example" }),
    existing({ name: "SID", domain: "accounts.google.com" }),
  ];
  const target = fakeTarget(initial, {
    remove: async (_url, name) => {
      await tick();
      if (name === "victim") throw new Error("forced victim removal failure");
    },
  });
  const before = target.values();

  const result = await write([cookie(), cookie({ name: "theme" })], { ...target, now: NOW });

  assert.deepEqual(result, { imported: 0, failed: 2, domains: [], replaced: 0, excluded: {}, error: "Could not clear existing cookies; existing cookies were restored" });
  assert.deepEqual(target.values(), before);
  const restored = target.sets();
  assert.deepEqual(restored.map((params) => params.name).sort(), ["chips", "session", "victim"]);
  const chips = restored.find((params) => params.name === "chips");
  assert.deepEqual(chips, { url: "https://embed.acme.example/", name: "chips", value: "stale", path: "/", secure: true, httpOnly: false, sameSite: "None", expires: LATER, partitionKey: CHIPS_KEY });
  assert.equal(target.count("flush"), 0);
});

test("write removes nothing when it cannot first record what it would remove", async () => {
  const failing = fakeTarget([existing()], {
    command: (method) => {
      if (method === "Network.getAllCookies") throw new Error("debugger detached");
    },
  });
  const result = await write([cookie()], { ...failing, now: NOW });
  assert.deepEqual(result, { imported: 0, failed: 1, domains: [], replaced: 0, excluded: {}, error: "debugger detached" });
  assert.deepEqual(failing.values(), ["session@.acme.example=stale"]);
  assert.equal(failing.count("remove"), 0);

  // A cookie the listing does not show, or one in an opaque partition, cannot be restored, so nothing is removed.
  const entry = { cookie: { name: "session", domain: ".acme.example", path: "/", hostOnly: false }, url: "https://acme.example/" };
  assert.throws(() => cookieIdentitiesFromCdp([entry], []), /Could not snapshot cookie identity/);
  assert.throws(() => cookieIdentitiesFromCdp([entry], [existing({ partitionKeyOpaque: true })]), /Could not snapshot cookie identity/);
  assert.equal(cookieIdentitiesFromCdp([entry], [existing(), existing({ partitionKey: CHIPS_KEY })]).length, 2);
});

test("writes into one session run one after the other", async () => {
  const target = fakeTarget();
  const batch = (label) => [cookie({ value: label }), cookie({ name: "token", value: label, domain: "api.acme.example" }), cookie({ name: "theme", value: label })];

  const [first, second, third] = await Promise.all([
    write(batch("first"), { ...target, now: NOW }),
    write(batch("second"), { ...target, now: NOW }),
    write(batch("third"), { ...target, now: NOW }),
  ]);

  assert.deepEqual([first.imported, second.imported, third.imported], [3, 3, 3]);
  assert.deepEqual([first.replaced, second.replaced, third.replaced], [0, 3, 3]);
  assert.deepEqual(target.sets().map((params) => params.value), ["first", "first", "first", "second", "second", "second", "third", "third", "third"]);
  // Each import reads the jar only once the one before it has written its last cookie.
  const kinds = target.events.filter(([kind]) => kind === "get" || kind === "Network.setCookie").map(([kind]) => kind);
  assert.deepEqual(kinds, ["get", ...Array(3).fill("Network.setCookie"), "get", ...Array(3).fill("Network.setCookie"), "get", ...Array(3).fill("Network.setCookie")]);
  assert.deepEqual(target.values(), ["session@.acme.example=third", "theme@.acme.example=third", "token@api.acme.example=third"]);
});

test("write waits for whoever holds the session's mutation lock", async () => {
  const target = fakeTarget();
  const release = await acquireCookieMutationLock(target.session);
  const pending = write([cookie()], { ...target, now: NOW });
  await tick();
  await tick();
  assert.deepEqual(target.events, []);
  release();
  assert.equal((await pending).imported, 1);
});

test("writes into different sessions do not wait for each other", async () => {
  let unblock;
  const blocked = new Promise((resolve) => {
    unblock = resolve;
  });
  const slow = fakeTarget([], { command: () => blocked });
  const fast = fakeTarget();

  const slowWrite = write([cookie()], { ...slow, now: NOW });
  assert.equal((await write([cookie()], { ...fast, now: NOW })).imported, 1);
  assert.equal(slow.jar.length, 0);
  unblock();
  assert.equal((await slowWrite).imported, 1);
});

test("the next write is admitted while an earlier flush is still pending", async () => {
  let releaseFlush;
  let flushes = 0;
  const flushing = new Promise((resolve) => {
    releaseFlush = resolve;
  });
  const target = fakeTarget([], {
    flush: () => {
      flushes += 1;
      return flushes === 1 ? flushing : undefined;
    },
  });

  const first = write([cookie({ value: "first" })], { ...target, now: NOW });
  while (flushes === 0) await tick();
  const second = await write([cookie({ value: "second" })], { ...target, now: NOW });
  assert.equal(second.imported, 1);
  assert.deepEqual(target.values(), ["session@.acme.example=second"]);
  releaseFlush();
  assert.equal((await first).imported, 1);
});

test("a hung command retires the debugger before the lock is released, fails the remaining rows at once, then admits a second import", async () => {
  const target = fakeTarget();
  const logged = [];
  let sent = 0;
  let retired = 0;
  let rejectedWhenNextHolderRan = -1;
  let nextHolder;
  const hung = () => {
    sent += 1;
    return new Promise(() => {});
  };

  const first = await write([cookie(), cookie({ name: "remaining" })], {
    session: target.session,
    sendCommand: hung,
    commandTimeoutMs: 20,
    onCommandTimeout: () => {
      retired += 1;
      // Whoever queues for the lock at this moment gets it only once both rows have failed.
      nextHolder = withCookieMutationLock(target.session, async () => {
        rejectedWhenNextHolderRan = logged.length;
      });
    },
    log: (message) => logged.push(message),
    now: NOW,
  });
  await nextHolder;

  assert.deepEqual(first, { imported: 0, failed: 2, domains: [], replaced: 0, excluded: {} });
  assert.equal(sent, 1);
  assert.equal(retired, 1);
  assert.equal(rejectedWhenNextHolderRan, 2);
  assert.match(logged[0], /Cookie session command timed out/);
  assert.match(logged[1], /retired after a command timed out/);
  assert.equal(target.count("flush"), 0);

  const second = await write([cookie({ value: "new" })], { ...target, now: NOW });
  assert.equal(second.imported, 1);
  assert.deepEqual(target.values(), ["session@.acme.example=new"]);
});

test("a bounded sender passes results and errors through, and stays retired after a timeout", async () => {
  const send = boundedCommandSender(async (method) => {
    if (method === "fail") throw new Error("nope");
    if (method === "hang") return new Promise(() => {});
    return { method };
  }, { timeoutMs: 20 });
  assert.deepEqual(await send("ok"), { method: "ok" });
  await assert.rejects(send("fail"), /nope/);
  await assert.rejects(send("hang"), /Cookie session command timed out/);
  await assert.rejects(send("ok"), /retired after a command timed out/);
});

test("the mutation lock runs its holders in order and releases after a failure", async () => {
  const owner = {};
  const order = [];
  const run = (label, fail = false) =>
    withCookieMutationLock(owner, async () => {
      order.push(`${label} in`);
      await tick();
      order.push(`${label} out`);
      if (fail) throw new Error(label);
    });
  const results = await Promise.allSettled([run("a"), run("b", true), run("c")]);
  assert.deepEqual(order, ["a in", "a out", "b in", "b out", "c in", "c out"]);
  assert.deepEqual(results.map((result) => result.status), ["fulfilled", "rejected", "fulfilled"]);
});
