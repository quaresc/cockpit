// What the main process accepts from a page for a pick: bounded, filtered and redacted.

import assert from "node:assert/strict";
import { test } from "node:test";
import {
  GRAB_BUDGET,
  clampGrabPayload,
  containsGrabSecret,
  isAnnotationPageFull,
  readGrabFailure,
  readGrabSelection,
  sanitizeAnnotation,
  sanitizeGrabUrl,
} from "../main/grab/index.js";

const TRUNCATED = " (truncated)";

/** What the page returns for a pick on acme/web's sign-in form; `target` and `page` override single fields. */
function raw({ target = {}, page = {}, ...rest } = {}) {
  return {
    page: {
      sanitizedUrl: "http://localhost:3000/signin",
      title: "Sign in - acme/web",
      viewportWidth: 1280,
      viewportHeight: 800,
      scrollX: 0,
      scrollY: 0,
      devicePixelRatio: 2,
      capturedAt: "2026-01-12T09:30:00.000Z",
      ...page,
    },
    target: {
      tagName: "a",
      selector: "form#signin > a.reset",
      elementPath: "#signin > a",
      fullPath: "html > body > form#signin > a.reset",
      cssClasses: "reset link",
      nearbyElements: ['button.submit "Sign in"'],
      selectedText: null,
      isFixed: false,
      reactComponents: "<SignIn> <ResetLink>",
      sourceFile: "src/signin/ResetLink.tsx:12:5",
      textSnippet: "Forgot your password?",
      htmlSnippet: '<a class="reset link" href="/reset">Forgot your password?</a>',
      attributes: { class: "reset link", href: "http://localhost:3000/reset" },
      accessibility: { role: "a", accessibleName: "Forgot your password?", ariaLabel: null, ariaLabelledBy: null },
      rectViewport: { x: 40, y: 300, width: 160, height: 20 },
      rectPage: { x: 40, y: 300, width: 160, height: 20 },
      computedStyles: { display: "inline", position: "static", color: "rgb(37, 99, 235)", fontSize: "14px" },
      ...target,
    },
    nearbyText: ["Email", "Sign in"],
    ancestorPath: ["form", "body"],
    screenshot: null,
    ...rest,
  };
}

test("a value without a page and a target object is refused", () => {
  for (const value of [null, undefined, "payload", 42, [], {}, { page: {} }, { target: {} }, { page: "x", target: {} }, { page: {}, target: 1 }]) {
    assert.equal(clampGrabPayload(value), null);
  }
});

test("a well-formed payload keeps its content", () => {
  const payload = clampGrabPayload(raw());
  assert.equal(payload.page.sanitizedUrl, "http://localhost:3000/signin");
  assert.equal(payload.target.selector, "form#signin > a.reset");
  assert.equal(payload.target.sourceFile, "src/signin/ResetLink.tsx:12:5");
  assert.deepEqual(payload.target.attributes, { class: "reset link", href: "http://localhost:3000/reset" });
  assert.deepEqual(payload.target.rectViewport, { x: 40, y: 300, width: 160, height: 20 });
  assert.deepEqual(payload.nearbyText, ["Email", "Sign in"]);
  assert.equal(payload.target.computedStyles.fontSize, "14px");
  assert.equal(payload.target.computedStyles.zIndex, "");
});

