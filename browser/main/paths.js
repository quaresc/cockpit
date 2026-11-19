// Where the shell keeps its files, all under the home folder and never in a worktree.

import os from "node:os";
import path from "node:path";
import { createHash } from "node:crypto";

const home = () => process.env.HOME || os.homedir();

/** What must last: profiles, sign-ins, each session's tabs. */
export const dataDir = () => path.join(home(), ".config", "cockpit", "browser");

/** What can be made again: picked elements, frames, the log. */
export const cacheDir = () => path.join(home(), ".cache", "cockpit", "browser");

/** The control socket, the one the hub says, else the one beside the hub's. */
export const socketPath = () => process.env.COCKPIT_BROWSER_SOCKET || path.join(home(), ".cache", "cockpit", "browser.sock");

/** A short stable name for a folder, used for file names and session names. */
export const keyOf = (folder) => createHash("sha256").update(String(folder)).digest("hex").slice(0, 12);

export const userData = () => path.join(dataDir(), "user-data");
export const profilesFile = () => path.join(dataDir(), "profiles.json");
export const historyFile = () => path.join(dataDir(), "history.json");
export const browserFile = (owner) => path.join(dataDir(), "browsers", `${keyOf(owner)}.json`);
export const grabsDir = () => path.join(cacheDir(), "grabs");
export const downloadsDir = (owner) => path.join(cacheDir(), "downloads", keyOf(owner));
export const logFile = () => path.join(cacheDir(), "browser.log");
