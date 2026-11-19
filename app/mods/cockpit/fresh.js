// How fresh each source a view reads is, worded for its chip at drawing time, so an age keeps moving while nothing new
// arrives: its age, or what went wrong and what fixes it, from the hub's record ({source, label, at, next, every, error}).

/** Sources are read one after another: a read is late only two intervals, five minutes at least, past when it was due. */
const GRACE_MIN = 5 * 60_000;

/** When data turns stale, as `cockpit_core::fresh::stale_after` says it. */
export const staleAfter = (s) => Math.max(s.next ?? 0, s.at + s.every) + Math.max(2 * s.every, GRACE_MIN);

/** never | fresh | stale | failing, as `cockpit_core::fresh::Freshness::state` says it. */
export function state(s, now) {
  if (s?.error) return "failing";
  if (!s?.at) return "never";
  if (s.every && now > staleAfter(s)) return "stale";
  return "fresh";
}

/** "just now", "4 min ago", "2 h ago", "3 d ago", as `fresh::ago` says it. */
export function ago(at, now) {
  const secs = Math.floor(Math.max(0, now - at) / 1000);
  if (secs < 45) return "just now";
  if (secs < 3600) return `${Math.floor((secs + 30) / 60)} min ago`;
  if (secs < 86_400) return `${Math.floor(secs / 3600)} h ago`;
  return `${Math.floor(secs / 86_400)} d ago`;
}

/** HH:MM, local. */
export const clock = (ms) => {
  const d = new Date(ms);
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
};

const span = (ms) => (ms < 60_000 ? `${Math.round(ms / 1000)} s` : ms < 3_600_000 ? `${Math.round(ms / 60_000)} min` : `${Math.round(ms / 3_600_000)} h`);

/**
 * One source's chip: `{id, label, text, words, tone: quiet | warn | bad, state, action: {kind: sign_in | refresh, label,
 * key} | null, tooltip}`. `clock` writes times (HH:MM); tests pass their own.
 */
export function chip(s, now, { clock: hm = clock, readOnly = false } = {}) {
  const label = s.label || s.source;
  const st = state(s, now);
  const read = s.at ? `last read ${ago(s.at, now)}` : "never read";
  let words, tone = "quiet", action = null, tooltip = read;
  if (st === "failing" && s.elsewhere && s.elsewhere > (s.error.since ?? 0)) {
    // A Claude session brought the data in after the hub's own read failed: not old, only read another way.
    words = ago(s.elsewhere, now);
    tooltip = `Came in through a Claude session at ${hm(s.elsewhere)}; the hub's own read fails: ${s.error.message}`;
  } else if (st === "failing") {
    const since = hm(s.error.since ?? now);
    const kind = s.error.kind;
    tone = "bad";
    words =
      kind === "auth" ? "sign in again"
      : kind === "rate" ? (s.next ? `limited until ${hm(s.next)}` : "rate limited")
      : kind === "network" ? `offline since ${since}`
      : kind === "not_found" ? `not found since ${since}`
      : kind === "server" ? `server error since ${since}`
      : `failing since ${since}`;
    // A login that is not a command (a token from the environment) is said as it is, and read again once done.
    const said = s.login && s.login.includes(": ");
    action =
      kind === "auth" && said ? { kind: "refresh", label: `${s.login[0].toUpperCase()}${s.login.slice(1)}`, key: "R" }
      : kind === "auth" ? { kind: "sign_in", label: s.login ? `Sign in again: ${s.login}` : "Sign in again", key: "L" }
      : kind === "rate" ? { kind: "refresh", label: s.next ? `Read again at ${hm(s.next)}, when the limit resets` : "Read again once the limit resets", key: "R" }
      : { kind: "refresh", label: "Read it again now", key: "R" };
    tooltip = `${s.error.message} · since ${since} · ${read}`;
  } else if (st === "stale") {
    tone = "warn";
    words = `stale, ${ago(s.at, now)}`;
    action = { kind: "refresh", label: "Read it again now", key: "R" };
    tooltip = `Not read again when due${s.every ? ` (every ${span(s.every)})` : ""} · ${read}`;
  } else if (st === "never") {
    words = "not read yet";
  } else {
    words = ago(s.at, now);
    if (s.next && s.next > now) tooltip = `${read} · next read ${hm(s.next)}`;
  }
  // A remote desk is read-only: its chips say, they do not act.
  return { id: s.source, label, text: `${label} · ${words}`, words, tone, state: st, action: readOnly ? null : action, tooltip };
}

/**
 * The time to word ages by: the build's own clock (`now` in each record) moved on by how long this desk has shown it,
 * so a desk whose clock differs from the hub's says them as the hub would. `seen` is when this desk first drew them.
 */
export const nowOf = (sources, seen, now) => (sources?.[0]?.now ? sources[0].now + Math.max(0, now - seen) : now);

/** Every source's chip, in the order the view model gives them. */
export const chips = (sources, now, opts) => (sources ?? []).map((s) => chip(s, now, opts));

/** The first failing source of a view, as Home's Apps card names it: `{text, words, label, tone}` or null. */
export function failing(sources, now, opts) {
  const c = chips(sources, now, opts).find((c) => c.state === "failing" && c.tone !== "quiet");
  return c ? { text: c.text, words: c.words, label: c.label, tone: "destructive" } : null;
}

/** The chip a sign-in key acts on: the first whose login was refused. */
export const signIn = (sources, now) => chips(sources, now).find((c) => c.action?.kind === "sign_in") ?? null;

/** The chips a narrow header keeps: the loud ones, else the first. */
export function narrow(list) {
  const loud = list.filter((c) => c.tone !== "quiet");
  return loud.length ? loud : list.slice(0, 1);
}