test("strings and lists are cut to their budgets", () => {
  const long = (n) => "a".repeat(n + 50);
  const payload = clampGrabPayload(
    raw({
      page: { title: long(500), capturedAt: long(100) },
      target: {
        tagName: long(50),
        selector: long(GRAB_BUDGET.selectorMaxLength),
        elementPath: long(GRAB_BUDGET.pathMaxLength),
        cssClasses: long(GRAB_BUDGET.cssClassesMaxLength),
        textSnippet: long(GRAB_BUDGET.textSnippetMaxLength),
        htmlSnippet: long(GRAB_BUDGET.htmlSnippetMaxLength),
        nearbyElements: Array.from({ length: 30 }, () => long(GRAB_BUDGET.nearbyElementMaxLength)),
        attributes: { class: long(200), title: long(500) },
      },
      nearbyText: Array.from({ length: 30 }, () => long(GRAB_BUDGET.nearbyTextEntryMaxLength)),
      ancestorPath: Array.from({ length: 30 }, () => long(200)),
    }),
  );
  const cut = (n) => "a".repeat(n) + TRUNCATED;
  assert.equal(payload.page.title, cut(500));
  assert.equal(payload.page.capturedAt, cut(100));
  assert.equal(payload.target.tagName, cut(50));
  assert.equal(payload.target.selector, cut(700));
  assert.equal(payload.target.elementPath, cut(900));
  assert.equal(payload.target.cssClasses, cut(500));
  assert.equal(payload.target.textSnippet, cut(200));
  assert.equal(payload.target.htmlSnippet, cut(4096));
  assert.deepEqual(payload.target.nearbyElements, Array(6).fill(cut(160)));
  assert.deepEqual(payload.target.attributes, { class: cut(200), title: cut(500) });
  assert.deepEqual(payload.nearbyText, Array(10).fill(cut(200)));
  assert.deepEqual(payload.ancestorPath, Array(10).fill(cut(200)));
});

test("forged types become empty strings, zeros and empty lists", () => {
  const payload = clampGrabPayload({
    page: { sanitizedUrl: 7, title: { toString: () => "x" }, viewportWidth: "1280", scrollY: Number.NaN, devicePixelRatio: Infinity },
    target: {
      tagName: ["a"],
      selector: null,
      isFixed: "true",
      attributes: "class=x",
      accessibility: "button",
      rectViewport: { x: "1", y: Infinity, width: -Infinity, height: null },
      rectPage: "0,0",
      computedStyles: null,
      nearbyElements: "x",
    },
    nearbyText: { 0: "a", length: 1 },
    ancestorPath: [1, null, "main"],
    screenshot: { mimeType: "image/png", dataUrl: "data:image/png;base64,AAAA", width: 1, height: 1 },
  });
  assert.deepEqual(payload.page, { sanitizedUrl: "", title: "", viewportWidth: 0, viewportHeight: 0, scrollX: 0, scrollY: 0, devicePixelRatio: 1, capturedAt: "" });
  assert.equal(payload.target.tagName, "");
  assert.equal(payload.target.selector, "");
  assert.equal(payload.target.isFixed, false);
  assert.deepEqual(payload.target.attributes, {});
  assert.deepEqual(payload.target.accessibility, { role: null, accessibleName: null, ariaLabel: null, ariaLabelledBy: null });
  assert.deepEqual(payload.target.rectViewport, { x: 0, y: 0, width: 0, height: 0 });
  assert.deepEqual(payload.target.rectPage, { x: 0, y: 0, width: 0, height: 0 });
  assert.equal(payload.target.computedStyles.display, "");
  assert.deepEqual(payload.target.nearbyElements, []);
  assert.deepEqual(payload.nearbyText, []);
  assert.deepEqual(payload.ancestorPath, ["", "", "main"]);
  assert.equal(payload.screenshot, null);
});

test("only safe and aria attributes are kept, with their names in lower case", () => {
  const attributes = {
    ID: "reset",
    "ARIA-Label": "Start over",
    "aria-describedby": "hint",
    onclick: "steal()",
    style: "color:red",
    value: "maya@example.com",
    "data-user": "42",
    placeholder: "you@example.com",
  };
  assert.deepEqual(clampGrabPayload(raw({ target: { attributes } })).target.attributes, {
    id: "reset",
    "aria-label": "Start over",
    "aria-describedby": "hint",
    placeholder: "you@example.com",
  });
});

test("attribute values that look like credentials are redacted", () => {
  const attributes = {
    href: "http://localhost:3000/callback?access_token=abc123",
    name: "csrf-token",
    title: "Your API_KEY is shown here",
    alt: "X-Amz-Signature",
    class: "password-field",
    id: "plain",
  };
  assert.deepEqual(clampGrabPayload(raw({ target: { attributes } })).target.attributes, {
    href: "[redacted]",
    name: "[redacted]",
    title: "[redacted]",
    alt: "[redacted]",
    class: "[redacted]",
    id: "plain",
  });
});

