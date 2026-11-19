// Ported from Orca (https://github.com/stablyai/orca, commit 49498c3), src/main/browser/cookies/chromium/prepare-import.ts,
// chromium/scan-source-rows.ts, chromium/finalize-import.ts and sources/chromium-cookie-time.ts.
// MIT licence, Copyright (c) 2026 Lovecast Inc. Translated from TypeScript and adapted to cockpit's browser shell.

import { existsSync } from "node:fs";
import { DatabaseSync } from "node:sqlite";
import { decryptCookieValueRaw, deriveMacEncryptionKey } from "./decrypt.js";
import { failure, isMacPrivacyDenial } from "./failure.js";
import { readChromiumRowPartition } from "./partition.js";
import { isGoogleSourceBoundCookie, isNonTransplantableCookieDomain, normalizeCookieImportDomain } from "./policy.js";
import { createCookieSnapshot } from "./snapshot.js";
import { databaseSameSite, isExpired } from "./validate.js";

const CHROMIUM_EPOCH_OFFSET = 11644473600n;

/** A Chromium timestamp (microseconds since 1601) as Unix seconds, or 0 when it is unset or unreadable. */
export function chromiumTimestampToUnix(chromiumTs) {
  if (!chromiumTs || chromiumTs === "0") return 0;
  try {
    const ts = typeof chromiumTs === "bigint" ? chromiumTs : BigInt(typeof chromiumTs === "number" ? Math.round(chromiumTs) : chromiumTs);
    if (ts === 0n) return 0;
    return Math.max(Number(ts / 1000000n - CHROMIUM_EPOCH_OFFSET), 0);
  } catch {
    return 0;
  }
}

function count(skipped, reason) {
  skipped[reason] = (skipped[reason] ?? 0) + 1;
}

/** Whether policy leaves a row behind whatever its value, so it is never decrypted. */
function leftBehind(row) {
  if (isGoogleSourceBoundCookie(row.name, row.host_key)) return "googleSourceBound";
  return isNonTransplantableCookieDomain(row.host_key) ? "nonTransplantableDomain" : null;
}

function hasEncryptedValue(row) {
  return row.encrypted_value instanceof Uint8Array && row.encrypted_value.length > 0;
}

/** Reads every row of a snapshot's `cookies` table, with the table's column names. */
function readRows(databasePath) {
  // Chromium's microsecond timestamps exceed what a JavaScript number holds exactly.
  const database = new DatabaseSync(databasePath, { readOnly: true, readBigInts: true });
  try {
    const columns = new Set(
      database
        .prepare("PRAGMA table_info(cookies)")
        .all()
        .flatMap((column) => (typeof column.name === "string" ? [column.name] : [])),
    );
    return { columns, rows: database.prepare("SELECT * FROM cookies ORDER BY rowid").all() };
  } finally {
    database.close();
  }
}

/** A row's value as bytes: decrypted when it is stored encrypted, else the plain column; null when it cannot be decrypted. */
function rowValue(row, key) {
  if (hasEncryptedValue(row)) {
    // node:sqlite returns a BLOB as a Uint8Array; a version with no key on macOS comes back null.
    return key ? decryptCookieValueRaw(Buffer.from(row.encrypted_value), key, row.host_key) : null;
  }
  if (row.value instanceof Uint8Array) return Buffer.from(row.value);
  return typeof row.value === "string" ? Buffer.from(row.value, "latin1") : Buffer.alloc(0);
}

/** Turns the snapshot's rows into cookies, counting each row it leaves out under the reason why. */
function scanRows(rows, columns, key, nowSeconds) {
  const cookies = [];
  const skipped = {};
  const domainValidity = new Map();
  for (const row of rows) {
    if (typeof row.host_key !== "string" || typeof row.name !== "string") {
      count(skipped, "invalid");
      continue;
    }
    const policy = leftBehind(row);
    if (policy) {
      count(skipped, policy);
      continue;
    }
    const value = rowValue(row, key);
    if (!value) {
      count(skipped, "undecryptable");
      continue;
    }
    const domain = row.host_key;
    let validDomain = domainValidity.get(domain);
    if (validDomain === undefined) {
      validDomain = normalizeCookieImportDomain(domain) !== null;
      domainValidity.set(domain, validDomain);
    }
    if (!validDomain) {
      count(skipped, "invalid");
      continue;
    }
    const expiresUtc = chromiumTimestampToUnix(row.expires_utc);
    const cookie = {
      name: row.name,
      // latin1 keeps each raw byte of the value as one character.
      value: value.toString("latin1"),
      domain,
      path: typeof row.path === "string" ? row.path : "/",
      secure: row.is_secure === 1n,
      httpOnly: row.is_httponly === 1n,
      sameSite: databaseSameSite(Number(row.samesite ?? -1)),
      ...(expiresUtc > 0 ? { expirationDate: expiresUtc } : {}),
      partition: readChromiumRowPartition(row, columns),
    };
    if (isExpired(cookie, nowSeconds)) {
      count(skipped, "expired");
      continue;
    }
    cookies.push(cookie);
  }
  return { cookies, skipped };
}

/**
 * Reads the cookies of one Chromium profile from a snapshot of its database, asking for the Safe Storage password
 * only when a row that policy lets through is encrypted.
 * @param {{label: string, cookiesPath: string, keychain: {service: string, account: string}}} browser
 * @param {{getKey: (service: string, account: string) => Promise<string | null> | string | null, tmpDir?: string, now?: number, copyFile?: (from: string, to: string) => void}} options
 * @returns {Promise<{ok: true, cookies: import("./validate.js").ImportCookie[], skipped: Record<string, number>, total: number} | {ok: false, reason: string, message: string}>}
 */
export async function readChromiumCookies(browser, options) {
  const named = { browser: browser.label };
  if (!existsSync(browser.cookiesPath)) return failure("databaseNotFound", named);

  let snapshot;
  try {
    // Recent cookies may live only in the write-ahead log, so the copy takes both files.
    snapshot = createCookieSnapshot(browser.cookiesPath, { tempRoot: options.tmpDir, copyFile: options.copyFile });
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

  let key = null;
  if (source.rows.some((row) => hasEncryptedValue(row) && !leftBehind(row))) {
    let password = null;
    try {
      password = await options.getKey(browser.keychain.service, browser.keychain.account);
    } catch {}
    if (typeof password !== "string" || password.length === 0) return failure("keyDenied", named);
    key = deriveMacEncryptionKey(password);
  }

  const nowSeconds = (options.now ?? Date.now()) / 1000;
  return { ok: true, ...scanRows(source.rows, source.columns, key, nowSeconds), total: source.rows.length };
}
