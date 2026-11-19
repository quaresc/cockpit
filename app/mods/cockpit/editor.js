// Open in editor: a folder in the person's editor, on this machine or over SSH for a remote space (VS Code's and
// Cursor's Remote-SSH). The editor is the cockpit config's `editor` (code, cursor, zed, ...), else VS Code.

import { exec, readTextFile, hc, window as win } from "herdr";
import * as hub from "./hub.js";

const APPS = { code: "Visual Studio Code", cursor: "Cursor", zed: "Zed", windsurf: "Windsurf", subl: "Sublime Text" };

/**
 * The config's `editor`, one command: from the resolved copy in the cache (plain JSON), else the file as written when it
 * reads as plain JSON (one with comments has its copy); else VS Code's `code`.
 */
export async function editor() {
  const { home, cache } = await hub.install().catch(() => ({}));
  for (const path of [cache && `${cache}/config.json`, home && `${home}/.config/cockpit/config.json`].filter(Boolean)) {
    try {
      const name = JSON.parse((await readTextFile(path)) ?? "null")?.editor;
      if (typeof name === "string" && name.trim() && !name.trim().startsWith("-")) return name.trim();
    } catch {}
  }
  return "code";
}

/** The SSH host of a machine `hc machine ls` knows, else its name. */
async function hostOf(machine) {
  try {
    const r = await hc(["machine", "ls", "--json"]);
    const list = JSON.parse(r.stdout);
    const m = (Array.isArray(list) ? list : list.machines ?? []).find((x) => x.name === machine);
    const target = m?.target ?? m?.host ?? m?.ssh ?? machine;
    return String(target).replace(/^ssh:\/\//, "");
  } catch {
    return machine;
  }
}

/** Opens `folder` in the editor; resolves with what to tell the person when it could not. */
export async function openInEditor(folder, machine = null) {
  if (!folder) return "no folder to open";
  const name = await editor();
  if (machine) {
    const uri = `vscode-remote://ssh-remote+${await hostOf(machine)}${folder}`;
    const r = await exec(name, ["--folder-uri", uri]).catch(() => null);
    return r?.code === 0 ? null : `${name} could not open ${machine}:${folder}`;
  }
  const r = await exec(name, [folder]).catch(() => null);
  if (r?.code === 0) return null;
  const app = APPS[name] ?? name;
  // An editor whose command isn't on the PATH still opens by its app name on macOS.
  const opened = win()?.platform === "macos" ? await exec("open", ["-a", app, folder]).catch(() => null) : null;
  return opened?.code === 0 ? null : `${app} is not installed (set "editor" in the cockpit config)`;
}

/** The editor's name, for labels: "VS Code", "Cursor", ... */
export async function editorLabel() {
  const name = await editor();
  return name === "code" ? "VS Code" : (APPS[name] ?? name);
}