test("URLs in attributes lose their query and fragment, and unsafe schemes are emptied", () => {
  const attributes = {
    href: "http://localhost:3000/orders?customer=42&page=2#latest",
    src: "https://cdn.example.com/logo.png?v=3",
    action: "javascript:alert(1)",
  };
  assert.deepEqual(clampGrabPayload(raw({ target: { attributes } })).target.attributes, {
    href: "http://localhost:3000/orders",
    src: "https://cdn.example.com/logo.png",
    action: "",
  });
  assert.deepEqual(clampGrabPayload(raw({ target: { attributes: { href: "/orders?page=2", src: "data:image/png;base64,AAAA" } } })).target.attributes, { href: "", src: "" });
});

test("metadata that looks like a credential is redacted, while page text stays", () => {
  const payload = clampGrabPayload(
    raw({
      target: {
        elementPath: "#signin > input[aria-label=\"password\"]",
        fullPath: "html > body > form#session_id-form",
        cssClasses: "field client_secret",
        selectedText: "my passwd is hunter2",
        reactComponents: "<SecretField>",
        sourceFile: "src/auth/apikey.tsx:3:1",
        nearbyElements: ['label "Password"', 'button.submit "Sign in"'],
        accessibility: { role: "textbox", accessibleName: "Password", ariaLabel: "Password", ariaLabelledBy: "csrf-label" },
        textSnippet: "Forgot your password?",
      },
    }),
  );
  assert.equal(payload.target.elementPath, "[redacted]");
  assert.equal(payload.target.fullPath, "[redacted]");
  assert.equal(payload.target.cssClasses, "[redacted]");
  assert.equal(payload.target.selectedText, "[redacted]");
  assert.equal(payload.target.reactComponents, "[redacted]");
  assert.equal(payload.target.sourceFile, "[redacted]");
  assert.deepEqual(payload.target.nearbyElements, ["[redacted]", 'button.submit "Sign in"']);
  assert.deepEqual(payload.target.accessibility, { role: "textbox", accessibleName: "[redacted]", ariaLabel: "[redacted]", ariaLabelledBy: "[redacted]" });
  assert.equal(payload.target.textSnippet, "Forgot your password?");
});

test("empty metadata becomes null where the payload allows it", () => {
  const payload = clampGrabPayload(raw({ target: { selectedText: "", reactComponents: "", sourceFile: undefined, nearbyElements: ["", "a.back"] } }));
  assert.equal(payload.target.selectedText, null);
  assert.equal(payload.target.reactComponents, null);
  assert.equal(payload.target.sourceFile, null);
  assert.deepEqual(payload.target.nearbyElements, ["a.back"]);
});

test("the page URL loses its query and fragment", () => {
  const url = (sanitizedUrl) => clampGrabPayload(raw({ page: { sanitizedUrl } })).page.sanitizedUrl;
  assert.equal(url("http://localhost:3000/signin?token=abc&next=/orders#form"), "http://localhost:3000/signin");
  assert.equal(url("https://app.example.com/a/b?session_id=1"), "https://app.example.com/a/b");
  assert.equal(url("file:///tmp/acme-web/dist/index.html?draft=1"), "file:///tmp/acme-web/dist/index.html");
  assert.equal(url("about:blank"), "about:blank");
  assert.equal(url("about:srcdoc"), "");
  assert.equal(url("chrome://settings/passwords"), "");
  assert.equal(url("data:text/html,<p>hi</p>"), "");
  assert.equal(url("not a url"), "");
  assert.equal(sanitizeGrabUrl("http://localhost:3000/?api_key=1"), "http://localhost:3000/");
});

