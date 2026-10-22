import type { EngineInterface } from 'claude-code'

import type { CiFailure, CiRun, CiState, DeployedPr, Inbox, MergedPr, Sandbox } from '../types'

import { cacheDir, loadConfig } from './config'

type $ = EngineInterface

const CI_STALE_MS = 55_000
const CI_VERSION = 2
const LOCK_STALE_MS = 2 * 60_000

// ---------------------------------------------------------------------------
// Shared plumbing

export async function notify($: $, title: string, body: string) {
  if (!(await loadConfig($)).notifications) return
  await $.process.run(['herdr', 'notification', 'show', title, '--body', body, '--sound', 'request'], { timeoutMs: 10_000 })
}

// A refused background call, said in one line that names the permission rule to add.
export function shortError(err: Error) {
  const tool = err.message.match(/no verdict for (mcp__\S+?):/)?.[1]
  return tool ? `blocked by auto mode: allow ${tool}` : err.message.slice(0, 200)
}

export type Source = 'linear' | 'circleci' | 'platform' | 'calendar'

// One MCP call to a source's configured server, its first text block parsed as JSON; throws on a tool error.
export async function mcp<T>($: $, source: Source, tool: string, args: Record<string, unknown>): Promise<T> {
  const server = (await loadConfig($))[source].server
  const result = await $.mcp.call(server, tool, args)
  const block = result.content.find(c => c.type === 'text')
  const text = block && 'text' in block ? String(block.text) : ''
  if (result.isError) throw new Error(`${server} ${tool}: ${text.slice(0, 160)}`)
  return JSON.parse(text) as T
}

