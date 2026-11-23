// The cockpit hub over TCP: its address and token from hub.json, the hub started when none answers, one request per
// connection, and subscriptions that come back by themselves when the hub restarts or a copy of the window goes.

import net from "net";
import { exec, appHome, readTextFile, run, window as win } from "herdr";
import * as herdr from "herdr";

/** What `herdr-app check` gets in place of the hub: a check loads the mod for real, and must start or reach nothing. */
const CHECKING = "herdr-app check is loading the mod: it starts and asks nothing";

/** Whether `herdr-app check` loads the mod now; a Herdr older than 0.1.16 never says so. */
export const checking = () => Boolean(herdr.checking?.());

/** Closes a socket, already closed or not: a throw here would end the subscription that is tidying up. */
const close = (socket) => {
  try {
    socket?.close();
  } catch {}
  owned().delete(socket);
};

/**
 * This evaluation of hub.js. Herdr can evaluate the mod once per copy, or evaluate a build it never runs, so a newer
 * load says nothing about the older ones: a load counts as gone only once it stops showing signs of life.
 */
export const LOAD = (globalThis.cockpitHubLoads = (globalThis.cockpitHubLoads ?? 0) + 1);

/** When each load last showed it runs (a beat, a request, a subscription), on globalThis which every load shares. */
const signs = () => (globalThis.cockpitHubSigns ??= new Map());

/** This load runs: a copy of it beats or asks the hub. */
const sign = () => signs().set(LOAD, Date.now());

/** Whether a load showed a sign of life within `GONE_MS` (this load always does). */
export const liveLoad = (load, now = Date.now()) => load === LOAD || now - (signs().get(load) ?? 0) <= GONE_MS;

/** Every socket open in the window, with the load that opened it and when. */
const owned = () => (globalThis.cockpitHubOwned ??= new Map());

/** A socket this load opened, kept so that another load can close it once this one is gone. */
function adopt(socket) {
  owned().set(socket, { load: LOAD, at: Date.now() });
  return socket;
}

/** How long a socket of a gone load may stay open: past every bound of a request (20 s to connect, 60 s to answer). */
const STRAY_MS = 90_000;

/**
 * Closes what loads that stopped running left open, `now` being a time this load runs: their sockets once past every
 * bound of a request, and their focus listeners. Their subscriptions are the watchdog's to park.
 */
export function sweep(now = Date.now()) {
  sign();
  for (const [socket, { load, at }] of owned()) {
    if (!liveLoad(load, now) && now - at > STRAY_MS) close(socket);
  }
  for (const [load, at] of signs()) if (now - at > FORGOTTEN_MS && load !== LOAD) signs().delete(load);
  const focus = globalThis.cockpitRemoteFocus;
  for (const listener of [...(focus?.listeners ?? [])]) if (!liveLoad(listener.load, now)) focus.listeners.delete(listener);
}

/**
 * The promise, or a rejection after `ms` on the context's timer: a hub that never answers, or a socket whose other end
 * went without the read ever ending, is not waited on forever.
 */
export function within(cx, promise, ms, what) {
  if (!cx) return promise;
  promise.catch(() => {});
  let timer;
  const late = new Promise((_, reject) => {
    timer = cx.timer.after(ms, () => reject(new Error(`${what} did not answer`)));
  });
  return Promise.race([promise, late]).finally(() => timer?.cancel());
}

/**
 * How often a subscription promises the hub a line, in seconds. A copy of the mod that Herdr reloads away keeps its
 * sockets open but runs no more: the hub ends a subscription that breaks the promise three times over.
 */
const KEEPALIVE_SECS = 30;

/** Waits `ms` on the context's timer (mods have no setTimeout). */
export const sleep = (cx, ms) => new Promise((resolve) => cx.timer.after(ms, () => resolve()));

