// The shell end to end, in a real Electron: its control socket, tabs per session, profiles, cookie import, sleeping
// tabs, design mode and what survives a restart. It needs Node 22 or later and the Electron binary to run:
//
//   COCKPIT_BROWSER_ELECTRON=/path/to/Electron node --test browser/tests/smoke.electron.mjs
//
// It runs on a home folder of its own under the temp folder, with Chromium's mock keychain, so no profile, tab or
// keychain of the person's is touched.

import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import net from "node:net";
import http from "node:http";
import path from "node:path";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";

const electron = process.env.COCKPIT_BROWSER_ELECTRON;
const here = path.dirname(fileURLToPath(import.meta.url));
const shellDir = path.join(here, "..");
const TOKEN = "smoke-token";
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/** Polls until `probe` answers something truthy, or fails after `ms`. */
async function until(probe, what, ms = 10_000) {
  const end = Date.now() + ms;
  for (;;) {
    const found = await probe().catch(() => null);
    if (found) return found;
    if (Date.now() > end) throw new Error(`timed out waiting for ${what}`);
    await sleep(100);
  }
}

// Clicks an element for the picker until the note's card opens: a click can land before the picker listens.
function pick(page, selector, card) {
  return until(async () => {
    await page.click(selector);
    return until(card, "the note's card", 2_000).catch(() => null);
  }, "the note's card", 20_000);
}

/** One request on the control socket and its answer. */
function ask(socket, request) {
  return new Promise((resolve, reject) => {
    const conn = net.createConnection(socket);
    let text = "";
    conn.on("connect", () => conn.write(`${JSON.stringify(request)}\n`));
    conn.on("data", (chunk) => {
      text += chunk;
      if (text.includes("\n")) (conn.destroy(), resolve(JSON.parse(text.split("\n")[0])));
    });
    conn.on("error", reject);
    setTimeout(() => (conn.destroy(), reject(new Error(`${request.op}: no answer`))), 15_000).unref();
  });
}

/** Every event the shell emits from now on, kept in a list. */
function follow(socket) {
  const events = [];
  const conn = net.createConnection(socket);
  let text = "";
  conn.on("connect", () => conn.write('{"op":"subscribe"}\n'));
  conn.on("data", (chunk) => {
    text += chunk;
    for (let at; (at = text.indexOf("\n")) >= 0; text = text.slice(at + 1)) {
      const line = JSON.parse(text.slice(0, at));
      if (line.event) events.push(line);
    }
  });
  conn.on("error", () => {});
  return { events, stop: () => conn.destroy() };
}

/** A debugging session on one page: `send(method, params)` and `evaluate(expression)`. */
async function debug(port, match) {
  const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
  const target = targets.find(match);
  if (!target) throw new Error("no such debugging target");
  const ws = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => ((ws.onopen = resolve), (ws.onerror = reject)));
  let serial = 0;
  const waiting = new Map();
  ws.onmessage = (message) => {
    const data = JSON.parse(message.data);
    if (data.id && waiting.has(data.id)) (waiting.get(data.id)(data), waiting.delete(data.id));
  };
  const send = (method, params = {}) =>
    new Promise((resolve, reject) => {
      const id = ++serial;
      waiting.set(id, (data) => (data.error ? reject(new Error(`${method}: ${data.error.message}`)) : resolve(data.result)));
      ws.send(JSON.stringify({ id, method, params }));
    });
  const evaluate = async (expression) => {
    const { result, exceptionDetails } = await send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true });
    if (exceptionDetails) throw new Error(exceptionDetails.text);
    return result.value;
  };
  /** A real press and release in the middle of an element. */
  const click = async (selector) => {
    const box = await evaluate(`(() => { const r = document.querySelector(${JSON.stringify(selector)}).getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })()`);
    await send("Input.dispatchMouseEvent", { type: "mouseMoved", x: box.x, y: box.y });
    await sleep(120);
    await send("Input.dispatchMouseEvent", { type: "mousePressed", x: box.x, y: box.y, button: "left", clickCount: 1 });
    await send("Input.dispatchMouseEvent", { type: "mouseReleased", x: box.x, y: box.y, button: "left", clickCount: 1 });
  };
  return { send, evaluate, click, close: () => ws.close(), id: target.id };
}