// A cross-session lock, so one Claude session fetches and notifies for all of them.
export async function withLock<T>($: $, name: string, fn: () => Promise<T>): Promise<T | undefined> {
  const lock = `${await cacheDir($)}/${name}.lock`
  try {
    const age = Date.now() - (await $.fs.stat(lock)).mtimeMs
    if (age > LOCK_STALE_MS) await $.process.run(['rmdir', lock])
  } catch {
    // No lock yet.
  }
  if ((await $.process.run(['mkdir', '-p', await cacheDir($)])).exitCode !== 0) return undefined
  if ((await $.process.run(['mkdir', lock])).exitCode !== 0) return undefined
  try {
    return await fn()
  } finally {
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
// CircleCI: the latest run of each branch I pushed, with progress, failure and image job

type RunNode = {
  id: string
  attributes: { created_at: string; current_outcome: string; phase: string }
  references: { event: { attributes: { vcs: { branch: string; origin_repository_url: string; tag: string } } } }
}
type WorkflowNode = { id: string; attributes: { name: string; outcome: string; phase: string; created_at: string } }
type JobNode = { id: string; attributes: { name: string; outcome: string; phase: string } }
type JobDetail = { attributes: { parallel_executions?: { steps: { name: string; exit_code?: number; outcome: string }[] }[] } }
type TestNode = { name?: string; classname?: string; message?: string }

async function failureOf($: $, job: JobNode): Promise<CiFailure> {
  const detail = await mcp<JobDetail>($, 'circleci', 'get_job', { job: job.id })
  const steps = detail.attributes.parallel_executions?.flatMap(p => p.steps) ?? []
  const step = steps.find(s => s.outcome === 'failed' || (s.exit_code !== undefined && s.exit_code !== 0))?.name
  const tests = await mcp<{ tests?: TestNode[] }>($, 'circleci', 'list_job_tests', { job: job.id, limit: 3 })
    .then(t => (t.tests ?? []).map(x => x.name ?? x.classname ?? '?'))
    .catch(() => [] as string[])
  return { job: job.attributes.name, jobId: job.id, step, tests }
}

async function detailsOf($: $, run: RunNode, base: Omit<CiRun, 'done' | 'total'>): Promise<CiRun> {
  const listed = await mcp<{ workflows: WorkflowNode[] | null }>($, 'circleci', 'list_run_workflows', { run: run.id })
  // A rerun adds a workflow of the same name to the run: only the newest of each name counts.
  const newest = new Map<string, WorkflowNode>()
  for (const workflow of listed.workflows ?? []) {
    const seen = newest.get(workflow.attributes.name)
    if (!seen || seen.attributes.created_at < workflow.attributes.created_at) newest.set(workflow.attributes.name, workflow)
  }
  const workflows = [...newest.values()]
  const jobs: JobNode[] = []
  for (const workflow of workflows) {
    const listed = await mcp<{ jobs: JobNode[] | null }>($, 'circleci', 'list_workflow_jobs', { workflow: workflow.id })
    jobs.push(...(listed.jobs ?? []))
  }
  const failedJob = jobs.find(j => j.attributes.outcome === 'failed')
  const { imageJob } = (await loadConfig($)).circleci
  const docker = imageJob ? jobs.find(j => j.attributes.name === imageJob) : undefined
  return {
    ...base,
    workflowId: workflows.find(w => w.attributes.outcome === 'failed')?.id ?? workflows[0]?.id,
    done: jobs.filter(j => j.attributes.phase === 'ended').length,
    total: jobs.length,
    failed: failedJob ? await failureOf($, failedJob) : undefined,
    dockerTag: docker ? (docker.attributes.phase === 'ended' ? docker.attributes.outcome : 'running') : undefined,
  }
}

function ciChanges(prev: CiState, next: CiState) {
  const before = new Map(prev.runs.map(r => [`${r.repo}/${r.branch}`, r]))
  const out: string[] = []
  for (const run of next.runs) {
    if (run.isRunning) continue
    const was = before.get(`${run.repo}/${run.branch}`)
    const isNewlyEnded = was ? was.runId !== run.runId || was.isRunning : Date.parse(run.createdAt) > prev.at
    if (!isNewlyEnded) continue
    if (run.outcome === 'succeeded') out.push(`🟢 CI passed: ${run.repo} ${run.branch}`)
    else if (run.outcome === 'failed') out.push(`🔴 CI failed: ${run.repo} ${run.branch}${run.failed ? ` · ${run.failed.job}` : ''}`)
  }
  return out
}

let ciInflight: Promise<void> | undefined
let isCircleMissing = false

export function refreshCi($: $, isForced = false) {
  ciInflight ??= (async () => {
    try {
      if (isCircleMissing || !(await loadConfig($)).circleci.enabled) return
      // A cache from an older version of this code is rebuilt from scratch, without notifications.
      const stored = await readCi($)
      const prev = stored?.version === CI_VERSION ? stored : null
      if (!isForced && prev && Date.now() - prev.at < CI_STALE_MS) return
      await withLock($, 'ci', async () => {
        const listed = await mcp<{ runs: RunNode[] | null }>($, 'circleci', 'list_runs', { mine: true, limit: 30 }).catch(async (err: Error) => {
          // A session started before the CircleCI connector was added has no such server: leave it to the others.
          if (err.message.includes('no connected MCP tool')) {
            isCircleMissing = true
            return null
          }
          const failed: CiState = { at: prev?.at ?? 0, runs: prev?.runs ?? [], error: `CircleCI ${shortError(err)}` }
          await $.fs.write(`${await cacheDir($)}/ci.json`, JSON.stringify(failed))
          return null
        })
        if (!listed) return
        const { runs } = listed
        const latest = new Map<string, RunNode>()
        for (const run of runs ?? []) {
          const vcs = run.references.event.attributes.vcs
          const repo = vcs.origin_repository_url.split('/').pop()
          if (!repo || !vcs.branch || vcs.tag) continue
          const key = `${repo}/${vcs.branch}`
          const seen = latest.get(key)
          if (!seen || seen.attributes.created_at < run.attributes.created_at) latest.set(key, run)
        }
        const cached = new Map((prev?.runs ?? []).map(r => [r.runId, r]))
        const next: CiRun[] = []
        for (const [key, run] of latest) {
          const [repo, ...rest] = key.split('/')
          const base = {
            repo: repo ?? '?',
            branch: rest.join('/'),
            runId: run.id,
            createdAt: run.attributes.created_at,
            isRunning: run.attributes.phase !== 'ended',
            outcome: run.attributes.current_outcome,
          }
          const known = cached.get(run.id)
          // An ended run never changes, so its details are fetched once.
          next.push(known && !known.isRunning && !base.isRunning ? known : await detailsOf($, run, base).catch(() => ({ ...base, done: 0, total: 0 })))
        }
        const state: CiState = { at: Date.now(), runs: next, version: CI_VERSION }
        await $.fs.write(`${await cacheDir($)}/ci.json`, JSON.stringify(state))
        if (prev) for (const line of ciChanges(prev, state)) await notify($, '🔁 CircleCI', line)
      })
    } finally {
      ciInflight = undefined
    }
  })()
  return ciInflight
}

// ---------------------------------------------------------------------------
// Deploys: is each merged PR of a "To Deploy" ticket in the production image yet?

type DeployedApp = { repository?: string; imageTag: string | null; commitSha: string | null; name: string }

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
  if (waiting.length === 0 || !(await loadConfig($)).platform.enabled) return {}
  const { apps } = await mcp<{ apps: DeployedApp[] }>($, 'platform', 'deploys_status', { env: 'production' })
  const appOf = (repo: string) => apps.find(a => a.repository?.toLowerCase().endsWith(`/${repo.toLowerCase()}`))
  const out: Record<string, DeployedPr[]> = {}
  for (const ticket of waiting) {
    const prs = merged.filter(pr => pr.branch.toLowerCase().includes(ticket.id.toLowerCase()))
    out[ticket.id] = await Promise.all(
      prs.map(async pr => {
        const app = appOf(pr.repo)
        const ref = app?.imageTag ?? app?.commitSha ?? null
        return { repo: pr.repo, number: pr.number, tag: ref, isLive: ref ? await isContained($, pr.slug, pr.mergeSha, ref) : false }
      }),
    )
  }
  return out
}

export function deployChanges(prev: Inbox, next: Inbox) {
  const out: string[] = []
  for (const [ticket, prs] of Object.entries(next.deploys ?? {})) {
    const wasLive = (prev.deploys?.[ticket] ?? []).length > 0 && (prev.deploys?.[ticket] ?? []).every(p => p.isLive)
    if (prs.length > 0 && prs.every(p => p.isLive) && !wasLive) {
      out.push(`🚀 Live in production: ${ticket} (${prs.map(p => `${p.repo} ${p.tag}`).join(', ')})`)
    }
  }
  return out
}

// ---------------------------------------------------------------------------
// Sandboxes I own, and which branch each product runs there

type SandboxNode = {
  id: string
  expiresAt: string
  status: { health: string }
  products?: { name: string; enabled: boolean; revision?: string }[]
}

let login: string | undefined

export async function sandboxesFor($: $): Promise<Sandbox[]> {
  if (!(await loadConfig($)).platform.enabled) return []
  if (!login) {
    const { exitCode, stdout } = await $.process.run(['gh', 'api', 'user', '--jq', '.login'])
    if (exitCode !== 0) return []
    login = stdout.trim()
  }
  const { sandboxes } = await mcp<{ sandboxes: SandboxNode[] | null }>($, 'platform', 'sandboxes_list', {
    owner: login,
    withApps: true,
  })
  return (sandboxes ?? []).flatMap(box =>
    (box.products ?? [])
      .filter(p => p.enabled && p.revision && !['main', 'master'].includes(p.revision))
      .map(p => ({ stage: box.id, product: p.name, branch: p.revision ?? '', health: box.status.health, expiresAt: box.expiresAt })),
  )
}

// ---------------------------------------------------------------------------
// Requests the herdr inbox app drops in ~/.cache/cockpit/requests: CircleCI reruns, sandbox redeploys

type Request =
  | { type: 'rerun'; workflowId: string; label: string }
  | { type: 'redeploy'; stage: string; product: string; label: string }
  | { type: 'release'; ticket: string; label: string }
  | { type: 'extend'; stage: string; days: number; label: string }

type SandboxFlags = {
  id: string
  activated: boolean
  settingA?: boolean
  settingB?: boolean
  settingC?: boolean
  settingD?: boolean
  settingE?: boolean
}

// Adds days to a sandbox (expired or not) and switches it back on. Its flags are passed as they
// are, so the update does not reset monitoring and the rest to the sandbox defaults.
async function extend($: $, stage: string, days: number) {
  const { sandboxes } = await mcp<{ sandboxes: SandboxFlags[] | null }>($, 'platform', 'sandboxes_list', {})
  const box = (sandboxes ?? []).find(s => s.id === stage)
  if (!box) throw new Error(`sandbox ${stage} not found`)
  const keys = ['settingA', 'settingB', 'settingC', 'settingD', 'settingE'] as const
  const flags = Object.fromEntries(keys.filter(key => box[key] !== undefined).map(key => [key, box[key]]))
  await mcp($, 'platform', 'sandboxes_update', { stage, days: days, ...flags })
  if (!box.activated) await mcp($, 'platform', 'sandboxes_activate', { stage, activated: true })
}

type Status = { id: string; type: string; name: string }

// Moves a ticket to its team's "Done" state, else the team's first completed one.
async function release($: $, ticket: string) {
  const team = ticket.split('-')[0] ?? ''
  const statuses = await mcp<Status[]>($, 'linear', 'list_issue_statuses', { team })
  const done = statuses.find(s => s.type === 'completed' && /^done$/i.test(s.name)) ?? statuses.find(s => s.type === 'completed')
  if (!done) throw new Error(`no completed state in team ${team}`)
  await mcp($, 'linear', 'save_issue', { id: ticket, state: done.id })
  return done.name
}

// A ticket moved to a completed state leaves the inbox at once, without waiting for the next fetch.
async function dropTicket($: $, id: string) {
  const path = `${await cacheDir($)}/inbox.json`
  const inbox = await readJson<Inbox>($, path)
  if (!inbox) return
  await $.fs.write(path, JSON.stringify({ ...inbox, tickets: inbox.tickets.filter(t => t.id !== id) }))
}

export async function processRequests($: $, onDone: () => void) {
  const dir = `${await cacheDir($)}/requests`
  const entries = await $.fs.list(dir).catch(() => [])
  for (const entry of entries) {
    if (!entry.name.endsWith('.json') || entry.name.startsWith('claimed-')) continue
    const claimed = `${dir}/claimed-${entry.name}`
    // mv is atomic: whichever session moves the file handles it.
    if ((await $.process.run(['mv', `${dir}/${entry.name}`, claimed])).exitCode !== 0) continue
    const request = await readJson<Request>($, claimed)
    await $.process.run(['rm', '-f', claimed])
    if (!request) continue
    // The app waits for this file to show the outcome on the row it came from.
    const result = (ok: boolean, message: string) =>
      $.fs.write(`${dir}/results/${entry.name}`, JSON.stringify({ ok, message, at: Date.now() }))
    try {
      if (request.type === 'rerun') {
        await mcp($, 'circleci', 'rerun_workflow', { workflow: request.workflowId, from_failed: true })
        await result(true, 'rerun started')
        await notify($, '🔁 CircleCI', `Rerunning failed jobs: ${request.label}`)
      } else if (request.type === 'redeploy') {
        await mcp($, 'platform', 'sandboxes_redeploy', { stage: request.stage, applications: [request.product] })
        await result(true, 'redeploying')
        await notify($, '🧪 Sandbox', `Redeploying ${request.product} on ${request.stage}`)
      } else if (request.type === 'extend') {
        await extend($, request.stage, request.days)
        await result(true, `extended by ${request.days} days`)
        await notify($, '🧪 Sandbox', `${request.stage} extended by ${request.days} days and redeploying`)
      } else {
        const state = await release($, request.ticket)
        await dropTicket($, request.ticket)
        await result(true, `moved to ${state}`)
        await notify($, '🎫 Linear', `${request.ticket} moved to ${state}`)
      }
      // Whatever changed, the next fetch picks it up now rather than in five minutes.
      await $.process.run(['touch', `${await cacheDir($)}/refresh-request`])
      onDone()
    } catch (err) {
      await result(false, shortError(err as Error))
      await notify($, '⚠️ Cockpit', `${request.label}: ${(err as Error).message}`)
    }
  }
}
