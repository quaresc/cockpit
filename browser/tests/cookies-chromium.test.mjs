// Reading a Chromium browser's cookies: the snapshot of its live database, the decryption of its values with an
// injected Safe Storage password, and what the reader leaves out. Every database is built here, on the acme world.
// The snapshot, Safe Storage and undecryptable-row cases are ported from Orca (https://github.com/stablyai/orca, commit 49498c3),
// src/main/browser/cookies/chromium/chromium-cookie-snapshot.test.ts, browser-cookie-import.test.ts and
// sources/browser-cookie-import-undecryptable.test.ts. MIT licence, Copyright (c) 2026 Lovecast Inc.

import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import os from "node:os";
import { dirname, join } from "node:path";
import { DatabaseSync } from "node:sqlite";
import { afterEach, beforeEach, test } from "node:test";
import { chromiumTimestampToUnix } from "../main/cookies/chromium.js";
import { cookieEncryptionVersion, decryptCookieValueRaw, deriveMacEncryptionKey } from "../main/cookies/decrypt.js";
import { read, sources } from "../main/cookies/index.js";
import { createCookieSnapshot } from "../main/cookies/snapshot.js";
import { chromiumTime, createChromiumCookieDatabase, encryptMacChromiumCookie } from "./fixtures/cookies/build.mjs";

const PASSWORD = "acme-safe-storage-password";
const NOW = Date.UTC(2026, 0, 15);
const NOW_SECONDS = NOW / 1000;

let root;
let home;
let tmpDir;
let writer;

beforeEach(() => {
  root = mkdtempSync(join(os.tmpdir(), "cockpit-cookies-chromium-test-"));
  home = join(root, "home");
  tmpDir = join(root, "tmp");
  mkdirSync(tmpDir, { recursive: true });
  writer = null;
});

afterEach(() => {
  writer?.close();
  rmSync(root, { recursive: true, force: true });
});

const chromeCookiesPath = (profile = "Default") => join(home, "Library", "Application Support", "Google", "Chrome", profile, "Network", "Cookies");

/** Builds a Chrome profile's cookie database under the fake home and returns the detected source. */
function chrome(rows, options) {
  createChromiumCookieDatabase(chromeCookiesPath(), rows, options).close();
  return sources({ home }).find((source) => source.id === "chrome");
}

/** A getKey that hands out the fixture password and records who asked. */
function keyring(password = PASSWORD) {
  const asked = [];
  const getKey = async (service, account) => {
    asked.push([service, account]);
    return password;
  };
  return { asked, getKey };
}

const encrypted = (value, options) => encryptMacChromiumCookie(value, PASSWORD, options);

function sourceFiles(databasePath) {
  const files = new Map();
  for (const suffix of ["", "-wal", "-shm"]) {
    if (existsSync(databasePath + suffix)) files.set(suffix, readFileSync(databasePath + suffix));
  }
  return files;
}

test("a Chromium timestamp converts from microseconds since 1601 to Unix seconds", () => {
  assert.equal(chromiumTimestampToUnix(chromiumTime(1_800_000_000)), 1_800_000_000);
  assert.equal(chromiumTimestampToUnix(String(chromiumTime(1_800_000_000))), 1_800_000_000);
  assert.equal(chromiumTimestampToUnix(Number(chromiumTime(1_800_000_000))), 1_800_000_000);
  assert.equal(chromiumTimestampToUnix(0n), 0);
  assert.equal(chromiumTimestampToUnix(0), 0);
  assert.equal(chromiumTimestampToUnix("0"), 0);
  assert.equal(chromiumTimestampToUnix(undefined), 0);
  assert.equal(chromiumTimestampToUnix("not a number"), 0);
  // A timestamp before 1970 clamps to 0.
  assert.equal(chromiumTimestampToUnix(1n), 0);
});

test("a v10 value decrypts with the key derived from the Safe Storage password", () => {
  const key = deriveMacEncryptionKey(PASSWORD);
  assert.equal(key.keysByVersion.v10.length, 16);
  assert.equal(decryptCookieValueRaw(encrypted("maya-session"), key, ".acme.example").toString("latin1"), "maya-session");
  assert.equal(cookieEncryptionVersion(encrypted("x")), "v10");
  assert.equal(cookieEncryptionVersion(Buffer.from("plain")), null);
  assert.equal(cookieEncryptionVersion(Buffer.from("v1")), null);
});

