// Design mode's element picker: the scripts injected in a page, and the pure helpers around them. Ported from Orca
// (https://github.com/stablyai/orca, commit 49498c3, MIT licence, Copyright (c) 2026 Lovecast Inc.); each file names its source.
//
// Step scripts: `grabScript(step)` returns the string to pass to `webContents.executeJavaScript`, which resolves to:
//   "arm"           `true`. Installs `window.__cockpitGrab` and the overlay, and starts the hover outline; arming again
//                   replaces the earlier picker.
//   "awaitClick"    a promise, settled by the next trusted click or right-click:
//                     click        -> the raw payload `{ page, target, nearbyText, ancestorPath, screenshot: null }`
//                     right-click  -> `{ __cockpitContextMenu: true, payload }`
//                     teardown     -> `{ __cockpitCancelled: true }`
//                   It rejects with "Grab not armed" before an arm, "cancelled" when nothing is hovered, or the
//                   extraction error. After a pick the outline stays frozen on the element, which carries
//                   `data-cockpit-grab-selected="<payload.page.capturedAt>"`, until a teardown or a new arm.
//                   Pass the result to `readGrabSelection`, a rejection to `readGrabFailure`.
//                   Presses are kept from the page from the arm on, so a click that lands before this script runs
//                   is dropped without a pick: run it right after the arm.
//   "finalize"      the raw payload of the hovered element, or `null`; removes the picker.
//   "extractHover"  the raw payload of the hovered element, or `null`; leaves the picker armed.
//   "teardown"      `true`. Removes the picker and settles a pending "awaitClick" with the cancel marker.
// A raw payload is page-controlled: pass it through `clampGrabPayload` (or `readGrabSelection`) before using it.
//
// scripts.js
//   grabScript(step)                         the script of one step, as above; throws on an unknown step.
// types.js (typedefs: GrabPayload, GrabTarget, GrabPageContext, GrabRect, GrabScreenshot, GrabResult, GrabStep,
//           GrabIntent, GrabCancelReason, PageAnnotation, AnnotationIntent, AnnotationPriority)
//   GRAB_STEPS                               ["arm", "awaitClick", "finalize", "extractHover", "teardown"].
//   GRAB_BUDGET                              every size limit, including 20 notes per page and 2000 characters per comment.
//   GRAB_SAFE_ATTRIBUTE_NAMES                the Set of attribute names a payload may carry besides aria-*.
//   GRAB_SECRET_PATTERNS                     the substrings that get a value redacted.
//   ANNOTATION_INTENTS                       ["fix", "change", "question", "approve"].
//   ANNOTATION_PRIORITIES                    ["blocking", "important", "suggestion"].
//   DEFAULT_ANNOTATION_PRIORITY              "important".
//   GRAB_GLOBAL                              "__cockpitGrab", the global the arm script installs.
//   GRAB_SELECTED_ATTRIBUTE                  "data-cockpit-grab-selected", set on the frozen element.
//   GRAB_CONTEXT_MENU_KEY                    "__cockpitContextMenu".
//   GRAB_CANCELLED_KEY                       "__cockpitCancelled".
//   isAnnotationPageFull(count)              whether a page already holds 20 notes.
//   isAriaAttribute(name)                    whether an attribute name starts with "aria-".
// payload.js
//   clampGrabPayload(raw, documentUrl?)      a GrabPayload with every field bounded, filtered and redacted, or null when
//                                            `raw` has no page or target; a `file:` documentUrl names the page by its path.
//   sanitizeGrabUrl(url)                     the URL without query and fragment, "" unless http, https, file or about:blank.
//   containsGrabSecret(value)                whether a value holds a secret pattern.
//   isSafeDisplayCharacter(character)        false for control, format and line-separator characters.
//   sanitizeAnnotation(annotation)           the note with its comment cut to 2000 characters and no screenshot.
// selection.js
//   readGrabSelection(raw, documentUrl?)     `{ kind: "cancelled" }`, `{ kind: "selected" | "context-selected", payload }`
//                                            with a clamped payload, or `{ kind: "error", reason }`.
//   readGrabFailure(error)                   `{ kind: "cancelled" }` or `{ kind: "error", reason }` for a rejected awaitClick.
//   GRAB_TIMEOUT_MS                          120000, how long a session waits for a click.
// screenshot.js
//   captureSelectionScreenshot(rect, contents)   `Promise<{ mimeType: "image/png", dataUrl, width, height } | null>`, width
//                                            and height in CSS pixels. `contents` needs `executeJavaScript(code)` and
//                                            `capturePage()` resolving to an image with `getSize`, `crop`, `toPNG`, `isEmpty?`.
//   selectionCropRect(rect, bitmapSize, viewportCssWidth)   `{ x, y, width, height, scaleFactor }` in bitmap pixels, or null.
//   HIDE_GRAB_OVERLAY_SCRIPT, RESTORE_GRAB_OVERLAY_SCRIPT   hide and show the picker and the note badges around a capture.
// markdown.js
//   formatAnnotationsAsMarkdown(annotations, { includeDiagnostics? })   the notes as Markdown for a coding agent, "" for none.
//   ANNOTATION_INLINE_TEXT_MAX_LENGTH        2048, the cap on one line of page-controlled text.
// text.js
//   formatGrabPayloadAsText(payload)         one picked element as the plain text a grab copies.
// keys.js
//   grabKeyAction(input)                     "copy" (bare C), "screenshot" (bare S), "cancel" (bare Escape) or null, from
//                                            the input of a `before-input-event` while a pick is armed.
//   GRAB_TOGGLE_GUARD_SCRIPT                 evaluates to `false` while the focus is in an editable element or text is
//                                            selected, `true` when a shortcut may arm the picker.
// viewport-bridge.js (typedefs: AnnotationViewportMarker, AnnotationViewportBridgeOptions)
//   buildAnnotationViewportBridgeScript({ enabled, emitViewport, markers, selectionId?, token })
//                                            the script for `executeJavaScriptInIsolatedWorld(ANNOTATION_VIEWPORT_BRIDGE_WORLD_ID,
//                                            [{ code }])`; evaluates to `true`. It draws the numbered badges, and with
//                                            `emitViewport` writes `console.debug(PREFIX + token + ":" + json)` on scroll and resize.
//   ANNOTATION_VIEWPORT_BRIDGE_WORLD_ID      1207.
//   isValidAnnotationViewportBridgeToken(value)     whether a token is 16 to 80 URL-safe characters.
//   isValidAnnotationViewportBridgeMarkers(value)   whether a value is at most 50 well-formed markers.
// viewport-report.js (typedef: AnnotationViewportReport)
//   readAnnotationViewportMessage(message, token)   `{ scrollX, scrollY, viewportWidth?, target? }` from a console line
//                                            carrying this token, else null.
//   parseAnnotationViewportReport(json)      the same from the JSON alone.
//   isValidGrabRect(value)                   whether a value is a finite rectangle of non-negative size.
//   ANNOTATION_VIEWPORT_MESSAGE_PREFIX       "__cockpit_annotation_viewport__:".

