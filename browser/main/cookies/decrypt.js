// Ported from Orca (https://github.com/stablyai/orca, commit 49498c3), src/main/browser/cookies/sources/browser-cookie-decryption.ts,
// sources/chromium-encryption-key-contract.ts and the macOS key derivation of sources/browser-cookie-key.ts.
// MIT licence, Copyright (c) 2026 Lovecast Inc. Translated from TypeScript and adapted to cockpit's browser shell.

import { createDecipheriv, createHash, pbkdf2Sync } from "node:crypto";

const PBKDF2_ITERATIONS = 1003;
const PBKDF2_KEY_LENGTH = 16;
const PBKDF2_SALT = "saltysalt";

// Chromium prefixes a cookie's plaintext with the 32-byte SHA-256 of its host key.
const HOST_DIGEST_LENGTH = 32;

/**
 * The keys a Chromium profile's cookies are encrypted with, by the version prefix of the stored value.
 * @typedef {{mode: "aes-128-cbc", keysByVersion: {v10?: Buffer}}} EncryptionKey
 */

/**
 * The AES key macOS Chromium browsers derive from their Safe Storage password in the Keychain.
 * @param {string} password
 * @returns {EncryptionKey}
 */
export function deriveMacEncryptionKey(password) {
  return { mode: "aes-128-cbc", keysByVersion: { v10: pbkdf2Sync(password, PBKDF2_SALT, PBKDF2_ITERATIONS, PBKDF2_KEY_LENGTH, "sha1") } };
}

/** Whether a plaintext without a matching digest still starts with one: 8 unprintable bytes among its first 32. */
function hasDigestLikePrefix(buf) {
  if (buf.length <= HOST_DIGEST_LENGTH) return false;
  let nonPrintable = 0;
  for (let i = 0; i < HOST_DIGEST_LENGTH; i += 1) {
    if (buf[i] < 0x20 || buf[i] > 0x7e) nonPrintable += 1;
  }
  return nonPrintable >= 8;
}

/** Whether a plaintext starts with the SHA-256 of its host key; an exact match also covers an empty cookie value. */
function hasHostKeyDigestPrefix(buf, hostKey) {
  return buf.length >= HOST_DIGEST_LENGTH && buf.subarray(0, HOST_DIGEST_LENGTH).equals(createHash("sha256").update(hostKey).digest());
}

function stripHostDigest(buf, hostKey) {
  return hasHostKeyDigestPrefix(buf, hostKey) || hasDigestLikePrefix(buf) ? buf.subarray(HOST_DIGEST_LENGTH) : buf;
}

/** The version prefix of an encrypted value (`v10`, `v11`, `v20`), or null when it has none. */
export function cookieEncryptionVersion(encryptedBuffer) {
  if (encryptedBuffer.length < 3) return null;
  const version = encryptedBuffer.subarray(0, 3).toString("utf-8");
  return /^v\d\d$/.test(version) ? version : null;
}

/**
 * Decrypts a versioned `os_crypt` value, or returns null when there is no key for its version or the key is wrong.
 * @param {Buffer} encryptedBuffer
 * @param {EncryptionKey} keyResult
 * @returns {Buffer | null}
 */
export function decryptChromiumEncryptedValue(encryptedBuffer, keyResult) {
  if (!encryptedBuffer || encryptedBuffer.length === 0) return null;
  const version = cookieEncryptionVersion(encryptedBuffer);
  const key = version ? keyResult.keysByVersion[version] : undefined;
  if (!key) return null;
  const ciphertext = encryptedBuffer.subarray(3);
  if (!ciphertext.length) return null;
  try {
    const decipher = createDecipheriv("aes-128-cbc", key, Buffer.alloc(16, " "));
    decipher.setAutoPadding(true);
    return Buffer.concat([decipher.update(ciphertext), decipher.final()]);
  } catch {
    return null;
  }
}

/**
 * Decrypts a cookie's `encrypted_value` and drops the host-key digest newer Chromium schemas put before the value.
 * @param {Buffer} encryptedBuffer
 * @param {EncryptionKey} keyResult
 * @param {string} hostKey The row's `host_key`.
 * @returns {Buffer | null}
 */
export function decryptCookieValueRaw(encryptedBuffer, keyResult, hostKey) {
  const decrypted = decryptChromiumEncryptedValue(encryptedBuffer, keyResult);
  return decrypted && stripHostDigest(decrypted, hostKey);
}
