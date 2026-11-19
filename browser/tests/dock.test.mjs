import test from "node:test";
import assert from "node:assert/strict";
import { slotOnScreen, glued, next, initial, Dock } from "../main/dock.js";

const host = { id: 7, x: 100, y: 50, w: 1280, h: 820 };
const slot = { visible: true, cover: false, rect: { x: 860, y: 40, width: 400, height: 700 }, viewport: { width: 1280, height: 792 } };

test("the slot on screen adds the window's place and its title bar", () => {
  assert.deepEqual(slotOnScreen(host, slot), { x: 960, y: 118, width: 400, height: 700 });
  assert.deepEqual(slotOnScreen(host, { ...slot, viewport: null }), { x: 960, y: 90, width: 400, height: 700 });
});

test("a slot is cut at the window's edge and refused when too small", () => {
  const wide = { ...slot, rect: { x: 1000, y: 40, width: 600, height: 700 } };
  assert.equal(slotOnScreen(host, wide).width, 280);
  assert.equal(slotOnScreen(host, { ...slot, rect: { x: 1200, y: 40, width: 400, height: 700 } }), null);
  assert.equal(slotOnScreen(null, slot), null);
});

test("glued means only the browser's own windows sit between it and the host", () => {
  assert.equal(glued([{ id: 1, pid: 9 }], 9), true);
  assert.equal(glued([{ id: 3, pid: 4 }, { id: 1, pid: 9 }], 9), true, "another app in front of both is fine");
  assert.equal(glued([{ id: 1, pid: 9 }, { id: 3, pid: 4 }], 9), false, "another app's window slipped in between");
  assert.equal(glued([{ id: 3, pid: 4 }], 9), false);
  assert.equal(glued([{ id: 1, pid: 9 }, { id: 2, pid: 9 }], 9), true);
});

test("the window waits for the slot to rest, places itself, then stays", () => {
  let state = next(initial(), { host, above: [] }, slot, 9, 1000);
  assert.equal(state.step, "wait");
  state = next(state, { host, above: [] }, slot, 9, 1010);
  assert.equal(state.step, "wait");
  state = next(state, { host, above: [] }, slot, 9, 1100);
  assert.equal(state.step, "place");
  assert.equal(state.shown, true);
  state = next(state, { host, above: [{ id: 1, pid: 9 }] }, slot, 9, 1200);
  assert.equal(state.step, "none");
});

test("a host that moves hides the window until it rests, and a raised host is covered again", () => {
  let state = { ...initial(), shown: true, host, target: slotOnScreen(host, slot) };
  const moved = { ...host, x: 300 };
  state = next(state, { host: moved, above: [{ id: 1, pid: 9 }] }, slot, 9, 2000);
  assert.equal(state.step, "hide");
  state = next(state, { host: moved, above: [] }, slot, 9, 2200);
  assert.equal(state.step, "place");
  assert.deepEqual(state.target, slotOnScreen(moved, slot));
  state = next(state, { host: moved, above: [] }, slot, 9, 2300);
  assert.equal(state.step, "raise");
});

test("no host on screen, a hidden slot or a cover hides the window", () => {
  const shown = { ...initial(), shown: true, host, target: slotOnScreen(host, slot) };
  assert.equal(next(shown, { host: null, above: [] }, slot, 9, 1).step, "hide");
  assert.equal(next(shown, { host, above: [] }, { ...slot, visible: false }, 9, 1).step, "hide");
  assert.equal(next(shown, { host, above: [] }, { ...slot, cover: true }, 9, 1).step, "hide");
  assert.equal(next(initial(), { host: null, above: [] }, slot, 9, 1).step, "none");
});

test("the dock drives the window through its ops", async () => {
  const calls = [];
  const dock = new Dock({ hide: () => calls.push("hide"), place: (r) => calls.push(["place", r.x]), raise: () => calls.push("raise") }, 9);
  dock.setSlot(slot);
  dock.track({ host, above: [] });
  await new Promise((r) => setTimeout(r, 80));
  assert.deepEqual(calls, [["place", 960]]);
  dock.track({ host, above: [{ id: 5, pid: 4 }] });
  assert.deepEqual(calls.at(-1), "raise");
  dock.setSlot(null);
  assert.deepEqual(calls.at(-1), "hide");
  dock.stop();
});
