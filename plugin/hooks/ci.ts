import type { EngineInterface } from 'claude-code'

import type { CiFailure, CiRun, CiState, DeployedPr, Inbox, MergedPr, Preview } from '../types'

import { cacheDir, fill, home, loadConfig, writeAtomic } from './config'
import { appFor, call, callMapped, can, createPreview, dashboardName, deployedApps, extendPreview, listAt, listPreviews, pick, previewUrl, text } from './platform'
import { hubOwnsCi } from './feed'
import { ciChanges, hasTicket } from './match'
import { checkRequest } from './protocol'
import type { Outcome } from './protocol'
import { completeTicket } from './tickets'

type $ = EngineInterface

const CI_STALE_MS = 55_000
const CI_VERSION = 3
const LOCK_STALE_MS = 2 * 60_000
const LOCK_BEAT_MS = 30_000
const REQUEST_TTL_MS = 10 * 60_000
const RESULTS_TTL_MS = 86_400_000

// ---------------------------------------------------------------------------
// Shared plumbing

// A system notification; a click opens `action`, a URL or a focus.sh target such as space:inbox or runbook:<slug>,
// and each button, [label, action], runs its own.
export async function notify($: $, title: string, body: string, action = '', buttons: [string, string][] = []) {
  if (!(await loadConfig($)).notifications) return
  const script = `${await home($)}/.config/cockpit/herdr/notify.sh`
  const isLinked = await $.fs.stat(script).then(() => true, () => false)
  const command = isLinked
    ? ['sh', script, title, body, action, ...buttons.map(([label, target]) => `${label}=${target}`)]
    : ['herdr', 'notification', 'show', title, '--body', body, '--sound', 'request']
  await $.process.run(command, { timeoutMs: 30_000 })
}

// Where a click on a ticket's notification goes: its runbook when there is one, else the ticket.
export async function ticketAction($: $, id: string, url: string | undefined, steps: string[] = []) {
  const slug = id.toLowerCase()
  const hasRunbook = await $.fs.stat(`${await home($)}/.config/cockpit/runbooks/${slug}.md`).then(() => true, () => false)
  return hasRunbook ? `runbook:${slug}${steps.length > 0 ? `:${steps.join(',')}` : ''}` : (url ?? 'space:inbox')
}

// A refused background call, said in one line that names the permission rule to add.
export function shortError(err: Error) {
  const tool = err.message.match(/no verdict for (mcp__\S+?):/)?.[1]
  return tool ? `blocked by auto mode: allow ${tool}` : err.message.slice(0, 200)
}

// A cross-session lock, so one Claude session fetches and notifies for all of them. The holder touches it while it works,
// so only a lock left by a dead session goes stale; renaming it away first means a single session takes it over.
export async function withLock<T>($: $, name: string, fn: () => Promise<T>): Promise<T | undefined> {
  const lock = `${await cacheDir($)}/${name}.lock`
  try {
    const age = Date.now() - (await $.fs.stat(lock)).mtimeMs
    const stale = `${lock}.stale-${Math.random().toString(36).slice(2)}`
    if (age > LOCK_STALE_MS && (await $.process.run(['mv', lock, stale])).exitCode === 0) await $.process.run(['rmdir', stale])
  } catch {
    // No lock yet.
  }
  if ((await $.process.run(['mkdir', '-p', await cacheDir($)])).exitCode !== 0) return undefined
  if ((await $.process.run(['mkdir', lock])).exitCode !== 0) return undefined
  const beat = $.clock.every(LOCK_BEAT_MS, () => void $.process.run(['touch', lock]).catch(() => undefined))
  try {
    return await fn()
  } finally {
    beat.cancel()
    await $.process.run(['rmdir', lock])
  }
}

async function readJson<T>($: $, path: string): Promise<T | null> {
  try {
    return JSON.parse(await $.fs.read(path)) as T
  } catch {
    return null
  }
}

export async function readCi($: $) {
  return readJson<CiState>($, `${await cacheDir($)}/ci.json`)
}

// ---------------------------------------------------------------------------
// CI: the latest run of each branch I pushed, with progress, failure and image job; GitHub's checks unless a CI
// connector is mapped (`ci.server`), whose tools and answers the config maps like the platform's.

