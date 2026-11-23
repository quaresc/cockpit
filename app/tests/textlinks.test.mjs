import assert from "node:assert/strict";
import { test } from "node:test";
import { linkify, imagesIn, pathOf, fileAndLine, expand } from "../mods/cockpit/textlinks.js";

test("bare paths and URLs become links, their sentence's punctuation left outside", () => {
  assert.equal(
    linkify("See /tmp/acme/shot.png and ~/code/api/lib/quota.ex:12."),
    "See [/tmp/acme/shot.png](file:///tmp/acme/shot.png) and [~/code/api/lib/quota.ex:12](file://~/code/api/lib/quota.ex:12).",
  );
  assert.equal(
    linkify("Open https://github.com/acme/api/pull/12, then (https://linear.app/acme/issue/ENG-412)."),
    "Open [https://github.com/acme/api/pull/12](https://github.com/acme/api/pull/12), then ([https://linear.app/acme/issue/ENG-412](https://linear.app/acme/issue/ENG-412)).",
  );
});

test("a code span holding only a path or a URL is linked, and what Markdown already links stays as written", () => {
  assert.equal(linkify("In `~/code/api` run `mix test`."), "In [`~/code/api`](file://~/code/api) run `mix test`.");
  const kept = "Read [the docs](https://acme.example/a/b), <https://acme.example/c> and `/tmp/a b`.";
  assert.equal(linkify(kept), kept);
});

test("a slash command, a relative path and a URL's own path are no path", () => {
  const plain = "Type /plan, then edit lib/quota.ex.";
  assert.equal(linkify(plain), plain);
  assert.equal(linkify("https://acme.example/a/b"), "[https://acme.example/a/b](https://acme.example/a/b)");
});

test("the images a text names are found once each, under home, never a web picture", () => {
  const text = "Before ~/shots/before.png, after /tmp/acme/after.webp and /tmp/acme/after.webp again; https://acme.example/x/y.png is a page.";
  assert.deepEqual(imagesIn(text, "/Users/me"), ["/Users/me/shots/before.png", "/tmp/acme/after.webp"]);
  assert.deepEqual(imagesIn("a /tmp/1.png b /tmp/2.png c /tmp/3.png", "/h", 2), ["/tmp/1.png", "/tmp/2.png"]);
});

test("a clicked path is read back whole, with its line", () => {
  assert.equal(pathOf("file:///tmp/acme%20notes/plan.md"), "/tmp/acme notes/plan.md");
  assert.equal(pathOf("https://acme.example"), null);
  assert.deepEqual(fileAndLine("/r/api/lib/quota.ex:42"), { file: "/r/api/lib/quota.ex", line: 42 });
  assert.deepEqual(fileAndLine("/r/api/README.md"), { file: "/r/api/README.md", line: null });
  assert.equal(expand("~/code/api", "/Users/me"), "/Users/me/code/api");
});
