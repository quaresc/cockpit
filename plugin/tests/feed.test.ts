import { expect, test } from 'claude-code/testing'

import type { CiState, Inbox } from '../types'

import { HUB_FRESH_MS, hubOwns, hubOwnsCi, modFetchedAt, modPart } from '../hooks/feed'
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

test('the mod sends only what it fetched, and its own fetch time', async () => {
  const pr = { repo: 'api', number: 5, title: 't', url: 'https://github.com/acme/api/pull/5', author: 'me', updatedAt: '', isDraft: false, isDirect: false, review: null, ci: null }
  expect(modPart(null, null, false, 5 * 60_000, now)).toEqual({ at: now, modAt: now })
  const fetched = modPart({ direct: [], team: [], botRepos: [], mine: [pr], merged: [] }, [], true, 5 * 60_000, now)
  expect([fetched.mine, fetched.tickets, fetched.direct]).toEqual([[pr], [], []])
  // A failed source makes the mod due again a minute later rather than five.
  expect(fetched.modAt).toBe(now - 4 * 60_000)
})
