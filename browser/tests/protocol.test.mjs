import test from "node:test";
import assert from "node:assert/strict";
import { parse, trusted, OPS, GUARDED, HUB_ONLY } from "../main/protocol.js";
import { areas, pageBox, TOOLBAR } from "../main/layout.js";
import { verdict, siteOf, allowedNow } from "../main/permissions.js";
import { noteOf, markersOf, labelOf, summaryOf } from "../main/notes.js";

test("a request is one JSON object with a known op and its fields", () => {
  assert.deepEqual(parse('{"op":"ping"}'), { request: { op: "ping" } });
  assert.match(parse("nope").error, /one JSON object/);
  assert.match(parse("[1]").error, /one JSON object/);
  assert.match(parse('{"op":"fly"}').error, /unknown op "fly"/);
  assert.match(parse('{"op":"open"}').error, /open needs owner/);
  assert.match(parse('{"op":"open","owner":"app:t3","worktree":"acme"}').error, /full path/);
  assert.match(parse('{"op":"open","owner":"app t3"}').error, /names a session/);
  assert.equal(parse('{"op":"open","owner":"app:t3","worktree":"/work/acme/api"}').request.owner, "app:t3");
  assert.equal(parse('{"op":"open","owner":"app:t3","worktree":""}').request.worktree, "", "a session in no folder is fine");
});

test("what changes a profile is guarded, and sending notes is the hub's alone", () => {
  for (const op of [...GUARDED, ...HUB_ONLY]) assert.ok(op in OPS, op);
  assert.equal(trusted({ token: "s3" }, "s3"), true);
  assert.equal(trusted({ token: "other" }, "s3"), false);
  assert.equal(trusted({}, ""), false, "no token set, nothing trusted");
});

test("the window is a toolbar over a page, with DevTools under it or at its right", () => {
  const size = { width: 1000, height: 800 };
  assert.deepEqual(areas(size).page, { x: 0, y: TOOLBAR, width: 1000, height: 800 - TOOLBAR });
  assert.equal(areas(size, { toolbarOpen: true }).toolbar.height, 800);
  const bottom = areas(size, { devtools: "bottom" });
  assert.equal(bottom.page.height + bottom.devtools.height, 800 - TOOLBAR);
  assert.equal(bottom.devtools.y, TOOLBAR + bottom.page.height);
  const right = areas(size, { devtools: "right" });
  assert.equal(right.page.width + right.devtools.width, 1000);
  assert.equal(areas({ width: 500, height: 800 }, { devtools: "right" }).devtools.x, 0, "a narrow window puts DevTools under the page");
});

test("a page at a viewport is scaled to fit and centred", () => {
  const area = { x: 0, y: TOOLBAR, width: 600, height: 700 };
  assert.deepEqual(pageBox(area, null), { ...area, scale: 1 });
  const phone = pageBox(area, { width: 375, height: 667, scale: 2, mobile: true });
  assert.deepEqual([phone.width, phone.height, phone.scale, phone.x], [375, 667, 1, 113]);
  const desktop = pageBox(area, { width: 1200, height: 800, scale: 1, mobile: false });
  assert.deepEqual([desktop.width, desktop.height, desktop.scale], [600, 400, 0.5]);
});

test("a page is granted little, asks the person for more, and is denied the rest", () => {
  assert.equal(verdict("fullscreen"), "grant");
  assert.deepEqual(verdict("loopback-network"), ["local-network"], "a site reaching this machine's servers asks first");
  assert.deepEqual(verdict("geolocation"), ["location"]);
  assert.deepEqual(verdict("media", ["audio", "video"]), ["camera", "microphone"]);
  assert.equal(verdict("media", []), "deny");
  assert.equal(verdict("hid"), "deny");
  assert.equal(siteOf("http://localhost:3000/orders?x=1"), "http://localhost:3000");
  assert.equal(siteOf("file:///tmp/x.html"), null);
  const answers = { get: (profile, site, thing) => (profile === "default" && site === "https://acme.example" && thing === "camera" ? true : null) };
  assert.equal(allowedNow(answers, "default", "media", "https://acme.example/call", ["video"]), true);
  assert.equal(allowedNow(answers, "default", "media", "https://acme.example/call", ["video", "audio"]), false);
  assert.equal(allowedNow(answers, "other", "media", "https://acme.example/call", ["video"]), false);
});

const payload = (name, rect) => ({
  page: { sanitizedUrl: "http://localhost:3000/orders", title: "acme web — orders", viewportWidth: 1200, viewportHeight: 800, scrollX: 0, scrollY: 0, devicePixelRatio: 2, capturedAt: "2026-01-05T10:00:00.000Z" },
  target: { tagName: "button", selector: "#find", textSnippet: name, htmlSnippet: `<button id="find">${name}</button>`, attributes: {}, accessibility: { role: "button", accessibleName: name, ariaLabel: null, ariaLabelledBy: null }, rectViewport: rect, rectPage: rect, computedStyles: {}, reactComponents: "<Orders> <FindButton>", sourceFile: "src/orders/FindButton.tsx:12:5" },
  nearbyText: [],
  ancestorPath: [],
  screenshot: { mimeType: "image/png", dataUrl: "data:image/png;base64,AAAA", width: 10, height: 10 },
});

test("a note keeps what the agent needs and drops the picture's bytes", () => {
  const rect = { x: 10, y: 20, width: 100, height: 30 };
  const note = noteOf({ id: "n1", tab: "t1", payload: payload("Find orders", rect), comment: "  Make it the primary colour  ", intent: "fix", priority: "nope", image: "/tmp/acme/find.png" }, new Date("2026-01-05T10:00:00Z"));
  assert.equal(note.comment, "Make it the primary colour");
  assert.equal(note.intent, "fix");
  assert.equal(note.priority, "important", "an unknown priority falls back");
  assert.equal(note.payload.screenshot, null);
  assert.deepEqual(note.imagePaths, ["/tmp/acme/find.png"]);
  assert.equal(labelOf(note.payload), "<FindButton> “Find orders”");
  const other = noteOf({ id: "n2", tab: "t2", payload: payload("Other", rect), comment: "x" });
  assert.deepEqual(markersOf([note, other], "t1"), [{ id: "n1", index: 0, label: "Fix · Important\nMake it the primary colour", rectPage: rect, rectViewport: rect, isFixed: false }]);
  assert.deepEqual(summaryOf(note), { id: "n1", label: "<FindButton> “Find orders”", comment: "Make it the primary colour", intent: "fix", priority: "important" });
});
