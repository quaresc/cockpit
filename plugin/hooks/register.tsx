import { atom, read, update } from 'claude-code'
import type { EngineInterface, Register, RenderChildren, SessionMeasureInput } from 'claude-code'

import type { CiRun, Inbox, MergedPr, Pr, Ticket, TicketLinks, Work } from '../types'

import { activityOf, clashWith, logActivity } from './activity'
import { openUrl } from './browser'
import { dashboardName } from './platform'
import { announcePreviews, ciMark, deployChanges, deploysFor, notify, previewsFor, processRequests, readCi, refreshCi, shortError, startDeployWatch, ticketAction, withLock } from './ci'
import { cacheDir, fill, home, loadConfig } from './config'
import { dashboardFor, deployLinksFor, linksFor, ticketsIn } from './links'
import { chipsFor, citedIn, siteMark } from './sites'
import { myTickets, ticketUrl } from './tickets'
import { maybeBrief } from './brief'
import { refreshMeetings } from './meetings'
import { checkUpdate, forget, heartbeat } from './upkeep'
import { REPORT_SPEC, REPORT_TOOL_FULL, bindHandoff, reportStep, settleTurn } from './handoffs'
import { SHOW_SPEC, SHOW_TOOL_FULL, collectAnswers, commandsOf, refreshSources, show, toggleView, viewsOf } from './show'
import { registerSettings, registerSettingsCommand } from './settings'

const PANE = 'inbox'
const STALE_MS = 5 * 60_000
const DAY_MS = 86_400_000
const OLD_REVIEW_DAYS = 60
const STALE_PR_DAYS = 30

const inbox = atom({ plugin: 'cockpit', key: 'inbox' } as const, null)
const isBusy = atom({ plugin: 'cockpit', key: 'isBusy' } as const, false)
const expanded = atom({ plugin: 'cockpit', key: 'expanded' } as const, [])
const note = atom({ plugin: 'cockpit', key: 'note' } as const, '')
const muted = atom({ plugin: 'cockpit', key: 'muted' } as const, [])
const work = atom({ plugin: 'cockpit', key: 'work' } as const, null)
const usage = atom({ plugin: 'cockpit', key: 'usage' } as const, null)
const views = atom({ plugin: 'cockpit', key: 'views' } as const, [])
const ticket = atom({ plugin: 'cockpit', key: 'ticket' } as const, null)
const links = atom({ plugin: 'cockpit', key: 'links' } as const, null)
const sidePage = atom({ plugin: 'cockpit', key: 'sidePage' } as const, null)

type $ = EngineInterface
type Limit = { name: string; percent: number; resetsAt?: string }

// Review requests only need the rollup; my own PRs also list their checks, to name the failing ones.
// Listing the checks of every review request makes the query too heavy for GitHub (HTTP 502).
const PR_BASE = `number title url isDraft updatedAt headRefName baseRefName repository { name }
  author { __typename login ... on User { name } } reviewDecision`
const PR_FIELDS = `__typename ... on PullRequest { ${PR_BASE} commits(last: 1) { nodes { commit { statusCheckRollup { state } } } } }`
const MY_PR_FIELDS = `__typename ... on PullRequest { ${PR_BASE} commits(last: 1) { nodes { commit { statusCheckRollup { state
  contexts(first: 30) { nodes { ... on CheckRun { name conclusion } ... on StatusContext { context state } } } } } } } }`

const mergedSince = () => new Date(Date.now() - 30 * DAY_MS).toISOString().slice(0, 10)

// My own PRs are limited to the configured organization when there is one.
const query = (org: string) => {
  const scope = org ? ` org:${org.replace(/[^A-Za-z0-9_.-]/g, '')}` : ''
  return `query {
  review: search(query: "is:pr is:open archived:false review-requested:@me", type: ISSUE, first: 60) { nodes { ${PR_FIELDS} } }
  direct: search(query: "is:pr is:open archived:false user-review-requested:@me", type: ISSUE, first: 50) { nodes { ... on PullRequest { url } } }
  mine: search(query: "is:pr is:open archived:false author:@me${scope}", type: ISSUE, first: 30) { nodes { ${MY_PR_FIELDS} } }
  merged: search(query: "is:pr is:merged author:@me${scope} merged:>=${mergedSince()}", type: ISSUE, first: 50) {
    nodes { ... on PullRequest { number url headRefName mergedAt repository { name nameWithOwner } mergeCommit { oid } } } }
}`
}


// ---------------------------------------------------------------------------
// Formatting and classification