test("the host-key digest before a value is dropped, and a value without one is kept whole", () => {
  const key = deriveMacEncryptionKey(PASSWORD);
  const decrypt = (buffer, hostKey) => decryptCookieValueRaw(buffer, key, hostKey)?.toString("latin1");
  assert.equal(decrypt(encrypted("maya-session", { hostKey: ".acme.example" }), ".acme.example"), "maya-session");
  // An exact digest match also covers an empty value, which is too short for the fallback to see.
  assert.equal(decrypt(encrypted("", { hostKey: "api.acme.example" }), "api.acme.example"), "");
  // A digest that matches no host key still reads as one when it is mostly unprintable bytes.
  assert.equal(decrypt(encrypted("maya-session", { prefix: createHash("sha256").update("another-host").digest() }), ".acme.example"), "maya-session");
  // A long printable value has no digest to drop.
  const long = "a-printable-cookie-value-that-is-longer-than-thirty-two-bytes";
  assert.equal(decrypt(encrypted(long), ".acme.example"), long);
});

test("a value with no key for its version, a wrong key or no ciphertext does not decrypt", () => {
  const key = deriveMacEncryptionKey(PASSWORD);
  assert.equal(decryptCookieValueRaw(encrypted("x", { version: "v11" }), key, ".acme.example"), null);
  assert.equal(decryptCookieValueRaw(encrypted("x", { version: "v20" }), key, ".acme.example"), null);
  assert.equal(decryptCookieValueRaw(Buffer.from("v10"), key, ".acme.example"), null);
  assert.equal(decryptCookieValueRaw(Buffer.alloc(0), key, ".acme.example"), null);
  assert.equal(decryptCookieValueRaw(Buffer.from("plain text"), key, ".acme.example"), null);
  assert.equal(decryptCookieValueRaw(encrypted("maya-session"), deriveMacEncryptionKey("a-wrong-password"), ".acme.example"), null);
});

test("read uses the Safe Storage password for encrypted rows", async () => {
  const source = chrome([{ name: "sid", value: "", encryptedValue: encrypted("encrypted-value") }]);
  assert.equal(source.needsKey, true);
  const { asked, getKey } = keyring();

  const result = await read(source, { getKey, tmpDir, now: NOW });

  assert.deepEqual(asked, [["Chrome Safe Storage", "Chrome"]]);
  assert.deepEqual(result, {
    ok: true,
    cookies: [{ name: "sid", value: "encrypted-value", domain: ".acme.example", path: "/", secure: false, httpOnly: false, sameSite: "unspecified", partition: { status: "unpartitioned" } }],
    skipped: {},
    total: 1,
  });
});

test("read returns every column of a row, with and without the host-key digest", async () => {
  const source = chrome([
    { name: "plain", value: "plain-value", domain: "acme.example", path: "/app", sameSite: 1 },
    { name: "session", domain: ".acme.example", encryptedValue: encrypted("maya-session", { hostKey: ".acme.example" }), isSecure: 1, isHttpOnly: 1, sameSite: 2, expiresUtc: chromiumTime(NOW_SECONDS + 3600) },
    { name: "token", domain: "api.acme.example", encryptedValue: encrypted("api-token"), isSecure: 1, sameSite: 0 },
    { name: "empty", domain: "api.acme.example", encryptedValue: encrypted("", { hostKey: "api.acme.example" }), sameSite: null },
  ]);

  const result = await read(source, { getKey: keyring().getKey, tmpDir, now: NOW });

  assert.equal(result.ok, true);
  assert.deepEqual(result.skipped, {});
  assert.deepEqual(result.cookies, [
    { name: "plain", value: "plain-value", domain: "acme.example", path: "/app", secure: false, httpOnly: false, sameSite: "lax", partition: { status: "unpartitioned" } },
    { name: "session", value: "maya-session", domain: ".acme.example", path: "/", secure: true, httpOnly: true, sameSite: "strict", expirationDate: NOW_SECONDS + 3600, partition: { status: "unpartitioned" } },
    { name: "token", value: "api-token", domain: "api.acme.example", path: "/", secure: true, httpOnly: false, sameSite: "no_restriction", partition: { status: "unpartitioned" } },
    { name: "empty", value: "", domain: "api.acme.example", path: "/", secure: false, httpOnly: false, sameSite: "unspecified", partition: { status: "unpartitioned" } },
  ]);
});

