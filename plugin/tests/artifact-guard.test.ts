import { type TestBody, expect, test } from 'claude-code/testing'

const HOME = '/home/me'
const PUBLISH = { tool: 'Artifact', file_path: '/tmp/scratchpad/signups.html' } as const

function setUp(on: Parameters<TestBody>[1], config?: Record<string, unknown>) {
  on('env.get', (_, e) => ({ value: (e as { name?: string }).name === 'HOME' ? HOME : undefined }))
  on('fs.read', (_, e) => {
    const path = (e as { path?: string }).path ?? ''
    if (config && path === `${HOME}/.config/cockpit/config.json`) return { value: JSON.stringify(config) }
    throw new Error('ENOENT')
  })
  on('fs.write', () => ({ value: undefined }))
  on('fs.stat', () => {
    throw new Error('ENOENT')
  })
  on('process.run', () => ({ value: { exitCode: 0, stdout: '', stderr: '', isStdoutTruncated: false, isStderrTruncated: false } }))
  on('tool.call', () => ({ result: { url: 'https://claude.ai/artifact/abc123' } }))
}

test('a new artifact is refused when no config turns the guard off', async ($, on) => {
  setUp(on)
  const done = await $.tool.call(PUBLISH)
  expect(done.isError).toBe(true)
  expect(done.text).toContain('mcp__cockpit__show')
})

test('a new artifact is refused when the config keeps the guard on', async ($, on) => {
  setUp(on, { artifactGuard: true })
  const done = await $.tool.call(PUBLISH)
  expect(done.isError).toBe(true)
  expect(done.text).toContain('mcp__cockpit__show')
})

test('a new artifact is published when the config turns the guard off', async ($, on) => {
  setUp(on, { artifactGuard: false })
  const done = await $.tool.call(PUBLISH)
  expect(done.isError).not.toBe(true)
  expect(done.result).toEqual({ url: 'https://claude.ai/artifact/abc123' })
})
