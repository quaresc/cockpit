// cockpit's install and config.json as the window reads them, for the few options a mod acts on by itself. It imports
// nothing of cockpit's, so a link handler loads it without the hub.

import { appHome, readTextFile } from "herdr";

/** Where cockpit lives, as `cockpit app install` wrote it: {bin, home, cache, env, machine?}, or null. */
export async function installOf() {
  try {
    return JSON.parse((await readTextFile(`${appHome()}/cockpit.json`)) ?? "null");
  } catch {
    return null;
  }
}

/** config.json as the hub reads it: the cache's copy first, else the one in ~/.config/cockpit; null when neither reads. */
export async function readConfig(install) {
  const where = install ?? (await installOf());
  if (!where || where.machine) return null;
  for (const path of [where.cache && `${where.cache}/config.json`, where.home && `${where.home}/.config/cockpit/config.json`].filter(Boolean)) {
    try {
      const config = JSON.parse((await readTextFile(path)) ?? "null");
      if (config) return config;
    } catch {}
  }
  return null;
}

/** The value at a dotted key of a config, or `fallback` when it holds none. */
export function optionOf(config, key, fallback) {
  const value = key.split(".").reduce((node, part) => (node && typeof node === "object" ? node[part] : undefined), config);
  return value === undefined || value === null ? fallback : value;
}

let cached = { config: null, at: 0 };

/** An option as last read, read again at most every 10 s; `fallback` until a read says otherwise. */
export function option(key, fallback) {
  if (Date.now() - cached.at > 10_000) {
    cached.at = Date.now();
    readConfig().then((config) => (cached = { config, at: cached.at }), () => {});
  }
  return optionOf(cached.config, key, fallback);
}
