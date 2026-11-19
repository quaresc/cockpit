// The Markdown a coding agent receives for design notes, and the plain text a grab copies.

import assert from "node:assert/strict";
import { test } from "node:test";
import { ANNOTATION_INLINE_TEXT_MAX_LENGTH, formatAnnotationsAsMarkdown, formatGrabPayloadAsText } from "../main/grab/index.js";

const STYLES = {
  display: "inline-flex",
  position: "static",
  width: "120px",
  height: "36px",
  margin: "0px",
  padding: "8px 16px",
  color: "rgb(255, 255, 255)",
  backgroundColor: "rgb(17, 24, 39)",
  border: "0px none rgb(255, 255, 255)",
  borderRadius: "6px",
  fontFamily: "Inter, sans-serif",
  fontSize: "14px",
  fontWeight: "600",
  lineHeight: "normal",
  textAlign: "center",
  zIndex: "auto",
};

/** A pick of the Pay button on acme/web's checkout page; `target` and `page` override single fields. */
function payload({ target = {}, page = {}, ...rest } = {}) {
  return {
    page: {
      sanitizedUrl: "http://localhost:3000/checkout",
      title: "Checkout - acme/web",
      viewportWidth: 1280,
      viewportHeight: 800,
      scrollX: 0,
      scrollY: 120,
      devicePixelRatio: 2,
      capturedAt: "2026-01-12T09:30:00.000Z",
      ...page,
    },
    target: {
      tagName: "button",
      selector: "form#checkout > button.pay",
      elementPath: "#checkout > button",
      fullPath: "html > body > main > form#checkout > button.pay",
      cssClasses: "pay primary",
      nearbyElements: ['a.back "Back to cart"'],
      selectedText: null,
      isFixed: false,
      reactComponents: "<Checkout> <PayButton>",
      sourceFile: "src/checkout/PayButton.tsx:42:7",
      textSnippet: "Pay now",
      htmlSnippet: '<button class="pay primary" type="submit">Pay now</button>',
      attributes: { class: "pay primary", type: "submit" },
      accessibility: { role: "button", accessibleName: "Pay now", ariaLabel: null, ariaLabelledBy: null },
      rectViewport: { x: 40.4, y: 300.6, width: 120, height: 36 },
      rectPage: { x: 40.4, y: 420.6, width: 120, height: 36 },
      computedStyles: STYLES,
      ...target,
    },
    nearbyText: ["Order total", "Back to cart"],
    ancestorPath: ["form", "main", "body"],
    screenshot: null,
    ...rest,
  };
}

/** A note by Maya on a pick. */
function note(fields = {}) {
  return {
    id: "note-1",
    browserPageId: "tab-1",
    comment: "The button is too close to the total.",
    intent: "fix",
    priority: "important",
    createdAt: "2026-01-12T09:31:00.000Z",
    payload: payload(),
    ...fields,
  };
}

test("no note gives an empty string", () => {
  assert.equal(formatAnnotationsAsMarkdown([]), "");
});

test("one note is a page heading, the URL and one numbered block", () => {
  assert.equal(
    formatAnnotationsAsMarkdown([note()]),
    [
      "## Design Feedback: /checkout",
      "",
      "**URL:** http://localhost:3000/checkout",
      "",
      '### 1. <Checkout> <PayButton> button "Pay now"',
      "**Intent:** fix",
      "**Selector:** `form#checkout > button.pay`",
      "**Source:** src/checkout/PayButton.tsx:42:7",
      '**Text:** "Pay now"',
      "**Feedback:** The button is too close to the total.",
    ].join("\n"),
  );
});

