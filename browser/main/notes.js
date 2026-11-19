// The notes a person writes on a page's elements, as plain data: what is kept of each, its badge in the page, its name.

import { sanitizeAnnotation, ANNOTATION_INTENTS, ANNOTATION_PRIORITIES, DEFAULT_ANNOTATION_PRIORITY } from "./grab/index.js";

/** A note as it is kept: what the formatter needs, and the tab it was made on. */
export function noteOf({ id, tab, payload, comment, intent, priority, image }, now = new Date()) {
  return sanitizeAnnotation({
    id,
    browserPageId: tab,
    comment: String(comment || "").trim(),
    intent: ANNOTATION_INTENTS.includes(intent) ? intent : "change",
    priority: ANNOTATION_PRIORITIES.includes(priority) ? priority : DEFAULT_ANNOTATION_PRIORITY,
    createdAt: now.toISOString(),
    payload: { ...payload, screenshot: null },
    imagePaths: image ? [image] : [],
  });
}

/** The numbered badges that mark a tab's notes in its page. */
export const markersOf = (notes, tab) =>
  notes
    .filter((n) => n.browserPageId === tab)
    .map((n, index) => ({ id: n.id, index, label: `${capital(n.intent)} · ${capital(n.priority)}\n${n.comment}`.slice(0, 400), rectPage: n.payload.target.rectPage, rectViewport: n.payload.target.rectViewport, isFixed: n.payload.target.isFixed === true }));

const capital = (word) => word.charAt(0).toUpperCase() + word.slice(1);

/** A note as the toolbar's list shows it. */
export const summaryOf = (note) => ({ id: note.id, label: labelOf(note.payload), comment: note.comment, intent: note.intent, priority: note.priority });

/** A short name for a picked element, for a menu or a card. */
export function labelOf(payload) {
  const { target } = payload;
  const name = target.accessibility?.accessibleName || target.textSnippet || "";
  const tag = (target.reactComponents || "").split(" ").pop() || `<${target.tagName}>`;
  return `${tag}${name ? ` “${name.slice(0, 40)}”` : ""}`;
}
