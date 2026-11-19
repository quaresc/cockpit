// Browser profiles: each one a separate store of cookies, local storage and cache, and which one a worktree's tabs use.
// The owner rules are ported from Orca (https://github.com/stablyai/orca, commit 49498c3),
// src/shared/browser/profiles/browser-profile-scope.ts. MIT licence, Copyright (c) 2026 Lovecast Inc.

import fs from "node:fs";
import path from "node:path";
import { keyOf } from "./paths.js";

export const DEFAULT = "default";

/** The registry before anyone made a profile: the one every worktree shares. */
export const emptyRegistry = () => ({ profiles: [{ id: DEFAULT, name: "Default", owner: null, created: 0 }] });

/** The Electron session partition that holds a profile's data. */
export const partitionOf = (id) => `persist:cockpit-${id}`;

/** Who a managed profile belongs to under a scope: a repo, a worktree, or nobody when every worktree shares one. */
export function ownerFor(scope, { root, repo }) {
  if (scope === "repo" && (repo || root)) return `repo:${repo || root}`;
  if (scope === "worktree" && root) return `worktree:${root}`;
  return null;
}

/** A registry read from disk, with the default profile always there. */
export function read(file) {
  let json = null;
  try {
    json = JSON.parse(fs.readFileSync(file, "utf8"));
  } catch {}
  const listed = Array.isArray(json?.profiles) ? json.profiles.filter((p) => p && typeof p.id === "string" && /^[a-z0-9-]{1,40}$/.test(p.id)) : [];
  const profiles = listed.map((p) => ({ id: p.id, name: String(p.name || p.id).slice(0, 60), owner: typeof p.owner === "string" ? p.owner : null, created: Number(p.created) || 0 }));
  return profiles.some((p) => p.id === DEFAULT) ? { profiles } : { profiles: [...emptyRegistry().profiles, ...profiles] };
}

export function write(file, registry) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const tmp = `${file}.${process.pid}.tmp`;
  fs.writeFileSync(tmp, JSON.stringify(registry, null, 2));
  fs.renameSync(tmp, file);
}

/** The registry with a new profile a person named, and that profile. */
export function created(registry, name, now = Date.now()) {
  const label = String(name || "").trim().slice(0, 60);
  if (!label) throw new Error("a profile needs a name");
  if (registry.profiles.some((p) => p.name.toLowerCase() === label.toLowerCase())) throw new Error(`a profile named ${label} exists already`);
  const profile = { id: `p-${keyOf(`${label}:${now}`)}`, name: label, owner: null, created: now };
  return [{ profiles: [...registry.profiles, profile] }, profile];
}

/** The registry with the profile a scope gives a worktree, made when it is the first to need it, and that profile. */
export function managed(registry, scope, place, now = Date.now()) {
  const owner = ownerFor(scope, place);
  if (!owner) return [registry, registry.profiles.find((p) => p.id === DEFAULT)];
  const found = registry.profiles.find((p) => p.owner === owner);
  if (found) return [registry, found];
  const folder = owner.slice(owner.indexOf(":") + 1);
  const profile = { id: `m-${keyOf(owner)}`, name: path.basename(folder) || folder, owner, created: now };
  return [{ profiles: [...registry.profiles, profile] }, profile];
}

export function renamed(registry, id, name) {
  const label = String(name || "").trim().slice(0, 60);
  if (!label) throw new Error("a profile needs a name");
  if (!registry.profiles.some((p) => p.id === id)) throw new Error("no such profile");
  return { profiles: registry.profiles.map((p) => (p.id === id ? { ...p, name: label } : p)) };
}

/** The registry without a profile; the default one stays. */
export function removed(registry, id) {
  if (id === DEFAULT) throw new Error("the default profile stays");
  if (!registry.profiles.some((p) => p.id === id)) throw new Error("no such profile");
  return { profiles: registry.profiles.filter((p) => p.id !== id) };
}

/** The profiles a removed worktree owned, to delete with it. */
export const ownedBy = (registry, root) => registry.profiles.filter((p) => p.owner === `worktree:${root}`);

/** The profile a worktree's tabs use: the one picked for it, else the one its scope gives it. */
export function chosen(registry, wt, scope) {
  const picked = wt.profile && registry.profiles.find((p) => p.id === wt.profile);
  return picked ? [registry, picked] : managed(registry, scope, wt);
}
