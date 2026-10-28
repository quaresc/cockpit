import type { EngineInterface } from 'claude-code'

import type { Handoff } from '../types'

import { cacheDir, home, writeAtomic } from './config'

type $ = EngineInterface

// The runbook app's run button hands a step to a session, which then owes its outcome: this tool records it.
export const REPORT_TOOL = 'report_step'
export const REPORT_TOOL_FULL = `mcp__cockpit__${REPORT_TOOL}`

const OUTCOMES = ['done', 'failed', 'blocked', 'waiting'] as const
type Reported = (typeof OUTCOMES)[number]
const OPEN: Handoff['state'][] = ['sent', 'running', 'waiting']
const HANDOFF = /\bHandoff ([a-z0-9][\w.-]*):/

export const REPORT_SPEC = {
  name: REPORT_TOOL,
  description:
    'Records how a runbook step handed to this session went, and updates that step\'s status cell in the runbook. ' +
    'Call it once the step has run (done, failed or blocked), or with waiting when the step needs the person\'s go before it runs.',
  inputSchema: {
    type: 'object',
    properties: {
      handoff: { type: 'string', description: 'The handoff id the request named, e.g. eng-410-3' },
      outcome: { type: 'string', enum: OUTCOMES, description: 'done, failed, blocked, or waiting for the person\'s go' },
      message: { type: 'string', description: 'One line on what happened or what it waits for' },
    },
    required: ['handoff', 'outcome', 'message'],
  },
}

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec']

// The runbook status cell each final outcome writes, the way the runbook app's done button writes it.
const statusOf = (outcome: Exclude<Reported, 'waiting'>) => {
  const now = new Date()
  return { done: `✅ done ${String(now.getDate()).padStart(2, '0')} ${MONTHS[now.getMonth()]}`, failed: '❌ failed', blocked: '❌ blocked' }[outcome]
}

async function dirOf($: $) {
  return `${await cacheDir($)}/handoffs`
}

async function readHandoff($: $, id: string) {
  if (!/^[\w.-]+$/.test(id)) return null
  try {
    return JSON.parse(await $.fs.read(`${await dirOf($)}/${id}.json`)) as Handoff
  } catch {
    return null
  }
}

async function writeHandoff($: $, handoff: Handoff) {
  await $.fs.write(`${await dirOf($)}/${handoff.id}.json`, JSON.stringify(handoff))
}

async function listHandoffs($: $) {
  const entries = await $.fs.list(await dirOf($)).catch(() => [])
  const found = await Promise.all(entries.filter(e => e.name.endsWith('.json')).map(e => readHandoff($, e.name.slice(0, -5))))
  return found.filter((h): h is Handoff => h !== null)
}

const isOpen = (h: Handoff) => OPEN.includes(h.state)

// Sets one step's status cell, the fourth column of its `| # | Step | Where | Status | Command |` row.
async function setStatus($: $, path: string, step: string, status: string) {
  const text = await $.fs.read(path).catch(() => null)
  if (text === null) return false
  const lines = text.split('\n')
  const index = lines.findIndex(l => l.trim().startsWith('|') && l.split('|').length > 5 && l.split('|')[1]?.trim() === step)
  if (index < 0) return false
  const cells = lines[index]!.split('|')
  cells[4] = ` ${status} `
  lines[index] = cells.join('|')
  await $.fs.write(path, lines.join('\n'))
  return true
}

// The request that names a handoff binds it to the session it reached; a person's next prompt resumes one that waited.
export async function bindHandoff($: $, text: string, pane: string | undefined, isFromPlugin: boolean) {
  const session = await $.session.id()
  const id = text.match(HANDOFF)?.[1]
  if (id) {
    const handoff = await readHandoff($, id)
    if (handoff?.state === 'sent') await writeHandoff($, { ...handoff, state: 'running', session, pane, isNudged: false })
    return
  }
  if (isFromPlugin) return
  for (const handoff of await listHandoffs($)) {
    if (handoff.session === session && handoff.state === 'waiting') await writeHandoff($, { ...handoff, state: 'running', isNudged: false })
  }
}

