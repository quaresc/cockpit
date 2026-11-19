// Ported from Orca (https://github.com/stablyai/orca, commit 49498c3), src/main/browser/cookies/sources/browser-cookie-key.ts (macOS path).
// MIT licence, Copyright (c) 2026 Lovecast Inc. Translated from TypeScript and adapted to cockpit's browser shell.

import { execFile } from "node:child_process";

const SECURITY = "/usr/bin/security";
// macOS holds `security` open while its Keychain prompt waits for an answer.
const KEYCHAIN_TIMEOUT_MS = 30_000;

/**
 * Reads a browser's Safe Storage password from the login Keychain, which may ask the person to allow it.
 * It resolves to null when the item is missing, the person refuses, or the prompt times out.
 * @param {string} service The Keychain item's service, such as "Chrome Safe Storage".
 * @param {string} account The Keychain item's account, such as "Chrome".
 * @returns {Promise<string | null>}
 */
export function keychainPassword(service, account) {
  return new Promise((resolve) => {
    execFile(SECURITY, ["find-generic-password", "-s", service, "-a", account, "-w"], { timeout: KEYCHAIN_TIMEOUT_MS, encoding: "utf8" }, (error, stdout) => {
      const password = error ? "" : String(stdout).trim();
      resolve(password || null);
    });
  });
}
