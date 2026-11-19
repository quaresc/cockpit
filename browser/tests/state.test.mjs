import test from "node:test";
import assert from "node:assert/strict";
import { emptyBrowser, withTab, withoutTab, withActive, tabShowing, typedUrl, allowedUrl, saved, restored, pageKey, withTabMoved } from "../main/state.js";
import { toSleep, keptAwake, idleMsOf } from "../main/discard.js";
import { emptyRegistry, created, managed, removed, renamed, chosen, ownerFor, partitionOf, ownedBy, DEFAULT } from "../main/profiles.js";
import { emptyHistory, visited, suggest, withoutProfile } from "../main/history.js";
import { viewportOf, fitScale, emulation, PRESETS } from "../main/viewport.js";

const tab = (id, url) => ({ id, url, title: id });

test("tabs open at the end and become active, unless opened in the background", () => {
  let wt = withTab(emptyBrowser("app:t3", "/work/acme/api"), tab("a", "http://localhost:3000/"));
  wt = withTab(wt, tab("b", "http://localhost:3000/orders"));
  assert.equal(wt.active, "b");
  wt = withTab(wt, tab("c", "http://localhost:3000/users"), { background: true, after: "a" });
  assert.deepEqual(wt.tabs.map((t) => t.id), ["a", "c", "b"]);
  assert.equal(wt.active, "b");
});

test("closing the active tab shows its right neighbour, else its left one, and remembers the page", () => {
  let wt = ["a", "b", "c"].reduce((w, id) => withTab(w, tab(id, `http://localhost:3000/${id}`)), emptyBrowser("app:t3", "/work/acme/api"));
  wt = withoutTab(withActive(wt, "b"), "b");
  assert.equal(wt.active, "c");
  wt = withoutTab(wt, "c");
  assert.equal(wt.active, "a");
  assert.deepEqual(wt.closed.map((c) => c.url), ["http://localhost:3000/c", "http://localhost:3000/b"]);
  assert.equal(withoutTab(wt, "a").active, null);
});

test("a page already open is found whatever its trailing slash", () => {
  const wt = withTab(emptyBrowser("app:t3", "/work/acme/api"), tab("a", "http://localhost:3000/orders/"));
  assert.equal(tabShowing(wt, "http://localhost:3000/orders")?.id, "a");
  assert.equal(tabShowing(wt, "http://localhost:3000/users"), null);
  assert.equal(pageKey("/tmp/page.html"), "file:///tmp/page.html");
});

test("what is typed becomes a URL, a local file, or a search", () => {
  assert.equal(typedUrl("localhost:3000/orders"), "http://localhost:3000/orders");
  assert.equal(typedUrl("acme.example/docs"), "https://acme.example/docs");
  assert.equal(typedUrl("https://acme.example"), "https://acme.example");
  assert.equal(typedUrl("/tmp/page.html"), "file:///tmp/page.html");
  assert.equal(typedUrl("how to centre a div"), "https://duckduckgo.com/?q=how%20to%20centre%20a%20div");
  assert.equal(typedUrl("? acme.example"), "https://duckduckgo.com/?q=acme.example");
  assert.equal(typedUrl("javascript:alert(1)"), "javascript:alert(1)", "a scheme the shell refuses is not searched for");
  assert.equal(typedUrl("acme.example:8080/orders"), "https://acme.example:8080/orders");
  assert.equal(allowedUrl("javascript:alert(1)"), false);
  assert.equal(allowedUrl("chrome://settings"), false);
  assert.equal(allowedUrl("file:///tmp/page.html"), true);
});

test("a browser comes back from disk with its tabs asleep and bad ones dropped", () => {
  let wt = withTab(emptyBrowser("app:t3", "/work/acme/api", "/work/acme"), { ...tab("a", "http://localhost:3000/"), scroll: { x: 0, y: 420 }, viewport: { preset: "tablet" } });
  wt = withTab(wt, tab("b", "javascript:alert(1)"));
  const back = restored(JSON.parse(JSON.stringify(saved(wt))), "app:t3");
  assert.deepEqual(back.tabs.map((t) => [t.id, t.asleep, t.scroll?.y]), [["a", true, 420]]);
  assert.equal(back.active, "a");
  assert.deepEqual([back.owner, back.root, back.repo], ["app:t3", "/work/acme/api", "/work/acme"], "it keeps whose it is and where that session works");
  assert.deepEqual(restored(null, "app:t7").tabs, []);
});

test("a tab is moved within the row", () => {
  const wt = ["a", "b", "c"].reduce((w, id) => withTab(w, tab(id, `http://localhost:3000/${id}`)), emptyBrowser("app:t3"));
  assert.deepEqual(withTabMoved(wt, "c", 0).tabs.map((t) => t.id), ["c", "a", "b"]);
});

