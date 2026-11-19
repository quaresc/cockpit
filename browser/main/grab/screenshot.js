// Ported from Orca (https://github.com/stablyai/orca, commit 49498c3), src/main/browser/element-grab/browser-grab-screenshot.ts.
// MIT licence, Copyright (c) 2026 Lovecast Inc. Translated from TypeScript and adapted to cockpit's browser shell.

import { GRAB_BUDGET } from "./types.js";

/** Hides the picker and the note badges in the page, so a capture shows the page alone. */
export const HIDE_GRAB_OVERLAY_SCRIPT = `(function(){
  var g = window.__cockpitGrab;
  if (g && g.host) g.host.style.display = 'none';
  document.querySelectorAll('[data-cockpit-annotation-overlay]').forEach(function(el) {
    el.setAttribute('data-cockpit-previous-display', el.style.display || '');
    el.style.display = 'none';
  });
})()`;

/** Shows the picker and the note badges again after a capture. */
export const RESTORE_GRAB_OVERLAY_SCRIPT = `(function(){
  var g = window.__cockpitGrab;
  if (g && g.host) g.host.style.display = '';
  document.querySelectorAll('[data-cockpit-annotation-overlay]').forEach(function(el) {
    el.style.display = el.getAttribute('data-cockpit-previous-display') || '';
    el.removeAttribute('data-cockpit-previous-display');
  });
})()`;

const safeN = (n, fallback = 0) => (typeof n === "number" && Number.isFinite(n) ? n : fallback);

/**
 * The part of a captured bitmap that shows a rectangle given in CSS pixels, and the bitmap pixels per CSS pixel. Returns
 * null when the rectangle falls outside the bitmap or the viewport width is unknown.
 * @param {import("./types.js").GrabRect} rect In viewport CSS pixels.
 * @param {{ width: number, height: number }} bitmapSize
 * @param {number} viewportCssWidth The page's `window.innerWidth`.
 * @returns {{ x: number, y: number, width: number, height: number, scaleFactor: number } | null}
 */
export function selectionCropRect(rect, bitmapSize, viewportCssWidth) {
  if (!(viewportCssWidth > 0) || !(bitmapSize?.width > 0)) return null;
  // The scale comes from this capture, so a zoomed page or a second display cannot skew it.
  const scaleFactor = bitmapSize.width / viewportCssWidth;
  const x = Math.max(0, Math.round(safeN(rect?.x) * scaleFactor));
  const y = Math.max(0, Math.round(safeN(rect?.y) * scaleFactor));
  const width = Math.min(bitmapSize.width - x, Math.round(safeN(rect?.width) * scaleFactor));
  const height = Math.min(bitmapSize.height - y, Math.round(safeN(rect?.height) * scaleFactor));
  if (width <= 0 || height <= 0) return null;
  return { x, y, width, height, scaleFactor };
}

/**
 * Captures the page without the picker and crops it to a rectangle; resolves to a PNG data URL sized in CSS pixels, or
 * null when the capture is empty, fails, falls outside the page or weighs more than the screenshot budget.
 * @param {import("./types.js").GrabRect} rect In viewport CSS pixels, such as a payload's `target.rectViewport`.
 * @param {{ executeJavaScript: (code: string) => Promise<unknown>, capturePage: () => Promise<{ isEmpty?: () => boolean, getSize: () => { width: number, height: number }, crop: (rect: { x: number, y: number, width: number, height: number }) => { toPNG: () => Buffer } }> }} contents
 *   A `webContents`, or anything shaped like one.
 * @returns {Promise<import("./types.js").GrabScreenshot | null>}
 */
export async function captureSelectionScreenshot(rect, contents) {
  try {
    // The overlays come back even when the capture throws.
    await contents.executeJavaScript(HIDE_GRAB_OVERLAY_SCRIPT).catch(() => {});
    let image;
    try {
      image = await contents.capturePage();
    } finally {
      await contents.executeJavaScript(RESTORE_GRAB_OVERLAY_SCRIPT).catch(() => {});
    }
    if (!image || image.isEmpty?.()) return null;

    const viewportCssWidth = await contents.executeJavaScript("window.innerWidth");
    const crop = selectionCropRect(rect, image.getSize(), viewportCssWidth);
    if (!crop) return null;

    const png = image.crop({ x: crop.x, y: crop.y, width: crop.width, height: crop.height }).toPNG();
    if (png.byteLength > GRAB_BUDGET.screenshotMaxBytes) return null;

    return {
      mimeType: "image/png",
      dataUrl: `data:image/png;base64,${png.toString("base64")}`,
      width: Math.round(crop.width / crop.scaleFactor),
      height: Math.round(crop.height / crop.scaleFactor),
    };
  } catch {
    // The page or its compositor surface may disappear during the capture.
    return null;
  }
}
