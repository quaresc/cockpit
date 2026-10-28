import { atom, read, update } from 'claude-code'
import type { EngineInterface } from 'claude-code'

import type { Inbox, MergedPr, Pr } from '../types'

import { announcePreviews, deploysFor, notify, previewsFor, shortError, startDeployWatch, ticketAction } from './ci'
import { cacheDir, home, loadConfig } from './config'
import { requestRunbookRefresh } from './handoffs'
import { deployLinksFor } from './links'
import { hubOwns, modFetchedAt, modPart } from './feed'
import { deployChanges, prChanges, ticketChanges } from './match'
import type { News } from './match'
import { reportDashboard } from './sidebar'
import { withLease, writeDoc } from './store'
import { myTickets } from './tickets'

type $ = EngineInterface

const inbox = atom({ plugin: 'cockpit', key: 'inbox' } as const, null)
const muted = atom({ plugin: 'cockpit', key: 'muted' } as const, [])

// The inbox: GitHub + tickets, cached on disk so every session shares one fetch; the herdr inbox app draws it. The hub
// fetches GitHub (and tickets, given a Linear token) itself: while its part is fresh, the mod fetches only the rest.

const STALE_MS = 5 * 60_000
const DAY_MS = 86_400_000
// The shape of inbox.json: a cache another version wrote is fetched again at once, and the app waits for it.
export const INBOX_VERSION = 1

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

async function fetchGithub($: $) {
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

// Repos whose team and bot review requests are hidden (muted.json, which the inbox app writes).
async function loadMuted($: $) {
  try {
    const list = JSON.parse(await $.fs.read(`${await home($)}/.config/cockpit/muted.json`)) as string[]
    await update($, muted, () => list)
  } catch {
    await update($, muted, () => [])
  }
}

async function readInbox($: $) {
  try {
    return JSON.parse(await $.fs.read(`${await cacheDir($)}/inbox.json`)) as Inbox
  } catch {
    return null
  }
}

async function loadCached($: $) {
  try {
    const cached = await readInbox($)
    if (!cached) return null
    const current = await read($, inbox)
    if (current?.at !== cached.at) await update($, inbox, () => cached)
    return cached
  } catch {
    return null
  }
}

let inflight: Promise<void> | undefined

export function refresh($: $, isForced: boolean) {
  inflight ??= (async () => {
    try {
      await loadMuted($)
      const cached = await loadCached($)
      const isCurrent = cached?.version === INBOX_VERSION
      // The herdr inbox app asks for a refresh by touching this file.
      const asked = await $.fs.stat(`${await cacheDir($)}/refresh-request`).then(s => s.mtimeMs, () => 0)
      const isAsked = cached !== null && asked > modFetchedAt(cached)
      if (!isForced && !isAsked && isCurrent && Date.now() - modFetchedAt(cached) < STALE_MS) return
      // A cache from another version is not compared against, so its shape never turns into notifications.
      await withLease($, 'inbox', () => fetchInbox($, isCurrent ? cached : null))
    } finally {
      await reportDashboard($).catch(() => undefined)
      inflight = undefined
    }
  })()
  return inflight
}

async function fetchInbox($: $, cached: Inbox | null) {
  const errors: string[] = []
  const [gh, tickets] = await Promise.all([
    hubOwns(cached, 'github') ? null : fetchGithub($).catch((err: Error) => (errors.push(err.message), null)),
    hubOwns(cached, 'tickets') ? null : myTickets($).catch(async (err: Error) => {
      const { name } = (await loadConfig($)).tickets
      const isConnecting = err.message.includes('no connected MCP tool')
      errors.push(isConnecting ? `${name} connector still connecting, retrying in a minute` : `${name} ${shortError(err)}`)
      return null
    }),
  ])
  // The hub's fields as it last wrote them, beside what this fetch brought, to work out deploys and previews.
  const latest = await readInbox($)
  const kept = latest?.version === INBOX_VERSION ? latest : cached
  // A failed source is retried on the next minute tick instead of waiting out the cache.
  const part: Partial<Inbox> = { ...modPart(gh, tickets, errors.length > 0, STALE_MS), version: INBOX_VERSION }
  const seen = { direct: [], team: [], botRepos: [], mine: [], tickets: [], merged: [], errors: [], ...kept, ...part } as Inbox
  part.deploys = await deploysFor($, seen.tickets, seen.merged ?? []).catch((err: Error) => {
    errors.push(`Deploy status ${shortError(err)}`)
    return cached?.deploys ?? {}
  })
  const deployRepos = [...seen.mine.map(pr => pr.repo), ...Object.values(part.deploys ?? {}).flat().map(d => d.repo)]
  part.deployLinks = await deployLinksFor($, deployRepos).catch(() => cached?.deployLinks ?? {})
  part.previews = await previewsFor($).catch((err: Error) => {
    errors.push(`Previews ${shortError(err)}`)
    return cached?.previews ?? []
  })
  part.errors = errors
  const next = await writeDoc<Inbox>($, 'inbox', part, 'patch')
  await update($, inbox, () => next)
  await announcePreviews($, next.previews ?? []).catch(() => undefined)

  const live: News[] = []
  for (const { text, ticket: id } of cached ? deployChanges(cached, next) : []) {
    await startDeployWatch($, id, next.deploys?.[id] ?? []).catch(() => undefined)
    // A ticket with a runbook gets it refreshed instead: its notification follows, opening the steps that turned done.
    if (await requestRunbookRefresh($, id, '🚀 live in production')) continue
    live.push({ text, action: await ticketAction($, id, next.tickets.find(t => t.id === id)?.url) })
  }
  // Each side announces the changes in what it fetched: the hub its own.
  const news = cached ? [...(gh ? prChanges(cached, next) : []), ...(tickets ? ticketChanges(cached, next) : []), ...live] : []
  if (news.length > 3) await notify($, `📥 ${news.length} updates`, news.slice(0, 3).map(n => n.text).join('\n') + '\n…', 'space:inbox')
  else for (const n of news) await notify($, '📥 Inbox', n.text, n.action, n.buttons)
}
