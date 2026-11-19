// Ported from Orca (https://github.com/stablyai/orca, commit 49498c3), src/main/browser/cookies/policy/cookie-domain.ts,
// policy/cookie-import-scope.ts and policy/cookie-import-exclusions.ts.
// MIT licence, Copyright (c) 2026 Lovecast Inc. Translated from TypeScript and adapted to cockpit's browser shell.
// A short built-in list of public suffixes stands in for the public-suffix library Orca links.

import { isIP } from "node:net";

/**
 * Common public suffixes of more than one label, under which unrelated people register their own names.
 * It is a short list, not the whole public-suffix list: an unlisted suffix is read as an ordinary domain.
 */
const COMMON_PUBLIC_SUFFIXES = new Set([
  // Country second levels.
  "co.uk", "org.uk", "ac.uk", "gov.uk", "me.uk", "ltd.uk", "plc.uk", "net.uk", "sch.uk", "nhs.uk",
  "com.au", "net.au", "org.au", "edu.au", "gov.au", "id.au", "co.nz", "net.nz", "org.nz", "govt.nz", "ac.nz",
  "co.jp", "ne.jp", "or.jp", "ac.jp", "go.jp", "ad.jp", "co.kr", "or.kr", "go.kr", "ac.kr",
  "com.cn", "net.cn", "org.cn", "gov.cn", "edu.cn", "com.hk", "org.hk", "edu.hk", "com.tw", "org.tw", "edu.tw",
  "com.sg", "edu.sg", "org.sg", "com.my", "com.ph", "com.vn", "co.th", "co.id", "or.id", "ac.id", "go.id",
  "co.in", "net.in", "org.in", "gen.in", "firm.in", "ac.in", "edu.in", "gov.in", "com.pk", "com.bd", "com.np",
  "com.br", "net.br", "org.br", "gov.br", "edu.br", "com.ar", "com.mx", "org.mx", "com.co", "com.pe", "com.ve",
  "com.uy", "com.ec", "com.bo", "com.py", "com.do", "com.gt", "co.cr", "com.pa",
  "co.za", "org.za", "net.za", "gov.za", "ac.za", "co.il", "org.il", "ac.il", "com.tr", "org.tr", "edu.tr", "gov.tr",
  "com.sa", "com.eg", "com.ng", "co.ke", "co.tz", "co.ug", "com.gh", "co.ma", "com.qa", "com.kw", "com.lb",
  "com.ua", "com.pl", "com.pt", "com.es", "com.gr", "com.cy", "com.mt", "com.ro", "co.at", "or.at", "co.hu",
  "com.fr", "asso.fr", "gouv.fr", "co.rs",
  // Hosting platforms that hand each customer a name of their own.
  "github.io", "githubusercontent.com", "gitlab.io", "vercel.app", "now.sh", "netlify.app", "web.app", "firebaseapp.com",
  "pages.dev", "workers.dev", "herokuapp.com", "appspot.com", "run.app", "a.run.app", "cloudfunctions.net",
  "blogspot.com", "cloudfront.net", "s3.amazonaws.com", "elasticbeanstalk.com", "amplifyapp.com",
  "azurewebsites.net", "azurestaticapps.net", "cloudapp.net", "fly.dev", "onrender.com", "up.railway.app",
  "ngrok.io", "ngrok.app", "ngrok.dev", "ngrok-free.app", "ngrok-free.dev", "trycloudflare.com",
  "myshopify.com", "glitch.me", "repl.co", "replit.app", "replit.dev", "surge.sh", "deno.dev",
  "wixsite.com", "webflow.io", "framer.app", "framer.website", "bubbleapps.io", "hf.space", "csb.app",
  "app.github.dev", "preview.app.github.dev",
]);

