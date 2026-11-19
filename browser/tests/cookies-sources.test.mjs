// Finding the browsers under a fake home folder, and reading Firefox, Safari and cookie-file stores built here on
// the acme world.
// The large Safari page case is ported from Orca (https://github.com/stablyai/orca, commit 49498c3),
// src/main/browser/cookies/browser-cookie-import.test.ts. MIT licence, Copyright (c) 2026 Lovecast Inc.

import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import os from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, beforeEach, test } from "node:test";
import { CHROMIUM_BROWSERS, isSafeBrowserProfileDirectory, resolveChromiumCookiesPath, selectProfile } from "../main/cookies/detect.js";
import { read, readFile, sources } from "../main/cookies/index.js";
import { decodeSafariBinaryCookies, decodeSafariCookie, decodeSafariPage } from "../main/cookies/safari.js";
import { buildSafariBinaryCookies, buildSafariCookie, createFirefoxCookieDatabase } from "./fixtures/cookies/build.mjs";

const NOW = Date.UTC(2026, 0, 15);
const NOW_SECONDS = NOW / 1000;
const FIXTURES = join(dirname(fileURLToPath(import.meta.url)), "fixtures", "cookies");

let root;
let home;
let tmpDir;

beforeEach(() => {
  root = mkdtempSync(join(os.tmpdir(), "cockpit-cookies-sources-test-"));
  home = join(root, "home");
  tmpDir = join(root, "tmp");
  mkdirSync(tmpDir, { recursive: true });
});

afterEach(() => {
  rmSync(root, { recursive: true, force: true });
});

const support = (...parts) => join(home, "Library", "Application Support", ...parts);

function touch(path, content = "") {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, content);
}

test("a Mac with no browser has no source", () => {
  assert.deepEqual(sources({ home }), []);
});

test("every Chromium browser is found at its own folder with its own Keychain item", () => {
  for (const browser of CHROMIUM_BROWSERS) touch(support(browser.macRoot, "Default", "Network", "Cookies"));

  const found = sources({ home });

  assert.deepEqual(
    found.map((source) => [source.id, source.label, source.keychain.service, source.keychain.account]),
    [
      ["chrome", "Google Chrome", "Chrome Safe Storage", "Chrome"],
      ["edge", "Microsoft Edge", "Microsoft Edge Safe Storage", "Microsoft Edge"],
      ["arc", "Arc", "Arc Safe Storage", "Arc"],
      ["brave", "Brave", "Brave Safe Storage", "Brave"],
      ["comet", "Comet", "Comet Safe Storage", "Comet"],
      ["helium", "Helium", "Helium Storage Key", "Helium"],
      ["vivaldi", "Vivaldi", "Vivaldi Safe Storage", "Vivaldi"],
    ],
  );
  assert.equal(found.every((source) => source.family === "chromium" && source.needsKey === true), true);
  assert.deepEqual(found[2].profiles, [{ name: "Default", directory: "Default", path: support("Arc", "User Data", "Default", "Network", "Cookies") }]);
  assert.deepEqual(found[5].profiles[0].path, support("net.imput.helium", "Default", "Network", "Cookies"));
});

test("Chromium profiles come from Local State, only those that hold a cookie database", () => {
  const info_cache = {
    Default: { name: "Maya" },
    "Profile 1": { name: "Sam (work)" },
    "Profile 2": { name: "No cookies yet" },
    "Profile 3": {},
    "../../escape": { name: "Unsafe" },
  };
  touch(support("Google", "Chrome", "Local State"), JSON.stringify({ profile: { info_cache } }));
  touch(support("Google", "Chrome", "Default", "Network", "Cookies"));
  touch(support("Google", "Chrome", "Default", "Cookies"));
  touch(support("Google", "Chrome", "Profile 1", "Cookies"));
  touch(support("Google", "Chrome", "Profile 3", "Network", "Cookies"));
  touch(support("Google", "escape", "Network", "Cookies"));

  const [chrome] = sources({ home });

  assert.deepEqual(chrome.profiles, [
    { name: "Maya", directory: "Default", path: support("Google", "Chrome", "Default", "Network", "Cookies") },
    { name: "Sam (work)", directory: "Profile 1", path: support("Google", "Chrome", "Profile 1", "Cookies") },
    { name: "Profile 3", directory: "Profile 3", path: support("Google", "Chrome", "Profile 3", "Network", "Cookies") },
  ]);
  assert.equal(selectProfile(chrome, undefined).directory, "Default");
  assert.equal(selectProfile(chrome, "Sam (work)").directory, "Profile 1");
  assert.equal(selectProfile(chrome, { directory: "Profile 3" }).name, "Profile 3");
  assert.equal(selectProfile(chrome, "Profile 2"), null);
});

