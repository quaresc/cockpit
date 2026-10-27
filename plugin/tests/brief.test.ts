import { expect, test } from 'claude-code/testing'

import type { Inbox, Pr } from '../types'

import { compose } from '../hooks/brief'

const pr = (number: number, over: Partial<Pr> = {}): Pr => ({
  repo: 'api', number, title: 'A change', url: `https://github.com/o/api/pull/${number}`, author: 'me', updatedAt: new Date().toISOString(),
  isDraft: false, isDirect: false, review: 'APPROVED', ci: 'SUCCESS', ...over,
})

const inbox = (over: Partial<Inbox> = {}): Inbox => ({ at: 0, direct: [], team: [], botRepos: [], mine: [], tickets: [], errors: [], ...over })

const lineFor = (data: Inbox, icon: string, org = '') => compose(data, null, null, null, '2026-10-26', org).lines.find(l => l.icon === icon)

const query = (url = '') => new URL(url).searchParams.get('q')

test('a brief line about one PR opens that PR', async () => {
  expect(lineFor(inbox({ mine: [pr(1)] }), 'approved')?.url).toBe('https://github.com/o/api/pull/1')
  expect(lineFor(inbox({ mine: [pr(1, { ci: 'FAILURE' })] }), 'fail')?.url).toBe('https://github.com/o/api/pull/1/checks')
})

test('a brief line about several PRs opens the search that lists them', async () => {
  const ready = lineFor(inbox({ mine: [pr(1), pr(2), pr(3)] }), 'approved', 'o')
  expect(ready?.url?.startsWith('https://github.com/pulls?q=')).toBe(true)
  expect(query(ready?.url)).toBe('is:pr is:open archived:false author:@me org:o review:approved status:success')

  const failing = lineFor(inbox({ mine: [pr(1, { ci: 'FAILURE' }), pr(2, { ci: 'ERROR' })] }), 'fail')
  expect(query(failing?.url)).toBe('is:pr is:open archived:false author:@me status:failure')

  const changes = lineFor(inbox({ mine: [pr(1, { review: 'CHANGES_REQUESTED' }), pr(2, { review: 'CHANGES_REQUESTED' })] }), 'changes')
  expect(query(changes?.url)).toBe('is:pr is:open archived:false author:@me review:changes-requested')

  const reviews = lineFor(inbox({ direct: [pr(1), pr(2)] }), 'review', 'o')
  expect(query(reviews?.url)).toBe('is:pr is:open archived:false user-review-requested:@me')
})
