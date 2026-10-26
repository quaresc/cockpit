import type { EngineInterface } from 'claude-code'

type $ = EngineInterface

// The herdr CLI, and where this session sits in herdr: its pane, tab and space, and the browsers beside it.

export function herdr($: $, args: string[]) {
  return $.process.run(['herdr', ...args], { timeoutMs: 10_000 })
}

let isBackground = false

// A helper session (a `claude -p` run, a daemon session working for another) inherits its parent's herdr pane: it stays
// out of that pane's rows and heartbeat. A background job the person started with /bg has a job folder and keeps its pane.
export async function setBackground($: $, isInteractive: boolean) {
  const isDaemon = (await $.env.get('CLAUDE_CODE_SESSION_KIND')) === 'bg'
  const jobDir = await $.env.get('CLAUDE_JOB_DIR')
  const isJob = isDaemon && Boolean(jobDir) && (await $.fs.exists(jobDir ?? ''))
  isBackground = !isJob && (!isInteractive || isDaemon)
  return isBackground
}

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

// This session's space: its id, its label without the "[N] " prefix, and whether it is the one in view.
export async function spaceOf($: $) {
  const here = await herdrHere($)
  if (!here) return undefined
  const space = (await spaces($)).find(w => w.workspace_id === here.space)
  return space && { id: space.workspace_id, label: bare(space.label), isFocused: space.focused }
}

export async function spaceLabel($: $, id: string) {
  const space = (await spaces($)).find(w => w.workspace_id === id)
  return space && bare(space.label)
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
