// Which tabs out of view are put to sleep, and when: a sleeping tab keeps its address, title and scroll, and loads again
// when it is shown. The reasons a tab stays awake follow Orca's memory saver (https://github.com/stablyai/orca,
// commit 49498c3, src/main/browser/guest-lifecycle/browser-guest-discard-protection.ts; MIT, Copyright (c) 2026 Lovecast Inc.).

/** Why a tab out of view stays awake, or null when it may sleep. */
export function keptAwake(tab) {
  if (tab.audible) return "audio";
  if (tab.devtools) return "devtools";
  if (tab.edited) return "edits";
  if (tab.capturing) return "capture";
  if (tab.loading) return "loading";
  if (tab.agentAt && tab.now - tab.agentAt < 60_000) return "agent";
  return null;
}

/** The tabs to put to sleep now: awake, out of view for longer than the delay, and with nothing keeping them. */
export function toSleep(tabs, now, idleMs) {
  if (!(idleMs > 0)) return [];
  return tabs.filter((t) => !t.asleep && !t.shown && now - (t.hiddenAt || now) >= idleMs && !keptAwake({ ...t, now })).map((t) => t.id);
}

/** Minutes from config.json as a delay; 0 keeps every tab awake. */
export const idleMsOf = (minutes) => (Number(minutes) > 0 ? Math.min(Number(minutes), 10_080) * 60_000 : 0);
