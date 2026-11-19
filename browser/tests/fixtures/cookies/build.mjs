// Builders for the cookie stores the import tests read, all on the acme demo world.
// The Chromium database and its encryption are ported from Orca (https://github.com/stablyai/orca, commit 49498c3),
// src/main/browser/cookies/browser-cookie-import-test-database.ts, and the Safari bytes from browser-cookie-import.test.ts.
// MIT licence, Copyright (c) 2026 Lovecast Inc. Translated from TypeScript and adapted to cockpit's browser shell.

import { createCipheriv, createHash, pbkdf2Sync } from "node:crypto";
import { mkdirSync } from "node:fs";
import { dirname } from "node:path";
import { DatabaseSync } from "node:sqlite";

/** Seconds from 1601 to 1970, the gap between Chromium's clock and Unix time. */
const CHROMIUM_EPOCH_OFFSET = 11644473600n;

/** Unix seconds as the microsecond timestamp Chromium stores. */
export const chromiumTime = (unixSeconds) => (BigInt(unixSeconds) + CHROMIUM_EPOCH_OFFSET) * 1000000n;

/**
 * Creates a Chromium `Cookies` database with one row per entry and returns it open.
 * A row takes {domain, name, value, encryptedValue, path, expiresUtc, topFrameSiteKey, hasCrossSiteAncestor, isSecure, isHttpOnly, sameSite}.
 */
