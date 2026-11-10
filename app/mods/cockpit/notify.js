// The hub's notifications in the Herdr app: Herdr's own toast while the window is in front, its click bringing the row
// the notification is about into view (where its other buttons are); the system's notification while it is behind,
// whose click and buttons lead to the same row through `cockpit focus herdr-app:<target>`.

import { notify, window as win } from "herdr";
import * as hub from "./hub.js";

const LEVEL = { "🔴": "error", "❌": "error", "⚠️": "warning", "✅": "success", "🟢": "success", "🚀": "success" };

const WHERE = { inbox: "the inbox", runbook: "the runbook", meeting: "Meetings", monitor: "the monitor", routine: "Routines", space: "cockpit" };

/** A notification from the hub, {title, body, action, row, buttons}: shown as fits the window. */
export function deliver(n, install, cx) {
  const always = Boolean(install?.machine) || install?.env?.COCKPIT_TOASTS === "always";
  if (!win().active && !always) {
    cx.spawn(async () => hub.cockpit(["notify", "--system", n.title ?? "", n.body ?? "", n.action ?? "", ...(n.buttons ?? [])]));
    return;
  }
  const level = Object.entries(LEVEL).find(([mark]) => `${n.title ?? ""} ${n.body ?? ""}`.includes(mark))?.[1] ?? "info";
  const target = n.row || n.action || "";
  const labels = (n.buttons ?? []).map((b) => b.split("=")[0]).filter(Boolean);
  const where = WHERE[target.split(":")[0]];
  // The toast has one action: the other buttons wait on the row it opens, and say so.
  const more = labels.length && where ? `\n${labels.join(" · ")} in ${where}` : "";
  notify({
    title: n.title ?? "",
    body: `${n.body ?? ""}${more}`,
    level,
    icon: "mods/cockpit/cockpit.svg",
    timeout: level === "error" ? 12000 : 6000,
    ...(target ? { action: { name: `cockpit.target:${target}` } } : {}),
  });
}

/** What a toast's click asks: a URL opens, the rest goes through `cockpit focus` (rows, spaces, tabs, buttons). */
export function follow(target, cx) {
  if (/^https?:\/\//.test(target)) cx.spawn(async () => hub.openUrl(target));
  else cx.spawn(async () => hub.cockpit(["focus", target]));
}
