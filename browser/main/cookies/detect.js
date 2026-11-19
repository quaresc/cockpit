// Ported from Orca (https://github.com/stablyai/orca, commit 49498c3), src/main/browser/cookies/sources/browser-cookie-detection.ts,
// sources/browser-cookie-source-profiles.ts and chromium/chromium-cookie-path.ts (macOS rows).
// MIT licence, Copyright (c) 2026 Lovecast Inc. Translated from TypeScript and adapted to cockpit's browser shell.

import { existsSync, readFileSync, readdirSync } from "node:fs";
import os from "node:os";
import { join } from "node:path";

/**
 * The Chromium browsers cookies can come from: where each keeps its profiles under ~/Library/Application Support,
 * and the Keychain item that holds its Safe Storage password.
 */
export const CHROMIUM_BROWSERS = [
  { id: "chrome", label: "Google Chrome", keychainService: "Chrome Safe Storage", keychainAccount: "Chrome", macRoot: "Google/Chrome" },
  { id: "edge", label: "Microsoft Edge", keychainService: "Microsoft Edge Safe Storage", keychainAccount: "Microsoft Edge", macRoot: "Microsoft Edge" },
  { id: "arc", label: "Arc", keychainService: "Arc Safe Storage", keychainAccount: "Arc", macRoot: "Arc/User Data" },
  { id: "brave", label: "Brave", keychainService: "Brave Safe Storage", keychainAccount: "Brave", macRoot: "BraveSoftware/Brave-Browser" },
  { id: "comet", label: "Comet", keychainService: "Comet Safe Storage", keychainAccount: "Comet", macRoot: "Comet" },
  { id: "helium", label: "Helium", keychainService: "Helium Storage Key", keychainAccount: "Helium", macRoot: "net.imput.helium" },
  { id: "vivaldi", label: "Vivaldi", keychainService: "Vivaldi Safe Storage", keychainAccount: "Vivaldi", macRoot: "Vivaldi" },
];

/**
 * The file-system reads detection makes, each replaceable so a test needs no real browser.
 * @typedef {object} DetectOptions
 * @property {string} [home] The home folder; the person's own by default.
 * @property {(path: string) => boolean} [exists]
 * @property {(path: string) => string} [readText] A file's text; it may throw.
 * @property {(path: string) => string[]} [listDirs] The names of a folder's sub-folders; it may throw.
 */

function probes(options = {}) {
  return {
    home: options.home ?? process.env.HOME ?? os.homedir(),
    exists: options.exists ?? existsSync,
    readText: options.readText ?? ((path) => readFileSync(path, "utf-8")),
    listDirs:
      options.listDirs ??
      ((path) =>
        readdirSync(path, { withFileTypes: true })
          .filter((entry) => entry.isDirectory())
          .map((entry) => entry.name)),
  };
}

/** Whether a profile folder name read from a browser's own files is safe to use as one path segment. */
export function isSafeBrowserProfileDirectory(directory) {
  return (
    typeof directory === "string" &&
    directory.length > 0 &&
    directory !== "." &&
    !directory.includes("\0") &&
    !directory.includes("/") &&
    !directory.includes("\\") &&
    !directory.includes("..")
  );
}

/** A Chromium profile's cookie database: under Network/ from Chromium 96 on, at the profile root in older versions. */
export function resolveChromiumCookiesPath(profileDir, exists = existsSync) {
  const networkPath = join(profileDir, "Network", "Cookies");
  if (exists(networkPath)) return networkPath;
  const rootPath = join(profileDir, "Cookies");
  return exists(rootPath) ? rootPath : null;
}

/** A Chromium browser's profiles as its Local State names them, or the Default profile alone when that cannot be read. */
function discoverChromiumProfiles(browserRoot, fs) {
  const fallback = [{ name: "Default", directory: "Default" }];
  try {
    const localStatePath = join(browserRoot, "Local State");
    if (!fs.exists(localStatePath)) return fallback;
    const infoCache = JSON.parse(fs.readText(localStatePath))?.profile?.info_cache;
    if (!infoCache || typeof infoCache !== "object") return fallback;
    const profiles = [];
    for (const [directory, info] of Object.entries(infoCache)) {
      if (!isSafeBrowserProfileDirectory(directory)) continue;
      profiles.push({ name: typeof info?.name === "string" && info.name ? info.name : directory, directory });
    }
    return profiles.length > 0 ? profiles : fallback;
  } catch {
    return fallback;
  }
}

