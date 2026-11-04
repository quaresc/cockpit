import { expect, test } from 'claude-code/testing'

import { messagesOf } from '../hooks/mail'

// Claude's Gmail connector answers search_threads with threads and, in the summary view, their latest message.
const GMAIL = {
  enabled: true,
  name: 'Gmail',
  server: 'claude.ai Gmail',
  tools: {},
  fields: {
    messages: 'threads[].messages', id: 'threadId', threadId: 'threadId', from: 'sender', fromName: 'addressDisplayNames.0.displayName',
    subject: 'subject', snippet: 'snippet', url: 'viewUrl', date: 'date', unread: 'labelIds',
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
})
