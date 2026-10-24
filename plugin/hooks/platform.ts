import type { EngineInterface } from 'claude-code'

import { cacheDir, loadConfig } from './config'
import type { Platform } from './config'

type $ = EngineInterface

// Deploys and sandboxes through whatever MCP tools the platform connector offers. The config maps each capability to a
// tool and its arguments, and each value cockpit reads to a path in the tool's answer; /cockpit:setup writes both.

export type Capability =
  | 'deployStatus'
  | 'deployHistory'
  | 'listSandboxes'
  | 'sandboxStatus'
  | 'createSandbox'
  | 'redeploySandbox'
  | 'stopSandbox'
  | 'startSandbox'
  | 'extendSandbox'

export type App = { id: string; repo: string; link?: string; tag?: string; sha?: string }
export type Deployment = { workflow: string; namespace: string; tag?: string; branch?: string }
export type Box = {
  stage: string
  expiresAt: string
  activated?: boolean
  health: string
  products: { name: string; enabled: boolean; branch?: string }[]
  raw: Record<string, unknown>
}

// A value at a dotted path ("status.health", "urls.0.url"); "a|b" takes the first path that has one.
export function pick(value: unknown, path: string | undefined): unknown {
  if (!path) return undefined
  for (const option of path.split('|')) {
    const found = option.split('.').reduce<unknown>((node, key) => (node == null ? undefined : (node as Record<string, unknown>)[key]), value)
    if (found !== undefined && found !== null) return found
  }
  return undefined
}

const text = (value: unknown) => (value === undefined || value === null ? undefined : String(value))
const list = (value: unknown) => (Array.isArray(value) ? (value as unknown[]) : [])

// The arguments with their "{name}" placeholders filled: a whole-string placeholder keeps the value's type.
export function fillArgs(template: unknown, values: Record<string, unknown>): unknown {
  if (typeof template === 'string') {
    const whole = template.match(/^\{(\w+)\}$/)
    if (whole) return values[whole[1]!]
    return template.replace(/\{(\w+)\}/g, (_, key: string) => String(values[key] ?? ''))
  }
  if (Array.isArray(template)) return template.map(t => fillArgs(t, values))
  if (template && typeof template === 'object') {
    return Object.fromEntries(Object.entries(template).map(([k, v]) => [k, fillArgs(v, values)]))
  }
  return template
}

export async function platform($: $): Promise<Platform | undefined> {
  const config = (await loadConfig($)).platform
  return config.enabled ? config : undefined
}

export async function can($: $, capability: Capability) {
  return Boolean((await platform($))?.tools[capability]?.tool)
}

// One call to the tool mapped to a capability, its first text block parsed as JSON; throws when it is not mapped.
export async function call($: $, capability: Capability, values: Record<string, unknown> = {}, extra: Record<string, unknown> = {}): Promise<unknown> {
  const config = await platform($)
  const recipe = config?.tools[capability]
  if (!config || !recipe?.tool) throw new Error(`no platform tool for ${capability}: run /cockpit:setup`)
  const args = { ...(fillArgs(recipe.args ?? {}, values) as Record<string, unknown>), ...extra }
  const result = await $.mcp.call(config.server, recipe.tool, args)
  const block = result.content.find(c => c.type === 'text')
  const body = block && 'text' in block ? String(block.text) : ''
  if (result.isError) throw new Error(`${config.server} ${recipe.tool}: ${body.slice(0, 160)}`)
  return body ? JSON.parse(body) : {}
}

const APPS_TTL_MS = 10 * 60_000
const DEPLOYS_TTL_MS = 5 * 60_000

// The deployed apps, with the repo each one builds from; cached on disk for every session.
export async function deployedApps($: $): Promise<App[]> {
  const path = `${await cacheDir($)}/deployed-apps.json`
  try {
    const cached = JSON.parse(await $.fs.read(path)) as { at: number; apps: App[] }
    if (Date.now() - cached.at < APPS_TTL_MS) return cached.apps
  } catch {
    // Not cached yet.
  }
  const config = await platform($)
  if (!config || !(await can($, 'deployStatus'))) return []
  const f = config.fields
  const skip = new Set(f.skipKinds ?? [])
  const apps = list(pick(await call($, 'deployStatus'), f.apps))
    .filter(a => !skip.has(text(pick(a, f.appKind)) ?? ''))
    .map((a): App => ({
      id: text(pick(a, f.appId)) ?? '',
      repo: text(pick(a, f.appRepo)) ?? '',
      link: text(pick(a, f.appLink)),
      tag: text(pick(a, f.appTag)),
      sha: text(pick(a, f.appSha)),
    }))
  await $.fs.write(path, JSON.stringify({ at: Date.now(), apps }))
  return apps
}