const NOT_INSTALLED = "cockpit is not installed in this Herdr home";
const READ_ONLY = "the remote cockpit client is read-only; this action needs a local cockpit installation";
const REMOTE_READS = new Set(["ping", "view", "search", "fetch", "events", "history", "shown"]);
const APPS = new Set(["inbox", "todo", "runbook", "meetings", "mail", "monitor", "routines", "show"]);
/** The actions a remote desk runs on the runner with `cockpit hub act <app>`, the hub's TCP gate untouched. */
const RUNNER_ACTS = {
  review: new Set(["send", "walkthrough", "stop_walkthrough"]),
  runbook: new Set(["done", "set", "add", "undo", "run_step"]),
};
/** The app behind a pinned space's name, as `cockpit focus space:<name>` gives it. */
const SPACES = { inbox: "inbox", runbook: "runbook", "to-do": "todo", todo: "todo", meetings: "meetings", monitor: "monitor", routines: "routines", mail: "mail" };

/** Brings an app into view on this desktop, without a row to select. */
function localShow(app) {
  if (!APPS.has(app)) throw new Error(READ_ONLY);
  run(`cockpit.go:${app}`);
  return { ok: true };
}

function localFocus(target) {
  if (!APPS.has(target?.app)) throw new Error(READ_ONLY);
  const focus = (globalThis.cockpitRemoteFocus ??= { targets: new Map(), listeners: new Set() });
  focus.targets.set(target.app, target);
  if (target.app === "show") {
    run("cockpit.go:work");
    run("layout.select", { stack: "panel-body", child: "show" });
    run("layout.show", { id: "panel" });
  } else run(`cockpit.go:${target.app}`);
  for (const listener of focus.listeners) if (liveLoad(listener.load) && listener(target)) break;
  return { ok: true };
}

/** Whether an error (or a subscription's "error: …" state) says this Herdr home has no cockpit.json. */
export const isNotInstalled = (e) => String(e?.message ?? e ?? "").includes(NOT_INSTALLED);

/** Opens a URL or file in the system's default app: `open` on macOS, `start` on Windows, `xdg-open` elsewhere. */
export function openUrl(url) {
  const platform = win()?.platform;
  if (platform === "macos") return exec("open", [url]);
  if (platform === "windows") return exec("cmd", ["/c", "start", "", url]);
  return exec("xdg-open", [url]);
}

/** Where cockpit lives, as `cockpit app install` wrote it: { bin, cache, home, version }. Read again every few seconds,
 *  so an install or update reaches a window that stays open. */
let installed = null;

/** The machine of a remote desk, as the last read of the install says; null on this machine or before any read. */
export const remoteMachine = () => installed?.value?.machine ?? null;
export async function install() {
  if (installed && Date.now() - installed.at < 3000) return installed.value;
  const text = await readTextFile(`${appHome()}/cockpit.json`);
  if (!text) throw new Error(`${NOT_INSTALLED}: run \`cockpit app install\`, or install cockpit first: github.com/quaresc/cockpit#install`);
  const value = installed?.text === text ? installed.value : JSON.parse(text);
  if (value.machine !== undefined && (typeof value.machine !== "string" || !value.machine.trim() || typeof value.host !== "string" || !value.host.trim())) {
    throw new Error("a remote cockpit install needs a machine and host");
  }
  installed = { at: Date.now(), value, text };
  return installed.value;
}

/** hub.json as written; a read the window never answers (a copy made in the middle of a layout switch) gives up. */
async function readInfo(cx) {
  const { cache, machine } = await install();
  const text = await within(cx, readTextFile(`${cache}/hub.json`, machine ? { on: machine } : undefined), 10_000, "hub.json");
  return text ? JSON.parse(text) : null;
}

// One `cockpit hub start` at a time for every view, and after a failure (the binary gone, a hub that won't start) the
// next try waits 30 s: until then every caller gets the same error. Kept on globalThis, which every copy shares; a start
// left behind by a copy that went (its promise never settles) is given up after 40 s.
const HUB_STOPPED = "the cockpit hub was stopped on purpose";

/** Whether the hub was stopped with `cockpit hub stop`: no window starts it again by itself until Start it. */
async function stoppedOnPurpose(cx) {
  const { cache, machine } = await install();
  if (machine) return false;
  return (await within(cx, readTextFile(`${cache}/hub.stopped`).catch(() => null), 10_000, "hub.stopped")) !== null;
}

/**
 * Starts the hub, or joins the start already under way. Only `explicit` (Start it) starts one that was stopped on
 * purpose, or a remote runner's: a window never restarts those by itself.
 */