test("read never asks for the key when no importable row is encrypted", async () => {
  const source = chrome([
    { name: "theme", value: "dark" },
    { name: "SIDCC", domain: ".google.com", encryptedValue: encrypted("integrity") },
    { name: "SID", domain: "accounts.google.com", encryptedValue: encrypted("device-bound") },
  ]);
  const { asked, getKey } = keyring();

  const result = await read(source, { getKey, tmpDir, now: NOW });

  assert.deepEqual(asked, []);
  assert.equal(result.ok, true);
  assert.deepEqual(result.cookies.map((cookie) => cookie.name), ["theme"]);
  assert.deepEqual(result.skipped, { googleSourceBound: 1, nonTransplantableDomain: 1 });
  assert.equal(result.total, 3);
});

test("read refuses when the Keychain gives no password", async () => {
  const source = chrome([{ name: "sid", encryptedValue: encrypted("v") }]);
  const denied = { ok: false, reason: "keyDenied", message: "Could not access Google Chrome encryption key. macOS may have denied access to the Keychain." };
  assert.deepEqual(await read(source, { getKey: async () => null, tmpDir }), denied);
  assert.deepEqual(await read(source, { getKey: async () => "", tmpDir }), denied);
  assert.deepEqual(
    await read(source, {
      getKey: async () => {
        throw new Error("the prompt was dismissed");
      },
      tmpDir,
    }),
    denied,
  );
});

test("read counts the rows it cannot decrypt and still returns the others", async () => {
  const source = chrome([
    { name: "v10", encryptedValue: encrypted("v10-value") },
    // Valid padding under the v10 key must not make another version's row readable.
    { name: "v11", encryptedValue: encrypted("wrong-key-garbage", { version: "v11" }) },
    { name: "v20", encryptedValue: encrypted("app-bound", { version: "v20" }) },
    { name: "garbage", encryptedValue: Buffer.from("not a versioned value") },
  ]);

  const result = await read(source, { getKey: keyring().getKey, tmpDir, now: NOW });

  assert.equal(result.ok, true);
  assert.deepEqual(result.cookies.map((cookie) => [cookie.name, cookie.value]), [["v10", "v10-value"]]);
  assert.deepEqual(result.skipped, { undecryptable: 3 });
});

test("read with the wrong password returns no cookie rather than garbage", async () => {
  const source = chrome([
    { name: "a", encryptedValue: encrypted("maya-session") },
    { name: "b", encryptedValue: encrypted("api-token") },
  ]);
  const result = await read(source, { getKey: keyring("a-wrong-password").getKey, tmpDir, now: NOW });
  assert.deepEqual(result, { ok: true, cookies: [], skipped: { undecryptable: 2 }, total: 2 });
});

test("read leaves out expired rows and rows on a domain an import refuses", async () => {
  const source = chrome([
    { name: "live", value: "v", expiresUtc: chromiumTime(NOW_SECONDS + 60) },
    { name: "old", value: "v", expiresUtc: chromiumTime(NOW_SECONDS - 60) },
    { name: "super", value: "v", domain: ".co.uk" },
    { name: "bad", value: "v", domain: "acme.example/x" },
  ]);
  const result = await read(source, { getKey: keyring().getKey, tmpDir, now: NOW });
  assert.deepEqual(result.cookies.map((cookie) => cookie.name), ["live"]);
  assert.deepEqual(result.skipped, { expired: 1, invalid: 2 });
});

test("read reports each row's site partition, and marks one it cannot read", async () => {
  const source = chrome([
    { name: "chips", value: "v", domain: "embed.acme.example", topFrameSiteKey: "https://acme.example", hasCrossSiteAncestor: 1 },
    { name: "opaque", value: "v", domain: "embed.acme.example", topFrameSiteKey: "not a site" },
    { name: "flagless", value: "v", domain: "embed.acme.example", topFrameSiteKey: "https://acme.example", hasCrossSiteAncestor: 7 },
  ]);
  const result = await read(source, { getKey: keyring().getKey, tmpDir, now: NOW });
  assert.deepEqual(result.cookies.map((cookie) => [cookie.name, cookie.partition.status]), [
    ["chips", "partitioned"],
    ["opaque", "unreadable"],
    ["flagless", "unreadable"],
  ]);
  assert.deepEqual(result.cookies[0].partition.partitionKey, { topLevelSite: "https://acme.example", hasCrossSiteAncestor: true });
});

