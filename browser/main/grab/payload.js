// Ported from Orca (https://github.com/stablyai/orca, commit 49498c3), src/main/browser/element-grab/browser-grab-payload.ts,
// src/shared/text/display-text-safety.ts and src/renderer/src/store/slices/browser/annotations/browser-page-annotation.ts.
// MIT licence, Copyright (c) 2026 Lovecast Inc. Translated from TypeScript and adapted to cockpit's browser shell.

import { isAbsolute } from "node:path";
import { fileURLToPath } from "node:url";
import { GRAB_BUDGET, GRAB_SAFE_ATTRIBUTE_NAMES, GRAB_SECRET_PATTERNS } from "./types.js";

const SAFE_GRAB_URL_PROTOCOLS = new Set(["http:", "https:", "file:"]);

// Controls, format characters (bidi overrides, zero-width, tags) and line separators can reorder or hide text.
const UNSAFE_DISPLAY_CHARACTER = /[\p{Cc}\p{Cf}\p{Cs}\p{Zl}\p{Zp}]/u;
// ZWNJ and ZWJ are format characters that Persian, Indic scripts and emoji sequences need.
const SAFE_JOINERS = new Set(["\u200c", "\u200d"]);

/** Whether a character can be shown as is, without reordering or hiding the text around it. */
export function isSafeDisplayCharacter(character) {
  return SAFE_JOINERS.has(character) || !UNSAFE_DISPLAY_CHARACTER.test(character);
}

/** Whether a value holds one of the credential-shaped substrings. */
export function containsGrabSecret(value) {
  const lower = String(value ?? "").toLowerCase();
  return GRAB_SECRET_PATTERNS.some((pattern) => lower.includes(pattern));
}

/** A URL without its query and fragment, which may hold tokens; empty for anything but http, https, file and about:blank. */
export function sanitizeGrabUrl(rawUrl) {
  const str = typeof rawUrl === "string" ? rawUrl : "";
  if (!str) return "";
  try {
    const url = new URL(str);
    if (url.protocol === "about:") return url.toString() === "about:blank" ? "about:blank" : "";
    if (!SAFE_GRAB_URL_PROTOCOLS.has(url.protocol)) return "";
    url.search = "";
    url.hash = "";
    return url.toString();
  } catch {
    return "";
  }
}

/** The local path a `file:` document URL names, or null when it is not one or holds characters unsafe to show. */
function localFilePath(documentUrl) {
  if (typeof documentUrl !== "string" || !documentUrl.startsWith("file:")) return null;
  try {
    const path = fileURLToPath(documentUrl);
    return isAbsolute(path) && [...path].every(isSafeDisplayCharacter) ? path : null;
  } catch {
    return null;
  }
}

const clampStr = (s, max) => {
  const str = typeof s === "string" ? s : "";
  return str.length <= max ? str : `${str.slice(0, max)} (truncated)`;
};

const clampArray = (arr, maxEntries, maxEntryLength) => (Array.isArray(arr) ? arr : []).slice(0, maxEntries).map((item) => clampStr(item, maxEntryLength));

const safeStr = (s, max = 500) => clampStr(s, max);

const safeNum = (n, fallback = 0) => (typeof n === "number" && Number.isFinite(n) ? n : fallback);

function safeAttributes(attrs) {
  if (!attrs || typeof attrs !== "object") return {};
  const filtered = {};
  for (const [key, value] of Object.entries(attrs)) {
    const name = key.toLowerCase();
    if (!name.startsWith("aria-") && !GRAB_SAFE_ATTRIBUTE_NAMES.has(name)) continue;
    const strValue = safeStr(value, 2000);
    if (containsGrabSecret(strValue)) filtered[name] = "[redacted]";
    else if ((name === "href" || name === "src" || name === "action") && strValue) filtered[name] = sanitizeGrabUrl(strValue);
    else if (name === "class") filtered[name] = safeStr(value, 200);
    else filtered[name] = safeStr(value, 500);
  }
  return filtered;
}

const safeMetadataStr = (value, max) => {
  const strValue = safeStr(value, max);
  return strValue && containsGrabSecret(strValue) ? "[redacted]" : strValue;
};

const safeNullableMetadataStr = (value, max) => safeMetadataStr(value, max) || null;

const safeMetadataArray = (arr, maxEntries, maxEntryLength) =>
  (Array.isArray(arr) ? arr : [])
    .slice(0, maxEntries)
    .map((item) => safeMetadataStr(item, maxEntryLength))
    .filter(Boolean);

function safeRect(r) {
  if (!r || typeof r !== "object") return { x: 0, y: 0, width: 0, height: 0 };
  return { x: safeNum(r.x), y: safeNum(r.y), width: safeNum(r.width), height: safeNum(r.height) };
}