export async function startHub({ explicit = false, cx = null } = {}) {
  if (checking()) throw new Error(CHECKING);
  const s = (globalThis.cockpitHubStart ??= { starting: null, failed: null, at: 0 });
  if (!explicit) {
    // A runner's hub is found again (its port and token) but never started from a desk by itself.
    if ((await install()).machine && (await cockpit(["hub", "ping"])).code !== 0) throw new Error("the runner's cockpit hub is not running: start it there, or with Start it");
    if (await stoppedOnPurpose(cx)) throw new Error(HUB_STOPPED);
    if (s.failed && Date.now() - s.failed.at < 30_000) throw s.failed.error;
  }
  if (s.starting && Date.now() - (s.at ?? 0) > 40_000) s.starting = null;
  if (s.starting) return s.starting;
  s.at = Date.now();
  const starting = (async () => {
    try {
      // The hub runs on the install's HOME and drives this Herdr window, whatever the app's own environment says.
      const r = await cockpit(["hub", "start"]);
      if (r.code !== 0) throw new Error(r.stderr.trim() || "the cockpit hub did not start");
      s.failed = null;
      return JSON.parse(r.stdout);
    } catch (e) {
      s.failed = { at: Date.now(), error: e instanceof Error ? e : new Error(String(e)) };
      throw s.failed.error;
    } finally {
      // A start given up and begun again since is not this one's to clear.
      if (s.starting === starting) s.starting = null;
    }
  })();
  s.starting = starting;
  return starting;
}

/** Whether an error says the hub was stopped on purpose, which only Start it undoes. */
export const isStoppedOnPurpose = (e) => String(e?.message ?? e ?? "").includes(HUB_STOPPED);

/** Why this window last failed to reach the hub, and when: kept on globalThis, which every copy shares. */
const failure = () => (globalThis.cockpitHubFailure ??= { message: null, at: 0 });

/** Keeps an error that stopped the window from reaching the hub; one of the window's own making says nothing of the hub. */
function failed(e) {
  const message = String(e?.message ?? e ?? "").trim();
  if (!message || message === "subscription stopped") return;
  Object.assign(failure(), { message, at: Date.now() });
}

/** How long a failure to reach the hub is worth saying. */
const FAILURE_MS = 10 * 60_000;

/** The window reached the hub: what failed before is no longer why. */
function reached() {
  Object.assign(failure(), { message: null, at: 0 });
}

/** The last failure to reach the hub, `{message, at}`, while it is recent and nothing reached the hub since; else null. */
export const lastFailure = (now = Date.now()) => (failure().message && now - failure().at < FAILURE_MS ? { ...failure() } : null);

/**
 * Why the hub does not answer, in the error's own words: a subscription's "error: …" state, else the last failure the
 * window kept; "" when nothing failed.
 */
export function downReason(e) {
  const text = String(e?.message ?? e ?? "").replace(/^error:\s*/, "").trim();
  return text && text !== "connecting" && text !== "live" ? text : (lastFailure()?.message ?? "");
}

/** A hub that does not answer in one line with its reason: "The cockpit hub isn't running: connection refused". */
export function downTip(e) {
  const words = downWords(e);
  if (isStoppedOnPurpose(e)) return `${words} on purpose, with cockpit hub stop`;
  const why = downReason(e);
  return why ? `${words}: ${why}` : words;
}

/** What every surface says of a hub that does not answer, in full or `short`: stopped on purpose, else not running. */
export function downWords(e, short = false) {
  const stopped = isStoppedOnPurpose(e);
  if (short) return stopped ? "Hub stopped" : "Hub not running";
  return stopped ? "The cockpit hub was stopped" : "The cockpit hub isn't running";
}

/** How often a connection waiting on a hub start reads hub.json again. */
const APPEAR_MS = 500;

/** Whether two reads of hub.json name the same hub. */
const sameHub = (a, b) => Boolean(a && b) && a.port === b.port && a.token === b.token && a.pid === b.pid;

/**
 * hub.json's { port, token }: as written, else after `cockpit hub start` started a hub. While that start runs (one
 * shared start can take up to Herdr's 30 s bound on a command), a hub.json naming another hub than `seen` is taken at once.
 */
