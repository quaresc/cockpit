// Ported from Orca (https://github.com/stablyai/orca, commit 49498c3), src/main/browser/cookies/browser-cookie-import-clear.ts
// and storage/cookie-removal-url.ts.
// MIT licence, Copyright (c) 2026 Lovecast Inc. Translated from TypeScript and adapted to cockpit's browser shell.

import { cdpCookiesFromCommand, cookieIdentitiesFromCdp, writeIdentityWithCdp } from "./cdp.js";
import { domainIsInImportedScope, isNonTransplantableCookieDomain, normalizeCookieDomain, registrableFamily } from "./policy.js";

const COOKIE_CLEAR_CONCURRENCY = 8;

function cookieClearKey(url, name) {
  return JSON.stringify([url, name]);
}

/** The URL `cookies.remove` takes for a cookie, from its normalized domain, Secure flag and path. */
function cookieRemovalUrl(cookie, normalizedDomain) {
  const url = new URL(`${cookie.secure ? "https" : "http"}://${normalizedDomain}/`);
  url.pathname = cookie.path?.startsWith("/") ? cookie.path : "/";
  return url.toString();
}

/** The existing cookies an import replaces: those in its scope, outside the protected sites and the preserved families. */
function removableCookieEntries(cookies, preserveFamilies, importScope) {
  const removable = [];
  for (const cookie of cookies) {
    if (isNonTransplantableCookieDomain(cookie.domain ?? "")) continue;
    const scopedDomain = cookie.domain ? normalizeCookieDomain(cookie.domain) : null;
    if (scopedDomain === null || !domainIsInImportedScope(importScope, scopedDomain, cookie.hostOnly === true)) continue;
    if (preserveFamilies.size > 0) {
      const family = registrableFamily(cookie.domain ?? "");
      if (family !== null && preserveFamilies.has(family)) continue;
    }
    removable.push({ cookie, url: cookieRemovalUrl(cookie, scopedDomain) });
  }
  return removable;
}

function assertIdentitiesCoverRemovable(removable, identities) {
  const covered = new Set(identities.map((identity) => cookieClearKey(identity.url, identity.name)));
  for (const item of removable) {
    if (!covered.has(cookieClearKey(item.url, item.cookie.name))) throw new Error("Could not clear existing cookies; the session was left unchanged");
  }
}

function groupRemovableCookies(removable) {
  const groups = new Map();
  for (const item of removable) {
    const key = cookieClearKey(item.url, item.cookie.name);
    const group = groups.get(key) ?? [];
    group.push(item);
    groups.set(key, group);
  }
  return [...groups.values()];
}

/** Runs `run` over the items, a few at a time, and returns each one's failure or null. */
async function settleWithConcurrency(items, limit, run) {
  const failures = new Array(items.length).fill(null);
  let nextIndex = 0;
  const worker = async () => {
    while (nextIndex < items.length) {
      const index = nextIndex;
      nextIndex += 1;
      try {
        await run(items[index]);
      } catch (error) {
        failures[index] = error ?? new Error("Cookie removal failed");
      }
    }
  };
  await Promise.all(Array.from({ length: Math.min(limit, items.length) }, worker));
  return failures.filter((failure) => failure !== null);
}

/** Writes the removed cookies back, last removed first, then throws to say how the clear ended. */
async function restoreClearedCookies(send, identities, failures) {
  const restoreFailures = [];
  for (const identity of identities.toReversed()) {
    try {
      await writeIdentityWithCdp(send, identity, "restore");
    } catch (error) {
      restoreFailures.push(error);
    }
  }
  if (restoreFailures.length > 0) {
    throw new AggregateError([...failures, ...restoreFailures], "Could not clear existing cookies; the session was left partially cleared");
  }
  throw new AggregateError(failures, "Could not clear existing cookies; existing cookies were restored");
}

/**
 * Removes the destination's existing cookies that fall in the import's scope, so the import replaces a site's
 * session instead of mixing with it. A removal that fails puts back every cookie already removed, then throws.
 * The caller holds the session's mutation lock.
 * @param {{get: (filter: object) => Promise<any[]>, remove: (url: string, name: string) => Promise<void>}} store The destination session's `cookies`.
 * @param {(method: string, params?: Record<string, unknown>) => Promise<unknown>} send
 * @param {ReadonlySet<string>} preserveFamilies
 * @param {{exact: Set<string>, ancestors: Set<string>, descendantRoots: Set<string>}} importScope
 * @returns {Promise<number>} How many existing cookies were removed.
 */
export async function removeCookiesInScope(store, send, preserveFamilies, importScope) {
  if (importScope.exact.size === 0) return 0;
  const initialCookies = await store.get({});
  if (initialCookies.length === 0) return 0;
  const removable = removableCookieEntries(initialCookies, preserveFamilies, importScope);
  if (removable.length === 0) return 0;

  // cookies.get() leaves out partition keys, so the identities to restore come from a CDP listing.
  const identities = cookieIdentitiesFromCdp(removable, cdpCookiesFromCommand(await send("Network.getAllCookies")));
  assertIdentitiesCoverRemovable(removable, identities);

  // Removal goes cookie by cookie: a bulk clear matches whole registrable domains and would take host-only siblings.
  const failures = await settleWithConcurrency(groupRemovableCookies(removable), COOKIE_CLEAR_CONCURRENCY, async (group) => {
    for (const { cookie, url } of group) await store.remove(url, cookie.name);
  });
  if (failures.length > 0) await restoreClearedCookies(send, identities, failures);
  return removable.length;
}
