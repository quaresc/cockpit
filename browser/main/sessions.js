// One Electron session per profile, set up once: what pages may do, where downloads go, and a user agent without the
// shell's own name in it, so sites treat it as the Chromium it is.

import fs from "node:fs";
import path from "node:path";
import { session, dialog } from "electron";
import { partitionOf } from "./profiles.js";
import { verdict, siteOf, allowedNow, WORDS } from "./permissions.js";

const ready = new Map();

/** A user agent with Electron's and the app's tokens taken out. */
export const cleanAgent = (agent) => agent.replace(/\s(Electron|cockpit-browser|Cockpit Browser)\/\S+/gi, "");

/** A file name that is free in a folder: `report.pdf`, then `report (1).pdf`. */
export function freeName(dir, name, exists = fs.existsSync) {
  const safe = path.basename(name || "download").replace(/[\0/\\:]/g, "_") || "download";
  const { name: stem, ext } = path.parse(safe);
  for (let n = 0; n < 1000; n++) {
    const candidate = n === 0 ? safe : `${stem} (${n})${ext}`;
    if (!exists(path.join(dir, candidate))) return candidate;
  }
  return `${stem}-${Date.now()}${ext}`;
}

/**
 * The session of a profile. `hooks` gives the shell's side: `window()` for dialogs, `answers`, `downloadDir(webContents)`
 * and `onDownload(item, info)`.
 */
export function sessionFor(profile, hooks) {
  if (ready.has(profile)) return ready.get(profile);
  const ses = session.fromPartition(partitionOf(profile));
  ses.setUserAgent(cleanAgent(ses.getUserAgent()));
  ses.setPermissionCheckHandler((_wc, permission, origin, details) => allowedNow(hooks.answers, profile, permission, origin || details?.requestingUrl || "", details?.mediaType ? [details.mediaType] : details?.mediaTypes));
  ses.setPermissionRequestHandler((wc, permission, done, details) => {
    const needs = verdict(permission, details?.mediaTypes);
    if (needs === "grant") return done(true);
    const site = siteOf(details?.requestingUrl || wc?.getURL() || "");
    if (needs === "deny" || !site) return done(false);
    const open = needs.filter((thing) => hooks.answers.get(profile, site, thing) === null);
    if (needs.some((thing) => hooks.answers.get(profile, site, thing) === false)) return done(false);
    if (!open.length) return done(true);
    // A native dialog: the person answers it, and nothing that drives the page can.
    const words = open.map((thing) => WORDS[thing] || thing).join(" and ");
    dialog
      .showMessageBox(hooks.window(), { type: "question", message: `Allow ${site} to ${words}?`, detail: "Your answer is remembered for this site in this profile.", buttons: ["Allow", "Don't allow"], defaultId: 1, cancelId: 1 })
      .then(({ response }) => {
        for (const thing of open) hooks.answers.set(profile, site, thing, response === 0);
        done(response === 0);
      })
      .catch(() => done(false));
  });
  // Sharing the screen is never offered: a page that asks gets no source.
  ses.setDisplayMediaRequestHandler((_request, done) => done({}));
  ses.on("will-download", (_event, item, wc) => {
    const dir = hooks.downloadDir(wc);
    fs.mkdirSync(dir, { recursive: true });
    const file = path.join(dir, freeName(dir, item.getFilename()));
    item.setSavePath(file);
    hooks.onDownload(item, { file, profile, webContents: wc });
  });
  ready.set(profile, ses);
  return ses;
}

/** Wipes a profile's data and forgets its session. */
export async function clearProfile(profile) {
  const ses = session.fromPartition(partitionOf(profile));
  await ses.clearStorageData();
  await ses.clearCache();
  ready.delete(profile);
}

/** Wipes a profile's cookies only. */
export async function clearCookies(profile) {
  await session.fromPartition(partitionOf(profile)).clearStorageData({ storages: ["cookies"] });
}
