// The extraction helpers of the arm script, run in node:vm against a small stand-in for the DOM.

import assert from "node:assert/strict";
import { test } from "node:test";
import vm from "node:vm";
import { clampGrabPayload } from "../main/grab/index.js";
import { COMPOSED_TREE_SCRIPT } from "../main/grab/script-composed-tree.js";
import { GRAB_CONTENT_SCRIPT } from "../main/grab/script-content.js";
import { GRAB_ELEMENT_CONTEXT_SCRIPT } from "../main/grab/script-element-context.js";
import { GRAB_FOUNDATION_SCRIPT } from "../main/grab/script-foundation.js";
import { GRAB_REACT_SCRIPT } from "../main/grab/script-react.js";

const HELPERS = [
  "clampStr", "containsSecret", "sanitizeUrl", "getBoundedText", "getSelectedText", "getHtmlSnippet", "getSafeAttributes", "getAriaLabelledByIds",
  "getAccessibility", "getComputedStyleSubset", "cssEscape", "looksHashy", "getStableClasses", "buildSelector", "buildReadablePath", "buildFullPath",
  "getNearbyText", "getAncestorPath", "getNearbyElements", "composedParent", "shadowRootsAbove", "isElementFixed", "getReactMetadata", "cleanSourcePath",
  "shouldSkipReactName", "extractPayload",
];
// The arm script without its overlay, closed so that it hands its helpers out.
const HELPERS_SCRIPT = [GRAB_FOUNDATION_SCRIPT, COMPOSED_TREE_SCRIPT, GRAB_CONTENT_SCRIPT, GRAB_ELEMENT_CONTEXT_SCRIPT, GRAB_REACT_SCRIPT, `return { ${HELPERS.join(", ")} };\n})()`].join("");

class Text {
  nodeType = 3;
  parentNode = null;
  constructor(value) {
    this.nodeValue = value;
  }
}

class El {
  nodeType = 1;
  parentNode = null;
  childNodes = [];
  assignedSlot = null;
  constructor(tag, attrs = {}, kids = []) {
    this.localName = tag;
    this.tagName = tag.toUpperCase();
    this.attrs = new Map(Object.entries(attrs));
    for (const kid of kids) this.append(typeof kid === "string" ? new Text(kid) : kid);
  }
  append(kid) {
    kid.parentNode = this;
    this.childNodes.push(kid);
    return kid;
  }
  get attributes() {
    return [...this.attrs].map(([name, value]) => ({ name, value }));
  }
  getAttribute(name) {
    return this.attrs.has(name) ? this.attrs.get(name) : null;
  }
  get id() {
    return this.attrs.get("id") ?? "";
  }
  get classList() {
    return (this.attrs.get("class") ?? "").split(/\s+/).filter(Boolean);
  }
  get parentElement() {
    return this.parentNode?.nodeType === 1 ? this.parentNode : null;
  }
  get children() {
    return this.childNodes.filter((node) => node.nodeType === 1);
  }
  sibling(step) {
    const all = this.parentNode?.children ?? [];
    return all[all.indexOf(this) + step] ?? null;
  }
  get previousElementSibling() {
    return this.sibling(-1);
  }
  get nextElementSibling() {
    return this.sibling(1);
  }
  getBoundingClientRect() {
    return this.rect ?? { x: 0, y: 0, width: 10, height: 10 };
  }
  remove() {
    this.parentNode.childNodes.splice(this.parentNode.childNodes.indexOf(this), 1);
  }
  cloneNode() {
    return new El(this.localName, Object.fromEntries(this.attrs), this.childNodes.map((node) => (node.nodeType === 3 ? node.nodeValue : node.cloneNode())));
  }
  get outerHTML() {
    const attrs = this.attributes.map(({ name, value }) => ` ${name}="${value}"`).join("");
    return `<${this.localName}${attrs}>${this.childNodes.map((node) => (node.nodeType === 3 ? node.nodeValue : node.outerHTML)).join("")}</${this.localName}>`;
  }
  querySelectorAll(selector) {
    return queryAll(this, selector);
  }
}

