// The cookie import's policy: which domains it accepts, which cookies stay behind, and which existing cookies an
// import replaces.

import assert from "node:assert/strict";
import { test } from "node:test";
import { plan } from "../main/cookies/index.js";
import { failure, isMacPrivacyDenial } from "../main/cookies/failure.js";
import { readJsonCookiePartition, normalizeCookiePartitionSite } from "../main/cookies/partition.js";
import {
  domainIsInImportedScope,
  importedDomainScope,
  isGoogleSourceBoundCookie,
  isNonTransplantableCookieDomain,
  normalizeCookieDomain,
  normalizeCookieImportDomain,
  registrableDomain,
  registrableFamily,
} from "../main/cookies/policy.js";
import { databaseSameSite, deriveUrl, normalizeSameSite, validateCookieEntry } from "../main/cookies/validate.js";

const NOW = Date.UTC(2026, 0, 15);
const LATER = NOW / 1000 + 3600;

const cookie = (fields = {}) => ({
  name: "session",
  value: "v",
  domain: ".acme.example",
  path: "/",
  secure: true,
  httpOnly: true,
  sameSite: "lax",
  partition: { status: "unpartitioned" },
  ...fields,
});

test("a cookie domain normalizes to a lower-case host without its leading dot", () => {
  assert.equal(normalizeCookieDomain(".Acme.Example"), "acme.example");
  assert.equal(normalizeCookieDomain("  api.acme.example "), "api.acme.example");
  assert.equal(normalizeCookieDomain("localhost"), "localhost");
  assert.equal(normalizeCookieDomain("127.0.0.1"), "127.0.0.1");
  assert.equal(normalizeCookieDomain("[::1]"), "[::1]");
});

test("a cookie domain that is not a plain host is refused", () => {
  for (const bad of ["", ".", "acme.example/path", "acme.example:8443", "me@acme.example", "acme.example?x", "acme.example#x", "a%2e.example", "acme..example", "acme.example.", "a\\b", 42, null]) {
    assert.equal(normalizeCookieDomain(bad), null, String(bad));
  }
});

test("the registrable domain is the listed public suffix plus one label, else the last two labels", () => {
  assert.equal(registrableDomain("acme.example"), "acme.example");
  assert.equal(registrableDomain("api.eu.acme.example"), "acme.example");
  assert.equal(registrableDomain("shop.acme.co.uk"), "acme.co.uk");
  assert.equal(registrableDomain("acme.co.uk"), "acme.co.uk");
  assert.equal(registrableDomain("preview.acme.github.io"), "acme.github.io");
  assert.equal(registrableDomain("acme-web.a.run.app"), "acme-web.a.run.app");
  assert.equal(registrableDomain("co.uk"), null);
  assert.equal(registrableDomain("github.io"), null);
  assert.equal(registrableDomain("localhost"), "localhost");
  assert.equal(registrableDomain("10.0.0.7"), "10.0.0.7");
  assert.equal(registrableDomain("[::1]"), "[::1]");
  assert.equal(registrableFamily(".API.acme.example"), "acme.example");
  assert.equal(registrableFamily("not a host"), null);
});

test("an import refuses a cookie set on a listed public suffix", () => {
  assert.equal(normalizeCookieImportDomain(".co.uk"), null);
  assert.equal(normalizeCookieImportDomain("github.io"), null);
  assert.equal(normalizeCookieImportDomain(".acme.co.uk"), "acme.co.uk");
  assert.equal(normalizeCookieImportDomain("acme.github.io"), "acme.github.io");
  assert.equal(normalizeCookieImportDomain("localhost"), "localhost");
  assert.equal(normalizeCookieImportDomain("[::1]"), "[::1]");
});

test("Google's device-bound cookies stay behind, on google.com and its subdomains only", () => {
  for (const name of ["SIDCC", "__Secure-1PSIDCC", "__Secure-3PSIDCC", "__Secure-STRP", "AEC"]) {
    assert.equal(isGoogleSourceBoundCookie(name, ".google.com"), true, name);
    assert.equal(isGoogleSourceBoundCookie(name, "accounts.google.com"), true, name);
    assert.equal(isGoogleSourceBoundCookie(name, ".acme.example"), false, name);
  }
  assert.equal(isGoogleSourceBoundCookie("SID", ".google.com"), false);
  assert.equal(isNonTransplantableCookieDomain(".google.com"), true);
  assert.equal(isNonTransplantableCookieDomain("accounts.Google.com"), true);
  assert.equal(isNonTransplantableCookieDomain(".youtube.com"), false);
  assert.equal(isNonTransplantableCookieDomain("notgoogle.com"), false);
  assert.equal(isNonTransplantableCookieDomain("google.com.acme.example"), false);
  assert.equal(isNonTransplantableCookieDomain("not a host"), false);
});

