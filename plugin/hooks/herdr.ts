import type { EngineInterface } from 'claude-code'

import { loadConfig } from './config'

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
  isJob = isDaemon && Boolean(jobDir) && (await $.fs.exists(jobDir ?? ''))
  isBackground = !isJob && (!isInteractive || isDaemon)
  return isBackground
}

// A /bg job has no REPL to bind the mod's MCP calls to: it draws its own rows, and leaves fetching to the sessions at a prompt.
export const isBackgroundJob = () => isJob

export async function isInHerdr($: $) {
  return !isBackground && (await $.env.get('HERDR_ENV')) === '1'
}

export type Here = { pane: string; tab: string; space: string }

// The pane, tab and space this session sits in now; a pane moved to another space gets a new id.
export async function herdrHere($: $): Promise<Here | undefined> {
  if (!(await isInHerdr($))) return undefined
  const { exitCode, stdout } = await herdr($, ['pane', 'current', '--current'])
  if (exitCode !== 0) return undefined
  const pane = JSON.parse(stdout)?.result?.pane as { pane_id: string; tab_id: string; workspace_id: string } | undefined
  return pane && { pane: pane.pane_id, tab: pane.tab_id, space: pane.workspace_id }
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
export async function placeOf($: $, at: { space: string; tab?: string }): Promise<Place | undefined> {
  const space = (await spaces($)).find(w => w.workspace_id === at.space)
  if (!space) return undefined
  const tab = at.tab && (await isPinned($, space.label)) ? await tabOf($, at.tab) : undefined
  if (tab) return { label: tab.label, isFocused: tab.focused, target: `tab:${tab.tab_id}` }
  return { label: bare(space.label), isFocused: space.focused, target: `workspace:${space.workspace_id}` }
}

export async function placeHere($: $) {
  const here = await herdrHere($)
  return here && placeOf($, here)
}

export type Browser = { tabs?: { url: string; title?: string; active?: boolean }[] }

// The terminal-browser browsers in this session's tab (an agent's checks, browser views); none outside herdr.
export async function browsersBeside($: $, here: { pane: string; tab: string }): Promise<Browser[]> {
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
