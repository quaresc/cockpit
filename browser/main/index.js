// Cockpit's browser: an Electron shell the hub starts. It listens on a unix socket, shows one window (docked over the
// Herdr window's slot, or on its own), and opens Chromium's debugging port on loopback for agents' tools.

import fs from "node:fs";
import path from "node:path";
import { spawn } from "node:child_process";
import readline from "node:readline";
import { app, Menu } from "electron";
import * as paths from "./paths.js";
import { serve, alive } from "./control.js";
import { Shell } from "./shell.js";
import { Menus } from "./menus.js";
import { Design } from "./design.js";
import { handlers } from "./ops.js";

if (process.platform !== "darwin") {
  console.error("the Chromium side browser runs on macOS only");
  process.exit(2);
}

fs.mkdirSync(paths.cacheDir(), { recursive: true });
fs.mkdirSync(paths.dataDir(), { recursive: true });
const log = (line) => {
  try {
    if (fs.statSync(paths.logFile(), { throwIfNoEntry: false })?.size > 512 * 1024) fs.rmSync(paths.logFile(), { force: true });
    fs.appendFileSync(paths.logFile(), `${new Date().toISOString()} ${line}\n`);
  } catch {}
};

let options = {};
try {
  options = JSON.parse(process.env.COCKPIT_BROWSER_OPTIONS || "{}");
} catch {}
const docked = options.dock !== false;

app.setName("Cockpit Browser");
app.setPath("userData", paths.userData());
// The port file of the run before would be read as this run's.
fs.rmSync(path.join(paths.userData(), "DevToolsActivePort"), { force: true });
app.commandLine.appendSwitch("remote-debugging-port", "0");
// A page under another app's window keeps drawing: an agent's snapshot or screenshot must not wait for the person to
// uncover the browser. Tabs that are not shown, and the window while it is hidden, stay paused.
app.commandLine.appendSwitch("disable-backgrounding-occluded-windows");
// A sandbox or a test runs on a home folder with no keychain: Chromium keeps its cookie key in memory there, and the
// system never asks where to store it.
if (process.env.COCKPIT_SANDBOX || process.env.COCKPIT_BROWSER_MOCK_KEYCHAIN) app.commandLine.appendSwitch("use-mock-keychain");
// Docked, the browser is part of the Herdr window: no Dock icon, no place among the apps to switch to.
if (docked) app.setActivationPolicy("accessory");

/** The port Chromium chose for its debugging endpoint, written by Chromium beside the user data. */
async function debuggingPort() {
  const file = path.join(paths.userData(), "DevToolsActivePort");
  for (let i = 0; i < 100; i++) {
    try {
      const port = Number(fs.readFileSync(file, "utf8").split("\n")[0]);
      if (port > 0) return port;
    } catch {}
    await new Promise((r) => setTimeout(r, 50));
  }
  return 0;
}

/** Follows the host window on screen through `cockpit browser track`, restarted for each host. */
function follow(shell) {
  const cockpit = process.env.COCKPIT_BIN;
  let child = null;
  let pid = 0;
  const start = (host) => {
    child?.kill();
    child = null;
    pid = host;
    if (!host || !cockpit) return;
    child = spawn(cockpit, ["browser", "track", "--host", String(host)], { stdio: ["ignore", "pipe", "ignore"] });
    readline.createInterface({ input: child.stdout }).on("line", (line) => {
      try {
        shell.dock.track(JSON.parse(line));
      } catch {}
    });
    child.on("exit", () => child && pid === host && ((child = null), shell.dock.track({ host: null, above: [] })));
  };
  return { host: (wanted) => wanted !== pid && start(wanted), stop: () => child?.kill() };
}

app.on("window-all-closed", () => {});

app.whenReady().then(async () => {
  if (await alive(paths.socketPath())) {
    console.error("cockpit's browser is running already");
    app.exit(0);
    return;
  }
  Menu.setApplicationMenu(Menu.buildFromTemplate([{ role: "appMenu" }, { role: "editMenu" }, { role: "windowMenu" }]));
  const shell = new Shell({ options, token: process.env.COCKPIT_BROWSER_TOKEN || "", log });
  shell.menus = new Menus(shell);
  shell.design = new Design(shell);
  shell.cdpPort = await debuggingPort();
  const tracker = follow(shell);
  shell.listeners.add((event, payload) => event === "slot" && tracker.host(payload.hostPid || 0));
  const handle = handlers(shell, { quit: () => quit(), tracker });
  const server = serve(paths.socketPath(), {
    token: shell.token,
    log,
    handle: (request, who) => handle[request.op](request, who),
    subscribe: (send) => {
      const listener = (event, payload) => send({ event, ...payload });
      shell.listeners.add(listener);
      return () => shell.listeners.delete(listener);
    },
  });
  let quitting = false;
  async function quit() {
    if (quitting) return;
    quitting = true;
    tracker.stop();
    server.close();
    fs.rmSync(paths.socketPath(), { force: true });
    await shell.shutdown();
    app.exit(0);
  }
  app.on("before-quit", (event) => {
    if (!quitting) (event.preventDefault(), quit());
  });
  for (const signal of ["SIGTERM", "SIGINT"]) process.on(signal, quit);
  log(`started: pid ${process.pid}, debugging port ${shell.cdpPort}, ${docked ? "docked" : "window"}`);
});
