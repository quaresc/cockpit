import type { EngineInterface } from 'claude-code'

import { cacheDir, writeAtomic } from './config'

type $ = EngineInterface

// How a session's spend splits over tickets: each stretch goes to the ticket it works on then, or to NO_TICKET, in
// ~/.cache/cockpit/costs/<session>.json, which only that session writes; the hub prices the session and splits it so.

/** The key spend made on no ticket goes under: it weighs in the split, and is never shown as a ticket. */
export const NO_TICKET = '(no ticket)'

/** The book with `spent` added to the ticket worked on, else to NO_TICKET. */
export function booked(book: Record<string, number>, spent: number, ticket: string | null): Record<string, number> {
  const key = ticket || NO_TICKET
  return { ...book, [key]: (book[key] ?? 0) + spent }
}

let base: number | undefined

// The figure the session starts from: what it cost before (a resumed session) was counted then, or by no ticket.
export async function startCosts($: $) {
  base = (await $.session.usage()).cost?.usd
}

export async function recordCost($: $, usd: number | undefined, ticket: string | null) {
  if (usd === undefined) return
  // A /clear starts the figure over.
  if (base === undefined || usd < base) {
    base = usd
    return
  }
  const spent = usd - base
  base = usd
  if (spent <= 0) return
  const dir = `${await cacheDir($)}/costs`
  const path = `${dir}/${await $.session.id()}.json`
  const book = JSON.parse(await $.fs.read(path).catch(() => '{}')) as Record<string, number>
  await $.process.run(['mkdir', '-p', dir])
  await writeAtomic($, path, JSON.stringify(booked(book, spent, ticket)))
}
