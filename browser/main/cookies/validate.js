// Ported from Orca (https://github.com/stablyai/orca, commit 49498c3), src/main/browser/cookies/sources/browser-cookie-validation.ts.
// MIT licence, Copyright (c) 2026 Lovecast Inc. Translated from TypeScript and adapted to cockpit's browser shell.

import { normalizeCookieDomain } from "./policy.js";
import { readJsonCookiePartition } from "./partition.js";

/**
 * A cookie as every reader returns it and the writer takes it.
 * @typedef {object} ImportCookie
 * @property {string} name
 * @property {string} value
 * @property {string} domain As the source stores it: a leading dot marks a domain cookie, none a host-only one.
 * @property {string} path
 * @property {boolean} secure
 * @property {boolean} httpOnly
 * @property {"unspecified" | "no_restriction" | "lax" | "strict"} sameSite
 * @property {number} [expirationDate] Unix seconds; absent for a session cookie.
 * @property {import("./partition.js").SourcePartition} partition
 */

/** SameSite from a database column: 0 None, 1 Lax, 2 Strict, anything else unspecified. */
export function databaseSameSite(raw) {
  if (raw === 0) return "no_restriction";
  if (raw === 1) return "lax";
  if (raw === 2) return "strict";
  return "unspecified";
}

/** SameSite from a cookie-file entry, as a number or as a word in any case. */
export function normalizeSameSite(raw) {
  if (typeof raw === "number") return databaseSameSite(raw);
  if (typeof raw !== "string") return "unspecified";
  const lower = raw.toLowerCase();
  if (lower === "lax") return "lax";
  if (lower === "strict") return "strict";
  if (lower === "none" || lower === "no_restriction") return "no_restriction";
  return "unspecified";
}

/** The URL a cookie is set through, from its domain and Secure flag, or null for a domain that is not a plain host. */
export function deriveUrl(domain, secure) {
  const normalizedDomain = normalizeCookieDomain(domain);
  if (!normalizedDomain) return null;
  try {
    return new URL(`${secure ? "https" : "http"}://${normalizedDomain}/`).toString();
  } catch {
    return null;
  }
}

/** Cookie values written from text sources keep printable ASCII only, which is all the cookie store accepts. */
export function stripNonPrintable(value) {
  return value.replace(/[^\x20-\x7E]/g, "");
}

/** Whether a cookie's expiry is at or before `nowSeconds`; a session cookie never is. */
export function isExpired(cookie, nowSeconds = Date.now() / 1000) {
  return cookie.expirationDate !== undefined && cookie.expirationDate <= nowSeconds;
}

/**
 * One cookie-file entry as a cookie, or null when a required field is missing or malformed.
 * @param {Record<string, unknown>} raw
 * @returns {ImportCookie | null}
 */
export function validateCookieEntry(raw) {
  if (typeof raw.domain !== "string" || raw.domain.trim().length === 0) return null;
  if (typeof raw.name !== "string" || raw.name.trim().length === 0) return null;
  if (typeof raw.value !== "string") return null;
  const domain = raw.domain.trim();
  const secure = raw.secure === true || raw.secure === 1;
  if (!deriveUrl(domain, secure)) return null;
  const expirationDate = typeof raw.expirationDate === "number" && raw.expirationDate > 0 ? raw.expirationDate : undefined;
  return {
    name: raw.name.trim(),
    value: raw.value,
    domain,
    path: typeof raw.path === "string" ? raw.path : "/",
    secure,
    httpOnly: raw.httpOnly === true || raw.httpOnly === 1,
    sameSite: normalizeSameSite(raw.sameSite),
    ...(expirationDate === undefined ? {} : { expirationDate }),
    partition: readJsonCookiePartition(raw.partitionKey, raw.partitionKeyOpaque),
  };
}
