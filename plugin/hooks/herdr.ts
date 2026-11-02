import type { EngineInterface } from 'claude-code'

import { cockpitCommand, loadConfig } from './config'

type $ = EngineInterface

// The herdr CLI, and where this session sits in herdr: its pane, tab and space, and the browsers beside it.

export function herdr($: $, args: string[]) {
  return $.process.run(['herdr', ...args], { timeoutMs: 10_000 })
}

let isBackground = false
let isJob = false

// A helper session (a `claude -p` run, a daemon session working for another) inherits its parent's herdr pane: it stays
// out of that pane's rows and heartbeat. A background job the person started with /bg has a job folder and keeps its pane.
export async function setBackground($: $, isInteractive: boolean) {
  const isDaemon = (await $.env.get('CLAUDE_CODE_SESSION_KIND')) === 'bg'
  const jobDir = await $.env.get('CLAUDE_JOB_DIR')
  // A job's own process may not carry the bg kind: its job folder is what says it is one.
  isJob = Boolean(jobDir) && (await $.fs.exists(jobDir ?? ''))
  isBackground = !isJob && (!isInteractive || isDaemon)
  return isBackground
}

// A /bg job has no REPL to bind the mod's MCP calls to: it draws its own rows, and leaves fetching to the sessions at a prompt.
export const isBackgroundJob = () => isJob

export async function isInHerdr($: $) {
  return !isBackground && (await $.env.get('HERDR_ENV')) === '1' && !(await isAppTerminal($))
}

// The Herdr desktop app gives its terminals HC_TERMINAL_ID.
export async function isInApp($: $) {
  return !isBackground && (await isAppTerminal($))
}

let appTerminal: { key: string; value: boolean } | undefined

// herdr's variables leak into the app's terminals when herdr-core was started from a herdr pane, and HC_TERMINAL_ID
// into a herdr started in an app terminal: with both, the session is in the app when it descends from the app
// terminal's shell, since a herdr pane's programs descend from the herdr server instead.
async function isAppTerminal($: $) {
  const terminal = await $.env.get('HC_TERMINAL_ID')
  const isTui = (await $.env.get('HERDR_ENV')) === '1'
  if (!terminal || !isTui) return Boolean(terminal)
  if (appTerminal?.key === terminal) return appTerminal.value
  const script = `${APP_PATH}; shell=$("\${COCKPIT_HC_BIN:-hc}" get "$1" --json | sed -n 's/.*"pid":\\([0-9]*\\).*/\\1/p' | tail -1)
pid=$PPID
while [ -n "$pid" ] && [ "$pid" -gt 1 ]; do [ "$pid" = "$shell" ] && exit 0; pid=$(ps -o ppid= -p "$pid" | tr -d ' '); done
exit 1`
  const found = await $.process.run(['sh', '-c', script, 'sh', terminal], { timeoutMs: 5_000 }).catch(() => undefined)
  appTerminal = { key: terminal, value: found?.exitCode === 0 }
  return appTerminal.value
}

export type Frontend = 'tui' | 'app'

export async function frontendOf($: $): Promise<Frontend | undefined> {
  return (await isInHerdr($)) ? 'tui' : (await isInApp($)) ? 'app' : undefined
}

// The Herdr app's programs, from ~/.local/bin where its installer puts them.
const APP_PATH = 'PATH="$HOME/.local/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"'

// `herdr-app do <command> <json>`: what it made, undefined when it failed.
export async function appDo($: $, command: string, args: Record<string, unknown> = {}): Promise<Record<string, unknown> | undefined> {
  const done = await $.process.run(['sh', '-c', `${APP_PATH} exec "\${HERDR_APP_BIN:-herdr-app}" do "$1" "$2"`, 'sh', command, JSON.stringify(args)], { timeoutMs: 10_000 })
  if (done.exitCode !== 0) return undefined
  const made = JSON.parse(done.stdout || '{}') as Record<string, unknown>
  return made.error ? undefined : made
}