test("read takes a schema older than partitioned cookies as unpartitioned", async () => {
  const source = chrome([{ name: "legacy", value: "v" }], { partitionColumns: false });
  const result = await read(source, { getKey: keyring().getKey, tmpDir, now: NOW });
  assert.deepEqual(result.cookies.map((cookie) => cookie.partition), [{ status: "unpartitioned" }]);
});

test("read picks the asked profile by folder or by name, and refuses one the source does not list", async () => {
  const base = join(home, "Library", "Application Support", "Google", "Chrome");
  mkdirSync(base, { recursive: true });
  createChromiumCookieDatabase(join(base, "Default", "Network", "Cookies"), [{ name: "who", value: "maya" }]).close();
  createChromiumCookieDatabase(join(base, "Profile 1", "Cookies"), [{ name: "who", value: "sam" }]).close();
  const localState = { profile: { info_cache: { Default: { name: "Maya" }, "Profile 1": { name: "Sam" } } } };
  createChromiumCookieDatabase(join(root, "elsewhere", "Cookies"), [{ name: "who", value: "nobody" }]).close();
  writeFileSync(join(base, "Local State"), JSON.stringify(localState));
  const source = sources({ home }).find((detected) => detected.id === "chrome");
  const who = async (profile) => (await read(source, { profile, tmpDir, now: NOW })).cookies?.[0].value;

  assert.equal(await who(undefined), "maya");
  assert.equal(await who("Profile 1"), "sam");
  assert.equal(await who("Sam"), "sam");
  assert.equal(await who(source.profiles[1]), "sam");
  assert.deepEqual(await read(source, { profile: "Profile 9", tmpDir }), { ok: false, reason: "invalidProfile", message: "Google Chrome has no such profile." });
  assert.equal((await read(source, { profile: { directory: "../elsewhere", path: join(root, "elsewhere", "Cookies") }, tmpDir })).reason, "invalidProfile");
  assert.equal((await read({ ...source, family: "netscape" }, { tmpDir })).reason, "invalidProfile");
  assert.equal((await read(null, { tmpDir })).reason, "invalidProfile");
});

test("read reads committed rows that live only in the write-ahead log, and leaves no snapshot behind", async () => {
  writer = createChromiumCookieDatabase(chromeCookiesPath(), [{ name: "wal-session", value: "fresh-value" }], { journalMode: "wal" });
  const before = sourceFiles(chromeCookiesPath());
  assert.equal(before.has("-wal"), true);
  const source = sources({ home }).find((detected) => detected.id === "chrome");

  const result = await read(source, { getKey: keyring().getKey, tmpDir, now: NOW });

  assert.deepEqual(result.cookies.map((cookie) => [cookie.name, cookie.value]), [["wal-session", "fresh-value"]]);
  assert.deepEqual(sourceFiles(chromeCookiesPath()), before);
  assert.deepEqual(readdirSync(tmpDir), []);
});

test("read says so when the database is gone, empty, or cannot be copied", async () => {
  const source = chrome([]);
  assert.deepEqual(await read(source, { tmpDir }), { ok: false, reason: "noCookies", message: "No cookies found in Google Chrome." });

  const refuse = (code) => () => {
    throw Object.assign(new Error(code), { code });
  };
  const denied = await read(source, { tmpDir, copyFile: refuse("EPERM") });
  assert.equal(denied.reason, "fullDiskAccess");
  assert.match(denied.message, /^macOS denied access to Google Chrome cookies\. Grant Full Disk Access/);
  assert.equal((await read(source, { tmpDir, copyFile: refuse("EBUSY") })).reason, "databaseCopyFailed");
  assert.deepEqual(readdirSync(tmpDir), []);

  rmSync(chromeCookiesPath());
  assert.deepEqual(await read(source, { tmpDir }), { ok: false, reason: "databaseNotFound", message: "Google Chrome cookies database not found." });
});

