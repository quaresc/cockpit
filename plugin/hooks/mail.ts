import type { EngineInterface } from 'claude-code'

import { isMcpNotReady, notify, shortError } from './ci'
import { cacheDir, loadConfig, writeAtomic } from './config'
import type { Mail } from './config'
import { HUB_FRESH_MS } from './feed'
import { callMapped, listAt, pick, text } from './platform'
import { withLease, writeDoc } from './store'

type $ = EngineInterface

// Unread mail through Claude Code's mail connector, for a mail server the hub does not reach itself: the same `mail`
// document the hub writes (crates/connectors mail.rs), refreshed by one session, and the threads read whole ahead of
// time (or when the mail space asks), kept a week in mail/<id>.json.

const STALE_MS = 2 * 60_000
const MAX_MESSAGES = 30
const MAX_SNIPPET = 200
const MAX_HTML = 400_000
const KEEP_THREADS_MS = 7 * 86_400_000
// How many threads one refresh reads ahead, so a full inbox fills over a few refreshes.
const READ_AHEAD = 10

export type MailMessage = {
  id: string
  threadId: string
  from: string
  fromName: string
  subject: string
  snippet: string
  url: string
  date: string
  unread: boolean
  labels: string[]
}

export type ThreadMessage = { id: string; from: string; fromName: string; date: string; html: string; text: string }
type MailState = { at: number; messages: MailMessage[]; notified: string[]; error?: string; by?: 'hub' }

async function readMail($: $): Promise<MailState | null> {
  try {
    return JSON.parse(await $.fs.read(`${await cacheDir($)}/mail.json`)) as MailState
  } catch {
    return null
  }
}

// "Ana Lima <ana@acme.com>" as its address and name.
function sender(from: string): [string, string | undefined] {
  const m = from.match(/^\s*"?([^"<]*?)"?\s*<([^>]+)>\s*$/)
  return m ? [m[2]!.trim(), m[1]!.trim() || undefined] : [from.trim(), undefined]
}

const isoDate = (date: string) => (Number.isNaN(Date.parse(date)) ? date : new Date(date).toISOString().replace(/\.\d{3}Z$/, 'Z'))

const isUnread = (v: unknown) =>
  Array.isArray(v) ? v.some(l => String(l).toUpperCase() === 'UNREAD') : v === true || v === 1 || ['true', 'unread', 'yes', '1'].includes(String(v).toLowerCase())

const listOf = (v: unknown) => (Array.isArray(v) ? v.filter((l): l is string => typeof l === 'string') : [])

export function messagesOf(mail: Mail, answer: unknown): MailMessage[] {
  const f = mail.fields
  const out: MailMessage[] = []
  for (const raw of listAt(answer, f.messages)) {
    const id = text(pick(raw, f.id))
    if (!id) continue
    const threadId = text(pick(raw, f.threadId)) ?? ''
    const [address, named] = sender(text(pick(raw, f.from)) ?? '')
    const date = text(pick(raw, f.date)) ?? ''
    out.push({
      id,
      threadId,
      from: address,
      fromName: text(pick(raw, f.fromName)) || named || address,
      subject: text(pick(raw, f.subject)) ?? '',
      snippet: (text(pick(raw, f.snippet)) ?? '').split(/\s+/).join(' ').trim().slice(0, MAX_SNIPPET),
      url: text(pick(raw, f.url)) || mail.urlTemplate.replace('{id}', id).replace('{threadId}', threadId),
      date: isoDate(date),
      unread: !f.unread || isUnread(pick(raw, f.unread)),
      labels: listOf(pick(raw, f.labels)),
    })
  }
  return out.sort((a, b) => (Date.parse(b.date) || 0) - (Date.parse(a.date) || 0)).slice(0, MAX_MESSAGES)
}

let inflight: Promise<void> | undefined

export function refreshMail($: $, isForced = false) {
  inflight ??= (async () => {
    try {
      const { mail } = await loadConfig($)
      if (!mail.enabled || !mail.server || !mail.tools.listInbox?.tool) return
      const prev = await readMail($)
      // The hub keeps mail.json while it reaches the mail server itself.
      if (prev?.by === 'hub' && Date.now() - prev.at < HUB_FRESH_MS) return
      const every = Math.max(15, Number((await loadConfig($)).refresh?.mail ?? 60)) * 1000
      if (!isForced && prev && Date.now() - prev.at < Math.min(every, STALE_MS)) return
      await withLease($, 'mail', async () => {
        const messages = await callMapped($, mail, 'listInbox')
          .then(answer => messagesOf(mail, answer))
          .catch(async (err: Error) => {
            if (!isMcpNotReady(err)) await writeDoc($, 'mail', { ...(prev ?? { messages: [], notified: [] }), at: prev?.at ?? 0, error: `${mail.name} ${shortError(err)}` })
            return null
          })
        if (!messages) return
        const unread = messages.filter(m => m.unread).map(m => m.id)
        const known = new Set(prev?.notified ?? [])
        const fresh = prev ? messages.filter(m => m.unread && !known.has(m.id)) : []
        await writeDoc($, 'mail', { at: Date.now(), messages, notified: unread })
        if (fresh.length > 3) await notify($, `✉️ ${fresh.length} new mails`, fresh.slice(0, 3).map(m => `${m.fromName}: ${m.subject}`).join('\n'), 'space:mail')
        else for (const m of fresh) await notify($, `✉️ ${m.fromName}`, m.subject || m.snippet, `mail:${m.id}`)
        await readAhead($, messages)
      })
    } finally {
      inflight = undefined
    }
  })()
  return inflight
}

