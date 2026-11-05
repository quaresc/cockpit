import { expect, test } from 'claude-code/testing'

import { isFresh, messagesOf, threadFile, threadOf, unescape } from '../hooks/mail'
import { checkRequest } from '../hooks/protocol'

// Claude's Gmail connector answers search_threads with threads and, in the summary view, their latest message.
const GMAIL = {
  enabled: true,
  name: 'Gmail',
  server: 'claude.ai Gmail',
  tools: {},
  fields: {
    messages: 'threads[].messages', id: 'threadId', threadId: 'threadId', from: 'sender', fromName: 'addressDisplayNames.0.displayName',
    subject: 'subject', snippet: 'snippet', url: 'viewUrl', date: 'date', unread: 'labelIds', labels: 'labelIds|label_ids',
    thread: 'messages', body: 'plaintextBody|plaintext_body', html: 'htmlBody|html_body',
  },
  urlTemplate: '',
}

test('unread Gmail threads read as the inbox mail, newest first', () => {
  const answer = {
    threads: [
      { id: 't1', messages: [{ threadId: 't1', sender: 'notifications@linear.app', addressDisplayNames: [{ displayName: 'Linear' }], subject: '6 unread',
        snippet: 'You have\n 6   unread', date: '2026-11-04T14:53:22Z', labelIds: ['UNREAD', 'INBOX'], viewUrl: 'https://mail.google.com/#all/t1' }] },
      { id: 't2', messages: [{ threadId: 't2', sender: 'Ana Lima <ana@acme.io>', subject: 'Invitation', snippet: 'Join', date: '2026-11-04T15:00:00Z',
        labelIds: ['INBOX'], viewUrl: 'https://mail.google.com/#all/t2' }] },
    ],
  }
  const messages = messagesOf(GMAIL, answer)
  expect(messages.map(m => m.id)).toEqual(['t2', 't1'])
  expect(messages[0]).toMatchObject({ from: 'ana@acme.io', fromName: 'Ana Lima', unread: false })
  expect(messages[1]).toMatchObject({ fromName: 'Linear', snippet: 'You have 6 unread', unread: true, url: 'https://mail.google.com/#all/t1' })
  expect(messages[1]?.labels).toEqual(['UNREAD', 'INBOX'])
})

test('a Gmail thread reads whole in either case, oldest first, as the mail space keeps it', () => {
  // get_thread with FULL_CONTENT: camelCase as the connector answers, snake_case as it documents its fields.
  const answer = {
    id: 't2',
    messages: [
      { id: 'm2', sender: 'Ana Lima <ana@acme.io>', date: '2026-11-04T15:00:00Z', htmlBody: '<p>Noon?</p>', plaintextBody: 'Noon?' },
      { id: 'm1', sender: 'me@acme.example', date: '2026-11-04T14:00:00Z', plaintext_body: 'Lunch?', html_body: '' },
    ],
  }
  const thread = threadOf(GMAIL, answer)
  expect(thread.map(m => m.id)).toEqual(['m1', 'm2'])
  expect(thread[0]).toMatchObject({ from: 'me@acme.example', fromName: 'me@acme.example', text: 'Lunch?', html: '' })
  expect(thread[1]).toMatchObject({ fromName: 'Ana Lima', html: '<p>Noon?</p>', text: 'Noon?' })
  expect(threadOf({ ...GMAIL, fields: {} }, { from: 'Sam <sam@example.com>', body: 'Hi' })).toMatchObject([{ fromName: 'Sam', text: 'Hi' }])
  expect(threadFile('../a/b:c')).toBe('_a_b_c.json')
})

test('the mail space asks a session for a thread with a mail-get request', () => {
  expect(checkRequest({ type: 'mail-get', label: 'Invitation', message: 't2' })).toEqual({ request: { type: 'mail-get', label: 'Invitation', message: 't2' } })
  expect(checkRequest({ type: 'mail-get', label: 'Invitation' })).toMatchObject({ refusal: expect.stringContaining('mail-get needs message as a string') })
})

test('a kept thread is read again when a reply lands after it, or when reading it failed', () => {
  const date = '2026-11-04T10:00:00Z'
  const at = Date.parse(date)
  expect(isFresh({ at: at + 1 }, { date })).toBe(true)
  expect(isFresh({ at: at - 1 }, { date })).toBe(false)
  expect(isFresh({ at: at + 1, error: 'Gmail timed out' }, { date })).toBe(false)
  expect(isFresh(null, { date })).toBe(false)
})

test('escaped snippets and subjects read as text', () => {
  expect(unescape('haven&#39;t &amp; R&amp;D &lt;3 &#x2019; &bogus;')).toBe("haven't & R&D <3 \u2019 &bogus;")
})
