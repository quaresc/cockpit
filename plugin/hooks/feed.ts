import type { CiState, Inbox, MergedPr, Pr, Ticket } from '../types'

// The hub fetches GitHub on its own (and Linear or CircleCI given their tokens) and keeps those fields of the inbox, and
// ci.json. While its part is fresh the mod leaves it alone and fetches only the rest.

export const HUB_FRESH_MS = 3 * 60_000

export type HubSource = 'github' | 'tickets'

export const hubOwns = (inbox: Inbox | null, source: HubSource, now = Date.now()) => now - (inbox?.hub?.[source] ?? 0) < HUB_FRESH_MS

// When the mod itself last fetched: never, for an inbox only the hub has written.
export const modFetchedAt = (inbox: Inbox) => inbox.modAt ?? (inbox.hub ? 0 : inbox.at)

export const hubOwnsCi = (ci: CiState | null, now = Date.now()) => ci?.by === 'hub' && now - ci.at < HUB_FRESH_MS

export type GithubPart = { direct: Pr[]; team: Pr[]; botRepos: string[]; mine: Pr[]; merged: MergedPr[] }

// The mod's part of the inbox: what it fetched, with its own fetch time; the hub keeps the rest and adds its errors after
// the mod's. A failed source makes that time due again within a minute.
export function modPart(gh: GithubPart | null, tickets: Ticket[] | null, isRetry: boolean, staleMs: number, now = Date.now()): Partial<Inbox> {
  return { at: now, modAt: isRetry ? now - staleMs + 60_000 : now, ...(gh ?? {}), ...(tickets ? { tickets } : {}) }
}