test("a file document is named by its local path", () => {
  const path = (documentUrl) => clampGrabPayload(raw({ page: { sanitizedUrl: "file:///tmp/acme-web/dist/index.html" } }), documentUrl).page.sanitizedUrl;
  assert.equal(path("file:///tmp/acme-web/dist/index.html?draft=1#top"), "/tmp/acme-web/dist/index.html");
  assert.equal(path("file:///tmp/acme%20web/index.html"), "/tmp/acme web/index.html");
  assert.equal(path("file:///tmp/acme-web/\u202Egnp.html"), "file:///tmp/acme-web/dist/index.html");
  assert.equal(path("http://localhost:3000/signin"), "file:///tmp/acme-web/dist/index.html");
  assert.equal(path(""), "file:///tmp/acme-web/dist/index.html");
});

test("secret patterns match in any case, and ordinary words do not", () => {
  for (const value of ["ACCESS_TOKEN=1", "x-amz-security-token", "sessionId", "Passwd", "oauth_state"]) assert.equal(containsGrabSecret(value), true);
  for (const value of ["code", "state", "token-list", "btn-primary", "", null]) assert.equal(containsGrabSecret(value), false);
});

test("a kept note has a bounded comment and no screenshot", () => {
  const payload = { ...clampGrabPayload(raw()), screenshot: { mimeType: "image/png", dataUrl: "data:image/png;base64,AAAA", width: 1, height: 1 } };
  const note = { id: "note-1", browserPageId: "tab-1", comment: "a".repeat(GRAB_BUDGET.annotationCommentMaxLength + 10), intent: "fix", priority: "important", createdAt: "2026-01-12T09:31:00.000Z", payload };
  const kept = sanitizeAnnotation(note);
  assert.equal(kept.comment.length, 2000);
  assert.equal(kept.payload.screenshot, null);
  assert.equal(kept.payload.target.selector, "form#signin > a.reset");
  assert.equal(sanitizeAnnotation({ ...note, comment: "Short." }).comment, "Short.");
  assert.equal(note.payload.screenshot.width, 1);
});

test("a page holds at most twenty notes", () => {
  assert.equal(GRAB_BUDGET.annotationsMaxPerPage, 20);
  assert.equal(isAnnotationPageFull(19), false);
  assert.equal(isAnnotationPageFull(20), true);
});

test("an awaitClick result reads as a click, a right-click or a cancelled pick", () => {
  const selected = readGrabSelection(raw({ page: { sanitizedUrl: "http://localhost:3000/signin?next=/orders" } }));
  assert.equal(selected.kind, "selected");
  assert.equal(selected.payload.page.sanitizedUrl, "http://localhost:3000/signin");

  const context = readGrabSelection({ __cockpitContextMenu: true, payload: raw() });
  assert.equal(context.kind, "context-selected");
  assert.equal(context.payload.target.tagName, "a");

  assert.deepEqual(readGrabSelection({ __cockpitCancelled: true }), { kind: "cancelled" });
  assert.deepEqual(readGrabSelection({ message: "cancelled" }), { kind: "cancelled" });
  for (const value of [null, undefined, true, "cancelled"]) assert.deepEqual(readGrabSelection(value), { kind: "cancelled" });
});

test("a page cannot fake a cancellation or a selection", () => {
  assert.equal(readGrabSelection({ ...raw(), message: "cancelled" }).kind, "selected");
  assert.equal(readGrabSelection({ ...raw(), __cockpitContextMenu: "yes" }).kind, "selected");
  assert.deepEqual(readGrabSelection({ __cockpitContextMenu: true, payload: "x" }), { kind: "error", reason: "The page returned an invalid payload" });
  assert.deepEqual(readGrabSelection({ page: {} }), { kind: "error", reason: "The page returned an invalid payload" });
});

test("a rejected awaitClick reads as a cancelled pick or an error", () => {
  assert.deepEqual(readGrabFailure(new Error("cancelled")), { kind: "cancelled" });
  assert.deepEqual(readGrabFailure(new Error("Script failed to execute: Error: cancelled")), { kind: "cancelled" });
  assert.deepEqual(readGrabFailure(new Error("Grab not armed")), { kind: "error", reason: "Grab not armed" });
  assert.deepEqual(readGrabFailure({ message: "Object has been destroyed" }), { kind: "error", reason: "Object has been destroyed" });
  assert.deepEqual(readGrabFailure(undefined), { kind: "error", reason: "Selection failed" });
});
