// ⌘V with an image in the chat's box: the clipboard's image written to a new private file by the system's own tool
// (stand-ins for osascript and xclip here), then attached as the + menu and a drop attach a file.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, chmodSync, symlinkSync, statSync, readFileSync, readdirSync, rmSync, utimesSync } from "node:fs";
import { readFile, readdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createContext, SourceTextModule, SyntheticModule } from "node:vm";
import { test, after } from "node:test";

const MODS = new URL("../mods/cockpit/", import.meta.url);

// The folders the tests make, gone once they ran.
const made = [];
const scratch = (prefix) => (made.push(mkdtempSync(join(tmpdir(), prefix))), made.at(-1));
after(() => made.forEach((dir) => rmSync(dir, { recursive: true, force: true })));

/** The mod's modules on a stand-in Herdr whose `exec` the test answers. */
async function load(exec) {
  const element = () => {
    const made = [];
    const el = new Proxy({}, { get: (_t, name) => (name === "made" ? made : (...a) => (made.push([name, ...a]), el)) });
    return el;
  };
  const imports = {
    herdr: { exec, shared: () => null, setShared: () => null, run: () => null, notify: () => null, layout: () => null, focused: () => null, keys: () => [], settings: () => ({}), theme: () => ({}) },
    "gpui-kit": { div: element, svg: element },
    "gpui-base": { h_flex: element, v_flex: element },
    "./hub.js": { subscribe: () => ({ stop() {} }), sleep: async () => {}, act: async () => ({}), view: async () => null, remoteMachine: () => null, openUrl: () => {} },
  };
  const context = createContext({ console });
  const sources = new Map(), modules = new Map();
  for (const name of await readdir(MODS)) {
    if (!name.endsWith(".js")) continue;
    const text = await readFile(new URL(name, MODS), "utf8");
    sources.set(`./${name}`, text);
    for (const match of text.matchAll(/import\s*\{([^}]+)\}\s*from\s*"(herdr|gpui-kit|gpui-base)"/g)) {
      const exports = (imports[match[2]] ??= {});
      for (const word of match[1].split(",")) {
        const key = word.trim().split(/\s+as\s+/)[0];
        exports[key] ??= () => { throw new Error(`unexpected native call: ${key}`); };
      }
    }
  }
  const link = (name) => {
    if (modules.has(name)) return modules.get(name);
    const native = imports[name];
    const module = native
      ? new SyntheticModule(Object.keys(native), function () {
        for (const [key, value] of Object.entries(native)) this.setExport(key, value);
      }, { context })
      : new SourceTextModule(sources.get(name), { context, identifier: name });
    modules.set(name, module);
    return module;
  };
  const module = new SourceTextModule('export * as paste from "./paste.js"; export { default as Chat } from "./chat.js";', { context });
  await module.link(link);
  await module.evaluate();
  return module.namespace;
}

/** A folder of tools: the few the script needs, and stand-ins for the clipboard's. */
function tools(fakes) {
  const bin = scratch("paste-bin-");
  for (const tool of ["mktemp", "rm", "chmod", "grep", "find", "id", "wc"]) {
    const real = spawnSync("/bin/sh", ["-c", `command -v ${tool}`], { encoding: "utf8" }).stdout.trim();
    symlinkSync(real, join(bin, tool));
  }
  for (const [name, body] of Object.entries(fakes)) {
    writeFileSync(join(bin, name), `#!/bin/sh\n${body}\n`);
    chmodSync(join(bin, name), 0o755);
  }
  return bin;
}

// macOS's osascript: `clipboard info` lists the clipboard's types; the script run writes the image to its argument.
const OSASCRIPT = `case "$*" in
  *"clipboard info"*) printf '%s' "$FAKE_CLIP" ;;
  *) [ -z "$FAKE_FAIL" ] || { echo "execution error: Can't make the clipboard into type PNG picture. (-1700)" >&2; exit 1; }
     eval last=\\\${$#}; printf 'PNG' > "$last" ;;
esac`;
// X11's xclip: TARGETS lists the clipboard's types, image/png gives the image.
const XCLIP = `case "$*" in *TARGETS*) printf '%s\\n' $FAKE_TARGETS ;; *image/png*) printf 'PNG' ;; esac`;

function runScript(script, bin, env = {}, tmp = scratch("paste-tmp-")) {
  const r = spawnSync("/bin/sh", ["-c", script], { encoding: "utf8", env: { PATH: bin, TMPDIR: tmp, ...env } });
  return { code: r.status, stdout: r.stdout, stderr: r.stderr, tmp };
}

