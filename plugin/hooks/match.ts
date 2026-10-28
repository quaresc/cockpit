import type { CiState, Inbox, Pr } from '../types'

// Pure rules shared by the hooks, with no engine call, so tests run them directly.

const DAY_MS = 86_400_000

export const OLD_REVIEW_DAYS = 60

export const ageDays = (iso: string, now = Date.now()) => (now - Date.parse(iso)) / DAY_MS

export const isFailing = (pr: Pr) => pr.ci === 'FAILURE' || pr.ci === 'ERROR'
export const isOldReview = (pr: Pr, now = Date.now()) => ageDays(pr.updatedAt, now) > OLD_REVIEW_DAYS
export const isDeploying = (t: { statusType: string; status: string }) => t.statusType === 'started' && /deploy/i.test(t.status)

const escape = (text: string) => text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')

// Whether a branch or a text names the ticket as a whole token: eng-4-login does, eng-42-login does not.
export function hasTicket(text: string | undefined, id: string) {
  return Boolean(id) && new RegExp(`(^|[^a-z0-9])${escape(id.toLowerCase())}(?![0-9])`).test((text ?? '').toLowerCase())
}

// The team keys a ticket id may use: the configured ones and those of my assigned tickets.
export function teamKeys(inbox: Inbox | null, known: string[]) {
  return new Set([...known.map(t => t.toUpperCase()), ...(inbox?.tickets ?? []).map(t => t.id.split('-')[0] ?? '')].filter(Boolean))
}

// The tickets a text names, in order, limited to known team keys.
export function ticketsIn(text: string, inbox: Inbox | null, known: string[]) {
  const teams = teamKeys(inbox, known)
  return [...text.matchAll(/\b([A-Z][A-Z0-9]{1,9})-(\d{1,6})\b/gi)]
    .map(m => `${(m[1] ?? '').toUpperCase()}-${m[2]}`)
    .filter(id => teams.has(id.split('-')[0] ?? ''))
}

// The ticket a branch is named after, e.g. ENG-42 for me/eng-42-login-form; only a known team key counts.
export const ticketKey = (branch: string, inbox: Inbox | null, known: string[]) => ticketsIn(branch.replace(/[/_]/g, ' '), inbox, known)[0]

export function isNewer(latest: string, current: string) {
  const a = latest.split('.').map(Number)
  const b = current.split('.').map(Number)
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    if ((a[i] ?? 0) !== (b[i] ?? 0)) return (a[i] ?? 0) > (b[i] ?? 0)
  }
  return false
}

export type News = { text: string; action: string; buttons?: [string, string][] }

// What changed in my PRs and review requests since the previous fetch, worth a notification, each with where its click leads.
export function prChanges(prev: Inbox, next: Inbox) {
  const out: News[] = []
  const before = new Map(prev.mine.map(pr => [pr.url, pr]))
  const known = new Set(prev.direct.map(pr => pr.url))
  for (const pr of next.direct) {
    if (!known.has(pr.url)) out.push({ text: `👀 Review requested: ${pr.repo}#${pr.number} ${pr.title}`, action: pr.url, buttons: [['Review', `act:review:${pr.url}`]] })
  }
  for (const pr of next.mine) {
    const was = before.get(pr.url)
    if (!was) continue
    if (isFailing(pr) && !isFailing(was)) {
      out.push({ text: `🔴 CI failing: ${pr.repo}#${pr.number}`, action: `${pr.url}/checks`, buttons: [['Why?', `act:ci:${pr.url}`], ['Rerun', `act:rerun:${pr.url}`]] })
    }
    if (pr.review !== was.review && pr.review === 'APPROVED') out.push({ text: `✅ Approved: ${pr.repo}#${pr.number}`, action: pr.url })
    if (pr.review !== was.review && pr.review === 'CHANGES_REQUESTED') out.push({ text: `❌ Changes requested: ${pr.repo}#${pr.number}`, action: `${pr.url}/files` })
  }
  return out
}

// Tickets newly assigned to me, once there was a list to compare with.
export function ticketChanges(prev: Inbox, next: Inbox) {
  if (prev.tickets.length === 0) return []
  const ids = new Set(prev.tickets.map(t => t.id))
  return next.tickets.filter(t => !ids.has(t.id)).map((t): News => ({ text: `🎫 Assigned: ${t.id} ${t.title}`, action: t.url, buttons: [['Plan', `act:plan:${t.id}`]] }))
}

export const changes = (prev: Inbox, next: Inbox) => [...prChanges(prev, next), ...ticketChanges(prev, next)]

// A finished run's notification, its click opening the checks of the branch's PR.
export function ciChanges(prev: CiState, next: CiState, inbox: Inbox | null) {
  const before = new Map(prev.runs.map(r => [`${r.repo}/${r.branch}`, r]))
  const out: News[] = []
  for (const run of next.runs) {
    if (run.isRunning) continue
    const was = before.get(`${run.repo}/${run.branch}`)
    const isNewlyEnded = was ? was.runId !== run.runId || was.isRunning : Date.parse(run.createdAt) > prev.at
    if (!isNewlyEnded) continue
    const pr = inbox?.mine.find(p => p.repo === run.repo && p.branch === run.branch)
    const action = pr ? `${pr.url}/checks` : 'space:inbox'
    if (run.outcome === 'succeeded') out.push({ text: `🟢 CI passed: ${run.repo} ${run.branch}`, action })
    else if (run.outcome === 'failed') {
      const buttons: [string, string][] = pr ? [['Why?', `act:ci:${pr.url}`], ...(run.workflowId ? [['Rerun', `act:rerun:${pr.url}`] as [string, string]] : [])] : []
      out.push({ text: `🔴 CI failed: ${run.repo} ${run.branch}${run.failed ? ` · ${run.failed.job}` : ''}`, action, buttons })
    }
  }
  return out
}

// Tickets whose merged PRs all reached production since the previous fetch.
export function deployChanges(prev: Inbox, next: Inbox) {
  const out: { text: string; ticket: string }[] = []
  for (const [ticket, prs] of Object.entries(next.deploys ?? {})) {
    const wasLive = (prev.deploys?.[ticket] ?? []).length > 0 && (prev.deploys?.[ticket] ?? []).every(p => p.isLive)
    if (prs.length > 0 && prs.every(p => p.isLive) && !wasLive) {
      out.push({ text: `🚀 Live in production: ${ticket} (${prs.map(p => `${p.repo} ${p.tag}`).join(', ')})`, ticket })
    }
  }
  return out
}
