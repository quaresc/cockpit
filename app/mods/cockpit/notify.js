// The hub's notifications in the Herdr app: Herdr's own toast while the window is in front, its click bringing the row
// the notification is about into view (where its other buttons are); the system's notification while it is behind,
// whose click and buttons lead to the same row through `cockpit focus herdr-app:<target>`.

import { notify, window as win } from "herdr";
import * as hub from "./hub.js";
import { openUpdate } from "./update.js";

const LEVEL = { "🔴": "error", "❌": "error", "⚠️": "warning", "✅": "success", "🟢": "success", "🚀": "success" };

const WHERE = { inbox: "the inbox", runbook: "the runbook", meeting: "Meetings", monitor: "the monitor", routine: "Routines", space: "cockpit" };

/**
 * A notification from the hub, {title, body, action, row, buttons, from}: shown as fits the window. One a client sent
 * (`from` names who) is drawn plain, never with cockpit's icon or a success or failure colour its words may claim.
 */
export function deliver(n, install, cx) {
  const always = Boolean(install?.machine) || install?.env?.COCKPIT_TOASTS === "always";
  if (!win().active && !always) {
    cx.spawn(async () => hub.cockpit(["notify", "--system", n.title ?? "", n.body ?? "", n.action ?? "", ...(n.buttons ?? [])]));
    return;
  }
  const level = n.from ? "info" : (Object.entries(LEVEL).find(([mark]) => `${n.title ?? ""} ${n.body ?? ""}`.includes(mark))?.[1] ?? "info");
  const target = n.row || n.action || "";
  const labels = (n.buttons ?? []).map((b) => b.split("=")[0]).filter(Boolean);
  const where = WHERE[target.split(":")[0]];
  // The toast has one action: the other buttons wait on the row it opens, and say so.
  const more = labels.length && where ? `\n${labels.join(" · ")} in ${where}` : "";
  notify({
    title: n.title ?? "",
    body: `${n.body ?? ""}${more}`,
    level,
    ...(n.from ? {} : { icon: "mods/cockpit/cockpit.svg" }),
    timeout: level === "error" ? 12000 : 6000,
    ...(target ? { action: { name: `cockpit.target:${target}` } } : {}),
  });
}

/** "https://github.com/acme/api/pull/4" out of that PR's URL or one of its pages, else the reference as given. */
const prOf = (ref) => /^(https?:\/\/[^\s]*?\/pull\/\d+)/.exec(ref)?.[1] ?? ref;

/**
 * The row a notification's button is about, where the person acts: `act:<review|ci|rerun|merge>:<PR>` is the PR's row,
 * `act:plan:<ticket>` the ticket's and `act:routine:<slug>` the routine; any other target is itself.
 */
export function rowOf(target) {
  const found = /^(?:herdr-app:)?act:([a-z]+):(.+)$/.exec(target);
  if (!found) return target;
  const [, kind, ref] = found;
  if (kind === "routine") return `routine:${ref}`;
  if (kind === "plan") return `inbox:${ref.toUpperCase()}`;
  return `inbox:${prOf(ref)}`;
}

/**
 * What a toast's click asks, which only ever navigates: a URL opens, the update its dialog, the rest goes through
 * `cockpit focus` (rows, spaces, tabs), a button's action as the row it is about.
 */
export function follow(target, cx) {
  if (/^https?:\/\//.test(target)) cx.spawn(async () => hub.openUrl(target));
  else if (target === "update") openUpdate();
  // A remote desk refuses what needs its own terminals: the click says so instead of going nowhere.
  else cx.spawn(async () => hub.cockpit(["focus", rowOf(target)]).catch((e) => notify({ title: "Could not follow it", body: String(e?.message ?? e), level: "warning", timeout: 5000 })));
}