export type CiCapability = 'listRuns' | 'listWorkflows' | 'listJobs' | 'getJob' | 'listTests' | 'logs' | 'rerun'

type Ci = Awaited<ReturnType<typeof loadConfig>>['ci']

const FINAL = ['succeeded', 'failed', 'canceled']

// The CI's glyph: CircleCI's, GitHub Actions', GitLab's, else a generic run.
const CI_MARKS: Record<string, string> = { circleci: '\ue78c', github: '\ueaff', gitlab: '\u{f0ba0}' }
export const ciMark = (name: string) => CI_MARKS[name.toLowerCase()] ?? '\ueb9e'

// Whether a job, check or image build named after the configured image job.
const isImageJob = (name: string, imageJob: string) => Boolean(imageJob) && (name === imageJob || name.endsWith(`: ${imageJob}`) || name.endsWith(`/${imageJob}`))

// --- GitHub checks, read with gh: one run per open PR of mine, its head commit's checks.

const CHECKS_QUERY = (org: string) => {
  const scope = org ? ` org:${org.replace(/[^A-Za-z0-9_.-]/g, '')}` : ''
  return `query {
  mine: search(query: "is:pr is:open archived:false author:@me${scope}", type: ISSUE, first: 30) { nodes { ... on PullRequest {
    headRefName repository { name nameWithOwner } commits(last: 1) { nodes { commit { oid committedDate statusCheckRollup { state
      contexts(first: 30) { nodes { ... on CheckRun { databaseId name status conclusion detailsUrl checkSuite { workflowRun { databaseId } } }
        ... on StatusContext { context state targetUrl } } } } } } } } } }
}`
}

type CheckContext = {
  databaseId?: number
  name?: string
  status?: string
  conclusion?: string | null
  checkSuite?: { workflowRun?: { databaseId?: number } | null }
  context?: string
  state?: string
  targetUrl?: string | null
  detailsUrl?: string | null
}
type ChecksPr = {
  headRefName?: string
  repository?: { name: string; nameWithOwner: string }
  commits?: { nodes: { commit: { oid: string; committedDate: string; statusCheckRollup: { state: string; contexts: { nodes: CheckContext[] } } | null } }[] }
}

// A cancelled check counts as failed, as in the inbox's PR rollup.
const CHECK_FAILED = ['FAILURE', 'ERROR', 'TIMED_OUT', 'STARTUP_FAILURE', 'ACTION_REQUIRED', 'CANCELLED']
const checkName = (c: CheckContext) => c.name ?? c.context ?? '?'
const isActionsJob = (c: CheckContext) => Boolean(c.databaseId && c.checkSuite?.workflowRun?.databaseId)
const isCheckDone = (c: CheckContext) => (c.context !== undefined ? !['PENDING', 'EXPECTED'].includes(c.state ?? '') : c.status === 'COMPLETED')
const checkOutcome = (c: CheckContext) => {
  const value = c.conclusion ?? c.state ?? ''
  return value === 'SUCCESS' || value === 'NEUTRAL' || value === 'SKIPPED' ? 'succeeded' : CHECK_FAILED.includes(value) ? 'failed' : value.toLowerCase()
}

// The failing step of a GitHub Actions job; a check from another app has none to read.
async function failedStep($: $, slug: string, job: number | undefined) {
  if (!job) return undefined
  const { exitCode, stdout } = await $.process.run(['gh', 'api', `repos/${slug}/actions/jobs/${job}`, '--jq', '[.steps[] | select(.conclusion == "failure") | .name][0] // empty'])
  return exitCode === 0 ? stdout.trim() || undefined : undefined
}