test("the clipboard's image lands in a new file only its owner reads, a new one each paste", async () => {
  const { paste } = await load(async () => null);
  const bin = tools({ osascript: OSASCRIPT });
  const first = runScript(paste.PASTE_IMAGE, bin, { FAKE_CLIP: "«class PNGf», 2048, TIFF picture, 9000" });
  const got = paste.pasted(first);
  assert.match(got.path ?? "", /\/pasted-[A-Za-z0-9]{6}\.png$/, JSON.stringify(first));
  assert.ok(got.path.startsWith(first.tmp), "in the person's own temporary folder");
  assert.equal(readFileSync(got.path, "utf8"), "PNG");
  assert.equal(statSync(got.path).mode & 0o777, 0o600, "the file is private");
  assert.equal(statSync(join(got.path, "..")).mode & 0o777, 0o700, "and so is its folder");
  const again = paste.pasted(runScript(paste.PASTE_IMAGE, bin, { FAKE_CLIP: "«class PNGf», 2048" }));
  assert.notEqual(again.path.split("/").pop(), got.path.split("/").pop(), "each paste's chip has a name of its own");
});

test("a clipboard with no image says so with its own exit, and leaves nothing behind", async () => {
  const { paste } = await load(async () => null);
  const r = runScript(paste.PASTE_IMAGE, tools({ osascript: OSASCRIPT }), { FAKE_CLIP: "«class utf8», 12" });
  assert.equal(r.code, 2);
  assert.deepEqual({ ...paste.pasted(r) }, { none: true });
  assert.deepEqual(readdirSync(r.tmp), []);
});

test("an image that cannot be written is an error with the tool's words, and leaves nothing behind", async () => {
  const { paste } = await load(async () => null);
  const r = runScript(paste.PASTE_IMAGE, tools({ osascript: OSASCRIPT }), { FAKE_CLIP: "TIFF picture, 9000", FAKE_FAIL: "1" });
  const got = paste.pasted(r);
  assert.equal(got.error, "the clipboard's image could not be read");
  assert.match(got.detail, /-1700/);
  assert.deepEqual(readdirSync(r.tmp), []);
});

test("on Linux, xclip reads it; a machine with no clipboard tool says what to do instead", async () => {
  const { paste } = await load(async () => null);
  const x = tools({ xclip: XCLIP });
  assert.ok(paste.pasted(runScript(paste.PASTE_IMAGE, x, { FAKE_TARGETS: "TARGETS image/png text/plain" })).path);
  assert.equal(runScript(paste.PASTE_IMAGE, x, { FAKE_TARGETS: "TARGETS UTF8_STRING" }).code, 2);
  const none = runScript(paste.PASTE_IMAGE, tools({}));
  assert.equal(none.code, 3);
  assert.match(paste.pasted(none).error, /drop the file on the box/);
});

test("⌘V with an image attaches it as the + menu does; with none it does nothing; a failure says so above the box", async () => {
  let answer = null;
  const runs = [];
  const { Chat } = await load(async (cmd, args) => (runs.push([cmd, args[0]]), answer));
  const chat = Object.create(Chat.prototype);
  Object.assign(chat, { files: [], pasteNote: null });
  const work = [];
  const cx = { notify() {}, spawn: (f) => work.push(f(cx)) };
  const paste = async (asked) => (chat.pasteFromClipboard(cx, asked), await Promise.all(work.splice(0)));
  answer = { code: 0, stdout: "/tmp/cockpit-paste.ab12/pasted.png\n", stderr: "" };
  await paste(false);
  assert.deepEqual([...chat.files], ["/tmp/cockpit-paste.ab12/pasted.png"], "the same file chip a drop gives, sent the same way");
  assert.deepEqual(runs.at(-1), ["sh", "-c"]);
  answer = { code: 2, stdout: "", stderr: "" };
  await paste(false);
  assert.equal(chat.pasteNote, null, "a paste with neither text nor image does nothing");
  assert.equal(chat.files.length, 1);
  await paste(true);
  assert.equal(chat.pasteNote.why, "the clipboard holds no image", "asked from the + menu, it says so");
  answer = { code: 1, stdout: "", stderr: "execution error (-1700)" };
  await paste(false);
  assert.deepEqual({ ...chat.pasteNote }, { why: "the clipboard's image could not be read", detail: "execution error (-1700)" });
  // The + menu's clipboard item goes the same way.
  answer = { code: 0, stdout: "/tmp/cockpit-paste.cd34/pasted.png\n", stderr: "" };
  chat.pick("clipboard", cx);
  await Promise.all(work.splice(0));
  assert.equal(chat.files.length, 2);
  assert.equal(chat.pasteNote, null, "an image attached puts the note away");
});

