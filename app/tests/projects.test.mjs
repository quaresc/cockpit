import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { createContext, SourceTextModule, SyntheticModule } from "node:vm";
import { test } from "node:test";

/** One of the panels' modules on stand-ins for everything it imports: only its pure functions are called. */
async function load(name, stubs = {}) {
  const context = createContext({ console });
  const source = await readFile(new URL(`../mods/cockpit/${name}`, import.meta.url), "utf8");
  const mod = new SourceTextModule(source, { context, identifier: name });
  const stub = (spec) => {
    const values = stubs[spec] ?? {};
    const names = [...new Set([...Object.keys(values), ...[...source.matchAll(new RegExp(`import \\{([^}]*)\\} from "${spec.replace(/[./]/g, "\\$&")}"`, "g"))].flatMap((m) => m[1].split(",").map((s) => s.trim().split(/\s+as\s+/)[0]).filter(Boolean))])];
    const star = new RegExp(`import \\* as \\w+ from "${spec.replace(/[./]/g, "\\$&")}"`).test(source);
    return new SyntheticModule(star ? [...names, "default"] : names, function () {
      for (const n of names) this.setExport(n, values[n] ?? (() => null));
      if (star) this.setExport("default", values);
    }, { context, identifier: spec });
  };
  await mod.link(async (spec) => {
    const m = stub(spec);
    await m.link(() => {});
    return m;
  });
  await mod.evaluate();
  return mod.namespace;
}

test("Files marks each changed file and counts the changes under every folder up to its repository", async () => {
  const files = await load("files.js");
  const { files: marks, under } = files.marksOf([{ root: "/w/api", files: [{ path: "lib/quota.ex", status: "M" }, { path: "lib/plans/free.ex", status: "A" }] }]);
  assert.equal(marks.get("/w/api/lib/quota.ex"), "M");
  assert.equal(under.get("/w/api/lib"), 2);
  assert.equal(under.get("/w/api/lib/plans"), 1);
  assert.equal(under.get("/w/api"), 2, "the repository's own row counts them all");
  assert.equal(under.get("/w"), undefined, "nothing above the repository");
});

test("Files opens the way from the root down to each repository the agent works in, and one outside it on its own", async () => {
  const files = await load("files.js");
  const repos = [{ path: "/w/apps/api" }, { path: "/elsewhere/web" }];
  assert.deepEqual([...files.pathsTo("/w", repos)], ["/w/apps", "/w/apps/api", "/elsewhere/web"]);
  assert.deepEqual(files.outside("/w", repos).map((r) => r.path), ["/elsewhere/web"], "listed above the folder's tree");
});

test("Changes numbers a diff's lines from its hunks, removed lines on the old side", async () => {
  const changes = await load("changes.js");
  const rows = changes.diffRows(["@@ -10,2 +10,3 @@", " keep", "-old", "+new", "+more"]);
  assert.deepEqual(rows.map((r) => [r.kind, r.number ?? null]), [["@", null], [" ", 10], ["-", 11], ["+", 11], ["+", 12]]);
});

test("a status entry reads as one letter: conflicts, deletions, additions and new files, else a change", async () => {
  const projects = await load("projects.js");
  assert.deepEqual(["UU", " D", "??", "A ", " M", "AA"].map(projects.statusLetter), ["U", "D", "A", "A", "M", "U"]);
});

test("the Changes header names the repository shown and the branch checked out there", async () => {
  const changes = await load("changes.js");
  const c = Object.create(changes.default.prototype);
  c.repos = [{ path: "/w/api", name: "api", branch: "eng-412-quotas", count: 2 }, { path: "/w/web", name: "web", branch: null, count: 0 }];
  c.picked = null;
  assert.equal(c.title(), "api · eng-412-quotas");
  c.picked = "/w/web";
  assert.equal(c.title(), "web", "no branch known: the name alone");
  c.repos = [];
  assert.equal(c.title(), "");
});
