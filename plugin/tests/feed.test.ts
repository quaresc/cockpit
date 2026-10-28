import { expect, test } from 'claude-code/testing'

import type { CiState, Inbox } from '../types'

import { HUB_FRESH_MS, hubErrors, hubOwns, hubOwnsCi, modFetchedAt, modInbox } from '../hooks/feed'
import { changes, prChanges, ticketChanges } from '../hooks/match'

const inbox = (over: Partial<Inbox> = {}): Inbox => ({ at: 0, direct: [], team: [], botRepos: [], mine: [], tickets: [], errors: [], ...over })
const now = 10 * HUB_FRESH_MS

test('the hub owns a part only while it wrote it less than three minutes ago', async () => {
  const data = inbox({ hub: { at: now - 1000, github: now - 1000, errors: [] } })
  expect(hubOwns(data, 'github', now)).toBe(true)
  expect(hubOwns(data, 'tickets', now)).toBe(false)
  expect(hubOwns(data, 'github', now + HUB_FRESH_MS)).toBe(false)
  expect(hubOwns(null, 'github', now)).toBe(false)
  expect(hubOwns(inbox(), 'github', now)).toBe(false)
})

test('the hub errors show while the hub runs, and go with it', async () => {
  const data = inbox({ hub: { at: now - 1000, errors: ['GitHub: rate limited, next try at 10:42'] } })
  expect(hubErrors(data, now)).toEqual(['GitHub: rate limited, next try at 10:42'])
  expect(hubErrors(data, now + HUB_FRESH_MS)).toEqual([])
  expect(hubErrors(inbox(), now)).toEqual([])
})

test('the mod times its own fetches, not the hub writes', async () => {
  expect(modFetchedAt(inbox({ at: 5 }))).toBe(5)
  expect(modFetchedAt(inbox({ at: 9, modAt: 5, hub: { at: 9, errors: [] } }))).toBe(5)
  // An inbox only the hub has written was never fetched by the mod: its tickets are due at once.
  expect(modFetchedAt(inbox({ at: 9, hub: { at: 9, github: 9, errors: [] } }))).toBe(0)
})

test('ci.json is the hub one only while fresh and marked so', async () => {
  const ci: CiState = { at: now - 1000, runs: [], version: 3, by: 'hub' }
  expect(hubOwnsCi(ci, now)).toBe(true)
  expect(hubOwnsCi(ci, now + HUB_FRESH_MS)).toBe(false)
  expect(hubOwnsCi({ ...ci, by: undefined }, now)).toBe(false)
  expect(hubOwnsCi(null, now)).toBe(false)
})

test('PR and ticket news split, so each side announces only what it fetched', async () => {
  const ticket = { id: 'ENG-2', title: 'b', url: 'u', status: 'Todo', statusType: 'unstarted' }
  const prev = inbox({ tickets: [{ ...ticket, id: 'ENG-1' }] })
  const pr = { repo: 'web', number: 9, title: 'Dark mode', url: 'https://github.com/acme/web/pull/9', author: 'maya', updatedAt: '', isDraft: false, isDirect: true, review: null, ci: null }
  const next = inbox({ direct: [pr], tickets: [{ ...ticket, id: 'ENG-1' }, ticket] })
  expect(prChanges(prev, next).map(n => n.text)).toEqual(['👀 Review requested: web#9 Dark mode'])
  expect(ticketChanges(prev, next).map(n => n.text)).toEqual(['🎫 Assigned: ENG-2 b'])
  expect(changes(prev, next).length).toBe(2)
})

test('the mod writes what it fetched and keeps the hub part as the file has it', async () => {
  const pr = (n: number) => ({ repo: 'api', number: n, title: 't', url: `https://github.com/acme/api/pull/${n}`, author: 'me', updatedAt: '', isDraft: false, isDirect: false, review: null, ci: null })
  const ticket = { id: 'ENG-1', title: 'a', url: 'u', status: 'Todo', statusType: 'unstarted' }
  const hub = { at: now, github: now, errors: ['GitHub: HTTP 502'] }
  const file = inbox({ version: 1, mine: [pr(4)], botRepos: ['web'], tickets: [ticket], hub })
  const kept = modInbox(file, null, null, false, 5 * 60_000, now)
  expect([kept.mine, kept.botRepos, kept.tickets, kept.hub, kept.at, kept.modAt]).toEqual([[pr(4)], ['web'], [ticket], hub, now, now])
  const fetched = modInbox(file, { direct: [], team: [], botRepos: [], mine: [pr(5)], merged: [] }, [], true, 5 * 60_000, now)
  expect(fetched.mine).toEqual([pr(5)])
  expect(fetched.tickets).toEqual([])
  // A failed source makes the mod due again a minute later rather than five.
  expect(fetched.modAt).toBe(now - 4 * 60_000)
})
