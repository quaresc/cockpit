import type { EngineInterface } from 'claude-code'

import { cacheDir, loadConfig } from './config'

type $ = EngineInterface

// The show tool: a view built from a fixed catalog of blocks, drawn by herdr/show.py in a pane split from the session's.
// A later call with the same pane name rewrites the view and the open pane redraws.
export const SHOW_TOOL = 'show'
export const SHOW_TOOL_FULL = `mcp__cockpit__${SHOW_TOOL}`

const PLACEMENTS = ['right', 'down', 'zoomed'] as const
const BLOCKS = ['markdown', 'table', 'chart', 'diff'] as const
const MAX_ROWS = 5000
const MAX_PATCH = 500_000

export const SHOW_SPEC = {
  name: SHOW_TOOL,
  description:
    'Shows a chart, a table, a diff or Markdown in a terminal pane beside this session, like an artifact. Use it to graph ' +
    'query results (Metabase, SQL), to show a diff of a repo or a patch, or for a long table. Blocks: ' +
    '{type:"chart", kind:"line"|"bar", columns, rows, x:<column>, y:[<columns>]} with rows as arrays of values (a line ' +
    'chart for values over time, a bar chart for categories); {type:"table", columns, rows}; {type:"diff", repo:<absolute ' +
    'path>, base?:<git ref, default HEAD>} which follows the working tree, or {type:"diff", patch:<unified diff>}; ' +
    '{type:"markdown", text}. Any block takes a title. Call it again with the same pane name to update that pane.',
  inputSchema: {
    type: 'object',
    properties: {
      pane: { type: 'string', description: 'A short name such as quota-chart; the same name updates the same pane' },
      title: { type: 'string' },
      placement: { type: 'string', enum: PLACEMENTS, description: 'Where a new pane opens: right of this session (default), below it, or zoomed' },
      blocks: { type: 'array', items: { type: 'object', properties: { type: { type: 'string', enum: BLOCKS } }, required: ['type'] } },
    },
    required: ['pane', 'title', 'blocks'],
  },
  isDeferred: false,
}

type Block = Record<string, unknown>

const isText = (v: unknown): v is string => typeof v === 'string'
const list = (values: readonly string[]) => values.map(v => JSON.stringify(v)).join(', ')

// The first rule a block breaks, said with its path and what would be accepted; undefined when it is fine.
function blockError(block: Block, at: string): string | undefined {
  const type = block.type
  if (!BLOCKS.includes(type as (typeof BLOCKS)[number])) return `at ${at}: unknown block type ${JSON.stringify(type)}; allowed: ${list(BLOCKS)}`
  if (block.title !== undefined && !isText(block.title)) return `at ${at}/title: a title is a string`
  if (type === 'markdown') return isText(block.text) ? undefined : `at ${at}/text: markdown needs its text as a string`
  if (type === 'diff') {
    const hasPatch = isText(block.patch)
    const hasRepo = isText(block.repo) && block.repo.startsWith('/')
    if (hasPatch === hasRepo) return `at ${at}: a diff takes either patch (a unified diff) or repo (an absolute path), not ${hasPatch ? 'both' : 'neither'}`
    if (hasPatch && (block.patch as string).length > MAX_PATCH) return `at ${at}/patch: at most ${MAX_PATCH} characters; show a repo diff instead`
    if (block.base !== undefined && !(isText(block.base) && /^[\w.~^@{}/][\w./~^@{}-]*$/.test(block.base))) return `at ${at}/base: a git ref such as main or HEAD~3`
    return undefined
  }
  const columns = block.columns
  const rows = block.rows
  if (!Array.isArray(columns) || columns.length === 0 || !columns.every(isText)) return `at ${at}/columns: a non-empty list of column names`
  if (!Array.isArray(rows)) return `at ${at}/rows: a list of rows, each a list of ${columns.length} values`
  if (rows.length > MAX_ROWS) return `at ${at}/rows: at most ${MAX_ROWS} rows; aggregate in the query first`
  const bad = rows.findIndex(r => !Array.isArray(r) || r.length !== columns.length || r.some(v => v !== null && typeof v === 'object'))
  if (bad >= 0) return `at ${at}/rows/${bad}: each row is a list of ${columns.length} plain values (${list(columns)})`
  if (type === 'table') return undefined
  if (block.kind !== 'line' && block.kind !== 'bar') return `at ${at}/kind: a chart is "line" or "bar"`
  if (!isText(block.x) || !columns.includes(block.x)) return `at ${at}/x: one of the columns: ${list(columns)}`
  const y = block.y
  if (!Array.isArray(y) || y.length === 0) return `at ${at}/y: a non-empty list of the columns to draw`
  const missing = y.find(c => !isText(c) || !columns.includes(c))
  if (missing !== undefined) return `at ${at}/y: ${JSON.stringify(missing)} is not one of the columns: ${list(columns)}`
  const yIndexes = y.map(c => columns.indexOf(c as string))
  const textual = yIndexes.find(i => rows.some(r => (r as unknown[])[i] !== null && typeof (r as unknown[])[i] !== 'number'))
  if (textual !== undefined) return `at ${at}/y: column ${JSON.stringify(columns[textual])} holds values that are not numbers; cast them in the query`
  return undefined
}