async function githubRuns($: $, prev: CiState | null): Promise<CiRun[]> {
  const { github, ci } = await loadConfig($)
  const { exitCode, stdout, stderr } = await $.process.run(['gh', 'api', 'graphql', '-f', `query=${CHECKS_QUERY(github.org)}`], { timeoutMs: 30_000 })
  if (exitCode !== 0) throw new Error(`gh: ${stderr.trim().split('\n')[0]}`)
  const prs = ((JSON.parse(stdout).data as { mine: { nodes: ChecksPr[] } }).mine.nodes ?? []).filter(pr => pr.headRefName && pr.repository)
  const cached = new Map((prev?.runs ?? []).map(r => [`${r.repo}/${r.branch}/${r.runId}`, r]))
  const runs: CiRun[] = []
  for (const pr of prs) {
    const commit = pr.commits?.nodes[0]?.commit
    const checks = commit?.statusCheckRollup?.contexts.nodes ?? []
    if (!commit || checks.length === 0) continue
    const { name: repo, nameWithOwner: slug } = pr.repository!
    const branch = pr.headRefName!
    const isRunning = checks.some(c => !isCheckDone(c))
    const failures = checks.filter(c => isCheckDone(c) && checkOutcome(c) === 'failed')
    // An Actions job first: its logs and its rerun are within reach of gh.
    const failed = failures.find(isActionsJob) ?? failures[0]
    const workflows = [...new Set(failures.filter(isActionsJob).map(c => String(c.checkSuite!.workflowRun!.databaseId)))]
    const image = checks.find(c => isImageJob(checkName(c), ci.imageJob))
    const job = failed && isActionsJob(failed) ? failed.databaseId : undefined
    // A rerun keeps the commit, so only the failing step of the same job is reused from the cache.
    const known = cached.get(`${repo}/${branch}/${commit.oid}`)?.failed
    const step = !failed || isRunning ? undefined : known?.jobId === String(job ?? '') && known.job === checkName(failed) ? known.step : await failedStep($, slug, job)
    runs.push({
      repo,
      slug,
      branch,
      runId: commit.oid,
      createdAt: commit.committedDate,
      isRunning,
      outcome: isRunning ? 'running' : failed ? 'failed' : 'succeeded',
      workflowId: workflows.join(',') || undefined,
      done: checks.filter(isCheckDone).length,
      total: checks.length,
      failed: failed
        ? {
            job: checkName(failed),
            jobId: String(job ?? ''),
            step,
            tests: [],
            logs: job ? `gh run view --job ${job} --log-failed -R ${slug}` : (failed.detailsUrl ?? failed.targetUrl) ? `its page, ${failed.detailsUrl ?? failed.targetUrl}` : 'gh pr checks',
          }
        : undefined,
      dockerTag: image ? (isCheckDone(image) ? checkOutcome(image) : 'running') : undefined,
    })
  }
  return runs
}

// --- A CI connector, through the tools and fields the config maps.

const outcomeOf = (ci: Ci, value: unknown) => {
  const raw = text(value) ?? ''
  return ci.outcomes[raw] ?? raw
}

// Ended by its phase when the connector reports one, else by a final outcome.
const isEnded = (ci: Ci, node: unknown, phasePath: string | undefined, outcomePath: string | undefined) =>
  phasePath ? ci.endedPhases.includes(text(pick(node, phasePath)) ?? '') : FINAL.includes(outcomeOf(ci, pick(node, outcomePath)))

// How a session reads the failing job's logs, from the tools the connector has.
const logsHint = (ci: Ci, job: string) => {
  const tools = (['getJob', 'listTests', 'logs'] as const).map(c => ci.tools[c]?.tool).filter(Boolean)
  return `the ${ci.name} MCP${tools.length > 0 ? ` (${tools.join(', ')}; keep log tails small)` : ''}, job id ${job}`
}

async function failureOf($: $, ci: Ci, job: unknown): Promise<CiFailure> {
  const f = ci.fields
  const id = text(pick(job, f.jobId)) ?? ''
  const detail = ci.tools.getJob?.tool ? await callMapped($, ci, 'getJob', { job: id }).catch(() => null) : null
  const step = listAt(detail, f.steps).find(s => outcomeOf(ci, pick(s, f.stepOutcome)) === 'failed' || ![undefined, 0].includes(pick(s, f.stepExit) as number | undefined))
  const tests = ci.tools.listTests?.tool
    ? await callMapped($, ci, 'listTests', { job: id })
        .then(t => listAt(t, f.tests).slice(0, 3).map(x => text(pick(x, f.testName)) ?? '?'))
        .catch(() => [] as string[])
    : []
  return { job: text(pick(job, f.jobName)) ?? '?', jobId: id, step: text(pick(step, f.stepName)), tests, logs: logsHint(ci, id) }
}