const h = (tag, attrs, ...kids) => new El(tag, attrs, kids);
const descendants = (root) => root.children.flatMap((child) => [child, ...descendants(child)]);

/** One step of a selector: a tag with an id, classes and an nth-of-type, as the picker writes them. */
function parseCompound(part) {
  const m = /^([a-z][a-z0-9-]*)((?:[#.](?:\\.|[\w-])+)*)(?::nth-of-type\((\d+)\))?$/i.exec(part);
  if (!m) throw new SyntaxError(`unsupported selector: ${part}`);
  const tokens = [...m[2].matchAll(/([#.])((?:\\.|[\w-])+)/g)].map(([, kind, name]) => [kind, name.replace(/\\(.)/g, "$1")]);
  return { tag: m[1], id: tokens.find(([kind]) => kind === "#")?.[1], classes: tokens.filter(([kind]) => kind === ".").map(([, name]) => name), nth: m[3] ? Number(m[3]) : 0 };
}

function matchesCompound(el, { tag, id, classes, nth }) {
  if (el.localName !== tag || (id !== undefined && el.id !== id) || !classes.every((name) => el.classList.includes(name))) return false;
  const sameType = (el.parentNode?.children ?? [el]).filter((other) => other.tagName === el.tagName);
  return !nth || sameType.indexOf(el) + 1 === nth;
}

/** The descendants a selector of child combinators matches. */
function queryAll(root, selector) {
  const steps = selector.split(" > ").map(parseCompound);
  return descendants(root).filter((el) => {
    let current = el;
    for (let i = steps.length - 1; i >= 0; i -= 1) {
      if (!current || !matchesCompound(current, steps[i])) return false;
      current = current.parentElement;
    }
    return true;
  });
}

/** acme/web's checkout page, and the picker's helpers evaluated against it. */
function checkout() {
  const pay = h("button", { class: "pay primary", type: "submit", "aria-label": "Pay now", "data-token": "abc", onclick: "pay()" }, "Pay ", h("span", {}, "now"));
  const email = h("input", { id: "email", type: "email", name: "email", placeholder: "maya@example.com", value: "maya@example.com" });
  const items = h("ul", { class: "items" }, h("li", {}, "Keyboard"), h("li", {}, "Mouse"), h("li", {}, "Cable"));
  const totals = h("ul", { class: "totals" }, h("li", {}, "Subtotal"), h("li", {}, "Shipping"));
  const total = h("p", { class: "total css-1x2y3z" }, "Order total:\n      42.00\tEUR");
  const back = h("a", { class: "back", href: "http://localhost:3000/cart?session=7#items" }, "Back to cart");
  const form = h("form", { id: "checkout", class: "checkout-form" }, h("label", { for: "email" }, "Email"), email, total, items, totals, pay, back);
  const main = h("main", { id: "app", role: "main" }, h("h1", {}, "Checkout"), form);
  const body = h("body", {}, main);
  const html = h("html", {}, body);
  const document = {
    documentElement: html,
    body,
    title: "Checkout - acme/web",
    getElementById: (id) => descendants(html).find((el) => el.id === id) ?? null,
    querySelectorAll: (selector) => queryAll({ children: [html] }, selector),
    createTreeWalker(root, _show, filter) {
      const texts = [];
      const walk = (node) => {
        for (const kid of node.childNodes ?? []) {
          if (kid.nodeType !== 3) walk(kid);
          else if (!filter || filter.acceptNode(kid) === 1) texts.push(kid);
        }
      };
      walk(root);
      return { nextNode: () => texts.shift() ?? null };
    },
  };
  const window = {
    selection: null,
    getSelection: () => window.selection,
    getComputedStyle: (el) => ({ position: el.css?.position ?? "static", getPropertyValue: (name) => el.css?.[name] ?? "" }),
    location: { href: "http://localhost:3000/checkout?step=2#pay" },
    innerWidth: 1280,
    innerHeight: 800,
    scrollX: 0,
    scrollY: 120,
    devicePixelRatio: 2,
  };
  const context = { window, document, URL, Node: { TEXT_NODE: 3, ELEMENT_NODE: 1 }, NodeFilter: { SHOW_TEXT: 4, FILTER_ACCEPT: 1, FILTER_REJECT: 2 } };
  return { window, document, html, body, main, form, pay, email, items, totals, total, back, helpers: vm.runInNewContext(HELPERS_SCRIPT, context) };
}

/** A value from the vm realm as plain data of this one. */
const plain = (value) => JSON.parse(JSON.stringify(value));

test("strings are clamped, secrets spotted and URLs stripped in the page as in the main process", () => {
  const { helpers } = checkout();
  assert.equal(helpers.clampStr("a".repeat(12), 10), `${"a".repeat(10)} (truncated)`);
  assert.equal(helpers.clampStr(42, 10), "");
  assert.equal(helpers.containsSecret("X-Amz-Signature=1"), true);
  assert.equal(helpers.containsSecret("checkout-form"), false);
  assert.equal(helpers.sanitizeUrl("http://localhost:3000/cart?session=7#items"), "http://localhost:3000/cart");
  assert.equal(helpers.sanitizeUrl("javascript:alert(1)"), "");
  assert.equal(helpers.sanitizeUrl("/cart"), "");
  assert.equal(helpers.sanitizeUrl("about:blank"), "about:blank");
});

test("text is read across nodes with single spaces, within its budget", () => {
  const page = checkout();
  assert.equal(page.helpers.getBoundedText(page.pay, 200), "Pay now");
  assert.equal(page.helpers.getBoundedText(page.total, 200), "Order total: 42.00 EUR");
  assert.equal(page.helpers.getBoundedText(page.form, 20), "Email Order total: 4 (truncated)");
  const many = h("div", {}, ...Array.from({ length: 200 }, (_, i) => h("i", {}, `w${i}`)));
  assert.equal(page.helpers.getBoundedText(many, 5000).split(" ").length, 80);
});

test("only safe attributes are read, with credentials redacted and URLs stripped", () => {
  const page = checkout();
  assert.deepEqual(plain(page.helpers.getSafeAttributes(page.pay)), { class: "pay primary", type: "submit", "aria-label": "Pay now" });
  assert.deepEqual(plain(page.helpers.getSafeAttributes(page.back)), { class: "back", href: "http://localhost:3000/cart" });
  assert.deepEqual(plain(page.helpers.getSafeAttributes(page.email)), { id: "email", type: "email", name: "email", placeholder: "maya@example.com" });
  const leaky = h("a", { href: "http://localhost:3000/callback?access_token=abc", title: "csrf", class: "x".repeat(300) });
  assert.deepEqual(plain(page.helpers.getSafeAttributes(leaky)), { href: "[redacted]", title: "[redacted]", class: `${"x".repeat(200)} (truncated)` });
});

test("the accessible name comes from aria-label, labelled-by ids, the element's text, then title or alt", () => {
  const page = checkout();
  assert.deepEqual(plain(page.helpers.getAccessibility(page.pay)), { role: "button", accessibleName: "Pay now", ariaLabel: "Pay now", ariaLabelledBy: null });
  assert.equal(page.helpers.getAccessibility(page.back).accessibleName, "Back to cart");
  page.form.append(h("span", { id: "sum-label" }, "Order"));
  page.form.append(h("span", { id: "sum-unit" }, "in EUR"));
  const sum = h("output", { "aria-labelledby": " sum-label\tmissing  sum-unit ", role: "status" });
  assert.deepEqual(plain(page.helpers.getAccessibility(sum)), { role: "status", accessibleName: "Order in EUR", ariaLabel: null, ariaLabelledBy: " sum-label\tmissing  sum-unit " });
  assert.equal(page.helpers.getAccessibility(h("img", { alt: "acme logo" })).accessibleName, "acme logo");
  assert.equal(page.helpers.getAccessibility(h("div", { title: "Cart" })).accessibleName, "Cart");
  assert.equal(page.helpers.getAccessibility(h("div", {}, "plain")).accessibleName, null);
  assert.equal(page.helpers.getAriaLabelledByIds(Array.from({ length: 100 }, (_, i) => `id${i}`).join(" ")).length, 32);
});

test("generated and secret-looking class names are not used", () => {
  const { helpers } = checkout();
  const el = h("div", { class: `card css-1x2y3z sc-AxjAm1Kd9QzP api_key-box ${"w".repeat(61)} wide tall` });
  assert.deepEqual(plain(helpers.getStableClasses(el, 5)), ["card", "wide", "tall"]);
  assert.deepEqual(plain(helpers.getStableClasses(el, 2)), ["card", "wide"]);
  assert.equal(helpers.looksHashy("checkout-form-wide"), false);
  assert.equal(helpers.looksHashy("aB3dE6gH9jK2"), true);
});

test("a selector is the shortest unique path of ids, stable classes and nth-of-type", () => {
  const page = checkout();
  const { buildSelector } = page.helpers;
  assert.equal(buildSelector(page.email), "input#email");
  assert.equal(buildSelector(page.pay), "button.pay.primary");
  assert.equal(buildSelector(page.total), "p.total");
  assert.equal(buildSelector(page.items.children[1]), "ul.items > li:nth-of-type(2)");
  assert.equal(buildSelector(page.items.children[0]), "ul.items > li:nth-of-type(1)");
  assert.equal(buildSelector(page.totals.children[1]), "ul.totals > li:nth-of-type(2)");
  assert.equal(buildSelector(page.pay.children[0]), "span");
  for (const el of [page.email, page.pay, page.items.children[1], page.totals.children[1]]) {
    assert.deepEqual(page.document.querySelectorAll(buildSelector(el)), [el]);
  }
});

test("ids are escaped for CSS, and a secret-looking id is skipped", () => {
  const page = checkout();
  const odd = page.form.append(h("button", { id: "pay:now" }, "Pay later"));
  assert.equal(page.helpers.cssEscape("pay:now"), "pay\\:now");
  assert.equal(page.helpers.buildSelector(odd), "button#pay\\:now");
  assert.deepEqual(page.document.querySelectorAll(page.helpers.buildSelector(odd)), [odd]);
  const hidden = page.form.append(h("input", { id: "csrf_token", class: "hidden-field", type: "hidden" }));
  assert.equal(page.helpers.buildSelector(hidden), "input.hidden-field");
});

test("the readable path, the full path and the ancestors describe where the element sits", () => {
  const page = checkout();
  assert.equal(page.helpers.buildReadablePath(page.pay), '#app > #checkout > button[aria-label="Pay now"]');
  assert.equal(page.helpers.buildReadablePath(page.items.children[1]), "#app > #checkout > .items > li");
  assert.equal(page.helpers.buildFullPath(page.pay), "body > main#app > form#checkout > button.pay.primary");
  assert.deepEqual(plain(page.helpers.getAncestorPath(page.pay)), ["form", "main[role=main]", "body"]);
});

test("nearby text and elements alternate between the siblings before and after", () => {
  const page = checkout();
  assert.deepEqual(plain(page.helpers.getNearbyText(page.total)), ["Keyboard Mouse Cable", "Email", "Subtotal Shipping", "Pay now", "Back to cart"]);
  page.email.rect = { x: 0, y: 0, width: 0, height: 0 };
  assert.deepEqual(plain(page.helpers.getNearbyElements(page.total)), ['ul.items "Keyboard Mouse Cable"', 'label "Email"', 'ul.totals "Subtotal Shipping"', 'button.pay "Pay now"', 'a.back "Back to cart"']);
  const row = h("div", {}, ...Array.from({ length: 30 }, (_, i) => h("span", {}, `cell ${i}`)));
  assert.equal(page.helpers.getNearbyText(row.children[15]).length, 10);
  assert.equal(page.helpers.getNearbyElements(row.children[15]).length, 6);
  assert.deepEqual(plain(page.helpers.getNearbyText(page.html)), []);
});

test("the HTML snippet has no script and stays within its budget", () => {
  const { helpers } = checkout();
  const card = h("div", { class: "card" }, h("script", {}, "track('Priya')"), h("p", {}, "Total"), h("section", {}, h("script", {}, "more()")));
  assert.equal(helpers.getHtmlSnippet(card), '<div class="card"><p>Total</p><section></section></div>');
  assert.equal(card.children.length, 3);
  const big = h("p", {}, "x".repeat(5000));
  assert.equal(helpers.getHtmlSnippet(big).length, 4096 + " (truncated)".length);
});

test("the selected text is the part of the range inside its text node", () => {
  const page = checkout();
  assert.equal(page.helpers.getSelectedText(), "");
  const text = page.total.childNodes[0];
  const range = { commonAncestorContainer: text, startContainer: text, startOffset: 6, endContainer: text, endOffset: 25, intersectsNode: () => true };
  page.window.selection = { rangeCount: 1, getRangeAt: () => range };
  assert.equal(page.helpers.getSelectedText(), "total: 42.00");
});

test("computed styles are read by their CSS names", () => {
  const page = checkout();
  page.pay.css = { display: "inline-flex", "background-color": "rgb(17, 24, 39)", "border-radius": "6px", "z-index": "2" };
  const styles = plain(page.helpers.getComputedStyleSubset(page.pay));
  assert.equal(Object.keys(styles).length, 16);
  assert.deepEqual([styles.display, styles.backgroundColor, styles.borderRadius, styles.zIndex, styles.fontSize], ["inline-flex", "rgb(17, 24, 39)", "6px", "2", ""]);
});

test("React components and the source file come from the element's fiber", () => {
  const page = checkout();
  function PayButton() {}
  function Checkout() {}
  function ThemeProvider() {}
  function App() {}
  const root = { type: App, return: null };
  const provider = { type: ThemeProvider, return: root };
  const checkoutFiber = { type: Checkout, return: provider };
  const memo = { type: { type: PayButton }, return: checkoutFiber, _debugSource: { fileName: "webpack-internal:///./src/checkout/PayButton.tsx?v=1", lineNumber: 42, columnNumber: 7 } };
  page.pay["__reactFiber$acme1"] = { type: "button", return: memo };
  assert.deepEqual(plain(page.helpers.getReactMetadata(page.pay)), { reactComponents: "<App> <Checkout> <PayButton>", sourceFile: "src/checkout/PayButton.tsx:42:7" });

  page.back["__reactInternalInstance$acme1"] = { type: "a", _debugOwner: { _debugSource: { fileName: "/work/acme/web/src/BackLink.tsx", lineNumber: 9 } }, return: null };
  assert.deepEqual(plain(page.helpers.getReactMetadata(page.back)), { reactComponents: null, sourceFile: "/work/acme/web/src/BackLink.tsx:9" });

  page.email["__reactFiber$acme1"] = { type: "input", _debugSource: { fileName: "src/auth/password/Field.tsx", lineNumber: 3 }, return: null };
  assert.deepEqual(plain(page.helpers.getReactMetadata(page.email)), { reactComponents: null, sourceFile: null });
  assert.deepEqual(plain(page.helpers.getReactMetadata(page.total)), { reactComponents: null, sourceFile: null });
});

test("bundler prefixes are removed from source paths, and wrapper components are skipped", () => {
  const { helpers } = checkout();
  const cleaned = {
    "turbopack:///[project]/src/app/page.tsx": "src/app/page.tsx",
    "webpack-internal:///./src/Cart.tsx": "src/Cart.tsx",
    "webpack-internal:///(app-pages-browser)/./src/Cart.tsx": "src/Cart.tsx",
    "webpack:///./src/Cart.tsx?abcd": "src/Cart.tsx",
    "webpack:///src/Cart.tsx": "src/Cart.tsx",
    "http://localhost:3000/src/Cart.tsx?t=1700000000#x": "src/Cart.tsx",
    "file:///work/acme/web/src/Cart.tsx": "/work/acme/web/src/Cart.tsx",
    "./src/Cart.tsx": "src/Cart.tsx",
    "": "",
  };
  for (const [from, to] of Object.entries(cleaned)) assert.equal(helpers.cleanSourcePath(from), to, from);
  for (const name of ["Fragment", "Suspense", "ErrorBoundary", "AppRouter", "ThemeProvider", "LayoutWrapper", "InnerLayoutRouter", "HotReload", "Go", ""]) {
    assert.equal(helpers.shouldSkipReactName(name), true, name);
  }
  for (const name of ["PayButton", "Checkout", "App"]) assert.equal(helpers.shouldSkipReactName(name), false, name);
});

test("the composed tree is walked through slots and shadow hosts", () => {
  const page = checkout();
  const host = page.form.append(h("acme-drawer", {}));
  const root = { nodeType: 11, host, childNodes: [], parentNode: null };
  const inner = h("div", {}, h("button", {}, "Close"));
  inner.parentNode = root;
  const slot = h("slot", {});
  const slotted = h("span", {}, "Slotted");
  slotted.assignedSlot = slot;
  assert.equal(page.helpers.composedParent(inner), host);
  assert.equal(page.helpers.composedParent(inner.children[0]), inner);
  assert.equal(page.helpers.composedParent(slotted), slot);
  assert.equal(page.helpers.composedParent(page.html), null);
  // A form control named "host" shadows the property; only a real fragment counts as a shadow root.
  const shadowed = h("input", { name: "host" });
  shadowed.parentNode = { nodeType: 1, host: shadowed, tagName: "FORM" };
  assert.equal(page.helpers.composedParent(shadowed), shadowed.parentNode);
  assert.deepEqual([...page.helpers.shadowRootsAbove(inner.children[0])], [root]);
  assert.deepEqual([...page.helpers.shadowRootsAbove(page.pay)], []);

  assert.equal(page.helpers.isElementFixed(inner.children[0]), false);
  host.css = { position: "fixed" };
  assert.equal(page.helpers.isElementFixed(inner.children[0]), true);
  assert.equal(page.helpers.isElementFixed(page.pay), false);
});

test("a whole payload is extracted and passes the main-process clamp unchanged", () => {
  const page = checkout();
  page.pay.rect = { x: 40.5, y: 300, width: 120, height: 36 };
  page.pay.css = { display: "inline-flex", "font-size": "14px" };
  const payload = plain(page.helpers.extractPayload(page.pay));
  assert.deepEqual(Object.keys(payload), ["page", "target", "nearbyText", "ancestorPath", "screenshot"]);
  assert.deepEqual({ ...payload.page, capturedAt: "" }, {
    sanitizedUrl: "http://localhost:3000/checkout",
    title: "Checkout - acme/web",
    viewportWidth: 1280,
    viewportHeight: 800,
    scrollX: 0,
    scrollY: 120,
    devicePixelRatio: 2,
    capturedAt: "",
  });
  assert.match(payload.page.capturedAt, /^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{3}Z$/);
  assert.equal(payload.target.tagName, "button");
  assert.equal(payload.target.selector, "button.pay.primary");
  assert.equal(payload.target.cssClasses, "pay primary");
  assert.equal(payload.target.textSnippet, "Pay now");
  assert.equal(payload.target.selectedText, null);
  assert.equal(payload.target.isFixed, false);
  assert.deepEqual(payload.target.rectViewport, { x: 40.5, y: 300, width: 120, height: 36 });
  assert.deepEqual(payload.target.rectPage, { x: 40.5, y: 420, width: 120, height: 36 });
  assert.equal(payload.target.htmlSnippet, '<button class="pay primary" type="submit" aria-label="Pay now" data-token="abc" onclick="pay()">Pay <span>now</span></button>');
  assert.equal(payload.screenshot, null);
  assert.deepEqual(clampGrabPayload(payload), payload);

  const secret = h("div", { class: "password-reveal" }, "x");
  page.form.append(secret);
  assert.equal(page.helpers.extractPayload(secret).target.cssClasses, "[redacted]");
});