test("read reports a file that is not a cookie database", async () => {
  mkdirSync(dirname(chromeCookiesPath()), { recursive: true });
  writeFileSync(chromeCookiesPath(), "this is not sqlite");
  const source = sources({ home }).find((detected) => detected.id === "chrome");
  assert.equal((await read(source, { tmpDir })).reason, "readFailed");
  assert.deepEqual(readdirSync(tmpDir), []);
});

test("a snapshot reads committed WAL-only rows without changing any live source file", () => {
  const sourcePath = chromeCookiesPath();
  writer = createChromiumCookieDatabase(sourcePath, [{ name: "wal-session", value: "fresh-value" }], { journalMode: "wal" });
  const before = sourceFiles(sourcePath);

  const snapshot = createCookieSnapshot(sourcePath, { tempRoot: tmpDir });
  const snapshotDir = dirname(snapshot.databasePath);
  const database = new DatabaseSync(snapshot.databasePath, { readOnly: true });
  const rows = database.prepare("SELECT name, value FROM cookies").all();
  database.close();

  assert.deepEqual(rows.map((row) => ({ ...row })), [{ name: "wal-session", value: "fresh-value" }]);
  assert.equal(existsSync(`${snapshot.databasePath}-wal`), true);
  assert.equal(existsSync(`${snapshot.databasePath}-shm`), true);
  assert.deepEqual(sourceFiles(sourcePath), before);

  snapshot.cleanup();
  assert.equal(existsSync(snapshotDir), false);
});

test("a snapshot retries when a WAL appears while the main database is being copied", () => {
  const sourcePath = join(home, "Chrome", "Default", "Cookies");
  createChromiumCookieDatabase(sourcePath, []).close();
  let raced = false;
  const copyFile = (from, to) => {
    if (from === sourcePath && !raced) {
      raced = true;
      writer = new DatabaseSync(sourcePath);
      writer.exec(
        "PRAGMA journal_mode = WAL; PRAGMA wal_autocheckpoint = 0; INSERT INTO cookies (creation_utc, host_key, name, value, path, expires_utc, is_secure, is_httponly, samesite) VALUES (1, '.acme.example', 'raced-in', 'wal-value', '/', 0, 0, 0, 0)",
      );
    }
    copyFileSync(from, to);
  };

  const snapshot = createCookieSnapshot(sourcePath, { tempRoot: tmpDir, copyFile });
  const database = new DatabaseSync(snapshot.databasePath, { readOnly: true });
  const row = database.prepare("SELECT value FROM cookies WHERE name = 'raced-in'").get();
  database.close();

  assert.equal(raced, true);
  assert.equal(row.value, "wal-value");
  snapshot.cleanup();
});

test("a snapshot retries when the sidecars disappear after the browser checkpoints on close", () => {
  const sourcePath = join(home, "Chrome", "Default", "Cookies");
  writer = createChromiumCookieDatabase(sourcePath, [{ name: "checkpointed", value: "main-value" }], { journalMode: "wal" });
  const copyFile = (from, to) => {
    if (from === sourcePath && writer) {
      writer.close();
      writer = null;
    }
    copyFileSync(from, to);
  };

  const snapshot = createCookieSnapshot(sourcePath, { tempRoot: tmpDir, copyFile });
  const database = new DatabaseSync(snapshot.databasePath, { readOnly: true });
  const row = database.prepare("SELECT value FROM cookies WHERE name = 'checkpointed'").get();
  database.close();

  assert.equal(row.value, "main-value");
  snapshot.cleanup();
});

test("a snapshot gives up and cleans up when the database never stays still or is missing", () => {
  const sourcePath = join(home, "Chrome", "Default", "Cookies");
  createChromiumCookieDatabase(sourcePath, []).close();
  let copies = 0;
  const copyFile = (from, to) => {
    copies += 1;
    copyFileSync(from, to);
    const churn = new DatabaseSync(sourcePath);
    churn.exec(`CREATE TABLE churn_${copies} (id INTEGER)`);
    churn.close();
  };
  assert.throws(() => createCookieSnapshot(sourcePath, { tempRoot: tmpDir, copyFile }), /changed while creating a snapshot/);
  assert.equal(copies, 5);
  assert.throws(() => createCookieSnapshot(join(home, "missing", "Cookies"), { tempRoot: tmpDir }), /does not exist/);
  assert.deepEqual(readdirSync(tmpDir), []);
});