async function detailsOf($: $, ci: Ci, run: unknown, base: Omit<CiRun, 'done' | 'total'>): Promise<CiRun> {
  const f = ci.fields
  const jobs: unknown[] = []
  let workflowIds: { id: string; isFailed: boolean }[] = []
  if (ci.tools.listWorkflows?.tool) {
    // A rerun adds a workflow of the same name to the run: only the newest of each name counts.
    const newest = new Map<string, unknown>()
    for (const workflow of listAt(await callMapped($, ci, 'listWorkflows', { run: base.runId }), f.workflows)) {
      const name = text(pick(workflow, f.workflowName)) ?? ''
      const seen = newest.get(name)
      if (!seen || (text(pick(seen, f.workflowCreated)) ?? '') < (text(pick(workflow, f.workflowCreated)) ?? '')) newest.set(name, workflow)
    }
    workflowIds = [...newest.values()].map(w => ({ id: text(pick(w, f.workflowId)) ?? '', isFailed: outcomeOf(ci, pick(w, f.workflowOutcome)) === 'failed' }))
    for (const { id } of workflowIds) jobs.push(...listAt(await callMapped($, ci, 'listJobs', { workflow: id, run: base.runId }), f.jobs))
  } else {
    jobs.push(...listAt(await callMapped($, ci, 'listJobs', { run: base.runId, workflow: base.runId }), f.jobs))
  }
  const failedJob = jobs.find(j => outcomeOf(ci, pick(j, f.jobOutcome)) === 'failed')
  const image = jobs.find(j => isImageJob(text(pick(j, f.jobName)) ?? '', ci.imageJob))
  return {
    ...base,
    workflowId: workflowIds.find(w => w.isFailed)?.id ?? workflowIds[0]?.id ?? base.runId,
    done: jobs.filter(j => isEnded(ci, j, f.jobPhase, f.jobOutcome)).length,
    total: jobs.length,
    failed: failedJob ? await failureOf($, ci, failedJob) : undefined,
    dockerTag: image ? (isEnded(ci, image, f.jobPhase, f.jobOutcome) ? outcomeOf(ci, pick(image, f.jobOutcome)) : 'running') : undefined,
  }
}

async function connectorRuns($: $, ci: Ci, prev: CiState | null): Promise<CiRun[]> {
  const f = ci.fields
  const latest = new Map<string, unknown>()
  for (const run of listAt(await callMapped($, ci, 'listRuns', {}), f.runs)) {
    const repo = (text(pick(run, f.runRepo)) ?? '').replace(/\.git$/, '').split('/').pop()
    const branch = text(pick(run, f.runBranch))
    if (!repo || !branch || text(pick(run, f.runTag))) continue
    const key = `${repo}/${branch}`
    const seen = latest.get(key)
    if (!seen || (text(pick(seen, f.runCreated)) ?? '') < (text(pick(run, f.runCreated)) ?? '')) latest.set(key, run)
  }
  const cached = new Map((prev?.runs ?? []).map(r => [r.runId, r]))
  const runs: CiRun[] = []
  for (const [key, run] of latest) {
    const [repo, ...rest] = key.split('/')
    const outcome = outcomeOf(ci, pick(run, f.runOutcome))
    const base = {
      repo: repo ?? '?',
      branch: rest.join('/'),
      runId: text(pick(run, f.runId)) ?? '',
      createdAt: text(pick(run, f.runCreated)) ?? '',
      isRunning: !isEnded(ci, run, f.runPhase, f.runOutcome),
      outcome: outcome || 'running',
    }
    const known = cached.get(base.runId)
    // An ended run never changes, so its details are fetched once.
    runs.push(known && !known.isRunning && !base.isRunning ? known : await detailsOf($, ci, run, base).catch(() => ({ ...base, done: 0, total: 0 })))
  }
  return runs
}