test("notes on one page are numbered under a single heading", () => {
  const second = note({
    id: "note-2",
    intent: "question",
    comment: "Should Sam's copy say 'Back to basket'?",
    payload: payload({
      target: {
        tagName: "a",
        selector: "a.back",
        reactComponents: null,
        sourceFile: null,
        textSnippet: "Back to cart",
        accessibility: { role: "a", accessibleName: null, ariaLabel: null, ariaLabelledBy: null },
      },
    }),
  });
  const markdown = formatAnnotationsAsMarkdown([note(), second]);
  assert.equal(markdown.match(/^## /gm).length, 1);
  assert.equal(
    markdown.split("\n\n").at(-1),
    ['### 2. a "Back to cart"', "**Intent:** question", "**Selector:** `a.back`", '**Text:** "Back to cart"', "**Feedback:** Should Sam's copy say 'Back to basket'?"].join("\n"),
  );
});

test("the block title falls back from the accessible name to the clipped text, then to the tag", () => {
  const bare = { reactComponents: null, accessibility: { role: null, accessibleName: null, ariaLabel: null, ariaLabelledBy: null } };
  const long = "Priya's very long promotional banner text that keeps going well past sixty characters";
  const clipped = formatAnnotationsAsMarkdown([note({ payload: payload({ target: { ...bare, tagName: "p", textSnippet: long } }) })]);
  assert.match(clipped, new RegExp(`^### 1\\. p "${long.slice(0, 60)}"$`, "m"));
  const tagOnly = formatAnnotationsAsMarkdown([note({ payload: payload({ target: { ...bare, tagName: "img", textSnippet: "" } }) })]);
  assert.match(tagOnly, /^### 1\. img$/m);
  assert.doesNotMatch(tagOnly, /\*\*Text:\*\*/);
});

test("selected text replaces the element text", () => {
  const markdown = formatAnnotationsAsMarkdown([note({ payload: payload({ target: { selectedText: "Pay\n  now" } }) })]);
  assert.match(markdown, /^\*\*Selected text:\*\* "Pay now"$/m);
  assert.doesNotMatch(markdown, /\*\*Text:\*\*/);
});

test("a comment of several lines stays on one line, and page text is capped", () => {
  const markdown = formatAnnotationsAsMarkdown([note({ comment: "  Too tight.\r\n\r\nAsk\tLeo\u00a0about spacing.  " })]);
  assert.match(markdown, /^\*\*Feedback:\*\* Too tight\. Ask Leo about spacing\.$/m);
  const huge = formatAnnotationsAsMarkdown([note({ comment: "x".repeat(ANNOTATION_INLINE_TEXT_MAX_LENGTH + 500) })]);
  assert.equal(huge.split("\n").at(-1), `**Feedback:** ${"x".repeat(ANNOTATION_INLINE_TEXT_MAX_LENGTH)}`);
});

test("attached images are listed as files to open", () => {
  const markdown = formatAnnotationsAsMarkdown([note({ imagePaths: ["/tmp/acme-web/notes/pay-button.png", "/tmp/acme-web/notes/total.png"] })]);
  assert.equal(
    markdown.split("\n").slice(-3).join("\n"),
    ["**Images (open these files):**", "- `/tmp/acme-web/notes/pay-button.png`", "- `/tmp/acme-web/notes/total.png`"].join("\n"),
  );
});

test("page details add the tab, the viewport, the layout facts and the HTML", () => {
  assert.equal(
    formatAnnotationsAsMarkdown([note()], { includeDiagnostics: true }),
    [
      "## Design Feedback: /checkout",
      "",
      "**URL:** http://localhost:3000/checkout",
      "**Browser tab id:** tab-1",
      "**Viewport:** 1280x800",
      "",
      '### 1. <Checkout> <PayButton> button "Pay now"',
      "**Intent:** fix",
      "**Selector:** `form#checkout > button.pay`",
      "**Source:** src/checkout/PayButton.tsx:42:7",
      '**Text:** "Pay now"',
      "**Location:** `#checkout > button`",
      "**React:** <Checkout> <PayButton>",
      "**Bounds:** x=40, y=301, 120x36",
      "**Classes:** `pay primary`",
      "**Nearby text:**",
      "- Order total",
      "- Back to cart",
      "**Nearby elements:**",
      '- a.back "Back to cart"',
      "**Computed styles:**",
      "- display: inline-flex",
      "- width: 120px",
      "- height: 36px",
      "- margin: 0px",
      "- padding: 8px 16px",
      "- color: rgb(255, 255, 255)",
      "- background: rgb(17, 24, 39)",
      "- border: 0px none rgb(255, 255, 255)",
      "- border-radius: 6px",
      "- font-family: Inter, sans-serif",
      "- font-size: 14px",
      "- font-weight: 600",
      "- text-align: center",
      "**Full DOM path:** `html > body > main > form#checkout > button.pay`",
      "**HTML:**",
      "````html",
      '<button class="pay primary" type="submit">Pay now</button>',
      "````",
      "**Feedback:** The button is too close to the total.",
    ].join("\n"),
  );
});

test("default styles are left out of the page details", () => {
  const styles = { ...STYLES, display: "inline", backgroundColor: "rgba(0, 0, 0, 0)", width: "auto", fontWeight: "" };
  const markdown = formatAnnotationsAsMarkdown([note({ payload: payload({ target: { computedStyles: styles } }) })], { includeDiagnostics: true });
  for (const name of ["display", "position", "background", "width", "font-weight", "line-height", "z-index"]) {
    assert.doesNotMatch(markdown, new RegExp(`^- ${name}:`, "m"));
  }
});

test("notes from several pages are grouped by page and numbered across them", () => {
  const cart = (fields) => note({ browserPageId: "tab-2", payload: payload({ page: { sanitizedUrl: "http://localhost:3000/cart" } }), ...fields });
  const markdown = formatAnnotationsAsMarkdown([note(), cart({ id: "note-2", comment: "Omar: empty state is missing." }), note({ id: "note-3", comment: "Third." })], {
    includeDiagnostics: true,
  });
  const lines = markdown.split("\n");
  assert.equal(lines[0], "## Design Feedback across pages");
  assert.deepEqual(
    lines.filter((line) => /^(## Design Feedback:|\*\*URL:|### |\*\*Browser tab id:)/.test(line)),
    [
      "## Design Feedback: /checkout",
      "**URL:** `http://localhost:3000/checkout`",
      '### 1. <Checkout> <PayButton> button "Pay now"',
      "**Browser tab id:** `tab-1`",
      '### 3. <Checkout> <PayButton> button "Pay now"',
      "**Browser tab id:** `tab-1`",
      "## Design Feedback: /cart",
      "**URL:** `http://localhost:3000/cart`",
      '### 2. <Checkout> <PayButton> button "Pay now"',
      "**Browser tab id:** `tab-2`",
    ],
  );
});

test("backticks in page content cannot close a code span or a fence", () => {
  const target = { selector: "div.a`b", htmlSnippet: "<pre>```\ncode\n````</pre>", fullPath: "`x" };
  const markdown = formatAnnotationsAsMarkdown([note({ payload: payload({ target }) })], { includeDiagnostics: true });
  assert.match(markdown, /^\*\*Selector:\*\* ``div\.a`b``$/m);
  assert.match(markdown, /^\*\*Full DOM path:\*\* `` `x ``$/m);
  assert.match(markdown, /^`````html\n<pre>```\ncode\n````<\/pre>\n`````$/m);
});

test("a local file is headed by its path, and an unknown page by a placeholder", () => {
  const file = formatAnnotationsAsMarkdown([note({ payload: payload({ page: { sanitizedUrl: "/tmp/acme-web/dist/index.html" } }) })]);
  assert.equal(file.split("\n")[0], "## Design Feedback: /tmp/acme-web/dist/index.html");
  const unknown = formatAnnotationsAsMarkdown([note({ payload: payload({ page: { sanitizedUrl: "" } }) })]);
  assert.equal(unknown.split("\n")[0], "## Design Feedback: current page");
});

test("a grab copies the element as plain text", () => {
  assert.equal(
    formatGrabPayloadAsText(payload()),
    [
      "Attached browser context from http://localhost:3000/checkout",
      "",
      "Selected element:",
      "button",
      'Accessible name: "Pay now"',
      "Role: button",
      "Selector: form#checkout > button.pay",
      "Source: src/checkout/PayButton.tsx:42:7",
      "React: <Checkout> <PayButton>",
      "Dimensions: 120x36",
      "",
      "Text content:",
      "Pay now",
      "",
      "Nearby context:",
      "- Order total",
      "- Back to cart",
      "",
      "Computed styles:",
      "  display: inline-flex",
      "  font-size: 14px",
      "  color: rgb(255, 255, 255)",
      "  background: rgb(17, 24, 39)",
      "",
      "HTML:",
      '<button class="pay primary" type="submit">Pay now</button>',
      "",
      "Ancestor path: form > main > body",
      "Full DOM path: html > body > main > form#checkout > button.pay",
    ].join("\n"),
  );
});

test("the copied text leaves out what the element does not have", () => {
  const bare = payload({
    target: {
      tagName: "img",
      selector: "img.logo",
      reactComponents: null,
      sourceFile: null,
      textSnippet: "",
      htmlSnippet: "",
      fullPath: "",
      accessibility: { role: null, accessibleName: null, ariaLabel: null, ariaLabelledBy: null },
      computedStyles: { ...STYLES, display: "inline", fontSize: "", color: "", backgroundColor: "rgba(0, 0, 0, 0)" },
    },
    nearbyText: [],
    ancestorPath: [],
  });
  assert.equal(
    formatGrabPayloadAsText(bare),
    ["Attached browser context from http://localhost:3000/checkout", "", "Selected element:", "img", "Selector: img.logo", "Dimensions: 120x36"].join("\n"),
  );
});
