// How the window's views are laid out: a toolbar on top, the page under it, DevTools under or beside the page.

import { fitScale } from "./viewport.js";

/** The toolbar's height: a row of tabs over a row with the address. */
export const TOOLBAR = 76;

/** The page's box inside the window for a viewport: the whole area, or the viewport scaled to fit and centred in it. */
export function pageBox(area, viewport) {
  if (!viewport) return { ...area, scale: 1 };
  const scale = fitScale(viewport, area);
  const width = Math.round(viewport.width * scale);
  const height = Math.round(viewport.height * scale);
  return { x: area.x + Math.round((area.width - width) / 2), y: area.y + Math.max(0, Math.round((area.height - height) / 2)), width, height, scale };
}

/** The areas of a window of this size: the toolbar, the page, and DevTools when it is open at a side. */
export function areas(size, { devtools = null, toolbarOpen = false, split = 0.45 } = {}) {
  const toolbar = { x: 0, y: 0, width: size.width, height: toolbarOpen ? size.height : TOOLBAR };
  const body = { x: 0, y: TOOLBAR, width: size.width, height: Math.max(0, size.height - TOOLBAR) };
  if (!devtools) return { toolbar, page: body, devtools: null };
  if (devtools === "right" && body.width >= 700) {
    const w = Math.round(body.width * split);
    return { toolbar, page: { ...body, width: body.width - w }, devtools: { x: body.width - w, y: body.y, width: w, height: body.height } };
  }
  const h = Math.round(body.height * split);
  return { toolbar, page: { ...body, height: body.height - h }, devtools: { x: 0, y: body.y + body.height - h, width: body.width, height: h } };
}