type AppTerminal = { terminal: string; pane: string; status: string | null; name: string | null; app: string | null }
type AppTab = { id: string; name: string; active: boolean; terminals: AppTerminal[] }
export type AppSpace = { id: string; name: string; folder: string; machine: string | null; active: boolean; tabs: AppTab[] }

// `herdr-app ls`: the spaces with their tabs and terminals.
export async function appSpaces($: $): Promise<AppSpace[]> {
  const listed = await $.process.run(['sh', '-c', `${APP_PATH} exec "\${HERDR_APP_BIN:-herdr-app}" ls`], { timeoutMs: 10_000 })
  return listed.exitCode === 0 ? ((JSON.parse(listed.stdout || '{}') as { spaces?: AppSpace[] }).spaces ?? []) : []
}

// What `herdr-app status` says is on screen: the selected space and tab.
async function appStatus($: $) {
  const shown = await $.process.run(['sh', '-c', `${APP_PATH} exec "\${HERDR_APP_BIN:-herdr-app}" status`], { timeoutMs: 10_000 })
  return shown.exitCode === 0 ? (JSON.parse(shown.stdout || '{}') as { space?: string; tab?: string; layout?: string }) : {}
}

// The terminal, its pane, tab and space in the app's listing.
export function findTerminal(spaces: AppSpace[], terminal: string) {
  for (const space of spaces) {
    for (const tab of space.tabs) {
      const found = tab.terminals.find(t => t.terminal === terminal)
      if (found) return { space, tab, terminal: found }
    }
  }
  return undefined
}

// `pane` is the handle cockpit's CLI and hub take: a herdr pane id, or `app:<terminal>` in the Herdr app, where
// `paneId` is the app's pane and `terminal` the core terminal.
export type Here = { pane: string; tab: string; space: string; frontend?: Frontend; terminal?: string; paneId?: string }

// The pane, tab and space this session sits in now; a pane moved to another space gets a new id.
export async function herdrHere($: $): Promise<Here | undefined> {
  if (await isInApp($)) return appHere($)
  if (!(await isInHerdr($))) return undefined
  const { exitCode, stdout } = await herdr($, ['pane', 'current', '--current'])
  if (exitCode !== 0) return undefined
  const pane = JSON.parse(stdout)?.result?.pane as { pane_id: string; tab_id: string; workspace_id: string } | undefined
  return pane && { pane: pane.pane_id, tab: pane.tab_id, space: pane.workspace_id, frontend: 'tui' }
}

async function appHere($: $): Promise<Here | undefined> {
  const terminal = (await $.env.get('HC_TERMINAL_ID')) ?? ''
  const found = findTerminal(await appSpaces($), terminal)
  return found && { pane: `app:${terminal}`, tab: found.tab.id, space: found.space.id, frontend: 'app', terminal, paneId: found.terminal.pane }
}

// herdrHere for every tool call would be one herdr call each: the answer is kept a minute.
let hereCache: { at: number; value: Here | undefined } | undefined
export async function hereSoon($: $) {
  if (!hereCache || Date.now() - hereCache.at > 60_000) hereCache = { at: Date.now(), value: await herdrHere($) }
  return hereCache.value
}

type Space = { workspace_id: string; label: string; focused: boolean }

async function spaces($: $) {
  const { exitCode, stdout } = await herdr($, ['workspace', 'list'])
  return exitCode === 0 ? ((JSON.parse(stdout)?.result?.workspaces as Space[] | undefined) ?? []) : []
}

const bare = (label: string) => label.replace(/^\[\d+\]\s*/, '')

export async function inboxSpace($: $) {
  if (!(await isInHerdr($))) return undefined
  return (await spaces($)).find(w => w.label.endsWith('  inbox'))?.workspace_id
}

type Tab = { tab_id: string; label: string; focused: boolean }

