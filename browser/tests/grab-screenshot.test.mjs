// The crop of a picked element out of a page capture, with a fake page and a fake image.

import assert from "node:assert/strict";
import { test } from "node:test";
import { GRAB_BUDGET, HIDE_GRAB_OVERLAY_SCRIPT, RESTORE_GRAB_OVERLAY_SCRIPT, captureSelectionScreenshot, selectionCropRect } from "../main/grab/index.js";

const PNG = Buffer.from("acme-web pay button");

/** An image like Electron's NativeImage, recording the crops asked of it. */
function fakeImage({ width = 2560, height = 1600, empty = false, png = PNG } = {}) {
  const crops = [];
  return {
    crops,
    isEmpty: () => empty,
    getSize: () => ({ width, height }),
    crop(rect) {
      crops.push(rect);
      return { toPNG: () => png };
    },
  };
}

/** A page like a webContents, recording the scripts it runs and the captures it takes, in order. */
function fakeContents({ image = fakeImage(), innerWidth = 1280, capture } = {}) {
  const calls = [];
  return {
    calls,
    image,
    async executeJavaScript(code) {
      calls.push(code === HIDE_GRAB_OVERLAY_SCRIPT ? "hide" : code === RESTORE_GRAB_OVERLAY_SCRIPT ? "restore" : code);
      return code === "window.innerWidth" ? innerWidth : undefined;
    },
    async capturePage(...args) {
      calls.push(["capturePage", ...args]);
      return capture ? capture() : image;
    },
  };
}

test("the crop is scaled from CSS pixels to the bitmap, and sized back in CSS pixels", async () => {
  const contents = fakeContents();
  const shot = await captureSelectionScreenshot({ x: 40, y: 300, width: 120, height: 36 }, contents);
  assert.deepEqual(contents.image.crops, [{ x: 80, y: 600, width: 240, height: 72 }]);
  assert.deepEqual(shot, { mimeType: "image/png", dataUrl: `data:image/png;base64,${PNG.toString("base64")}`, width: 120, height: 36 });
});

test("the picker is hidden for the capture and shown again after it", async () => {
  const contents = fakeContents();
  await captureSelectionScreenshot({ x: 0, y: 0, width: 10, height: 10 }, contents);
  assert.deepEqual(contents.calls, ["hide", ["capturePage"], "restore", "window.innerWidth"]);
});

test("a failed capture still shows the picker again, and yields nothing", async () => {
  const contents = fakeContents({
    capture: () => {
      throw new Error("Object has been destroyed");
    },
  });
  assert.equal(await captureSelectionScreenshot({ x: 0, y: 0, width: 10, height: 10 }, contents), null);
  assert.deepEqual(contents.calls, ["hide", ["capturePage"], "restore"]);
});

test("a page that refuses scripts still gets captured", async () => {
  const contents = fakeContents();
  const run = contents.executeJavaScript;
  contents.executeJavaScript = (code) => (code === "window.innerWidth" ? run(code) : Promise.reject(new Error("blocked")));
  const shot = await captureSelectionScreenshot({ x: 0, y: 0, width: 10, height: 10 }, contents);
  assert.equal(shot.width, 10);
});

test("an empty capture or an unknown viewport width yields nothing", async () => {
  const rect = { x: 0, y: 0, width: 10, height: 10 };
  assert.equal(await captureSelectionScreenshot(rect, fakeContents({ image: fakeImage({ empty: true }) })), null);
  assert.equal(await captureSelectionScreenshot(rect, fakeContents({ innerWidth: 0 })), null);
  assert.equal(await captureSelectionScreenshot(rect, fakeContents({ innerWidth: null })), null);
});

test("a rectangle is clipped to the capture, and one outside it yields nothing", async () => {
  const clipped = fakeContents();
  const shot = await captureSelectionScreenshot({ x: 1200, y: 760, width: 200, height: 100 }, clipped);
  assert.deepEqual(clipped.image.crops, [{ x: 2400, y: 1520, width: 160, height: 80 }]);
  assert.deepEqual([shot.width, shot.height], [80, 40]);

  const outside = fakeContents();
  assert.equal(await captureSelectionScreenshot({ x: 1300, y: 0, width: 50, height: 50 }, outside), null);
  assert.deepEqual(outside.image.crops, []);
});

test("numbers that are not finite count as zero", async () => {
  const contents = fakeContents();
  assert.equal(await captureSelectionScreenshot({ x: Number.NaN, y: Infinity, width: "120", height: 36 }, contents), null);
  const origin = fakeContents();
  await captureSelectionScreenshot({ x: Number.NaN, y: -Infinity, width: 10, height: 10 }, origin);
  assert.deepEqual(origin.image.crops, [{ x: 0, y: 0, width: 20, height: 20 }]);
});

test("a PNG over the budget is dropped", async () => {
  const atBudget = fakeContents({ image: fakeImage({ png: Buffer.alloc(GRAB_BUDGET.screenshotMaxBytes) }) });
  assert.notEqual(await captureSelectionScreenshot({ x: 0, y: 0, width: 10, height: 10 }, atBudget), null);
  const over = fakeContents({ image: fakeImage({ png: Buffer.alloc(GRAB_BUDGET.screenshotMaxBytes + 1) }) });
  assert.equal(await captureSelectionScreenshot({ x: 0, y: 0, width: 10, height: 10 }, over), null);
});

test("the scale follows the capture, whatever the display or the zoom", () => {
  assert.deepEqual(selectionCropRect({ x: 10, y: 20, width: 100, height: 50 }, { width: 1280, height: 800 }, 1280), { x: 10, y: 20, width: 100, height: 50, scaleFactor: 1 });
  assert.deepEqual(selectionCropRect({ x: 10, y: 20, width: 100, height: 50 }, { width: 2560, height: 1600 }, 853), {
    x: 30,
    y: 60,
    width: 300,
    height: 150,
    scaleFactor: 2560 / 853,
  });
  assert.equal(selectionCropRect({ x: 0, y: 0, width: 0, height: 10 }, { width: 1280, height: 800 }, 1280), null);
  assert.equal(selectionCropRect({ x: 0, y: 0, width: 10, height: 10 }, { width: 0, height: 0 }, 1280), null);
  assert.equal(selectionCropRect(null, { width: 1280, height: 800 }, 1280), null);
});
