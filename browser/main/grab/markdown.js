// Ported from Orca (https://github.com/stablyai/orca, commit 49498c3), src/shared/browser/annotations/browser-annotation-output.ts.
// MIT licence, Copyright (c) 2026 Lovecast Inc. Translated from TypeScript and adapted to cockpit's browser shell.

import { isAbsolute } from "node:path";

/** The most characters of page-controlled text one line of the Markdown carries. */
export const ANNOTATION_INLINE_TEXT_MAX_LENGTH = 2048;

/** The heading of a page: its path and query for a URL, the path itself for a local file. */
function formatPageHeading(payload) {
  const url = payload.page.sanitizedUrl;
  if (isAbsolute(url)) return url;
  try {
    const parsed = new URL(url);
    return `${parsed.pathname}${parsed.search}`;
  } catch {
    return url || "current page";
  }
}

/** The title of a note: the React components when known, then the tag with its accessible name or its text. */
function annotationElementLabel(payload) {
  const { target } = payload;
  const react = target.reactComponents;
  const accessibleName = target.accessibility.accessibleName;
  const base = accessibleName
    ? `${target.tagName} "${inlineText(accessibleName)}"`
    : target.textSnippet
      ? `${target.tagName} "${inlineText(target.textSnippet).slice(0, 60)}"`
      : target.tagName;
  return react ? `${inlineText(react)} ${base}` : base;
}

function isInlineWhitespaceCode(code) {
  return (
    code === 0x20 ||
    (code >= 0x09 && code <= 0x0d) ||
    code === 0xa0 ||
    code === 0x1680 ||
    (code >= 0x2000 && code <= 0x200a) ||
    code === 0x2028 ||
    code === 0x2029 ||
    code === 0x202f ||
    code === 0x205f ||
    code === 0x3000 ||
    code === 0xfeff
  );
}

/** Text on one line: runs of whitespace become one space, and both the scan and the result stop at the limit. */
function inlineText(content, maxLength = ANNOTATION_INLINE_TEXT_MAX_LENGTH) {
  let normalized = "";
  let pendingSpace = false;
  for (let index = 0; index < content.length && normalized.length < maxLength; ) {
    const code = content.charCodeAt(index);
    if (isInlineWhitespaceCode(code)) {
      if (code === 13 && content.charCodeAt(index + 1) === 10) index += 1;
      pendingSpace = normalized.length > 0;
      index += 1;
      continue;
    }
    const codePoint = content.codePointAt(index);
    if (codePoint === undefined) break;
    const char = String.fromCodePoint(codePoint);
    if (normalized.length + (pendingSpace ? 1 : 0) + char.length > maxLength) break;
    if (pendingSpace) {
      normalized += " ";
      pendingSpace = false;
    }
    normalized += char;
    index += char.length;
  }
  return normalized;
}

/** The computed styles worth showing, one list item each, without the browser defaults. */
function formatStyles(styles) {
  const entries = [
    ["display", styles.display],
    ["position", styles.position],
    ["width", styles.width],
    ["height", styles.height],
    ["margin", styles.margin],
    ["padding", styles.padding],
    ["color", styles.color],
    ["background", styles.backgroundColor],
    ["border", styles.border],
    ["border-radius", styles.borderRadius],
    ["font-family", styles.fontFamily],
    ["font-size", styles.fontSize],
    ["font-weight", styles.fontWeight],
    ["line-height", styles.lineHeight],
    ["text-align", styles.textAlign],
    ["z-index", styles.zIndex],
  ];
  const lines = [];
  for (const [name, value] of entries) {
    if (!value || value === "auto" || value === "normal") continue;
    if (name === "position" && value === "static") continue;
    if (name === "display" && value === "inline") continue;
    if (name === "background" && value === "rgba(0, 0, 0, 0)") continue;
    lines.push(`- ${name}: ${value}`);
  }
  return lines;
}

/** The longest run of backticks in a text, or the floor when it is longer; a loop, since page HTML can hold any number of runs. */
function maxBacktickRunLength(content, floor) {
  let maxRun = floor;
  let currentRun = 0;
  for (let index = 0; index < content.length; index += 1) {
    if (content.charCodeAt(index) !== 96) {
      currentRun = 0;
      continue;
    }
    currentRun += 1;
    if (currentRun > maxRun) maxRun = currentRun;
  }
  return maxRun;
}

/** A fenced code block whose fence is longer than any run of backticks inside it. */
function fence(language, content) {
  const marker = "`".repeat(maxBacktickRunLength(content, 3) + 1);
  return [`${marker}${language}`, content, marker];
}

/** A code span whose delimiter is longer than any run of backticks inside it. */
function inlineCode(content) {
  const marker = "`".repeat(maxBacktickRunLength(content, 0) + 1);
  const padding = content.startsWith("`") || content.endsWith("`") ? " " : "";
  return `${marker}${padding}${content}${padding}${marker}`;
}

