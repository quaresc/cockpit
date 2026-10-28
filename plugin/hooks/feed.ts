import type { CiState, Inbox, MergedPr, Pr, Ticket } from '../types'

// The hub fetches GitHub on its own (and Linear or CircleCI given their tokens) and writes those fields of inbox.json,
// and ci.json, under the same locks. While its part is fresh the mod leaves it alone and fetches only the rest.

export const HUB_FRESH_MS = 3 * 60_000

export type HubSource = 'github' | 'tickets'

export const hubOwns = (inbox: Inbox | null, source: HubSource, now = Date.now()) => now - (inbox?.hub?.[source] ?? 0) < HUB_FRESH_MS

// The hub's error lines, while it is still running.
export const hubErrors = (inbox: Inbox | null, now = Date.now()) => (inbox?.hub && now - inbox.hub.at < HUB_FRESH_MS ? inbox.hub.errors : [])

// When the mod itself last fetched: never, for an inbox only the hub has written.
export const modFetchedAt = (inbox: Inbox) => inbox.modAt ?? (inbox.hub ? 0 : inbox.at)

export const hubOwnsCi = (ci: CiState | null, now = Date.now()) => ci?.by === 'hub' && now - ci.at < HUB_FRESH_MS

export type GithubPart = { direct: Pr[]; team: Pr[]; botRepos: string[]; mine: Pr[]; merged: MergedPr[] }

// The inbox the mod writes: what it fetched, else what the file holds (the hub's part among it), with its own fetch time;
// a failed source makes that time due again within a minute.
export function modInbox(kept: Inbox | null, gh: GithubPart | null, tickets: Ticket[] | null, isRetry: boolean, staleMs: number, now = Date.now()): Inbox {
  return {
    version: kept?.version ?? 1,
    at: now,
    modAt: isRetry ? now - staleMs + 60_000 : now,
    direct: gh?.direct ?? kept?.direct ?? [],
    team: gh?.team ?? kept?.team ?? [],
    botRepos: gh?.botRepos ?? kept?.botRepos ?? [],
    mine: gh?.mine ?? kept?.mine ?? [],
    tickets: tickets ?? kept?.tickets ?? [],
    errors: [],
    merged: gh?.merged ?? kept?.merged ?? [],
    hub: kept?.hub,
  }
}
