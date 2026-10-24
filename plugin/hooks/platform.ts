import type { EngineInterface } from 'claude-code'

import { cacheDir, loadConfig } from './config'
import type { Platform, Recipe } from './config'

type $ = EngineInterface

// Deploys and preview environments through whatever MCP tools the platform connector offers. The config maps each capability to a
// tool and its arguments, and each value cockpit reads to a path in the tool's answer; /cockpit:setup writes both.

export type Capability =
  | 'deployStatus'
  | 'deployHistory'
  | 'listPreviews'
  | 'previewStatus'
  | 'createPreview'
  | 'redeployPreview'
  | 'stopPreview'
  | 'startPreview'
  | 'extendPreview'

export type App = { id: string; repo: string; link?: string; tag?: string; sha?: string }
export type Deployment = { workflow: string; namespace: string; tag?: string; branch?: string }
export type PreviewEnv = {
  id: string
  expiresAt: string
  activated?: boolean
  health: string
  apps: { name: string; enabled: boolean; branch?: string }[]
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

export const text = (value: unknown) => (value === undefined || value === null ? undefined : String(value))
export const list = (value: unknown) => (Array.isArray(value) ? (value as unknown[]) : [])

// The list at a dotted path of an answer, where a `[]` segment flattens a list on the way
// ("attributes.parallel_executions[].steps"); an empty path is the answer itself.
export function listAt(value: unknown, path: string | undefined) {
  if (!path) return list(value)
  const nodes = path.split('.').reduce<unknown[]>(
    (found, key) => (key.endsWith('[]') ? found.flatMap(n => list(pick(n, key.slice(0, -2)))) : found.map(n => pick(n, key))),
    [value],
  )
  return nodes.flatMap(list)
}

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

// One call to a recipe's tool with its placeholders filled, its first text block parsed as JSON.
export async function callRecipe($: $, server: string, recipe: Recipe, values: Record<string, unknown> = {}, extra: Record<string, unknown> = {}): Promise<unknown> {
  // A placeholder without a value (a first page's token) leaves its argument out.
  const filled = Object.entries(fillArgs(recipe.args ?? {}, values) as Record<string, unknown>).filter(([, v]) => v !== undefined)
  const args = { ...Object.fromEntries(filled), ...extra }
  const result = await $.mcp.call(server, recipe.tool, args)
  const block = result.content.find(c => c.type === 'text')
  const body = block && 'text' in block ? String(block.text) : ''
  if (result.isError) throw new Error(`${server} ${recipe.tool}: ${body.slice(0, 160)}`)
  try {
    return body ? (JSON.parse(body) as unknown) : {}
  } catch {
    throw new Error(`${server} ${recipe.tool} answered no JSON: ${body.slice(0, 160)}`)
  }
}

// One call to the tool a connector's mapping gives a capability; throws when none is mapped.
export async function callMapped<C extends string>(
  $: $,
  connector: { name?: string; server: string; tools: Partial<Record<C, Recipe>> },
  capability: C,
  values: Record<string, unknown> = {},
  extra: Record<string, unknown> = {},
): Promise<unknown> {
  const recipe = connector.tools[capability]
  if (!recipe?.tool) throw new Error(`no ${connector.name ?? 'platform'} tool for ${capability}: run /cockpit:setup`)
  return callRecipe($, connector.server, recipe, values, extra)
}

// One call to the platform tool mapped to a capability.
export async function call($: $, capability: Capability, values: Record<string, unknown> = {}, extra: Record<string, unknown> = {}): Promise<unknown> {
  const config = await platform($)
  if (!config) throw new Error(`no platform tool for ${capability}: run /cockpit:setup`)
  return callMapped($, config, capability, values, extra)
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

export async function latestDeploy($: $, appId: string): Promise<Deployment | null> {
  const hit = deploys.get(appId)
  if (hit && Date.now() - hit.at < DEPLOYS_TTL_MS) return hit.latest
  const config = await platform($)
  if (!config || !(await can($, 'deployHistory'))) return null
  const f = config.fields
  const first = list(pick(await call($, 'deployHistory', { appId }), f.deployments))[0]
  const latest = first
    ? { workflow: text(pick(first, f.deployWorkflow)) ?? '', namespace: text(pick(first, f.deployNamespace)) ?? '', tag: text(pick(first, f.deployTag)), branch: text(pick(first, f.deployBranch)) }
    : null
  deploys.set(appId, { at: Date.now(), latest })
  return latest
}

export async function listPreviews($: $, values: Record<string, unknown> = {}): Promise<PreviewEnv[]> {
  const config = await platform($)
  if (!config || !(await can($, 'listPreviews'))) return []
  const f = config.fields
  return list(pick(await call($, 'listPreviews', values), f.previews)).map(raw => ({
    id: text(pick(raw, f.previewId)) ?? '',
    expiresAt: text(pick(raw, f.previewExpires)) ?? '',
    activated: pick(raw, f.previewActive) as boolean | undefined,
    health: text(pick(raw, f.previewHealth)) ?? '',
    apps: list(pick(raw, f.previewApps)).map(a => ({
      name: text(pick(a, f.appName)) ?? '',
      enabled: pick(a, f.appEnabled) !== false,
      branch: text(pick(a, f.appBranch)),
    })),
    raw: raw as Record<string, unknown>,
  }))
}

// An app's first URL on a preview, cached ten minutes per preview.
const statusCache = new Map<string, { at: number; status: unknown }>()
export async function previewUrl($: $, preview: string, app: string) {
  const config = await platform($)
  if (!config || !(await can($, 'previewStatus'))) return undefined
  const hit = statusCache.get(preview)
  const status = hit && Date.now() - hit.at < 10 * 60_000 ? hit.status : await call($, 'previewStatus', { preview })
  statusCache.set(preview, { at: Date.now(), status })
  const f = config.fields
  const item = list(pick(status, f.statusApps)).find(a => text(pick(a, f.appName)) === app)
  return text(pick(item, f.statusUrl))
}

export async function createPreview($: $, app: string, branch: string) {
  const config = await platform($)
  const created = await call($, 'createPreview', { app, branch }, config?.previewDefaults ?? {})
  return text(pick(created, config?.fields.createdId)) ?? ''
}

// Adds days to a preview (expired or not) and switches it back on. The fields `keep` names are sent back as they are,
// so a tool that replaces the whole setup does not reset them to its defaults.
export async function extendPreview($: $, preview: string, days: number) {
  const config = await platform($)
  const env = (await listPreviews($)).find(p => p.id === preview)
  if (!env) throw new Error(`preview ${preview} not found`)
  const keep = config?.tools.extendPreview?.keep ?? []
  const kept = Object.fromEntries(keep.filter(key => env.raw[key] !== undefined).map(key => [key, env.raw[key]]))
  await call($, 'extendPreview', { preview, days }, kept)
  if (!env.activated && (await can($, 'startPreview'))) await call($, 'startPreview', { preview })
}

// What a dashboard link is, from its host, for the buttons and chips that open it.
export function dashboardName(url: string) {
  const host = url.match(/^https?:\/\/([^/]+)/)?.[1] ?? ''
  if (/(^|\.)argo(cd)?[.-]/.test(host)) return 'ArgoCD'
  if (host.endsWith('vercel.com')) return 'Vercel'
  if (host.endsWith('netlify.app') || host.endsWith('netlify.com')) return 'Netlify'
  if (host.endsWith('render.com')) return 'Render'
  if (host.endsWith('railway.app')) return 'Railway'
  if (host.endsWith('fly.io')) return 'Fly'
  return 'Dashboard'
}
