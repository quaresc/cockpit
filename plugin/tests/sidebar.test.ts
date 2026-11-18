import { expect, test } from 'claude-code/testing'

import { writeAtomic } from '../hooks/config'
import type { Writer } from '../hooks/config'
import { fitWork, fitWords } from '../hooks/sidebar'

const cells = (text: string) => [...text].length

test('the work row keeps the CI progress, tightening and shortening the repo first', async () => {
  const roomy = fitWork('api', 'ENG-4', '● 3/12', 32) ?? ''
  expect(roomy.endsWith('api · ENG-4  ● 3/12')).toBe(true)
  const tight = fitWork('checkout-front', 'ENG-4940', '● 10/12', 32) ?? ''
  expect(tight.endsWith('c-front · ENG-4940 ● 10/12')).toBe(true)
  expect(cells(tight) <= 32).toBe(true)
  expect(fitWork('accounts-console-front', 'ENG-4099', '● 10/12', 20)?.endsWith(' ENG-4099 ● 10/12')).toBe(true)
})

test('a topic drops whole words to fit', async () => {
  expect(fitWords('ops 4120 runbook step 11', 24)).toBe('ops 4120 runbook step 11')
  expect(fitWords('ops 4120 runbook step 11', 22)).toBe('ops 4120 runbook step')
  expect(fitWords('refactoring', 5)).toBe('refactoring')
})

const ran = (exitCode: number) => ({ exitCode, stdout: '', stderr: '', isStdoutTruncated: false, isStderrTruncated: false })

// The engine's file and process calls as a list of steps, `mv` answering `moved`.
const writer = (steps: string[], moved: number): Writer => ({
  fs: { write: async (path: string) => void steps.push(`write ${path}`) },
  process: {
    run: async (argv: readonly string[]) => {
      steps.push(argv.join(' '))
      return ran(argv[0] === 'mv' ? moved : 0)
    },
  },
})

test('limits.json is written whole: under its own name first, then moved over the old one', async () => {
  const steps: string[] = []
  await writeAtomic(writer(steps, 0), '/home/me/.cache/cockpit/limits.json', '{"at":1,"limits":[]}')
  const tmp = steps[0]?.replace('write ', '') ?? ''
  expect(tmp.startsWith('/home/me/.cache/cockpit/limits.json.')).toBe(true)
  expect(tmp.endsWith('.tmp')).toBe(true)
  expect(steps).toEqual([`write ${tmp}`, `mv -f ${tmp} /home/me/.cache/cockpit/limits.json`])
})

test('a move that fails leaves the last good limits.json and no stray file', async () => {
  const steps: string[] = []
  const failed = await writeAtomic(writer(steps, 1), '/home/me/.cache/cockpit/limits.json', '{}').then(() => false, () => true)
  expect(failed).toBe(true)
  expect(steps.includes('write /home/me/.cache/cockpit/limits.json')).toBe(false)
  expect(steps[2]?.startsWith('rm -f /home/me/.cache/cockpit/limits.json.')).toBe(true)
})
