// The picker's overlay, run in node:vm against a stand-in for the DOM with events, shadow roots and animation frames.

import assert from "node:assert/strict";
import { test } from "node:test";
import vm from "node:vm";
import {
  GRAB_CANCELLED_KEY,
  GRAB_CONTEXT_MENU_KEY,
  GRAB_GLOBAL,
  GRAB_SELECTED_ATTRIBUTE,
  HIDE_GRAB_OVERLAY_SCRIPT,
  RESTORE_GRAB_OVERLAY_SCRIPT,
  buildAnnotationViewportBridgeScript,
  grabScript,
  readAnnotationViewportMessage,
} from "../main/grab/index.js";

const TOKEN = "acme-bridge-token-0001";

/** A page of acme/web with a Pay button, optionally inside an open shadow root, and the means to drive it. */
function openPage({ inShadowRoot = false, marker } = {}) {
  const frames = [];
  const shadowRoots = [];
  const observers = [];
  const consoleLines = [];

  class Target {
    listeners = [];
    addEventListener(type, fn, capture = false) {
      this.listeners.push({ type, fn, capture });
    }
    removeEventListener(type, fn, capture = false) {
      this.listeners = this.listeners.filter((l) => !(l.type === type && l.fn === fn && l.capture === capture));
    }
  }
  class Container extends Target {
    childNodes = [];
    appendChild(kid) {
      kid.parentNode = this;
      this.childNodes.push(kid);
      return kid;
    }
    removeChild(kid) {
      this.childNodes.splice(this.childNodes.indexOf(kid), 1);
      kid.parentNode = null;
    }
    get firstElementChild() {
      return this.childNodes[0] ?? null;
    }
    querySelectorAll(selector) {
      const all = this.childNodes.flatMap((kid) => [kid, ...kid.querySelectorAll("*")]);
      const attribute = /^\[(.+)\]$/.exec(selector)?.[1];
      return attribute ? all.filter((node) => node.attrs.has(attribute)) : all;
    }
  }
  class Root extends Container {
    nodeType = 11;
    parentNode = null;
    constructor(host) {
      super();
      this.host = host;
    }
  }
  class El extends Container {
    nodeType = 1;
    parentNode = null;
    attrs = new Map();
    style = {};
    shadowRoot = null;
    popoverCalls = [];
    rect = { x: 0, y: 0, width: 0, height: 0 };
    constructor(tag) {
      super();
      this.localName = tag;
      this.tagName = tag.toUpperCase();
    }
    get parentElement() {
      return this.parentNode?.nodeType === 1 ? this.parentNode : null;
    }
    get isConnected() {
      for (let node = this; node; node = node.parentNode ?? node.host) if (node === html) return true;
      return false;
    }
    getAttribute(name) {
      return this.attrs.has(name) ? this.attrs.get(name) : null;
    }
    setAttribute(name, value) {
      this.attrs.set(name, String(value));
    }
    removeAttribute(name) {
      this.attrs.delete(name);
    }
    getBoundingClientRect() {
      return { ...this.rect, top: this.rect.y, bottom: this.rect.y + this.rect.height };
    }
    attachShadow({ mode }) {
      const root = new Root(this);
      shadowRoots.push(root);
      if (mode === "open") this.shadowRoot = root;
      return root;
    }
    remove() {
      this.parentNode?.removeChild(this);
    }
    showPopover() {
      this.popoverCalls.push("show");
    }
    hidePopover() {
      this.popoverCalls.push("hide");
    }
  }

  const html = new El("html");
  const body = html.appendChild(new El("body"));
  const scroller = new El("div");
  const target = scroller.appendChild(new El("button"));
  Object.assign(target, { text: "Pay now", rect: { x: 40, y: 300, width: 120, height: 30 } });
  if (marker !== undefined) target.setAttribute(GRAB_SELECTED_ATTRIBUTE, marker);
  const shadowHost = new El("acme-checkout");
  body.appendChild(inShadowRoot ? shadowHost : scroller);
  let hit = target;
  if (inShadowRoot) {
    const pageRoot = shadowHost.attachShadow({ mode: "open" });
    pageRoot.appendChild(scroller);
    pageRoot.elementFromPoint = () => target;
    hit = shadowHost;
  }

  const window = Object.assign(new Target(), { innerWidth: 1280, innerHeight: 800, scrollX: 0, scrollY: 0 });
  const document = Object.assign(new Target(), {
    documentElement: html,
    body,
    createElement: (tag) => new El(tag),
    elementFromPoint: () => page.hit,
    querySelectorAll: (selector) => [html, ...html.querySelectorAll("*")].filter((node) => selector === "*" || node.attrs.has(/^\[(.+)\]$/.exec(selector)[1])),
    createTreeWalker: (root) => {
      const texts = root.text ? [{ nodeValue: root.text }] : [];
      return { nextNode: () => texts.shift() ?? null };
    },
  });
  class ResizeObserver {
    observed = [];
    constructor(callback) {
      this.callback = callback;
      observers.push(this);
    }
    observe(element, options) {
      this.observed.push([element, options]);
    }
    disconnect() {
      this.observed = [];
    }
  }
  const context = vm.createContext({
    window,
    document,
    URL,
    ResizeObserver,
    Node: { TEXT_NODE: 3, ELEMENT_NODE: 1 },
    NodeFilter: { SHOW_TEXT: 4 },
    requestAnimationFrame: (callback) => frames.push(callback),
    cancelAnimationFrame: () => {},
    console: { debug: (line) => consoleLines.push(line) },
  });

  const page = {
    El,
    window,
    document,
    html,
    body,
    scroller,
    target,
    shadowHost,
    hit,
    frames,
    observers,
    consoleLines,
    run: (script) => vm.runInContext(script, context),
    get grab() {
      return window[GRAB_GLOBAL];
    },
    host: () => html.childNodes.find((node) => node.id === "__cockpit-grab-host"),
    /** The overlay inside the picker's closed shadow root, then its outline and its label. */
    shadowRootOf: (host) => shadowRoots.find((root) => root.host === host),
    overlay: () => page.shadowRootOf(page.host()).firstElementChild,
    outline: () => page.overlay().childNodes[0].style,
    label: () => page.overlay().childNodes[1],
    outlineRect: () => ["left", "top", "width", "height"].map((side) => Number.parseFloat(page.outline()[side])),
    flushFrames() {
      for (const frame of frames.splice(0)) frame();
    },
    /** Dispatches an event down its capture path; only a composed event leaves a shadow root. */
    fire(on, type, props = {}) {
      const path = [];
      for (let node = on; node; ) {
        path.push(node);
        if (node === window) break;
        if (node.nodeType === 11) node = props.composed ? node.host : null;
        else node = node.parentNode ?? (node === html ? window : null);
      }
      const event = { type, target: on, isTrusted: true, detail: 1, button: 0, defaultPrevented: false, stopped: false, ...props };
      event.preventDefault = () => (event.defaultPrevented = true);
      event.stopImmediatePropagation = () => (event.stopped = true);
      for (const node of path.reverse()) {
        for (const l of [...node.listeners]) if (l.type === type && l.capture && !event.stopped) l.fn.call(node, event);
      }
      return event;
    },
    hover(element = page.hit) {
      page.hit = element;
      page.fire(window, "mousemove", { clientX: 50, clientY: 310 });
      page.flushFrames();
    },
  };
  return page;
}