export function viewError(input: Record<string, unknown>): string | undefined {
  if (!isText(input.pane) || !/^[a-z0-9][a-z0-9-]{0,39}$/.test(input.pane)) return 'at /pane: a short name of lowercase letters, digits and dashes, such as quota-chart'
  if (!isText(input.title)) return 'at /title: a string'
  if (input.placement !== undefined && !PLACEMENTS.includes(input.placement as (typeof PLACEMENTS)[number])) return `at /placement: one of ${list(PLACEMENTS)}`
  if (!Array.isArray(input.blocks) || input.blocks.length === 0) return 'at /blocks: a non-empty list of blocks'
  for (const [n, block] of (input.blocks as unknown[]).entries()) {
    if (typeof block !== 'object' || block === null) return `at /blocks/${n}: a block is an object with a type`
    const error = blockError(block as Block, `/blocks/${n}`)
    if (error) return error
  }
  return undefined
}

const shellQuote = (text: string) => `'${text.replaceAll("'", `'\\''`)}'`

// Whether the pane showing this view still runs: show.py keeps its pid in <name>.pid while it does.
async function isOpen($: $, dir: string, name: string) {
  const pid = (await $.fs.read(`${dir}/${name}.pid`).catch(() => '')).trim()
  return /^\d+$/.test(pid) && (await $.process.run(['kill', '-0', pid])).exitCode === 0
}

async function openPane($: $, here: { pane: string }, name: string, placement: (typeof PLACEMENTS)[number]) {
  const script = `${(await loadConfig($)).herdrPluginRoot}/show.py`
  if (!(await $.fs.stat(script).then(() => true, () => false))) return `the cockpit herdr plugin has no ${script} yet: update it`
  const split = await $.process.run(['herdr', 'pane', 'split', here.pane, '--direction', placement === 'down' ? 'down' : 'right'], { timeoutMs: 10_000 })
  const pane = (JSON.parse(split.stdout || '{}')?.result?.pane?.pane_id as string | undefined) ?? ''
  if (split.exitCode !== 0 || !pane) return `herdr pane split failed: ${split.stderr.trim()}`
  // exec, so the pane closes with the app.
  await $.process.run(['herdr', 'pane', 'run', pane, `exec python3 ${shellQuote(script)} ${shellQuote(name)}`], { timeoutMs: 10_000 })
  if (placement === 'zoomed') await $.process.run(['herdr', 'pane', 'zoom', pane, '--on'], { timeoutMs: 10_000 })
  // The keyboard stays with the conversation.
  else await $.process.run(['herdr', 'pane', 'focus', '--pane', pane, '--direction', placement === 'down' ? 'up' : 'left'], { timeoutMs: 10_000 })
  return undefined
}

export async function show($: $, input: Record<string, unknown>, here: { pane: string } | undefined): Promise<{ result: string } | { deny: string }> {
  const error = viewError(input)
  if (error) return { deny: error }
  const name = input.pane as string
  const dir = `${await cacheDir($)}/show`
  const spec = { title: input.title, blocks: input.blocks, at: Date.now() }
  await $.fs.write(`${dir}/${name}.json`, JSON.stringify(spec))
  if (await isOpen($, dir, name)) return { result: `Updated the ${name} pane.` }
  if (!here) return { result: `Wrote the view to ${dir}/${name}.json, but this session is not in herdr, so no pane opened.` }
  const failure = await openPane($, here, name, (input.placement as (typeof PLACEMENTS)[number] | undefined) ?? 'right')
  return failure ? { deny: failure } : { result: `Opened the ${name} pane beside this session; q in it closes it.` }
}