test("a delayed clipboard result stays with the original chat's draft after navigating elsewhere", async () => {
  for (const answer of [{ code: 0, stdout: "/tmp/acme-pasted.png" }, { code: 1, stderr: "clipboard unavailable" }]) {
    let finish;
    const { Chat } = await load(() => new Promise((resolve) => { finish = resolve; }));
    const chat = Object.create(Chat.prototype);
    Object.assign(chat, { target: "t1", files: [], drafts: new Map(), pasteNote: null });
    const work = [];
    const cx = { notify() {}, spawn: (f) => work.push(f(cx)) };
    chat.pasteFromClipboard(cx, true);
    chat.drafts.set("t1", { text: "Inspect these daily quotas", files: [] });
    chat.target = "t2";
    chat.files = ["/acme/api/README.md"];
    finish(answer);
    await Promise.all(work);
    assert.deepEqual([...chat.files], ["/acme/api/README.md"], "the newly viewed chat receives no attachment");
    assert.equal(chat.pasteNote, null, "the other chat receives no stale error note");
    assert.deepEqual([...chat.drafts.get("t1").files], answer.code === 0 ? ["/tmp/acme-pasted.png"] : []);
    assert.equal(chat.drafts.get("t1").text, "Inspect these daily quotas");
  }
});

test("a paste deletes this person's pastes older than a day, and keeps the recent ones", async () => {
  const { paste } = await load(async () => null);
  const tmp = scratch("paste-tmp-");
  for (const name of ["cockpit-paste.old111", "cockpit-paste.new222", "acme-keep.old333"]) mkdirSync(join(tmp, name));
  writeFileSync(join(tmp, "cockpit-paste.old111", "pasted-old111.png"), "PNG");
  const twoDaysAgo = new Date(Date.now() - 2 * 86_400_000);
  for (const name of ["cockpit-paste.old111", "acme-keep.old333"]) utimesSync(join(tmp, name), twoDaysAgo, twoDaysAgo);
  const r = runScript(paste.PASTE_IMAGE, tools({ osascript: OSASCRIPT }), { FAKE_CLIP: "«class PNGf», 2048" }, tmp);
  assert.ok(paste.pasted(r).path, JSON.stringify(r));
  const left = readdirSync(tmp).sort();
  assert.ok(!left.includes("cockpit-paste.old111"), "a day-old paste is gone");
  assert.ok(left.includes("cockpit-paste.new222") && left.includes("acme-keep.old333"), "a recent paste and anything else stay");
  assert.equal(left.filter((n) => n.startsWith("cockpit-paste.")).length, 2, "and the new one is there");
});

test("an image over the cap is refused with what to do instead, and leaves nothing behind", async () => {
  const { paste } = await load(async () => null);
  const small = paste.PASTE_IMAGE.replace(String(paste.MAX_IMAGE), "2");
  const r = runScript(small, tools({ osascript: OSASCRIPT }), { FAKE_CLIP: "«class PNGf», 2048" });
  assert.equal(r.code, 4);
  assert.match(paste.pasted(r).error, /over 20 MB: save it and drop the file on the box/);
  assert.deepEqual(readdirSync(r.tmp), []);
});

test("a held ⌘V pastes once; let go and pressed again, it pastes again", async () => {
  const runs = [];
  const { Chat } = await load(async () => (runs.push(1), { code: 0, stdout: `/tmp/cockpit-paste.ab${runs.length}/pasted.png\n` }));
  const chat = Object.create(Chat.prototype);
  Object.assign(chat, { files: [], pasteNote: null, down: new Map() });
  const work = [];
  const cx = { notify() {}, spawn: (f) => work.push(f(cx)) };
  for (let i = 0; i < 5; i++) chat.pasteKey(cx);
  await Promise.all(work.splice(0));
  assert.equal(runs.length, 1, "the repeats of a held ⌘V paste nothing more");
  chat.released({ key: "v" });
  chat.pasteKey(cx);
  await Promise.all(work.splice(0));
  assert.equal(runs.length, 2);
  assert.equal(chat.files.length, 2);
});
