// Ported from Orca (https://github.com/stablyai/orca, commit 49498c3),
// src/renderer/src/components/browser-pane/annotate/grab/GrabConfirmationSheet.tsx (`formatGrabPayloadAsText`).
// MIT licence, Copyright (c) 2026 Lovecast Inc. Translated from TypeScript and adapted to cockpit's browser shell.

/**
 * One picked element as the plain text a grab copies: the page, the element with its selector, source and size, then its
 * text, what sits around it, a few computed styles, its HTML and its DOM path.
 * @param {import("./types.js").GrabPayload} payload
 * @returns {string}
 */
export function formatGrabPayloadAsText(payload) {
  const { target } = payload;
  const lines = [`Attached browser context from ${payload.page.sanitizedUrl}`, "", "Selected element:", target.tagName];
  if (target.accessibility.accessibleName) lines.push(`Accessible name: "${target.accessibility.accessibleName}"`);
  if (target.accessibility.role) lines.push(`Role: ${target.accessibility.role}`);
  lines.push(`Selector: ${target.selector}`);
  if (target.sourceFile) lines.push(`Source: ${target.sourceFile}`);
  if (target.reactComponents) lines.push(`React: ${target.reactComponents}`);
  lines.push(`Dimensions: ${Math.round(target.rectViewport.width)}x${Math.round(target.rectViewport.height)}`);
  lines.push("");

  if (target.textSnippet) lines.push("Text content:", target.textSnippet, "");

  if (payload.nearbyText.length > 0) {
    lines.push("Nearby context:");
    for (const text of payload.nearbyText) lines.push(`- ${text}`);
    lines.push("");
  }

  const styles = target.computedStyles;
  const styleLines = [];
  if (styles.display && styles.display !== "inline") styleLines.push(`display: ${styles.display}`);
  if (styles.position && styles.position !== "static") styleLines.push(`position: ${styles.position}`);
  if (styles.fontSize) styleLines.push(`font-size: ${styles.fontSize}`);
  if (styles.color) styleLines.push(`color: ${styles.color}`);
  if (styles.backgroundColor && styles.backgroundColor !== "rgba(0, 0, 0, 0)") styleLines.push(`background: ${styles.backgroundColor}`);
  if (styleLines.length > 0) {
    lines.push("Computed styles:");
    for (const line of styleLines) lines.push(`  ${line}`);
    lines.push("");
  }

  if (target.htmlSnippet) lines.push("HTML:", target.htmlSnippet, "");

  if (payload.ancestorPath.length > 0) lines.push(`Ancestor path: ${payload.ancestorPath.join(" > ")}`);
  if (target.fullPath) lines.push(`Full DOM path: ${target.fullPath}`);

  return lines.join("\n").trimEnd();
}