test("an unreadable Local State falls back to the Default profile", () => {
  touch(support("BraveSoftware", "Brave-Browser", "Local State"), "{ not json");
  touch(support("BraveSoftware", "Brave-Browser", "Default", "Cookies"));
  touch(support("Vivaldi", "Local State"), JSON.stringify({ profile: {} }));
  touch(support("Vivaldi", "Default", "Network", "Cookies"));
  touch(support("Comet", "Local State"), JSON.stringify({ profile: { info_cache: { Default: { name: "Maya" } } } }));

  assert.deepEqual(
    sources({ home }).map((source) => [source.id, source.profiles.map((profile) => profile.name)]),
    [
      ["brave", ["Default"]],
      ["vivaldi", ["Default"]],
    ],
  );
});

test("a profile folder name is one safe path segment", () => {
  for (const safe of ["Default", "Profile 1", "x7k2.default-release"]) assert.equal(isSafeBrowserProfileDirectory(safe), true, safe);
  for (const unsafe of ["", ".", "..", "a/b", "a\\b", "a..b", "a\0b", 7]) assert.equal(isSafeBrowserProfileDirectory(unsafe), false, String(unsafe));
  assert.equal(resolveChromiumCookiesPath(support("nowhere")), null);
});

test("Firefox profiles list the default release first, named after their folder's suffix", () => {
  const profiles = support("Firefox", "Profiles");
  touch(join(profiles, "a1b2c3d4.dev-edition", "cookies.sqlite"));
  touch(join(profiles, "e5f6a7b8.default", "cookies.sqlite"));
  touch(join(profiles, "c9d0e1f2.default-release", "cookies.sqlite"));
  mkdirSync(join(profiles, "empty.profile"), { recursive: true });
  touch(join(profiles, "profiles.ini"));

  assert.deepEqual(sources({ home }), [
    {
      id: "firefox",
      family: "firefox",
      label: "Firefox",
      profiles: [
        { name: "default-release", directory: "c9d0e1f2.default-release", path: join(profiles, "c9d0e1f2.default-release", "cookies.sqlite") },
        { name: "default", directory: "e5f6a7b8.default", path: join(profiles, "e5f6a7b8.default", "cookies.sqlite") },
        { name: "dev-edition", directory: "a1b2c3d4.dev-edition", path: join(profiles, "a1b2c3d4.dev-edition", "cookies.sqlite") },
      ],
      needsKey: false,
    },
  ]);
});

test("Safari is found at its own cookie file, or the one in its container", () => {
  const container = join(home, "Library", "Containers", "com.apple.Safari", "Data", "Library", "Cookies", "Cookies.binarycookies");
  touch(container);
  assert.deepEqual(sources({ home }), [{ id: "safari", family: "safari", label: "Safari", profiles: [{ name: "Default", directory: "Default", path: container }], needsKey: false }]);

  const plain = join(home, "Library", "Cookies", "Cookies.binarycookies");
  touch(plain);
  assert.equal(sources({ home })[0].profiles[0].path, plain);
});

