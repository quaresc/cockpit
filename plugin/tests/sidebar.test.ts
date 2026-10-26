import { expect, test } from 'claude-code/testing'

import { fitWork, fitWords } from '../hooks/sidebar'

const cells = (text: string) => [...text].length

test('the work row keeps the CI progress, tightening and shortening the repo first', async () => {
  const roomy = fitWork('api', 'ENG-4', '● 3/12', 32)
  expect(roomy.endsWith('api · ENG-4  ● 3/12')).toBe(true)
  const tight = fitWork('checkout-front', 'ENG-4940', '● 10/12', 32)
  expect(tight.endsWith('c-front · ENG-4940 ● 10/12')).toBe(true)
  expect(cells(tight) <= 32).toBe(true)
  expect(fitWork('accounts-console-front', 'ENG-4099', '● 10/12', 20).endsWith(' ENG-4099 ● 10/12')).toBe(true)
})

test('a topic drops whole words to fit', async () => {
  expect(fitWords('ops 4120 runbook step 11', 24)).toBe('ops 4120 runbook step 11')
  expect(fitWords('ops 4120 runbook step 11', 22)).toBe('ops 4120 runbook step')
  expect(fitWords('refactoring', 5)).toBe('refactoring')
})
