import { atom, read, update } from 'claude-code'
import type { EngineInterface } from 'claude-code'

import type { Work } from '../types'

import { home, loadConfig } from './config'

type $ = EngineInterface

const work = atom({ plugin: 'cockpit', key: 'work' } as const, null)

// Which checkout this session works in, followed from the paths its tools touch: the repos folder, herdr's worktrees,
// and Claude Code's worktrees inside a repo. The checkout is git's own top folder, so a worktree reads its own branch.

const escape = (text: string) => text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')

// The folders a checkout may sit under, matched absolute and as ~/…
async function roots($: $) {
  const dir = await home($)
  const tilde = (path: string) => (path.startsWith(`${dir}/`) ? `~${path.slice(dir.length)}` : path)
  const reposDir = (await loadConfig($)).reposDir
  const pattern = [reposDir, `${dir}/.herdr/worktrees`].flatMap(f => [f, tilde(f)]).map(escape).join('|')
  return { reposDir, pattern, dir }
}

const resolved = new Map<string, Work | null>()

// The checkout holding a path, by git, with the repo named after its origin remote, as GitHub names it.
async function checkoutOf($: $, path: string): Promise<Work | null> {
  // A checkout already known holds most paths: no git call for them.
  const known = [...resolved.values()].filter((w): w is Work => w !== null).sort((a, b) => b.dir.length - a.dir.length)
  const holder = known.find(w => path === w.dir || path.startsWith(`${w.dir}/`))
  if (holder && !path.slice(holder.dir.length).startsWith('/.claude/worktrees/')) return holder
  let dir = path
  // A file, or one a Write is about to create: its nearest existing folder.
  while (dir.length > 1 && !(await $.fs.stat(dir).then(s => s.kind === 'dir', () => false))) dir = dir.slice(0, dir.lastIndexOf('/')) || '/'
  const hit = resolved.get(dir)
  if (hit !== undefined) return hit
  const top = await $.process.run(['git', '-C', dir, 'rev-parse', '--show-toplevel'])
  let found: Work | null = null
  if (top.exitCode === 0) {
    const root = top.stdout.trim()
    const origin = await $.process.run(['git', '-C', root, 'remote', 'get-url', 'origin'])
    const common = await $.process.run(['git', '-C', root, 'rev-parse', '--path-format=absolute', '--git-common-dir'])
    const named = origin.exitCode === 0 ? origin.stdout.trim() : common.exitCode === 0 ? common.stdout.trim().replace(/\/\.git\/?$/, '') : root
    found = { repo: named.replace(/\.git$/, '').split(/[/:]/).pop() || root, dir: root }
  }
  resolved.set(dir, found)
  return found
}

async function follow($: $, path: string) {
  const current = await read($, work)
  const found = await checkoutOf($, path)
  if (found && (found.dir !== current?.dir || found.repo !== current.repo)) await update($, work, () => found)
}

export async function trackWork($: $, args: Record<string, unknown>) {
  const text = ['command', 'file_path', 'path', 'notebook_path']
    .map(key => args[key])
    .filter((v): v is string => typeof v === 'string')
    .join(' ')
  const { reposDir, pattern, dir } = await roots($)
  const absolute = [...text.matchAll(new RegExp(`(?:${pattern})/[^\\s'"\`;|&()<>]+`, 'g'))].map(m => m[0].replace(/^~/, dir))
  const relative = [...text.matchAll(/(?:^|[;&|]\s*)cd\s+([A-Za-z0-9._-]+)(?=[\s;&|/]|$)/g)].map(m => `${reposDir}/${m[1]}`)
  const path = [...absolute, ...relative].at(-1)
  if (!path) return
  // Inside the current checkout nothing changes, unless the path enters one of Claude Code's worktrees within it.
  const current = await read($, work)
  const rest = current && (path === current.dir || path.startsWith(`${current.dir}/`)) ? path.slice(current.dir.length) : undefined
  if (rest !== undefined && !rest.startsWith('/.claude/worktrees/')) return
  await follow($, path)
}

// The session's own folder at start: one opened in a worktree's space knows its checkout before any tool runs.
export async function seedWork($: $, cwd: string) {
  const { reposDir, dir } = await roots($)
  if (cwd !== reposDir && cwd !== dir) await follow($, cwd)
}

export async function branchOf($: $, current: Work) {
  const { exitCode, stdout } = await $.process.run(['git', '-C', current.dir, 'branch', '--show-current'])
  const branch = exitCode === 0 ? stdout.trim() : ''
  return ['main', 'master', 'develop', ''].includes(branch) ? undefined : branch
}
