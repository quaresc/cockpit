// Ported from Orca (https://github.com/stablyai/orca, commit 49498c3), src/main/browser/element-grab/browser-grab-session-controller.ts
// (how a session reads what `awaitClick` settles with).
// MIT licence, Copyright (c) 2026 Lovecast Inc. Translated from TypeScript and adapted to cockpit's browser shell.

import { clampGrabPayload } from "./payload.js";
import { GRAB_CANCELLED_KEY, GRAB_CONTEXT_MENU_KEY } from "./types.js";

/** How long a session waits for a click before it cancels the pick with the reason "timeout". */
export const GRAB_TIMEOUT_MS = 120_000;

/** Whether an `awaitClick` result is the cancel marker rather than a payload a page could have written. */
function isCancellation(raw) {
  if (raw[GRAB_CANCELLED_KEY] === true) return true;
  if (raw.message !== "cancelled") return false;
  return !("page" in raw) && !("target" in raw) && !("payload" in raw);
}

/**
 * Reads what the `awaitClick` script resolved with: a cancelled pick, a click ("selected") or a right-click
 * ("context-selected") with its clamped payload, or an error when the page returned something else.
 * @param {unknown} raw
 * @param {string} [documentUrl] Passed to `clampGrabPayload`.
 * @returns {{ kind: "cancelled" } | { kind: "selected" | "context-selected", payload: import("./types.js").GrabPayload } | { kind: "error", reason: string }}
 */
export function readGrabSelection(raw, documentUrl = "") {
  if (!raw || typeof raw !== "object") return { kind: "cancelled" };
  if (isCancellation(raw)) return { kind: "cancelled" };
  const isContextMenu = raw[GRAB_CONTEXT_MENU_KEY] === true;
  const payload = clampGrabPayload(isContextMenu ? raw.payload : raw, documentUrl);
  if (!payload) return { kind: "error", reason: "The page returned an invalid payload" };
  return { kind: isContextMenu ? "context-selected" : "selected", payload };
}

/**
 * Reads why the `awaitClick` script rejected: a pick with no element under the pointer counts as cancelled, anything
 * else is an error carrying the page's message.
 * @param {unknown} error
 * @returns {{ kind: "cancelled" } | { kind: "error", reason: string }}
 */
export function readGrabFailure(error) {
  const message = typeof error?.message === "string" ? error.message : "Selection failed";
  return message.includes("cancelled") ? { kind: "cancelled" } : { kind: "error", reason: message };
}
