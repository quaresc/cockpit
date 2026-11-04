import type { EngineInterface } from 'claude-code'

import { home } from './config'
import type { Here } from './herdr'
import { writeDoc } from './store'
import { ownVersion } from './upkeep'

type $ = EngineInterface

// What this session tells the hub about itself for the `agents` view, in both frontends: the store document `agents`,
// keyed by the session's handle (`app:t3`, or a herdr pane id). The shape is in docs/app.md.

export type Facts = {
  session: string
  transcript: string | null
  cwd: string | null
  repo: string | null
  branch: string | null
  ticket: string | null
  task: string | null
  page: string | null
  pageTitle: string | null
  context: number | null
  cost: number | null
  account: string | null
  frontend: 'tui' | 'app'
  terminal: string | null
  space: string
  tab: string
  version: string
  pr: { number: number; url: string; failing: boolean } | null
  ci: { running: boolean; done: number; total: number } | null
  clash: { label: string; space: string; tab: string | null } | null
  views: { name: string; open: boolean }[]
  last: { at: number; durationMs: number; text: string } | null
}

const STALE_MS = 60_000

let known: Partial<Facts> = {}
let sent = { handle: '', body: '', at: 0 }

// Where Claude keeps a conversation: its folder named after the working directory, every other character a dash.
export function transcriptPath(homeDir: string, cwd: string, session: string) {
  return `${homeDir}/.claude/projects/${cwd.replace(/[^A-Za-z0-9]/g, '-')}/${session}.jsonl`
}

// Merges what changed into this session's facts and sends them when they differ from the last sent, or a minute on.
export async function reportFacts($: $, here: Here | undefined, change: Partial<Facts> = {}) {
  known = { ...known, ...change }
  if (!here) return
  const session = await $.session.id()
  const cwd = known.cwd ?? (await $.session.cwd()) ?? null
  const facts: Facts = {
    transcript: cwd ? transcriptPath(await home($), cwd, session) : null,
    repo: null, branch: null, ticket: null, task: null, page: null, pageTitle: null, context: null, cost: null, account: null,
    pr: null, ci: null, clash: null, views: [], last: null,
    ...known,
    session,
    cwd,
    frontend: here.frontend ?? 'tui',
    terminal: here.terminal ?? null,
    space: here.space,
    tab: here.tab,
    version: await ownVersion($),
  }
  const body = JSON.stringify(facts)
  if (sent.handle === here.pane && sent.body === body && Date.now() - sent.at < STALE_MS) return
  // A session moved to another pane leaves its old row.
  const gone = sent.handle && sent.handle !== here.pane ? { [sent.handle]: null } : {}
  await writeDoc($, 'agents', { ...gone, [here.pane]: { ...facts, at: Date.now() } }, 'patch')
  sent = { handle: here.pane, body, at: Date.now() }
}

// The session ended or was cleared: its row goes.
export async function forgetFacts($: $, handle: string | undefined) {
  const last = handle ?? sent.handle
  known = {}
  sent = { handle: '', body: '', at: 0 }
  if (last) await writeDoc($, 'agents', { [last]: null }, 'patch')
}
