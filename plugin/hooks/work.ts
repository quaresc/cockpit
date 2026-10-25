import { atom, read, update } from 'claude-code'
import type { EngineInterface } from 'claude-code'

import type { Work } from '../types'

import { home, loadConfig } from './config'

type $ = EngineInterface

const work = atom({ plugin: 'cockpit', key: 'work' } as const, null)

// Which repo this session works in, followed from the paths its tools touch, and the branch checked out there.

async function isDir($: $, path: string) {
  try {
    return (await $.fs.stat(path)).kind === 'dir'
  } catch {
    return false
  }
}

export async function trackWork($: $, args: Record<string, unknown>) {
  const text = ['command', 'file_path', 'path', 'notebook_path']
    .map(key => args[key])
    .filter((v): v is string => typeof v === 'string')
    .join(' ')
  const root = (await loadConfig($)).reposDir
  const tilde = root.startsWith(await home($)) ? `~${root.slice((await home($)).length)}` : root
  const under = [root, tilde].map(p => p.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')).join('|')
  const absolute = [...text.matchAll(new RegExp(`(?:${under})/([A-Za-z0-9._-]+)`, 'g'))].map(m => m[1])
  const relative = [...text.matchAll(/(?:^|[;&|]\s*)cd\s+([A-Za-z0-9._-]+)(?=[\s;&|/]|$)/g)].map(m => m[1])
  const repo = [...absolute, ...relative].filter((r): r is string => !!r).at(-1)
  if (!repo || repo === (await read($, work))?.repo) return
  const dir = `${root}/${repo}`
  if (await isDir($, `${dir}/.git`)) await update($, work, () => ({ repo, dir }))
}

export async function branchOf($: $, current: Work) {
  const { exitCode, stdout } = await $.process.run(['git', '-C', current.dir, 'branch', '--show-current'])
  const branch = exitCode === 0 ? stdout.trim() : ''
  return ['main', 'master', 'develop', ''].includes(branch) ? undefined : branch
}
