import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { createContext, SourceTextModule, SyntheticModule } from "node:vm";
import { test } from "node:test";

/** hub.js loaded as `herdr-app check` loads it, or as a window does: `calls` records every run, read and connection. */
async function load(checking) {
  const calls = [];
  const herdr = {
    appHome: () => "/herdr",
    window: () => ({ platform: "macos" }),
    run: (...args) => calls.push(["run", ...args]),
    readTextFile: async (path) => (calls.push(["read", path]), path.endsWith("cockpit.json") ? JSON.stringify({ bin: "/bin/cockpit", home: "/home", cache: "/cache" }) : null),
    exec: async (...args) => (calls.push(["exec", ...args]), { code: 1, stdout: "", stderr: "no hub here" }),
    ...(checking === undefined ? {} : { checking: () => checking }),
  };
  const imports = { herdr, net: { default: { connect: async (...args) => (calls.push(["connect", ...args]), new Promise(() => {})) } } };
  const context = createContext({ console });
  const module = new SourceTextModule(await readFile(new URL("../mods/cockpit/hub.js", import.meta.url), "utf8"), { context });
  await module.link((name) => new SyntheticModule(Object.keys(imports[name]), function () {
    for (const [key, value] of Object.entries(imports[name])) this.setExport(key, value);
  }, { context }));
  await module.evaluate();
  return { hub: module.namespace, calls };
}

const cx = { spawn: () => {}, timer: { after: () => {}, every: () => ({ stop() {} }) }, notify: () => {} };

test("under herdr-app check the mod starts, asks and follows nothing", async () => {
  const { hub, calls } = await load(true);
  assert.equal(hub.checking(), true);
  await assert.rejects(hub.cockpit(["hub", "start"]), /herdr-app check/);
  await assert.rejects(hub.startHub(), /herdr-app check/);
  await assert.rejects(hub.view("inbox"), /herdr-app check/);
  const sub = hub.subscribe(cx, { views: ["agents"] }, () => assert.fail("no message under a check"));
  assert.equal(sub.stopped, true);
  sub.stop();
  assert.deepEqual(calls, []);
});

test("a window, and a Herdr that has no checking(), reach the hub as before", async () => {
  for (const checking of [false, undefined]) {
    const { hub, calls } = await load(checking);
    assert.equal(hub.checking(), false);
    await hub.cockpit(["hub", "ping"]);
    assert.ok(calls.some((c) => c[0] === "exec"), "the binary runs");
  }
});
