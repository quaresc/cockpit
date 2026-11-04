import type { EngineInterface } from 'claude-code'

import { home } from './config'
import { MCP } from './tools'

type $ = EngineInterface

// Two tools over the hub: the palette's search across cockpit, and what changed in its shared data. Each runs the
// `cockpit` CLI through cockpit.sh and hands back its JSON as is.
const SEARCH = MCP.tools.cockpit_search
const CHANGES = MCP.tools.cockpit_changes
const READ = MCP.tools.cockpit_read
const HEALTH = MCP.tools.cockpit_health
export const SEARCH_TOOL = SEARCH.name
export const SEARCH_TOOL_FULL = `mcp__cockpit__${SEARCH_TOOL}` as const
export const CHANGES_TOOL = CHANGES.name
export const CHANGES_TOOL_FULL = `mcp__cockpit__${CHANGES_TOOL}` as const

const APPS = CHANGES.inputSchema.properties.app.enum

export const SEARCH_SPEC = { name: SEARCH.name, description: SEARCH.description.join(''), inputSchema: SEARCH.inputSchema, isDeferred: false }

export const CHANGES_SPEC = { name: CHANGES.name, description: CHANGES.description.join(''), inputSchema: CHANGES.inputSchema, isDeferred: false }

// Reads from the person's tools through cockpit's own servers, for any agent and any tool.
export const READ_TOOL_FULL = `mcp__cockpit__${READ.name}` as const
export const HEALTH_TOOL_FULL = `mcp__cockpit__${HEALTH.name}` as const
export const READ_SPEC = { name: READ.name, description: READ.description.join(''), inputSchema: READ.inputSchema, isDeferred: true }
export const HEALTH_SPEC = { name: HEALTH.name, description: HEALTH.description.join(''), inputSchema: HEALTH.inputSchema, isDeferred: true }

export function readArgs(input: Record<string, unknown>): string[] | string {
  const sources = READ.inputSchema.properties.source.enum as readonly string[]
  if (typeof input.source !== 'string' || !sources.includes(input.source)) return `source: one of ${sources.join(', ')}`
  if (typeof input.verb !== 'string' || input.verb === '') return 'verb: one of the source\'s read verbs'
  const values = input.values ?? {}
  if (typeof values !== 'object' || values === null || Array.isArray(values)) return 'values: an object of the verb\'s placeholders'
  return ['connector', 'read', input.source, input.verb, JSON.stringify(values)]
}

export function healthArgs(input: Record<string, unknown>): string[] | string {
  if (typeof input.service !== 'string' || input.service.trim() === '') return 'service: the service\'s name as the metrics tool knows it'
  const args = ['connector', 'health', input.service.trim()]
  if (typeof input.from === 'string') args.push(input.from)
  if (typeof input.to === 'string' && input.to !== 'now') args.push(input.to)
  return args
}

// The CLI's arguments for a search, or what is wrong with the input.
export function searchArgs(input: Record<string, unknown>): string[] | string {
  const query = typeof input.query === 'string' ? input.query.trim() : ''
  if (!query) return 'query: the words to search for, e.g. a ticket id or part of a PR title'
  return ['search', query]
}

// The CLI's arguments for the changes, or what is wrong with the input.
export function changesArgs(input: Record<string, unknown>): string[] | string {
  const args = ['events']
  if (input.since !== undefined) {
    const since = typeof input.since === 'string' || typeof input.since === 'number' ? String(input.since).trim() : ''
    if (!/^\d+[mhd]$/.test(since) && !/^\d{13,}$/.test(since)) return `since: 30m, 2h, 1d or epoch milliseconds, got ${JSON.stringify(input.since)}`
    args.push('--since', since)
  }
  if (input.app !== undefined) {
    if (!APPS.includes(input.app as (typeof APPS)[number])) return `app: one of ${APPS.join(', ')}, got ${JSON.stringify(input.app)}`
    args.push('--app', input.app as string)
  }
  return args
}

// Runs the CLI with the arguments the input makes; its JSON, or why there is none.
export async function find($: $, input: Record<string, unknown>, argsOf: (input: Record<string, unknown>) => string[] | string) {
  const args = argsOf(input)
  if (typeof args === 'string') return { deny: args }
  const command = `${await home($)}/.config/cockpit/herdr/cockpit.sh`
  const { exitCode, stdout, stderr } = await $.process.run(['sh', command, ...args], { timeoutMs: 60_000 })
  // cockpit.sh says nothing when the plugin has no binary yet: /cockpit:doctor tells why.
  if (exitCode !== 0 || stdout.trim() === '') return { deny: stderr.trim() || `cockpit ${args[0]}: no cockpit binary; run /cockpit:doctor` }
  return { result: stdout.trim() }
}
