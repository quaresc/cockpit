import assert from "node:assert/strict";
import { test } from "node:test";

import { ago, chip, chips, failing, narrow, nowOf, signIn, staleAfter, state } from "../mods/cockpit/fresh.js";

const MIN = 60_000;
const NOW = Date.UTC(2026, 10, 2, 14, 0);
const utc = (ms) => new Date(ms).toISOString().slice(11, 16);
const opts = { clock: utc };

// The acme sources as the hub records them: GitHub fresh, Linear refusing its token, CI offline, the calendar stale,
// mail rate-limited.
const ACME = [
  { source: "github", label: "GitHub", at: NOW - 2 * MIN, every: MIN, next: NOW + MIN, error: null, login: "gh auth login" },
  { source: "tickets", label: "Linear", at: NOW - 30 * MIN, every: MIN, error: { kind: "auth", message: "HTTP 401: token refused", since: NOW - 25 * MIN }, login: "cockpit connector login Linear" },
  { source: "ci", label: "CI", at: NOW - 60 * MIN, every: 30_000, error: { kind: "network", message: "unreachable: connection refused", since: NOW - 56 * MIN } },
  { source: "calendar", label: "Calendar", at: NOW - 120 * MIN, every: 5 * MIN, error: null },
  { source: "mail", label: "Mail", at: NOW - 3 * MIN, every: MIN, next: NOW + 30 * MIN, error: { kind: "rate", message: "rate limited", since: NOW - 2 * MIN } },
];

test("each state reads as its own chip, with its fix", () => {
  const all = chips(ACME, NOW, opts);
  assert.deepEqual(all.map((c) => c.text), ["GitHub · 2 min ago", "Linear · sign in again", "CI · offline since 13:04", "Calendar · stale, 2 h ago", "Mail · limited until 14:30"]);
  assert.deepEqual(all.map((c) => c.tone), ["quiet", "bad", "bad", "warn", "bad"]);
  assert.deepEqual(all.map((c) => c.action?.kind ?? null), [null, "sign_in", "refresh", "refresh", "refresh"]);
  assert.equal(all[1].action.label, "Sign in again: cockpit connector login Linear");
  assert.equal(all[1].action.key, "L");
  assert.equal(all[1].tooltip, "HTTP 401: token refused · since 13:35 · last read 30 min ago");
  assert.equal(all[4].action.label, "Read again at 14:30, when the limit resets");
  assert.match(all[3].tooltip, /^Not read again when due \(every 5 min\)/);
  assert.equal(all[0].tooltip, "last read 2 min ago · next read 14:01");
});

test("the other failures and the never-read source", () => {
  const at = (kind) => chip({ source: "x", label: "X", at: NOW - MIN, every: MIN, error: { kind, message: "m", since: NOW - 10 * MIN } }, NOW, opts).words;
  assert.deepEqual(["not_found", "server", "other"].map(at), ["not found since 13:50", "server error since 13:50", "failing since 13:50"]);
  const never = chip({ source: "monitor", label: "Monitor", at: null, every: MIN, error: null }, NOW, opts);
  assert.deepEqual([never.text, never.tone, never.action], ["Monitor · not read yet", "quiet", null]);
});

test("a token from the environment is said as it is, not offered as a sign-in", () => {
  const said = "cockpit reads GitHub with GH_TOKEN: replace that token where it is set, then refresh";
  const c = chip({ source: "github", label: "GitHub", at: NOW - MIN, every: MIN, error: { kind: "auth", message: "HTTP 401", since: NOW - MIN }, login: said }, NOW, opts);
  assert.deepEqual([c.action.kind, c.action.label, c.action.key], ["refresh", "Cockpit reads GitHub with GH_TOKEN: replace that token where it is set, then refresh", "R"]);
});

test("stale means well past the next read, and recovery is quiet again", () => {
  // Due a minute after it was read, then five minutes of grace: two intervals are less.
  const s = { source: "github", label: "GitHub", at: NOW - 6 * MIN, every: MIN, error: null };
  assert.equal(staleAfter(s), NOW);
  assert.equal(state(s, NOW), "fresh");
  assert.equal(state(s, NOW + 1), "stale");
  assert.equal(state({ ...s, next: NOW + MIN }, NOW + 1), "fresh", "a later next read moves it on");
  assert.equal(staleAfter({ at: 0, every: 60 * MIN }), 180 * MIN, "two intervals past due");
  assert.equal(state({ ...s, error: { kind: "auth" } }, NOW), "failing");
  assert.equal(state({ ...s, at: null }, NOW), "never");
  const back = chip({ ...ACME[2], at: NOW - 5_000, error: null }, NOW, opts);
  assert.deepEqual([back.text, back.tone, back.action], ["CI · just now", "quiet", null]);
});

test("ages read like a person would say them", () => {
  assert.deepEqual([ago(0, 10_000), ago(0, 4 * MIN), ago(0, 2 * 3_600_000), ago(0, 3 * 86_400_000)], ["just now", "4 min ago", "2 h ago", "3 d ago"]);
});

test("Home names the first failing source; a narrow header keeps the loud chips", () => {
  assert.deepEqual(failing(ACME, NOW, opts), { text: "Linear · sign in again", words: "sign in again", label: "Linear", tone: "destructive" });
  assert.equal(failing([ACME[4]], NOW, opts).text, "Mail · limited until 14:30");
  assert.equal(failing([ACME[0], ACME[3]], NOW, opts), null, "stale is not failing");
  assert.equal(failing(undefined, NOW), null);
  assert.equal(signIn(ACME, NOW).id, "tickets");
  assert.equal(signIn([ACME[0]], NOW), null);
  const all = chips(ACME, NOW, opts);
  assert.deepEqual(narrow(all).map((c) => c.id), ["tickets", "ci", "calendar", "mail"]);
  assert.deepEqual(narrow(chips([ACME[0]], NOW, opts)).map((c) => c.id), ["github"]);
});

test("data a Claude session brought in after a failed read is not old", () => {
  const linear = { ...ACME[1], elsewhere: NOW - 2 * MIN };
  const c = chip(linear, NOW, opts);
  assert.deepEqual([c.text, c.tone, c.action], ["Linear · 2 min ago", "quiet", null]);
  assert.match(c.tooltip, /^Came in through a Claude session at 13:58; the hub's own read fails: HTTP 401/);
  assert.equal(failing([linear], NOW, opts), null, "Home names no failing source for it");
  assert.equal(chip({ ...ACME[1], elsewhere: NOW - 40 * MIN }, NOW, opts).text, "Linear · sign in again", "older than the failure, it says nothing");
});

test("a remote desk says how fresh, by the hub's clock, and offers no fix", () => {
  const c = chip(ACME[1], NOW, { ...opts, readOnly: true });
  assert.deepEqual([c.text, c.action], ["Linear · sign in again", null]);
  const sources = [{ ...ACME[0], now: NOW }];
  assert.equal(nowOf(sources, 1_000, 61_000), NOW + MIN, "the build's time, moved on by how long this desk has shown it");
  assert.equal(nowOf([ACME[0]], 1_000, 61_000), 61_000, "without it, this desk's clock");
});
