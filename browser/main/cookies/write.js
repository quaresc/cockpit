// Ported from Orca (https://github.com/stablyai/orca, commit 49498c3), src/main/browser/cookies/browser-cookie-import-write.ts,
// imports/import-validated-cookies.ts and imports/cookie-import-contracts.ts.
// MIT licence, Copyright (c) 2026 Lovecast Inc. Translated from TypeScript and adapted to cockpit's browser shell.

import { boundedCommandSender, writeIdentityWithCdp } from "./cdp.js";
import { withCookieMutationLock } from "./lock.js";
import { classifyImport, summaryDomain } from "./plan.js";
import { importedDomainScope } from "./policy.js";
import { removeCookiesInScope } from "./replace.js";
import { deriveUrl } from "./validate.js";

const HOST_PREFIX = "__Host-";
const LOGGED_REJECTIONS = 5;

/**
 * The exact place an imported cookie is written to. A cookie the source held for one host only is written without
 * a domain, so the import never widens it to the subdomains.
 * @param {import("./validate.js").ImportCookie} cookie
 * @returns {import("./cdp.js").CookieIdentity}
 */
export function importedCookieIdentity(cookie) {
  const isHostPrefixed = cookie.name.startsWith(HOST_PREFIX);
  return {
    url: deriveUrl(cookie.domain, cookie.secure),
    name: cookie.name,
    value: cookie.value,
    domain: cookie.domain,
    hostOnly: isHostPrefixed || !cookie.domain.startsWith("."),
    path: isHostPrefixed ? "/" : cookie.path,
    secure: cookie.secure,
    httpOnly: cookie.httpOnly,
    sameSite: cookie.sameSite,
    ...(cookie.expirationDate === undefined ? {} : { expirationDate: cookie.expirationDate }),
    ...(cookie.partition.status === "partitioned" ? { partitionKey: cookie.partition.partitionKey } : {}),
  };
}

function firstNonPrintable(value) {
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index);
    if (code < 0x20 || code > 0x7e) return `pos=${index} char=U+${code.toString(16).padStart(4, "0")}`;
  }
  return "none found";
}

/** Writes each cookie through CDP; one the store refuses is counted and the rest still go in. */
async function writeImportedCookies(send, cookies, log) {
  const phase = { imported: 0, failed: 0, domains: new Set() };
  for (const cookie of cookies) {
    try {
      await writeIdentityWithCdp(send, importedCookieIdentity(cookie), "import");
      phase.imported += 1;
      phase.domains.add(summaryDomain(cookie.domain));
    } catch (error) {
      phase.failed += 1;
      if (phase.failed <= LOGGED_REJECTIONS) {
        log(`cookie write rejected: domain=${summaryDomain(cookie.domain)} valLen=${cookie.value.length} badChar=${firstNonPrintable(cookie.value)} err=${String(error)}`);
      }
    }
  }
  return phase;
}

/**
 * Imports cookies into one Electron session: applies the policy, then, holding that session's mutation lock,
 * removes the existing cookies of the imported domains and writes the new ones through CDP `Network.setCookie`,
 * which carries partition keys that `cookies.set` drops. The store is flushed to disk once the lock is released.
 * @param {readonly unknown[]} cookies
 * @param {object} target
 * @param {{cookies: {get: (filter: object) => Promise<any[]>, remove: (url: string, name: string) => Promise<void>, flushStore: () => Promise<void>}}} target.session The destination session; it is also the lock's key.
 * @param {(method: string, params?: Record<string, unknown>) => Promise<unknown>} target.sendCommand `debugger.sendCommand` of a page that lives in that session.
 * @param {(message: string) => void} [target.log]
 * @param {number} [target.commandTimeoutMs] How long one CDP command may take; 10 s by default.
 * @param {() => void} [target.onCommandTimeout] Runs the moment a command times out, to detach or destroy the debugger's page.
 * @param {number} [target.now]
 * @returns {Promise<{imported: number, failed: number, domains: string[], replaced: number, excluded: Record<string, number>, error?: string}>}
 */
export async function writeCookies(cookies, target) {
  const { session, sendCommand, log = () => {} } = target;
  const planned = classifyImport(cookies, { now: target.now });
  const result = { imported: 0, failed: 0, domains: [], replaced: 0, excluded: planned.excluded };
  if (planned.kept.length === 0) return result;

  const outcome = await withCookieMutationLock(session, async () => {
    const send = boundedCommandSender(sendCommand, { timeoutMs: target.commandTimeoutMs, onTimeout: target.onCommandTimeout });
    try {
      const scope = importedDomainScope(planned.kept.map((cookie) => cookie.domain));
      result.replaced = await removeCookiesInScope(session.cookies, send, planned.preserveFamilies, scope);
    } catch (error) {
      return { error };
    }
    return writeImportedCookies(send, planned.kept, log);
  });

  if (outcome.error) {
    log(`cookie import stopped before writing: ${String(outcome.error)}`);
    return { ...result, failed: planned.kept.length, error: outcome.error instanceof Error ? outcome.error.message : String(outcome.error) };
  }
  if (outcome.imported > 0) {
    try {
      await session.cookies.flushStore();
    } catch (error) {
      log(`cookie flush failed: ${String(error)}`);
    }
  }
  return { ...result, imported: outcome.imported, failed: outcome.failed, domains: [...outcome.domains].sort() };
}
