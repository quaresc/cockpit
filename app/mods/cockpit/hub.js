// The cockpit hub over TCP: its address and token from hub.json, the hub started when none answers, one request per
// connection, and subscriptions that come back by themselves when the hub restarts.

import net from "net";
import { Buffer } from "buffer";
import { exec, appHome, readTextFile, window as win } from "herdr";

/** Waits `ms` on the context's timer (mods have no setTimeout). */
export const sleep = (cx, ms) => new Promise((resolve) => cx.timer.after(ms, () => resolve()));

const NOT_INSTALLED = "cockpit is not installed in this Herdr home";

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
  installed = { at: Date.now(), value: installed?.text === text ? installed.value : JSON.parse(text), text };
  return installed.value;
}

async function readInfo() {
  const { cache } = await install();
  const text = await readTextFile(`${cache}/hub.json`);
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
async function address(fresh) {
  if (!fresh) {
    const info = await readInfo();
    if (info?.port) return info;
  }
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
        const line = Buffer.from(this.pending.slice(0, at)).toString("utf8");
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

async function open(fresh) {
  const info = await address(fresh);
  const socket = await net.connect("127.0.0.1", info.port);
  return { socket, token: info.token };
}

async function connect() {
  try {
    return await open(false);
  } catch {
    // hub.json named a hub that is gone: start one.
    return await open(true);
  }
}

/** Runs the cockpit binary on the install's HOME, driving this Herdr window: { code, stdout, stderr }. */
export async function cockpit(args) {
  const { bin, home, env = {} } = await install();
  const vars = Object.entries(env).map(([k, v]) => `${k}=${v}`);
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
  const { socket, token } = await connect();
  try {
    await socket.write(JSON.stringify({ op, token, frontend: "app", ...fields }) + "\n");
    const line = await new Lines(socket).next();
    if (line === null) throw new Error("the cockpit hub closed the connection");
    const answer = JSON.parse(line);
    if (answer.error && answer.note === undefined) throw new Error(answer.error);
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
 *
 *   subscribe(cx, { views: ["todo"] }, (m, cx) => { this.model = m.value; cx.notify(); });
 */
export function subscribe(cx, { views = [], docs = [], notify = false }, onMessage, onState = () => {}) {
  const state = { stopped: false };
  cx.spawn(async (cx) => {
    let wait = 250;
    while (!state.stopped) {
      onState("connecting", cx);
      try {
        const { socket, token } = await connect();
        state.socket = socket;
        await socket.write(JSON.stringify({ op: "subscribe", token, views, docs, notify }) + "\n");
        const lines = new Lines(socket);
        onState("live", cx);
        wait = 250;
        for (;;) {
          const line = await lines.next();
          if (line === null) break;
          if (!line.trim()) continue;
          const message = JSON.parse(line);
          if (message.error) throw new Error(message.error);
          onMessage(message, cx);
        }
      } catch (e) {
        onState(`error: ${e?.message ?? e}`, cx);
      }
      if (state.stopped) break;
      await sleep(cx, wait);
      wait = Math.min(wait * 2, 5000);
    }
  });
  return {
    stop() {
      state.stopped = true;
      try {
        state.socket?.close();
      } catch {}
    },
  };
}
