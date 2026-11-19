// The step scripts of a pick, run in node:vm against a stand-in for the armed picker, and the helpers around them.

import assert from "node:assert/strict";
import { test } from "node:test";
import { types } from "node:util";
import vm from "node:vm";
import {
  ANNOTATION_VIEWPORT_MESSAGE_PREFIX,
  GRAB_BUDGET,
  GRAB_CANCELLED_KEY,
  GRAB_CONTEXT_MENU_KEY,
  GRAB_GLOBAL,
  GRAB_SECRET_PATTERNS,
  GRAB_SELECTED_ATTRIBUTE,
  GRAB_STEPS,
  GRAB_TOGGLE_GUARD_SCRIPT,
  HIDE_GRAB_OVERLAY_SCRIPT,
  RESTORE_GRAB_OVERLAY_SCRIPT,
  buildAnnotationViewportBridgeScript,
  clampGrabPayload,
  grabKeyAction,
  grabScript,
  isValidAnnotationViewportBridgeMarkers,
  isValidAnnotationViewportBridgeToken,
  parseAnnotationViewportReport,
  readAnnotationViewportMessage,
  readGrabSelection,
} from "../main/grab/index.js";

const TOKEN = "acme-bridge-token-0001";
const MARKER = { id: "note-1", index: 0, isFixed: false, rectPage: { x: 80, y: 120, width: 100, height: 40 }, rectViewport: { x: 80, y: 120, width: 100, height: 40 } };

const validPayload = () => ({
  page: { title: "Checkout - acme/web", capturedAt: "2026-01-12T09:30:00.000Z" },
  target: { tagName: "button" },
  nearbyText: [],
  ancestorPath: [],
  screenshot: null,
});

/** A page whose picker is armed, with what the scripts did to it. */
function armed({ extractPayload = validPayload, getCurrentElement = () => ({}) } = {}) {
  const log = [];
  const grab = { extractPayload, getCurrentElement, freezeHighlight: (id) => log.push(["freeze", id]), cleanup: () => log.push(["cleanup"]) };
  const window = { [GRAB_GLOBAL]: grab };
  return { window, grab, log, pick: (type) => grab.onPick?.({ type }) };
}

const run = (step, window, globals = {}) => vm.runInNewContext(grabScript(step), { window, ...globals });

test("every injected script parses", () => {
  assert.deepEqual(GRAB_STEPS, ["arm", "awaitClick", "finalize", "extractHover", "teardown"]);
  for (const step of GRAB_STEPS) new vm.Script(grabScript(step), { filename: `grab-${step}.js` });
  for (const script of [HIDE_GRAB_OVERLAY_SCRIPT, RESTORE_GRAB_OVERLAY_SCRIPT, GRAB_TOGGLE_GUARD_SCRIPT]) new vm.Script(script);
  new vm.Script(buildAnnotationViewportBridgeScript({ enabled: true, emitViewport: true, markers: [MARKER], selectionId: "2026-01-12T09:30:00.000Z", token: TOKEN }));
  new vm.Script(buildAnnotationViewportBridgeScript({ enabled: false, emitViewport: false, markers: [], token: TOKEN }));
  assert.throws(() => grabScript("disarm"), /Unknown grab step: disarm/);
});

test("the scripts carry the shared names and budgets", () => {
  const arm = grabScript("arm");
  assert.ok(arm.includes(`window.${GRAB_GLOBAL} = {`));
  assert.ok(arm.includes(`var selectedAttribute = "${GRAB_SELECTED_ATTRIBUTE}";`));
  assert.ok(arm.includes(`var BUDGET = ${JSON.stringify(GRAB_BUDGET)};`));
  assert.ok(arm.includes(`var SECRET_PATTERNS = ${JSON.stringify(GRAB_SECRET_PATTERNS)};`));
  assert.ok(grabScript("awaitClick").includes(`{ ${GRAB_CONTEXT_MENU_KEY}: true, payload: payload }`));
  assert.ok(grabScript("awaitClick").includes(`resolve({ ${GRAB_CANCELLED_KEY}: true });`));
  for (const step of GRAB_STEPS) assert.ok(grabScript(step).includes(`window.${GRAB_GLOBAL}`));
  assert.ok(HIDE_GRAB_OVERLAY_SCRIPT.includes(`window.${GRAB_GLOBAL}`));
});

test("a click resolves the payload and freezes the outline on the element", async () => {
  const page = armed();
  const pending = run("awaitClick", page.window);
  page.pick("click");
  const result = await pending;
  assert.deepEqual(Object.keys(result), ["page", "target", "nearbyText", "ancestorPath", "screenshot"]);
  assert.deepEqual(page.log, [["freeze", "2026-01-12T09:30:00.000Z"]]);
  assert.equal(page.grab.onPick, null);
  assert.equal(readGrabSelection(result).kind, "selected");
});