const hhmm = (iso: string | undefined) => {
  if (!iso) return ''
  const d = new Date(iso)
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`
}

const ageDays = (iso: string) => (Date.now() - Date.parse(iso)) / DAY_MS

const ago = (iso: string) => {
  const min = Math.max(0, Math.round(ageDays(iso) * 1440))
  if (min < 60) return `${min}m`
  if (min < 48 * 60) return `${Math.round(min / 60)}h`
  return `${Math.round(min / 1440)}d`
}

const cut = (text: string, width: number) =>
  width <= 1 ? '' : text.length <= width ? text.padEnd(width) : `${text.slice(0, width - 1)}…`

const shellQuote = (text: string) => `'${text.replaceAll("'", `'\\''`)}'`

const isFailing = (pr: Pr) => pr.ci === 'FAILURE' || pr.ci === 'ERROR'
const isApproved = (pr: Pr) => pr.review === 'APPROVED'
const isOldReview = (pr: Pr) => ageDays(pr.updatedAt) > OLD_REVIEW_DAYS
const isStalePr = (pr: Pr) => ageDays(pr.updatedAt) > STALE_PR_DAYS

// Two plain glyphs for a PR: CI (● green, ✗ failing, ◌ running, · none), then review (✓ approved, ! changes, · pending).
const prIcons = (pr: Pr) => {
  const ci = isFailing(pr) ? '✗' : pr.ci === 'SUCCESS' ? '●' : pr.ci ? '◌' : '·'
  const rv = isApproved(pr) ? '✓' : pr.review === 'CHANGES_REQUESTED' ? '!' : '·'
  return `${ci}${rv}`
}

const ticketKey = (branch: string) => branch.match(/[a-z]+-\d+/i)?.[0]?.toUpperCase()

// The PR of mine whose branch carries the ticket key, e.g. me/eng-42-login-form for ENG-42.
const prForTicket = (data: Inbox, ticket: Ticket) =>
  data.mine.find(pr => pr.branch?.toLowerCase().includes(ticket.id.toLowerCase()))

const isDeploying = (t: Ticket) => t.statusType === 'started' && /deploy/i.test(t.status)

// ---------------------------------------------------------------------------
// Host plumbing

const herdr = ($: $, args: string[]) => $.process.run(['herdr', ...args], { timeoutMs: 10_000 })

const isInHerdr = async ($: $) => (await $.env.get('HERDR_ENV')) === '1'

// The pane, tab and space this session sits in now; a pane moved to another space gets a new id.
const herdrHere = async ($: $) => {
  if (!(await isInHerdr($))) return undefined
  const { exitCode, stdout } = await herdr($, ['pane', 'current', '--current'])
  if (exitCode !== 0) return undefined
  const pane = JSON.parse(stdout)?.result?.pane as { pane_id: string; tab_id: string; workspace_id: string } | undefined
  return pane && { pane: pane.pane_id, tab: pane.tab_id, space: pane.workspace_id }
}

// herdrHere for every tool call would be one herdr call each: the answer is kept a minute.
let hereCache: { at: number; value: Awaited<ReturnType<typeof herdrHere>> } | undefined
const hereSoon = async ($: $) => {
  if (!hereCache || Date.now() - hereCache.at > 60_000) hereCache = { at: Date.now(), value: await herdrHere($) }
  return hereCache.value
}

const inboxSpace = async ($: $) => {
  if (!(await isInHerdr($))) return undefined
  const { exitCode, stdout } = await herdr($, ['workspace', 'list'])
  if (exitCode !== 0) return undefined
  const spaces = JSON.parse(stdout)?.result?.workspaces as { workspace_id: string; label: string }[] | undefined
  return spaces?.find(w => w.label.endsWith('  inbox'))?.workspace_id
}

// This session's space: its id, its label without the "[N] " prefix, and whether it is the one in view.
const spaceOf = async ($: $) => {
  const here = await herdrHere($)
  if (!here) return undefined
  const { exitCode, stdout } = await herdr($, ['workspace', 'list'])
  if (exitCode !== 0) return undefined
  const spaces = JSON.parse(stdout)?.result?.workspaces as { workspace_id: string; label: string; focused: boolean }[] | undefined
  const space = spaces?.find(w => w.workspace_id === here.space)
  return space && { id: space.workspace_id, label: space.label.replace(/^\[\d+\]\s*/, ''), isFocused: space.focused }
}

const spaceLabel = async ($: $, id: string) => {
  const { exitCode, stdout } = await herdr($, ['workspace', 'list'])
  if (exitCode !== 0) return undefined
  const spaces = JSON.parse(stdout)?.result?.workspaces as { workspace_id: string; label: string }[] | undefined
  return spaces?.find(w => w.workspace_id === id)?.label.replace(/^\[\d+\]\s*/, '')
}

const FINISHED_AFTER_MS = 30_000

// A long turn that ended while its space was out of view: a notification whose click leads back to it.
const notifyFinished = async ($: $, durationMs: number, answer: string) => {
  if (durationMs < FINISHED_AFTER_MS) return
  const space = await spaceOf($)
  if (!space || space.isFocused) return
  const line = answer.split('\n').map(l => l.replace(/[*`#>_]/g, '').trim()).find(l => l.length > 0) ?? 'Done.'
  await notify($, `✓ ${space.label}`, line.length > 140 ? `${line.slice(0, 139)}…` : line, `workspace:${space.id}`)
}

const exists = async ($: $, path: string) => {
  try {
    return (await $.fs.stat(path)).kind === 'dir'
  } catch {
    return false
  }
}

// Repos whose team and bot review requests are hidden (muted.json, shared with the inbox app).
const mutedFile = async ($: $) => `${await home($)}/.config/cockpit/muted.json`

const loadMuted = async ($: $) => {
  try {
    const list = JSON.parse(await $.fs.read(await mutedFile($))) as string[]
    await update($, muted, () => list)
  } catch {
    await update($, muted, () => [])
  }
}

const setMuted = async ($: $, repo: string, isMuted: boolean) => {
  const list = await update($, muted, current =>
    isMuted ? [...new Set([...current, repo])].sort() : current.filter(r => r !== repo),
  )
  await $.fs.write(await mutedFile($), JSON.stringify(list))
  await reportDashboard($)
}

const toggle = ($: $, group: string) =>
  update($, expanded, list => (list.includes(group) ? list.filter(g => g !== group) : [...list, group]))

// ---------------------------------------------------------------------------
// Which repo this session works in, followed from the paths its tools touch

const trackWork = async ($: $, args: Record<string, unknown>) => {
  const text = ['command', 'file_path', 'path', 'notebook_path']
    .map(key => args[key])
    .filter((v): v is string => typeof v === 'string')
    .join(' ')
  const root = (await loadConfig($)).reposDir
  const tilde = root.startsWith(await home($)) ? `~${root.slice((await home($)).length)}` : root
  const under = [root, tilde].map(p => p.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')).join('|')
  const absolute = [...text.matchAll(new RegExp(`(?:${under})/([A-Za-z0-9._-]+)`, 'g'))].map(m => m[1])
  const relative = [...text.matchAll(/(?:^|[;&|]\s*)cd\s+([A-Za-z0-9._-]+)(?=[\s;&|/]|$)/g)].map(m => m[1])
  const repo = [...absolute, ...relative].filter((r): r is string => !!r).at(-1)
  if (!repo || repo === (await read($, work))?.repo) return
  const dir = `${root}/${repo}`
  if (await exists($, `${dir}/.git`)) await update($, work, () => ({ repo, dir }))
}

const branchOf = async ($: $, current: Work) => {
  const { exitCode, stdout } = await $.process.run(['git', '-C', current.dir, 'branch', '--show-current'])
  const branch = exitCode === 0 ? stdout.trim() : ''
  return ['main', 'master', 'develop', ''].includes(branch) ? undefined : branch
}

// ---------------------------------------------------------------------------
// Sidebar dashboard: counts and limits as tokens on the inbox space's rows

let lastDashboard = ''

const readLimits = async ($: $): Promise<Limit[]> => {
  try {
    const { at, limits } = JSON.parse(await $.fs.read(`${await cacheDir($)}/limits.json`)) as { at: number; limits: Limit[] }
    return Date.now() - at < 6 * 3600_000 ? limits : []
  } catch {
    return []
  }
}

const reportDashboard = async ($: $) => {
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
  const args = ['workspace', 'report-metadata', space, '--source', 'cockpit']
  for (const [name, value] of tokens) args.push(...(value === undefined ? ['--clear-token', name] : ['--token', `${name}=${value}`]))
  if ((await herdr($, args)).exitCode === 0) lastDashboard = key
}

// ---------------------------------------------------------------------------
// Context, limits and current work: status line, agent rows, shared limits file

const reportUsage = async ($: $, e: SessionMeasureInput) => {
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

  const here = await herdrHere($)
  if (!here) return
  const args = ['pane', 'report-metadata', here.pane, '--source', 'cockpit']
  const set = (name: string, value: string | undefined) =>
    args.push(...(value === undefined ? ['--clear-token', name] : ['--token', `${name}=${value}`]))

  const ctx = context.percent
  const level = ctx === undefined ? undefined : ctx >= 70 ? 'ctx_hot' : ctx >= 45 ? 'ctx_warn' : 'ctx_ok'
  for (const name of ['ctx_ok', 'ctx_warn', 'ctx_hot']) {
    set(name, name === level && ctx !== undefined ? `${glyph(ctx)} ${Math.round(ctx)}%` : undefined)
  }
  set('cost', undefined)

  // Third agent row: repo · ticket, coloured by the CI of that branch's PR.
  const current = await read($, work)
  const branch = current ? await branchOf($, current) : undefined
  const pr = current && branch ? (await read($, inbox))?.mine.find(p => p.repo === current.repo && p.branch === branch) : undefined
  const label = current ? [current.repo, branch && (ticketKey(branch) ?? branch.replace(/^[^/]+\//, '').slice(0, 14))].filter(Boolean).join(' · ') : undefined
  const run = current && branch ? (await readCi($))?.runs.find(r => r.repo === current.repo && r.branch === branch) : undefined
  const text = label && `${ICON.branch} ${label}${run?.isRunning ? `  ${ciMark((await loadConfig($)).ci.name)} ${run.done}/${run.total}` : ''}`
  const tone = !text ? undefined : run?.isRunning ? 'work_run' : pr && isFailing(pr) ? 'work_fail' : 'work'
  for (const name of ['work', 'work_run', 'work_fail', 'work_ok']) set(name, name === tone ? text : undefined)
  // Two agents on one branch overwrite each other: both rows say so.
  const other = current && branch ? await clashWith($, current.repo, branch, here.space) : undefined
  const otherLabel = other ? (await spaceLabel($, other)) ?? other : undefined
  set('clash', otherLabel ? `⚠ same branch as ${otherLabel}` : undefined)
  await herdr($, args)
}

const GITHUB_PR = /https:\/\/github\.com\/[^/\s]+\/[^/\s]+\/pull\/\d+/g

const lastMatch = (text: string, pattern: RegExp) => [...text.matchAll(pattern)].at(-1)?.[0]

const conversationText = async ($: $) =>
  (await $.session.messages())
    .slice(-40)
    .map(m => `${m.text} ${m.toolUses.map(t => `${JSON.stringify(t.input)} ${(t.text ?? '').slice(0, 2000)}`).join(' ')}`)
    .join('\n')

// This session's views for the footer, redrawn only when one appears, opens or hides.
const refreshViews = async ($: $) => {
  const next = await viewsOf($, await $.session.id())
  if (JSON.stringify(next) !== JSON.stringify(await read($, views))) await update($, views, () => next)
}

// What this conversation works on: the ticket it names last (else its branch's), and the links it cites.
const refreshLinks = async ($: $, prompt = '') => {
  const text = `${await conversationText($)}\n${prompt}`
  const data = await read($, inbox)
  const current = await read($, work)
  const branch = current ? await branchOf($, current) : undefined
  const { tickets } = await loadConfig($)
  const id = tickets.enabled ? (ticketsIn(text, data, tickets.keys).at(-1) ?? (branch ? ticketKey(branch) : undefined)) : undefined
  // Without the ticket itself, its URL comes from the template or an assigned ticket's.
  const guessed = id ? (ticketUrl(tickets, id, data?.tickets) ?? '') : ''
  const fromTicket = id ? ((await linksFor($, id, data).catch(() => null)) ?? { id, title: '', url: guessed, at: 0 }) : null
  const pr = lastMatch(text, GITHUB_PR)
  const found: TicketLinks = {
    id: id ?? '',
    title: fromTicket?.title ?? '',
    url: fromTicket?.url ?? '',
    pr: pr ? { url: pr, number: Number(pr.split('/').pop()) } : fromTicket?.pr,
    // The newest page of each site the conversation names wins over the ticket's own.
    cited: { ...fromTicket?.cited, ...citedIn([text], 'last') },
    at: Date.now(),
  }
  const repo = current?.repo ?? found.pr?.url.match(/github\.com\/[^/]+\/([^/]+)\/pull/)?.[1]
  const prBranch = data?.mine.find(p => p.url === found.pr?.url)?.branch
  Object.assign(found, await dashboardFor($, repo, branch ?? prBranch, text, data).catch(() => ({})))
  const isEmpty = !found.id && Object.keys(found.cited ?? {}).length === 0 && !found.pr && !found.dashboard && !found.preview
  await update($, ticket, () => id ?? null)
  await update($, links, () => (isEmpty ? null : found))
}

// Nerd Font glyphs present in FiraCode Nerd Font 3.0. Other sites' marks are in sites.ts.
const ICON = {
  review: '\uf441', ok: '\uf49e', fail: '\uf52f', ticket: '\uf41b', gauge: '\u{f029a}', branch: '\uf418',
  pr: '\uf407', view: '\uf080',
  argo: '\u{f0833}', dashboard: '\uf0e4', workflow: '\uf52e', app: '\uf0ac', external: '\uf08e',
}

// A rounded pill: the caps take the fill colour as their foreground.
const PILL = { main: { fill: '#FFC799', ink: '#101010' }, quiet: { fill: '#2C2C30', ink: '#D2D2D2' } }

const glyph = (percent: number) => (percent < 12.5 ? '○' : percent < 37.5 ? '◔' : percent < 62.5 ? '◑' : percent < 87.5 ? '◕' : '●')

// ---------------------------------------------------------------------------
// Inbox: GitHub + tickets, cached on disk so every session shares one fetch

type GqlPr = {
  __typename?: string
  number?: number
  title?: string
  url?: string
  isDraft?: boolean
  updatedAt?: string
  headRefName?: string
  baseRefName?: string
  repository?: { name: string }
  author?: { __typename: string; login: string; name?: string | null } | null
  reviewDecision?: string | null
  commits?: { nodes: { commit: { statusCheckRollup: { state: string; contexts?: { nodes: CheckNode[] } } | null } }[] }
}

type CheckNode = { name?: string; conclusion?: string | null; context?: string; state?: string }

const FAILED = ['FAILURE', 'ERROR', 'TIMED_OUT', 'CANCELLED', 'STARTUP_FAILURE']

const toPr = (node: GqlPr, directUrls: Set<string>): Pr => ({
  repo: node.repository?.name ?? '?',
  number: node.number ?? 0,
  title: node.title ?? '',
  url: node.url ?? '',
  author: node.author?.login ?? '?',
  authorName: node.author?.name ?? '',
  updatedAt: node.updatedAt ?? new Date(0).toISOString(),
  isDraft: node.isDraft ?? false,
  isDirect: directUrls.has(node.url ?? ''),
  review: node.reviewDecision ?? null,
  ci: node.commits?.nodes[0]?.commit.statusCheckRollup?.state ?? null,
  branch: node.headRefName,
  base: node.baseRefName,
  failedChecks: (node.commits?.nodes[0]?.commit.statusCheckRollup?.contexts?.nodes ?? [])
    .filter(c => FAILED.includes(c.conclusion ?? c.state ?? ''))
    .map(c => c.name ?? c.context ?? '?'),
})

const fetchGithub = async ($: $) => {
  const { github } = await loadConfig($)
  if (!github.enabled) return null
  const { exitCode, stdout, stderr } = await $.process.run(['gh', 'api', 'graphql', '-f', `query=${query(github.org)}`], {
    timeoutMs: 30_000,
  })
  if (exitCode !== 0) throw new Error(`gh: ${stderr.trim().split('\n')[0]}`)
  const data = JSON.parse(stdout).data as {
    review: { nodes: GqlPr[] }
    direct: { nodes: { url?: string }[] }
    mine: { nodes: GqlPr[] }
    merged: {
      nodes: { number?: number; url?: string; headRefName?: string; mergedAt?: string; repository?: { name: string; nameWithOwner: string }; mergeCommit?: { oid: string } | null }[]
    }
  }
  const directUrls = new Set(data.direct.nodes.map(n => n.url ?? ''))
  const prs = data.review.nodes.filter(n => n.__typename === 'PullRequest')
  const isBot = (n: GqlPr) => n.author?.__typename === 'Bot'
  const humans = prs.filter(n => !isBot(n)).map(n => toPr(n, directUrls))
  return {
    direct: humans.filter(p => p.isDirect),
    team: humans.filter(p => !p.isDirect),
    botRepos: prs.filter(isBot).map(n => n.repository?.name ?? '?'),
    mine: data.mine.nodes.filter(n => n.__typename === 'PullRequest').map(n => toPr(n, directUrls)),
    merged: data.merged.nodes
      .filter(n => n.mergeCommit && n.headRefName)
      .map((n): MergedPr => ({
        repo: n.repository?.name ?? '?',
        slug: n.repository?.nameWithOwner ?? '?',
        url: n.url ?? '',
        number: n.number ?? 0,
        branch: n.headRefName ?? '',
        mergeSha: n.mergeCommit?.oid ?? '',
        mergedAt: n.mergedAt ?? '',
      })),
  }
}

type News = { text: string; action: string; buttons?: [string, string][] }

// What changed since the previous fetch, worth a notification, each with where its click leads.
const changes = (prev: Inbox, next: Inbox) => {
  const out: News[] = []
  const before = new Map(prev.mine.map(pr => [pr.url, pr]))
  const known = new Set(prev.direct.map(pr => pr.url))
  for (const pr of next.direct) {
    if (!known.has(pr.url)) out.push({ text: `👀 Review requested: ${pr.repo}#${pr.number} ${pr.title}`, action: pr.url, buttons: [['Review', `act:review:${pr.url}`]] })
  }
  for (const pr of next.mine) {
    const was = before.get(pr.url)
    if (!was) continue
    if (isFailing(pr) && !isFailing(was)) {
      out.push({ text: `🔴 CI failing: ${pr.repo}#${pr.number}`, action: `${pr.url}/checks`, buttons: [['Why?', `act:ci:${pr.url}`], ['Rerun', `act:rerun:${pr.url}`]] })
    }
    if (pr.review !== was.review && pr.review === 'APPROVED') out.push({ text: `✅ Approved: ${pr.repo}#${pr.number}`, action: pr.url })
    if (pr.review !== was.review && pr.review === 'CHANGES_REQUESTED') out.push({ text: `❌ Changes requested: ${pr.repo}#${pr.number}`, action: `${pr.url}/files` })
  }
  if (prev.tickets.length > 0) {
    const ids = new Set(prev.tickets.map(t => t.id))
    for (const t of next.tickets) if (!ids.has(t.id)) out.push({ text: `🎫 Assigned: ${t.id} ${t.title}`, action: t.url, buttons: [['Plan', `act:plan:${t.id}`]] })
  }
  return out
}

let inflight: Promise<void> | undefined

const loadCached = async ($: $) => {
  try {
    const cached = JSON.parse(await $.fs.read(`${await cacheDir($)}/inbox.json`)) as Inbox
    const current = await read($, inbox)
    if (current?.at !== cached.at) await update($, inbox, () => cached)
    return cached
  } catch {
    return null
  }
}

const refresh = ($: $, isForced: boolean) => {
  inflight ??= (async () => {
    try {
      await loadMuted($)
      const cached = await loadCached($)
      const isCurrentShape = (cached?.merged !== undefined && cached.previews !== undefined && cached.tickets.every(t => t.project !== undefined)) ?? false
      // The herdr inbox app asks for a refresh by touching this file.
      const asked = await $.fs.stat(`${await cacheDir($)}/refresh-request`).then(s => s.mtimeMs, () => 0)
      const isAsked = cached !== null && asked > cached.at
      if (!isForced && !isAsked && cached && isCurrentShape && Date.now() - cached.at < STALE_MS) return
      await update($, isBusy, () => true)
      await withLock($, 'inbox', () => fetchInbox($, cached))
    } finally {
      await update($, isBusy, () => false)
      await reportDashboard($).catch(() => undefined)
      inflight = undefined
    }
  })()
  return inflight
}

const fetchInbox = async ($: $, cached: Inbox | null) => {
  const errors: string[] = []
  const [gh, tickets] = await Promise.all([
    fetchGithub($).catch((err: Error) => (errors.push(err.message), null)),
    myTickets($).catch(async (err: Error) => {
      const { name } = (await loadConfig($)).tickets
      const isConnecting = err.message.includes('no connected MCP tool')
      errors.push(isConnecting ? `${name} connector still connecting, retrying in a minute` : `${name} ${shortError(err)}`)
      return null
    }),
  ])
  // A failed source is retried on the next minute tick instead of waiting out the cache.
  const next: Inbox = {
    at: errors.length > 0 ? Date.now() - STALE_MS + 60_000 : Date.now(),
    direct: gh?.direct ?? cached?.direct ?? [],
    team: gh?.team ?? cached?.team ?? [],
    botRepos: gh?.botRepos ?? cached?.botRepos ?? [],
    mine: gh?.mine ?? cached?.mine ?? [],
    tickets: tickets ?? cached?.tickets ?? [],
    errors,
    merged: gh?.merged ?? cached?.merged ?? [],
  }
  next.deploys = await deploysFor($, next.tickets, next.merged ?? []).catch((err: Error) => {
    errors.push(`Deploy status ${shortError(err)}`)
    return cached?.deploys ?? {}
  })
  const deployRepos = [...next.mine.map(pr => pr.repo), ...Object.values(next.deploys ?? {}).flat().map(d => d.repo)]
  next.deployLinks = await deployLinksFor($, deployRepos).catch(() => cached?.deployLinks ?? {})
  next.previews = await previewsFor($).catch((err: Error) => {
    errors.push(`Previews ${shortError(err)}`)
    return cached?.previews ?? []
  })
  await $.fs.write(`${await cacheDir($)}/inbox.json`, JSON.stringify(next))
  await update($, inbox, () => next)
  await announcePreviews($, next.previews ?? []).catch(() => undefined)

  const live: News[] = []
  for (const { text, ticket: id } of cached ? deployChanges(cached, next) : []) {
    await startDeployWatch($, id, next.deploys?.[id] ?? []).catch(() => undefined)
    // A ticket with a runbook gets it refreshed instead: its notification follows, opening the steps that turned done.
    if (await requestRunbookRefresh($, id, '🚀 live in production')) continue
    live.push({ text, action: await ticketAction($, id, next.tickets.find(t => t.id === id)?.url) })
  }
  const news = cached && gh ? [...changes({ ...cached, tickets: tickets ? cached.tickets : [] }, next), ...live] : []
  if (news.length > 3) await notify($, `📥 ${news.length} updates`, news.slice(0, 3).map(n => n.text).join('\n') + '\n…', 'space:inbox')
  else for (const n of news) await notify($, '📥 Inbox', n.text, n.action, n.buttons)
}

// Asks the runbook app for a background refresh of the ticket's runbook; false when there is no runbook or no app running.
const requestRunbookRefresh = async ($: $, id: string, reason: string) => {
  const slug = id.toLowerCase()
  const hasBook = await $.fs.stat(`${await home($)}/.config/cockpit/runbooks/${slug}.md`).then(() => true, () => false)
  const isRunning = (await $.process.run(['pgrep', '-f', 'runbook\\.py cockpit-space'])).exitCode === 0
  if (!hasBook || !isRunning) return false
  const path = `${await cacheDir($)}/runbook-refresh-requests.json`
  const queued = JSON.parse(await $.fs.read(path).catch(() => '[]')) as { slug: string; reason: string }[]
  await $.fs.write(path, JSON.stringify([...queued.filter(r => r.slug !== slug), { slug, reason }]))
  return true
}

// ---------------------------------------------------------------------------
// Actions: peek in a herdr popup, open in the browser, or start Claude in a new space

const peek = async ($: $, pr: Pr) => {
  const opened = await herdr($, ['plugin', 'pane', 'open', '--plugin', 'cockpit', '--entrypoint', 'peek', '--placement', 'popup', '--env', `PEEK_URL=${pr.url}`])
  if (opened.exitCode !== 0) await openUrl($, pr.url)
}

const startSpace = async ($: $, cwd: string, label: string, command: string) => {
  if (!(await isInHerdr($))) {
    await update($, note, () => `not in herdr; run: cd ${cwd} && ${command}`)
    return
  }
  const created = await herdr($, ['workspace', 'create', '--cwd', cwd, '--label', label, '--focus'])
  const pane = JSON.parse(created.stdout || '{}')?.result?.root_pane?.pane_id as string | undefined
  if (created.exitCode !== 0 || !pane) {
    await update($, note, () => `herdr workspace create failed: ${created.stderr.trim()}`)
    return
  }
  await herdr($, ['pane', 'run', pane, command])
  await update($, note, () => `started "${label}"`)
}

// The page terminal-browser shows beside this session, for the model to know what "this page" is; undefined without one.
type Browser = { tabs?: { url: string; title?: string; active?: boolean }[] }

// The terminal-browser browsers in this session's tab (an agent's checks, browser views); none outside herdr.
const browsersBeside = async ($: $, here: { pane: string; tab: string }): Promise<Browser[]> => {
  const listed = await $.process.run(['sh', '-c', 'PATH="/opt/homebrew/bin:/usr/local/bin:$HOME/.local/bin:$PATH" exec terminal-browser ls --json'], {
    env: { HERDR_PANE_ID: here.pane, HERDR_TAB_ID: here.tab },
    timeoutMs: 2_000,
  }).catch(() => undefined)
  if (listed?.exitCode !== 0) return []
  return (JSON.parse(listed.stdout) as { browsers?: Browser[] }).browsers ?? []
}

const pageBeside = async ($: $) => {
  const here = await hereSoon($)
  return here ? (await browsersBeside($, here)).flatMap(b => b.tabs ?? []).find(t => t.active) : undefined
}

let lastRow = ''

let seenAt = Date.now()

// A browser beside this session closes once its space has been out of view for browserIdleMinutes, and no turn runs,
// so a forgotten page stops drawing; terminal-browser keeps the sign-ins for the next one.
const closeIdleBrowsers = async ($: $) => {
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
  const panes = ((JSON.parse(listed.stdout || '{}')?.result?.panes ?? []) as { pane_id: string; tab_id: string }[])
    .filter(p => p.tab_id === here.tab && p.pane_id !== here.pane)
  let closed = 0
  for (const pane of panes) {
    const info = await herdr($, ['pane', 'process-info', '--pane', pane.pane_id])
    const running = (JSON.parse(info.stdout || '{}')?.result?.process_info?.foreground_processes ?? []) as { cmdline?: string }[]
    if (!running.some(p => /terminal-browser/.test(p.cmdline ?? ''))) continue
    if ((await herdr($, ['pane', 'close', pane.pane_id])).exitCode === 0) closed++
  }
  if (closed > 0) await reportSide($)
}

const clip = (text: string, width: number) => (text.length <= width ? text : `${text.slice(0, width - 1).trimEnd()}…`)

// A dashboard link's mark: Argo's for an ArgoCD page, a gauge for any other.
const dashboardIcon = (url: string) => (dashboardName(url) === 'ArgoCD' ? ICON.argo : ICON.dashboard)

// A page's mark on the agent row, from its site, like a favicon in a browser's tab strip.
const siteIcon = (url: string) => {
  const host = url.match(/^https?:\/\/([^/]+)/)?.[1] ?? ''
  if (host.endsWith('github.com')) return '\uf09b'
  if (host.endsWith('linear.app')) return ICON.ticket
  if (host.startsWith('argocd.')) return ICON.argo
  return siteMark(url) ?? ICON.app
}

// The agent row, split like Zen's split view when panes sit beside the session: the agent's topic on the left, the
// page beside it on the right, "+N" for the other tabs and panes. herdr draws $agent, $side_sep and $side.
const reportSide = async ($: $) => {
  const here = await hereSoon($)
  if (!here) return
  const listed = await herdr($, ['pane', 'list', '--workspace', here.space])
  const panes = ((JSON.parse(listed.stdout || '{}')?.result?.panes ?? []) as { pane_id: string; tab_id: string; tokens?: Record<string, string> }[])
    .filter(p => p.tab_id === here.tab)
  const topic = panes.find(p => p.pane_id === here.pane)?.tokens?.topic || ((await spaceLabel($, here.space)) ?? '')
  const browsers = await browsersBeside($, here)
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
    : [['agent', topic], ['side_sep', undefined], ['side', undefined]]
  const key = JSON.stringify(tokens)
  if (key === lastRow) return
  const args = ['pane', 'report-metadata', here.pane, '--source', 'cockpit']
  for (const [name, value] of tokens) args.push(...(value === undefined ? ['--clear-token', name] : ['--token', `${name}=${value}`]))
  if ((await herdr($, args)).exitCode === 0) lastRow = key
}

let toldPage = ''

// Once per page: the next prompt carries the page beside the session, so "this page" means it.
const pageContext = async ($: $) => {
  const page = await pageBeside($).catch(() => undefined)
  if (!page || page.url === toldPage) return []
  toldPage = page.url
  return [`The browser beside this conversation shows "${page.title || page.url}" at ${page.url}. ` +
    'When the person says "this page" they mean it; read or drive it with `terminal-browser action -- snapshot` (the side-browser skill).']
}

const repoDir = async ($: $, pr: Pr) => {
  const root = (await loadConfig($)).reposDir
  const dir = `${root}/${pr.repo}`
  return (await exists($, dir)) ? { cwd: dir, target: String(pr.number) } : { cwd: root, target: pr.url }
}

const reviewPr = async ($: $, pr: Pr) => {
  const { cwd, target } = await repoDir($, pr)
  await startSpace($, cwd, `review ${pr.repo}#${pr.number}`, `claude ${shellQuote(`/code-review ${target}`)}`)
}

const ciPrompt = (pr: Pr, run: CiRun | undefined) => {
  const job = run?.failed
    ? ` The failing job is ${run.failed.job}${run.failed.step ? `, step "${run.failed.step}"` : ''}: read its logs with ${run.failed.logs}.`
    : ''
  const others = (pr.failedChecks ?? []).filter(c => c !== run?.failed?.job)
  const checks = others.length ? ` Failing GitHub checks: ${others.join(', ')} (gh pr checks, gh run view --log-failed).` : ''
  return `CI is failing on ${pr.url}.${job}${checks} Explain the cause and propose a fix. Do not push.`
}

const investigateCi = async ($: $, pr: Pr) => {
  const { cwd } = await repoDir($, pr)
  const run = (await readCi($))?.runs.find(r => r.repo === pr.repo && r.branch === pr.branch)
  const prompt = ciPrompt(pr, run)
  await startSpace($, cwd, `ci ${pr.repo}#${pr.number}`, `claude ${shellQuote(prompt)}`)
}

const startTicket = async ($: $, ticket: Ticket) => {
  const prompt = `Pick up ${ticket.id} (${ticket.url}): read the ticket, find the repos involved, and propose a plan before coding.`
  await startSpace($, (await loadConfig($)).reposDir, ticket.id.toLowerCase(), `claude ${shellQuote(prompt)}`)
}

const openInbox = ($: $) => $.ui.open({ id: PANE, title: 'Inbox' })

// ---------------------------------------------------------------------------

export const register: Register = on => {
  registerSettings(on)

  on('session.start', async ($, e, next) => {
    await $.command.register({ name: 'inbox', description: 'Toggle the PR review / ticket inbox pane' })
    await $.command.register({ name: 'todo', description: 'Add to the to-do list in the herdr to-do space; alone, list it', argumentHint: '[text]' })
    await registerSettingsCommand($)
    await $.tool.register(REPORT_SPEC).catch(() => undefined)
    await $.tool.register(SHOW_SPEC).catch(() => undefined)
    $.clock.every(60_000, () => void refresh($, false).then(() => maybeBrief($)).catch(() => undefined))
    $.clock.every(60_000, () => void refreshCi($).catch(() => undefined))
    $.clock.every(60_000, () => void refreshMeetings($).catch(() => undefined))
    void refreshMeetings($).catch(() => undefined)
    // A view hidden with q in its pane, or closed with herdr, turns its chip quiet within a few seconds.
    $.clock.every(3_000, () => void refreshViews($).catch(() => undefined))
    $.clock.every(10_000, () => void reportSide($).catch(() => undefined))
    $.clock.every(60_000, () => void closeIdleBrowsers($).catch(() => undefined))
    // An answer given in a pane reaches the session as the person's next message.
    $.clock.every(2_000, () => void $.session.id().then(id => collectAnswers($, id)).catch(() => undefined))
    // A chart or table with a Metabase or Datadog source refreshes while its pane is open.
    $.clock.every(15_000, () => void $.session.id().then(id => refreshSources($, id)).catch(() => undefined))
    $.clock.every(5_000, () => void processRequests($, () => void refreshCi($, true).catch(() => undefined)))
    const beat = async () => {
      const current = await read($, work)
      await heartbeat($, await hereSoon($), current ? { repo: current.repo, branch: await branchOf($, current) } : undefined)
    }
    $.clock.every(60_000, () => void beat().catch(() => undefined))
    void beat().catch(() => undefined)
    $.clock.every(3600_000, () => void checkUpdate($).catch(() => undefined))
    void checkUpdate($).catch(() => undefined)
    void refreshCi($).catch(() => undefined)
    void refresh($, false)
    $.ui.status(undefined)
    void refreshLinks($).catch(() => undefined)
    void reportSide($).catch(() => undefined)
    // Draw the agent row now, not after the next turn. A resumed session has no
    // figure until its first response, so the window is estimated locally until then.
    void (async () => {
      const u = await $.session.usage()
      const percent = u.context.percent ?? (await $.session.usage({ breakdown: 'summary' })).context.breakdown?.percentage
      await reportUsage($, { ...u, context: { ...u.context, percent }, changed: [] })
    })().catch(() => undefined)

    return next(e)
  })

  // The to-do list is ~/.config/cockpit/todo.md, Markdown checkboxes the herdr to-do app also edits.
  on('command.run', { command: 'todo' }, async ($, e) => {
    const path = `${await home($)}/.config/cockpit/todo.md`
    const body = await $.fs.read(path).catch(() => '# To-do\n\n')
    const items = body.split('\n').filter(l => /^- \[( |x)\] /.test(l))
    const text = e.args.trim()
    if (!text) {
      const open = items.filter(l => l.startsWith('- [ ] ')).map(l => l.slice(6))
      return { text: open.length > 0 ? open.map(t => `• ${t}`).join('\n') : 'Nothing on the to-do list.' }
    }
    await $.fs.write(path, ['# To-do', '', `- [ ] ${text}`, ...items, ''].join('\n'))
    return { text: `Added to the to-do list: ${text}` }
  })

  // /runbook is the plugin's markdown command (commands/runbook.md), a prompt the engine runs itself.
  // This hook only records which session ran it, the one the runbook app hands steps to.
  on('command.run', { command: ['runbook', 'cockpit:runbook'] }, async ($, e, next) => {
    const target = e.args.trim() || (await read($, ticket)) || ''
    const here = await herdrHere($).catch(() => undefined)
    if (/^[A-Za-z]+-\d+$/.test(target) && here) {
      const owners = `${await cacheDir($)}/runbook-owners.json`
      const map = JSON.parse(await $.fs.read(owners).catch(() => '{}')) as Record<string, unknown>
      await $.fs.write(owners, JSON.stringify({ ...map, [target.toLowerCase()]: { pane: here.pane, session: await $.session.id() } }))
    }
    return next(e)
  })
    .catch(($, e, next) => next(e))

  on('command.run', { command: 'inbox' }, async ($, e) => {
    const isOpen = (await $.ui.panes()).some(p => p.id === PANE)
    if (isOpen && e.args.trim() !== 'refresh') {
      await $.ui.close({ id: PANE })
      return { text: 'Inbox closed.' }
    }
    await openInbox($)
    void refresh($, e.args.trim() === 'refresh')
    return { text: 'Inbox opened.' }
  })

  on('tool.call', { tool: REPORT_TOOL_FULL }, async ($, e) => reportStep($, e as unknown as Record<string, unknown>))
  // It writes cockpit's own files and one runbook status cell, so it needs no permission prompt.
  on('tool.check', { tool: REPORT_TOOL_FULL }, async () => ({ decision: 'allow' }))
  on('tool.call', { tool: SHOW_TOOL_FULL }, async ($, e) => {
    const answer = await show($, e as unknown as Record<string, unknown>, await herdrHere($), await $.session.cwd())
    await refreshViews($).catch(() => undefined)
    void reportSide($).catch(() => undefined)
    return answer
  })
  // It writes the view to cockpit's cache and splits a pane beside this session; a view that runs commands asks first.
  on('tool.check', { tool: SHOW_TOOL_FULL }, async ($, e) => {
    const commands = commandsOf((e.input ?? {}) as Record<string, unknown>)
    return commands.length > 0 ? { decision: 'ask', reason: `The pane will run, again and again: ${commands.join(' ; ')}` } : { decision: 'allow' }
  })

  on('tool.call', async ($, e, next) => {
    await trackWork($, e as unknown as Record<string, unknown>).catch(() => undefined)
    const done = activityOf(String(e.tool), e as unknown as Record<string, unknown>, (await read($, work))?.dir ?? '')
    if (done && e.agentId === undefined) void hereSoon($).then(here => logActivity($, here?.space, done)).catch(() => undefined)
    return next(e)
  })

  on('turn.complete', async ($, e, next) => {
    if (e.agentId === undefined) void refreshLinks($).catch(() => undefined)
    if (e.agentId === undefined && e.reason === 'answer') void notifyFinished($, e.durationMs, e.answer).catch(() => undefined)
    if (e.agentId === undefined && e.reason === 'answer') void settleTurn($).catch(() => undefined)
    if (e.agentId === undefined && e.reason === 'answer') {
      const first = e.answer.split('\n').map(l => l.replace(/[*`#>_]/g, '').trim()).find(l => l.length > 0) ?? ''
      void hereSoon($).then(here => logActivity($, here?.space, { kind: 'turn', text: first.slice(0, 200) })).catch(() => undefined)
    }
    return next(e)
  })

  on('prompt.submit', async ($, e, next) => {
    void refreshLinks($, e.text).catch(() => undefined)
    void hereSoon($).then(here => bindHandoff($, e.text, here?.pane, e.origin?.kind === 'plugin')).catch(() => undefined)
    const page = e.command || e.origin?.kind !== 'composer' ? [] : await pageContext($)
    return next(page.length > 0 ? { ...e, context: [...(e.context ?? []), ...page] } : e)
  })

  on('ui.render', { component: 'SessionMode' }, async ($, e) => {
    const { Box, Text, Button } = $.ui.resolve(e)
    const current = await read($, usage)
    const found = await read($, links)
    const shown = await read($, views)
    const beside = await read($, sidePage)
    const tight = (current?.limits ?? []).filter(l => l.percent >= 50)
    const pill = (key: string, label: string, onPress: () => unknown, tone: keyof typeof PILL = 'quiet') => (
      <Button key={key} plain onPress={onPress}>
        <Text color={PILL[tone].fill}>{'\ue0b6'}</Text>
        <Text backgroundColor={PILL[tone].fill} color={PILL[tone].ink}>{label}</Text>
        <Text color={PILL[tone].fill}>{'\ue0b4'}</Text>
      </Button>
    )
    const chip = (key: string, label: string, url: string, tone: keyof typeof PILL = 'quiet') => pill(key, label, () => openUrl($, url), tone)
    return (
      <Box flexDirection="row" gap={1}>
        {e.props.modes.length > 0 && <Text dimColor>{e.props.modes.join(' & ')}</Text>}
        {found?.id && found.url && chip('lk-ticket', `${ICON.ticket} ${found.id}`, found.url, 'main')}
        {found?.pr && chip('lk-pr', `${ICON.pr} ${found.pr.number}`, found.pr.url)}
        {chipsFor(found?.cited ?? {}).map(site => chip(`lk-${site.kind}`, `${site.icon} ${site.label}`, site.url))}
        {found?.dashboard && chip('lk-dashboard', `${dashboardIcon(found.dashboard.url)} ${found.dashboard.label}`, found.dashboard.url)}
        {found?.workflow && chip('lk-workflow', `${ICON.workflow} ${found.workflow.label}`, found.workflow.url)}
        {found?.preview && chip('lk-preview', `${ICON.app} ${found.preview.label}`, found.preview.url)}
        {beside && pill('lk-external', `${ICON.external} browser`, () => openUrl($, beside.url))}
        {shown.map(v => pill(`lk-view-${v.name}`, `${ICON.view} ${v.name}`, () => hereSoon($).then(here => toggleView($, v.name, here)).then(() => refreshViews($)).then(() => reportSide($)), v.isOpen ? 'main' : 'quiet'))}
        {tight.map(l => (
          <Text key={`lim-${l.name}`} color={l.percent >= 80 ? 'error' : 'warning'}>
            {l.name} {Math.round(l.percent)}%
          </Text>
        ))}
      </Box>
    )
  })

  on('session.measure', async ($, e, next) => {
    await reportUsage($, e).catch(() => undefined)
    return next(e)
  })

  on('session.end', async ($, e, next) => {
    if (e.reason !== 'clear') await forget($).catch(() => undefined)
    // A /clear keeps the process: what the old conversation worked on must not follow the new one.
    if (e.reason === 'clear') {
      await update($, work, () => null)
      await update($, usage, () => null)
      await update($, ticket, () => null)
      await update($, links, () => null)
      await update($, views, () => [])
      await update($, sidePage, () => null)
    }
    const here = await herdrHere($).catch(() => undefined)
    if (here) {
      const args = ['pane', 'report-metadata', here.pane, '--source', 'cockpit']
      for (const name of ['ctx_ok', 'ctx_warn', 'ctx_hot', 'work', 'work_run', 'work_ok', 'work_fail', 'clash', 'agent', 'side_sep', 'side']) args.push('--clear-token', name)
      void herdr($, args)
    }
    return next(e)
  })

  on('ui.render', { component: 'Pane', requestId: PANE }, async ($, e) => {
    const { Box, Text, Button } = $.ui.resolve(e)
    const data = await read($, inbox)
    const busy = await read($, isBusy)
    const open = await read($, expanded)
    const message = await read($, note)
    const mutedRepos = await read($, muted)
    const ticketsName = (await loadConfig($)).tickets.name
    const width = Math.max(40, e.props.bodyColumns)
    const meta = width >= 100 ? 18 : 6

    if (!data) {
      return <Text dimColor>{busy ? 'Loading PRs and tickets…' : 'No data yet.'}</Text>
    }

    const heading = (label: string, extra: string) => (
      <Text>
        <Text bold>{label}</Text>
        <Text dimColor> {extra}</Text>
      </Text>
    )

    const gap = (key: string) => <Text key={key}> </Text>

    // A collapsed group: one dim line that toggles its rows.
    const fold = (group: string, count: number, label: string, rows: () => RenderChildren[]): RenderChildren[] =>
      count === 0
        ? []
        : [
            <Button
              key={`fold-${group}`}
              plain
              dimColor
              label={`${open.includes(group) ? '▾' : '▸'} ${count} ${label}`}
              onPress={() => toggle($, group)}
            />,
            ...(open.includes(group) ? rows() : []),
          ]

    // The row's text is one button: pressing it peeks at the PR in a herdr popup.
    const prLabel = (pr: Pr, lead: string, room: number) => (
      <Button key={`pk-${pr.url}`} plain onPress={() => peek($, pr)}>
        <Text>{lead} </Text>
        <Text color="suggestion">{cut(`${pr.repo}#${pr.number}`, 18)} </Text>
        <Text dimColor={pr.isDraft}>{cut(pr.title, room)}</Text>
      </Button>
    )

    const reviewRow = (pr: Pr) => (
      <Box key={`row-${pr.url}`} flexDirection="row">
        {prLabel(pr, pr.isDirect ? '•' : '·', width - 3 - 19 - meta - 23)}
        <Text dimColor> {cut(meta > 6 ? `${pr.author} ${ago(pr.updatedAt)}` : ago(pr.updatedAt), meta - 2)} </Text>
        <Button key={`rv-${pr.url}`} label="rev" onPress={() => reviewPr($, pr)} />
        <Button key={`op-${pr.url}`} label="web" onPress={() => openUrl($, pr.url)} />
        {!pr.isDirect && <Button key={`mu-${pr.url}`} label="mute" onPress={() => setMuted($, pr.repo, true)} />}
      </Box>
    )

    const mineRow = (pr: Pr) => (
      <Box key={`mine-${pr.url}`} flexDirection="row">
        {prLabel(pr, prIcons(pr), width - 5 - 19 - 22)}
        <Text dimColor> {cut(ago(pr.updatedAt), 4)}</Text>
        {isFailing(pr) && <Button key={`ci-${pr.url}`} label="why?" onPress={() => investigateCi($, pr)} />}
        <Button key={`op-${pr.url}`} label="web" onPress={() => openUrl($, pr.url)} />
      </Box>
    )

    const ticketRow = (t: Ticket) => {
      const pr = prForTicket(data, t)
      return (
        <Box key={`t-${t.id}`} flexDirection="row">
          <Button key={`tk-${t.id}`} plain onPress={() => openUrl($, t.url)}>
            <Text color="suggestion">{cut(t.id, 10)} </Text>
            <Text dimColor>{cut(t.status, 16)} </Text>
            <Text>{cut(t.title, width - 11 - 17 - 12 - 11)}</Text>
            <Text>{pr ? ` #${String(pr.number).padEnd(5)}${prIcons(pr)}` : ' '.repeat(12)}</Text>
          </Button>
          <Button key={`go-${t.id}`} label="start" onPress={() => startTicket($, t)} />
        </Box>
      )
    }

    // Review requests: fresh direct and team ones, muted repos and old ones folded away.
    const isShown = (repo: string) => !mutedRepos.includes(repo)
    const direct = data.direct.filter(pr => !isOldReview(pr))
    const teamAll = data.team.filter(pr => isShown(pr.repo) && !isOldReview(pr))
    const old = [...data.direct, ...data.team.filter(pr => isShown(pr.repo))].filter(isOldReview)
    const botRepos = data.botRepos ?? []
    const bots = botRepos.filter(isShown).length
    const hidden = data.team.length - data.team.filter(pr => isShown(pr.repo)).length + botRepos.length - bots
    const team = open.includes('team') ? teamAll : teamAll.slice(0, 5)

    // My PRs: what needs me first, then waiting for review, then ready to merge, then stale.
    const fresh = data.mine.filter(pr => !isStalePr(pr))
    const action = fresh.filter(pr => isFailing(pr) || pr.review === 'CHANGES_REQUESTED')
    const ready = fresh.filter(pr => isApproved(pr) && !isFailing(pr))
    const waiting = fresh.filter(pr => !action.includes(pr) && !ready.includes(pr))
    const stale = data.mine.filter(isStalePr)

    // Tickets: in progress, then backlog, with the to-deploy ones folded away.
    const working = data.tickets.filter(t => t.statusType === 'started' && !isDeploying(t))
    const deploying = data.tickets.filter(isDeploying)
    const backlog = data.tickets.filter(t => t.statusType !== 'started')

    return (
      <Box flexDirection="column">
        {heading('Review', `${direct.length} for you · ${teamAll.length} team · ${bots} bots${hidden > 0 ? ` · ${hidden} muted` : ''}`)}
        {direct.map(reviewRow)}
        {team.map(reviewRow)}
        {teamAll.length > 5 && (
          <Button
            key="team-toggle"
            plain
            dimColor
            label={open.includes('team') ? '▾ show less' : `▸ ${teamAll.length - 5} more team requests`}
            onPress={() => toggle($, 'team')}
          />
        )}
        {fold('old', old.length, `older than ${OLD_REVIEW_DAYS} days`, () => old.map(reviewRow))}
        {mutedRepos.length > 0 && (
          <Box flexDirection="row">
            <Text dimColor>muted: </Text>
            {mutedRepos.map(repo => (
              <Button key={`um-${repo}`} plain dimColor label={`${repo} ×`} onPress={() => setMuted($, repo, false)} />
            ))}
          </Box>
        )}
        {gap('gap-1')}
        {heading('My PRs', `${data.mine.length} open · CI then review`)}
        {action.length > 0 && <Text color="error">needs you</Text>}
        {action.map(mineRow)}
        {waiting.length > 0 && <Text dimColor>waiting for review</Text>}
        {waiting.map(mineRow)}
        {fold('ready', ready.length, 'ready to merge', () => ready.map(mineRow))}
        {fold('stale', stale.length, `stale, no update in ${STALE_PR_DAYS} days`, () => stale.map(mineRow))}
        {gap('gap-2')}
        {heading(ticketsName, `${data.tickets.length} assigned · ${working.length} in progress`)}
        {working.map(ticketRow)}
        {backlog.length > 0 && <Text dimColor>backlog</Text>}
        {backlog.map(ticketRow)}
        {fold('deploy', deploying.length, 'waiting for deploy', () => deploying.map(ticketRow))}
        {gap('gap-3')}
        <Box flexDirection="row">
          <Text dimColor>{busy ? 'refreshing… ' : `updated ${hhmm(new Date(data.at).toISOString())} · click a row to peek `}</Text>
          <Button key="refresh" label="refresh" onPress={() => refresh($, true)} />
        </Box>
        {data.errors.map(err => (
          <Text key={`err-${err}`} color="error">
            {err}
          </Text>
        ))}
        {message !== '' && <Text dimColor>{message}</Text>}
      </Box>
    )
  })
}
