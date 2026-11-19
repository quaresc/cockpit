// Ported from Orca (https://github.com/stablyai/orca, commit 49498c3), src/main/browser/cookies/chromium/chromium-cookie-snapshot.ts.
// MIT licence, Copyright (c) 2026 Lovecast Inc. Translated from TypeScript and adapted to cockpit's browser shell.

import { copyFileSync, mkdtempSync, rmSync, statSync, unlinkSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const SNAPSHOT_ATTEMPTS = 5;
export const SNAPSHOT_PREFIX = "cockpit-cookie-import-";

function isMissingFileError(error) {
  return error instanceof Error && error.code === "ENOENT";
}

function readFileState(path) {
  try {
    const stats = statSync(path, { bigint: true });
    return { device: stats.dev, inode: stats.ino, size: stats.size, modifiedAt: stats.mtimeNs, changedAt: stats.ctimeNs };
  } catch (error) {
    if (isMissingFileError(error)) return null;
    throw error;
  }
}

function sameFileState(left, right) {
  if (!left || !right) return left === right;
  return (
    left.device === right.device &&
    left.inode === right.inode &&
    left.size === right.size &&
    left.modifiedAt === right.modifiedAt &&
    left.changedAt === right.changedAt
  );
}

function removeAttemptFiles(databasePath) {
  for (const suffix of ["", "-wal", "-shm"]) {
    try {
      unlinkSync(databasePath + suffix);
    } catch {}
  }
}

/** Copies the database and its WAL once, and says whether the source stayed still while it did. */
function copyStableAttempt(sourcePath, databasePath, copyFile) {
  const sourceWalPath = `${sourcePath}-wal`;
  const databaseBefore = readFileState(sourcePath);
  const walBefore = readFileState(sourceWalPath);
  if (!databaseBefore) throw new Error("Cookies database does not exist");

  removeAttemptFiles(databasePath);
  copyFile(sourcePath, databasePath);
  if (walBefore) {
    try {
      // SQLite finds a WAL only beside a database of the same name.
      copyFile(sourceWalPath, `${databasePath}-wal`);
    } catch (error) {
      if (isMissingFileError(error)) return false;
      throw error;
    }
  }
  // The SHM index is left behind: SQLite rebuilds it beside the copied WAL.

  if (!sameFileState(databaseBefore, readFileState(sourcePath)) || !sameFileState(walBefore, readFileState(sourceWalPath))) return false;
  const copiedDatabase = readFileState(databasePath);
  const copiedWal = readFileState(`${databasePath}-wal`);
  return copiedDatabase?.size === databaseBefore.size && (walBefore ? copiedWal?.size === walBefore.size : copiedWal === null);
}

/**
 * Copies a live SQLite cookie database and its write-ahead log into a fresh temporary folder, retrying while the
 * browser changes them, so the copy can be read without touching the browser's own files.
 * @param {string} sourcePath
 * @param {{tempRoot?: string, fileName?: string, copyFile?: (from: string, to: string) => void}} [options]
 * @returns {{databasePath: string, cleanup: () => void}}
 */
export function createCookieSnapshot(sourcePath, options = {}) {
  const snapshotDir = mkdtempSync(join(options.tempRoot ?? tmpdir(), SNAPSHOT_PREFIX));
  const databasePath = join(snapshotDir, options.fileName ?? "Cookies");
  const copyFile = options.copyFile ?? copyFileSync;
  const cleanup = () => rmSync(snapshotDir, { recursive: true, force: true });
  let keepSnapshot = false;
  try {
    for (let attempt = 0; attempt < SNAPSHOT_ATTEMPTS; attempt += 1) {
      if (copyStableAttempt(sourcePath, databasePath, copyFile)) {
        keepSnapshot = true;
        return { databasePath, cleanup };
      }
    }
    throw new Error("Cookies database changed while creating a snapshot");
  } finally {
    if (!keepSnapshot) cleanup();
  }
}