test("a right-click wraps the payload with the context-menu marker", async () => {
  const page = armed();
  const pending = run("awaitClick", page.window);
  page.pick("contextmenu");
  const result = await pending;
  assert.equal(result[GRAB_CONTEXT_MENU_KEY], true);
  assert.equal(result.payload.target.tagName, "button");
  assert.equal(readGrabSelection(result).kind, "context-selected");
});

test("awaitClick rejects before an arm, with nothing hovered, and when the extraction throws", async () => {
  await assert.rejects(run("awaitClick", {}), /Grab not armed/);

  const nothing = armed({ getCurrentElement: () => null });
  const noElement = run("awaitClick", nothing.window);
  nothing.pick("click");
  await assert.rejects(noElement, /^Error: cancelled$/);
  assert.deepEqual(nothing.log, [["cleanup"]]);

  const broken = armed({
    extractPayload: () => {
      throw "detached";
    },
  });
  const failed = run("awaitClick", broken.window);
  broken.pick("click");
  await assert.rejects(failed, /Failed to extract element context/);
  assert.deepEqual(broken.log, [["cleanup"]]);
});

test("a teardown settles a pending awaitClick with the cancel marker", async () => {
  const page = armed();
  const pending = run("awaitClick", page.window);
  assert.equal(run("teardown", page.window), true);
  assert.deepEqual({ ...(await pending) }, { [GRAB_CANCELLED_KEY]: true });
  assert.deepEqual(page.log, [["cleanup"]]);
  assert.deepEqual(readGrabSelection(await pending), { kind: "cancelled" });
});

test("a teardown without a pending await removes the picker, and is harmless when nothing is armed", () => {
  const page = armed();
  assert.equal(run("teardown", page.window), true);
  assert.deepEqual(page.log, [["cleanup"]]);
  assert.equal(run("teardown", {}), true);
});

test("finalize returns the hovered element's payload and removes the picker", () => {
  const page = armed();
  assert.equal(run("finalize", page.window).target.tagName, "button");
  assert.deepEqual(page.log, [["cleanup"]]);

  assert.equal(run("finalize", {}), null);
  const nothing = armed({ getCurrentElement: () => null });
  assert.equal(run("finalize", nothing.window), null);
  assert.deepEqual(nothing.log, []);
  const broken = armed({
    extractPayload: () => {
      throw new Error("detached");
    },
  });
  assert.equal(run("finalize", broken.window), null);
  assert.deepEqual(broken.log, [["cleanup"]]);
});

test("extractHover returns the hovered element's payload and leaves the picker armed", () => {
  const page = armed();
  assert.equal(run("extractHover", page.window).target.tagName, "button");
  assert.deepEqual(page.log, []);
  assert.equal(run("extractHover", {}), null);
  assert.equal(run("extractHover", armed({ getCurrentElement: () => null }).window), null);
  const broken = armed({
    extractPayload: () => {
      throw new Error("detached");
    },
  });
  assert.equal(run("extractHover", broken.window), null);
  assert.deepEqual(broken.log, []);
});

test("awaitClick resolves across executeJavaScript when Zone.js replaces the page's Promise", async () => {
  // Zone.js keeps the native promise off the instance and exposes enumerable state fields instead.
  const nativeOf = new WeakMap();
  class ZoneAwarePromiseLike {
    __zone_symbol__state = null;
    __zone_symbol__value = undefined;

    constructor(executor) {
      nativeOf.set(
        this,
        new Promise((res, rej) => {
          executor(
            (value) => {
              this.__zone_symbol__state = true;
              this.__zone_symbol__value = value;
              res(value);
            },
            (reason) => {
              this.__zone_symbol__state = false;
              this.__zone_symbol__value = reason;
              rej(reason);
            },
          );
        }),
      );
    }

    get [Symbol.toStringTag]() {
      return "Promise";
    }

    then(onFulfilled, onRejected) {
      return nativeOf.get(this).then(onFulfilled ?? undefined, onRejected ?? undefined);
    }
  }
  // Electron unwraps genuine V8 promises and serializes any other thenable by value.
  const crossBoundary = async (completion) => (types.isPromise(completion) ? await completion : { ...completion });

  const page = armed();
  const completion = run("awaitClick", page.window, { Promise: ZoneAwarePromiseLike, Error });
  assert.equal(types.isPromise(completion), true);

  page.pick("click");
  const received = await crossBoundary(completion);
  assert.ok("page" in received && "target" in received);
  assert.equal("__zone_symbol__value" in received, false);
  assert.notEqual(clampGrabPayload(received), null);
});

