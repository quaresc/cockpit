// Ported from Orca (https://github.com/stablyai/orca, commit 49498c3), src/shared/browser/annotations/browser-annotation-viewport-bridge.ts.
// MIT licence, Copyright (c) 2026 Lovecast Inc. Translated from TypeScript and adapted to cockpit's browser shell.

import { COMPOSED_TREE_SCRIPT } from "./script-composed-tree.js";
import { GRAB_SELECTED_ATTRIBUTE } from "./types.js";
import { ANNOTATION_VIEWPORT_MESSAGE_PREFIX, isValidGrabRect } from "./viewport-report.js";

/**
 * A numbered badge the bridge draws for a saved note.
 * @typedef {object} AnnotationViewportMarker
 * @property {string} id
 * @property {number} index Zero-based; the badge shows `index + 1`.
 * @property {import("./types.js").GrabRect} rectPage
 * @property {import("./types.js").GrabRect} rectViewport
 * @property {boolean} isFixed Whether the badge stays put when the page scrolls.
 * @property {string} [label] What the badge says when the pointer rests on it: the note it stands for.
 */

/**
 * What the bridge script is built with.
 * @typedef {object} AnnotationViewportBridgeOptions
 * @property {boolean} enabled False removes the bridge from the page.
 * @property {boolean} emitViewport Whether the bridge reports the scroll and the picked element's box on the console.
 * @property {AnnotationViewportMarker[]} markers
 * @property {string} [selectionId] The value of the selected attribute on the element to follow.
 * @property {string} token A secret of the session, which page scripts never see.
 */

/** The isolated world the bridge runs in, so page scripts cannot read its token. */
export const ANNOTATION_VIEWPORT_BRIDGE_WORLD_ID = 1207;

/** Whether a value can serve as a bridge token: 16 to 80 letters, digits, dashes or underscores. */
export function isValidAnnotationViewportBridgeToken(value) {
  return typeof value === "string" && /^[a-zA-Z0-9_-]{16,80}$/.test(value);
}

/** Whether a value is a list of at most 50 well-formed markers. */
export function isValidAnnotationViewportBridgeMarkers(value) {
  return (
    Array.isArray(value) &&
    value.length <= 50 &&
    value.every(
      (marker) =>
        Boolean(marker) &&
        typeof marker === "object" &&
        typeof marker.id === "string" &&
        marker.id.length > 0 &&
        marker.id.length <= 100 &&
        Number.isInteger(marker.index) &&
        marker.index >= 0 &&
        marker.index < 100 &&
        typeof marker.isFixed === "boolean" &&
        (marker.label === undefined || (typeof marker.label === "string" && marker.label.length <= 400)) &&
        isValidGrabRect(marker.rectPage) &&
        isValidGrabRect(marker.rectViewport),
    )
  );
}

/**
 * The script that keeps the notes' numbered badges on their elements while the page scrolls, and reports the scroll and
 * the picked element's live box as console lines. It runs in the bridge's isolated world and evaluates to `true`.
 * @param {AnnotationViewportBridgeOptions} options
 * @returns {string}
 */