test("the shell, end to end", { skip: electron ? false : "set COCKPIT_BROWSER_ELECTRON to run it", timeout: 300_000 }, async (t) => {
  const home = fs.mkdtempSync(path.join(fs.realpathSync(os.tmpdir()), "ckb-"));
  const socket = path.join(home, "b.sock");
  // Two sessions, each with a browser of its own, working in two worktrees.
  const api = "app:t1";
  const web = "app:t2";
  const apiTree = path.join(home, "acme-api");
  const webTree = path.join(home, "acme-web");
  fs.mkdirSync(apiTree);
  fs.mkdirSync(webTree);
  // The acme pages over HTTP, so cookies have a site to belong to.
  const server = http.createServer((request, response) => {
    const name = request.url.split("?")[0].replace(/^\/+/, "") || "index.html";
    const file = path.join(here, "fixtures", "acme", path.basename(name));
    if (!fs.existsSync(file)) return response.writeHead(404).end();
    response.writeHead(200, { "content-type": name.endsWith(".js") ? "text/javascript" : "text/html; charset=utf-8" }).end(fs.readFileSync(file));
  });
  await new Promise((resolve) => server.listen(0, resolve));
  const site = `http://localhost:${server.address().port}`;
  let child = null;
  const start = async () => {
    const env = { ...process.env, HOME: home, COCKPIT_BROWSER_SOCKET: socket, COCKPIT_BROWSER_TOKEN: TOKEN, COCKPIT_BROWSER_OPTIONS: JSON.stringify({ dock: false, idleMinutes: 0 }), COCKPIT_BROWSER_MOCK_KEYCHAIN: "1" };
    delete env.ELECTRON_RUN_AS_NODE;
    child = spawn(electron, [shellDir], { env, stdio: "ignore" });
    return until(() => ask(socket, { op: "status" }), "the shell to start", 30_000);
  };
  const stop = async () => {
    await ask(socket, { op: "quit" }).catch(() => {});
    await until(async () => child.exitCode !== null || child.signalCode !== null, "the shell to quit");
  };
  t.after(async () => {
    child?.kill("SIGKILL");
    server.close();
    // What the shell said, for a failure that needs it: COCKPIT_BROWSER_SMOKE_LOG=1.
    if (process.env.COCKPIT_BROWSER_SMOKE_LOG) console.error(fs.readFileSync(path.join(home, ".cache", "cockpit", "browser", "browser.log"), "utf8"));
    fs.rmSync(home, { recursive: true, force: true });
  });
  const tabs = async (owner) => (await ask(socket, { op: "tabs.list", owner })).tabs;
  const page = async (status, owner, tab) => {
    const endpoint = await until(() => ask(socket, { op: "agent.endpoint", owner, tab }).then((e) => (e.target ? e : null)), "a debugging target");
    return debug(status.port, (target) => target.id === endpoint.target);
  };

  let status = await start();
  assert.equal(status.docked, false);
  assert.ok(status.port > 0, "Chromium's debugging port is open");

  await t.test("a page opens in its session's browser, once", async () => {
    const opened = await ask(socket, { op: "open", owner: api, worktree: apiTree, url: `${site}/index.html` });
    assert.equal(opened.reused, false);
    await until(async () => (await tabs(api))[0]?.title === "acme web — orders", "the page's title", 30_000);
    assert.equal((await ask(socket, { op: "open", owner: api, url: `${site}/index.html/` })).reused, true, "the same page comes forward");
    await ask(socket, { op: "open", owner: api, url: `${site}/second.html` });
    assert.deepEqual((await tabs(api)).map((tab) => tab.active), [false, true]);
    assert.deepEqual(await tabs(web), [], "another session has its own tabs");
    assert.match((await ask(socket, { op: "open", owner: api, url: "javascript:alert(1)" })).error, /does not open javascript: pages/);
  });

  await t.test("a page's own new window becomes a tab of the same browser", async () => {
    const second = await page(status, api);
    await until(async () => (await second.evaluate("document.title")) === "acme web — second", "the second page", 30_000);
    await until(async () => (await second.evaluate("document.readyState")) === "complete", "the second page to finish loading");
    await second.click("#popup");
    await until(async () => (await tabs(api)).length === 3, "the pop-up's tab");
    second.close();
    const listed = await tabs(api);
    await ask(socket, { op: "tab.close", owner: api, tab: listed.find((tab) => tab.active).id });
    assert.equal((await tabs(api)).length, 2);
  });

  await t.test("a viewport gives the page a phone's screen", async () => {
    const first = (await tabs(api))[0].id;
    await ask(socket, { op: "tab.activate", owner: api, tab: first });
    assert.equal((await ask(socket, { op: "tab.viewport", owner: api, preset: "mobile-m" })).viewport.width, 375);
    const orders = await page(status, api, first);
    await until(async () => (await orders.evaluate("devicePixelRatio")) === 2, "the phone's pixel ratio");
    assert.match((await ask(socket, { op: "tab.viewport", owner: api, width: 10, height: 10 })).error, /from 200 to 7680/);
    await ask(socket, { op: "tab.viewport", owner: api });
    orders.close();
  });

  await t.test("profiles keep their cookies apart, and take imported ones", async () => {
    const orders = await page(status, api);
    await orders.evaluate("document.cookie = 'cart=acme-api; path=/'; document.cookie");
    orders.close();
    const { profile } = await ask(socket, { op: "profile.create", name: "Staging", token: TOKEN });
    assert.equal(profile.name, "Staging");
    await ask(socket, { op: "open", owner: web, worktree: webTree, url: `${site}/index.html` });
    assert.equal((await ask(socket, { op: "profile.use", owner: web, profile: "Staging" })).profile.id, profile.id);
    const staged = await page(status, web);
    await until(async () => (await staged.evaluate("document.readyState")) === "complete", "the page in its new profile");
    assert.equal(await staged.evaluate("document.cookie"), "", "the other profile's cookie is not here");
    staged.close();
    const file = path.join(home, "cookies.json");
    fs.writeFileSync(file, JSON.stringify([{ domain: "localhost", name: "session", value: "maya", path: "/", secure: false, httpOnly: false, sameSite: "lax", expirationDate: Math.round(Date.now() / 1000) + 3600 }]));
    const imported = await ask(socket, { op: "profile.import", profile: "Staging", file, token: TOKEN });
    assert.equal(imported.imported, 1, JSON.stringify(imported));
    const after = await page(status, web);
    await until(async () => (await after.evaluate("document.cookie")).includes("session=maya"), "the imported cookie");
    after.close();
    const back = await page(status, api);
    assert.equal(await back.evaluate("document.cookie"), "cart=acme-api", "the first profile is as it was");
    back.close();
  });

  await t.test("a tab put to sleep wakes where it was", async () => {
    const [orders, second] = await tabs(api);
    await ask(socket, { op: "tab.activate", owner: api, tab: orders.id });
    const awake = await page(status, api, orders.id);
    await awake.evaluate("scrollTo(0, 420); scrollY");
    awake.close();
    await ask(socket, { op: "tab.activate", owner: api, tab: second.id });
    await ask(socket, { op: "tab.sleep", owner: api, tab: orders.id });
    assert.equal((await tabs(api))[0].asleep, true);
    await ask(socket, { op: "tab.activate", owner: api, tab: orders.id });
    const woken = await page(status, api, orders.id);
    await until(async () => (await woken.evaluate("scrollY")) === 420, "the scroll to come back");
    woken.close();
  });

  await t.test("a note on a picked element reaches the agent as Markdown", async () => {
    const { events, stop: unfollow } = follow(socket);
    const orders = await page(status, api);
    await orders.evaluate("scrollTo(0, 0); true");
    await ask(socket, { op: "grab.start", owner: api, mode: "annotate" });
    await until(() => orders.evaluate("Boolean(window.__cockpitGrab)"), "the picker");
    const toolbar = await debug(status.port, (target) => target.url.endsWith("/ui/toolbar.html"));
    await pick(orders, "#find", () => toolbar.evaluate("!document.getElementById('draft').hidden"));
    await toolbar.evaluate("document.getElementById('draft-comment').value = 'Use the primary colour'; document.getElementById('draft-intent').value = 'fix'; document.getElementById('draft').requestSubmit(); true");
    const notes = await until(async () => ((await ask(socket, { op: "notes.list", owner: api })).notes.length ? (await ask(socket, { op: "notes.list", owner: api })).notes : null), "the note");
    assert.equal(notes[0].payload.target.selector, "button#find");
    assert.equal(notes[0].intent, "fix");
    assert.ok(fs.existsSync(notes[0].imagePaths[0]), "the element's picture is on disk");
    assert.match((await ask(socket, { op: "notes.send", owner: api })).error, /the hub's to ask/);
    assert.equal((await ask(socket, { op: "notes.send", owner: api, token: TOKEN })).sent, 1);
    const sent = await until(async () => events.find((event) => event.event === "grab.send"), "the notes to be handed over");
    assert.equal(sent.owner, api, "they go to the session the browser belongs to, and nobody else");
    assert.match(sent.markdown, /## Design Feedback/);
    assert.match(sent.markdown, /\*\*Selector:\*\* `button#find`/);
    assert.match(sent.markdown, /\*\*Feedback:\*\* Use the primary colour/);
    assert.ok(sent.markdown.includes(notes[0].imagePaths[0]));
    await ask(socket, { op: "notes.sent", owner: api, ok: true });
    assert.deepEqual((await ask(socket, { op: "notes.list", owner: api })).notes, []);
    await ask(socket, { op: "grab.stop" });
    toolbar.close();
    orders.close();
    unfollow();
  });

  await t.test("the notes are listed in the toolbar, and a pick's one line goes straight to its session", async () => {
    const { events, stop: unfollow } = follow(socket);
    const orders = await page(status, api);
    const toolbar = await debug(status.port, (target) => target.url.endsWith("/ui/toolbar.html"));
    const card = () => toolbar.evaluate("!document.getElementById('draft').hidden");
    // Two notes, then the list the notes button opens: each with its number, and one removed from it.
    for (const [selector, comment] of [["#find", "Use the primary colour"], ["#title", "Shorter title"]]) {
      await ask(socket, { op: "grab.start", owner: api, mode: "annotate" });
      await until(() => orders.evaluate("Boolean(window.__cockpitGrab)"), "the picker");
      await pick(orders, selector, card);
      await toolbar.evaluate(`document.getElementById('draft-comment').value = ${JSON.stringify(comment)}; document.getElementById('draft').requestSubmit(); true`);
      await until(async () => !(await card()), "the card to close");
      await ask(socket, { op: "grab.stop" });
    }
    await until(() => toolbar.evaluate("document.getElementById('notes').textContent === '2'"), "the notes' count");
    await toolbar.evaluate("document.getElementById('annotate').click(); true");
    await until(() => toolbar.evaluate("!document.getElementById('recap').hidden && document.querySelectorAll('#recap-list li').length === 2"), "the list of notes");
    assert.match(await toolbar.evaluate("document.getElementById('recap-list').textContent"), /1Use the primary colour.*2Shorter title/s);
    assert.equal(await orders.evaluate("document.querySelectorAll('[data-cockpit-annotation-overlay]').length"), 1, "the page badges its noted elements");
    await toolbar.evaluate("document.querySelector('#recap-list li .x').click(); true");
    await until(async () => (await ask(socket, { op: "notes.list", owner: api })).notes.length === 1, "a note to go");
    await ask(socket, { op: "notes.clear", owner: api });
    // A card left open when the picker stops goes with it.
    await ask(socket, { op: "grab.start", owner: api, mode: "copy" });
    await until(() => orders.evaluate("Boolean(window.__cockpitGrab)"), "the picker");
    await pick(orders, "#find", card);
    await ask(socket, { op: "grab.stop" });
    await until(async () => !(await card()), "the card to go with the picker");
    // A pick: one element, one line, sent at once to the session the browser belongs to.
    await ask(socket, { op: "grab.start", owner: api, mode: "copy" });
    await until(() => orders.evaluate("Boolean(window.__cockpitGrab)"), "the picker");
    await pick(orders, "#find", card);
    assert.equal(await toolbar.evaluate("document.getElementById('draft-save').textContent"), "Send");
    // Enter sends it; with Shift it stays a new line in the note.
    const enter = (modifiers) => toolbar.send("Input.dispatchKeyEvent", { type: "rawKeyDown", key: "Enter", code: "Enter", windowsVirtualKeyCode: 13, modifiers });
    await toolbar.evaluate("document.getElementById('draft-comment').value = 'Why is this green?'; document.getElementById('draft-comment').focus(); true");
    await enter(8);
    assert.equal(await card(), true, "Shift+Enter leaves the card open");
    await enter(0);
    const sent = await until(async () => events.find((event) => event.event === "grab.send"), "the pick to be handed over");
    assert.equal(sent.owner, api);
    assert.match(sent.markdown, /\*\*Feedback:\*\* Why is this green\?/);
    assert.deepEqual((await ask(socket, { op: "notes.list", owner: api })).notes, [], "a pick is not kept as a note");
    await ask(socket, { op: "notes.sent", owner: api, ok: true });
    toolbar.close();
    orders.close();
    unfollow();
  });

  await t.test("tabs come back after a restart, asleep until shown", async () => {
    const before = await tabs(api);
    await stop();
    status = await start();
    const after = await tabs(api);
    assert.deepEqual(after.map((tab) => [tab.id, tab.url]), before.map((tab) => [tab.id, tab.url]));
    assert.ok(after.every((tab) => tab.asleep), "nothing loads until its session is in front");
    assert.equal((await ask(socket, { op: "profiles.list" })).profiles.some((p) => p.name === "Staging"), true);
  });

  await t.test("a session that ended, or a worktree that is removed, leaves nothing behind", async () => {
    const kept = () => fs.readdirSync(path.join(home, ".config", "cockpit", "browser", "browsers")).length;
    assert.equal(kept(), 2);
    await ask(socket, { op: "owner.forget", owner: api });
    assert.deepEqual(await tabs(api), []);
    assert.equal(kept(), 1, "only the other session's file is left");
    await ask(socket, { op: "worktree.forget", worktree: webTree });
    assert.equal(kept(), 0, "the session that worked in the removed worktree lost its browser too");
    assert.deepEqual((await ask(socket, { op: "owners.list" })).owners, []);
  });

  await stop();
});