async function address(fresh, state, cx, seen = null) {
  if (!fresh) {
    const info = await readInfo(cx);
    if (info?.port) return info;
  }
  if (state?.stopped) throw new Error("subscription stopped");
  const starting = startHub({ cx });
  if (!cx?.timer) return starting;
  let done = false;
  starting.then(() => (done = true), () => (done = true));
  const appeared = (async () => {
    while (!done && !state?.stopped) {
      await sleep(cx, APPEAR_MS);
      if (done) break;
      const info = await readInfo(cx).catch(() => null);
      if (info?.port && !sameHub(info, seen)) return info;
    }
    return starting;
  })();
  return Promise.race([starting, appeared]);
}

// Herdr's Windows runtime has no TextDecoder and its Buffer shim returns empty buffers, so it decodes percent-encoded bytes.
const utf8 = typeof TextDecoder === "function" ? new TextDecoder() : null;
const decode = bytes => (utf8 ? utf8.decode(bytes) : decodeURIComponent(Array.from(bytes, byte => `%${byte.toString(16).padStart(2, "0")}`).join("")));

/**
 * Herdr's socket read gives up after a while with no data and rejects with the system's "would block" (macOS: os error
 * 35, Linux: 11): the connection is fine and nothing came yet.
 */
export const idleRead = (e) => /resource temporarily unavailable|would ?block|os error (35|11)\b/i.test(String(e?.message ?? e ?? ""));

/** How many "would block" reads in a row are read again without a timer to wait on, before the socket counts as broken. */
const BARE_RETRIES = 1000;

/**
 * Splits a socket's bytes into lines, decoding each whole line (a UTF-8 character may span two reads). A read that only
 * says nothing came yet is read again on the same socket, after `pause` when the caller has a timer.
 */
export class Lines {
  constructor(socket, pause = null) {
    this.socket = socket;
    this.pending = new Uint8Array(0);
    this.pause = pause;
  }

  async read() {
    for (let idle = 0; ; idle++) {
      try {
        return await this.socket.read(65536);
      } catch (e) {
        if (!idleRead(e) || (!this.pause && idle >= BARE_RETRIES)) throw e;
        if (this.pause) await this.pause();
      }
    }
  }

  async next() {
    for (;;) {
      const at = this.pending.indexOf(10);
      if (at >= 0) {
        const line = decode(this.pending.subarray(0, at));
        this.pending = this.pending.slice(at + 1);
        return line;
      }
      const chunk = await this.read();
      if (chunk === null) return null;
      const joined = new Uint8Array(this.pending.length + chunk.length);
      joined.set(this.pending, 0);
      joined.set(chunk, this.pending.length);
      this.pending = joined;
    }
  }
}

/** One connection to the hub, answering ping; `tried.info` keeps the hub.json it used, for a try after it. */
async function open(fresh, state, cx, tried = {}) {
  const { host, machine } = await install();
  const step = () => state && (state.moved = Date.now());
  step();
  const info = await address(fresh, state, cx, tried.info);
  tried.info = info;
  step();
  if (state?.stopped) throw new Error("subscription stopped");
  const connecting = net.connect(machine ? host : (info.host ?? "127.0.0.1"), info.port).then(adopt);
  let socket;
  try {
    socket = await within(cx, connecting, 10_000, "the cockpit hub");
  } catch (error) {
    // Given up on, a connection that comes later is closed rather than left open for good.
    connecting.then(close, () => {});
    throw error;
  }
  step();
  const lines = new Lines(socket, cx?.timer ? () => sleep(cx, 50) : null);
  try {
    if (state?.stopped) throw new Error("subscription stopped");
    // Still asking: stopping the subscription, or giving the task up, closes it.
    state?.opening.add(socket);
    await socket.write(JSON.stringify({ op: "ping", token: info.token }) + "\n");
    const answer = JSON.parse(await within(cx, lines.next(), 10_000, "the cockpit hub"));
    if (answer?.ok !== true) throw new Error(answer?.error ?? "the cockpit hub did not answer ping");
    return { socket, token: info.token, lines, remote: machine ?? null, port: info.port };
  } catch (error) {
    close(socket);
    throw error;
  } finally {
    state?.opening.delete(socket);
  }
}

async function connect(state, cx) {
  const tried = {};
  try {
    return await open(false, state, cx, tried);
  } catch (error) {
    if (state?.stopped) throw error;
    // hub.json named a hub that is gone: start one.
    try {
      return await open(true, state, cx, tried);
    } catch (again) {
      failed(again);
      throw again;
    }
  }
}