export { grabScript } from "./scripts.js";
export {
  ANNOTATION_INTENTS,
  ANNOTATION_PRIORITIES,
  DEFAULT_ANNOTATION_PRIORITY,
  GRAB_BUDGET,
  GRAB_CANCELLED_KEY,
  GRAB_CONTEXT_MENU_KEY,
  GRAB_GLOBAL,
  GRAB_SAFE_ATTRIBUTE_NAMES,
  GRAB_SECRET_PATTERNS,
  GRAB_SELECTED_ATTRIBUTE,
  GRAB_STEPS,
  isAnnotationPageFull,
  isAriaAttribute,
} from "./types.js";
export { clampGrabPayload, containsGrabSecret, isSafeDisplayCharacter, sanitizeAnnotation, sanitizeGrabUrl } from "./payload.js";
export { GRAB_TIMEOUT_MS, readGrabFailure, readGrabSelection } from "./selection.js";
export { HIDE_GRAB_OVERLAY_SCRIPT, RESTORE_GRAB_OVERLAY_SCRIPT, captureSelectionScreenshot, selectionCropRect } from "./screenshot.js";
export { ANNOTATION_INLINE_TEXT_MAX_LENGTH, formatAnnotationsAsMarkdown } from "./markdown.js";
export { formatGrabPayloadAsText } from "./text.js";
export { GRAB_TOGGLE_GUARD_SCRIPT, grabKeyAction } from "./keys.js";
export {
  ANNOTATION_VIEWPORT_BRIDGE_WORLD_ID,
  buildAnnotationViewportBridgeScript,
  isValidAnnotationViewportBridgeMarkers,
  isValidAnnotationViewportBridgeToken,
} from "./viewport-bridge.js";
export { ANNOTATION_VIEWPORT_MESSAGE_PREFIX, isValidGrabRect, parseAnnotationViewportReport, readAnnotationViewportMessage } from "./viewport-report.js";