// Reruns the failed jobs of a run: each failed workflow through gh for GitHub Actions, else the connector's rerun tool.
async function rerun($: $, workflowId: string) {
  const { ci } = await loadConfig($)
  if (ci.server) return callMapped($, ci, 'rerun', { workflow: workflowId, run: workflowId })
  const run = (await readCi($))?.runs.find(r => r.workflowId === workflowId)
  if (!run?.slug) throw new Error(`no GitHub run ${workflowId} in the CI cache`)
  for (const id of workflowId.split(',')) {
    const { exitCode, stderr } = await $.process.run(['gh', 'run', 'rerun', id, '--failed', '-R', run.slug], { timeoutMs: 30_000 })
    if (exitCode !== 0) throw new Error(`gh run rerun: ${stderr.trim().split('\n')[0]}`)
  }
}

let ciInflight: Promise<void> | undefined
let isConnectorMissing = false

export function refreshCi($: $, isForced = false) {
  ciInflight ??= (async () => {
    try {
      const { ci, github } = await loadConfig($)
      if (!ci.enabled || (ci.server ? isConnectorMissing : !github.enabled)) return
      // A cache from an older version of this code is rebuilt from scratch, without notifications.
      const stored = await readCi($)
      // The hub keeps ci.json while its feed is fresh: GitHub's checks, or CircleCI given a token. It announces the runs too.
      if (hubOwnsCi(stored)) return
      const prev = stored?.version === CI_VERSION ? stored : null
      if (!isForced && prev && Date.now() - prev.at < CI_STALE_MS) return
      await withLock($, 'ci', async () => {
        const runs = await (ci.server ? connectorRuns($, ci, prev) : githubRuns($, prev)).catch(async (err: Error) => {
          // A session started before the CI connector was added has no such server: leave it to the others.
          if (err.message.includes('no connected MCP tool')) {
            isConnectorMissing = true
            return null
          }
          const failed: CiState = { at: prev?.at ?? 0, runs: prev?.runs ?? [], error: `${ci.name} ${shortError(err)}` }
          await writeAtomic($, `${await cacheDir($)}/ci.json`, JSON.stringify(failed))
          return null
        })
        if (!runs) return
        const state: CiState = { at: Date.now(), runs, version: CI_VERSION }
        await writeAtomic($, `${await cacheDir($)}/ci.json`, JSON.stringify(state))
        const inbox = await readJson<Inbox>($, `${await cacheDir($)}/inbox.json`)
        if (prev) for (const change of ciChanges(prev, state, inbox)) await notify($, `🔁 ${ci.name}`, change.text, change.action, change.buttons)
      })
    } finally {
      ciInflight = undefined
    }
  })()
  return ciInflight
}

// ---------------------------------------------------------------------------
// Deploys: is each merged PR of a "To Deploy" ticket in the production image yet?

const compareCache = new Map<string, boolean>()

async function isContained($: $, slug: string, sha: string, ref: string) {
  const key = `${slug}:${sha}:${ref}`
  const hit = compareCache.get(key)
  if (hit !== undefined) return hit
  const { exitCode, stdout } = await $.process.run(['gh', 'api', `repos/${slug}/compare/${sha}...${ref}`, '--jq', '.status'])
  const isLive = exitCode === 0 && ['identical', 'ahead'].includes(stdout.trim())
  if (exitCode === 0) compareCache.set(key, isLive)
  return isLive
}

export async function deploysFor($: $, tickets: Inbox['tickets'], merged: MergedPr[]) {
  const waiting = tickets.filter(t => t.statusType === 'started' && /deploy/i.test(t.status))
  if (waiting.length === 0 || !(await can($, 'deployStatus'))) return {}
  const apps = await deployedApps($)
  const out: Record<string, DeployedPr[]> = {}
  for (const ticket of waiting) {
    const prs = merged.filter(pr => hasTicket(pr.branch, ticket.id))
    out[ticket.id] = await Promise.all(
      prs.map(async pr => {
        const app = appFor(apps, pr.repo)
        const ref = app?.tag ?? app?.sha ?? null
        return { repo: pr.repo, number: pr.number, tag: ref, isLive: ref ? await isContained($, pr.slug, pr.mergeSha, ref) : false }
      }),
    )
  }
  return out
}