test("tabs out of view sleep after the delay unless something keeps them awake", () => {
  const now = 10_000_000;
  const tabs = [
    { id: "old", hiddenAt: now - 3_700_000 },
    { id: "audio", hiddenAt: now - 3_700_000, audible: true },
    { id: "tools", hiddenAt: now - 3_700_000, devtools: true },
    { id: "edits", hiddenAt: now - 3_700_000, edited: true },
    { id: "recent", hiddenAt: now - 60_000 },
    { id: "shown", shown: true, hiddenAt: now - 3_700_000 },
    { id: "asleep", asleep: true, hiddenAt: now - 3_700_000 },
    { id: "agent", hiddenAt: now - 3_700_000, agentAt: now - 5_000 },
  ];
  assert.deepEqual(toSleep(tabs, now, idleMsOf(60)), ["old"]);
  assert.deepEqual(toSleep(tabs, now, idleMsOf(0)), []);
  assert.equal(keptAwake({ audible: true }), "audio");
});

test("a scope gives a worktree its managed profile, made once", () => {
  const place = { root: "/work/acme/api-eng-412", repo: "/work/acme/api" };
  assert.equal(ownerFor("shared", place), null);
  assert.equal(ownerFor("worktree", { root: "", repo: "" }), null, "a session in no folder shares the default profile");
  let [registry, profile] = managed(emptyRegistry(), "shared", place);
  assert.equal(profile.id, DEFAULT);
  [registry, profile] = managed(registry, "repo", place, 1);
  assert.equal(profile.owner, "repo:/work/acme/api");
  assert.equal(profile.name, "api");
  const [again, same] = managed(registry, "repo", { root: "/work/acme/api-eng-413", repo: "/work/acme/api" }, 2);
  assert.equal(same.id, profile.id);
  assert.equal(again.profiles.length, 2);
  const [, own] = managed(registry, "worktree", place, 3);
  assert.equal(own.owner, "worktree:/work/acme/api-eng-412");
  assert.equal(partitionOf(own.id), `persist:cockpit-${own.id}`);
});

test("a person's profile is named once, picked for a worktree, renamed and removed", () => {
  let [registry, work] = created(emptyRegistry(), "Work", 5);
  assert.throws(() => created(registry, "work"), /exists already/);
  assert.throws(() => created(registry, "  "), /needs a name/);
  const wt = { root: "/w", repo: "/w", profile: work.id };
  assert.equal(chosen(registry, wt, "worktree")[1].id, work.id);
  registry = renamed(registry, work.id, "Staging");
  assert.equal(registry.profiles[1].name, "Staging");
  assert.throws(() => removed(registry, DEFAULT), /stays/);
  registry = removed(registry, work.id);
  assert.equal(chosen(registry, wt, "shared")[1].id, DEFAULT, "a picked profile that is gone falls back");
  const [withOwn] = managed(registry, "worktree", { root: "/w", repo: "/w" });
  assert.equal(ownedBy(withOwn, "/w").length, 1);
});

test("the address bar suggests visited pages, hosts first", () => {
  let history = emptyHistory();
  history = visited(history, { url: "https://docs.acme.example/orders", title: "Orders API", profile: "default" }, 1);
  history = visited(history, { url: "http://localhost:3000/orders", title: "acme web", profile: "default" }, 2);
  history = visited(history, { url: "http://localhost:3000/orders", title: "acme web — orders", profile: "default" }, 3);
  history = visited(history, { url: "file:///tmp/x.html", title: "x", profile: "default" }, 4);
  assert.equal(history.entries.length, 2);
  assert.deepEqual(suggest(history, "local ord").map((s) => s.url), ["http://localhost:3000/orders"]);
  assert.deepEqual(suggest(history, "orders").map((s) => s.url), ["http://localhost:3000/orders", "https://docs.acme.example/orders"]);
  assert.deepEqual(suggest(history, ""), []);
  assert.equal(withoutProfile(history, "default").entries.length, 0);
});

test("a viewport is a preset or a checked size, and fits its box", () => {
  assert.equal(PRESETS.length, 7);
  assert.deepEqual(viewportOf("tablet"), { preset: "tablet", width: 768, height: 1024, scale: 2, mobile: true });
  assert.deepEqual(viewportOf({ width: 390, height: 844, scale: 3, mobile: true }), { preset: null, width: 390, height: 844, scale: 3, mobile: true });
  assert.equal(viewportOf({ width: 10, height: 10 }), null);
  assert.equal(viewportOf("nope"), null);
  assert.equal(fitScale(viewportOf("tablet"), { width: 384, height: 2000 }), 0.5);
  assert.equal(fitScale(viewportOf("mobile-s"), { width: 900, height: 900 }), 1);
  assert.equal(emulation(null)[0][0], "Emulation.clearDeviceMetricsOverride");
  assert.equal(emulation(viewportOf("mobile-m"))[0][1].mobile, true);
});