/** Runs the cockpit binary on the install's HOME, driving this Herdr window: { code, stdout, stderr }. */
export async function cockpit(args) {
  if (checking()) throw new Error(CHECKING);
  const { bin, home, env = {}, machine, host } = await install();
  const vars = Object.entries(env).filter(([k]) => !machine || k !== "HERDR_APP_HOME").map(([k, v]) => `${k}=${v}`);
  if (machine) {
    if (args[0] === "focus" && args.length === 2) {
      const target = args[1].replace(/^herdr-app:/, "");
      const at = target.indexOf(":");
      if (at < 1) throw new Error(READ_ONLY);
      const app = target.slice(0, at);
      const key = target.slice(at + 1);
      if (app === "space") {
        localShow(SPACES[key] ?? key);
        return { code: 0, stdout: "", stderr: "" };
      }
      const step = key.indexOf(":");
      localFocus(app === "runbook"
        ? { app, key: step < 0 ? key : key.slice(0, step), steps: step < 0 ? [] : key.slice(step + 1).split(",").filter(Boolean) }
        : { app: { meeting: "meetings", routine: "routines" }[app] ?? app, key });
      return { code: 0, stdout: "", stderr: "" };
    }
    const runner = args[1] === "act" && (args.length === 5 || args.length === 6) && RUNNER_ACTS[args[2]]?.has(args[3]);
    if (args[0] !== "hub" || !(["start", "ping", "shown"].includes(args[1]) || runner)) throw new Error(READ_ONLY);
    return exec("env", [...vars, `HOME=${home}`, `COCKPIT_HUB_LISTEN=${host}`, "COCKPIT_FRONTEND=app", bin, ...args], { on: machine });
  }
  return exec("env", [`HOME=${home}`, `HERDR_APP_HOME=${appHome()}`, "COCKPIT_FRONTEND=app", ...vars, bin, ...args]);
}

/**
 * One request: its answer, or a thrown Error with the hub's `error`. Given the caller's `cx`, connecting and the answer
 * are bounded on its timer; one sent and never answered throws with `unconfirmed` (see `isUnconfirmed`).
 */
export async function request(op, fields = {}, cx = null) {
  if (checking()) throw new Error(CHECKING);
  // A page opens from the window itself: macOS keeps the browser behind when a background process (the hub) asks.
  if (op === "act" && fields.action === "open_url" && typeof fields.args?.[0] === "string") {
    const url = fields.args[0];
    const r = await openUrl(url);
    return r.code === 0 ? { note: `opened ${url}`, pending: {}, refreshing: {} } : { error: r.stderr.trim() || `could not open ${url}`, pending: {} };
  }
  sign();
  const { machine } = await install();
  if (machine) {
    if (op === "go") return localFocus(fields.target);
    if (op === "act" && RUNNER_ACTS[fields.app]?.has(fields.action)) return runnerAct(machine, fields.app, fields.action, fields.args ?? [], fields.armed);
    if (!REMOTE_READS.has(op)) throw new Error(READ_ONLY);
  }
  const connecting = connect(null, cx);
  let connection;
  try {
    connection = await within(cx, connecting, 20_000, "the cockpit hub");
  } catch (error) {
    // Given up on, a connection that comes later is closed rather than left open for good.
    connecting.then((c) => close(c.socket), () => {});
    throw error;
  }
  const { socket, token, lines, remote } = connection;
  reached();
  let sent = false;
  try {
    await socket.write(JSON.stringify({ op, token, frontend: "app", ...fields }) + "\n");
    sent = true;
    const line = await within(cx, lines.next(), 60_000, "the cockpit hub");
    if (line === null) throw new Error("the cockpit hub closed the connection");
    const answer = JSON.parse(line);
    if (answer.error && answer.note === undefined) throw Object.assign(new Error(answer.error), { answered: true });
    if (remote && op === "view" && fields.name === "agents") answer.value = { ...answer.value, remote };
    return answer;
  } catch (e) {
    // Sent and never answered: it may have been done, so it is not said to have failed.
    if (sent && !e?.answered) throw Object.assign(e instanceof Error ? e : new Error(String(e)), { unconfirmed: true });
    throw e;
  } finally {
    close(socket);
  }
}

