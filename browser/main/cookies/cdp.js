// Ported from Orca (https://github.com/stablyai/orca, commit 49498c3), src/main/browser/cookies/browser-cdp-cookie.ts,
// browser-cookie-clear-store.ts, storage/cookie-store-contracts.ts and the command deadline of browser-cookie-debugger-session.ts.
// MIT licence, Copyright (c) 2026 Lovecast Inc. Translated from TypeScript and adapted to cockpit's browser shell.

import { normalizeCookieDomain } from "./policy.js";
import { normalizeCookiePartitionSite } from "./partition.js";

const COOKIE_COMMAND_DEADLINE_MS = 10_000;

/**
 * One cookie at its exact place in a jar, partition included: what `Network.setCookie` needs to write it back.
 * @typedef {object} CookieIdentity
 * @property {string} url
 * @property {string} name
 * @property {string} value
 * @property {string} [domain]
 * @property {boolean} [hostOnly]
 * @property {string} [path]
 * @property {boolean} [secure]
 * @property {boolean} [httpOnly]
 * @property {"unspecified" | "no_restriction" | "lax" | "strict"} sameSite
 * @property {number} [expirationDate]
 * @property {{topLevelSite: string, hasCrossSiteAncestor: boolean}} [partitionKey]
 */

/**
 * Wraps a debugger's `sendCommand` so no command waits past the deadline. The first one that does retires the
 * sender: `onTimeout` runs at once and every later command fails without being sent.
 * @param {(method: string, params?: Record<string, unknown>) => Promise<unknown>} sendCommand
 * @param {{timeoutMs?: number, onTimeout?: () => void}} [options]
 * @returns {(method: string, params?: Record<string, unknown>) => Promise<unknown>}
 */
export function boundedCommandSender(sendCommand, options = {}) {
  const timeoutMs = options.timeoutMs ?? COOKIE_COMMAND_DEADLINE_MS;
  let retired = false;
  return async (method, params) => {
    if (retired) throw new Error("Cookie session was retired after a command timed out");
    let timer;
    try {
      return await Promise.race([
        Promise.resolve().then(() => sendCommand(method, params)),
        new Promise((_resolve, reject) => {
          timer = setTimeout(() => {
            // Retiring before the rejection lands keeps a hung command from outliving the mutation lock.
            retired = true;
            try {
              options.onTimeout?.();
            } catch {}
            reject(new Error("Cookie session command timed out"));
          }, timeoutMs);
        }),
      ]);
    } finally {
      clearTimeout(timer);
    }
  };
}

/** The `cookies` array of a `Network.getAllCookies` reply, which carries each cookie's partition key. */
export function cdpCookiesFromCommand(value) {
  if (typeof value !== "object" || value === null || !("cookies" in value)) return [];
  return Array.isArray(value.cookies) ? value.cookies : [];
}

function cdpSameSite(sameSite) {
  if (sameSite === "strict") return "Strict";
  if (sameSite === "no_restriction") return "None";
  return sameSite === "lax" ? "Lax" : undefined;
}

function electronSameSite(sameSite) {
  if (sameSite === "Strict") return "strict";
  if (sameSite === "None") return "no_restriction";
  return sameSite === "Lax" ? "lax" : "unspecified";
}

function partitionKeyFromCdp(cookie) {
  const opaque = cookie.partitionKeyOpaque;
  if (opaque === true || (opaque !== undefined && typeof opaque !== "boolean")) throw new Error("Could not snapshot cookie identity for an atomic clear");
  const partitionKey = cookie.partitionKey;
  if (partitionKey === undefined) return undefined;
  const topLevelSite = normalizeCookiePartitionSite(partitionKey?.topLevelSite ?? "");
  if (!topLevelSite || typeof partitionKey?.hasCrossSiteAncestor !== "boolean") throw new Error("Could not snapshot cookie identity for an atomic clear");
  return { topLevelSite, hasCrossSiteAncestor: partitionKey.hasCrossSiteAncestor };
}

