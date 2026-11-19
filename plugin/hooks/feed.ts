import type { CiState, Inbox, MeetingsState, MergedPr, Pr, Ticket } from '../types'

// The hub fetches GitHub on its own (and the sources it has a server or a token for) and keeps those fields of the inbox, and
// ci.json. While its part is fresh the mod leaves it alone and fetches only the rest.

export const HUB_FRESH_MS = 3 * 60_000

export type HubSource = 'github' | 'tickets' | 'platform'

export const hubOwns = (inbox: Inbox | null, source: HubSource, now = Date.now()) => now - (inbox?.hub?.[source] ?? 0) < HUB_FRESH_MS

// When the mod itself last fetched: never, for an inbox only the hub has written.
export const modFetchedAt = (inbox: Inbox) => inbox.modAt ?? (inbox.hub ? 0 : inbox.at)

// The monitor space's document, when the hub reaches the platform and Datadog itself.
export const hubOwnsMonitor = (monitor: { by?: string; at: number } | null, now = Date.now()) => monitor?.by === 'hub' && now - monitor.at < HUB_FRESH_MS

export const hubOwnsCi = (ci: CiState | null, now = Date.now()) => ci?.by === 'hub' && now - ci.at < HUB_FRESH_MS

// Given calendar.ics the hub reads the calendar every five minutes: meetings.json stays its own until a quarter of an hour
// without a good read.
export const HUB_MEETINGS_FRESH_MS = 15 * 60_000

export const hubOwnsMeetings = (meetings: MeetingsState | null, now = Date.now()) =>
  meetings?.by === 'hub' && now - meetings.at < HUB_MEETINGS_FRESH_MS

export type GithubPart = { direct: Pr[]; team: Pr[]; botRepos: string[]; mine: Pr[]; merged: MergedPr[] }

// The mod's part of the inbox: what it fetched, with when each source came in (`modFetched`, only the ones that did) and
// its own fetch time, which a failed source makes due again within a minute. The hub keeps the rest and adds its errors.
export function modPart(
  gh: GithubPart | null,
  tickets: Ticket[] | null,
  isRetry: boolean,
  staleMs: number,
  now = Date.now(),
  before: Inbox['modFetched'] = {},
): Partial<Inbox> {
  const modFetched = { ...before, ...(gh ? { github: now } : {}), ...(tickets ? { tickets: now } : {}) }
  return { at: now, modAt: isRetry ? now - staleMs + 60_000 : now, modFetched, ...(gh ?? {}), ...(tickets ? { tickets } : {}) }
}