// Some time after a ticket goes live, a headless Claude compares errors and latency before and after, adds the verdict
// to the runbook and notifies once (herdr/watch-deploy.sh); one watch per ticket.
export async function startDeployWatch($: $, ticket: string, prs: DeployedPr[]) {
  if (!(await loadConfig($)).deployWatch.enabled || prs.length === 0) return
  const script = `${await home($)}/.config/cockpit/herdr/watch-deploy.sh`
  if (!(await $.fs.stat(script).then(() => true, () => false))) return
  const repos = [...new Set(prs.map(p => p.repo))].join(',')
  const tag = prs.map(p => p.tag).find(t => t) ?? ''
  const log = `${await cacheDir($)}/watch-${ticket.toLowerCase()}.log`
  await $.process.run(['sh', '-c', 'nohup sh "$0" "$1" "$2" "$3" >"$4" 2>&1 &', script, ticket, repos, tag, log])
}

// ---------------------------------------------------------------------------
// Previews I own, and which branch each of their apps runs

let login: string | undefined

export async function previewsFor($: $): Promise<Preview[]> {
  if (!(await can($, 'listPreviews'))) return []
  if (!login) {
    const { exitCode, stdout } = await $.process.run(['gh', 'api', 'user', '--jq', '.login'])
    if (exitCode !== 0) return []
    login = stdout.trim()
  }
  return (await listPreviews($, { login })).flatMap(env =>
    env.apps
      .filter(p => p.enabled && p.branch && !['main', 'master'].includes(p.branch))
      .map(a => ({ id: env.id, app: a.name, branch: a.branch ?? '', health: env.health, expiresAt: env.expiresAt, activated: env.activated })),
  )
}

// ---------------------------------------------------------------------------
// Requests the herdr inbox app drops in ~/.cache/cockpit/requests: CI reruns, preview redeploys, ticket releases

// A ticket moved to a completed state leaves the inbox at once, without waiting for the next fetch.
async function dropTicket($: $, id: string) {
  const path = `${await cacheDir($)}/inbox.json`
  const inbox = await readJson<Inbox>($, path)
  if (!inbox) return
  await writeAtomic($, path, JSON.stringify({ ...inbox, tickets: inbox.tickets.filter(t => t.id !== id) }))
}

// Previews created from the inbox, one file each in pending-previews/, announced once the platform reports them healthy.
type Pending = { preview: string; app: string; branch: string; at: number }

export async function announcePreviews($: $, previews: Preview[]) {
  const dir = `${await cacheDir($)}/pending-previews`
  for (const entry of await $.fs.list(dir).catch(() => [])) {
    if (!entry.name.endsWith('.json')) continue
    const path = `${dir}/${entry.name}`
    const p = await readJson<Pending>($, path)
    if (!p) continue
    const box = previews.find(e => e.id === p.preview)
    if (box && box.health === ((await loadConfig($)).platform.fields.healthy ?? 'Healthy')) {
      const url = await previewUrl($, p.preview, p.app).catch(() => undefined)
      const dashboard = fill((await loadConfig($)).platform.previewDashboardUrl, { preview: p.preview })
      await notify($, '🧪 Preview ready', `${p.app} ${p.branch} on ${p.preview}`, url ?? (await previewAction($, p.preview)), dashboard ? [[dashboardName(dashboard), dashboard]] : [])
      await $.process.run(['rm', '-f', path])
    } else if (Date.now() - p.at > 3 * 3600_000) {
      await $.process.run(['rm', '-f', path])
    }
  }
}

async function startPreviewFor($: $, app: string, branch: string) {
  const preview = await createPreview($, app, branch)
  if (preview) {
    const dir = `${await cacheDir($)}/pending-previews`
    await $.process.run(['mkdir', '-p', dir])
    await writeAtomic($, `${dir}/${preview.replace(/[^A-Za-z0-9._-]/g, '_')}.json`, JSON.stringify({ preview, app, branch, at: Date.now() }))
  }
  return preview
}

async function previewAction($: $, preview: string) {
  return fill((await loadConfig($)).platform.previewDashboardUrl, { preview }) ?? 'space:inbox'
}

let prunedAt = 0

