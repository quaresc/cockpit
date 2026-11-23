import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { createContext, SourceTextModule, SyntheticModule } from "node:vm";
import { test } from "node:test";

/** link.js with config.js, on stubbed Herdr natives: `files` is what readTextFile finds, `calls` every exec. */
async function load(files, exitCode = 0) {
  const calls = [];
  const herdr = {
    appHome: () => "/herdr",
    readTextFile: async (path) => files[path] ?? null,
    exec: async (...args) => (calls.push(args), { code: exitCode, stdout: "", stderr: "" }),
  };
  const context = createContext({ console });
  const modules = new Map();
  const source = async (name) => {
    if (modules.has(name)) return modules.get(name);
    const m = new SourceTextModule(await readFile(new URL(`../mods/cockpit/${name}`, import.meta.url), "utf8"), { context, identifier: name });
    modules.set(name, m);
    return m;
  };
  const link = async (specifier) =>
    specifier === "herdr"
      ? new SyntheticModule(Object.keys(herdr), function () {
          for (const [key, value] of Object.entries(herdr)) this.setExport(key, value);
        }, { context })
      : source(specifier.replace("./", ""));
  const main = await source("link.js");
  await main.link(link);
  await main.evaluate();
  return { link: main.namespace, calls };
}

const INSTALL = JSON.stringify({ bin: "/bin/cockpit", home: "/home/me", cache: "/home/me/.cache/cockpit", env: {} });
const click = (url, source = {}) => ({ link: { kind: "url", raw: url, url }, source: { terminal: "t7", machine: null, cwd: "/work", pane: "p1", tab: "tb", space: "sp", ...source } });

test("a web link opens in the person's browser unless links.open asks for the side browser", async () => {
  const plain = await load({ "/herdr/cockpit.json": INSTALL });
  assert.equal(await plain.link.handleLink(click("https://github.com/acme/api/pull/7")), "pass");
  assert.deepEqual(plain.calls, [], "Herdr opens it, cockpit runs nothing");

  const side = await load({ "/herdr/cockpit.json": INSTALL, "/home/me/.config/cockpit/config.json": JSON.stringify({ links: { open: "side" } }) });
  assert.equal(await side.link.handleLink(click("http://localhost:4000/admin")), "handled");
  assert.deepEqual(JSON.parse(JSON.stringify(side.calls)), [["env", ["HOME=/home/me", "HERDR_APP_HOME=/herdr", "COCKPIT_FRONTEND=app", "HC_TERMINAL_ID=t7", "/bin/cockpit", "browse", "http://localhost:4000/admin"], { timeout: 25_000 }]]);
});

test("the cache's config wins, and a failed browse hands the click back to Herdr", async () => {
  const files = {
    "/herdr/cockpit.json": INSTALL,
    "/home/me/.cache/cockpit/config.json": JSON.stringify({ links: { open: "side" } }),
    "/home/me/.config/cockpit/config.json": JSON.stringify({ links: { open: "browser" } }),
  };
  const failing = await load(files, 1);
  assert.equal(await failing.link.handleLink(click("https://acme.example")), "pass");
  assert.equal(failing.calls.length, 1);
});

test("files, other machines, remote installs and odd links stay Herdr's", async () => {
  const config = { links: { open: "side" } };
  const install = JSON.parse(INSTALL);
  const { link } = await load({});
  assert.equal(link.routeOf(click("https://acme.example"), config, install), "side");
  assert.equal(link.routeOf({ link: { kind: "file", raw: "/a.png", path: "/a.png" }, source: { terminal: "t7" } }, config, install), null);
  assert.equal(link.routeOf(click("https://acme.example", { machine: "box" }), config, install), null);
  assert.equal(link.routeOf(click("https://acme.example"), config, { ...install, machine: "runner" }), null);
  assert.equal(link.routeOf(click("vscode://file/a.rs"), config, install), null);
  assert.equal(link.routeOf(click("https://acme.example"), null, install), null, "no config: the browser");
  assert.equal(link.routeOf(click("https://acme.example"), config, null), null, "cockpit not installed");
});
