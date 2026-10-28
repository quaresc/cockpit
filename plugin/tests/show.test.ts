import { expect, test } from 'claude-code/testing'

import { answerText, collectAnswers, viewError } from '../hooks/show'

const HOME = '/home/me'
const DIR = `${HOME}/.cache/cockpit/show`
const ran = (exitCode: number, stdout = '') => ({ exitCode, stdout, stderr: '', isStdoutTruncated: false, isStderrTruncated: false })

test('a compare block shows in a pane as well as in the browser', async () => {
  const blocks = [{ type: 'compare', before: '/tmp/before.png', after: 'https://example.com/after.png' }]
  expect(viewError({ pane: 'header', title: 'Header', blocks })).toBe(undefined)
  expect(viewError({ pane: 'header', title: 'Header', placement: 'browser', blocks })).toBe(undefined)
  expect(viewError({ pane: 'header', title: 'Header', blocks: [{ type: 'compare', before: 'before.png', after: '/a.png' }] })).toContain('absolute path')
})

test('a diff line comment reads as the person speaking', async () => {
  const comment = { block: 0, type: 'comment', value: { path: 'src/quota.rs', line: 3, text: 'keep 100 for pro' }, label: 'src/quota.rs:3' }
  expect(answerText('quota', comment, { type: 'diff' })).toBe('In the quota pane I commented on src/quota.rs:3: keep 100 for pro')
  const pick = { block: 1, type: 'choice', value: 'free', label: 'Free' }
  expect(answerText('quota', pick, { type: 'choice', prompt: 'Which plan?' })).toBe('In the quota pane I picked "Free" for: Which plan.')
})

test('each comment reaches the session and leaves the diff as it was', async () => {
  const spec = { title: 'Diff', at: Date.now(), session: 's1', blocks: [{ type: 'diff', patch: 'diff --git a/x b/x' }] }
  const files = new Map<string, string>([[`${DIR}/quota.json`, JSON.stringify(spec)]])
  const comment = (line: number, text: string) => JSON.stringify({ block: 0, type: 'comment', value: { path: 'x', line, text }, label: `x:${line}`, at: line })
  const written: string[] = []
  const said: string[] = []
  // Just what collectAnswers reaches for: files, rm, and the prompt.
  const fake = {
    env: { get: async (name: string) => (name === 'HOME' ? HOME : undefined) },
    fs: {
      read: async (path: string) => {
        const content = files.get(path)
        if (content === undefined) throw new Error('ENOENT')
        return content
      },
      write: async (path: string) => void written.push(path),
      list: async () => [...files.keys()].map(path => ({ name: path.slice(DIR.length + 1), kind: 'file', size: 1, mtimeMs: 1, isLink: false })),
    },
    process: {
      run: async (argv: string[]) => {
        if (argv[0] === 'rm') files.delete(argv[2]!)
        return ran(0)
      },
    },
    prompt: { submit: async ({ text }: { text: string }) => void said.push(text) },
  } as unknown as Parameters<typeof collectAnswers>[0]

  files.set(`${DIR}/quota.0.answer`, comment(1, 'first'))
  await collectAnswers(fake, 's1')
  files.set(`${DIR}/quota.0.answer`, comment(2, 'second'))
  await collectAnswers(fake, 's1')

  expect(said).toEqual(['In the quota pane I commented on x:1: first', 'In the quota pane I commented on x:2: second'])
  expect(written.filter(p => p.endsWith('quota.json'))).toEqual([])
  expect(files.has(`${DIR}/quota.0.answer`)).toBe(false)
})