// Outcomes nobody came back for (a notification's button has no row waiting) go after a day.
async function pruneResults($: $, dir: string) {
  if (Date.now() - prunedAt < 3600_000) return
  prunedAt = Date.now()
  for (const entry of await $.fs.list(dir).catch(() => [])) {
    if (entry.kind === 'file' && Date.now() - entry.mtimeMs > RESULTS_TTL_MS) await $.process.run(['rm', '-f', `${dir}/${entry.name}`])
  }
}

export async function processRequests($: $, onDone: () => void) {
  const dir = `${await cacheDir($)}/requests`
  const entries = await $.fs.list(dir).catch(() => [])
  void pruneResults($, `${dir}/results`).catch(() => undefined)
  for (const entry of entries) {
    if (!entry.name.endsWith('.json') || entry.name.startsWith('claimed-')) continue
    const claimed = `${dir}/claimed-${entry.name}`
    // mv is atomic: whichever session moves the file handles it.
    if ((await $.process.run(['mv', `${dir}/${entry.name}`, claimed])).exitCode !== 0) continue
    const raw = await readJson<unknown>($, claimed)
    await $.process.run(['rm', '-f', claimed])
    if (raw === null) continue
    // The app waits for this file to show the outcome on the row it came from; the .claimed marker says a session took it.
    const results = `${dir}/results/${entry.name}`
    const result = async (state: Outcome['state'], message: string) => {
      const outcome: Outcome = { ok: state === 'done', state, message, at: Date.now() }
      await $.fs.write(results, JSON.stringify(outcome))
      await $.process.run(['rm', '-f', `${results}.claimed`])
    }
    // A request no session took in time was given up on by the app: running it now would surprise.
    if (Date.now() - entry.mtimeMs > REQUEST_TTL_MS) {
      await result('refused', 'expired before a Claude session took it')
      continue
    }
    const checked = checkRequest(raw)
    if ('refusal' in checked) {
      await result('refused', checked.refusal)
      continue
    }
    const { request } = checked
    await $.fs.write(`${results}.claimed`, String(Date.now()))
    try {
      if (request.type === 'rerun') {
        await rerun($, request.workflowId)
        await result('done', 'rerun started')
        await notify($, `🔁 ${(await loadConfig($)).ci.name}`, `Rerunning failed jobs: ${request.label}`, 'space:inbox')
      } else if (request.type === 'redeploy') {
        await call($, 'redeployPreview', { preview: request.preview, app: request.app })
        await result('done', 'redeploying')
        await notify($, '🧪 Preview', `Redeploying ${request.app} on ${request.preview}`, await previewAction($, request.preview))
      } else if (request.type === 'create-preview') {
        const preview = await startPreviewFor($, request.app, request.branch)
        await result('done', preview ? `creating ${preview}` : 'creating')
        await notify($, '🧪 Preview', `Creating ${preview || 'a preview'} for ${request.app} ${request.branch}; a notification follows when it is up`, preview ? await previewAction($, preview) : 'space:inbox')
      } else if (request.type === 'stop-preview') {
        await call($, 'stopPreview', { preview: request.preview })
        await result('done', 'stopped')
        await notify($, '🧪 Preview', `${request.preview} stopped; it keeps its setup and extend switches it back on`, 'space:inbox')
      } else if (request.type === 'extend') {
        await extendPreview($, request.preview, request.days)
        await result('done', `extended by ${request.days} days`)
        await notify($, '🧪 Preview', `${request.preview} extended by ${request.days} days and redeploying`, await previewAction($, request.preview))
      } else {
        const state = await completeTicket($, request.ticket)
        const url = (await readJson<Inbox>($, `${await cacheDir($)}/inbox.json`))?.tickets.find(t => t.id === request.ticket)?.url
        await dropTicket($, request.ticket)
        await result('done', `moved to ${state}`)
        await notify($, `🎫 ${(await loadConfig($)).tickets.name}`, `${request.ticket} moved to ${state}`, await ticketAction($, request.ticket, url))
      }
      // Whatever changed, the next fetch picks it up now rather than in five minutes.
      await $.process.run(['touch', `${await cacheDir($)}/refresh-request`])
      onDone()
    } catch (err) {
      await result('failed', shortError(err as Error))
      await notify($, '⚠️ Cockpit', `${request.label}: ${(err as Error).message}`, 'space:inbox')
    }
  }
}
