import { expect, test } from 'claude-code/testing'

import { capture, fillArgs } from '../hooks/platform'
import { composedId } from '../hooks/tickets'

// The GitHub Issues connector: an issue is owner, repo and number, which the mapping carries through cockpit's one id.
const GITHUB = {
  enabled: true,
  name: 'GitHub Issues',
  server: 'github',
  keys: [],
  urlTemplate: '',
  idPattern: 'github.com/{owner}/{repo}/issues/{number}',
  idTemplate: '{owner}/{repo}#{number}',
  tools: {},
  fields: {},
  statusTypes: {},
}

test('a GitHub issue reads as owner/repo#number and gives its parts back', () => {
  expect(composedId(GITHUB, 'https://github.com/acme/api/issues/22')).toBe('acme/api#22')
  expect(composedId(GITHUB, 'https://gitlab.com/acme/api/-/issues/22')).toBeUndefined()
  const parts = capture(GITHUB.idTemplate, 'acme/api#22')
  expect(parts).toEqual({ owner: 'acme', repo: 'api', number: 22 })
  expect(fillArgs({ owner: '{owner}', repo: '{repo}', issue_number: '{number}' }, parts!)).toEqual({ owner: 'acme', repo: 'api', issue_number: 22 })
})

test('a template whose literal pieces are missing reads nothing', () => {
  expect(capture('{owner}/{repo}#{number}', 'ENG-1')).toBeUndefined()
  expect(capture('{owner}/{repo}#{number}', 'acme/api#')).toBeUndefined()
})
