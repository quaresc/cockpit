import type { EngineInterface } from 'claude-code'

import type { Inbox, Meeting, MeetingsState } from '../types'

import { notify, withLock } from './ci'
import { cacheDir, home, loadConfig } from './config'

type $ = EngineInterface

const STALE_MS = 5 * 60_000
const NOTIFY_BEFORE_MS = 5 * 60_000

type GEvent = {
  id: string
  summary?: string
  status?: string
  eventType?: string
  transparency?: string
  htmlLink?: string
  conferenceUrl?: string
  hangoutLink?: string
  location?: string
  start?: { dateTime?: string; date?: string }
  end?: { dateTime?: string; date?: string }
  attendees?: { self?: boolean; responseStatus?: string; resource?: boolean; email?: string; displayName?: string }[]
  description?: string
  recurringEventId?: string
  attachments?: { title?: string; fileUrl?: string }[]
}

export async function readMeetings($: $): Promise<MeetingsState | null> {
  try {
    return JSON.parse(await $.fs.read(`${await cacheDir($)}/meetings.json`)) as MeetingsState
  } catch {
    return null
  }
}

// The next 7 days of timed events from Google Calendar, without the declined ones and the
// all-day or "free" markers (working days, office days).
async function fetchMeetings($: $): Promise<Meeting[]> {
  // Small pages: one large answer is cut by the engine's size limit before it reaches the mod.
  const events: GEvent[] = []
  let pageToken: string | undefined
  for (let page = 0; page < 8; page++) {
    const result = await $.mcp.call((await loadConfig($)).calendar.server, 'list_events', { orderBy: 'startTime', pageSize: 12, ...(pageToken ? { pageToken } : {}) })
    const block = result.content.find(c => c.type === 'text')
    const text = block && 'text' in block ? String(block.text) : ''
    if (result.isError || !text.startsWith('{')) throw new Error(text.slice(0, 160))
    const body = JSON.parse(text) as { events?: GEvent[]; nextPageToken?: string }
    events.push(...(body.events ?? []))
    pageToken = body.nextPageToken
    if (!pageToken) break
  }
  return events
    .filter(e => e.status !== 'cancelled' && e.start?.dateTime && e.end?.dateTime && e.transparency !== 'transparent')
    .filter(e => e.eventType === undefined || e.eventType === 'DEFAULT')
    .map(e => {
      const me = e.attendees?.find(a => a.self)
      return {
        id: e.id,
        title: (e.summary ?? '(no title)').trim(),
        start: e.start?.dateTime ?? '',
        end: e.end?.dateTime ?? '',
        url: e.htmlLink ?? '',
        join: e.conferenceUrl ?? e.hangoutLink ?? (e.location?.startsWith('http') ? e.location : undefined),
        response: me?.responseStatus ?? 'accepted',
        attendees: (e.attendees ?? []).filter(a => !a.resource).length,
        description: e.description?.slice(0, 2000),
        series: e.recurringEventId,
        people: (e.attendees ?? []).filter(a => !a.self && !a.resource && a.responseStatus !== 'declined').map(a => a.displayName ?? a.email ?? '?').slice(0, 15),
        attachments: (e.attachments ?? []).filter(a => a.fileUrl).map(a => ({ title: a.title ?? 'file', url: a.fileUrl ?? '' })),
      }
    })
    .filter(m => m.response !== 'declined')
}

// A series seen on three days or more of the week is a daily: it gets no brief.
function markBriefable(meetings: Meeting[], skip: string[]) {
  const days = new Map<string, Set<string>>()
  for (const m of meetings) if (m.series) days.set(m.series, (days.get(m.series) ?? new Set()).add(m.start.slice(0, 10)))
  for (const m of meetings) {
    const isDaily = m.series !== undefined && (days.get(m.series)?.size ?? 0) >= 3
    const isSkipped = skip.some(word => m.title.toLowerCase().includes(word.toLowerCase()))
    m.briefable = !isDaily && !isSkipped && (m.people?.length ?? 0) > 0 && m.response !== 'declined'
  }
  return meetings
}

