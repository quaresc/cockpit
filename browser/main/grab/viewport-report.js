// Ported from Orca (https://github.com/stablyai/orca, commit 49498c3), src/shared/browser/annotations/browser-annotation-viewport-report.ts
// and src/renderer/src/components/browser-pane/annotate/viewport/browser-annotation-viewport-message.ts.
// MIT licence, Copyright (c) 2026 Lovecast Inc. Translated from TypeScript and adapted to cockpit's browser shell.

/**
 * What the viewport bridge reports while a note is being written, in the page's CSS pixels.
 * @typedef {object} AnnotationViewportReport
 * @property {number} scrollX
 * @property {number} scrollY
 * @property {number} [viewportWidth] The page's `innerWidth`; with the view's on-screen width it gives the zoom scale.
 * @property {import("./types.js").GrabRect} [target] The live box of the picked element; absent once the page lost it.
 */

/** The prefix of the console lines the viewport bridge writes; the token and a colon follow it, then the JSON report. */
export const ANNOTATION_VIEWPORT_MESSAGE_PREFIX = "__cockpit_annotation_viewport__:";

/** Whether a value is a rectangle of finite numbers with a width and height of zero or more. */
export function isValidGrabRect(value) {
  if (!value || typeof value !== "object") return false;
  const { x, y, width, height } = value;
  return (
    typeof x === "number" &&
    Number.isFinite(x) &&
    typeof y === "number" &&
    Number.isFinite(y) &&
    typeof width === "number" &&
    Number.isFinite(width) &&
    width >= 0 &&
    typeof height === "number" &&
    Number.isFinite(height) &&
    height >= 0
  );
}

const finiteOr = (value, fallback) => (typeof value === "number" && Number.isFinite(value) ? value : fallback);

/**
 * Parses the JSON of a viewport report, with zeros for a missing scroll; null when it is not a JSON object.
 * @param {string} json
 * @returns {AnnotationViewportReport | null}
 */
export function parseAnnotationViewportReport(json) {
  let next;
  try {
    next = JSON.parse(json);
  } catch {
    return null;
  }
  if (!next || typeof next !== "object") return null;
  const viewportWidth = finiteOr(next.viewportWidth, 0);
  return {
    scrollX: finiteOr(next.scrollX, 0),
    scrollY: finiteOr(next.scrollY, 0),
    ...(viewportWidth > 0 ? { viewportWidth } : {}),
    ...(isValidGrabRect(next.target) ? { target: next.target } : {}),
  };
}

/**
 * Reads a console line of the page as a viewport report; null unless it carries this bridge's token.
 * @param {unknown} message The text of a `console-message` event.
 * @param {string} token The token the bridge was installed with.
 * @returns {AnnotationViewportReport | null}
 */
export function readAnnotationViewportMessage(message, token) {
  const text = typeof message === "string" ? message : "";
  const prefix = `${ANNOTATION_VIEWPORT_MESSAGE_PREFIX}${token}:`;
  return text.startsWith(prefix) ? parseAnnotationViewportReport(text.slice(prefix.length)) : null;
}