// A mail request from the inbox (archive, mark read) through the connector, and the message gone or read in mail.json.
export async function mailRequest($: $, kind: 'mail-archive' | 'mail-read', id: string) {
  const { mail } = await loadConfig($)
  const state = await readMail($)
  const message = state?.messages.find(m => m.id === id)
  const values = { message: id, thread: message?.threadId || id }
  await callMapped($, mail, kind === 'mail-archive' ? 'archive' : 'markRead', values)
  if (!state) return
  const messages = kind === 'mail-archive' ? state.messages.filter(m => m.id !== id) : state.messages.map(m => (m.id === id ? { ...m, unread: false } : m))
  await writeDoc($, 'mail', { ...state, messages })
}

// A `get` answer as the thread, oldest first: the messages at the `thread` field, else the answer as one message.
export function threadOf(mail: Mail, answer: unknown): ThreadMessage[] {
  const f = mail.fields
  const listed = f.thread ? listAt(answer, f.thread) : []
  const raws = listed.length ? listed : [answer]
  return raws
    .map(raw => {
      const [address, named] = sender(text(pick(raw, f.from ?? 'from')) ?? '')
      return {
        id: text(pick(raw, 'id')) ?? '',
        from: address,
        fromName: text(pick(raw, f.fromName)) || named || address,
        date: isoDate(text(pick(raw, f.date ?? 'date')) ?? ''),
        html: (text(pick(raw, f.html ?? 'html')) ?? '').slice(0, MAX_HTML),
        text: text(pick(raw, f.body ?? 'body')) ?? '',
      }
    })
    .filter(m => m.html || m.text || m.from)
    .sort((a, b) => (Date.parse(a.date) || 0) - (Date.parse(b.date) || 0))
}

// Whether a kept thread holds the message: read without error since its date, as a reply landing makes it stale.
export const isFresh = (doc: { at: number; error?: string } | null, m: Pick<MailMessage, 'date'>) => Boolean(doc && !doc.error && doc.at >= (Date.parse(m.date) || 0))

async function keptFresh($: $, m: MailMessage) {
  const doc = await $.fs.read(`${await cacheDir($)}/mail/${threadFile(m.id)}`).then(t => JSON.parse(t) as { at: number; error?: string }).catch(() => null)
  return isFresh(doc, m)
}

// The threads of the listed messages read ahead, so opening one is instant: the newest first, a few per refresh.
async function readAhead($: $, messages: MailMessage[]) {
  const { mail } = await loadConfig($)
  if (!mail.tools.get?.tool) return
  let read = 0
  for (const m of messages) {
    if (read >= READ_AHEAD) return
    if (await keptFresh($, m)) continue
    read++
    await mailGet($, m.id).catch(() => undefined)
  }
}

// The file a message's thread is kept in, safe whatever its id holds; the hub names it the same (connectors mail.rs).
export const threadFile = (id: string) => `${id.replace(/[^A-Za-z0-9._-]/g, '_').replace(/^\.+/, '')}.json`

// A thread the mail space asked for, read whole through the connector into mail/<id>.json, where the space picks it up;
// a failure is kept there too, so the space says why. Threads older than a week go.
export async function mailGet($: $, id: string) {
  const { mail } = await loadConfig($)
  const dir = `${await cacheDir($)}/mail`
  await $.process.run(['mkdir', '-p', dir])
  for (const entry of await $.fs.list(dir).catch(() => [])) {
    if (entry.kind === 'file' && Date.now() - entry.mtimeMs > KEEP_THREADS_MS) await $.process.run(['rm', '-f', `${dir}/${entry.name}`])
  }
  const listed = (await readMail($))?.messages.find(m => m.id === id)
  const path = `${dir}/${threadFile(id)}`
  try {
    const answer = await callMapped($, mail, 'get', { message: id, thread: listed?.threadId || id })
    await writeAtomic($, path, JSON.stringify({ at: Date.now(), id, messages: threadOf(mail, answer) }))
  } catch (err) {
    await writeAtomic($, path, JSON.stringify({ at: Date.now(), id, messages: [], error: `${mail.name} ${shortError(err as Error)}` }))
    throw err
  }
}
