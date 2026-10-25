import type { EngineInterface } from 'claude-code'

import { cacheDir, writeAtomic } from './config'

type $ = EngineInterface

// What each ticket cost: the session's spend since its last figure goes to the ticket it works on then, in
// ~/.cache/cockpit/costs/<session>.json, which only that session writes; the inbox app sums them.

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
  if (spent <= 0 || !ticket) return
  const dir = `${await cacheDir($)}/costs`
  const path = `${dir}/${await $.session.id()}.json`
  const book = JSON.parse(await $.fs.read(path).catch(() => '{}')) as Record<string, number>
  book[ticket] = (book[ticket] ?? 0) + spent
  await $.process.run(['mkdir', '-p', dir])
  await writeAtomic($, path, JSON.stringify(book))
}
