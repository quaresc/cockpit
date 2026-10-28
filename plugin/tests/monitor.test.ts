import { expect, test } from 'claude-code/testing'

import type { MonitorState } from '../types'

import { bins, dataOf, parseIssues, refreshMonitor, verdictOf } from '../hooks/monitor'

const moved = { errorPoints: 0.5, p95Ratio: 1.5, p95Ms: 100, minHits: 50 }
const at = Date.parse('2026-10-28T16:00:00Z')

const ISSUES = `- issue_id: 1fcdadd8
  error_type: Elixir.Req.TransportError
  error_message: socket closed
  state: FOR_REVIEW
  first_seen: 2026-10-28T15:55:59.993Z
  first_seen_version: main-v2026.10.28.1
  issue_url: https://app.datadoghq.eu/error-tracking/issue/1fcdadd8
  total_count: 140
- issue_id: a3d948c6
  error_type: Elixir.Ecto.ConstraintError
  error_message: |
    constraint error when attempting to insert struct:

        * "users_email_index" (unique_constraint)
  state: FOR_REVIEW
  first_seen: 2024-04-12T14:40:40.324Z
  first_seen_version: main-68c0aaf
  issue_url: https://app.datadoghq.eu/error-tracking/issue/a3d948c6
  total_count: 28
- issue_id: 70127dda
  error_type: "500"
  error_message: 'exit: {:timeout, 5000}'
  total_count: 1
`

test('error tracking issues read from their YAML list, a block message to its first line', async () => {
  const issues = parseIssues(ISSUES, at - 86_400_000)
  expect(issues.map(i => [i.id, i.type, i.message, i.count, i.isNew])).toEqual([
    ['1fcdadd8', 'Req.TransportError', 'socket closed', 140, true],
    ['a3d948c6', 'Ecto.ConstraintError', 'constraint error when attempting to insert struct:', 28, false],
    ['70127dda', '500', 'exit: {:timeout, 5000}', 1, false],
  ])
  expect(issues[0]!.firstVersion).toBe('main-v2026.10.28.1')
})

test('a Datadog answer is read from its JSON block or as is', async () => {
  expect(dataOf('<METADATA>x</METADATA>\n<JSON_DATA>\n[{"id": 1}]\n</JSON_DATA>')).toEqual([{ id: 1 }])
  expect(dataOf('[2]')).toEqual([2])
  expect(dataOf('<METADATA>\n  <message>No data returned.</message>\n</METADATA>\n<JSON_DATA>\n\n</JSON_DATA>')).toEqual([])
})

test('a timeseries lands in bins: a count summed, a gauge averaged', async () => {
  const from = at - 20 * 60_000
  const series = { binned: [{ start_time: new Date(from).toISOString(), count: 3, avg: 2 }, { start_time: new Date(at - 60_000).toISOString(), count: 1, avg: 5 }] }
  expect(bins(series, from, at, true)).toEqual([6, ...new Array(18).fill(0), 5])
  expect(bins(series, from, at, false)[0]).toBe(2)
  expect(bins(undefined, from, at, true)).toEqual(new Array(20).fill(0))
})

test('a release is watched, then ok or moved by its error rate, p95 or new errors', async () => {
  const before = { errors: 2, hits: 1512, p95: 0.033 }
  const until = new Date(at).toISOString()
  expect(verdictOf('Succeeded', before, { errors: 2, hits: 1500, p95: 0.035 }, [], until, at - 1, moved).state).toBe('watching')
  const ok = verdictOf('Succeeded', before, { errors: 2, hits: 1500, p95: 0.035 }, [], until, at, moved)
  expect([ok.state, ok.summary]).toEqual(['ok', 'errors 0.13% → 0.13% · p95 33 → 35 ms'])
  expect(verdictOf('Succeeded', before, { errors: 31, hits: 1630, p95: 0.035 }, [], until, at, moved).state).toBe('moved')
  expect(verdictOf('Succeeded', before, { errors: 2, hits: 1500, p95: 0.161 }, [], until, at, moved).state).toBe('moved')
  expect(verdictOf('Succeeded', { ...before, p95: 0.013 }, { errors: 2, hits: 1500, p95: 0.046 }, [], until, at, moved).state).toBe('ok')
  expect(verdictOf('Succeeded', before, { errors: 2, hits: 1500, p95: 0.035 }, ['1fcdadd8'], until, at, moved).summary).toContain('1 new error')
  expect(verdictOf('Succeeded', before, { errors: 9, hits: 10, p95: 0.5 }, [], until, at, moved).state).toBe('ok')
  expect(verdictOf('Failed', null, null, [], until, at, moved).state).toBe('failed')
  expect(verdictOf('Succeeded', null, null, [], until, at, moved).state).toBe('unknown')
})

