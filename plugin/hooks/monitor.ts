import type { EngineInterface } from 'claude-code'

import type { Health, Inbox, Issue, MonitoredApp, MonitorState, Monitors, Release, Verdict } from '../types'

import { githubLogin, notify } from './ci'
import { cacheDir, home, loadConfig } from './config'
import type { Config } from './config'
import { call, can, list, pick, platform, text } from './platform'
import { withLease, writeDoc } from './store'

type $ = EngineInterface

// The monitor space's document: every app of the watched environment from the platform connector, and for those with a
// release in the last day (or pinned) their deploys, alerting monitors, open errors and a verdict on each release from
// Datadog's numbers before and after it. One session at a time refreshes it, every two minutes, when the monitor space
// is among the pinned spaces.

const REFRESH_MS = 2 * 60_000
const DAY_MS = 86_400_000
const BINS = 20
const MONITOR_PAGES = 4

type Moved = Config['monitor']['moved']

// ---------------------------------------------------------------------------
// Datadog's answers

// The JSON a Datadog tool answers, inside its <JSON_DATA> block or as the whole text.
export function dataOf(body: string): unknown {
  const inner = body.match(/<JSON_DATA>\s*([\s\S]*?)\s*<\/JSON_DATA>/)?.[1] ?? body
  return JSON.parse(inner) as unknown
}

const unquote = (value: string) => {
  const v = value.trim()
  if (v.startsWith("'") && v.endsWith("'")) return v.slice(1, -1).replace(/''/g, "'")
  if (v.startsWith('"') && v.endsWith('"')) return JSON.parse(v) as string
  return v
}

// The Error Tracking issues of a search, which answers a YAML list: the fields cockpit shows, a message to its first line.
export function parseIssues(body: string, since: number): Issue[] {
  const items: Record<string, string>[] = []
  let block: { key: string; lines: string[] } | null = null
  for (const line of body.split('\n')) {
    if (block !== null && (line.startsWith('    ') || line.trim() === '')) {
      block.lines.push(line.trim())
      continue
    }
    if (block !== null && items.length > 0) items[items.length - 1]![block.key] = block.lines.join('\n')
    block = null
    const field = /^(- | {2})([a-z_]+):\s?(.*)$/.exec(line)
    if (!field) continue
    const [, lead, key, value] = field as unknown as [string, string, string, string]
    if (lead === '- ') items.push({})
    if (value === '|' || value === '|-') block = { key, lines: [] }
    else if (items.length > 0) items[items.length - 1]![key] = unquote(value)
  }
  if (block !== null && items.length > 0) items[items.length - 1]![block.key] = block.lines.join('\n')
  return items
    .filter(i => i.issue_id)
    .map(i => ({
      id: i.issue_id!,
      type: (i.error_type ?? '').replace(/^Elixir\./, ''),
      message: (i.error_message ?? '').split('\n').find(l => l.trim()) ?? '',
      count: Number(i.total_count ?? 0),
      firstSeen: i.first_seen ?? '',
      firstVersion: i.first_seen_version ?? '',
      url: i.issue_url ?? '',
      isNew: Date.parse(i.first_seen ?? '') >= since,
    }))
}

type Binned = { start_time: string; count: number; avg: number }

// A timeseries answer as `bins` values over [from, to): a bin's sum for a count, its average otherwise.
export function bins(series: { binned?: Binned[] } | undefined, from: number, to: number, isCount: boolean, n = BINS): number[] {
  const out = new Array<number>(n).fill(0)
  const width = (to - from) / n
  for (const b of series?.binned ?? []) {
    const i = Math.min(n - 1, Math.max(0, Math.floor((Date.parse(b.start_time) - from) / width)))
    out[i] = Math.round((out[i]! + (isCount ? b.avg * b.count : b.avg)) * 1e6) / 1e6
  }
  return out
}

const rate = (h: Health) => (h.hits > 0 ? (100 * h.errors) / h.hits : 0)
const pct = (n: number) => `${n < 1 ? n.toFixed(2) : n.toFixed(1)}%`
const ms = (s: number | null) => (s === null ? '?' : `${Math.round(s * 1000)}`)
const hhmm = (iso: string) => new Date(iso).toTimeString().slice(0, 5)

