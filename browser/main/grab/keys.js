// Ported from Orca (https://github.com/stablyai/orca, commit 49498c3),
// src/main/browser/guest-lifecycle/input/shortcuts/browser-guest-grab-shortcuts.ts and
// src/main/browser/guest-lifecycle/input/shortcuts/browser-guest-escape-forwarding.ts.
// MIT licence, Copyright (c) 2026 Lovecast Inc. Translated from TypeScript and adapted to cockpit's browser shell.

/**
 * What a key pressed in the page means while a pick is armed: "copy" for a bare C, "screenshot" for a bare S, "cancel"
 * for a bare Escape, or null for a key that stays the page's. The caller prevents the default of a key it acts on.
 * @param {{ type?: string, key?: string, meta?: boolean, control?: boolean, alt?: boolean, shift?: boolean, isAutoRepeat?: boolean }} input
 *   The input of a `before-input-event`.
 * @returns {"copy" | "screenshot" | "cancel" | null}
 */
export function grabKeyAction(input) {
  if (!input || input.type !== "keyDown" || input.meta || input.control || input.alt || input.shift) return null;
  const key = String(input.key ?? "").toLowerCase();
  if (key === "c") return "copy";
  if (key === "s") return "screenshot";
  if (key === "escape" && !input.isAutoRepeat) return "cancel";
  return null;
}

/**
 * Evaluates in the page to whether a shortcut may arm the picker: false while the focus is in an editable element or a
 * frame, or while text is selected, so the keys keep their usual meaning there.
 */
export const GRAB_TOGGLE_GUARD_SCRIPT = `(() => {
  const active = document.activeElement
  const tag = active?.tagName
  const isEditable =
    active instanceof HTMLInputElement ||
    active instanceof HTMLTextAreaElement ||
    active?.isContentEditable === true ||
    tag === 'SELECT' ||
    tag === 'IFRAME'
  if (isEditable) {
    return false
  }
  const selection = window.getSelection()
  return Boolean(selection && selection.type === 'Range' && selection.toString().trim().length > 0)
    ? false
    : true
})()`;
