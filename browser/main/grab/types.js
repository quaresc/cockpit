// Ported from Orca (https://github.com/stablyai/orca, commit 49498c3), src/shared/browser/element-grab/browser-grab-types.ts.
// MIT licence, Copyright (c) 2026 Lovecast Inc. Translated from TypeScript and adapted to cockpit's browser shell.

/**
 * Page-level facts captured when an element is picked.
 * @typedef {object} GrabPageContext
 * @property {string} sanitizedUrl The page URL without query or fragment, or a local file path.
 * @property {string} title
 * @property {number} viewportWidth
 * @property {number} viewportHeight
 * @property {number} scrollX
 * @property {number} scrollY
 * @property {number} devicePixelRatio
 * @property {string} capturedAt ISO date; the arm script also uses it as the selection id.
 */

/**
 * Accessibility facts of the picked element.
 * @typedef {object} GrabAccessibility
 * @property {string | null} role
 * @property {string | null} accessibleName
 * @property {string | null} ariaLabel
 * @property {string | null} ariaLabelledBy
 */

/**
 * The computed styles a payload carries.
 * @typedef {object} GrabComputedStyles
 * @property {string} display
 * @property {string} position
 * @property {string} width
 * @property {string} height
 * @property {string} margin
 * @property {string} padding
 * @property {string} color
 * @property {string} backgroundColor
 * @property {string} border
 * @property {string} borderRadius
 * @property {string} fontFamily
 * @property {string} fontSize
 * @property {string} fontWeight
 * @property {string} lineHeight
 * @property {string} textAlign
 * @property {string} zIndex
 */

/**
 * A rectangle in CSS pixels, relative to the viewport or to the page.
 * @typedef {object} GrabRect
 * @property {number} x
 * @property {number} y
 * @property {number} width
 * @property {number} height
 */

/**
 * What the picker reads from the picked element.
 * @typedef {object} GrabTarget
 * @property {string} tagName
 * @property {string} selector A CSS selector unique in the document when one exists.
 * @property {string} [elementPath] A short readable path made of ids, aria labels, roles and stable classes.
 * @property {string} [fullPath] The selector parts of every ancestor up to the root.
 * @property {string} [cssClasses]
 * @property {string[]} [nearbyElements]
 * @property {string | null} [selectedText]
 * @property {boolean} [isFixed] Whether the element or an ancestor is fixed or sticky.
 * @property {string | null} [reactComponents] The component chain, outermost first, such as `<Checkout> <PayButton>`.
 * @property {string | null} [sourceFile] `file:line:column` from React's `_debugSource`.
 * @property {string} textSnippet
 * @property {string} htmlSnippet
 * @property {Record<string, string>} attributes
 * @property {GrabAccessibility} accessibility
 * @property {GrabRect} rectViewport
 * @property {GrabRect} rectPage
 * @property {GrabComputedStyles} computedStyles
 */

/**
 * A PNG crop of the picked element, sized in CSS pixels.
 * @typedef {object} GrabScreenshot
 * @property {"image/png"} mimeType
 * @property {string} dataUrl
 * @property {number} width
 * @property {number} height
 */

/**
 * Everything one pick yields.
 * @typedef {object} GrabPayload
 * @property {GrabPageContext} page
 * @property {GrabTarget} target
 * @property {string[]} nearbyText
 * @property {string[]} ancestorPath
 * @property {GrabScreenshot | null} screenshot
 */

/** @typedef {"copy" | "annotate"} GrabIntent */

/** @typedef {"arm" | "awaitClick" | "finalize" | "extractHover" | "teardown"} GrabStep */

/**
 * Why a pick ended without a selection.
 * @typedef {"user" | "tab-inactive" | "navigation" | "evicted" | "timeout"} GrabCancelReason
 */

/**
 * How one pick ended.
 * @typedef {{ opId: string, kind: "selected", payload: GrabPayload }
 *   | { opId: string, kind: "context-selected", payload: GrabPayload }
 *   | { opId: string, kind: "cancelled", reason: GrabCancelReason }
 *   | { opId: string, kind: "error", reason: string }} GrabResult
 */