test("detection runs on injected probes alone, without touching the disk", () => {
  const fake = "/Users/maya";
  const app = (...parts) => join(fake, "Library", "Application Support", ...parts);
  const files = new Map([
    [app("Microsoft Edge", "Local State"), JSON.stringify({ profile: { info_cache: { Default: { name: "Maya" }, "Profile 1": { name: "Priya" } } } })],
    [app("Microsoft Edge", "Default", "Network", "Cookies"), ""],
    [app("Microsoft Edge", "Profile 1", "Network", "Cookies"), ""],
    [app("Firefox", "Profiles"), ""],
    [app("Firefox", "Profiles", "x7k2m9.default-release", "cookies.sqlite"), ""],
    [join(fake, "Library", "Cookies", "Cookies.binarycookies"), ""],
  ]);
  const probed = [];
  const found = sources({
    home: fake,
    exists: (path) => {
      probed.push(path);
      return files.has(path);
    },
    readText: (path) => files.get(path),
    listDirs: (path) => (path === app("Firefox", "Profiles") ? ["x7k2m9.default-release"] : []),
  });

  assert.deepEqual(
    found.map((source) => [source.id, source.profiles.map((profile) => profile.name)]),
    [
      ["edge", ["Maya", "Priya"]],
      ["firefox", ["default-release"]],
      ["safari", ["Default"]],
    ],
  );
  assert.equal(probed.every((path) => path.startsWith(fake)), true);
});

/** Builds a Firefox profile under the fake home and returns the detected source. */
function firefox(rows, options) {
  createFirefoxCookieDatabase(support("Firefox", "Profiles", "x7k2m9.default-release", "cookies.sqlite"), rows, options).close();
  return sources({ home }).find((source) => source.id === "firefox");
}

test("read returns a Firefox profile's cookies without asking for a key", async () => {
  const source = firefox([
    { name: "session", value: "maya-session", host: ".acme.example", expiry: NOW_SECONDS + 3600, isSecure: 1, isHttpOnly: 1, sameSite: 1 },
    { name: "token", value: "api-tökén\u0001", host: "api.acme.example", path: "", sameSite: 2 },
    { name: "modern", value: "v", host: "acme.example", expiry: (NOW_SECONDS + 60) * 1000, sameSite: 256 },
    { name: "old", value: "v", expiry: NOW_SECONDS - 60 },
    { name: "old-modern", value: "v", expiry: (NOW_SECONDS - 60) * 1000 },
    { name: "", value: "v" },
    { name: "hostless", value: "v", host: "" },
    { name: "bad-host", value: "v", host: "acme.example/x" },
    { name: "chips", value: "v", host: "embed.acme.example", isPartitionedAttributeSet: 1 },
  ]);
  const getKey = () => assert.fail("Firefox cookies are not encrypted");

  const result = await read(source, { getKey, tmpDir, now: NOW });

  assert.deepEqual(result, {
    ok: true,
    cookies: [
      { name: "session", value: "maya-session", domain: ".acme.example", path: "/", secure: true, httpOnly: true, sameSite: "lax", expirationDate: NOW_SECONDS + 3600, partition: { status: "unpartitioned" } },
      { name: "token", value: "api-tkn", domain: "api.acme.example", path: "/", secure: false, httpOnly: false, sameSite: "strict", partition: { status: "unpartitioned" } },
      { name: "modern", value: "v", domain: "acme.example", path: "/", secure: false, httpOnly: false, sameSite: "unspecified", expirationDate: NOW_SECONDS + 60, partition: { status: "unpartitioned" } },
      {
        name: "chips",
        value: "v",
        domain: "embed.acme.example",
        path: "/",
        secure: false,
        httpOnly: false,
        sameSite: "no_restriction",
        partition: { status: "unreadable", reason: "Firefox partitioned-attribute cookie has no cross-site-ancestor bit to read" },
      },
    ],
    skipped: { expired: 2, invalid: 3 },
    total: 9,
  });
  assert.deepEqual(readdirSync(tmpDir), []);
});

test("read takes a Firefox schema without the partitioned column", async () => {
  const source = firefox([{ name: "legacy", value: "v" }], { partitionColumn: false });
  const result = await read(source, { tmpDir, now: NOW });
  assert.deepEqual(result.cookies.map((cookie) => [cookie.name, cookie.partition.status]), [["legacy", "unpartitioned"]]);
});