export const appFor = (apps: App[], repo: string) => apps.find(a => a.repo.toLowerCase().replace(/\.git$/, '').endsWith(`/${repo.toLowerCase()}`))

const deploys = new Map<string, { at: number; latest: Deployment | null }>()

export async function latestDeploy($: $, app: string): Promise<Deployment | null> {
  const hit = deploys.get(app)
  if (hit && Date.now() - hit.at < DEPLOYS_TTL_MS) return hit.latest
  const config = await platform($)
  if (!config || !(await can($, 'deployHistory'))) return null
  const f = config.fields
  const first = list(pick(await call($, 'deployHistory', { app }), f.deployments))[0]
  const latest = first
    ? { workflow: text(pick(first, f.deployWorkflow)) ?? '', namespace: text(pick(first, f.deployNamespace)) ?? '', tag: text(pick(first, f.deployTag)), branch: text(pick(first, f.deployBranch)) }
    : null
  deploys.set(app, { at: Date.now(), latest })
  return latest
}

export async function listSandboxes($: $, values: Record<string, unknown> = {}): Promise<Box[]> {
  const config = await platform($)
  if (!config || !(await can($, 'listSandboxes'))) return []
  const f = config.fields
  return list(pick(await call($, 'listSandboxes', values), f.sandboxes)).map(raw => ({
    stage: text(pick(raw, f.sandboxId)) ?? '',
    expiresAt: text(pick(raw, f.sandboxExpires)) ?? '',
    activated: pick(raw, f.sandboxActive) as boolean | undefined,
    health: text(pick(raw, f.sandboxHealth)) ?? '',
    products: list(pick(raw, f.sandboxProducts)).map(p => ({
      name: text(pick(p, f.productName)) ?? '',
      enabled: pick(p, f.productEnabled) !== false,
      branch: text(pick(p, f.productBranch)),
    })),
    raw: raw as Record<string, unknown>,
  }))
}

// A product's first URL on a sandbox, cached ten minutes per sandbox.
const statusCache = new Map<string, { at: number; status: unknown }>()
export async function sandboxUrl($: $, stage: string, product: string) {
  const config = await platform($)
  if (!config || !(await can($, 'sandboxStatus'))) return undefined
  const hit = statusCache.get(stage)
  const status = hit && Date.now() - hit.at < 10 * 60_000 ? hit.status : await call($, 'sandboxStatus', { stage })
  statusCache.set(stage, { at: Date.now(), status })
  const f = config.fields
  const item = list(pick(status, f.statusProducts)).find(p => text(pick(p, f.productName)) === product)
  return text(pick(item, f.statusUrl))
}

export async function createSandbox($: $, product: string, branch: string) {
  const config = await platform($)
  const created = await call($, 'createSandbox', { product, branch }, config?.sandboxDefaults ?? {})
  return text(pick(created, config?.fields.createdId)) ?? ''
}

// Adds days to a sandbox (expired or not) and switches it back on. The fields `keep` names are sent back as they are,
// so a tool that replaces the whole setup does not reset them to its defaults.
export async function extendSandbox($: $, stage: string, days: number) {
  const config = await platform($)
  const box = (await listSandboxes($)).find(b => b.stage === stage)
  if (!box) throw new Error(`sandbox ${stage} not found`)
  const keep = config?.tools.extendSandbox?.keep ?? []
  const kept = Object.fromEntries(keep.filter(key => box.raw[key] !== undefined).map(key => [key, box.raw[key]]))
  await call($, 'extendSandbox', { stage, days }, kept)
  if (!box.activated && (await can($, 'startSandbox'))) await call($, 'startSandbox', { stage })
}