/** A page with the picker armed and the Pay button hovered. */
function armedPage(options) {
  const page = openPage(options);
  assert.equal(page.run(grabScript("arm")), true);
  page.hover();
  return page;
}

const PAYLOAD = { page: { capturedAt: "2026-01-12T09:30:00.000Z" }, target: { tagName: "button" }, nearbyText: [], ancestorPath: [], screenshot: null };

/** Starts awaitClick with a canned extraction, since this stand-in has no selectors or styles to read. */
function awaitClick(page) {
  page.grab.extractPayload = () => PAYLOAD;
  return page.run(grabScript("awaitClick"));
}

test("arming covers the viewport with a closed shadow root at the highest z-index, shown in the top layer", () => {
  const page = armedPage();
  const host = page.host();
  assert.equal(host.parentNode, page.html);
  assert.match(host.style.cssText, /position:fixed;top:0;left:0;width:100vw;height:100vh;z-index:2147483647;pointer-events:all;cursor:crosshair;/);
  assert.equal(host.shadowRoot, null);
  assert.equal(page.overlay().popover, "manual");
  assert.deepEqual(page.overlay().popoverCalls, ["show"]);
  assert.equal(page.grab.host, host);
  assert.equal(page.grab.getCurrentElement(), page.target);
});

test("hovering outlines the element under the pointer and labels it", () => {
  const page = armedPage();
  assert.deepEqual(page.outlineRect(), [40, 300, 120, 30]);
  assert.equal(page.outline().display, "block");
  assert.equal(page.label().textContent, 'button  "Pay now"  120x30');
  assert.deepEqual([page.label().style.left, page.label().style.top], ["40px", "336px"]);
  assert.equal(page.host().style.pointerEvents, "all");

  page.target.setAttribute("role", "menuitem");
  page.target.rect = { x: 0, y: 780, width: 60.4, height: 20 };
  page.hover();
  assert.equal(page.label().textContent, 'button  role=menuitem  "Pay now"  60x20');
  assert.deepEqual([page.label().style.left, page.label().style.top], ["4px", "752px"]);

  page.hover(page.body);
  assert.equal(page.outline().display, "none");
  assert.equal(page.label().style.display, "none");
  assert.equal(page.grab.getCurrentElement(), null);
});