/**
 * Checks and clamps what the page returned for a pick, since a page can forge it: every string is bounded, attributes
 * are filtered, credential-looking values are redacted and URLs lose their query and fragment. Returns null when the
 * value has no `page` or `target` object; a `documentUrl` that is a `file:` URL names the page by its local path.
 * @param {unknown} raw
 * @param {string} [documentUrl] The page's address as the main process knows it (`webContents.getURL()`).
 * @returns {import("./types.js").GrabPayload | null}
 */
export function clampGrabPayload(raw, documentUrl = "") {
  if (!raw || typeof raw !== "object") return null;
  const { page, target } = raw;
  if (!page || typeof page !== "object") return null;
  if (!target || typeof target !== "object") return null;

  const accessibility = target.accessibility;
  const computedStyles = target.computedStyles;

  return {
    page: {
      sanitizedUrl: localFilePath(documentUrl) ?? sanitizeGrabUrl(page.sanitizedUrl),
      title: safeStr(page.title, 500),
      viewportWidth: safeNum(page.viewportWidth),
      viewportHeight: safeNum(page.viewportHeight),
      scrollX: safeNum(page.scrollX),
      scrollY: safeNum(page.scrollY),
      devicePixelRatio: safeNum(page.devicePixelRatio, 1),
      capturedAt: safeStr(page.capturedAt, 100),
    },
    target: {
      tagName: safeStr(target.tagName, 50),
      selector: safeStr(target.selector, GRAB_BUDGET.selectorMaxLength),
      elementPath: safeMetadataStr(target.elementPath, GRAB_BUDGET.pathMaxLength),
      fullPath: safeMetadataStr(target.fullPath, GRAB_BUDGET.pathMaxLength),
      cssClasses: safeMetadataStr(target.cssClasses, GRAB_BUDGET.cssClassesMaxLength),
      nearbyElements: safeMetadataArray(target.nearbyElements, GRAB_BUDGET.nearbyElementsMaxEntries, GRAB_BUDGET.nearbyElementMaxLength),
      selectedText: safeMetadataStr(target.selectedText, GRAB_BUDGET.selectedTextMaxLength) || null,
      isFixed: target.isFixed === true,
      reactComponents: safeNullableMetadataStr(target.reactComponents, GRAB_BUDGET.reactComponentsMaxLength),
      sourceFile: safeNullableMetadataStr(target.sourceFile, GRAB_BUDGET.sourceFileMaxLength),
      textSnippet: clampStr(target.textSnippet, GRAB_BUDGET.textSnippetMaxLength),
      htmlSnippet: clampStr(target.htmlSnippet, GRAB_BUDGET.htmlSnippetMaxLength),
      attributes: safeAttributes(target.attributes),
      accessibility: {
        role: safeNullableMetadataStr(accessibility?.role, 500),
        accessibleName: safeNullableMetadataStr(accessibility?.accessibleName, 500),
        ariaLabel: safeNullableMetadataStr(accessibility?.ariaLabel, 500),
        ariaLabelledBy: safeNullableMetadataStr(accessibility?.ariaLabelledBy, 500),
      },
      rectViewport: safeRect(target.rectViewport),
      rectPage: safeRect(target.rectPage),
      computedStyles: {
        display: safeStr(computedStyles?.display),
        position: safeStr(computedStyles?.position),
        width: safeStr(computedStyles?.width),
        height: safeStr(computedStyles?.height),
        margin: safeStr(computedStyles?.margin),
        padding: safeStr(computedStyles?.padding),
        color: safeStr(computedStyles?.color),
        backgroundColor: safeStr(computedStyles?.backgroundColor),
        border: safeStr(computedStyles?.border),
        borderRadius: safeStr(computedStyles?.borderRadius),
        fontFamily: safeStr(computedStyles?.fontFamily),
        fontSize: safeStr(computedStyles?.fontSize),
        fontWeight: safeStr(computedStyles?.fontWeight),
        lineHeight: safeStr(computedStyles?.lineHeight),
        textAlign: safeStr(computedStyles?.textAlign),
        zIndex: safeStr(computedStyles?.zIndex),
      },
    },
    nearbyText: clampArray(raw.nearbyText, GRAB_BUDGET.nearbyTextMaxEntries, GRAB_BUDGET.nearbyTextEntryMaxLength),
    ancestorPath: clampArray(raw.ancestorPath, GRAB_BUDGET.ancestorPathMaxEntries, 200),
    screenshot: null,
  };
}

/**
 * A note ready to keep: its comment cut to the comment limit and its payload without the screenshot bytes.
 * @param {import("./types.js").PageAnnotation} annotation
 * @returns {import("./types.js").PageAnnotation}
 */
export function sanitizeAnnotation(annotation) {
  const comment = typeof annotation.comment === "string" ? annotation.comment : "";
  return {
    ...annotation,
    comment: comment.length > GRAB_BUDGET.annotationCommentMaxLength ? comment.slice(0, GRAB_BUDGET.annotationCommentMaxLength) : comment,
    payload: { ...annotation.payload, screenshot: null },
  };
}