export function createChromiumCookieDatabase(databasePath, rows, options = {}) {
  mkdirSync(dirname(databasePath), { recursive: true });
  const database = new DatabaseSync(databasePath);
  if (options.journalMode === "wal") database.exec("PRAGMA journal_mode = WAL; PRAGMA wal_autocheckpoint = 0");
  const partitionColumns = options.partitionColumns === false ? "" : "top_frame_site_key TEXT NOT NULL DEFAULT '', has_cross_site_ancestor INTEGER NOT NULL DEFAULT 0,";
  database.exec(`
    CREATE TABLE cookies (
      creation_utc INTEGER NOT NULL,
      host_key TEXT NOT NULL,
      ${partitionColumns}
      name TEXT NOT NULL,
      value TEXT NOT NULL,
      encrypted_value BLOB NOT NULL DEFAULT X'',
      path TEXT NOT NULL,
      expires_utc INTEGER NOT NULL,
      is_secure INTEGER NOT NULL,
      is_httponly INTEGER NOT NULL,
      samesite INTEGER,
      source_scheme INTEGER NOT NULL DEFAULT 0,
      source_port INTEGER NOT NULL DEFAULT -1,
      last_update_utc INTEGER NOT NULL DEFAULT 0
    )
  `);
  const insert = database.prepare(
    `INSERT INTO cookies (creation_utc, host_key, name, value, encrypted_value, path, expires_utc, is_secure, is_httponly, samesite)
     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
  );
  const setPartition = options.partitionColumns === false ? null : database.prepare("UPDATE cookies SET top_frame_site_key = ?, has_cross_site_ancestor = ? WHERE creation_utc = ?");
  rows.forEach((row, index) => {
    const created = 13_300_000_000_000_000n + BigInt(index);
    insert.run(
      created,
      row.domain ?? ".acme.example",
      row.name,
      row.value ?? "",
      row.encryptedValue ?? Buffer.alloc(0),
      row.path ?? "/",
      row.expiresUtc ?? 0n,
      row.isSecure ?? 0,
      row.isHttpOnly ?? 0,
      row.sameSite === undefined ? -1 : row.sameSite,
    );
    if (setPartition && row.topFrameSiteKey !== undefined) setPartition.run(row.topFrameSiteKey, row.hasCrossSiteAncestor ?? 0, created);
  });
  return database;
}

/**
 * Encrypts a cookie value as a macOS Chromium browser does: AES-128-CBC under the key derived from the Safe Storage
 * password, behind a version prefix. `hostKey` puts the host-key digest newer schemas store before the value.
 */
export function encryptMacChromiumCookie(value, password, { version = "v10", hostKey, prefix } = {}) {
  const key = pbkdf2Sync(password, "saltysalt", 1003, 16, "sha1");
  const cipher = createCipheriv("aes-128-cbc", key, Buffer.alloc(16, " "));
  const digest = prefix ?? (hostKey === undefined ? Buffer.alloc(0) : createHash("sha256").update(hostKey).digest());
  return Buffer.concat([Buffer.from(version), cipher.update(Buffer.concat([digest, Buffer.from(value, "latin1")])), cipher.final()]);
}

/**
 * Creates a Firefox `cookies.sqlite` with one row per entry and returns it open.
 * A row takes {name, value, host, path, expiry, isSecure, isHttpOnly, sameSite, isPartitionedAttributeSet}.
 */
export function createFirefoxCookieDatabase(databasePath, rows, options = {}) {
  mkdirSync(dirname(databasePath), { recursive: true });
  const database = new DatabaseSync(databasePath);
  const partitioned = options.partitionColumn !== false;
  database.exec(`
    CREATE TABLE moz_cookies (
      id INTEGER PRIMARY KEY,
      originAttributes TEXT NOT NULL DEFAULT '',
      name TEXT,
      value TEXT,
      host TEXT,
      path TEXT,
      expiry INTEGER,
      lastAccessed INTEGER,
      creationTime INTEGER,
      isSecure INTEGER,
      isHttpOnly INTEGER,
      inBrowserElement INTEGER DEFAULT 0,
      sameSite INTEGER DEFAULT 0
      ${partitioned ? ", isPartitionedAttributeSet INTEGER DEFAULT 0" : ""}
    )
  `);
  const insert = database.prepare("INSERT INTO moz_cookies (name, value, host, path, expiry, isSecure, isHttpOnly, sameSite) VALUES (?, ?, ?, ?, ?, ?, ?, ?)");
  const setPartitioned = partitioned ? database.prepare("UPDATE moz_cookies SET isPartitionedAttributeSet = ? WHERE id = ?") : null;
  for (const row of rows) {
    const { lastInsertRowid } = insert.run(row.name, row.value ?? "", row.host ?? ".acme.example", row.path ?? "/", row.expiry ?? 0, row.isSecure ?? 0, row.isHttpOnly ?? 0, row.sameSite ?? 0);
    if (setPartitioned && row.isPartitionedAttributeSet !== undefined) setPartitioned.run(row.isPartitionedAttributeSet, lastInsertRowid);
  }
  return database;
}

/** Seconds from 1970 to 2001, the gap between Unix time and Safari's clock. */
const MAC_EPOCH_DELTA = 978_307_200;

/** One Safari cookie record: a 48-byte header, then the domain, name, path and value as C strings. */
export function buildSafariCookie({ domain, name, path = "/", value = "", expires = 0, secure = false, httpOnly = false }) {
  const strings = [domain, name, path, value];
  let cursor = 48;
  const offsets = strings.map((text) => {
    const offset = cursor;
    cursor += Buffer.byteLength(text) + 1;
    return offset;
  });
  const cookie = Buffer.alloc(cursor);
  cookie.writeUInt32LE(cookie.length, 0);
  cookie.writeUInt32LE((secure ? 1 : 0) | (httpOnly ? 4 : 0), 8);
  offsets.forEach((offset, index) => cookie.writeUInt32LE(offset, 16 + index * 4));
  cookie.writeDoubleLE(expires > 0 ? expires - MAC_EPOCH_DELTA : 0, 40);
  strings.forEach((text, index) => cookie.write(text, offsets[index], "utf8"));
  return cookie;
}

/** A `Cookies.binarycookies` file of one page per list of cookies; `expires` is Unix seconds. */
export function buildSafariBinaryCookies(...pages) {
  const pageBuffers = pages.map((cookies) => {
    const records = cookies.map(buildSafariCookie);
    const header = Buffer.alloc(8 + records.length * 4);
    header.writeUInt32BE(0x00000100, 0);
    header.writeUInt32LE(records.length, 4);
    let offset = header.length;
    records.forEach((record, index) => {
      header.writeUInt32LE(offset, 8 + index * 4);
      offset += record.length;
    });
    // A page ends with four zero bytes.
    return Buffer.concat([header, ...records, Buffer.alloc(4)]);
  });
  const head = Buffer.alloc(8 + pageBuffers.length * 4);
  head.write("cook", 0, "utf8");
  head.writeUInt32BE(pageBuffers.length, 4);
  pageBuffers.forEach((page, index) => head.writeUInt32BE(page.length, 8 + index * 4));
  return Buffer.concat([head, ...pageBuffers]);
}