test("an imported domain cookie puts its host, its parents and its subdomains in scope", () => {
  const scope = importedDomainScope([".api.acme.example"]);
  assert.deepEqual([...scope.exact], ["api.acme.example"]);
  assert.deepEqual([...scope.ancestors].sort(), ["acme.example", "api.acme.example"]);
  assert.deepEqual([...scope.descendantRoots], ["api.acme.example"]);

  assert.equal(domainIsInImportedScope(scope, "api.acme.example", true), true);
  assert.equal(domainIsInImportedScope(scope, "v2.api.acme.example", true), true);
  // A parent's domain cookie reaches the imported host; a parent's host-only cookie does not.
  assert.equal(domainIsInImportedScope(scope, "acme.example", false), true);
  assert.equal(domainIsInImportedScope(scope, "acme.example", true), false);
  assert.equal(domainIsInImportedScope(scope, "app.acme.example", true), false);
  assert.equal(domainIsInImportedScope(scope, "example", false), false);
  assert.equal(domainIsInImportedScope(scope, "other.example", false), false);
  assert.equal(domainIsInImportedScope(scope, "notacme.example", false), false);
});

test("an imported host-only cookie leaves the host's subdomains out of scope", () => {
  const scope = importedDomainScope(["acme.example"]);
  assert.deepEqual([...scope.exact], ["acme.example"]);
  assert.equal(scope.descendantRoots.size, 0);
  assert.equal(domainIsInImportedScope(scope, "acme.example", true), true);
  assert.equal(domainIsInImportedScope(scope, "app.acme.example", true), false);
});

test("a public suffix never becomes a scope, so one cookie cannot clear every site under it", () => {
  const scope = importedDomainScope([".co.uk", ".github.io", "github.io", ".acme.co.uk", "not a host"]);
  assert.deepEqual([...scope.exact], ["acme.co.uk"]);
  assert.deepEqual([...scope.ancestors], ["acme.co.uk"]);
  assert.deepEqual([...scope.descendantRoots], ["acme.co.uk"]);
  assert.equal(domainIsInImportedScope(scope, "other.co.uk", false), false);
  assert.equal(domainIsInImportedScope(scope, "co.uk", false), false);
});

test("a single-label or IP host is in scope for itself alone", () => {
  const scope = importedDomainScope([".localhost", "10.0.0.7"]);
  assert.deepEqual([...scope.exact].sort(), ["10.0.0.7", "localhost"]);
  assert.equal(scope.descendantRoots.size, 0);
  assert.equal(domainIsInImportedScope(scope, "app.localhost", true), false);
});

test("the scope stops at the registrable domain under a listed suffix", () => {
  const scope = importedDomainScope(["shop.acme.co.uk"]);
  assert.deepEqual([...scope.ancestors].sort(), ["acme.co.uk", "shop.acme.co.uk"]);
});

test("SameSite reads from a database number or a file's word", () => {
  assert.deepEqual([0, 1, 2, -1, 3, 256].map(databaseSameSite), ["no_restriction", "lax", "strict", "unspecified", "unspecified", "unspecified"]);
  assert.deepEqual(["Lax", "STRICT", "none", "no_restriction", "unspecified", "", undefined, 1].map(normalizeSameSite), [
    "lax",
    "strict",
    "no_restriction",
    "no_restriction",
    "unspecified",
    "unspecified",
    "unspecified",
    "lax",
  ]);
});

test("the URL a cookie is set through follows its Secure flag", () => {
  assert.equal(deriveUrl(".acme.example", true), "https://acme.example/");
  assert.equal(deriveUrl("api.acme.example", false), "http://api.acme.example/");
  assert.equal(deriveUrl("acme.example/evil", true), null);
});

test("a cookie-file entry validates into a cookie", () => {
  assert.deepEqual(validateCookieEntry({ domain: " .acme.example ", name: " session ", value: "v", secure: 1, httpOnly: true, sameSite: "Strict", expirationDate: LATER }), {
    name: "session",
    value: "v",
    domain: ".acme.example",
    path: "/",
    secure: true,
    httpOnly: true,
    sameSite: "strict",
    expirationDate: LATER,
    partition: { status: "unpartitioned" },
  });
  const session = validateCookieEntry({ domain: "acme.example", name: "s", value: "", path: "/app", expirationDate: 0 });
  assert.equal(session.path, "/app");
  assert.equal(session.secure, false);
  assert.equal("expirationDate" in session, false);
});