/** The page details of one note: location, React components, bounds, classes, surroundings, styles, DOM path and HTML. */
function formatDiagnostics(payload) {
  const { target } = payload;
  const rect = target.rectViewport;
  const lines = [];
  if (target.elementPath) lines.push(`**Location:** ${inlineCode(target.elementPath)}`);
  if (target.reactComponents) lines.push(`**React:** ${inlineText(target.reactComponents)}`);
  lines.push(`**Bounds:** x=${Math.round(rect.x)}, y=${Math.round(rect.y)}, ${Math.round(rect.width)}x${Math.round(rect.height)}`);
  if (target.cssClasses) lines.push(`**Classes:** ${inlineCode(target.cssClasses)}`);
  if (payload.nearbyText.length > 0) {
    lines.push("**Nearby text:**");
    for (const text of payload.nearbyText) lines.push(`- ${inlineText(text)}`);
  }
  if (target.nearbyElements?.length) {
    lines.push("**Nearby elements:**");
    for (const element of target.nearbyElements) lines.push(`- ${inlineText(element)}`);
  }
  const styleLines = formatStyles(target.computedStyles);
  if (styleLines.length > 0) {
    lines.push("**Computed styles:**");
    lines.push(...styleLines);
  }
  if (target.fullPath) lines.push(`**Full DOM path:** ${inlineCode(target.fullPath)}`);
  if (target.htmlSnippet) {
    lines.push("**HTML:**");
    lines.push(...fence("html", target.htmlSnippet));
  }
  return lines;
}

/**
 * The notes as the Markdown a coding agent receives: a `## Design Feedback: <page>` heading with the URL, then one
 * `### n. <element>` block per note with its intent, selector, source file, text, feedback and image files. Notes from
 * several pages are grouped under one heading per page, and `includeDiagnostics` adds the page details to each note.
 * @param {import("./types.js").PageAnnotation[]} annotations
 * @param {{ includeDiagnostics?: boolean }} [options]
 * @returns {string} Empty when there is no note.
 */
export function formatAnnotationsAsMarkdown(annotations, { includeDiagnostics = false } = {}) {
  if (annotations.length === 0) return "";

  const firstAnnotation = annotations[0];
  const first = firstAnnotation.payload;
  const groups = new Map();
  annotations.forEach((annotation, index) => {
    const url = annotation.payload.page.sanitizedUrl;
    const entries = groups.get(url) ?? [];
    entries.push({ annotation, index });
    groups.set(url, entries);
  });
  const mixedPages = groups.size > 1;
  const lines = mixedPages
    ? ["## Design Feedback across pages", ""]
    : [
        `## Design Feedback: ${formatPageHeading(first)}`,
        "",
        `**URL:** ${first.page.sanitizedUrl}`,
        ...(includeDiagnostics
          ? [`**Browser tab id:** ${firstAnnotation.browserPageId}`, `**Viewport:** ${first.page.viewportWidth}x${first.page.viewportHeight}`]
          : []),
        "",
      ];

  for (const [url, entries] of groups) {
    if (mixedPages) {
      lines.push(`## Design Feedback: ${inlineText(formatPageHeading(entries[0].annotation.payload))}`);
      lines.push("", `**URL:** ${inlineCode(inlineText(url))}`, "");
    }
    for (const { annotation, index } of entries) {
      const { payload } = annotation;
      const { target } = payload;

      lines.push(`### ${index + 1}. ${annotationElementLabel(payload)}`);
      if (mixedPages && includeDiagnostics) {
        lines.push(`**Browser tab id:** ${inlineCode(inlineText(annotation.browserPageId))}`);
        lines.push(`**Viewport:** ${payload.page.viewportWidth}x${payload.page.viewportHeight}`);
      }
      lines.push(`**Intent:** ${annotation.intent}`);
      lines.push(`**Selector:** ${inlineCode(target.selector)}`);
      if (target.sourceFile) lines.push(`**Source:** ${inlineText(target.sourceFile)}`);
      if (target.selectedText) lines.push(`**Selected text:** "${inlineText(target.selectedText)}"`);
      else if (target.textSnippet) lines.push(`**Text:** "${inlineText(target.textSnippet)}"`);
      if (includeDiagnostics) lines.push(...formatDiagnostics(payload));
      lines.push(`**Feedback:** ${inlineText(annotation.comment)}`);
      if (annotation.imagePaths?.length) {
        lines.push("**Images (open these files):**");
        for (const imagePath of annotation.imagePaths) lines.push(`- ${inlineCode(imagePath)}`);
      }
      lines.push("");
    }
  }

  return lines.join("\n").trimEnd();
}