test("the hit test steps aside for the page, and descends through open shadow roots", () => {
  const page = armedPage();
  const seen = [];
  page.document.elementFromPoint = () => {
    seen.push(page.host().style.pointerEvents);
    return page.shadowHost;
  };
  const inner = new page.El("acme-pay");
  const deep = new page.El("span");
  page.shadowHost.attachShadow({ mode: "open" }).elementFromPoint = () => inner;
  inner.attachShadow({ mode: "open" }).elementFromPoint = () => deep;
  page.hover();
  assert.deepEqual(seen, ["none"]);
  assert.equal(page.host().style.pointerEvents, "all");
  assert.equal(page.grab.getCurrentElement(), deep);

  // A form control named "shadowRoot" shadows the property: the deepest real hit stays.
  deep.shadowRoot = { name: "shadowRoot" };
  page.hover();
  assert.equal(page.grab.getCurrentElement(), deep);
  assert.equal(page.host().style.pointerEvents, "all");
});

test("presses are kept from the page, and only a click or a right-click picks", () => {
  const page = armedPage();
  const picks = [];
  for (const type of ["pointerdown", "mousedown", "pointerup", "mouseup", "dblclick", "auxclick", "click", "contextmenu"]) {
    page.grab.onPick = (event) => picks.push(event.type);
    const event = page.fire(page.target, type);
    assert.equal(event.defaultPrevented && event.stopped, true, type);
  }
  assert.deepEqual(picks, ["click", "contextmenu"]);
});

test("scripted clicks and keyboard activation keep their default", () => {
  const page = armedPage();
  const picks = [];
  page.grab.onPick = (event) => picks.push(event.type);
  for (const [type, props] of [["click", { isTrusted: false }], ["click", { detail: 0 }], ["contextmenu", { button: -1 }], ["keydown", { key: "Enter" }]]) {
    const event = page.fire(page.target, type, props);
    assert.equal(event.defaultPrevented || event.stopped, false, `${type} ${JSON.stringify(props)}`);
  }
  assert.deepEqual(picks, []);
});

test("a click settles awaitClick, marks the element and hands the pointer back to the page", async () => {
  const page = armedPage();
  const pending = awaitClick(page);
  const click = page.fire(page.target, "click");
  assert.equal(click.defaultPrevented, true);
  assert.equal((await pending).target.tagName, "button");
  assert.equal(page.target.getAttribute(GRAB_SELECTED_ATTRIBUTE), "2026-01-12T09:30:00.000Z");
  assert.deepEqual([page.host().style.pointerEvents, page.host().style.cursor], ["none", "default"]);
  assert.equal(page.outline().transition, "none");

  assert.equal(page.fire(page.target, "click").defaultPrevented, false);
  page.hover(page.body);
  assert.equal(page.grab.getCurrentElement(), page.target);
  assert.deepEqual(page.outlineRect(), [40, 300, 120, 30]);
});