/** Whether a request went to the hub and no answer came back: done or not, nobody knows. */
export const isUnconfirmed = (e) => Boolean(e?.unconfirmed);

/** A review or runbook action run by the runner's own cockpit, which hands it to its hub on the unix socket. Review
 *  comments go to a runner agent only: its address here is `<machine>/t7`, `t7` there. */
async function runnerAct(machine, app, action, args, armed) {
  if (app === "review" && action === "send") {
    const terminal = String(args[0] ?? "").replace(/^app:/, "");
    const local = terminal.startsWith(`${machine}/`) ? terminal.slice(machine.length + 1) : "";
    if (!/^[A-Za-z0-9]+$/.test(local)) throw new Error("focus one of the runner's agents to send it the review");
    args = [local, ...args.slice(1)];
  }
  const r = await cockpit(["hub", "act", app, action, JSON.stringify(args), ...(armed ? [armed] : [])]);
  if (r.code !== 0) throw new Error(r.stderr.trim() || `${app} ${action} failed on ${machine}`);
  return JSON.parse(r.stdout);
}

/** An app's action: { note, pending, refreshing, error }; bounded on `cx`'s timer when one is given. */
export const act = (app, action, args = [], cx = null) => request("act", { app, action, args }, cx);

/** A view model once. */
export const view = async (name, args = null) => (await request("view", { name, args })).value;

/** The palette's search across apps. */
export const search = async (query) => (await request("search", { query })).entries ?? [];

/** Brings a found item into view, in its app. */
export const go = (target) => request("go", { target });

/** Details on demand: ci_log, failed_tests, review_threads, pr_files. */
export const fetchDetails = (kind, args) => request("fetch", { kind, args });

/** What changed for an app since `since` (epoch ms), newest first. */
export const events = async (app, since) => (await request("events", { app, since, limit: 500 })).events ?? [];

/** A sparkline's series, oldest first. */
export const history = async (kind, key) => (await request("history", { kind, key })).values ?? [];

/** The window's live subscriptions, on globalThis which every copy shares: what the watchdog looks after. */
const following = () => (globalThis.cockpitHubSubscriptions ??= new Set());

/** How often each copy beats. */
const BEAT_MS = 1000;

/** How long a subscription's copy may stay silent, behind the liveliest one, before it counts as gone. */
const GONE_MS = 10_000;

/** How long a parked subscription is kept for a copy that may still beat again. */
const FORGOTTEN_MS = 5 * 60_000;

/** Longer than any bounded step of a connection and the slowest `cockpit hub start` (two tries of 3 s and 15 s). */
export const STUCK_MS = 40_000;

/**
 * How long a subscription that never got an answer (live or an error) waits on one step: just past the 10 s bound of
 * each step, since a first task made mid-switch can hang on a read that has none while its timer still beats.
 */
export const FIRST_MS = 12_000;

/** How long the task's own timer may stay silent: past it, the task's context went (a copy made mid-switch). */
export const SILENT_MS = 3_000;

/**
 * Whether the task following the hub is to be given up for a new one: its own timer stopped beating (its context is
 * gone), or getting to the hub took longer than every bounded step together. A wait between tries is neither.
 */
export function stuck(state, now) {
  if (!state.connecting || state.parked) return false;
  return now - (state.alive ?? now) > SILENT_MS || now - state.moved > (state.answered ? STUCK_MS : FIRST_MS);
}

/**
 * The watchdog's verdict on the window's subscriptions, from a ping that gave the hub's `port` (null when it did not
 * answer): `park` those whose copy stopped beating, `close` the sockets on another hub's port. Nothing is closed on a
 * ping that failed: a slow hub is not a gone one.
 */
export function verdict(subs, port, { woke = false } = {}) {
  const live = subs.filter((s) => !s.parked && !s.stopped);
  const freshest = Math.max(0, ...live.map((s) => s.beat));
  // Just after a sleep every other copy looks behind: none is taken for gone until each has had time to beat again.
  const park = woke ? [] : live.filter((s) => freshest - s.beat > GONE_MS);
  const close = port === null ? [] : live.filter((s) => !park.includes(s) && s.socket && s.port !== null && s.port !== port);
  return { park, close };
}