export async function reportStep($: $, input: Record<string, unknown>): Promise<{ result: string } | { deny: string }> {
  const id = typeof input.handoff === 'string' ? input.handoff : ''
  const outcome = input.outcome as Reported
  const message = typeof input.message === 'string' ? input.message.trim() : ''
  const handoff = await readHandoff($, id)
  if (!handoff) {
    const open = (await listHandoffs($)).filter(isOpen).map(h => h.id)
    return { deny: `no handoff ${JSON.stringify(id)}; open handoffs: ${open.join(', ') || 'none'}` }
  }
  if (!isOpen(handoff)) {
    return { deny: `handoff ${id} was already reported ${handoff.state}${handoff.message ? ` (${handoff.message})` : ''}; there is nothing left to report` }
  }
  if (!OUTCOMES.includes(outcome)) return { deny: `outcome must be one of ${OUTCOMES.join(', ')}, got ${JSON.stringify(input.outcome)}` }
  if (!message) return { deny: 'message must say in one line what happened, or what the step waits for' }
  const name = `step ${handoff.step} of ${handoff.slug.toUpperCase()}`
  if (outcome === 'waiting') {
    await writeHandoff($, { ...handoff, state: 'waiting', message })
    return { result: `Recorded: ${name} waits for the person's go. Once it has run, report done, failed or blocked.` }
  }
  const status = statusOf(outcome)
  const isWritten = await setStatus($, handoff.path, handoff.step, status)
  await writeHandoff($, { ...handoff, state: outcome, message, reportedAt: Date.now() })
  return {
    result: isWritten
      ? `Recorded: ${name} ${outcome}; its runbook status now reads "${status}".`
      : `Recorded: ${name} ${outcome}, but no row ${handoff.step} was found in ${handoff.path}: set its status there yourself.`,
  }
}

// At the end of a turn, a handoff this session holds with nothing reported gets one reminder, then is marked unreported.
export async function settleTurn($: $) {
  const session = await $.session.id()
  for (const handoff of await listHandoffs($)) {
    if (handoff.session !== session || handoff.state !== 'running') continue
    if (handoff.isNudged) {
      await writeHandoff($, { ...handoff, state: 'unreported' })
      continue
    }
    await writeHandoff($, { ...handoff, isNudged: true })
    void $.prompt.submit({
      text: `Step ${handoff.step} of the ${handoff.slug.toUpperCase()} runbook (handoff ${handoff.id}) has no reported outcome yet. ` +
        `Call ${REPORT_TOOL_FULL} with done, failed or blocked, or with waiting while it needs my go.`,
    })
  }
}

// Asks the runbook app for a background refresh of the ticket's runbook; false when there is no runbook or no app running.
export async function requestRunbookRefresh($: $, id: string, reason: string) {
  const slug = id.toLowerCase()
  const hasBook = await $.fs.exists(`${await home($)}/.config/cockpit/runbooks/${slug}.md`)
  // The text, Textual or Rust runbook app, whichever the apps setting runs.
  const isRunning = (await $.process.run(['pgrep', '-f', 'runbook(_tx)?(\\.py)? cockpit-space'])).exitCode === 0
  if (!hasBook || !isRunning) return false
  const dir = `${await cacheDir($)}/runbook-refresh`
  await $.process.run(['mkdir', '-p', dir])
  await writeAtomic($, `${dir}/${slug}.json`, JSON.stringify({ slug, reason }))
  return true
}

// The session that ran /runbook for a ticket: the one the runbook app hands that runbook's steps to.
export async function recordRunbookOwner($: $, ticket: string, pane: string) {
  const dir = `${await cacheDir($)}/runbook-owners`
  await $.process.run(['mkdir', '-p', dir])
  await writeAtomic($, `${dir}/${ticket.toLowerCase()}.json`, JSON.stringify({ pane, session: await $.session.id() }))
}
