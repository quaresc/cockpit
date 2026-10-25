import { expect, test } from 'claude-code/testing'

import type { CiState, Inbox, Pr, Ticket } from '../types'

import { changes, ciChanges, deployChanges, hasTicket, isNewer, ticketKey, ticketsIn } from '../hooks/match'
import { checkRequest } from '../hooks/protocol'

const ticket = (id: string): Ticket => ({ id, title: id, url: `https://linear.app/t/issue/${id}`, status: 'In Progress', statusType: 'started' })

const pr = (over: Partial<Pr> = {}): Pr => ({
  repo: 'api', number: 1, title: 'A change', url: 'https://github.com/o/api/pull/1', author: 'me', updatedAt: new Date().toISOString(),
  isDraft: false, isDirect: false, review: null, ci: 'SUCCESS', branch: 'me/eng-4-login', ...over,
})

const inbox = (over: Partial<Inbox> = {}): Inbox => ({ at: 0, direct: [], team: [], botRepos: [], mine: [], tickets: [], errors: [], ...over })

test('a ticket matches its branch as a whole key only', async () => {
  expect(hasTicket('me/eng-4-login', 'ENG-4')).toBe(true)
  expect(hasTicket('eng-4', 'ENG-4')).toBe(true)
  expect(hasTicket('me/ENG-4_login', 'eng-4')).toBe(true)
  expect(hasTicket('me/eng-42-login', 'ENG-4')).toBe(false)
  expect(hasTicket('me/xeng-4-login', 'ENG-4')).toBe(false)
  expect(hasTicket(undefined, 'ENG-4')).toBe(false)
  expect(hasTicket('anything', '')).toBe(false)
})

test('a branch names a ticket only with a known team key', async () => {
  const data = inbox({ tickets: [ticket('ROI-2099')] })
  expect(ticketKey('cq/roi-2099-admin-unpublish', data, [])).toBe('ROI-2099')
  expect(ticketKey('cq/can-1315-eb-medias', data, ['CAN'])).toBe('CAN-1315')
  expect(ticketKey('cq/labs-week-3-march', data, ['CAN'])).toBeUndefined()
  expect(ticketKey('fix-404-page', data, [])).toBeUndefined()
  expect(ticketKey('release-2026', null, [])).toBeUndefined()
})

test('ticketsIn keeps the order and the known teams', async () => {
  expect(ticketsIn('see eng-4 then FOO-1 and ENG-12', null, ['ENG'])).toEqual(['ENG-4', 'ENG-12'])
})

test('isNewer compares versions part by part', async () => {
  expect(isNewer('0.12.0', '0.11.9')).toBe(true)
  expect(isNewer('0.11.0', '0.11.0')).toBe(false)
  expect(isNewer('0.11', '0.11.1')).toBe(false)
  expect(isNewer('1.0.0', '0.99.99')).toBe(true)
})

test('changes notify a new review request, a newly failing PR and a new ticket', async () => {
  const prev = inbox({ mine: [pr()], tickets: [ticket('ENG-1')] })
  const next = inbox({
    direct: [pr({ url: 'https://github.com/o/web/pull/9', repo: 'web', number: 9 })],
    mine: [pr({ ci: 'FAILURE' })],
    tickets: [ticket('ENG-1'), ticket('ENG-2')],
  })
  expect(changes(prev, next).map(n => n.text.split(':')[0])).toEqual(['👀 Review requested', '🔴 CI failing', '🎫 Assigned'])
  expect(changes(next, next)).toEqual([])
})

test('ciChanges announce a run once, when it ends', async () => {
  const run = { repo: 'api', branch: 'me/eng-4-login', runId: 'a', createdAt: new Date(1000).toISOString(), done: 3, total: 3, outcome: 'succeeded' }
  const prev: CiState = { at: 0, runs: [{ ...run, isRunning: true, outcome: 'running' }] }
  const next: CiState = { at: 1, runs: [{ ...run, isRunning: false }] }
  expect(ciChanges(prev, next, null).map(c => c.text)).toEqual(['🟢 CI passed: api me/eng-4-login'])
  expect(ciChanges(next, next, null)).toEqual([])
})

test('deployChanges fire once every PR of a ticket is live', async () => {
  const waiting = inbox({ deploys: { 'ENG-4': [{ repo: 'api', number: 1, isLive: false, tag: 'v1' }] } })
  const live = inbox({ deploys: { 'ENG-4': [{ repo: 'api', number: 1, isLive: true, tag: 'v2' }] } })
  expect(deployChanges(waiting, live).map(c => c.ticket)).toEqual(['ENG-4'])
  expect(deployChanges(live, live)).toEqual([])
})

test('a request is checked against its declared fields', async () => {
  expect('request' in checkRequest({ type: 'rerun', workflowId: '12', label: 'rerun api#1' })).toBe(true)
  expect('refusal' in checkRequest({ type: 'rerun', label: 'rerun api#1' })).toBe(true)
  expect('refusal' in checkRequest({ type: 'nuke', label: 'x' })).toBe(true)
  expect('refusal' in checkRequest({ type: 'extend', preview: 'p', days: '2', label: 'x' })).toBe(true)
})
