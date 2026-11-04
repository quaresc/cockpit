import type { EngineInterface } from 'claude-code'

import { isMcpNotReady, notify, shortError } from './ci'
import { cacheDir, loadConfig } from './config'
import type { Mail } from './config'
import { HUB_FRESH_MS } from './feed'
import { callMapped, listAt, pick, text } from './platform'
import { withLease, writeDoc } from './store'

type $ = EngineInterface

// Unread mail through Claude Code's mail connector, for a mail server the hub does not reach itself: the same `mail`
// document the hub writes (crates/connectors mail.rs), refreshed every two minutes by one session.

const STALE_MS = 2 * 60_000
const MAX_MESSAGES = 30
const MAX_SNIPPET = 200

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
}
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

const isUnread = (v: unknown) =>
  Array.isArray(v) ? v.some(l => String(l).toUpperCase() === 'UNREAD') : v === true || v === 1 || ['true', 'unread', 'yes', '1'].includes(String(v).toLowerCase())

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
      date: Number.isNaN(Date.parse(date)) ? date : new Date(date).toISOString().replace(/\.\d{3}Z$/, 'Z'),
      unread: !f.unread || isUnread(pick(raw, f.unread)),
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
      if (!isForced && prev && Date.now() - prev.at < STALE_MS) return
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
        if (fresh.length > 3) await notify($, `✉️ ${fresh.length} new mails`, fresh.slice(0, 3).map(m => `${m.fromName}: ${m.subject}`).join('\n'), 'space:inbox')
        else for (const m of fresh) await notify($, `✉️ ${m.fromName}`, m.subject, m.url || 'space:inbox')
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