function detectChromium(browser, fs) {
  const root = join(fs.home, "Library", "Application Support", browser.macRoot);
  const profiles = [];
  for (const profile of discoverChromiumProfiles(root, fs)) {
    const path = resolveChromiumCookiesPath(join(root, profile.directory), fs.exists);
    if (path) profiles.push({ ...profile, path });
  }
  if (profiles.length === 0) return null;
  return {
    id: browser.id,
    family: "chromium",
    label: browser.label,
    profiles,
    needsKey: true,
    keychain: { service: browser.keychainService, account: browser.keychainAccount },
  };
}

/** Firefox's profile folders, the default-release one first, each named by what follows its random prefix. */
function discoverFirefoxProfiles(profilesRoot, fs) {
  try {
    if (!fs.exists(profilesRoot)) return [];
    const rank = (directory) => (directory.includes("default-release") ? 0 : directory.includes("default") ? 1 : 2);
    return fs
      .listDirs(profilesRoot)
      .filter(isSafeBrowserProfileDirectory)
      .sort((a, b) => rank(a) - rank(b))
      .map((directory) => ({ name: directory.includes(".") ? directory.split(".").slice(1).join(".") : directory, directory }));
  } catch {
    return [];
  }
}

function detectFirefox(fs) {
  const profilesRoot = join(fs.home, "Library", "Application Support", "Firefox", "Profiles");
  const profiles = [];
  for (const profile of discoverFirefoxProfiles(profilesRoot, fs)) {
    const path = join(profilesRoot, profile.directory, "cookies.sqlite");
    if (fs.exists(path)) profiles.push({ ...profile, path });
  }
  if (profiles.length === 0) return null;
  return { id: "firefox", family: "firefox", label: "Firefox", profiles, needsKey: false };
}

function detectSafari(fs) {
  const candidates = [
    join(fs.home, "Library", "Cookies", "Cookies.binarycookies"),
    join(fs.home, "Library", "Containers", "com.apple.Safari", "Data", "Library", "Cookies", "Cookies.binarycookies"),
  ];
  const path = candidates.find((candidate) => fs.exists(candidate));
  if (!path) return null;
  return { id: "safari", family: "safari", label: "Safari", profiles: [{ name: "Default", directory: "Default", path }], needsKey: false };
}

/**
 * The browsers on this Mac that hold a cookie store, each with the profiles that have one.
 * @param {DetectOptions} [options]
 * @returns {Array<{id: string, family: "chromium" | "firefox" | "safari", label: string, profiles: Array<{name: string, directory: string, path: string}>, needsKey: boolean, keychain?: {service: string, account: string}}>}
 */
export function detectSources(options) {
  const fs = probes(options);
  const detected = [];
  for (const browser of CHROMIUM_BROWSERS) {
    const source = detectChromium(browser, fs);
    if (source) detected.push(source);
  }
  const firefox = detectFirefox(fs);
  if (firefox) detected.push(firefox);
  const safari = detectSafari(fs);
  if (safari) detected.push(safari);
  return detected;
}

/**
 * The profile of a source that `wanted` names, by folder or by display name, or the first one when nothing is asked.
 * @param {{profiles: Array<{name: string, directory: string, path: string}>}} source
 * @param {string | {directory?: string} | undefined} wanted
 * @returns {{name: string, directory: string, path: string} | null}
 */
export function selectProfile(source, wanted) {
  const profiles = Array.isArray(source?.profiles) ? source.profiles : [];
  if (wanted === undefined || wanted === null) return profiles[0] ?? null;
  const key = typeof wanted === "string" ? wanted : wanted.directory;
  return profiles.find((profile) => profile.directory === key) ?? profiles.find((profile) => profile.name === key) ?? null;
}
