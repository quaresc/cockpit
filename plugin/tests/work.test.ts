import { expect, test } from 'claude-code/testing'

const HOME = '/home/me'
const REPO = `${HOME}/code/api`
const CLAUDE_TREE = `${REPO}/.claude/worktrees/fix`
const HERDR_TREE = `${HOME}/.herdr/worktrees/api/me-eng-4`
const FOLDERS = [`${HOME}/code`, REPO, `${REPO}/src`, CLAUDE_TREE, `${CLAUDE_TREE}/src`, HERDR_TREE, `${HERDR_TREE}/src`]

// git's answers for the three checkouts: the main one, a Claude Code worktree inside it, a herdr worktree outside.
const ran = (exitCode: number, stdout = '', stderr = '') => ({ exitCode, stdout, stderr, isStdoutTruncated: false, isStderrTruncated: false })

const git = (args: readonly string[]) => {
  const dir = args[args.indexOf('-C') + 1] ?? ''
  const top = [CLAUDE_TREE, HERDR_TREE, REPO].find(t => dir === t || dir.startsWith(`${t}/`))
  if (!top) return ran(128, '', 'not a git repository')
  if (args.includes('--show-toplevel')) return ran(0, `${top}\n`)
  if (args.includes('get-url')) return ran(0, 'git@github.com:o/api.git\n')
  if (args.includes('--show-current')) return ran(0, `${top === REPO ? 'main' : 'me/eng-4'}\n`)
  return ran(0)
}

test('a session follows the checkout its tools touch, worktrees included', async ($, on) => {
  on('env.get', (_, e) => ({ value: (e as { name?: string }).name === 'HOME' ? HOME : undefined }))
  on('fs.read', (_, e) => {
    const path = (e as { path?: string }).path ?? ''
    if (path.endsWith('/.config/cockpit/config.json')) return { value: JSON.stringify({ reposDir: '~/code', tickets: { enabled: false } }) }
    throw new Error('ENOENT')
  })
  on('fs.write', () => ({ value: undefined }))
  on('fs.list', () => ({ value: [] }))
  on('fs.stat', (_, e) => {
    const path = (e as { path?: string }).path ?? ''
    if (FOLDERS.includes(path)) return { value: { kind: 'dir', size: 0, mtimeMs: 0, isLink: false } }
    throw new Error('ENOENT')
  })
  on('process.run', (_, e) => ({ value: e.argv[0] === 'git' ? git(e.argv) : ran(0) }))
  on('tool.call', () => ({ result: {} }))
  // The kit keeps $.state beneath this hook: each write of the session's checkout is seen on its way down.
  let seen: unknown = null
  on('state.set', ($, e, next) => {
    const set = e as unknown as { key?: string; value?: unknown }
    if (set.key === 'work') seen = set.value
    return next(e)
  })
  const work = async () => seen

  await $.tool.call({ tool: 'Read', file_path: `${REPO}/src/a.ts` })
  expect(await work()).toEqual({ repo: 'api', dir: REPO })

  await $.tool.call({ tool: 'Edit', file_path: `${CLAUDE_TREE}/src/a.ts`, old_string: 'a', new_string: 'b' })
  expect(await work()).toEqual({ repo: 'api', dir: CLAUDE_TREE })

  await $.tool.call({ tool: 'Bash', command: `cd ~/.herdr/worktrees/api/me-eng-4/src && ls` })
  expect(await work()).toEqual({ repo: 'api', dir: HERDR_TREE })
})
