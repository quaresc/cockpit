import type { EngineInterface } from 'claude-code'

import type { Meeting, MeetingsState } from '../types'

import { notify, withLock } from './ci'
import { cacheDir, loadConfig } from './config'

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
  attendees?: { self?: boolean; responseStatus?: string; resource?: boolean }[]
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
      }
    })
    .filter(m => m.response !== 'declined')
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
          if (meetings) state = { ...state, at: Date.now(), meetings, error: undefined }
        }
        const soon = state.meetings.filter(m => {
          const until = Date.parse(m.start) - Date.now()
          return until > 0 && until <= NOTIFY_BEFORE_MS && !state.notified.includes(m.id)
        })
        for (const m of soon) await notify($, '📅 Meeting', `${m.title} in ${Math.round((Date.parse(m.start) - Date.now()) / 60_000)} min`)
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

// The meeting to show in a session's footer: the one running now, else the next one today or later.
export function nextMeeting(state: MeetingsState | null) {
  const now = Date.now()
  return (state?.meetings ?? [])
    .filter(m => Date.parse(m.end) > now)
    .sort((a, b) => Date.parse(a.start) - Date.parse(b.start))[0]
}