test("read says so when Firefox has no cookie, none valid, or a file that is not a database", async () => {
  const empty = firefox([]);
  assert.deepEqual(await read(empty, { tmpDir, now: NOW }), { ok: false, reason: "noCookies", message: "No cookies found in Firefox." });
  rmSync(empty.profiles[0].path);

  const stale = firefox([{ name: "old", value: "v", expiry: NOW_SECONDS - 60 }]);
  assert.deepEqual(await read(stale, { tmpDir, now: NOW }), { ok: false, reason: "noValidCookies", message: "No valid cookies found in Firefox." });

  writeFileSync(stale.profiles[0].path, "this is not sqlite");
  assert.equal((await read(stale, { tmpDir, now: NOW })).reason, "readFailed");
  rmSync(stale.profiles[0].path);
  assert.equal((await read(stale, { tmpDir, now: NOW })).reason, "databaseNotFound");
  assert.deepEqual(readdirSync(tmpDir), []);
});

/** Writes a Safari cookie file under the fake home and returns the detected source. */
function safari(...pages) {
  touch(join(home, "Library", "Cookies", "Cookies.binarycookies"), buildSafariBinaryCookies(...pages));
  return sources({ home }).find((source) => source.id === "safari");
}

test("read returns Safari's cookies from every page of its binary file", async () => {
  const source = safari(
    [
      { domain: ".acme.example", name: "session", value: "maya-session", expires: NOW_SECONDS + 3600, secure: true, httpOnly: true },
      { domain: "api.acme.example", name: "token", value: "api-token", path: "/v1" },
    ],
    [
      { domain: "app.acme.example", name: "old", value: "v", expires: NOW_SECONDS - 60 },
      { domain: "app.acme.example", name: "theme", value: "dark", secure: true },
    ],
  );

  const result = await read(source, { tmpDir, now: NOW });

  assert.deepEqual(result, {
    ok: true,
    cookies: [
      { name: "session", value: "maya-session", domain: ".acme.example", path: "/", secure: true, httpOnly: true, sameSite: "unspecified", expirationDate: NOW_SECONDS + 3600, partition: { status: "unpartitioned" } },
      { name: "token", value: "api-token", domain: "api.acme.example", path: "/v1", secure: false, httpOnly: false, sameSite: "unspecified", partition: { status: "unpartitioned" } },
      { name: "theme", value: "dark", domain: "app.acme.example", path: "/", secure: true, httpOnly: false, sameSite: "unspecified", partition: { status: "unpartitioned" } },
    ],
    skipped: { expired: 1 },
    total: 4,
  });
});

test("read says so when Safari's cookies are all expired, absent, or behind Full Disk Access", async () => {
  const source = safari([{ domain: ".acme.example", name: "old", value: "v", expires: NOW_SECONDS - 60 }]);
  assert.deepEqual(await read(source, { now: NOW }), { ok: false, reason: "allCookiesExpired", message: "All Safari cookies are expired." });

  writeFileSync(source.profiles[0].path, "not a cookie file");
  assert.deepEqual(await read(source, { now: NOW }), { ok: false, reason: "noCookies", message: "No cookies found in Safari." });

  const refuse = (code) => () => {
    throw Object.assign(new Error(code), { code });
  };
  const denied = await read(source, { readBytes: refuse("EPERM") });
  assert.equal(denied.ok, false);
  assert.equal(denied.reason, "fullDiskAccess");
  assert.match(denied.message, /^macOS denied access to Safari cookies\. Grant Full Disk Access/);
  assert.equal((await read(source, { readBytes: refuse("EIO") })).reason, "readFailed");

  rmSync(source.profiles[0].path);
  assert.equal((await read(source, { now: NOW })).reason, "databaseNotFound");
});

test("a large Safari page decodes without overflowing the call stack", () => {
  const count = 150_000;
  const cookies = Array.from({ length: count }, (_, index) => ({ domain: `.expired-${index}.acme.example`, name: `sid-${index}`, value: "expired", expires: 1_000_000_000 }));
  const decoded = decodeSafariBinaryCookies(buildSafariBinaryCookies(cookies));
  assert.equal(decoded.length, count);
  assert.deepEqual([decoded[count - 1].name, decoded[count - 1].domain, decoded[count - 1].expirationDate], [`sid-${count - 1}`, `.expired-${count - 1}.acme.example`, 1_000_000_000]);
});

