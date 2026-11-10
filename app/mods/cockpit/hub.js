// The cockpit hub over TCP: its address and token from hub.json, the hub started when none answers, one request per
// connection, and subscriptions that come back by themselves when the hub restarts.

import net from "net";
import { exec, appHome, readTextFile, run, window as win } from "herdr";

/** Waits `ms` on the context's timer (mods have no setTimeout). */
export const sleep = (cx, ms) => new Promise((resolve) => cx.timer.after(ms, () => resolve()));

const NOT_INSTALLED = "cockpit is not installed in this Herdr home";
const READ_ONLY = "the remote cockpit client is read-only; this action needs a local cockpit installation";
const REMOTE_READS = new Set(["ping", "view", "search", "fetch", "events", "history", "shown"]);
const APPS = new Set(["inbox", "todo", "runbook", "meetings", "mail", "monitor", "routines", "show"]);

function localFocus(target) {
  if (!APPS.has(target?.app)) throw new Error(READ_ONLY);
  const focus = (globalThis.cockpitRemoteFocus ??= { targets: new Map(), listeners: new Set() });
  focus.targets.set(target.app, target);
  if (target.app === "show") {
    run("cockpit.go:work");
    run("layout.select", { stack: "panel-body", child: "show" });
    run("layout.show", { id: "panel" });
  } else run(`cockpit.go:${target.app}`);
  for (const listener of focus.listeners) if (listener(target)) break;
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

async function readInfo() {
  const { cache, machine } = await install();
  const text = await readTextFile(`${cache}/hub.json`, machine ? { on: machine } : undefined);
  return text ? JSON.parse(text) : null;
}

// One `cockpit hub start` at a time for every view, and after a failure (the binary gone, a hub that won't start) the
// next try waits 30 s: until then every caller gets the same error. Kept on globalThis, which every copy shares.
async function startHub() {
  const s = (globalThis.cockpitHubStart ??= { starting: null, failed: null });
  if (s.failed && Date.now() - s.failed.at < 30_000) throw s.failed.error;
  s.starting ??= (async () => {
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
      s.starting = null;
    }
  })();
  return s.starting;
}

/** hub.json's { port, token }: as written, else after `cockpit hub start` started a hub. */
async function address(fresh, state) {
  if (!fresh) {
    const info = await readInfo();
    if (info?.port) return info;
  }
  if (state?.stopped) throw new Error("subscription stopped");
  return startHub();
}

/** Splits a socket's bytes into lines, decoding each whole line (a UTF-8 character may span two reads). */
class Lines {
  constructor(socket) {
    this.socket = socket;
    this.pending = new Uint8Array(0);
  }

  async next() {
    for (;;) {
      const at = this.pending.indexOf(10);
      if (at >= 0) {
        // Herdr's Windows Buffer shim returns empty buffers; decode complete UTF-8 lines with the runtime built-in.
        const line = decodeURIComponent(Array.from(this.pending.subarray(0, at), byte => `%${byte.toString(16).padStart(2, "0")}`).join(""));
        this.pending = this.pending.slice(at + 1);
        return line;
      }
      const chunk = await this.socket.read(65536);
      if (chunk === null) return null;
      const joined = new Uint8Array(this.pending.length + chunk.length);
      joined.set(this.pending, 0);
      joined.set(chunk, this.pending.length);
      this.pending = joined;
    }
  }
}

async function open(fresh, state) {
  const { host, machine } = await install();
  const info = await address(fresh, state);
  if (state?.stopped) throw new Error("subscription stopped");
  const socket = await net.connect(machine ? host : (info.host ?? "127.0.0.1"), info.port);
  const lines = new Lines(socket);
  try {
    if (state?.stopped) throw new Error("subscription stopped");
    if (state) state.socket = socket;
    await socket.write(JSON.stringify({ op: "ping", token: info.token }) + "\n");
    const answer = JSON.parse(await lines.next());
    if (answer?.ok !== true) throw new Error(answer?.error ?? "the cockpit hub did not answer ping");
    return { socket, token: info.token, lines, remote: machine ?? null };
  } catch (error) {
    socket.close();
    throw error;
  }
}

async function connect(state) {
  try {
    return await open(false, state);
  } catch (error) {
    if (state?.stopped) throw error;
    // hub.json named a hub that is gone: start one.
    return await open(true, state);
  }
}

