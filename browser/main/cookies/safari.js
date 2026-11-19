// Ported from Orca (https://github.com/stablyai/orca, commit 49498c3), src/main/browser/cookies/sources/browser-cookie-safari-parser.ts
// and sources/browser-cookie-safari-import.ts.
// MIT licence, Copyright (c) 2026 Lovecast Inc. Translated from TypeScript and adapted to cockpit's browser shell.

import { readFileSync } from "node:fs";
import { failure, isMacPrivacyDenial } from "./failure.js";
import { UNPARTITIONED } from "./partition.js";
import { deriveUrl, stripNonPrintable } from "./validate.js";

// Safari dates count seconds from 2001-01-01.
const MAC_EPOCH_DELTA = 978_307_200;

function readCString(buf, offset, end) {
  if (offset < 0 || offset >= end) return null;
  let cursor = offset;
  while (cursor < end && buf[cursor] !== 0) cursor += 1;
  if (cursor >= end) return null;
  return buf.toString("utf8", offset, cursor);
}

/** One cookie record of a page, or null when the record is truncated or names no host. */
export function decodeSafariCookie(buf) {
  if (buf.length < 48) return null;
  // The record's own size is not trusted past the bytes that are there.
  const size = Math.min(buf.readUInt32LE(0), buf.length);
  if (size < 48) return null;

  const flags = buf.readUInt32LE(8);
  const secure = (flags & 1) !== 0;
  const httpOnly = (flags & 4) !== 0;
  const urlOffset = buf.readUInt32LE(16);
  const nameOffset = buf.readUInt32LE(20);
  const pathOffset = buf.readUInt32LE(24);
  const valueOffset = buf.readUInt32LE(28);
  const expiration = buf.readDoubleLE(40);

  const name = readCString(buf, nameOffset, size);
  if (!name) return null;
  // The format keeps the cookie's domain in the field it calls URL.
  const domain = readCString(buf, urlOffset, size) ?? "";
  if (!domain || !deriveUrl(domain, secure)) return null;

  return {
    name,
    value: readCString(buf, valueOffset, size) ?? "",
    domain,
    path: readCString(buf, pathOffset, size) ?? "/",
    secure,
    httpOnly,
    sameSite: "unspecified",
    ...(expiration > 0 ? { expirationDate: Math.round(expiration + MAC_EPOCH_DELTA) } : {}),
    // The format has no partition field.
    partition: UNPARTITIONED,
  };
}

/** The cookies of one page: a count, an offset per record, then the records. */
export function decodeSafariPage(page) {
  if (page.length < 16 || page.readUInt32BE(0) !== 0x00000100) return [];
  const cookieCount = page.readUInt32LE(4);
  if (8 + cookieCount * 4 > page.length) return [];
  const cookies = [];
  for (let i = 0; i < cookieCount; i += 1) {
    const cookie = decodeSafariCookie(page.subarray(page.readUInt32LE(8 + i * 4)));
    if (cookie) cookies.push(cookie);
  }
  return cookies;
}

/**
 * Every cookie of a `Cookies.binarycookies` file: the `cook` magic, a page count, each page's size, then the pages.
 * @param {Buffer} buffer
 * @returns {import("./validate.js").ImportCookie[]}
 */
export function decodeSafariBinaryCookies(buffer) {
  if (buffer.length < 8 || buffer.subarray(0, 4).toString("utf8") !== "cook") return [];
  const pageCount = buffer.readUInt32BE(4);
  let cursor = 8;
  if (cursor + pageCount * 4 > buffer.length) return [];
  const pageSizes = [];
  for (let i = 0; i < pageCount; i += 1) {
    pageSizes.push(buffer.readUInt32BE(cursor));
    cursor += 4;
  }
  const cookies = [];
  for (const pageSize of pageSizes) {
    // Pushed one by one: a page can hold more cookies than a spread call takes arguments.
    for (const cookie of decodeSafariPage(buffer.subarray(cursor, cursor + pageSize))) cookies.push(cookie);
    cursor += pageSize;
  }
  return cookies;
}

/**
 * Reads Safari's cookies from its binary cookie file, which macOS only opens to apps with Full Disk Access.
 * @param {{label: string, cookiesPath: string}} browser
 * @param {{now?: number, readBytes?: (path: string) => Buffer}} [options]
 * @returns {Promise<{ok: true, cookies: import("./validate.js").ImportCookie[], skipped: Record<string, number>, total: number} | {ok: false, reason: string, message: string}>}
 */
export async function readSafariCookies(browser, options = {}) {
  const named = { browser: browser.label };
  let data;
  try {
    data = (options.readBytes ?? readFileSync)(browser.cookiesPath);
  } catch (error) {
    if (isMacPrivacyDenial(error)) return failure("fullDiskAccess", named);
    return failure(error?.code === "ENOENT" ? "databaseNotFound" : "readFailed", named);
  }

  let decoded;
  try {
    decoded = decodeSafariBinaryCookies(data);
  } catch {
    return failure("readFailed", named);
  }
  if (decoded.length === 0) return failure("noCookies", named);

  const nowSeconds = Math.floor((options.now ?? Date.now()) / 1000);
  const cookies = [];
  for (const cookie of decoded) {
    if (!cookie.expirationDate || cookie.expirationDate > nowSeconds) cookies.push({ ...cookie, value: stripNonPrintable(cookie.value) });
  }
  if (cookies.length === 0) return failure("allCookiesExpired", named);
  const expired = decoded.length - cookies.length;
  return { ok: true, cookies, skipped: expired > 0 ? { expired } : {}, total: decoded.length };
}