test("bare C, S and Escape act on an armed pick, and other keys stay the page's", () => {
  const down = (key, more = {}) => ({ type: "keyDown", key, ...more });
  assert.equal(grabKeyAction(down("c")), "copy");
  assert.equal(grabKeyAction(down("C")), "copy");
  assert.equal(grabKeyAction(down("s")), "screenshot");
  assert.equal(grabKeyAction(down("Escape")), "cancel");
  assert.equal(grabKeyAction(down("Escape", { isAutoRepeat: true })), null);
  assert.equal(grabKeyAction(down("c", { isAutoRepeat: true })), "copy");
  for (const modifier of ["meta", "control", "alt", "shift"]) {
    assert.equal(grabKeyAction(down("c", { [modifier]: true })), null);
    assert.equal(grabKeyAction(down("Escape", { [modifier]: true })), null);
  }
  assert.equal(grabKeyAction({ type: "keyUp", key: "c" }), null);
  assert.equal(grabKeyAction(down("x")), null);
  assert.equal(grabKeyAction(down(undefined)), null);
  assert.equal(grabKeyAction(null), null);
});

test("a viewport report is read from a console line carrying the bridge's token", () => {
  const line = (token, body) => `${ANNOTATION_VIEWPORT_MESSAGE_PREFIX}${token}:${body}`;
  const report = { scrollX: 0, scrollY: 240, viewportWidth: 1280, target: { x: 40, y: 60, width: 120, height: 36 } };
  assert.deepEqual(readAnnotationViewportMessage(line(TOKEN, JSON.stringify(report)), TOKEN), report);
  assert.equal(readAnnotationViewportMessage(line("another-bridge-token-0002", JSON.stringify(report)), TOKEN), null);
  assert.equal(readAnnotationViewportMessage("Leo logged something", TOKEN), null);
  assert.equal(readAnnotationViewportMessage(undefined, TOKEN), null);
  assert.equal(readAnnotationViewportMessage(line(TOKEN, "{not json"), TOKEN), null);
});

test("a viewport report drops what is not a finite number or a real rectangle", () => {
  assert.deepEqual(parseAnnotationViewportReport('{"scrollX":"12","scrollY":null,"viewportWidth":0,"target":{"x":1,"y":2,"width":-3,"height":4}}'), { scrollX: 0, scrollY: 0 });
  assert.deepEqual(parseAnnotationViewportReport('{"scrollY":12.5,"target":null}'), { scrollX: 0, scrollY: 12.5 });
  assert.equal(parseAnnotationViewportReport("null"), null);
  assert.equal(parseAnnotationViewportReport("7"), null);
});

test("bridge tokens and markers are checked before they reach a script", () => {
  assert.equal(isValidAnnotationViewportBridgeToken(TOKEN), true);
  for (const token of ["short", "a".repeat(81), "has spaces in the token", "quote'in-the-token-000", 1234567890123456, null]) {
    assert.equal(isValidAnnotationViewportBridgeToken(token), false);
  }
  assert.equal(isValidAnnotationViewportBridgeMarkers([]), true);
  assert.equal(isValidAnnotationViewportBridgeMarkers([MARKER, { ...MARKER, id: "note-2", index: 1, isFixed: true }]), true);
  assert.equal(isValidAnnotationViewportBridgeMarkers(Array.from({ length: 51 }, () => MARKER)), false);
  for (const marker of [null, "note-1", { ...MARKER, id: "" }, { ...MARKER, index: 1.5 }, { ...MARKER, index: 100 }, { ...MARKER, isFixed: "no" }, { ...MARKER, rectPage: { x: 0, y: 0, width: Infinity, height: 1 } }]) {
    assert.equal(isValidAnnotationViewportBridgeMarkers([marker]), false);
  }
  assert.equal(isValidAnnotationViewportBridgeMarkers({ length: 0 }), false);
});

test("a disabled bridge removes itself from the page", () => {
  const removed = [];
  const state = { raf: 7, requestUpdate: () => {}, onBeforeUnload: () => {}, targetObserver: { disconnect: () => removed.push("observer") }, targetShadowRoots: [], host: null };
  const listeners = { removeEventListener: (type) => removed.push(type) };
  const context = { window: listeners, document: listeners, cancelAnimationFrame: (id) => removed.push(`frame ${id}`), __cockpitAnnotationViewportBridge: state };
  const script = buildAnnotationViewportBridgeScript({ enabled: false, emitViewport: false, markers: [], token: TOKEN });
  assert.equal(vm.runInNewContext(script, context), true);
  assert.equal("__cockpitAnnotationViewportBridge" in context, false);
  assert.deepEqual(removed, ["frame 7", "scroll", "scroll", "resize", "beforeunload", "observer"]);
  assert.equal(vm.runInNewContext(script, {}), true);
});