function cookieScopeKey(name, domain, path, hostOnly) {
  const normalizedDomain = domain ? normalizeCookieDomain(domain) : null;
  return normalizedDomain ? JSON.stringify([name, normalizedDomain, path || "/", hostOnly]) : null;
}

function indexCdpCookies(cookies) {
  const index = new Map();
  for (const cookie of cookies) {
    const key = cookieScopeKey(cookie.name, cookie.domain, cookie.path, !cookie.domain?.startsWith("."));
    if (!key) continue;
    const matches = index.get(key) ?? [];
    matches.push(cookie);
    index.set(key, matches);
  }
  return index;
}

function identityFromCdpCookie(url, cdpCookie) {
  const partitionKey = partitionKeyFromCdp(cdpCookie);
  return {
    url,
    name: cdpCookie.name,
    value: cdpCookie.value,
    domain: cdpCookie.domain,
    hostOnly: !cdpCookie.domain?.startsWith("."),
    path: cdpCookie.path,
    secure: cdpCookie.secure,
    httpOnly: cdpCookie.httpOnly,
    sameSite: electronSameSite(cdpCookie.sameSite),
    ...(cdpCookie.session === true || cdpCookie.expires == null ? {} : { expirationDate: cdpCookie.expires }),
    ...(partitionKey ? { partitionKey } : {}),
  };
}

/**
 * The full identity of each cookie about to be removed, every partition's copy included, matched in a CDP listing.
 * It throws when a cookie has no match or an opaque partition, since removing it could not be undone.
 * @param {ReadonlyArray<{cookie: {name: string, domain?: string, path?: string, hostOnly?: boolean}, url: string}>} cookies
 * @param {ReadonlyArray<Record<string, any>>} cdpCookies
 * @returns {CookieIdentity[]}
 */
export function cookieIdentitiesFromCdp(cookies, cdpCookies) {
  const identities = [];
  const seen = new Set();
  const cdpCookieIndex = indexCdpCookies(cdpCookies);
  for (const item of cookies) {
    const hostOnly = item.cookie.hostOnly ?? !item.cookie.domain?.startsWith(".");
    const scopeKey = cookieScopeKey(item.cookie.name, item.cookie.domain, item.cookie.path, hostOnly);
    const matches = scopeKey ? (cdpCookieIndex.get(scopeKey) ?? []) : [];
    if (matches.length === 0) throw new Error("Could not snapshot cookie identity for an atomic clear");
    for (const match of matches) {
      const key = JSON.stringify([item.url, match.name, match.domain, match.path, partitionKeyFromCdp(match) ?? null]);
      if (seen.has(key)) continue;
      seen.add(key);
      identities.push(identityFromCdpCookie(item.url, match));
    }
  }
  return identities;
}

/** The `Network.setCookie` parameters that write an identity; a host-only cookie goes without a domain. */
export function cdpSetCookieParamsFromIdentity(identity) {
  const sameSite = cdpSameSite(identity.sameSite);
  return {
    url: identity.url,
    name: identity.name,
    value: identity.value,
    ...(identity.hostOnly ? {} : { domain: identity.domain }),
    ...(identity.path ? { path: identity.path } : {}),
    secure: identity.secure,
    httpOnly: identity.httpOnly,
    ...(sameSite ? { sameSite } : {}),
    ...(identity.expirationDate ? { expires: identity.expirationDate } : {}),
    ...(identity.partitionKey ? { partitionKey: identity.partitionKey } : {}),
  };
}

/**
 * Writes one identity through `Network.setCookie`, and throws when the reply says the cookie was refused.
 * @param {(method: string, params?: Record<string, unknown>) => Promise<unknown>} send
 * @param {CookieIdentity} identity
 * @param {string} failureLabel The verb for the error, such as "import" or "restore".
 */
export async function writeIdentityWithCdp(send, identity, failureLabel) {
  const result = await send("Network.setCookie", cdpSetCookieParamsFromIdentity(identity));
  if (typeof result === "object" && result !== null && result.success === false) throw new Error(`Could not ${failureLabel} cookie ${identity.name}`);
}