async function tabOf($: $, id: string) {
  const { exitCode, stdout } = await herdr($, ['tab', 'get', id])
  return exitCode === 0 ? (JSON.parse(stdout)?.result?.tab as Tab | undefined) : undefined
}

// A pinned space, labelled "<icon>  <name>", holds sessions in tabs of their own, such as the actions space's.
async function isPinned($: $, label: string) {
  const names = (await loadConfig($)).spaces.map(s => (typeof s === 'string' ? s : s.name))
  return names.some(name => label.endsWith(`  ${name}`))
}

export type Place = { label: string; isFocused: boolean; target: string }

// Where a session shows: its tab when it sits in a pinned space, else its space; `target` is what a notification's click focuses.
export async function placeOf($: $, at: { space: string; tab?: string; frontend?: Frontend }): Promise<Place | undefined> {
  if (at.frontend === 'app' || (at.frontend === undefined && (await isInApp($)))) return appPlaceOf($, at)
  const space = (await spaces($)).find(w => w.workspace_id === at.space)
  if (!space) return undefined
  const tab = at.tab && (await isPinned($, space.label)) ? await tabOf($, at.tab) : undefined
  if (tab) return { label: tab.label, isFocused: tab.focused, target: `tab:${tab.tab_id}` }
  return { label: bare(space.label), isFocused: space.focused, target: `workspace:${space.workspace_id}` }
}

// In the app a session shows as its tab when the tab has a name of its own, else as its space; the window's
// selected tab is what is in view.
async function appPlaceOf($: $, at: { space: string; tab?: string }): Promise<Place | undefined> {
  const space = (await appSpaces($)).find(s => s.id === at.space)
  if (!space) return undefined
  const tab = space.tabs.find(t => t.id === at.tab)
  const shown = await appStatus($)
  const isFocused = tab ? shown.tab === tab.id : shown.space === space.id
  if (tab && !/^\d+$/.test(tab.name)) return { label: tab.name, isFocused, target: `tab:app:${tab.id}` }
  return { label: space.name, isFocused, target: tab ? `tab:app:${tab.id}` : `workspace:app:${space.id}` }
}

export async function placeHere($: $) {
  const here = await herdrHere($)
  return here && placeOf($, here)
}

export type Browser = { tabs?: { url: string; title?: string; active?: boolean }[] }

// The terminal-browser browsers in this session's tab (an agent's checks, browser views); none outside herdr and the app.
export async function browsersBeside($: $, here: Here): Promise<Browser[]> {
  if (here.frontend === 'app') {
    const cockpit = await cockpitCommand($)
    const found = cockpit ? await $.process.run([...cockpit, 'browse', '--browsers', await $.session.id()], { timeoutMs: 15_000 }).catch(() => undefined) : undefined
    return found?.exitCode === 0 ? (JSON.parse(found.stdout || '[]') as Browser[]) : []
  }
  const listed = await $.process.run(['sh', '-c', 'PATH="/opt/homebrew/bin:/usr/local/bin:$HOME/.local/bin:$PATH" exec terminal-browser ls --json'], {
    env: { HERDR_PANE_ID: here.pane, HERDR_TAB_ID: here.tab },
    timeoutMs: 2_000,
  }).catch(() => undefined)
  if (listed?.exitCode !== 0) return []
  return (JSON.parse(listed.stdout) as { browsers?: Browser[] }).browsers ?? []
}

async function pageBeside($: $) {
  const here = await hereSoon($)
  return here ? (await browsersBeside($, here)).flatMap(b => b.tabs ?? []).find(t => t.active) : undefined
}

let toldPage = ''

// Once per page: the next prompt carries the page beside the session, so "this page" means it.
export async function pageContext($: $) {
  const page = await pageBeside($).catch(() => undefined)
  if (!page || page.url === toldPage) return []
  toldPage = page.url
  return [`The browser beside this conversation shows "${page.title || page.url}" at ${page.url}. ` +
    'When the person says "this page" they mean it; read or drive it with `terminal-browser action -- snapshot` (the side-browser skill).']
}
