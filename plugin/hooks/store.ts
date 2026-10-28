import type { EngineInterface } from 'claude-code'

import { cacheDir, home } from './config'

type $ = EngineInterface

// The documents the hub keeps for everyone (inbox, ci, meetings, brief, monitor) and its leases, through `cockpit store`. The hub
// is their only writer and mirrors each to <name>.json, which the mod reads as before.

const LEASE_TTL_S = 120
const LEASE_BEAT_MS = 30_000

export type Doc = 'inbox' | 'ci' | 'meetings' | 'brief' | 'monitor'

async function store($: $, args: string[]) {
  const command = `${await home($)}/.config/cockpit/herdr/cockpit.sh`
  const { exitCode, stdout, stderr } = await $.process.run(['sh', command, 'store', ...args], { timeoutMs: 20_000 })
  // cockpit.sh says nothing when the plugin has no binary yet: /cockpit:doctor tells why.
  if (exitCode !== 0 || stdout.trim() === '') throw new Error(`cockpit store ${args[0]}: ${stderr.trim() || 'no cockpit binary'}`)
  return stdout.trim()
}

// Sends the document whole (`put`), or only the fields given (`patch`), and returns the document as the hub has it after.
export async function writeDoc<T>($: $, doc: Doc, value: unknown, how: 'put' | 'patch' = 'put'): Promise<T> {
  const file = `${await cacheDir($)}/store-${doc}-${Math.random().toString(36).slice(2)}.json`
  await $.fs.write(file, JSON.stringify(value))
  try {
    return JSON.parse(await store($, [how, doc, file])) as T
  } finally {
    await $.process.run(['rm', '-f', file])
  }
}

// Runs `fn` while this session holds the lease `name`, so one session does the job for all of them; undefined when
// another holds it. The lease is renewed while `fn` runs and lapses on its own when the session dies.
export async function withLease<T>($: $, name: string, fn: () => Promise<T>): Promise<T | undefined> {
  const holder = `${await $.session.id()}-${Math.random().toString(36).slice(2)}`
  if ((await store($, ['lease', name, holder, String(LEASE_TTL_S)]).catch(() => 'held')) !== 'granted') return undefined
  const beat = $.clock.every(LEASE_BEAT_MS, () => void store($, ['renew', name, holder, String(LEASE_TTL_S)]).catch(() => undefined))
  try {
    return await fn()
  } finally {
    beat.cancel()
    await store($, ['release', name, holder]).catch(() => undefined)
  }
}