// ---------------------------------------------------------------------------
// One refresh, the platform connector and Datadog answering as they do

const HOME = '/home/me'
const CACHE = `${HOME}/.cache/cockpit`
const NOW = Date.now()
const iso = (ago: number) => new Date(NOW - ago).toISOString()
const MIN = 60_000

const CONFIG = {
  github: { org: 'acme' },
  spaces: ['inbox', 'monitor'],
  datadog: { server: 'Datadog' },
  deployWatch: { afterMinutes: 30 },
  platform: {
    enabled: true,
    server: 'backstage',
    tools: { deployStatus: { tool: 'ship_get-deploy-status', args: { env: 'production' } }, deployHistory: { tool: 'ship_list-deployments', args: { env: 'production', appId: '{appId}' } } },
    fields: { apps: 'apps', appId: 'id', appRepo: 'githubRepoUrl', appLink: 'argoUrl', appTag: 'imageTag', appKind: 'type', skipKinds: ['cdn'], deployments: 'deployments', deployTag: 'tag' },
  },
}

const status = {
  apps: [
    { id: 'api-production', name: 'api', type: 'kubernetes', argoUrl: 'https://argo/api', health: 'Healthy', imageTag: 'v2', lastDeployedAt: iso(90 * MIN), commitsBehindHead: 1, githubRepoUrl: 'https://github.com/acme/api', service: 'api', datadogUrl: 'https://app.datadoghq.eu/apm/entity/service%3Aapi' },
    { id: 'billing-production', name: 'billing', type: 'kubernetes', argoUrl: 'https://argo/billing', health: 'Healthy', imageTag: 'v9', lastDeployedAt: iso(9 * 86_400_000), commitsBehindHead: 7, githubRepoUrl: 'https://github.com/acme/billing', service: 'billing', datadogUrl: '' },
    { id: 'assets-cdn', name: 'assets', type: 'cdn', lastDeployedAt: iso(MIN) },
  ],
}
const deployments = {
  deployments: [
    { id: 622, tag: 'v2', actor: 'user:default/me', phase: 'Succeeded', startedAt: iso(95 * MIN), finishedAt: iso(90 * MIN) },
    { id: 619, tag: 'v1', actor: 'user:default/jonas', phase: 'Succeeded', startedAt: iso(30 * 3600_000), finishedAt: iso(30 * 3600_000) },
  ],
}
const wrap = (data: unknown) => `<METADATA>\n  <is_truncated>false</is_truncated>\n</METADATA>\n<JSON_DATA>\n${JSON.stringify(data)}\n</JSON_DATA>`