/** Runs the cockpit binary on the install's HOME, driving this Herdr window: { code, stdout, stderr }. */
export async function cockpit(args) {
  const { bin, home, env = {}, machine, host } = await install();
  const vars = Object.entries(env).filter(([k]) => !machine || k !== "HERDR_APP_HOME").map(([k, v]) => `${k}=${v}`);
  if (machine) {
    if (args[0] === "focus" && args.length === 2) {
      const target = args[1].replace(/^herdr-app:/, "");
      const at = target.indexOf(":");
      if (at < 1) throw new Error(READ_ONLY);
      const app = target.slice(0, at);
      const key = target.slice(at + 1);
      const step = key.indexOf(":");
      localFocus(app === "runbook"
        ? { app, key: step < 0 ? key : key.slice(0, step), steps: step < 0 ? [] : key.slice(step + 1).split(",").filter(Boolean) }
        : { app: { meeting: "meetings", routine: "routines" }[app] ?? app, key });
      return { code: 0, stdout: "", stderr: "" };
    }
    if (args[0] !== "hub" || !["start", "ping", "shown"].includes(args[1])) throw new Error(READ_ONLY);
    return exec("env", [...vars, `HOME=${home}`, `COCKPIT_HUB_LISTEN=${host}`, "COCKPIT_FRONTEND=app", bin, ...args], { on: machine });
  }
  return exec("env", [`HOME=${home}`, `HERDR_APP_HOME=${appHome()}`, "COCKPIT_FRONTEND=app", ...vars, bin, ...args]);
}

/** One request: its answer, or a thrown Error with the hub's `error`. */
export async function request(op, fields = {}) {
  // A page opens from the window itself: macOS keeps the browser behind when a background process (the hub) asks.
  if (op === "act" && fields.action === "open_url" && typeof fields.args?.[0] === "string") {
    const url = fields.args[0];
    const r = await openUrl(url);
    return r.code === 0 ? { note: `opened ${url}`, pending: {}, refreshing: {} } : { error: r.stderr.trim() || `could not open ${url}`, pending: {} };
  }
  if ((await install()).machine) {
    if (op === "go") return localFocus(fields.target);
    if (!REMOTE_READS.has(op)) throw new Error(READ_ONLY);
  }
  const { socket, token, lines, remote } = await connect();
  try {
    await socket.write(JSON.stringify({ op, token, frontend: "app", ...fields }) + "\n");
    const line = await lines.next();
    if (line === null) throw new Error("the cockpit hub closed the connection");
    const answer = JSON.parse(line);
    if (answer.error && answer.note === undefined) throw new Error(answer.error);
    if (remote && op === "view" && fields.name === "agents") answer.value = { ...answer.value, remote };
    return answer;
  } finally {
    socket.close();
  }
}

/** An app's action: { note, pending, refreshing, error }. */
export const act = (app, action, args = []) => request("act", { app, action, args });

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
  const state = { stopped: false };
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
  navigation.listeners.add(focused);
  cx.spawn(async (cx) => {
    let wait = 250;
    while (!state.stopped) {
      onState("connecting", cx);
      try {
        const { socket, token, lines, remote } = await connect(state);
        state.socket = socket;
        if (state.stopped) break;
        await socket.write(JSON.stringify({ op: "subscribe", token, views, docs, notify }) + "\n");
        onState("live", cx);
        wait = 250;
        for (;;) {
          let deadline;
          let line;
          try {
            line = await Promise.race([
              lines.next(),
              new Promise((_, reject) => {
                deadline = cx.timer.after(65_000, () => reject(new Error("the cockpit hub heartbeat timed out")));
              }),
            ]);
          } finally {
            deadline?.cancel();
          }
          if (line === null) break;
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
        if (!state.stopped) onState(`error: ${e?.message ?? e}`, cx);
      } finally {
        state.socket?.close();
        state.socket = null;
      }
      if (state.stopped) break;
      await sleep(cx, wait);
      wait = Math.min(wait * 2, 5000);
    }
  });
  return {
    stop() {
      state.stopped = true;
      navigation.listeners.delete(focused);
      try {
        state.socket?.close();
      } catch {}
    },
  };
}
