import { expect, test } from 'claude-code/testing'

import { NO_TICKET, booked } from '../hooks/costs'

test('spend on a ticket goes to it, and spend on none is booked too so a ticket never takes the whole session', async () => {
  const book = booked(booked({}, 1.5, 'ENG-412'), 0.5, null)
  expect(book).toEqual({ 'ENG-412': 1.5, [NO_TICKET]: 0.5 })
  expect(booked(book, 0.25, 'ENG-412')).toEqual({ 'ENG-412': 1.75, [NO_TICKET]: 0.5 })
  expect(booked({}, 1, '')).toEqual({ [NO_TICKET]: 1 })
})
