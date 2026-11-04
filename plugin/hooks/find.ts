import type { EngineInterface } from 'claude-code'

import { home } from './config'
import { MCP } from './tools'

type $ = EngineInterface

// Two tools over the hub: the palette's search across cockpit, and what changed in its shared data. Each runs the
// `cockpit` CLI through cockpit.sh and hands back its JSON as is.
const SEARCH = MCP.tools.cockpit_search
const CHANGES = MCP.tools.cockpit_changes
export const SEARCH_TOOL = SEARCH.name
export const SEARCH_TOOL_FULL = `mcp__cockpit__${SEARCH_TOOL}` as const
export const CHANGES_TOOL = CHANGES.name
export const CHANGES_TOOL_FULL = `mcp__cockpit__${CHANGES_TOOL}` as const

const APPS = CHANGES.inputSchema.properties.app.enum

export const SEARCH_SPEC = { name: SEARCH.name, description: SEARCH.description.join(''), inputSchema: SEARCH.inputSchema, isDeferred: false }

export const CHANGES_SPEC = { name: CHANGES.name, description: CHANGES.description.join(''), inputSchema: CHANGES.inputSchema, isDeferred: false }

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
  const { exitCode, stdout, stderr } = await $.process.run(['sh', command, ...args], { timeoutMs: 20_000 })
  // cockpit.sh says nothing when the plugin has no binary yet: /cockpit:doctor tells why.
  if (exitCode !== 0 || stdout.trim() === '') return { deny: stderr.trim() || `cockpit ${args[0]}: no cockpit binary; run /cockpit:doctor` }
  return { result: stdout.trim() }
}
