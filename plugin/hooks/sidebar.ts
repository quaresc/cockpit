import { atom, read, update } from 'claude-code'
import type { EngineInterface, SessionMeasureInput } from 'claude-code'

import { clashWith } from './activity'
import { ciMark, notify, readCi } from './ci'
import { cacheDir, home, loadConfig } from './config'
import { browsersBeside, herdr, hereSoon, inboxSpace, spaceLabel, spaceOf } from './herdr'
import { isDeploying, isFailing, isOldReview, ticketKey } from './match'
import { siteMark } from './sites'
import { ICON } from './icons'
import { branchOf } from './work'

type $ = EngineInterface

const inbox = atom({ plugin: 'cockpit', key: 'inbox' } as const, null)
const muted = atom({ plugin: 'cockpit', key: 'muted' } as const, [])
const sidePage = atom({ plugin: 'cockpit', key: 'sidePage' } as const, null)
const usage = atom({ plugin: 'cockpit', key: 'usage' } as const, null)
const views = atom({ plugin: 'cockpit', key: 'views' } as const, [])
const work = atom({ plugin: 'cockpit', key: 'work' } as const, null)
type Limit = { name: string; percent: number; resetsAt?: string }

// What the herdr sidebar shows: counts and limits on the inbox space's row, and on this agent's rows its context,
// its repo and branch, and the page beside it.

const tokenArgs = (args: string[], tokens: [string, string | undefined][]) => {
  for (const [name, value] of tokens) args.push(...(value === undefined ? ['--clear-token', name] : ['--token', `${name}=${value}`]))
  return args
}

let lastDashboard = ''

async function readLimits($: $): Promise<Limit[]> {
  try {
    const { at, limits } = JSON.parse(await $.fs.read(`${await cacheDir($)}/limits.json`)) as { at: number; limits: Limit[] }
    return Date.now() - at < 6 * 3600_000 ? limits : []
  } catch {
    return []
  }
}

export async function reportDashboard($: $) {
  const data = await read($, inbox)
  const space = await inboxSpace($)
  if (!data || !space) return
  const mutedRepos = await read($, muted)
  const direct = data.direct.filter(pr => !isOldReview(pr)).length
  const team = data.team.filter(pr => !mutedRepos.includes(pr.repo) && !isOldReview(pr)).length
  const failing = data.mine.filter(isFailing).length
  const working = data.tickets.filter(t => t.statusType === 'started' && !isDeploying(t)).length
  const limits = await readLimits($)

  const tokens: [string, string | undefined][] = [
    ['review', direct + team === 0 ? `${ICON.review}  nothing to review` : `${ICON.review}  ${direct} you · ${team} team`],
    ['ci_fail', failing > 0 ? `${ICON.fail}  ${failing} failing` : undefined],
    ['ci_ok', failing === 0 ? `${ICON.ok}  all green` : undefined],
    ['tickets', `${ICON.ticket}  ${working} active · ${data.tickets.length}`],
    ['limits', limits.length > 0 ? `${ICON.gauge}  ${limits.map(l => `${l.name} ${Math.round(l.percent)}%`).join(' · ')}` : undefined],
  ]
  const key = `${space}|${JSON.stringify(tokens)}`
  if (key === lastDashboard) return
  if ((await herdr($, tokenArgs(['workspace', 'report-metadata', space, '--source', 'cockpit'], tokens))).exitCode === 0) lastDashboard = key
}

// Rows are fitted to the sidebar's width from herdr's config, so herdr never cuts their end off with an ellipsis.
let sidebarCols: number | undefined
async function sidebarWidth($: $) {
  if (sidebarCols === undefined) {
    const toml = await $.fs.read(`${await home($)}/.config/herdr/config.toml`).catch(() => '')
    sidebarCols = Number(/^\s*sidebar_width\s*=\s*(\d+)/m.exec(toml)?.[1] ?? 34)
  }
  return sidebarCols
}

const cells = (text: string) => [...text].length
// A dashed repo name down to its initials and last word: sourcing-front -> s-front.
const shortRepo = (repo: string) => repo.split('-').map((word, i, words) => (i < words.length - 1 ? word[0] : word)).join('-')

// The work row in `room` cells: repo · ticket and a running CI's progress, tighter spacing first, then the repo
// shortened, then the ticket alone.
export function fitWork(repo: string, named: string | undefined, ci: string | undefined, room: number) {
  const tail = (gap: string) => (ci ? `${gap}${ci}` : '')
  const candidates = [
    `${ICON.branch} ${[repo, named].filter(Boolean).join(' · ')}${tail('  ')}`,
    `${ICON.branch} ${[repo, named].filter(Boolean).join(' ')}${tail(' ')}`,
    `${ICON.branch} ${[shortRepo(repo), named].filter(Boolean).join(' · ')}${tail(' ')}`,
    `${ICON.branch} ${[shortRepo(repo), named].filter(Boolean).join(' ')}${tail(' ')}`,
    `${ICON.branch} ${named ?? shortRepo(repo)}${tail(' ')}`,
  ]
  return candidates.find(c => cells(c) <= room) ?? candidates[candidates.length - 1]
}

