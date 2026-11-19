// Ported from Orca (https://github.com/stablyai/orca, commit 49498c3), src/main/browser/cookies/mutation/cookie-mutation-lock.ts.
// MIT licence, Copyright (c) 2026 Lovecast Inc. Translated from TypeScript and adapted to cockpit's browser shell.

/** @type {WeakMap<object, Promise<void>>} */
const mutationLocks = new WeakMap();

/**
 * Waits for the owner's turn and returns the function that ends it. The owner is the partition's Electron session,
 * so every change to one cookie jar queues behind the one before it.
 * @param {object} owner
 * @returns {Promise<() => void>}
 */
export async function acquireCookieMutationLock(owner) {
  const previous = mutationLocks.get(owner) ?? Promise.resolve();
  let release;
  const current = new Promise((resolve) => {
    release = resolve;
  });
  mutationLocks.set(
    owner,
    previous.then(() => current),
  );
  await previous;
  return release;
}

/**
 * Runs `run` while holding the owner's lock, and releases it whether `run` returns or throws.
 * @template T
 * @param {object} owner
 * @param {() => Promise<T>} run
 * @returns {Promise<T>}
 */
export async function withCookieMutationLock(owner, run) {
  const release = await acquireCookieMutationLock(owner);
  try {
    return await run();
  } finally {
    release();
  }
}
