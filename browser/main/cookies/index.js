// Cookie import for a browser profile: find the browsers on this Mac, read one's cookies or a cookie file, apply the
// policy, and write the result into an Electron session. The readers, the policy and the writer are ported from Orca
// (https://github.com/stablyai/orca, commit 49498c3), src/main/browser/cookies/. MIT licence, Copyright (c) 2026 Lovecast Inc.
//
// The contract. Nothing here imports `electron`; nothing throws for a failure a person can cause.
//
//   sources({home?, exists?, readText?, listDirs?})
//     -> [{id, family, label, profiles: [{name, directory, path}], needsKey, keychain?: {service, account}}]
//     id       "chrome" | "edge" | "arc" | "brave" | "comet" | "helium" | "vivaldi" | "firefox" | "safari"
//     family   "chromium" | "firefox" | "safari": which reader takes it
//     profiles only those that hold a cookie store; `path` is that file, `directory` the profile's folder name
//     needsKey true when reading may ask the Keychain for `keychain.service` / `keychain.account`
//     home is the home folder to look in; exists(path) -> boolean, readText(path) -> string and
//     listDirs(path) -> string[] replace the file-system reads (all default to the real ones).
//
//   read(source, {profile?, getKey?, tmpDir?, now?})
//     -> Promise<{ok: true, cookies: Cookie[], skipped: {[reason]: count}, total: number}
//              | {ok: false, reason: string, message: string}>
//     source   one entry of sources(); profile is a profile's `directory` or `name`, or the profile object
//              (the first profile when omitted)
//     getKey   (service, account) -> Promise<string | null>: the Safe Storage password; `keychainPassword` by default
//     tmpDir   where the database snapshot is made and removed (os.tmpdir() by default); now is epoch milliseconds
//     skipped  "invalid" | "expired" | "undecryptable" | "googleSourceBound" | "nonTransplantableDomain"
//              a Chromium row that policy leaves behind is never decrypted, so read counts it here under the name
//              plan uses in `excluded`: add the two for the whole count
//     reason   "invalidProfile" | "databaseNotFound" | "databaseCopyFailed" | "fullDiskAccess" | "keyDenied"
//              | "noCookies" | "noValidCookies" | "allCookiesExpired" | "readFailed"
//
//   readFile(path)
//     -> Promise<{ok: true, cookies: Cookie[], skipped: {invalid?: count}, total: number}
//              | {ok: false, reason: string, message: string}>
//     a JSON array of {domain, name, value, path?, secure?, httpOnly?, sameSite?, expirationDate?, partitionKey?,
//     partitionKeyOpaque?}
//     reason   "fileUnreadable" | "fileInvalidJson" | "fileNotArray" | "fileEmpty" | "fileNoValidCookies"
//
//   plan(cookies, {now?})
//     -> {kept: Cookie[], excluded: {[reason]: count}, domains: string[]}
//     excluded "invalid" | "invalidDomain" | "googleSourceBound" | "nonTransplantableDomain" | "partitionUnreadable"
//              | "expired"; domains are the kept cookies' hosts, sorted, without a leading dot
//
//   write(cookies, {session, sendCommand, log?, commandTimeoutMs?, onCommandTimeout?, now?})
//     -> Promise<{imported, failed, domains: string[], replaced, excluded: {[reason]: count}, error?: string}>
//     session          {cookies: {get(filter), remove(url, name), flushStore()}}: the destination Electron session,
//                      also the key of the mutation lock, so pass the same object for every import into a profile
//     sendCommand      (method, params) -> Promise: `debugger.sendCommand` of a page that lives in that session
//     onCommandTimeout runs the moment a CDP command passes commandTimeoutMs (10 s by default)
//     write runs plan itself, so passing read's cookies straight in is safe; `replaced` counts the existing cookies
//     removed, and `error` is set when the import stopped before writing anything.
//
//   keychainPassword(service, account) -> Promise<string | null>   the default getKey, through /usr/bin/security
//
//   Cookie: {name, value, domain, path, secure, httpOnly, sameSite, expirationDate?, partition}
//     domain         as the source stores it: ".acme.example" is a domain cookie, "acme.example" a host-only one
//     sameSite       "unspecified" | "no_restriction" | "lax" | "strict"
//     expirationDate Unix seconds; absent for a session cookie
//     partition      {status: "unpartitioned"}
//                  | {status: "partitioned", partitionKey: {topLevelSite, hasCrossSiteAncestor}}
//                  | {status: "unreadable", reason}   kept in the list so plan and write know its site, never written

import { readChromiumCookies } from "./chromium.js";
import { detectSources, selectProfile } from "./detect.js";
import { failure } from "./failure.js";
import { readCookieFile } from "./file.js";
import { readFirefoxCookies } from "./firefox.js";
import { keychainPassword } from "./key.js";
import { planImport } from "./plan.js";
import { readSafariCookies } from "./safari.js";
import { writeCookies } from "./write.js";

export { keychainPassword };

/** The browsers on this Mac that cookies can be imported from. */
export const sources = detectSources;

/** Reads the cookies of one profile of a detected browser. */
export async function read(source, options = {}) {
  const label = typeof source?.label === "string" ? source.label : "The browser";
  const profile = selectProfile(source, options.profile);
  if (!profile || typeof profile.path !== "string") return failure("invalidProfile", { browser: label });
  const browser = { label, cookiesPath: profile.path, keychain: source.keychain };
  try {
    if (source.family === "firefox") return await readFirefoxCookies(browser, options);
    if (source.family === "safari") return await readSafariCookies(browser, options);
    if (source.family !== "chromium" || !source.keychain) return failure("invalidProfile", { browser: label });
    return await readChromiumCookies(browser, { ...options, getKey: options.getKey ?? keychainPassword });
  } catch {
    return failure("readFailed", { browser: label });
  }
}

/** Reads a cookie file. */
export const readFile = readCookieFile;

/** What an import of these cookies writes, and what policy leaves out. */
export const plan = planImport;

/** Imports cookies into an Electron session. */
export const write = writeCookies;