function engine(stored: unknown) {
  const files = new Map<string, string>([[`${HOME}/.config/cockpit/config.json`, JSON.stringify(CONFIG)], [`${CACHE}/inbox.json`, JSON.stringify({ deploys: { 'ENG-412': [{ repo: 'api', number: 2219, tag: 'v2', isLive: true }] }, tickets: [{ id: 'ENG-412', url: 'https://linear.app/acme/issue/ENG-412' }] })]])
  if (stored) files.set(`${CACHE}/monitor.json`, JSON.stringify(stored))
  const calls: string[] = []
  const notified: string[] = []
  let doc: unknown = null
  const ran = (stdout = '') => ({ exitCode: 0, stdout, stderr: '' })
  const $ = {
    env: { get: async (name: string) => (name === 'HOME' ? HOME : undefined) },
    session: { id: async () => 's1' },
    clock: { every: () => ({ cancel: () => undefined }) },
    fs: {
      read: async (path: string) => {
        const content = files.get(path)
        if (content === undefined) throw new Error('ENOENT')
        return content
      },
      write: async (path: string, body: string) => void files.set(path, body),
      stat: async (path: string) => {
        if (!files.has(path) && !path.endsWith('cockpit.sh')) throw new Error('ENOENT')
        return { mtimeMs: 1 }
      },
    },
    process: {
      run: async (argv: string[]) => {
        if (argv[0] === 'gh') return ran('me\n')
        if (argv[0] === 'sh' && argv[2] === 'store') {
          const [verb, , file] = argv.slice(3)
          if (verb === 'lease') return ran('granted')
          if (verb === 'release') return ran('released')
          doc = JSON.parse(files.get(file!)!)
          files.set(`${CACHE}/monitor.json`, JSON.stringify(doc))
          return ran(JSON.stringify(doc))
        }
        if (argv[0] === 'sh' && argv[2] === 'notify') notified.push(`${argv[3]} | ${argv[4]}`)
        return ran()
      },
    },
    mcp: {
      call: async (server: string, tool: string, args: Record<string, unknown>) => {
        calls.push(`${server} ${tool} ${String(args.appId ?? args.query ?? '')}`.trim())
        const text = (t: string) => ({ isError: false, content: [{ type: 'text', text: t }] })
        if (tool === 'ship_get-deploy-status') return text(JSON.stringify(status))
        if (tool === 'ship_list-deployments') return text(JSON.stringify(deployments))
        if (tool === 'search_datadog_monitors') return text(wrap([{ id: 1, name: '[api] Availability', status: 'Alert' }, { id: 2, name: '[api] Latency', status: 'OK' }, { id: 3, name: '[api] Export', status: 'No Data' }]))
        if (tool === 'search_datadog_error_tracking_issues') return text(`- issue_id: e1\n  error_type: Elixir.Req.TransportError\n  error_message: socket closed\n  first_seen: ${iso(80 * MIN)}\n  first_seen_version: main-v2\n  issue_url: https://dd/e1\n  total_count: 140\n`)
        if (tool === 'get_datadog_metric' && args.response_format === 'scalar') {
          const queries = args.queries as { query: string }[]
          const isAfter = Date.parse(String(args.from)) >= NOW - 95 * MIN
          return text(wrap([{ values: { [queries[0]!.query]: isAfter ? 31 : 2, [queries[1]!.query]: isAfter ? 1630 : 1512, [queries[2]!.query]: isAfter ? 0.161 : 0.033 } }]))
        }
        if (tool === 'get_datadog_metric') return text(wrap([{ binned: [{ start_time: iso(MIN), count: 2, avg: 7 }] }, { binned: [{ start_time: iso(MIN), count: 3, avg: 0.2 }] }]))
        throw new Error(`unexpected ${tool}`)
      },
    },
  } as unknown as Parameters<typeof refreshMonitor>[0]
  return { $, calls, notified, doc: () => doc as MonitorState }
}

test('a refresh watches the apps released today, judges their releases and notifies a mine that moved once', async () => {
  const first = engine(null)
  await refreshMonitor(first.$, true)
  const doc = first.doc()
  expect(doc.apps.map(a => [a.id, a.watched])).toEqual([['api-production', true], ['billing-production', false]])
  const api = doc.apps[0]!
  expect([api.repo, api.health, api.tag, api.behind, api.service]).toEqual(['api', 'Healthy', 'v2', 1, 'api'])
  expect([api.monitors?.ok, api.monitors?.alert, api.monitors?.nodata, api.monitors?.alerting[0]?.url]).toEqual([1, 1, 1, 'https://app.datadoghq.eu/monitors/1'])
  expect(api.series?.errors.at(-1)).toBe(14)
  expect(api.issues.map(i => [i.id, i.isNew])).toEqual([['e1', true]])
  const [mine, old] = api.releases
  expect(old).toBeUndefined()
  expect([mine?.tag, mine?.actor, mine?.isMine, mine?.tickets[0]?.id, mine?.prs[0]?.url]).toEqual(['v2', 'me', true, 'ENG-412', 'https://github.com/acme/api/pull/2219'])
  expect([mine?.verdict?.state, mine?.verdict?.newIssues]).toEqual(['moved', ['e1']])
  expect(mine?.verdict?.summary).toBe('errors 0.13% → 1.9% · p95 33 → 161 ms · 1 new error')
  expect(first.calls.filter(c => c.includes('billing'))).toEqual([])
  expect(first.notified).toEqual([])

  // The release seen while still watching turns moved: announced once.
  const watching = structuredClone(doc)
  watching.apps[0]!.releases[0]!.verdict!.state = 'watching'
  const second = engine({ ...watching, at: 0 })
  await refreshMonitor(second.$, true)
  expect(second.notified).toEqual(['⚠️ api v2 | errors 0.13% → 1.9% · p95 33 → 161 ms · 1 new error'])
  const third = engine({ ...second.doc(), at: 0 })
  await refreshMonitor(third.$, true)
  expect(third.notified).toEqual([])
  expect(third.calls.filter(c => c.includes('get_datadog_metric')).length).toBe(1)
})