/** @typedef {"fix" | "change" | "question" | "approve"} AnnotationIntent */

/** @typedef {"blocking" | "important" | "suggestion"} AnnotationPriority */

/**
 * One note a person leaves on a picked element.
 * @typedef {object} PageAnnotation
 * @property {string} id
 * @property {string} browserPageId The page (tab) the note belongs to.
 * @property {string} comment
 * @property {AnnotationIntent} intent
 * @property {AnnotationPriority} priority
 * @property {string} createdAt
 * @property {GrabPayload} payload Kept without its screenshot.
 * @property {string[]} [imagePaths] Local image files attached to the note.
 */

/** The steps of a pick, in the order a session runs them. */
export const GRAB_STEPS = Object.freeze(["arm", "awaitClick", "finalize", "extractHover", "teardown"]);

/** The intents a note can carry. */
export const ANNOTATION_INTENTS = Object.freeze(["fix", "change", "question", "approve"]);

/** The priorities a note can carry. */
export const ANNOTATION_PRIORITIES = Object.freeze(["blocking", "important", "suggestion"]);

/** The priority a new note starts with. */
export const DEFAULT_ANNOTATION_PRIORITY = "important";

/** The size limits of a payload, enforced in the page and again in the main process. */
export const GRAB_BUDGET = Object.freeze({
  textSnippetMaxLength: 200,
  nearbyTextEntryMaxLength: 200,
  nearbyTextMaxEntries: 10,
  htmlSnippetMaxLength: 4096,
  ancestorPathMaxEntries: 10,
  nearbyElementsMaxEntries: 6,
  nearbyElementMaxLength: 160,
  selectorMaxLength: 700,
  pathMaxLength: 900,
  cssClassesMaxLength: 500,
  selectedTextMaxLength: 500,
  sourceFileMaxLength: 500,
  reactComponentsMaxLength: 500,
  annotationCommentMaxLength: 2000,
  annotationsMaxPerPage: 20,
  /** A PNG larger than this many bytes is dropped. */
  screenshotMaxBytes: 2 * 1024 * 1024,
});

/** Whether a page already holds as many notes as it can. */
export function isAnnotationPageFull(annotationCount) {
  return annotationCount >= GRAB_BUDGET.annotationsMaxPerPage;
}

/** The attribute names a payload may carry, besides `aria-*`. */
export const GRAB_SAFE_ATTRIBUTE_NAMES = new Set([
  "id",
  "class",
  "name",
  "type",
  "role",
  "href",
  "src",
  "alt",
  "title",
  "placeholder",
  "for",
  "action",
  "method",
]);

/** Whether an attribute is an `aria-*` one, which a payload always carries. */
export function isAriaAttribute(name) {
  return name.startsWith("aria-");
}

/**
 * Substrings that mark a value as a credential to redact. They are credential-shaped on purpose: broad words such as
 * "code" or "state" match ordinary CSS classes.
 */
export const GRAB_SECRET_PATTERNS = Object.freeze([
  "access_token",
  "auth_token",
  "api_key",
  "apikey",
  "client_secret",
  "oauth_state",
  "x-amz-",
  "session_id",
  "sessionid",
  "csrf",
  "secret",
  "password",
  "passwd",
]);

/** The global the arm script installs in the page. */
export const GRAB_GLOBAL = "__cockpitGrab";

/** The attribute the picker sets on the element it froze on, holding the selection id. */
export const GRAB_SELECTED_ATTRIBUTE = "data-cockpit-grab-selected";

/** The key that marks an `awaitClick` result as a right-click pick, with the payload under `payload`. */
export const GRAB_CONTEXT_MENU_KEY = "__cockpitContextMenu";

/** The key that marks an `awaitClick` result as a cancelled pick. */
export const GRAB_CANCELLED_KEY = "__cockpitCancelled";
