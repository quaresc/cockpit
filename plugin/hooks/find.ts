import type { EngineInterface } from 'claude-code'

import { home } from './config'

type $ = EngineInterface

// Two tools over the hub: the palette's search across cockpit, and what changed in its shared data. Each runs the
// `cockpit` CLI through cockpit.sh and hands back its JSON as is.
export const SEARCH_TOOL = 'cockpit_search'
export const SEARCH_TOOL_FULL = `mcp__cockpit__${SEARCH_TOOL}`
export const CHANGES_TOOL = 'cockpit_changes'
export const CHANGES_TOOL_FULL = `mcp__cockpit__${CHANGES_TOOL}`

const APPS = ['inbox', 'monitor'] as const

export const SEARCH_SPEC = {
  name: SEARCH_TOOL,
  description:
    'Searches everything cockpit knows, as its palette does: the person\'s PRs and the reviews asked of them, their tickets, ' +
    'runbooks and their steps, show views, to-dos, meetings, monitored apps and their releases, the week\'s events (a review ' +
    'requested, CI turning red, a ticket moving, a release judged) and past Claude sessions by what was said in them. Use it ' +
    'to find a PR, ticket, runbook, meeting, release or past session the person names loosely ("the quota PR", "yesterday\'s ' +
    'session on dark mode") before asking them for a link or an id. Returns JSON {entries: [{app, kind, title, help, target}]}, ' +
    'best first; a PR\'s target key is its URL, a ticket\'s its id.',
  inputSchema: {
    type: 'object',
    properties: { query: { type: 'string', description: 'Words to match, fuzzily: a ticket id, a repo, part of a title' } },
    required: ['query'],
  },
  isDeferred: false,
}

export const CHANGES_SPEC = {
  name: CHANGES_TOOL,
  description:
    'What changed in the person\'s work, newest first, from the week of history cockpit keeps: reviews requested, PRs opened, ' +
    'approved, merged or asked for changes, CI turning red or green, tickets moving status, releases judged or moving their ' +
    'numbers. Use it for "what changed since this morning", "what happened while I was away", "anything new on my PRs". ' +
    'Returns JSON {events: [{at, when, app, doc, key, title}]}.',
  inputSchema: {
    type: 'object',
    properties: {
      since: { type: 'string', description: 'How far back: 30m, 2h, 1d (the default) up to 7d, or epoch milliseconds' },
      app: { type: 'string', enum: APPS, description: 'Only the inbox (PRs, reviews, CI, tickets) or only the monitor (releases)' },
    },
  },
  isDeferred: false,
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
  const { exitCode, stdout, stderr } = await $.process.run(['sh', command, ...args], { timeoutMs: 20_000 })
  // cockpit.sh says nothing when the plugin has no binary yet: /cockpit:doctor tells why.
  if (exitCode !== 0 || stdout.trim() === '') return { deny: stderr.trim() || `cockpit ${args[0]}: no cockpit binary; run /cockpit:doctor` }
  return { result: stdout.trim() }
}