test("a cookie-file entry with a missing or malformed field is refused", () => {
  assert.equal(validateCookieEntry({ name: "s", value: "v" }), null);
  assert.equal(validateCookieEntry({ domain: "  ", name: "s", value: "v" }), null);
  assert.equal(validateCookieEntry({ domain: "acme.example", name: "", value: "v" }), null);
  assert.equal(validateCookieEntry({ domain: "acme.example", name: "s" }), null);
  assert.equal(validateCookieEntry({ domain: "acme.example", name: "s", value: 7 }), null);
  assert.equal(validateCookieEntry({ domain: "acme.example/x", name: "s", value: "v" }), null);
});

test("a partition key reads only when it is complete", () => {
  assert.deepEqual(readJsonCookiePartition(undefined), { status: "unpartitioned" });
  assert.deepEqual(readJsonCookiePartition({ topLevelSite: "https://Acme.example", hasCrossSiteAncestor: false }), {
    status: "partitioned",
    partitionKey: { topLevelSite: "https://acme.example", hasCrossSiteAncestor: false },
  });
  for (const [raw, opaque] of [
    [undefined, true],
    [undefined, "yes"],
    ["https://acme.example", undefined],
    [null, undefined],
    [{ hasCrossSiteAncestor: true }, undefined],
    [{ topLevelSite: "https://acme.example" }, undefined],
    [{ topLevelSite: "https://acme.example/path", hasCrossSiteAncestor: true }, undefined],
    [{ topLevelSite: "https://acme.example:8443", hasCrossSiteAncestor: true }, undefined],
    [{ topLevelSite: "chrome-extension://abcdef", hasCrossSiteAncestor: true }, undefined],
  ]) {
    assert.equal(readJsonCookiePartition(raw, opaque).status, "unreadable", JSON.stringify([raw, opaque]));
  }
  assert.equal(normalizeCookiePartitionSite("http://acme.example"), "http://acme.example");
});

test("plan keeps what policy lets through and counts the rest by reason", () => {
  const cookies = [
    cookie(),
    cookie({ name: "token", domain: "api.acme.example" }),
    cookie({ name: "theme", domain: "api.acme.example", expirationDate: LATER }),
    cookie({ name: "SIDCC", domain: ".google.com" }),
    cookie({ name: "SID", domain: "accounts.google.com" }),
    cookie({ name: "PREF", domain: ".youtube.com" }),
    cookie({ name: "super", domain: ".co.uk" }),
    cookie({ name: "bad", domain: "acme.example/x" }),
    cookie({ name: "old", expirationDate: NOW / 1000 - 1 }),
    cookie({ name: "chips", partition: { status: "unreadable", reason: "partition key was opaque" } }),
    cookie({ name: "embed", partition: { status: "partitioned", partitionKey: { topLevelSite: "https://acme.example", hasCrossSiteAncestor: true } } }),
    cookie({ name: "nopartition", partition: undefined }),
    cookie({ name: "halfkey", partition: { status: "partitioned", partitionKey: { topLevelSite: "https://acme.example" } } }),
    cookie({ name: "" }),
    cookie({ value: 7 }),
    cookie({ sameSite: "Lax" }),
    null,
  ];
  const planned = plan(cookies, { now: NOW });
  assert.deepEqual(
    planned.kept.map((kept) => kept.name),
    ["session", "token", "theme", "PREF", "embed"],
  );
  assert.deepEqual(planned.excluded, { googleSourceBound: 1, nonTransplantableDomain: 1, invalidDomain: 2, expired: 1, partitionUnreadable: 1, invalid: 6 });
  assert.deepEqual(planned.domains, ["acme.example", "api.acme.example", "youtube.com"]);
  assert.deepEqual(Object.keys(planned).sort(), ["domains", "excluded", "kept"]);
});

test("plan of nothing is empty", () => {
  assert.deepEqual(plan([]), { kept: [], excluded: {}, domains: [] });
  assert.deepEqual(plan(undefined), { kept: [], excluded: {}, domains: [] });
});

test("a refusal carries its code and a sentence naming the browser", () => {
  assert.deepEqual(failure("noCookies", { browser: "Arc" }), { ok: false, reason: "noCookies", message: "No cookies found in Arc." });
  assert.match(failure("fullDiskAccess", { browser: "Safari" }).message, /Full Disk Access/);
  assert.match(failure("fileNoValidCookies", { count: 3 }).message, /^No valid cookies found\. 3 entries/);
});

test("only a permission error reads as macOS refusing access", () => {
  assert.equal(isMacPrivacyDenial(Object.assign(new Error("operation not permitted"), { code: "EPERM" })), true);
  assert.equal(isMacPrivacyDenial(Object.assign(new Error("no such file"), { code: "ENOENT" })), false);
  assert.equal(isMacPrivacyDenial("EPERM"), false);
});
