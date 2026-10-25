import type { EngineInterface } from 'claude-code'

import { cacheDir, writeAtomic } from './config'

type $ = EngineInterface

const KEEP = 300

// What a session did, one JSON line per event in ~/.cache/cockpit/activity/<space>.jsonl, for the herdr recap popup.
export type Activity = { at: number; kind: 'edit' | 'command' | 'turn'; text: string }

const appended = new Map<string, number>()
let queue: Promise<unknown> = Promise.resolve()

// One line appended per event, in call order; every 50 lines the file is cut back to its last KEEP.
export async function logActivity($: $, space: string | undefined, entry: Omit<Activity, 'at'>) {
  if (!space) return
  const line = JSON.stringify({ at: Date.now(), ...entry })
  queue = queue.then(() => appendActivity($, space, line)).catch(() => undefined)
  return queue
}

async function appendActivity($: $, space: string, line: string) {
  const dir = `${await cacheDir($)}/activity`
  const path = `${dir}/${space}.jsonl`
  await $.process.run(['sh', '-c', 'mkdir -p "$1" && printf "%s\\n" "$3" >> "$2"', 'sh', dir, path, line])
  const count = (appended.get(path) ?? 0) + 1
  appended.set(path, count)
  if (count % 50 !== 0) return
  const lines = (await $.fs.read(path).catch(() => '')).split('\n').filter(l => l.length > 0)
  if (lines.length > KEEP) await writeAtomic($, path, `${lines.slice(-KEEP).join('\n')}\n`)
}

// The entry a tool call leaves: an edited file, or a shell command's first line; reads and searches leave none.
export function activityOf(tool: string, input: Record<string, unknown>, root: string): Omit<Activity, 'at'> | undefined {
  const path = input.file_path ?? input.notebook_path
  if (['Edit', 'Write', 'MultiEdit', 'NotebookEdit'].includes(tool) && typeof path === 'string') {
    return { kind: 'edit', text: path.startsWith(`${root}/`) ? path.slice(root.length + 1) : path }
  }
  if (tool === 'Bash' && typeof input.command === 'string') {
    const line = input.command.split('\n')[0] ?? ''
    return { kind: 'command', text: line.length > 90 ? `${line.slice(0, 89)}…` : line }
  }
  return undefined
}

type Beat = { at: number; space: string | null; repo?: string; branch?: string }

// Another live session on the same repo and branch: the space it runs in, to warn both before they collide.
export async function clashWith($: $, repo: string, branch: string, space: string | undefined) {
  const dir = `${await cacheDir($)}/sessions`
  const self = await $.session.id()
  for (const entry of await $.fs.list(dir).catch(() => [])) {
    if (!entry.name.endsWith('.json') || entry.name === `${self}.json`) continue
    const beat = JSON.parse(await $.fs.read(`${dir}/${entry.name}`).catch(() => '{}')) as Partial<Beat>
    const isLive = Date.now() - (beat.at ?? 0) < 3 * 60_000
    if (isLive && beat.repo === repo && beat.branch === branch && beat.space && beat.space !== space) return beat.space
  }
  return undefined
}
