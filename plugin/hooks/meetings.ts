import type { EngineInterface } from 'claude-code'

import type { Inbox, Meeting, MeetingsState } from '../types'

import { notify, withLock } from './ci'
import { cacheDir, home, loadConfig } from './config'
import { hasTicket } from './match'
import { callMapped, listAt, pick, text } from './platform'

type $ = EngineInterface

const STALE_MS = 5 * 60_000
const NOTIFY_BEFORE_MS = 5 * 60_000
// A claude.ai connector connects a moment after the session starts: try it again soon rather than never.
const MISSING_RETRY_MS = 30_000

export async function readMeetings($: $): Promise<MeetingsState | null> {
  try {
    return JSON.parse(await $.fs.read(`${await cacheDir($)}/meetings.json`)) as MeetingsState
  } catch {
    return null
  }
}

export type CalendarCapability = 'listEvents' | 'getEvent'

// The coming timed events of my calendar, without the declined ones and those the mapping leaves out (cancelled, free,
// out of office); an all-day marker has no start time and drops out too.
async function fetchMeetings($: $): Promise<Meeting[]> {
  const calendar = (await loadConfig($)).calendar
  const f = calendar.fields
  // Small pages: one large answer is cut by the engine's size limit before it reaches the mod.
  const events: unknown[] = []
  let pageToken: string | undefined
  for (let page = 0; page < 8; page++) {
    const body = await callMapped($, calendar, 'listEvents', { pageToken })
    events.push(...listAt(body, f.events))
    pageToken = text(pick(body, f.nextPage))
    if (!pageToken) break
  }
  const isSkipped = (e: unknown) =>
    Object.entries(calendar.skip).some(([path, values]) => values.includes(text(pick(e, path)) ?? '')) ||
    Object.entries(calendar.only).some(([path, values]) => !values.includes(text(pick(e, path)) ?? ''))
  const response = (value: unknown) => {
    const raw = text(value) ?? 'accepted'
    return calendar.responses[raw] ?? raw
  }
  return events
    .filter(e => text(pick(e, f.start)) && text(pick(e, f.end)) && !isSkipped(e))
    .map((e): Meeting => {
      const attendees = listAt(e, f.attendees)
      const me = attendees.find(a => pick(a, f.attendeeSelf) === true)
      const others = attendees.filter(a => pick(a, f.attendeeResource) !== true)
      const location = text(pick(e, f.location))
      return {
        id: text(pick(e, f.id)) ?? '',
        title: (text(pick(e, f.title)) ?? '(no title)').trim(),
        start: text(pick(e, f.start)) ?? '',
        end: text(pick(e, f.end)) ?? '',
        url: text(pick(e, f.url)) ?? '',
        join: text(pick(e, f.join)) ?? (location?.startsWith('http') ? location : undefined),
        response: response(pick(e, f.myResponse) ?? pick(me, f.attendeeResponse)),
        attendees: others.length,
        description: text(pick(e, f.description))?.slice(0, 2000),
        series: text(pick(e, f.series)),
        people: others
          .filter(a => a !== me && response(pick(a, f.attendeeResponse)) !== 'declined')
          .map(a => text(pick(a, f.attendeeName)) ?? '?')
          .slice(0, 15),
        attachments: listAt(e, f.attachments)
          .map(a => ({ title: text(pick(a, f.attachmentTitle)) ?? 'file', url: text(pick(a, f.attachmentUrl)) ?? '' }))
          .filter(a => a.url),
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
    hasTicket(lower, t.id) || (t.project && t.project.length >= 4 && lower.includes(t.project.toLowerCase())),
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
let calendarMissingAt = 0

// Refreshes the shared meetings cache (one session at a time), then warns once, 5 minutes ahead.
export function refreshMeetings($: $, isForced = false) {
  inflight ??= (async () => {
    try {
      const prev = await readMeetings($)
      await withLock($, 'meetings', async () => {
        let state: MeetingsState = prev ?? { at: 0, meetings: [], notified: [] }
        if (isForced || Date.now() - state.at > STALE_MS) {
          const isOff = Date.now() - calendarMissingAt < MISSING_RETRY_MS || !(await loadConfig($)).calendar.enabled
          const meetings = isOff ? null : await fetchMeetings($).catch((err: Error) => {
            // The connector may not be connected yet (at startup, or added later): try again shortly.
            if (err.message.includes('no connected MCP tool')) calendarMissingAt = Date.now()
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