/**
 * One check for the whole window, every 12 s while all is live and every 3 s otherwise: the hub answers a ping, every
 * subscription is on that hub, and its copy still beats. Two failed pings in a row start a hub, when a window may.
 */
function watchdog(cx, slept = false) {
  const w = (globalThis.cockpitHubWatchdog ??= { at: 0, busy: false, since: 0, failures: 0, woke: false });
  // A late beat (the machine slept, the window was throttled): the next check alone takes nobody for gone.
  if (slept) w.woke = true;
  const calm = w.failures === 0 && [...following()].every((s) => s.parked || !s.connecting);
  // One check at a time; one left hanging by a copy that went is given up after a while.
  if ((w.busy && Date.now() - w.since < 15_000) || Date.now() - w.at < (calm ? 12_000 : 3_000)) return;
  w.busy = true;
  w.at = w.since = Date.now();
  cx.spawn(async (cx) => {
    let port = null;
    try {
      const c = await open(false, null, cx);
      close(c.socket);
      port = c.port;
      w.failures = 0;
    } catch {
      w.failures += 1;
      if (w.failures >= 2) await startHub({ cx }).catch(() => {});
    }
    try {
      const woke = w.woke === true;
      w.woke = false;
      // Just after a sleep every load looks silent: none is taken for gone.
      if (!woke) sweep();
      const { park, close: stale } = verdict([...following()], port, { woke });
      for (const s of park) {
        // A subscription made by an older version of this module may not know how to park: it is stopped.
        try {
          (s.park ?? s.stop)?.call(s);
        } catch {}
      }
      for (const s of stale) close(s.socket);
      // Parked for minutes, a copy will not beat again: it is let go.
      for (const s of following()) if (s.parked && Date.now() - s.beat > FORGOTTEN_MS) following().delete(s);
    } finally {
      w.busy = false;
    }
  });
}

/** The beats of each copy: one timer per view context, however many subscriptions it holds. */
const beats = () => (globalThis.cockpitHubBeats ??= new WeakMap());

/** Beats every second for the subscriptions made with this context, while any of them lives. */
function beatFor(cx, state, tick) {
  if (typeof cx?.timer?.every !== "function") return () => {};
  let b = beats().get(cx);
  if (!b) {
    b = { states: new Map(), timer: null };
    b.timer = cx.timer.every(BEAT_MS, (later) => {
      const now = Date.now();
      sign();
      // Long since its last beat, this copy was asleep with the machine: what it sees of the others is stale.
      const slept = b.last !== undefined && now - b.last > 2 * BEAT_MS;
      b.last = now;
      for (const [st, fn] of b.states) {
        st.beat = now;
        fn(later ?? cx);
      }
      watchdog(later ?? cx, slept);
    });
    beats().set(cx, b);
  }
  b.states.set(state, tick);
  return () => {
    b.states.delete(state);
    if (b.states.size === 0) {
      b.timer?.cancel?.();
      beats().delete(cx);
    }
  };
}

/**
 * Follows views and store documents for as long as the mod copy lives: `onMessage` gets each
 * `{view, args, value}`, `{doc, value}` and `{focus, target}` line, then every new version; with `notify`, the
 * hub's notifications too, as `{notify}` (the sidebar asks, one copy per window). `onState` hears
 * "connecting", "live" and "error: …". Reconnects with backoff whenever the hub goes.
 * `focus` names the app whose pending local navigation this view handles.
 *
 *   subscribe(cx, { views: ["todo"] }, (m, cx) => { this.model = m.value; cx.notify(); });
 */
