// Ported from Orca (https://github.com/stablyai/orca, commit 49498c3), src/main/browser/cookies/imports/import-validated-cookies.ts
// and the write plan of browser-cookie-import-write.ts.
// MIT licence, Copyright (c) 2026 Lovecast Inc. Translated from TypeScript and adapted to cockpit's browser shell.

import { isGoogleSourceBoundCookie, isNonTransplantableCookieDomain, normalizeCookieImportDomain, registrableFamily } from "./policy.js";
import { deriveUrl, isExpired } from "./validate.js";

const SAME_SITE = new Set(["unspecified", "no_restriction", "lax", "strict"]);

function wellFormed(cookie) {
  if (typeof cookie !== "object" || cookie === null) return false;
  if (typeof cookie.name !== "string" || cookie.name.length === 0 || typeof cookie.value !== "string") return false;
  if (typeof cookie.domain !== "string" || typeof cookie.path !== "string" || !SAME_SITE.has(cookie.sameSite)) return false;
  if (cookie.expirationDate !== undefined && typeof cookie.expirationDate !== "number") return false;
  const partition = cookie.partition;
  if (partition?.status === "unpartitioned" || partition?.status === "unreadable") return true;
  // A cookie must say which partition it belongs to: guessing "none" would write it to the wrong jar.
  return partition?.status === "partitioned" && typeof partition.partitionKey?.topLevelSite === "string" && typeof partition.partitionKey?.hasCrossSiteAncestor === "boolean";
}

/** Why policy leaves a cookie out of the import, or null when it goes in. */
function exclusionReason(cookie, domainValidity, nowSeconds) {
  if (!wellFormed(cookie)) return "invalid";
  let validDomain = domainValidity.get(cookie.domain);
  if (validDomain === undefined) {
    validDomain = normalizeCookieImportDomain(cookie.domain) !== null && deriveUrl(cookie.domain, cookie.secure === true) !== null;
    domainValidity.set(cookie.domain, validDomain);
  }
  if (!validDomain) return "invalidDomain";
  if (isGoogleSourceBoundCookie(cookie.name, cookie.domain)) return "googleSourceBound";
  if (isNonTransplantableCookieDomain(cookie.domain)) return "nonTransplantableDomain";
  if (cookie.partition.status === "unreadable") return "partitionUnreadable";
  // Chromium reads an already expired cookie as a request to delete the one in place.
  return isExpired(cookie, nowSeconds) ? "expired" : null;
}

/** A cookie's domain as a summary shows it, without the dot that marks a domain cookie. */
export function summaryDomain(domain) {
  return domain.startsWith(".") ? domain.slice(1) : domain;
}

/**
 * Sorts cookies into those an import writes and those policy leaves out, and names the families whose existing
 * cookies stay untouched because one of their cookies has a partition the source could not describe.
 * @param {readonly unknown[]} cookies
 * @param {{now?: number}} [options]
 * @returns {{kept: import("./validate.js").ImportCookie[], excluded: Record<string, number>, domains: string[], preserveFamilies: Set<string>}}
 */
export function classifyImport(cookies, options = {}) {
  const nowSeconds = (options.now ?? Date.now()) / 1000;
  const kept = [];
  const excluded = {};
  const domains = new Set();
  const preserveFamilies = new Set();
  const domainValidity = new Map();
  for (const cookie of Array.isArray(cookies) ? cookies : []) {
    const reason = exclusionReason(cookie, domainValidity, nowSeconds);
    if (!reason) {
      kept.push(cookie);
      domains.add(summaryDomain(cookie.domain));
      continue;
    }
    excluded[reason] = (excluded[reason] ?? 0) + 1;
    if (reason === "partitionUnreadable") {
      const family = registrableFamily(cookie.domain);
      if (family) preserveFamilies.add(family);
    }
  }
  return { kept, excluded, domains: [...domains].sort(), preserveFamilies };
}

/**
 * What an import of these cookies writes once policy has had its say.
 * @param {readonly unknown[]} cookies
 * @param {{now?: number}} [options]
 * @returns {{kept: import("./validate.js").ImportCookie[], excluded: Record<string, number>, domains: string[]}}
 */
export function planImport(cookies, options) {
  const { kept, excluded, domains } = classifyImport(cookies, options);
  return { kept, excluded, domains };
}
