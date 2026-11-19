// Ported from Orca (https://github.com/stablyai/orca, commit 49498c3), src/main/browser/cookies/sources/browser-cookie-firefox-import.ts.
// MIT licence, Copyright (c) 2026 Lovecast Inc. Translated from TypeScript and adapted to cockpit's browser shell.

import { existsSync } from "node:fs";
import { DatabaseSync } from "node:sqlite";
import { failure, isMacPrivacyDenial } from "./failure.js";
import { readFirefoxRowPartition } from "./partition.js";
import { createCookieSnapshot } from "./snapshot.js";
import { databaseSameSite, deriveUrl, stripNonPrintable } from "./validate.js";

// An expiry past this many seconds (the year 5138) is one of the millisecond expiries newer Firefox schemas store.
const MILLISECOND_EXPIRY_FLOOR = 100_000_000_000;

function expirySeconds(raw) {
  const expiry = Number(raw ?? 0);
  if (!Number.isFinite(expiry) || expiry <= 0) return 0;
  return expiry > MILLISECOND_EXPIRY_FLOOR ? Math.floor(expiry / 1000) : expiry;
}

function readRows(databasePath) {
  const database = new DatabaseSync(databasePath, { readOnly: true });
  try {
    const columns = new Set(
      database
        .prepare("PRAGMA table_info(moz_cookies)")
        .all()
        .map((column) => column.name),
    );
    // Older Firefox schemas have no column for the Partitioned attribute.
    const partitionColumn = columns.has("isPartitionedAttributeSet") ? ", isPartitionedAttributeSet" : "";
    const rows = database.prepare(`SELECT name, value, host, path, expiry, isSecure, isHttpOnly, sameSite${partitionColumn} FROM moz_cookies`).all();
    return { columns, rows };
  } finally {
    database.close();
  }
}

/**
 * Reads the cookies of one Firefox profile from a snapshot of its `cookies.sqlite`.
 * @param {{label: string, cookiesPath: string}} browser
 * @param {{tmpDir?: string, now?: number}} [options]
 * @returns {Promise<{ok: true, cookies: import("./validate.js").ImportCookie[], skipped: Record<string, number>, total: number} | {ok: false, reason: string, message: string}>}
 */
export async function readFirefoxCookies(browser, options = {}) {
  const named = { browser: browser.label };
  if (!existsSync(browser.cookiesPath)) return failure("databaseNotFound", named);

  let snapshot;
  try {
    snapshot = createCookieSnapshot(browser.cookiesPath, { tempRoot: options.tmpDir, fileName: "cookies.sqlite" });
  } catch (error) {
    return failure(isMacPrivacyDenial(error) ? "fullDiskAccess" : "databaseCopyFailed", named);
  }

  let source;
  try {
    source = readRows(snapshot.databasePath);
  } catch {
    return failure("readFailed", named);
  } finally {
    snapshot.cleanup();
  }
  if (source.rows.length === 0) return failure("noCookies", named);

  const nowSeconds = Math.floor((options.now ?? Date.now()) / 1000);
  const cookies = [];
  const skipped = {};
  const skip = (reason) => {
    skipped[reason] = (skipped[reason] ?? 0) + 1;
  };
  for (const row of source.rows) {
    if (typeof row.name !== "string" || !row.name || typeof row.host !== "string" || !row.host) {
      skip("invalid");
      continue;
    }
    const expiry = expirySeconds(row.expiry);
    if (expiry > 0 && expiry < nowSeconds) {
      skip("expired");
      continue;
    }
    const secure = row.isSecure === 1;
    if (!deriveUrl(row.host, secure)) {
      skip("invalid");
      continue;
    }
    cookies.push({
      name: row.name,
      value: stripNonPrintable(typeof row.value === "string" ? row.value : ""),
      domain: row.host,
      path: row.path || "/",
      secure,
      httpOnly: row.isHttpOnly === 1,
      sameSite: databaseSameSite(row.sameSite),
      ...(expiry > 0 ? { expirationDate: expiry } : {}),
      partition: readFirefoxRowPartition(row, source.columns),
    });
  }
  if (cookies.length === 0) return failure("noValidCookies", named);
  return { ok: true, cookies, skipped, total: source.rows.length };
}