export function buildAnnotationViewportBridgeScript({ emitViewport, enabled, markers, selectionId, token }) {
  return `(() => {
  'use strict';

  const enabled = ${JSON.stringify(enabled)};
  const emitViewportMessages = ${JSON.stringify(emitViewport)};
  const markers = ${JSON.stringify(markers)};
  const selectionId = ${JSON.stringify(selectionId ?? null)};
  const token = ${JSON.stringify(token)};
  const prefix = ${JSON.stringify(ANNOTATION_VIEWPORT_MESSAGE_PREFIX)};
  const stateKey = '__cockpitAnnotationViewportBridge';
  const hostAttribute = 'data-cockpit-annotation-overlay';
  const selectedAttribute = ${JSON.stringify(GRAB_SELECTED_ATTRIBUTE)};
  const markerSize = 24;
${COMPOSED_TREE_SCRIPT}

  const removeOverlay = (state) => {
    if (state && state.host && state.host.parentNode) {
      state.host.parentNode.removeChild(state.host);
    }
  };

  const cleanup = (state) => {
    if (!state) return;
    if (state.raf) {
      cancelAnimationFrame(state.raf);
    }
    if (state.requestUpdate) {
      window.removeEventListener('scroll', state.requestUpdate, true);
      document.removeEventListener('scroll', state.requestUpdate, true);
      window.removeEventListener('resize', state.requestUpdate, true);
    }
    if (state.onBeforeUnload) {
      window.removeEventListener('beforeunload', state.onBeforeUnload);
    }
    if (state.targetObserver) {
      state.targetObserver.disconnect();
    }
    state.targetShadowRoots.forEach((root) => root.removeEventListener('scroll', state.requestUpdate, true));
    removeOverlay(state);
  };

  const existing = globalThis[stateKey];
  if (!enabled) {
    cleanup(existing);
    delete globalThis[stateKey];
    return true;
  }

  const toNumber = (value, fallback) =>
    typeof value === 'number' && Number.isFinite(value) ? value : fallback;

  const readViewport = () => ({
    scrollX: toNumber(window.scrollX, toNumber(window.pageXOffset, 0)),
    scrollY: toNumber(window.scrollY, toNumber(window.pageYOffset, 0))
  });

  const findMarked = (root) =>
    Array.from(root.querySelectorAll('[' + selectedAttribute + ']')).filter(
      (candidate) => candidate.getAttribute(selectedAttribute) === selectionId
    );

  // Document queries stop at shadow roots, so open ones are walked when the light DOM has no match.
  const findMarkedInShadowRoots = (root) =>
    Array.from(root.querySelectorAll('*')).flatMap((element) =>
      element.shadowRoot ? findMarked(element.shadowRoot).concat(findMarkedInShadowRoots(element.shadowRoot)) : []
    );

  // The card must follow the element itself, not a click-time copy; the grab overlay
  // marks it in the shared DOM because this world cannot see the page's objects.
  const trackTarget = (state) => {
    const current = state.target;
    let next = null;
    if (emitViewportMessages && selectionId) {
      let found = findMarked(document);
      if (found.length === 0) {
        // The marker is set before the selection reaches this bridge, so one shadow walk per selection suffices.
        const keep = current && current.isConnected && current.getAttribute(selectedAttribute) === selectionId;
        if (keep) found = [current];
        else if (state.searchedSelectionId !== selectionId) {
          state.searchedSelectionId = selectionId;
          found = findMarkedInShadowRoots(document);
        }
      }
      next = found.length === 1 ? found[0] : null;
    }
    if (next !== current) {
      state.target = next;
      state.targetShadowRoots.forEach((root) => root.removeEventListener('scroll', state.requestUpdate, true));
      state.targetShadowRoots = next ? shadowRootsAbove(next) : [];
      state.targetShadowRoots.forEach((root) => root.addEventListener('scroll', state.requestUpdate, true));
      if (state.targetObserver) {
        state.targetObserver.disconnect();
        if (next) {
          state.targetObserver.observe(next, { box: 'border-box' });
          if (next !== document.documentElement) {
            state.targetObserver.observe(document.documentElement);
          }
        }
      }
    }
    return next;
  };

  const readTarget = (target) => {
    if (!target) return null;
    const rect = target.getBoundingClientRect();
    return { x: rect.x, y: rect.y, width: rect.width, height: rect.height };
  };

  const emitViewport = (state) => {
    const target = trackTarget(state);
    if (!emitViewportMessages) return;
    try {
      const report = readViewport();
      report.viewportWidth = toNumber(window.innerWidth, 0);
      report.target = readTarget(target);
      console.debug(prefix + token + ':' + JSON.stringify(report));
    } catch (e) {}
  };

  const getRoot = () => document.body || document.documentElement;

  const ensureOverlay = (state) => {
    if (state.host && state.shadowRoot) {
      return state.shadowRoot;
    }
    const root = getRoot();
    if (!root) return null;
    const host = document.createElement('div');
    host.setAttribute(hostAttribute, '');
    host.style.cssText = 'position:fixed;inset:0;z-index:2147483646;pointer-events:none;contain:layout style paint;overflow:hidden;';
    const shadowRoot = host.attachShadow({ mode: 'closed' });
    const style = document.createElement('style');
    style.textContent = '.marker{box-sizing:border-box;position:absolute;left:0;top:0;width:24px;height:24px;display:flex;align-items:center;justify-content:center;border-radius:9999px;border:1px solid rgba(255,255,255,0.95);background:#2563eb;color:#fff;font:600 11px/1 -apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif;box-shadow:0 10px 24px rgba(0,0,0,.18);will-change:transform;pointer-events:auto;cursor:default;user-select:none;}.marker[data-note]:hover{z-index:1;}.marker[data-note]:hover::after{content:attr(data-note);position:absolute;top:28px;left:0;width:max-content;max-width:260px;padding:8px 10px;border-radius:8px;background:#1f1e1c;color:#fff;font:400 12px/1.4 -apple-system,BlinkMacSystemFont,sans-serif;white-space:pre-wrap;text-align:left;box-shadow:0 8px 24px rgba(0,0,0,.28);}.marker[data-flip][data-note]:hover::after{left:auto;right:0;}';
    shadowRoot.appendChild(style);
    root.appendChild(host);
    state.host = host;
    state.shadowRoot = shadowRoot;
    return shadowRoot;
  };

  const updateMarkers = (state, nextMarkers) => {
    state.markers = Array.isArray(nextMarkers) ? nextMarkers : [];
    if (state.markers.length === 0) {
      removeOverlay(state);
      state.host = null;
      state.shadowRoot = null;
      state.markerElements = new Map();
      return;
    }
    const shadowRoot = ensureOverlay(state);
    if (!shadowRoot) return;
    const liveIds = new Set();
    state.markers.forEach((marker) => {
      liveIds.add(marker.id);
      let element = state.markerElements.get(marker.id);
      if (!element) {
        element = document.createElement('span');
        element.className = 'marker';
        shadowRoot.appendChild(element);
        state.markerElements.set(marker.id, element);
      }
      element.textContent = String(marker.index + 1);
      if (typeof marker.label === 'string' && marker.label) element.setAttribute('data-note', marker.label);
      else element.removeAttribute('data-note');
      if (toNumber(marker.rectViewport && marker.rectViewport.x, 0) > toNumber(window.innerWidth, 0) - 300) element.setAttribute('data-flip', '');
      else element.removeAttribute('data-flip');
    });
    state.markerElements.forEach((element, id) => {
      if (!liveIds.has(id)) {
        element.remove();
        state.markerElements.delete(id);
      }
    });
  };

  const positionMarkers = (state) => {
    if (!state.markers || state.markers.length === 0) return;
    const viewport = readViewport();
    const viewportWidth = toNumber(window.innerWidth, 0);
    const viewportHeight = toNumber(window.innerHeight, 0);
    state.markers.forEach((marker) => {
      const element = state.markerElements.get(marker.id);
      if (!element) return;
      const sourceRect = marker.isFixed ? marker.rectViewport : marker.rectPage;
      const x = marker.isFixed ? sourceRect.x : sourceRect.x - viewport.scrollX;
      const y = marker.isFixed ? sourceRect.y : sourceRect.y - viewport.scrollY;
      const width = toNumber(sourceRect.width, 0);
      const height = toNumber(sourceRect.height, 0);
      const visible =
        x + width >= 0 &&
        y + height >= 0 &&
        x <= viewportWidth &&
        y <= viewportHeight;
      if (!visible) {
        element.style.display = 'none';
        return;
      }
      element.style.display = 'flex';
      element.style.transform =
        'translate3d(' +
        (x + width / 2 - markerSize / 2) +
        'px,' +
        (y + height - markerSize / 2) +
        'px,0)';
    });
  };

  if (existing && existing.requestUpdate) {
    existing.emitViewport = emitViewport;
    updateMarkers(existing, markers);
    existing.requestUpdate();
    return true;
  }

  const state = {
    raf: 0,
    emitViewport,
    host: null,
    markerElements: new Map(),
    markers: [],
    shadowRoot: null,
    requestUpdate: null,
    onBeforeUnload: null,
    target: null,
    targetObserver: null,
    targetShadowRoots: [],
    searchedSelectionId: null
  };

  state.requestUpdate = () => {
    if (state.raf) return;
    state.raf = requestAnimationFrame(() => {
      state.raf = 0;
      positionMarkers(state);
      state.emitViewport(state);
    });
  };
  if (typeof ResizeObserver === 'function') {
    state.targetObserver = new ResizeObserver(state.requestUpdate);
  }

  // Navigation can hold IPC cleanup until the next document finishes loading.
  state.onBeforeUnload = () => {
    cleanup(state);
    delete globalThis[stateKey];
  };

  updateMarkers(state, markers);
  window.addEventListener('scroll', state.requestUpdate, true);
  document.addEventListener('scroll', state.requestUpdate, true);
  window.addEventListener('resize', state.requestUpdate, true);
  window.addEventListener('beforeunload', state.onBeforeUnload);
  globalThis[stateKey] = state;
  state.requestUpdate();
  return true;
})();`;
}
