import { expect, test } from 'claude-code/testing'

import { afterLook } from '../hooks/upkeep'

test('a look that reads no version keeps the one read last', async () => {
  expect(afterLook({ at: 1, latest: '0.41.0', notified: '0.41.0' }, undefined, 9)).toEqual({ at: 9, latest: '0.41.0', notified: '0.41.0' })
  expect(afterLook({ at: 1, latest: '0.41.0' }, '0.41.1', 9)).toEqual({ at: 9, latest: '0.41.1' })
  expect(afterLook({ at: 0 }, undefined, 9)).toEqual({ at: 9, latest: undefined })
})