// The first two lines of a meeting's brief, when it has one.
async function briefLines($: $, m: Meeting) {
  const name = m.id.replace(/[^A-Za-z0-9_-]/g, '_')
  const text = await $.fs.read(`${await cacheDir($)}/meeting-briefs/${name}.md`).catch(() => '')
  return text.split('\n').map(l => l.replace(/^[#>*\-\s]+/, '').trim()).filter(l => l.length > 0).slice(0, 2)
}

const OPEN_STEP = /^\|\s*\d+\s*\|[^|]*\|[^|]*\|\s*(⬜|⏳|🟡|❌)/

// What the meeting is about, from the tickets and PRs it names or whose project its title names: their state and runbook.
async function prepFor($: $, meeting: Meeting) {
  const inbox = JSON.parse(await $.fs.read(`${await cacheDir($)}/inbox.json`)) as Inbox
  const text = `${meeting.title}\n${meeting.description ?? ''}`
  const lower = text.toLowerCase()
  const tickets = inbox.tickets.filter(t =>
    lower.includes(t.id.toLowerCase()) || (t.project && t.project.length >= 4 && lower.includes(t.project.toLowerCase())),
  )
  const prs = inbox.mine.filter(pr => text.includes(pr.url) || text.includes(`${pr.repo}#${pr.number}`))
  const parts: string[] = []
  for (const t of tickets.filter(t => !t.parentId).slice(0, 2)) {
    const book = await $.fs.read(`${await home($)}/.config/cockpit/runbooks/${t.id.toLowerCase()}.md`).catch(() => '')
    const open = book.split('\n').filter(l => OPEN_STEP.test(l)).length
    parts.push(book ? `${t.id}: ${t.status}, runbook ${open} to do` : `${t.id}: ${t.status}`)
  }
  for (const pr of prs.slice(0, 2)) parts.push(`${pr.repo}#${pr.number}: ${pr.review === 'APPROVED' ? 'approved' : 'in review'}, CI ${(pr.ci ?? 'none').toLowerCase()}`)
  return parts.length > 0 ? `\n${parts.join('\n')}` : ''
}

let inflight: Promise<void> | undefined
let isCalendarMissing = false

// Refreshes the shared meetings cache (one session at a time), then warns once, 5 minutes ahead.
export function refreshMeetings($: $, isForced = false) {
  inflight ??= (async () => {
    try {
      const prev = await readMeetings($)
      await withLock($, 'meetings', async () => {
        let state: MeetingsState = prev ?? { at: 0, meetings: [], notified: [] }
        if (isForced || Date.now() - state.at > STALE_MS) {
          const isOff = isCalendarMissing || !(await loadConfig($)).calendar.enabled
          const meetings = isOff ? null : await fetchMeetings($).catch((err: Error) => {
            // A session started before the Google Calendar connector was added has no such server: leave it to the others.
            if (err.message.includes('no connected MCP tool')) isCalendarMissing = true
            else state = { ...state, error: `Calendar: ${err.message.replace(/^cockpit: /, '')}` }
            return null
          })
          if (meetings) state = { ...state, at: Date.now(), meetings: markBriefable(meetings, (await loadConfig($)).meetingBrief.skip), error: undefined }
        }
        const soon = state.meetings.filter(m => {
          const until = Date.parse(m.start) - Date.now()
          return until > 0 && until <= NOTIFY_BEFORE_MS && !state.notified.includes(m.id)
        })
        for (const m of soon) {
          // With a brief, the notification carries its opening lines and opens it; without, the call itself.
          const brief = await briefLines($, m)
          const prep = brief.length > 0 ? `\n${brief.join('\n')}` : await prepFor($, m).catch(() => '')
          const join: [string, string][] = brief.length > 0 && m.join ? [['Join', m.join]] : []
          await notify($, '📅 Meeting', `${m.title} in ${Math.round((Date.parse(m.start) - Date.now()) / 60_000)} min${prep}`, brief.length > 0 ? `meeting:${m.id}` : (m.join ?? m.url), join)
        }
        const live = new Set(state.meetings.map(m => m.id))
        state.notified = [...state.notified.filter(id => live.has(id)), ...soon.map(m => m.id)]
        await $.fs.write(`${await cacheDir($)}/meetings.json`, JSON.stringify(state))
      })
    } finally {
      inflight = undefined
    }
  })()
  return inflight
}
