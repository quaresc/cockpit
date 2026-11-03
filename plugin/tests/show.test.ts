import { expect, test } from 'claude-code/testing'

import { answerText, blueprintName, collectAnswers, isBlueprintFile, viewError } from '../hooks/show'

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
  const review = { block: 0, type: 'review', value: { comments: [] }, label: '1) src/quota.rs:3: keep 100 2) src/plan.rs:9: rename' }
  expect(answerText('quota', review, { type: 'diff' })).toBe('In the quota pane I commented on the diff: 1) src/quota.rs:3: keep 100 2) src/plan.rs:9: rename')
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

test('a blueprint is one Mermaid flowchart per view, named after its session', async () => {
  const plan = { type: 'blueprint', text: '---\ntitle: Quota limits\n---\nflowchart TD\n  A[Read limits] --> B[Count calls]\n  class A done' }
  expect(viewError({ pane: 'blueprint', title: 'Plan', blocks: [plan] })).toBe(undefined)
  expect(viewError({ pane: 'blueprint', title: 'Plan', blocks: [{ type: 'blueprint', text: 'sequenceDiagram\n A->>B: hi' }] })).toContain('flowchart')
  expect(viewError({ pane: 'blueprint', title: 'Plan', blocks: [plan, plan] })).toContain('one blueprint')
  expect(viewError({ pane: 'blueprint', title: 'Plan', placement: 'browser', blocks: [plan] })).toContain('in a pane')
  expect(blueprintName('blueprint', '3F2A1C9D-0000-4000-8000-000000000000')).toBe('blueprint-3f2a1c9d')
  const fake = { env: { get: async (name: string) => (name === 'HOME' ? HOME : undefined) } } as unknown as Parameters<typeof isBlueprintFile>[0]
  expect(await isBlueprintFile(fake, `${DIR}/blueprint-3f2a1c9d.mmd`)).toBe(true)
  expect(await isBlueprintFile(fake, `${DIR}/../config.mmd`)).toBe(false)
  expect(await isBlueprintFile(fake, `${HOME}/repo/plan.mmd`)).toBe(false)
})

test('a click on a blueprint box reads as a change request about it', async () => {
  const ask = { block: 0, type: 'blueprint', value: { node: 'B', label: 'Count calls', text: 'count per org, not per user' }, label: 'Count calls' }
  expect(answerText('blueprint-3f2a1c9d', ask, { type: 'blueprint' })).toBe('In the blueprint, about «Count calls» (box B): count per org, not per user')
})