export function subscribe(cx, { views = [], docs = [], notify = false, focus = null }, onMessage, onState = () => {}) {
  if (checking()) return { stopped: true, stop() {} };
  // `beat` moves while the copy lives (a copy the window dropped stops beating); `port` names the hub it is on;
  // `attempt` numbers the task following the hub, `moved` is when it last got anywhere, `answered` whether the hub ever
  // answered it (live or an error).
  sign();
  const state = { load: LOAD, stopped: false, parked: false, beat: Date.now(), alive: Date.now(), port: null, attempt: 0, moved: Date.now(), connecting: true, answered: false, opening: new Set(), stop: () => sub.stop(), park: () => park() };
  following().add(state);
  const navigation = (globalThis.cockpitRemoteFocus ??= { targets: new Map(), listeners: new Set() });
  const ready = new Set();
  const focused = target => {
    if (!state.stopped && focus === target.app && ready.has(target.app)) {
      navigation.targets.delete(target.app);
      onMessage({ focus: target.app, target }, cx);
      return true;
    }
    return false;
  };
  focused.load = LOAD;
  navigation.listeners.add(focused);
  // One task follows the hub at a time. A task can be left waiting on the window for good (a file read or a timer
  // made in the middle of a layout switch never comes back): the copy's own timer then starts another one.
  const follow = (cx) => {
    const mine = ++state.attempt;
    const current = () => !state.stopped && !state.parked && mine === state.attempt;
    state.alive = Date.now();
    cx.spawn(async (cx) => {
      // This task's own timer: while it fires, the task's context lives, however long a step waits.
      const alive = typeof cx.timer?.every === "function" ? cx.timer.every(1000, () => mine === state.attempt && (state.alive = Date.now())) : null;
      let wait = 250;
      // This task's own socket: it closes it whatever happens, never another task's.
      let own = null;
      while (current()) {
        state.connecting = true;
        state.moved = Date.now();
        onState("connecting", cx);
        try {
          const { socket, token, lines, remote, port } = await connect(state, cx);
          own = socket;
          if (!current()) break;
          state.socket = socket;
          state.port = port;
          await socket.write(JSON.stringify({ op: "subscribe", token, views, docs, notify, keepalive: KEEPALIVE_SECS }) + "\n");
          let wrote = Date.now();
          state.connecting = false;
          state.answered = true;
          reached();
          onState("live", cx);
          wait = 250;
          for (;;) {
            let deadline;
            let line;
            try {
              const next = lines.next();
              // The heartbeat may win: the read left behind must not reject into nothing.
              next.catch(() => {});
              line = await Promise.race([
                next,
                new Promise((_, reject) => {
                  deadline = cx.timer.after(65_000, () => reject(new Error("the cockpit hub heartbeat timed out")));
                }),
              ]);
            } finally {
              deadline?.cancel();
            }
            if (line === null || !current()) break;
            // The hub writes at least every 10 s, so answering a line keeps the promise of one every KEEPALIVE_SECS.
            if (Date.now() - wrote >= (KEEPALIVE_SECS - 5) * 1000) {
              await socket.write("\n");
              wrote = Date.now();
            }
            if (!line.trim()) continue;
            const message = JSON.parse(line);
            if (message.error) throw new Error(message.error);
            if (remote && message.focus) continue;
            if (remote && message.view === "agents") message.value = { ...message.value, remote };
            onMessage(message, cx);
            if (message.view && !ready.has(message.view)) {
              ready.add(message.view);
              const target = navigation.targets.get(message.view);
              if (target) focused(target);
            }
          }
        } catch (e) {
          state.answered = true;
          if (current()) {
            failed(e);
            onState(`error: ${e?.message ?? e}`, cx);
          }
        } finally {
          close(own);
          if (state.socket === own) state.socket = null;
          own = null;
        }
        if (!current()) break;
        state.connecting = true;
        state.moved = Date.now();
        await sleep(cx, wait);
        wait = Math.min(wait * 2, 5000);
      }
      alive?.cancel?.();
    });
  };
  // Given up by the watchdog (its copy stopped beating): the socket goes, and the copy's own beat follows again if it
  // was only slow.
  const park = () => {
    state.parked = true;
    state.attempt += 1;
    close(state.socket);
    for (const socket of state.opening) close(socket);
  };
  follow(cx);
  const unbeat = beatFor(cx, state, (cx) => {
    if (state.stopped) return;
    if (state.parked) {
      state.parked = false;
      return follow(cx);
    }
    if (stuck(state, Date.now())) {
      for (const socket of state.opening) close(socket);
      follow(cx);
    }
  });
  const sub = {
    /** Whether it is stopped, by its copy or by the watchdog. */
    get stopped() {
      return state.stopped;
    },
    stop() {
      state.stopped = true;
      following().delete(state);
      unbeat();
      navigation.listeners.delete(focused);
      close(state.socket);
      for (const socket of state.opening) close(socket);
    },
  };
  return sub;
}
