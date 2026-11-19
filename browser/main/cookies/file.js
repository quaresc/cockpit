// Ported from Orca (https://github.com/stablyai/orca, commit 49498c3), src/main/browser/cookies/imports/import-cookie-file.ts.
// MIT licence, Copyright (c) 2026 Lovecast Inc. Translated from TypeScript and adapted to cockpit's browser shell.

import { readFile } from "node:fs/promises";
import { failure } from "./failure.js";
import { stripNonPrintable, validateCookieEntry } from "./validate.js";

/**
 * Reads a cookie file: a JSON array of objects with `domain`, `name` and `value`, and optionally `path`, `secure`,
 * `httpOnly`, `sameSite`, `expirationDate` (Unix seconds), `partitionKey` and `partitionKeyOpaque`.
 * @param {string} filePath
 * @returns {Promise<{ok: true, cookies: import("./validate.js").ImportCookie[], skipped: Record<string, number>, total: number} | {ok: false, reason: string, message: string}>}
 */
export async function readCookieFile(filePath) {
  let rawContent;
  try {
    rawContent = await readFile(filePath, "utf-8");
  } catch {
    return failure("fileUnreadable");
  }

  let parsed;
  try {
    parsed = JSON.parse(rawContent);
  } catch {
    return failure("fileInvalidJson");
  }
  if (!Array.isArray(parsed)) return failure("fileNotArray");
  if (parsed.length === 0) return failure("fileEmpty");

  const cookies = [];
  for (const entry of parsed) {
    const cookie = typeof entry === "object" && entry !== null ? validateCookieEntry(entry) : null;
    if (cookie) cookies.push({ ...cookie, value: stripNonPrintable(cookie.value) });
  }
  const invalid = parsed.length - cookies.length;
  if (cookies.length === 0) return failure("fileNoValidCookies", { count: invalid });
  return { ok: true, cookies, skipped: invalid > 0 ? { invalid } : {}, total: parsed.length };
}