// A topic in `room` cells, whole words only.
export function fitWords(text: string, room: number) {
  if (cells(text) <= room) return text
  const words = text.split(' ')
  while (words.length > 1 && cells(words.join(' ')) > room) words.pop()
  return words.join(' ')
}

// What the context gauge takes on the agent row, with herdr's " · " before it; the topic leaves it that room.
let ctxCells = 8

const glyph = (percent: number) => (percent < 12.5 ? '○' : percent < 37.5 ? '◔' : percent < 62.5 ? '◑' : percent < 87.5 ? '◕' : '●')

// Context, limits and current work: status line, agent rows, shared limits file.
export async function reportUsage($: $, e: SessionMeasureInput) {
  const { context, rateLimits } = e
  const limits: Limit[] = rateLimits.map(l => ({
    name: l.kind === 'five_hour' ? '5h' : l.kind === 'seven_day' ? '7d' : l.kind,
    percent: l.percentUsed,
    resetsAt: l.resetsAt,
  }))
  $.ui.status(undefined)
  await update($, usage, () => ({ limits: limits.map(l => ({ name: l.name, percent: l.percent })) }))

  if (limits.length > 0) {
    await $.fs.write(`${await cacheDir($)}/limits.json`, JSON.stringify({ at: Date.now(), limits }))
    await reportDashboard($)
  }

  const here = await hereSoon($)
  if (!here) return
  const tokens: [string, string | undefined][] = []

  const ctx = context.percent
  const level = ctx === undefined ? undefined : ctx >= 70 ? 'ctx_hot' : ctx >= 45 ? 'ctx_warn' : 'ctx_ok'
  const gauge = ctx === undefined ? undefined : `${glyph(ctx)} ${Math.round(ctx)}%`
  ctxCells = gauge ? cells(` · ${gauge}`) : 0
  for (const name of ['ctx_ok', 'ctx_warn', 'ctx_hot']) tokens.push([name, name === level ? gauge : undefined])
  tokens.push(['cost', undefined])

  // Third agent row: repo · ticket, coloured by the CI of that branch's PR.
  const current = await read($, work)
  const branch = current ? await branchOf($, current) : undefined
  const data = await read($, inbox)
  const pr = current && branch ? data?.mine.find(p => p.repo === current.repo && p.branch === branch) : undefined
  const { tickets, ci } = await loadConfig($)
  const named = branch && (ticketKey(branch, data, tickets.keys) ?? branch.replace(/^[^/]+\//, '').slice(0, 14))
  const run = current && branch ? (await readCi($))?.runs.find(r => r.repo === current.repo && r.branch === branch) : undefined
  // The row sits under the agent's topic, two cells in.
  const progress = run?.isRunning ? `${ciMark(ci.name)} ${run.done}/${run.total}` : undefined
  const text = current ? fitWork(current.repo, named || undefined, progress, (await sidebarWidth($)) - 2) : undefined
  const tone = !text ? undefined : run?.isRunning ? 'work_run' : pr && isFailing(pr) ? 'work_fail' : 'work'
  for (const name of ['work', 'work_run', 'work_fail', 'work_ok']) tokens.push([name, name === tone ? text : undefined])
  // Two agents on one branch overwrite each other: both rows say so.
  const other = current && branch ? await clashWith($, current.repo, branch, here.space) : undefined
  const otherLabel = other ? (await spaceLabel($, other)) ?? other : undefined
  tokens.push(['clash', otherLabel ? `⚠ same branch as ${otherLabel}` : undefined])
  await herdr($, tokenArgs(['pane', 'report-metadata', here.pane, '--source', 'cockpit'], tokens))
}

export function clearRows($: $, pane: string) {
  const names = ['ctx_ok', 'ctx_warn', 'ctx_hot', 'work', 'work_run', 'work_ok', 'work_fail', 'clash', 'agent', 'side_sep', 'side']
  return herdr($, tokenArgs(['pane', 'report-metadata', pane, '--source', 'cockpit'], names.map(name => [name, undefined])))
}

const clip = (text: string, width: number) => (text.length <= width ? text : `${text.slice(0, width - 1).trimEnd()}…`)

// A page's mark on the agent row, from its site, like a favicon in a browser's tab strip.
const siteIcon = (url: string) => {
  const host = url.match(/^https?:\/\/([^/]+)/)?.[1] ?? ''
  if (host.endsWith('github.com')) return ''
  if (host.endsWith('linear.app')) return ICON.ticket
  if (host.startsWith('argocd.')) return ICON.argo
  return siteMark(url) ?? ICON.app
}

let lastRow = ''

type Pane = { pane_id: string; tab_id: string; tokens?: Record<string, string> }

// The agent row, split like Zen's split view when panes sit beside the session: the agent's topic on the left, the
// page beside it on the right, "+N" for the other tabs and panes. herdr draws $agent, $side_sep and $side.
export async function reportSide($: $) {
  const here = await hereSoon($)
  if (!here) return
  const listed = await herdr($, ['pane', 'list', '--workspace', here.space])
  const panes = ((JSON.parse(listed.stdout || '{}')?.result?.panes ?? []) as Pane[]).filter(p => p.tab_id === here.tab)
  const topic = panes.find(p => p.pane_id === here.pane)?.tokens?.topic || ((await spaceLabel($, here.space)) ?? '')
  // Alone in its tab, the session has no browser beside it: terminal-browser need not be asked.
  const browsers = panes.length > 1 ? await browsersBeside($, here) : []
  const tabs = browsers.flatMap(b => b.tabs ?? [])
  const page = tabs.find(t => t.active) ?? tabs[0]
  const shown = page ? { url: page.url, title: page.title ?? '' } : null
  if (JSON.stringify(shown) !== JSON.stringify(await read($, sidePage))) await update($, sidePage, () => shown)
  const view = (await read($, views)).find(v => v.isOpen)
  const others = Math.max(0, panes.length - 1 - browsers.length)
  const extra = tabs.length + others - 1
  const right = page ? `${siteIcon(page.url)} ${clip(page.title || page.url.replace(/^https?:\/\//, ''), 10)}` : view ? `${ICON.view} ${clip(view.name, 10)}` : others > 0 ? `${ICON.view} pane` : ''
  const tokens: [string, string | undefined][] = right
    ? [['agent', clip(topic, 12)], ['side_sep', '│'], ['side', extra > 0 ? `${right} +${extra}` : right]]
    : [['agent', fitWords(topic, (await sidebarWidth($)) - 2 - ctxCells)], ['side_sep', undefined], ['side', undefined]]
  const key = JSON.stringify(tokens)
  if (key === lastRow) return
  if ((await herdr($, tokenArgs(['pane', 'report-metadata', here.pane, '--source', 'cockpit'], tokens))).exitCode === 0) lastRow = key
}

let seenAt = Date.now()

// A browser beside this session closes once its space has been out of view for browserIdleMinutes, and no turn runs,
// so a forgotten page stops drawing; terminal-browser keeps the sign-ins for the next one.
export async function closeIdleBrowsers($: $) {
  const { browserIdleMinutes } = await loadConfig($)
  const space = await spaceOf($)
  const here = await hereSoon($)
  if (!space || !here) return
  const self = await herdr($, ['pane', 'get', here.pane])
  const isWorking = JSON.parse(self.stdout || '{}')?.result?.pane?.agent_status === 'working'
  if (space.isFocused || isWorking || !browserIdleMinutes) {
    seenAt = Date.now()
    return
  }
  if (Date.now() - seenAt < browserIdleMinutes * 60_000) return
  const listed = await herdr($, ['pane', 'list', '--workspace', here.space])
  const panes = ((JSON.parse(listed.stdout || '{}')?.result?.panes ?? []) as Pane[]).filter(p => p.tab_id === here.tab && p.pane_id !== here.pane)
  let closed = 0
  for (const pane of panes) {
    const info = await herdr($, ['pane', 'process-info', '--pane', pane.pane_id])
    const running = (JSON.parse(info.stdout || '{}')?.result?.process_info?.foreground_processes ?? []) as { cmdline?: string }[]
    if (!running.some(p => /terminal-browser/.test(p.cmdline ?? ''))) continue
    if ((await herdr($, ['pane', 'close', pane.pane_id])).exitCode === 0) closed++
  }
  if (closed > 0) await reportSide($)
}

const FINISHED_AFTER_MS = 30_000

// A long turn that ended while its space was out of view: a notification whose click leads back to it.
export async function notifyFinished($: $, durationMs: number, answer: string) {
  if (durationMs < FINISHED_AFTER_MS) return
  const space = await spaceOf($)
  if (!space || space.isFocused) return
  const line = answer.split('\n').map(l => l.replace(/[*`#>_]/g, '').trim()).find(l => l.length > 0) ?? 'Done.'
  await notify($, `✓ ${space.label}`, line.length > 140 ? `${line.slice(0, 139)}…` : line, `workspace:${space.id}`)
}
