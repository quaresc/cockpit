import { expect, test } from 'claude-code/testing'

import { changesArgs, find, searchArgs } from '../hooks/find'

const ran = (exitCode: number, stdout = '', stderr = '') => ({ exitCode, stdout, stderr, isStdoutTruncated: false, isStderrTruncated: false })

test('a search needs words, trimmed', async () => {
  expect(searchArgs({ query: '  ENG-401 ' })).toEqual(['search', 'ENG-401'])
  expect(searchArgs({ query: '   ' })).toContain('query:')
  expect(searchArgs({})).toContain('query:')
})

test('the changes take a duration or epoch milliseconds, and inbox or monitor', async () => {
  expect(changesArgs({})).toEqual(['events'])
  expect(changesArgs({ since: '30m', app: 'monitor' })).toEqual(['events', '--since', '30m', '--app', 'monitor'])
  expect(changesArgs({ since: 1761660000000 })).toEqual(['events', '--since', '1761660000000'])
  expect(changesArgs({ since: 'this morning' })).toContain('since:')
  expect(changesArgs({ since: '12' })).toContain('since:')
  expect(changesArgs({ app: 'todo' })).toContain('app:')
})

test('the CLI runs through cockpit.sh and its JSON comes back as is', async () => {
  const runs: string[][] = []
  const fake = (answer: ReturnType<typeof ran>) =>
    ({
      env: { get: async (name: string) => (name === 'HOME' ? '/home/me' : undefined) },
      process: {
        run: async (argv: string[]) => {
          runs.push(argv)
          return answer
        },
      },
    }) as unknown as Parameters<typeof find>[0]

  expect(await find(fake(ran(0, '{"entries":[]}\n')), { query: 'quota' }, searchArgs)).toEqual({ result: '{"entries":[]}' })
  expect(runs[0]).toEqual(['sh', '/home/me/.config/cockpit/herdr/cockpit.sh', 'search', 'quota'])
  expect(await find(fake(ran(1, '', 'cockpit events: the cockpit hub did not start')), { since: '2h' }, changesArgs)).toEqual({
    deny: 'cockpit events: the cockpit hub did not start',
  })
  expect(await find(fake(ran(0)), {}, changesArgs)).toEqual({ deny: 'cockpit events: no cockpit binary; run /cockpit:doctor' })
  expect(await find(fake(ran(0)), { app: 'todo' }, changesArgs)).toEqual({ deny: 'app: one of inbox, monitor, got "todo"' })
  expect(runs.length).toBe(3)
})