test("a right-click settles awaitClick with the context-menu marker", async () => {
  const page = armedPage();
  const pending = awaitClick(page);
  page.fire(page.target, "contextmenu", { button: 2 });
  const result = await pending;
  assert.equal(result[GRAB_CONTEXT_MENU_KEY], true);
  assert.equal(result.payload.target.tagName, "button");
});

test("the frozen outline follows an element inside an open shadow root when it scrolls", () => {
  const page = armedPage({ inShadowRoot: true });
  page.grab.freezeHighlight("selection-1");
  assert.equal(page.target.getAttribute(GRAB_SELECTED_ATTRIBUTE), "selection-1");
  assert.deepEqual(page.outlineRect(), [40, 300, 120, 30]);

  // A scroll event stays inside its shadow root, so a listener on the window never sees it.
  page.target.rect = { x: 40, y: 180, width: 120, height: 30 };
  page.fire(page.scroller, "scroll");
  page.flushFrames();
  assert.deepEqual(page.outlineRect(), [40, 180, 120, 30]);
});

test("the frozen outline follows window scrolls, resizes and size changes, and hides when the element leaves", () => {
  const page = armedPage();
  page.grab.freezeHighlight("selection-1");
  const [observer] = page.observers;
  assert.deepEqual(observer.observed.map(([element, options]) => [element, options?.box]), [[page.target, "border-box"], [page.html, undefined]]);

  const moves = [
    [() => page.fire(page.window, "scroll"), { x: 40, y: 120, width: 120, height: 30 }],
    [() => page.fire(page.window, "resize"), { x: 20, y: 120, width: 100, height: 30 }],
    [() => observer.callback(), { x: 20, y: 120, width: 100, height: 44 }],
  ];
  for (const [notify, rect] of moves) {
    page.target.rect = rect;
    notify();
    notify();
    assert.equal(page.frames.length, 1);
    page.flushFrames();
    assert.deepEqual(page.outlineRect(), [rect.x, rect.y, rect.width, rect.height]);
  }

  page.scroller.remove();
  page.fire(page.window, "scroll");
  page.flushFrames();
  assert.equal(page.outline().display, "none");
});

test("removing the picker restores the element's own marker, unless the page changed it since", () => {
  const untouched = armedPage();
  untouched.grab.freezeHighlight("selection-1");
  untouched.grab.cleanup();
  assert.equal(untouched.target.getAttribute(GRAB_SELECTED_ATTRIBUTE), null);

  const owned = armedPage({ marker: "page-owned" });
  owned.grab.freezeHighlight("selection-1");
  assert.equal(owned.target.getAttribute(GRAB_SELECTED_ATTRIBUTE), "selection-1");
  owned.grab.cleanup();
  assert.equal(owned.target.getAttribute(GRAB_SELECTED_ATTRIBUTE), "page-owned");

  const updated = armedPage({ marker: "page-owned" });
  updated.grab.freezeHighlight("selection-1");
  updated.target.setAttribute(GRAB_SELECTED_ATTRIBUTE, "page-updated");
  updated.grab.cleanup();
  assert.equal(updated.target.getAttribute(GRAB_SELECTED_ATTRIBUTE), "page-updated");
});

test("the outline is raised again when a popover opens or fullscreen changes", () => {
  const page = armedPage();
  page.fire(page.target, "toggle", { newState: "closed" });
  assert.deepEqual(page.overlay().popoverCalls, ["show"]);
  page.fire(page.target, "toggle", { newState: "open" });
  assert.deepEqual(page.overlay().popoverCalls, ["show", "hide", "show"]);
  page.fire(page.html, "fullscreenchange");
  assert.deepEqual(page.overlay().popoverCalls, ["show", "hide", "show", "hide", "show"]);
});

test("a teardown removes the host, the listeners and the global, and cancels a pending awaitClick", async () => {
  const page = armedPage({ inShadowRoot: true });
  page.grab.freezeHighlight("selection-1");
  const pending = awaitClick(page);
  assert.equal(page.run(grabScript("teardown")), true);
  assert.deepEqual({ ...(await pending) }, { [GRAB_CANCELLED_KEY]: true });
  assert.equal(page.host(), undefined);
  assert.equal(GRAB_GLOBAL in page.window, false);
  assert.deepEqual(page.window.listeners, []);
  assert.deepEqual(page.shadowHost.shadowRoot.listeners, []);
  assert.deepEqual(page.observers[0].observed, []);
  assert.equal(page.target.getAttribute(GRAB_SELECTED_ATTRIBUTE), null);
  assert.equal(page.fire(page.target, "click").defaultPrevented, false);
});

