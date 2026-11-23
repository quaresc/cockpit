// The cockpit mod's terminal link handler (Herdr 0.1.17): a Cmd-click on a web link in a terminal opens in the person's
// browser, Herdr's own way, unless `links.open` is "side": then in the side browser beside that terminal.

import { appHome, exec } from "herdr";
import { installOf, readConfig, optionOf } from "./config.js";

/** What a click asks of cockpit: "side" for the side browser beside its terminal, else null to leave it to Herdr. */
export function routeOf(click, config, install) {
  if (click?.link?.kind !== "url" || !/^https?:\/\//i.test(click.link.url ?? "")) return null;
  if (!install?.bin || install.machine || click.source?.machine || !click.source?.terminal) return null;
  return optionOf(config, "links.open", "browser") === "side" ? "side" : null;
}

/** `cockpit browse <url>` for the clicked terminal, on the install's HOME and driving this window. */
export function browseArgs(click, install, home = appHome()) {
  const vars = Object.entries(install.env ?? {}).map(([k, v]) => `${k}=${v}`);
  return [`HOME=${install.home}`, `HERDR_APP_HOME=${home}`, "COCKPIT_FRONTEND=app", `HC_TERMINAL_ID=${click.source.terminal}`, ...vars, install.bin, "browse", click.link.url];
}

export async function handleLink(click) {
  const install = await installOf();
  if (routeOf(click, await readConfig(install), install) !== "side") return "pass";
  const r = await exec("env", browseArgs(click, install), { timeout: 25_000 }).catch(() => null);
  return r?.code === 0 ? "handled" : "pass";
}