// A release's verdict from its numbers: failed when its deploy did, watching until the window ends, else moved when the
// error rate, the p95 or a new error says so, ok, or unknown without traffic.
export function verdictOf(phase: string, before: Health | null, after: Health | null, newIssues: string[], until: string, now: number, moved: Moved): Verdict {
  const base = { before, after, newIssues, until }
  if (/fail|error/i.test(phase)) return { state: 'failed', summary: `deploy ${phase.toLowerCase()}`, ...base }
  const parts: string[] = []
  if (before && after) {
    parts.push(`errors ${pct(rate(before))} → ${pct(rate(after))}`)
    if (before.p95 !== null || after.p95 !== null) parts.push(`p95 ${ms(before.p95)} → ${ms(after.p95)} ms`)
  }
  if (newIssues.length > 0) parts.push(`${newIssues.length} new error${newIssues.length > 1 ? 's' : ''}`)
  if (now < Date.parse(until)) return { state: 'watching', summary: [`watching until ${hhmm(until)}`, ...parts].join(' · '), ...base }
  if (!before || !after || (before.hits === 0 && after.hits === 0)) return { state: 'unknown', summary: parts.join(' · ') || 'no traffic to compare', ...base }
  const hasTraffic = after.hits >= moved.minHits
  const errorsUp = hasTraffic && rate(after) - rate(before) >= moved.errorPoints
  const slower = hasTraffic && before.p95 !== null && after.p95 !== null && before.p95 > 0 && after.p95 / before.p95 >= moved.p95Ratio
  return { state: errorsUp || slower || newIssues.length > 0 ? 'moved' : 'ok', summary: parts.join(' · '), ...base }
}

// ---------------------------------------------------------------------------
// Fetching

async function datadog($: $, server: string, tool: string, args: Record<string, unknown>) {
  const result = await $.mcp.call(server, tool, { ...args, telemetry: { intent: 'cockpit monitor space: health of recent releases' } })
  const block = result.content.find(c => c.type === 'text')
  const body = block && 'text' in block ? String(block.text) : ''
  if (result.isError) throw new Error(`Datadog ${tool}: ${body.slice(0, 160)}`)
  return body
}

const fill = (query: string, service: string, env: string) => query.replace(/\{service\}/g, service).replace(/\{env\}/g, env)

// Errors, hits and p95 of a service over [from, to].
async function health($: $, server: string, config: Config['monitor'], service: string, from: number, to: number): Promise<Health | null> {
  const q = config.queries
  const queries = [
    { name: 'errors', query: fill(q.errors, service, config.env), aggregator: 'sum' },
    { name: 'hits', query: fill(q.hits, service, config.env), aggregator: 'sum' },
    { name: 'p95', query: fill(q.p95, service, config.env), aggregator: 'avg' },
  ]
  const args = { queries, response_format: 'scalar', from: new Date(from).toISOString(), to: new Date(to).toISOString() }
  const values = (list(dataOf(await datadog($, server, 'get_datadog_metric', args)))[0] as { values?: Record<string, number | null> } | undefined)?.values ?? {}
  const value = (i: number) => values[queries[i]!.query] ?? values[queries[i]!.name] ?? null
  if (value(1) === null && value(0) === null) return null
  return { errors: value(0) ?? 0, hits: value(1) ?? 0, p95: value(2) }
}

async function monitorsOf($: $, server: string, service: string, site: string): Promise<Monitors> {
  const found: { id: number; name: string; status: string }[] = []
  for (let page = 0; page < MONITOR_PAGES; page++) {
    const body = await datadog($, server, 'search_datadog_monitors', { query: `service:${service}`, start_at: found.length, max_tokens: 20_000 })
    const batch = list(dataOf(body)) as { id: number; name: string; status: string }[]
    found.push(...batch)
    if (!body.includes('<is_truncated>true</is_truncated>') || batch.length === 0) break
  }
  const is = (s: string) => found.filter(m => m.status.toLowerCase() === s).length
  return {
    ok: is('ok'),
    warn: is('warn'),
    alert: is('alert'),
    nodata: is('no data'),
    alerting: found.filter(m => ['alert', 'warn'].includes(m.status.toLowerCase())).map(m => ({ id: m.id, name: m.name, status: m.status, url: `${site}/monitors/${m.id}` })),
  }
}

async function seriesOf($: $, server: string, config: Config['monitor'], service: string, now: number) {
  const from = now - DAY_MS
  const args = { queries: [fill(config.queries.errors, service, config.env), fill(config.queries.p95, service, config.env)], from: new Date(from).toISOString(), to: new Date(now).toISOString() }
  const [errors, p95] = list(dataOf(await datadog($, server, 'get_datadog_metric', args))) as { binned?: Binned[] }[]
  return { errors: bins(errors, from, now, true), p95: bins(p95, from, now, false) }
}

