import type { EngineInterface } from 'claude-code'

import type { CiState, Inbox, MeetingsState } from '../types'

import { notify, withLock } from './ci'
import { cacheDir, loadConfig } from './config'

type $ = EngineInterface

// One line of the brief; the inbox app shows them under its header, `url` making the line clickable.
type BriefLine = { icon: string; text: string; url?: string; tone?: 'warn' | 'bad' | 'good' }

export type Brief = {
  date: string
  at: number
  lines: BriefLine[]
  dismissed?: boolean
  // What the next brief compares against: each ticket's status, and the tickets live in production.
  tickets: Record<string, string>
  live: string[]
}

async function readJson<T>($: $, path: string): Promise<T | null> {
  try {
    return JSON.parse(await $.fs.read(path)) as T
  } catch {
    return null
  }
}

const localDate = (d: Date) => `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`

const hhmm = (iso: string) => {
  const d = new Date(iso)
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`
}

const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? '' : 's'}`

function compose(inbox: Inbox, ci: CiState | null, meetings: MeetingsState | null, previous: Brief | null, today: string) {
  const lines: BriefLine[] = []
  const now = Date.now()

  const todays = (meetings?.meetings ?? [])
    .filter(m => localDate(new Date(m.start)) === today && Date.parse(m.end) > now)
    .sort((a, b) => Date.parse(a.start) - Date.parse(b.start))
  const first = todays[0]
  if (first) {
    lines.push({ icon: 'calendar', text: `${plural(todays.length, 'meeting')}, first at ${hhmm(first.start)}: ${first.title}`, url: first.join ?? first.url })
  } else if (meetings) {
    lines.push({ icon: 'calendar', text: 'no meetings today', tone: 'good' })
  }

  const fresh = (iso: string) => now - Date.parse(iso) < 60 * 86_400_000
  const direct = inbox.direct.filter(pr => fresh(pr.updatedAt))
  if (direct.length > 0) {
    const names = direct.slice(0, 2).map(pr => `${pr.repo}#${pr.number}`).join(', ')
    lines.push({ icon: 'review', text: `${plural(direct.length, 'review')} for you: ${names}${direct.length > 2 ? '…' : ''}`, url: direct[0]?.url, tone: 'warn' })
  }

  const failing = inbox.mine.filter(pr => pr.ci === 'FAILURE' || pr.ci === 'ERROR')
  if (failing.length > 0) {
    lines.push({ icon: 'fail', text: `${plural(failing.length, 'PR')} of yours failing CI: ${failing.map(pr => `${pr.repo}#${pr.number}`).join(', ')}`, url: failing[0] && `${failing[0].url}/checks`, tone: 'bad' })
  }
  const changes = inbox.mine.filter(pr => pr.review === 'CHANGES_REQUESTED')
  if (changes.length > 0) {
    lines.push({ icon: 'changes', text: `changes requested on ${changes.map(pr => `${pr.repo}#${pr.number}`).join(', ')}`, url: changes[0]?.url, tone: 'warn' })
  }
  const ready = inbox.mine.filter(pr => pr.review === 'APPROVED' && pr.ci === 'SUCCESS')
  if (ready.length > 0) {
    lines.push({ icon: 'approved', text: `ready to merge: ${ready.map(pr => `${pr.repo}#${pr.number}`).join(', ')}`, url: ready[0]?.url, tone: 'good' })
  }

  // CI that ended in failure since the previous brief, or in the last 16 hours.
  const since = previous?.at ?? now - 16 * 3600_000
  const overnight = (ci?.runs ?? []).filter(r => !r.isRunning && r.outcome === 'failed' && Date.parse(r.createdAt) > since)
  for (const run of overnight.slice(0, 3)) {
    const pr = inbox.mine.find(p => p.repo === run.repo && p.branch === run.branch)
    lines.push({ icon: 'circleci', text: `CI failed overnight on ${run.repo} ${run.branch}${run.failed ? ` · ${run.failed.job}` : ''}`, url: pr && `${pr.url}/checks`, tone: 'bad' })
  }

  // Tickets that moved, appeared or went live since the previous brief.
  const before = previous?.tickets ?? {}
  if (previous) {
    for (const t of inbox.tickets) {
      const was = before[t.id]
      if (was === undefined) lines.push({ icon: 'linear', text: `${t.id} assigned: ${t.title}`, url: t.url })
      else if (was !== t.status) lines.push({ icon: 'linear', text: `${t.id} moved ${was} → ${t.status}`, url: t.url })
    }
  }
  const live = Object.entries(inbox.deploys ?? {})
    .filter(([, prs]) => prs.length > 0 && prs.every(p => p.isLive))
    .map(([id]) => id)
  for (const id of live.filter(id => previous && !previous.live.includes(id))) {
    lines.push({ icon: 'rocket', text: `${id} is live in production`, url: inbox.tickets.find(t => t.id === id)?.url, tone: 'good' })
  }

  return { lines, tickets: Object.fromEntries(inbox.tickets.map(t => [t.id, t.status])), live }
}

// Once a day, from the configured time on (weekdays unless weekends are on), when the inbox and the meetings are fresh:
// the brief is written for the inbox app, and one notification sums it up.
export async function maybeBrief($: $) {
  const { brief, calendar } = await loadConfig($)
  if (!brief.enabled) return
  const now = new Date()
  const today = localDate(now)
  const [h, m] = brief.at.split(':').map(Number)
  const start = new Date(now)
  start.setHours(h ?? 8, m ?? 30, 0, 0)
  const isWeekend = now.getDay() === 0 || now.getDay() === 6
  if (now < start || (isWeekend && !brief.weekends)) return

  const dir = await cacheDir($)
  const previous = await readJson<Brief>($, `${dir}/brief.json`)
  if (previous?.date === today) return
  const inbox = await readJson<Inbox>($, `${dir}/inbox.json`)
  const meetings = calendar.enabled ? await readJson<MeetingsState>($, `${dir}/meetings.json`) : null
  if (!inbox || inbox.at < start.getTime() || (calendar.enabled && (meetings?.at ?? 0) < start.getTime())) return

  await withLock($, 'brief', async () => {
    const again = await readJson<Brief>($, `${dir}/brief.json`)
    if (again?.date === today) return
    const ci = await readJson<CiState>($, `${dir}/ci.json`)
    const { lines, tickets, live } = compose(inbox, ci, meetings, previous, today)
    const next: Brief = { date: today, at: Date.now(), lines, tickets, live }
    await $.fs.write(`${dir}/brief.json`, JSON.stringify(next))
    const body = lines.length > 0 ? lines.slice(0, 4).map(l => l.text).join('\n') : 'Nothing waiting on you.'
    await notify($, '☀️ Morning brief', body, 'space:inbox')
  })
}
