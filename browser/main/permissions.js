// What a page may do without asking, what it must ask the person for, and what it never gets. The policy is ported from
// Orca (https://github.com/stablyai/orca, commit 49498c3),
// src/main/browser/site-settings/permissions/browser-session-permission-policy.ts. MIT licence, Copyright (c) 2026 Lovecast Inc.

import fs from "node:fs";
import path from "node:path";

const GRANTED = new Set(["fullscreen", "clipboard-sanitized-write", "persistent-storage", "pointerLock", "storage-access"]);

const ASKED = {
  geolocation: "location",
  notifications: "notifications",
  "clipboard-read": "clipboard",
  midi: "midi",
  midiSysex: "midi",
  "window-management": "window-management",
  "idle-detection": "idle-detection",
  "local-network-access": "local-network",
  "local-network": "local-network",
  "loopback-network": "local-network",
};

const MEDIA = { video: "camera", audio: "microphone" };
const ORDER = ["camera", "microphone"];

/** How each thing a site asks for is said to the person. */
export const WORDS = {
  location: "know your location",
  notifications: "show notifications",
  clipboard: "read your clipboard",
  midi: "use your MIDI devices",
  "window-management": "place windows on your screens",
  "idle-detection": "know when you are away",
  "local-network": "reach devices on your network",
  camera: "use your camera",
  microphone: "use your microphone",
};

/** What a request needs: "grant", "deny", or the list of things to ask the person about. */
export function verdict(permission, mediaTypes) {
  if (GRANTED.has(permission)) return "grant";
  if (permission === "media") {
    const wanted = new Set((mediaTypes || []).map((type) => MEDIA[type]));
    const asked = ORDER.filter((name) => wanted.has(name));
    return asked.length ? asked : "deny";
  }
  return ASKED[permission] ? [ASKED[permission]] : "deny";
}

/** The site a request comes from, as it is remembered: scheme, host and port. */
export function siteOf(url) {
  try {
    const { protocol, host } = new URL(url);
    return /^https?:$/.test(protocol) ? `${protocol}//${host}` : null;
  } catch {
    return null;
  }
}

/** The person's answers, kept per profile and site: {profile: {site: {thing: true|false}}}. */
export class Answers {
  constructor(file) {
    this.file = file;
    try {
      this.all = JSON.parse(fs.readFileSync(file, "utf8")) || {};
    } catch {
      this.all = {};
    }
  }

  get(profile, site, thing) {
    const answer = this.all[profile]?.[site]?.[thing];
    return typeof answer === "boolean" ? answer : null;
  }

  set(profile, site, thing, allowed) {
    ((this.all[profile] ||= {})[site] ||= {})[thing] = allowed;
    fs.mkdirSync(path.dirname(this.file), { recursive: true });
    fs.writeFileSync(this.file, JSON.stringify(this.all, null, 2));
  }

  forget(profile) {
    delete this.all[profile];
    try {
      fs.writeFileSync(this.file, JSON.stringify(this.all, null, 2));
    } catch {}
  }
}

/** Whether a check (not a request) passes: granted outright, or already answered yes by the person. */
export function allowedNow(answers, profile, permission, url, mediaTypes) {
  const needs = verdict(permission, mediaTypes);
  if (needs === "grant") return true;
  if (needs === "deny") return false;
  const site = siteOf(url);
  return !!site && needs.every((thing) => answers.get(profile, site, thing) === true);
}
