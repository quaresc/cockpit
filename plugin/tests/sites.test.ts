import { expect, test } from 'claude-code/testing'

import { chipsFor, citedIn } from '../hooks/sites'

// The kinds this table shares with the Rust core (herdr/tui/core/src/sites.rs) are checked by scripts/check-skills.sh.

test('the footer takes the last GitHub PR cited and does not draw it twice', () => {
  const text = 'opened https://github.com/acme/api/pull/2198, see https://github.com/acme/api/pull/2201 and https://acme.slack.com/archives/C1/p1.'
  const cited = citedIn([text], 'last')
  expect(cited.pr).toBe('https://github.com/acme/api/pull/2201')
  expect(citedIn([text], 'first').pr).toBe('https://github.com/acme/api/pull/2198')
  expect(chipsFor(cited).map(c => c.kind)).toEqual(['slack'])
  const [issue] = chipsFor({ issue: 'https://github.com/acme/api/issues/42', linear: 'https://linear.app/acme/issue/ENG-7/x' })
  expect(chipsFor({ linear: 'https://linear.app/acme/issue/ENG-7/x' })).toEqual([])
  expect([issue?.kind, issue?.label, issue?.detail, issue?.url]).toEqual(['issue', 'GitHub', '#42', 'https://github.com/acme/api/issues/42'])
})
