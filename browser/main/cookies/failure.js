// Ported from Orca (https://github.com/stablyai/orca, commit 49498c3), src/shared/browser/cookies/browser-cookie-import-failure-copy.ts
// and src/main/browser/cookies/sources/browser-cookie-macos-privacy-denial.ts.
// MIT licence, Copyright (c) 2026 Lovecast Inc. Translated from TypeScript and adapted to cockpit's browser shell.

/** One English sentence per refusal; `{{browser}}` and `{{count}}` are filled in by `failure`. */
export const FAILURE_MESSAGES = {
  invalidProfile: "{{browser}} has no such profile.",
  databaseNotFound: "{{browser}} cookies database not found.",
  databaseCopyFailed: "Could not copy {{browser}} cookies database. Try closing {{browser}} first.",
  fullDiskAccess:
    "macOS denied access to {{browser}} cookies. Grant Full Disk Access to Cockpit Browser in System Settings → Privacy & Security → Full Disk Access, then restart it.",
  keyDenied: "Could not access {{browser}} encryption key. macOS may have denied access to the Keychain.",
  noCookies: "No cookies found in {{browser}}.",
  noValidCookies: "No valid cookies found in {{browser}}.",
  allCookiesExpired: "All {{browser}} cookies are expired.",
  readFailed: "Could not read {{browser}} cookies. Try closing {{browser}} first.",
  fileUnreadable: "Could not read the selected file.",
  fileInvalidJson: "File is not valid JSON.",
  fileNotArray: "Expected a JSON array of cookie objects.",
  fileEmpty: "Cookie file is empty.",
  fileNoValidCookies: "No valid cookies found. {{count}} entries were skipped due to missing or invalid fields.",
};

/**
 * A refusal as a value: the code to branch on and the sentence to show.
 * @param {keyof typeof FAILURE_MESSAGES} reason
 * @param {Record<string, string | number>} [params]
 * @returns {{ok: false, reason: string, message: string}}
 */
export function failure(reason, params = {}) {
  const message = FAILURE_MESSAGES[reason].replace(/\{\{(\w+)\}\}/g, (slot, name) => (Object.hasOwn(params, name) ? String(params[name]) : slot));
  return { ok: false, reason, message };
}

/** Whether a file error is macOS refusing access to another app's data, which only Full Disk Access lifts. */
export function isMacPrivacyDenial(error) {
  return error instanceof Error && error.code === "EPERM";
}