test("a truncated or malformed Safari file decodes to what is whole", () => {
  assert.deepEqual(decodeSafariBinaryCookies(Buffer.alloc(0)), []);
  assert.deepEqual(decodeSafariBinaryCookies(Buffer.from("nope0000")), []);
  const file = buildSafariBinaryCookies([{ domain: ".acme.example", name: "session", value: "v" }]);
  // A page count that promises more sizes than the file holds.
  const lying = Buffer.from(file);
  lying.writeUInt32BE(1000, 4);
  assert.deepEqual(decodeSafariBinaryCookies(lying), []);
  // A file cut in the middle of its only record.
  assert.deepEqual(decodeSafariBinaryCookies(file.subarray(0, file.length - 12)), []);
  assert.deepEqual(decodeSafariPage(Buffer.alloc(16)), []);

  const record = buildSafariCookie({ domain: ".acme.example", name: "session", value: "v" });
  assert.equal(decodeSafariCookie(record).name, "session");
  assert.equal(decodeSafariCookie(record.subarray(0, 40)), null);
  // A record that claims to be larger than it is reads no further than its bytes.
  const oversized = Buffer.from(record);
  oversized.writeUInt32LE(1_000_000, 0);
  assert.equal(decodeSafariCookie(oversized).value, "v");
  // An offset outside the record names nothing.
  const wild = Buffer.from(record);
  wild.writeUInt32LE(9999, 20);
  assert.equal(decodeSafariCookie(wild), null);
  assert.equal(decodeSafariCookie(buildSafariCookie({ domain: "", name: "hostless" })), null);
  assert.equal(decodeSafariCookie(buildSafariCookie({ domain: "acme.example/x", name: "bad" })), null);
});

test("readFile reads a JSON array of cookies and counts the entries it refuses", async () => {
  const result = await readFile(join(FIXTURES, "acme-cookies.json"));
  assert.deepEqual(result, {
    ok: true,
    cookies: [
      { name: "session", value: "maya-session", domain: ".acme.example", path: "/", secure: true, httpOnly: true, sameSite: "lax", expirationDate: 4102444800, partition: { status: "unpartitioned" } },
      { name: "token", value: "api-token", domain: "api.acme.example", path: "/", secure: true, httpOnly: true, sameSite: "no_restriction", partition: { status: "unpartitioned" } },
      {
        name: "__Host-chips",
        value: "partitioned",
        domain: "app.acme.example",
        path: "/",
        secure: true,
        httpOnly: false,
        sameSite: "no_restriction",
        partition: { status: "partitioned", partitionKey: { topLevelSite: "https://acme.example", hasCrossSiteAncestor: true } },
      },
    ],
    skipped: { invalid: 3 },
    total: 6,
  });
});

test("readFile says what is wrong with a file it cannot use", async () => {
  const file = join(tmpDir, "cookies.json");
  const reasonFor = async (content) => {
    writeFileSync(file, content);
    return (await readFile(file)).reason;
  };
  assert.deepEqual(await readFile(join(tmpDir, "missing.json")), { ok: false, reason: "fileUnreadable", message: "Could not read the selected file." });
  assert.equal(await reasonFor("# Netscape HTTP Cookie File"), "fileInvalidJson");
  assert.equal(await reasonFor('{"cookies": []}'), "fileNotArray");
  assert.equal(await reasonFor("[]"), "fileEmpty");
  writeFileSync(file, JSON.stringify([{ name: "no-domain", value: "v" }, 7]));
  assert.deepEqual(await readFile(file), { ok: false, reason: "fileNoValidCookies", message: "No valid cookies found. 2 entries were skipped due to missing or invalid fields." });
  writeFileSync(file, JSON.stringify([{ domain: "acme.example", name: "note", value: "café\n" }]));
  assert.equal((await readFile(file)).cookies[0].value, "caf");
});