test("arming again replaces the earlier picker", () => {
  const page = armedPage();
  const first = page.host();
  const listeners = page.window.listeners.length;
  page.grab.freezeHighlight("selection-1");
  assert.equal(page.run(grabScript("arm")), true);
  assert.notEqual(page.host(), first);
  assert.equal(page.html.childNodes.filter((node) => node.id === "__cockpit-grab-host").length, 1);
  assert.equal(page.window.listeners.length, listeners);
  assert.equal(page.target.getAttribute(GRAB_SELECTED_ATTRIBUTE), null);
  assert.equal(page.grab.getCurrentElement(), null);
});

test("the viewport bridge reports the frozen element's box and keeps note badges on the page", () => {
  const page = armedPage();
  page.grab.freezeHighlight("selection-1");
  const marker = { id: "note-1", index: 2, isFixed: false, label: "Fix · Important\nUse the primary colour", rectPage: { x: 80, y: 520, width: 100, height: 40 }, rectViewport: { x: 80, y: 520, width: 100, height: 40 } };
  const pinned = { ...marker, id: "note-2", index: 3, isFixed: true, label: undefined, rectViewport: { x: 1100, y: 10, width: 30, height: 30 } };
  const bridge = (options) => buildAnnotationViewportBridgeScript({ enabled: true, emitViewport: true, markers: [marker, pinned], selectionId: "selection-1", token: TOKEN, ...options });
  assert.equal(page.run(bridge()), true);
  page.flushFrames();

  assert.deepEqual(readAnnotationViewportMessage(page.consoleLines.at(-1), TOKEN), { scrollX: 0, scrollY: 0, viewportWidth: 1280, target: { x: 40, y: 300, width: 120, height: 30 } });
  const badgeHost = page.body.childNodes.find((node) => node.attrs.has("data-cockpit-annotation-overlay"));
  assert.match(badgeHost.style.cssText, /z-index:2147483646;pointer-events:none/);
  assert.equal(badgeHost.shadowRoot, null);
  const badges = () => page.shadowRootOf(badgeHost).childNodes.filter((node) => node.className === "marker").map((node) => [node.textContent, node.style.transform]);
  assert.deepEqual(badges(), [["3", "translate3d(118px,548px,0)"], ["4", "translate3d(1103px,28px,0)"]]);
  // A badge says its note when the pointer rests on it, and opens leftwards near the page's right edge.
  const said = page.shadowRootOf(badgeHost).childNodes.filter((node) => node.className === "marker").map((node) => [node.getAttribute("data-note"), node.attrs.has("data-flip")]);
  assert.deepEqual(said, [["Fix · Important\nUse the primary colour", false], [null, true]]);

  page.window.scrollY = 200;
  page.target.rect = { x: 40, y: 100, width: 120, height: 30 };
  page.fire(page.window, "scroll");
  page.flushFrames();
  assert.deepEqual(readAnnotationViewportMessage(page.consoleLines.at(-1), TOKEN).target, { x: 40, y: 100, width: 120, height: 30 });
  assert.equal(readAnnotationViewportMessage(page.consoleLines.at(-1), TOKEN).scrollY, 200);
  assert.deepEqual(badges(), [["3", "translate3d(118px,348px,0)"], ["4", "translate3d(1103px,28px,0)"]]);

  page.run(HIDE_GRAB_OVERLAY_SCRIPT);
  assert.deepEqual([page.host().style.display, badgeHost.style.display], ["none", "none"]);
  page.run(RESTORE_GRAB_OVERLAY_SCRIPT);
  assert.deepEqual([page.host().style.display, badgeHost.style.display], ["", ""]);
  assert.equal(badgeHost.attrs.has("data-cockpit-previous-display"), false);

  assert.equal(page.run(bridge({ enabled: false })), true);
  assert.equal(page.body.childNodes.includes(badgeHost), false);
});