/** A cookie domain as a lower-case host without its leading dot, or null when it is not a plain host. */
export function normalizeCookieDomain(domain) {
  if (typeof domain !== "string") return null;
  const candidate = domain.trim().replace(/^\.+/, "");
  const isBracketedIpv6 = candidate.startsWith("[") && candidate.endsWith("]");
  if (!candidate || /[/\\@?#%]/.test(candidate) || (!isBracketedIpv6 && candidate.includes(":"))) return null;
  try {
    const parsed = new URL(`https://${candidate}/`);
    const normalized = parsed.hostname.toLowerCase();
    if (parsed.username || parsed.password || parsed.port || parsed.pathname !== "/" || parsed.search || parsed.hash) return null;
    if (normalized.endsWith(".") || normalized.includes("..")) return null;
    return normalized;
  } catch {
    return null;
  }
}

/** Whether a normalized host is an IP address, bracketed IPv6 included. */
function isIpHost(host) {
  if (isIP(host)) return true;
  return host.startsWith("[") && host.endsWith("]") && isIP(host.slice(1, -1)) === 6;
}

/** Whether a normalized host is one of the listed public suffixes. */
export function isCommonPublicSuffix(host) {
  return COMMON_PUBLIC_SUFFIXES.has(host);
}

/**
 * The name a host is registered under: the longest listed suffix plus one label, else its last two labels.
 * It is null for a listed suffix itself, and the host itself for an IP address or a single label.
 */
export function registrableDomain(host) {
  if (isIpHost(host)) return host;
  if (isCommonPublicSuffix(host)) return null;
  const labels = host.split(".");
  if (labels.length <= 2) return host;
  for (let index = 1; index < labels.length - 1; index += 1) {
    if (isCommonPublicSuffix(labels.slice(index).join("."))) return labels.slice(index - 1).join(".");
  }
  return labels.slice(-2).join(".");
}

/** The family a cookie domain belongs to when existing cookies are kept or replaced together, or null for a public suffix. */
export function registrableFamily(domain) {
  const host = normalizeCookieDomain(domain);
  return host ? registrableDomain(host) : null;
}

/** A cookie domain an import may write to: a normalized host that is not a listed public suffix. */
export function normalizeCookieImportDomain(domain) {
  const normalized = normalizeCookieDomain(domain);
  if (!normalized || isCommonPublicSuffix(normalized)) return null;
  return normalized;
}

const GOOGLE_SOURCE_BOUND_COOKIE_NAMES = new Set(["SIDCC", "__Secure-1PSIDCC", "__Secure-3PSIDCC", "__Secure-STRP", "AEC"]);

// Google binds these sessions to the device that signed in; YouTube accepts imported cookies and stays eligible.
const NON_TRANSPLANTABLE_DOMAINS = ["google.com"];

/** Whether a domain's cookies stay behind because the site binds its session to the source device. */
export function isNonTransplantableCookieDomain(domain) {
  const normalized = normalizeCookieDomain(domain);
  if (!normalized) return false;
  return NON_TRANSPLANTABLE_DOMAINS.some((root) => normalized === root || normalized.endsWith(`.${root}`));
}

/** Whether a cookie is one of Google's integrity cookies, which only the browser that received them can use. */
export function isGoogleSourceBoundCookie(name, domain) {
  if (!GOOGLE_SOURCE_BOUND_COOKIE_NAMES.has(name)) return false;
  const normalized = normalizeCookieDomain(domain);
  return normalized === "google.com" || normalized?.endsWith(".google.com") === true;
}

function domainSuffixes(domain) {
  const labels = domain.split(".");
  return labels.map((_, index) => labels.slice(index).join("."));
}

/** A domain and its parents up to the name it is registered under. */
function importDomainAncestors(domain) {
  const boundary = registrableDomain(domain) ?? domain;
  const ancestors = [];
  for (const suffix of domainSuffixes(domain)) {
    ancestors.push(suffix);
    if (suffix === boundary) break;
  }
  return ancestors;
}

/**
 * The existing cookies an import replaces, from the cookie domains it writes (a leading dot marks a domain cookie).
 * `exact` holds the imported hosts, `ancestors` their parents, and `descendantRoots` the hosts whose subdomains go too:
 * only those the import holds a domain cookie for, since a browser never stores one on a public suffix.
 * @param {readonly string[]} domains
 * @returns {{exact: Set<string>, ancestors: Set<string>, descendantRoots: Set<string>}}
 */
export function importedDomainScope(domains) {
  const exact = new Set();
  const ancestors = new Set();
  const descendantRoots = new Set();
  for (const domain of domains) {
    const normalized = normalizeCookieImportDomain(domain);
    if (!normalized) continue;
    const coversSubdomains = typeof domain === "string" && domain.trim().startsWith(".");
    if (coversSubdomains && normalized.includes(".") && !isIpHost(normalized)) descendantRoots.add(normalized);
    if (exact.has(normalized)) continue;
    exact.add(normalized);
    for (const suffix of importDomainAncestors(normalized)) ancestors.add(suffix);
  }
  return { exact, ancestors, descendantRoots };
}

/** Whether an existing cookie, by its normalized domain and host-only flag, falls in an import's scope. */
export function domainIsInImportedScope(scope, domain, hostOnly) {
  if (scope.exact.has(domain)) return true;
  if (!hostOnly && scope.ancestors.has(domain)) return true;
  return domainSuffixes(domain).some((suffix) => scope.descendantRoots.has(suffix));
}
