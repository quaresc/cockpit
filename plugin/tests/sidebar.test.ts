import { expect, test } from 'claude-code/testing'

import { fitWork, fitWords } from '../hooks/sidebar'

const cells = (text: string) => [...text].length

test('the work row keeps the CI progress, tightening and shortening the repo first', async () => {
  const roomy = fitWork('api', 'ENG-4', '● 3/12', 32)
  expect(roomy.endsWith('api · ENG-4  ● 3/12')).toBe(true)
  const tight = fitWork('sourcing-front', 'SOU-2940', '● 10/12', 32)
  expect(tight.endsWith('s-front · SOU-2940 ● 10/12')).toBe(true)
  expect(cells(tight) <= 32).toBe(true)
  expect(fitWork('welcome-accounts-front', 'ROI-2099', '● 10/12', 20).endsWith(' ROI-2099 ● 10/12')).toBe(true)
})

test('a topic drops whole words to fit', async () => {
  expect(fitWords('sou 2909 runbook step 11', 24)).toBe('sou 2909 runbook step 11')
  expect(fitWords('sou 2909 runbook step 11', 22)).toBe('sou 2909 runbook step')
  expect(fitWords('refactoring', 5)).toBe('refactoring')
})
