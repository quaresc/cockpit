// Ported from Orca (https://github.com/stablyai/orca, commit 49498c3), src/main/browser/element-grab/grab-guest-script.ts and
// src/main/browser/element-grab/grab-guest-selection-scripts.ts.
// MIT licence, Copyright (c) 2026 Lovecast Inc. Translated from TypeScript and adapted to cockpit's browser shell.

import { COMPOSED_TREE_SCRIPT } from "./script-composed-tree.js";
import { GRAB_CONTENT_SCRIPT } from "./script-content.js";
import { GRAB_ELEMENT_CONTEXT_SCRIPT } from "./script-element-context.js";
import { GRAB_FOUNDATION_SCRIPT } from "./script-foundation.js";
import { GRAB_OVERLAY_SCRIPT } from "./script-overlay.js";
import { GRAB_REACT_SCRIPT } from "./script-react.js";

/** Installs the picker in the page and starts the hover outline; evaluates to `true`. */
const ARM_SCRIPT = [
  GRAB_FOUNDATION_SCRIPT,
  COMPOSED_TREE_SCRIPT,
  GRAB_CONTENT_SCRIPT,
  GRAB_ELEMENT_CONTEXT_SCRIPT,
  GRAB_REACT_SCRIPT,
  GRAB_OVERLAY_SCRIPT,
].join("");

/**
 * Waits for the next click or right-click on the armed picker, freezes the outline on the element and resolves with its
 * payload. A right-click wraps the payload, a teardown resolves the cancel marker, and an unarmed page rejects.
 */
const AWAIT_CLICK_SCRIPT = `(async function() {
  // Async returns an intrinsic Promise Electron unwraps even when Zone.js replaces global Promise.
  return await new Promise(function(resolve, reject) {
    'use strict';
    var grab = window.__cockpitGrab;
    if (!grab) {
      reject(new Error('Grab not armed'));
      return;
    }

    function extractSelectedPayload(el) {
      try {
        return grab.extractPayload(el);
      } catch (error) {
        grab.cleanup();
        reject(error instanceof Error ? error : new Error('Failed to extract element context'));
        return null;
      }
    }

    // The overlay blocks the press and hands it here; a later await replaces this one.
    grab.onPick = function(e) {
      grab.onPick = null;
      var el = grab.getCurrentElement();
      if (!el) {
        grab.cleanup();
        reject(new Error('cancelled'));
        return;
      }
      var payload = extractSelectedPayload(el);
      if (!payload) return;
      // Keep the selected element outlined while the toolbar shows its copy menu.
      grab.freezeHighlight(payload.page.capturedAt);
      // The marker opens the action menu instead of auto-copying.
      resolve(e.type === 'contextmenu' ? { __cockpitContextMenu: true, payload: payload } : payload);
    };

    grab.cancelAwait = function() {
      grab.cleanup();
      // Resolve cancellation to avoid page-console errors during normal teardown.
      resolve({ __cockpitCancelled: true });
    };
  });
})()`;

/** Reads the payload of the hovered element and removes the picker; evaluates to the payload or `null`. */
const FINALIZE_SCRIPT = `(function() {
  'use strict';
  var grab = window.__cockpitGrab;
  if (!grab) return null;
  var el = grab.getCurrentElement();
  if (!el) return null;
  var payload = null;
  try {
    payload = grab.extractPayload(el);
  } catch (e) {
    grab.cleanup();
    return null;
  }
  grab.cleanup();
  return payload;
})()`;

/** Reads the payload of the hovered element and leaves the picker armed; evaluates to the payload or `null`. */
const EXTRACT_HOVER_SCRIPT = `(function() {
  'use strict';
  var grab = window.__cockpitGrab;
  if (!grab) return null;
  var el = grab.getCurrentElement();
  if (!el) return null;
  try {
    return grab.extractPayload(el);
  } catch (e) {
    return null;
  }
})()`;

/** Removes the picker and settles a pending `awaitClick` with the cancel marker; evaluates to `true`. */
const TEARDOWN_SCRIPT = `(function() {
  'use strict';
  var grab = window.__cockpitGrab;
  if (!grab) return true;
  // Settle the outstanding executeJavaScript call before removing grab state.
  if (grab.cancelAwait) {
    grab.cancelAwait();
  } else {
    grab.cleanup();
  }
  return true;
})()`;

const SCRIPTS = { arm: ARM_SCRIPT, awaitClick: AWAIT_CLICK_SCRIPT, finalize: FINALIZE_SCRIPT, extractHover: EXTRACT_HOVER_SCRIPT, teardown: TEARDOWN_SCRIPT };

/**
 * The script of one step of a pick, to run in the page with `webContents.executeJavaScript`.
 * @param {import("./types.js").GrabStep} step
 * @returns {string}
 */
export function grabScript(step) {
  if (!Object.hasOwn(SCRIPTS, step)) throw new Error(`Unknown grab step: ${step}`);
  return SCRIPTS[step];
}