async function issuesOf($: $, server: string, config: Config['monitor'], service: string, now: number) {
  const args = {
    query: `service:${service} env:${config.env}`,
    from: 'now-1d',
    states: ['FOR_REVIEW'],
    includes: ['timing', 'total_count', 'issue_url'],
    order_by: 'TOTAL_COUNT',
    max_tokens: 4000,
  }
  return parseIssues(await datadog($, server, 'search_datadog_error_tracking_issues', args), now - DAY_MS).slice(0, 8)
}

// The repo's name from its URL: https://github.com/acme/api(.git) is api.
const repoName = (url: string) => url.replace(/\.git$/, '').split('/').pop() ?? ''

async function pinnedApps($: $): Promise<string[]> {
  try {
    return JSON.parse(await $.fs.read(`${await home($)}/.config/cockpit/monitor-pinned.json`)) as string[]
  } catch {
    return []
  }
}

async function readJson<T>($: $, path: string): Promise<T | null> {
  try {
    return JSON.parse(await $.fs.read(path)) as T
  } catch {
    return null
  }
}

// An app's recent deploys, newest first, as releases: mine when I started it or it ships one of my tickets.
async function releasesOf($: $, app: { id: string; repo: string }, now: number, login: string, inbox: Inbox | null, org: string): Promise<Release[]> {
  const f = (await platform($))!.fields
  const deployments = list(pick(await call($, 'deployHistory', { appId: app.id }), f.deployments))
  return deployments
    .map(d => {
      const tag = text(pick(d, f.deployTag)) ?? ''
      const actor = (text(pick(d, f.deployActor ?? 'actor')) ?? '').replace(/^.*\//, '')
      const shipped = Object.entries(inbox?.deploys ?? {}).filter(([, prs]) => prs.some(p => p.repo === app.repo && p.tag === tag))
      const tickets = shipped.map(([id]) => ({ id, url: inbox?.tickets.find(t => t.id === id)?.url ?? '' }))
      const prs = shipped.flatMap(([, ps]) => ps.filter(p => p.repo === app.repo && p.tag === tag)).map(p => ({ repo: p.repo, number: p.number, url: org ? `https://github.com/${org}/${p.repo}/pull/${p.number}` : '' }))
      return {
        id: Number(text(pick(d, f.deployId ?? 'id')) ?? 0),
        tag,
        actor,
        isMine: (login !== '' && actor === login) || tickets.length > 0,
        phase: text(pick(d, f.deployPhase ?? 'phase')) ?? '',
        startedAt: text(pick(d, f.deployStarted ?? 'startedAt')) ?? '',
        finishedAt: text(pick(d, f.deployFinished ?? 'finishedAt')) ?? null,
        tickets,
        prs,
        verdict: null,
      }
    })
    .filter((r, i) => i === 0 || now - Date.parse(r.startedAt) < DAY_MS)
    .slice(0, 5)
}

// One app's details: its releases with their verdicts (a settled one kept from the last round), monitors, errors and a day
// of numbers. A source that fails leaves its part as it was and says so.
async function watch($: $, app: MonitoredApp, previous: MonitoredApp | undefined, ctx: { config: Config; now: number; login: string; inbox: Inbox | null }, errors: string[]) {
  const { config, now } = ctx
  const server = config.datadog.server
  const site = app.datadogUrl.match(/^https:\/\/[^/]+/)?.[0] ?? 'https://app.datadoghq.com'
  const window = config.deployWatch.afterMinutes * 60_000
  const keep = <T>(label: string, fallback: T) => (err: Error) => (errors.push(`${app.name} ${label}: ${err.message.slice(0, 120)}`), fallback)
  app.releases = await releasesOf($, app, now, ctx.login, ctx.inbox, config.github.org).catch(keep('deploys', previous?.releases ?? []))
  if (!app.service) return
  app.monitors = await monitorsOf($, server, app.service, site).catch(keep('monitors', previous?.monitors ?? null))
  app.series = await seriesOf($, server, config.monitor, app.service, now).catch(keep('series', previous?.series ?? null))
  app.issues = await issuesOf($, server, config.monitor, app.service, now).catch(keep('errors', previous?.issues ?? []))
  for (const release of app.releases) {
    const settled = previous?.releases.find(r => r.id === release.id)?.verdict
    if (settled && settled.state !== 'watching') {
      release.verdict = settled
      continue
    }
    const start = Date.parse(release.startedAt)
    const end = Date.parse(release.finishedAt ?? '') || start
    const until = new Date(end + window).toISOString()
    const before = await health($, server, config.monitor, app.service, start - window, start).catch(() => null)
    const after = now > end ? await health($, server, config.monitor, app.service, end, Math.min(now, end + window)).catch(() => null) : null
    const fresh = app.issues.filter(i => release.tag && i.firstVersion.endsWith(release.tag)).map(i => i.id)
    release.verdict = verdictOf(release.phase, before, after, fresh, until, now, config.monitor.moved)
  }
}

// The verdicts that turned bad since the last round, and my releases that came out fine, each announced once.
async function announce($: $, config: Config, before: MonitorState | null, after: MonitorState) {
  for (const app of after.apps) {
    for (const release of app.releases) {
      const was = before?.apps.find(a => a.id === app.id)?.releases.find(r => r.id === release.id)?.verdict?.state
      const state = release.verdict?.state
      if (!state || !was || state === was) continue
      const isBad = state === 'moved' || state === 'failed'
      const wanted = config.monitor.notify === 'all' || (config.monitor.notify === 'mine' && release.isMine)
      if (!wanted || (!isBad && !(state === 'ok' && release.isMine))) continue
      const title = `${isBad ? '⚠️' : '✅'} ${app.name} ${release.tag}`
      await notify($, title, release.verdict!.summary || state, `monitor:${app.id}`)
    }
  }
}

let inflight: Promise<void> | undefined

export function refreshMonitor($: $, isForced = false) {
  inflight ??= (async () => {
    try {
      const config = await loadConfig($)
      const isPinned = config.spaces.some(s => (typeof s === 'string' ? s : s.name) === 'monitor')
      if (!isPinned || !(await can($, 'deployStatus')) || !(await can($, 'deployHistory'))) return
      const path = `${await cacheDir($)}/monitor.json`
      const kept = await readJson<MonitorState>($, path)
      // The monitor app asks for a refresh by touching this file, as the inbox does.
      const asked = await $.fs.stat(`${await cacheDir($)}/refresh-request`).then(st => st.mtimeMs, () => 0)
      if (!isForced && kept && Date.now() - kept.at < REFRESH_MS && asked <= kept.at) return
      await withLease($, 'monitor', async () => {
        const previous = (await readJson<MonitorState>($, path)) ?? kept
        const now = Date.now()
        const f = (await platform($))!.fields
        const skip = new Set(f.skipKinds ?? [])
        const pinned = new Set(await pinnedApps($))
        const errors: string[] = []
        const status = list(pick(await call($, 'deployStatus'), f.apps)).filter(a => !skip.has(text(pick(a, f.appKind)) ?? ''))
        const apps: MonitoredApp[] = status.map(a => {
          const id = text(pick(a, f.appId)) ?? ''
          const deployedAt = text(pick(a, f.appDeployedAt ?? 'lastDeployedAt')) ?? ''
          return {
            id,
            name: text(pick(a, f.appName ?? 'name')) ?? id,
            service: text(pick(a, f.appService ?? 'service')) ?? '',
            repo: repoName(text(pick(a, f.appRepo)) ?? ''),
            health: text(pick(a, f.appHealth ?? 'health')) ?? '',
            tag: text(pick(a, f.appTag)) ?? text(pick(a, f.appSha)) ?? '',
            deployedAt,
            behind: Number(text(pick(a, f.appBehind ?? 'commitsBehindHead')) ?? 0),
            argoUrl: text(pick(a, f.appLink)) ?? '',
            datadogUrl: text(pick(a, f.appDatadog ?? 'datadogUrl')) ?? '',
            watched: pinned.has(id) || now - Date.parse(deployedAt) < DAY_MS,
            pinned: pinned.has(id),
            monitors: null,
            series: null,
            issues: [],
            releases: [],
          }
        })
        const ctx = { config, now, login: await githubLogin($), inbox: await readJson<Inbox>($, `${await cacheDir($)}/inbox.json`) }
        for (const app of apps.filter(a => a.watched)) await watch($, app, previous?.apps.find(p => p.id === app.id), ctx, errors)
        apps.sort((a, b) => Date.parse(b.deployedAt || '0') - Date.parse(a.deployedAt || '0'))
        const next: MonitorState = { version: 1, at: now, env: config.monitor.env, errors, apps }
        await writeDoc($, 'monitor', next)
        await announce($, config, previous, next)
      })
    } finally {
      inflight = undefined
    }
  })()
  return inflight
}
