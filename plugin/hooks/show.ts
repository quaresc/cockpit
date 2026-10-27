import type { EngineInterface } from 'claude-code'

import type { ViewChip } from '../types'

import { browse } from './browser'
import { cacheDir, loadConfig } from './config'
import { PAGE_BLOCKS, pageBlocks, pageHtml } from './page'
import { fetchSource } from './sources'

type $ = EngineInterface

// The show tool: a view built from a fixed catalog of blocks, drawn by herdr/show.py in a pane split from the session's.
// A later call with the same pane name rewrites the view and the open pane redraws.
export const SHOW_TOOL = 'show'
export const SHOW_TOOL_FULL = `mcp__cockpit__${SHOW_TOOL}`

const PLACEMENTS = ['right', 'down', 'zoomed', 'browser'] as const
const BLOCKS = ['markdown', 'table', 'chart', 'flow', 'sequence', 'mermaid', 'image', 'compare', 'diff', 'log', 'ci', 'choice', 'confirm'] as const
const NODE_KINDS = ['box', 'round', 'decision'] as const
const MAX_NODES = 60
const MAX_ROWS = 5000
const MAX_PATCH = 500_000
const MIN_EVERY = 30

export const SHOW_SPEC = {
  name: SHOW_TOOL,
  description:
    'Shows charts, tables, diagrams, pictures, diffs, logs or a question in a terminal pane beside this session, like an ' +
    'artifact. Use it to graph query results, draw a process or an exchange, show a screenshot or a Figma frame, a diff, a ' +
    'live log or CI run, or to ask the person to pick an option. Never draw diagrams in ASCII inside markdown. Blocks: ' +
    '{type:"chart", kind:"line"|"bar", columns, rows, x?, y?:[columns]} (rows as arrays; line over time, bar for categories); ' +
    '{type:"table", columns, rows}; a chart or table can instead take source:{metabase:{question:<id>}|{query:<MBQL>}} or ' +
    'source:{datadog:{queries:[...], from?:"now-1h"}} to refresh itself every `every` seconds, or command:<shell printing JSON> ' +
    'with every; {type:"flow", nodes:[{id, label, kind?:"box"|"round"|"decision"}], edges:[{from, to, label?}], ' +
    'direction?:"down"|"right"}; {type:"sequence", actors?:[...], messages:[{from, to, label, reply?} | {note, over}]}; ' +
    '{type:"mermaid", text} for a flowchart or sequenceDiagram; {type:"image", path|url, caption?}; {type:"diff", repo:<absolute ' +
    'path>, base?} following the working tree, or {type:"diff", patch}; {type:"log", command, every?, lines?} tailing a command; ' +
    '{type:"ci", repo, branch} for the CI run of a branch; {type:"choice", prompt?, options:[{label, value?, preview?}]} or ' +
    '{type:"confirm", summary, consequence?, confirm_label?}, whose answer reaches you as the person\'s next message; ' +
    '{type:"markdown", text}. Any block takes a title. The same pane name updates the same pane. placement:"browser" ' +
    'draws the view as a web page in the browser beside the session instead, at full resolution: use it for tables too ' +
    'wide for a pane, big diagrams, pictures and side-by-side diffs; it takes markdown, table, chart, flow, sequence, ' +
    'mermaid, image and diff blocks, with rows given in the call, and {type:"compare", before, after} for two screenshots ' +
    'side by side (a before/after of a front change).',
  inputSchema: {
    type: 'object',
    properties: {
      pane: { type: 'string', description: 'A short name such as quota-chart; the same name updates the same pane' },
      title: { type: 'string' },
      url: { type: 'string', description: 'Optional: what `o` in the pane opens in the browser' },
      placement: { type: 'string', enum: PLACEMENTS, description: 'Where a new pane opens: right of this session (default), below it, zoomed, or as a page in the browser beside it' },
      blocks: { type: 'array', items: { type: 'object', properties: { type: { type: 'string', enum: BLOCKS } }, required: ['type'] } },
    },
    required: ['pane', 'title', 'blocks'],
  },
  isDeferred: false,
}

export type Block = Record<string, unknown>
type Placement = (typeof PLACEMENTS)[number]
export type Spec = {
  title: string
  blocks: Block[]
  at: number
  session?: string
  placement?: Placement
  cwd?: string
  url?: string
}

const isText = (v: unknown): v is string => typeof v === 'string'
const isCount = (v: unknown) => v === undefined || (typeof v === 'number' && v >= 1)
const list = (values: readonly string[]) => values.map(v => JSON.stringify(v)).join(', ')

// The first rule a block breaks, said with its path and what would be accepted; undefined when it is fine.
function blockError(block: Block, at: string): string | undefined {
  const type = block.type
  if (!BLOCKS.includes(type as (typeof BLOCKS)[number])) return `at ${at}: unknown block type ${JSON.stringify(type)}; allowed: ${list(BLOCKS)}`
  if (block.title !== undefined && !isText(block.title)) return `at ${at}/title: a title is a string`
  if (type === 'markdown' || type === 'mermaid') return isText(block.text) ? undefined : `at ${at}/text: ${type} needs its text as a string`
  if (type === 'flow') return flowError(block, at)
  if (type === 'sequence') return sequenceError(block, at)
  if (type === 'choice') return choiceError(block, at)
  if (type === 'confirm') return isText(block.summary) ? undefined : `at ${at}/summary: what is being confirmed, as Markdown`
  if (type === 'ci') return isText(block.repo) && isText(block.branch) ? undefined : `at ${at}: a ci block is {repo, branch}`
  if (type === 'log') {
    if (!isText(block.command) || block.command === '') return `at ${at}/command: the shell command to tail`
    return isCount(block.every) && isCount(block.lines) ? undefined : `at ${at}: every and lines are numbers of at least 1`
  }
  if (type === 'compare') {
    const isPicture = (v: unknown) => isText(v) && (v.startsWith('/') || /^https?:\/\//.test(v))
    return isPicture(block.before) && isPicture(block.after) ? undefined : `at ${at}: a compare block is {before, after}, each an absolute path or an http(s) URL`
  }
  if (type === 'image') {
    const hasPath = isText(block.path) && block.path.startsWith('/')
    const hasUrl = isText(block.url) && /^https?:\/\//.test(block.url)
    if (hasPath === hasUrl) return `at ${at}: an image takes either path (an absolute path) or url (http or https), not ${hasPath ? 'both' : 'neither'}`
    return isCount(block.width) ? undefined : `at ${at}/width: a number of columns`
  }
  if (type === 'diff') {
    const hasPatch = isText(block.patch)
    const hasRepo = isText(block.repo) && block.repo.startsWith('/')
    if (hasPatch === hasRepo) return `at ${at}: a diff takes either patch (a unified diff) or repo (an absolute path), not ${hasPatch ? 'both' : 'neither'}`
    if (hasPatch && (block.patch as string).length > MAX_PATCH) return `at ${at}/patch: at most ${MAX_PATCH} characters; show a repo diff instead`
    if (block.base !== undefined && !(isText(block.base) && /^[\w.~^@{}/][\w./~^@{}-]*$/.test(block.base))) return `at ${at}/base: a git ref such as main or HEAD~3`
    return undefined
  }
  if (type === 'chart' && block.kind !== 'line' && block.kind !== 'bar') return `at ${at}/kind: a chart is "line" or "bar"`
  // A table or chart gets its rows from the call, from a command, or from a source the session refreshes.
  if (block.command !== undefined || block.source !== undefined) {
    if (block.command !== undefined && block.source !== undefined) return `at ${at}: either command or source, not both`
    if (block.command !== undefined && (!isText(block.command) || block.command === '')) return `at ${at}/command: a shell command printing JSON`
    if (block.source !== undefined) {
      const error = sourceError(block.source as Block, `${at}/source`)
      if (error) return error
      if (block.every !== undefined && !(typeof block.every === 'number' && block.every >= MIN_EVERY)) return `at ${at}/every: at least ${MIN_EVERY} seconds`
    }
    return isCount(block.every) ? undefined : `at ${at}/every: a number of seconds`
  }
  const columns = block.columns
  const rows = block.rows
  if (!Array.isArray(columns) || columns.length === 0 || !columns.every(isText)) return `at ${at}/columns: a non-empty list of column names, or a command or source to get them from`
  if (!Array.isArray(rows)) return `at ${at}/rows: a list of rows, each a list of ${columns.length} values`
  if (rows.length > MAX_ROWS) return `at ${at}/rows: at most ${MAX_ROWS} rows; aggregate in the query first`
  const bad = rows.findIndex(r => !Array.isArray(r) || r.length !== columns.length || r.some(v => v !== null && typeof v === 'object'))
  if (bad >= 0) return `at ${at}/rows/${bad}: each row is a list of ${columns.length} plain values (${list(columns)})`
  if (type === 'table') return undefined
  if (block.x !== undefined && (!isText(block.x) || !columns.includes(block.x))) return `at ${at}/x: one of the columns: ${list(columns)}`
  const y = block.y ?? columns.filter(c => c !== (block.x ?? columns[0]))
  if (!Array.isArray(y) || y.length === 0) return `at ${at}/y: a non-empty list of the columns to draw`
  const missing = y.find(c => !isText(c) || !columns.includes(c))
  if (missing !== undefined) return `at ${at}/y: ${JSON.stringify(missing)} is not one of the columns: ${list(columns)}`
  const yIndexes = y.map(c => columns.indexOf(c as string))
  const textual = yIndexes.find(i => rows.some(r => (r as unknown[])[i] !== null && typeof (r as unknown[])[i] !== 'number'))
  if (textual !== undefined) return `at ${at}/y: column ${JSON.stringify(columns[textual])} holds values that are not numbers; cast them in the query`
  return undefined
}

function sourceError(source: Block, at: string): string | undefined {
  const metabase = source?.metabase as Block | undefined
  const datadog = source?.datadog as Block | undefined
  if (!metabase === !datadog) return `at ${at}: either {metabase: {question | query}} or {datadog: {queries, from?}}`
  if (metabase) {
    const isQuestion = typeof metabase.question === 'number' && metabase.question >= 1
    const isQuery = typeof metabase.query === 'object' && metabase.query !== null
    return isQuestion !== isQuery ? undefined : `at ${at}/metabase: either question (a saved question's id) or query (an MBQL query)`
  }
  const queries = datadog?.queries
  if (!Array.isArray(queries) || queries.length === 0 || !queries.every(isText)) return `at ${at}/datadog/queries: a list of metric queries`
  return datadog?.from === undefined || isText(datadog.from) ? undefined : `at ${at}/datadog/from: a start such as now-1h`
}

function flowError(block: Block, at: string): string | undefined {
  if (block.direction !== undefined && block.direction !== 'down' && block.direction !== 'right') return `at ${at}/direction: "down" or "right"`
  const nodes = block.nodes
  if (!Array.isArray(nodes) || nodes.length === 0 || nodes.length > MAX_NODES) return `at ${at}/nodes: a list of 1 to ${MAX_NODES} nodes, each {id, label, kind?}`
  const ids = new Set<string>()
  for (const [n, node] of nodes.entries()) {
    const { id, label, kind } = (node ?? {}) as Block
    if (!isText(id) || id === '' || !isText(label)) return `at ${at}/nodes/${n}: a node is {id, label} with both strings`
    if (ids.has(id)) return `at ${at}/nodes/${n}/id: ${JSON.stringify(id)} is already the id of another node`
    if (kind !== undefined && !NODE_KINDS.includes(kind as (typeof NODE_KINDS)[number])) return `at ${at}/nodes/${n}/kind: one of ${list(NODE_KINDS)}`
    ids.add(id)
  }
  const edges = block.edges ?? []
  if (!Array.isArray(edges)) return `at ${at}/edges: a list of {from, to, label?}`
  for (const [n, edge] of edges.entries()) {
    const { from, to, label } = (edge ?? {}) as Block
    for (const [name, end] of [['from', from], ['to', to]] as const) {
      if (!isText(end) || !ids.has(end)) return `at ${at}/edges/${n}/${name}: ${JSON.stringify(end)} is not a node id; ids: ${list([...ids])}`
    }
    if (label !== undefined && !isText(label)) return `at ${at}/edges/${n}/label: a string`
  }
  return undefined
}

function sequenceError(block: Block, at: string): string | undefined {
  const actors = block.actors ?? []
  if (!Array.isArray(actors) || !actors.every(a => isText(a) || (isText((a as Block)?.id) && ((a as Block).label === undefined || isText((a as Block).label))))) {
    return `at ${at}/actors: a list of names, or of {id, label}`
  }
  const messages = block.messages
  if (!Array.isArray(messages) || messages.length === 0) return `at ${at}/messages: a non-empty list of {from, to, label} or {note, over}`
  for (const [n, m] of messages.entries()) {
    const { from, to, label, note, over } = (m ?? {}) as Block
    const isNote = isText(note) && isText(over)
    const isMessage = isText(from) && isText(to) && (label === undefined || isText(label))
    if (!isNote && !isMessage) return `at ${at}/messages/${n}: a message is {from, to, label, reply?}, a note {note, over}`
  }
  return undefined
}

function choiceError(block: Block, at: string): string | undefined {
  const options = block.options
  if (!Array.isArray(options) || options.length < 2) return `at ${at}/options: at least two {label, value?, preview?}`
  const bad = options.findIndex(o => !isText((o as Block)?.label) || ((o as Block).preview !== undefined && !isText((o as Block).preview)))
  return bad < 0 ? undefined : `at ${at}/options/${bad}: an option is {label, value?, preview?} with label and preview strings`
}

export function viewError(input: Record<string, unknown>): string | undefined {
  if (!isText(input.pane) || !/^[a-z0-9][a-z0-9-]{0,39}$/.test(input.pane)) return 'at /pane: a short name of lowercase letters, digits and dashes, such as quota-chart'
  if (!isText(input.title)) return 'at /title: a string'
  if (input.url !== undefined && !(isText(input.url) && /^https?:\/\//.test(input.url))) return 'at /url: an http or https URL'
  if (input.placement !== undefined && !PLACEMENTS.includes(input.placement as Placement)) return `at /placement: one of ${list(PLACEMENTS)}`
  if (!Array.isArray(input.blocks) || input.blocks.length === 0) return 'at /blocks: a non-empty list of blocks'
  for (const [n, block] of (input.blocks as unknown[]).entries()) {
    if (typeof block !== 'object' || block === null) return `at /blocks/${n}: a block is an object with a type`
    const error = blockError(block as Block, `/blocks/${n}`)
    if (error) return error
    const b = block as Block
    if (b.type === 'compare' && input.placement !== 'browser') return `at /blocks/${n}: a compare block needs placement "browser"`
    if (input.placement === 'browser' && (!PAGE_BLOCKS.includes(b.type as string) || b.source !== undefined || b.command !== undefined)) {
      return `at /blocks/${n}: a browser view takes ${list(PAGE_BLOCKS)} blocks with their data in the call; show this one in a pane`
    }
  }
  return undefined
}

// The shell commands a call would have the pane run on its own, which the person approves like any command.
export function commandsOf(input: Record<string, unknown>) {
  return (Array.isArray(input.blocks) ? (input.blocks as Block[]) : []).map(b => b?.command).filter(isText)
}

const shellQuote = (text: string) => `'${text.replaceAll("'", `'\\''`)}'`

async function showDir($: $) {
  return `${await cacheDir($)}/show`
}

// Whether the pane showing this view still runs: show.py keeps its pid in <name>.pid while it does.
async function isOpen($: $, dir: string, name: string) {
  const pid = (await $.fs.read(`${dir}/${name}.pid`).catch(() => '')).trim()
  return /^\d+$/.test(pid) && (await $.process.run(['kill', '-0', pid])).exitCode === 0
}

const PID_RECHECK_MS = 30_000
const openSeen = new Map<string, { at: number; mtime: number; isOpen: boolean }>()

// isOpen for the polls: kept until the pid file changes or 30 s pass, since a pane that quits removes its pid file.
async function isOpenSoon($: $, dir: string, name: string, pidMtime: number) {
  const seen = openSeen.get(name)
  if (seen && seen.mtime === pidMtime && Date.now() - seen.at < PID_RECHECK_MS) return seen.isOpen
  const value = pidMtime > 0 && (await isOpen($, dir, name))
  openSeen.set(name, { at: Date.now(), mtime: pidMtime, isOpen: value })
  return value
}

async function openPane($: $, here: { pane: string }, name: string, placement: Placement) {
  const script = `${(await loadConfig($)).herdrPluginRoot}/app.sh`
  if (!(await $.fs.stat(script).then(() => true, () => false))) return `the cockpit herdr plugin has no ${script} yet: update it`
  const split = await $.process.run(['herdr', 'pane', 'split', here.pane, '--direction', placement === 'down' ? 'down' : 'right'], { timeoutMs: 10_000 })
  const pane = (JSON.parse(split.stdout || '{}')?.result?.pane?.pane_id as string | undefined) ?? ''
  if (split.exitCode !== 0 || !pane) return `herdr pane split failed: ${split.stderr.trim()}`
  // exec, so the pane closes with the app.
  await $.process.run(['herdr', 'pane', 'run', pane, `exec sh ${shellQuote(script)} show ${shellQuote(name)}`], { timeoutMs: 10_000 })
  if (placement === 'zoomed') await $.process.run(['herdr', 'pane', 'zoom', pane, '--on'], { timeoutMs: 10_000 })
  // The keyboard stays with the conversation.
  else await $.process.run(['herdr', 'pane', 'focus', '--pane', pane, '--direction', placement === 'down' ? 'up' : 'left'], { timeoutMs: 10_000 })
  return undefined
}

export async function readSpec($: $, name: string) {
  return JSON.parse(await $.fs.read(`${await showDir($)}/${name}.json`).catch(() => 'null')) as Spec | null
}

export async function writeSpec($: $, name: string, spec: Spec) {
  await $.fs.write(`${await showDir($)}/${name}.json`, JSON.stringify(spec))
}

// A browser view: its page written beside the spec, then opened beside the session, or reloaded where it is open.
async function presentPage($: $, name: string, spec: Spec) {
  const patches = new Map<number, string>()
  for (const [n, b] of spec.blocks.entries()) {
    if (b.type !== 'diff' || typeof b.repo !== 'string') continue
    const done = await $.process.run(['git', '-C', b.repo, 'diff', typeof b.base === 'string' ? b.base : 'HEAD', '--'], { timeoutMs: 15_000 })
    patches.set(n, done.exitCode === 0 ? done.stdout : '')
  }
  const path = `${await showDir($)}/${name}.html`
  await $.fs.write(path, pageHtml(spec, pageBlocks(spec.blocks, patches)))
  await browse($, path, { size: 0.6, reload: true })
  return { result: `Opened the ${name} page in the browser beside this session.` }
}

// Writes a view and makes sure a pane shows it: the open one redraws, else one opens beside the session.
export async function present($: $, name: string, spec: Spec, here: { pane: string } | undefined) {
  await writeSpec($, name, spec)
  if (spec.placement === 'browser') return presentPage($, name, spec)
  if (await isOpen($, await showDir($), name)) return { result: `Updated the ${name} pane.` }
  if (!here) return { result: `Wrote the view to ${await showDir($)}/${name}.json, but this session is not in herdr, so no pane opened.` }
  const failure = await openPane($, here, name, spec.placement ?? 'right')
  return failure ? { deny: failure } : { result: `Opened the ${name} pane beside this session; q hides it, its footer chip brings it back.` }
}

export async function show($: $, input: Record<string, unknown>, here: { pane: string } | undefined, cwd: string | undefined): Promise<{ result: string } | { deny: string }> {
  const error = viewError(input)
  if (error) return { deny: error }
  const name = input.pane as string
  const spec: Spec = {
    title: input.title as string,
    blocks: input.blocks as Block[],
    at: Date.now(),
    session: await $.session.id(),
    placement: (input.placement as Placement | undefined) ?? 'right',
    cwd,
    url: input.url as string | undefined,
  }
  // A block with a source gets its first rows before the pane opens.
  spec.blocks = await Promise.all(spec.blocks.map(b => (b.source ? refreshed($, b) : b)))
  return present($, name, spec, here)
}

async function refreshed($: $, block: Block): Promise<Block> {
  try {
    const { columns, rows } = await fetchSource($, block.source as Block)
    return { ...block, columns, rows, refreshed_at: Date.now(), error: undefined }
  } catch (err) {
    return { ...block, columns: block.columns ?? [], rows: block.rows ?? [], refreshed_at: Date.now(), error: (err as Error).message.slice(0, 200) }
  }
}

// ---------------------------------------------------------------------------
// The footer chips: this session's latest views, open or hidden

const CHIPS = 3
const KEEP_MS = 7 * 86_400_000

type Listing = Awaited<ReturnType<EngineInterface['fs']['list']>>

let specsSeen: { key: string; specs: (Spec & { name: string })[] } = { key: '', specs: [] }

// Every view's spec, parsed again only when a spec file in the folder changed: several polls a second ask.
async function specsOf($: $, session: string, listing?: Listing) {
  const dir = await showDir($)
  const entries = (listing ?? (await $.fs.list(dir).catch(() => []))).filter(e => /^[a-z0-9][a-z0-9-]*\.json$/.test(e.name))
  const key = entries.map(e => `${e.name}:${e.mtimeMs}`).sort().join('|')
  if (key !== specsSeen.key) {
    const specs = await Promise.all(entries.map(async e => {
      const spec = JSON.parse(await $.fs.read(`${dir}/${e.name}`).catch(() => 'null')) as Spec | null
      return spec && { ...spec, name: e.name.slice(0, -5) }
    }))
    // A week-old view is gone for good, chip and file.
    for (const spec of specs) if (spec && Date.now() - spec.at > KEEP_MS) await $.process.run(['rm', '-f', `${dir}/${spec.name}.json`])
    specsSeen = { key, specs: specs.filter((s): s is Spec & { name: string } => s !== null && Date.now() - s.at <= KEEP_MS) }
  }
  return specsSeen.specs.filter(s => s.session === session)
}

const pidMtime = (listing: Listing, name: string) => listing.find(e => e.name === `${name}.pid`)?.mtimeMs ?? 0

export async function viewsOf($: $, session: string): Promise<ViewChip[]> {
  const dir = await showDir($)
  const listing = await $.fs.list(dir).catch(() => [])
  const mine = (await specsOf($, session, listing)).sort((a, b) => b.at - a.at).slice(0, CHIPS)
  return Promise.all(mine.map(async s => ({ name: s.name, isOpen: await isOpenSoon($, dir, s.name, pidMtime(listing, s.name)) })))
}

// A chip's press: an open view's pane closes, a hidden one opens again where it was. Returns whether it opened.
export async function toggleView($: $, name: string, here: { pane: string } | undefined) {
  const dir = await showDir($)
  if (await isOpen($, dir, name)) {
    const pid = (await $.fs.read(`${dir}/${name}.pid`)).trim()
    await $.process.run(['kill', pid])
    return false
  }
  const spec = await readSpec($, name)
  if (spec?.placement === 'browser') await presentPage($, name, spec)
  else if (spec && here) await openPane($, here, name, spec.placement ?? 'right')
  return true
}

export async function isViewOpen($: $, name: string) {
  return isOpen($, await showDir($), name)
}

// ---------------------------------------------------------------------------
// Live views: sources refreshed on their own, and answers handed back to the session

// Every view of this session whose pane is open gets its due sources fetched again.
export async function refreshSources($: $, session: string) {
  const dir = await showDir($)
  const listing = await $.fs.list(dir).catch(() => [])
  for (const spec of await specsOf($, session, listing)) {
    if (!spec.blocks.some(b => b.source) || !(await isOpenSoon($, dir, spec.name, pidMtime(listing, spec.name)))) continue
    const fresh = new Map<number, Block>()
    for (const [n, block] of spec.blocks.entries()) {
      const every = typeof block.every === 'number' ? block.every : (block.source as Block)?.datadog ? 60 : 300
      if (block.source && Date.now() - ((block.refreshed_at as number | undefined) ?? 0) >= every * 1000) fresh.set(n, await refreshed($, block))
    }
    // Only the refreshed blocks' data goes into the file as it is now, so an answer or a newer call written meanwhile stays.
    const current = await readSpec($, spec.name)
    if (fresh.size === 0 || !current || current.at !== spec.at) continue
    const blocks = current.blocks.map((b, n) => {
      const f = fresh.get(n)
      return f ? { ...b, columns: f.columns, rows: f.rows, refreshed_at: f.refreshed_at, error: f.error } : b
    })
    await writeSpec($, spec.name, { ...current, blocks })
  }
}

type Answer = { block: number; type: string; value: unknown; label: string }

// An answer the person gave in a pane becomes their next message to the session, once.
export async function collectAnswers($: $, session: string) {
  const dir = await showDir($)
  const listing = await $.fs.list(dir).catch(() => [])
  if (!listing.some(e => e.name.endsWith('.answer'))) return
  const names = new Set((await specsOf($, session, listing)).map(s => s.name))
  for (const entry of listing) {
    const found = entry.name.match(/^([a-z0-9][a-z0-9-]*)\.(\d+)\.answer$/)
    if (!found || !names.has(found[1]!)) continue
    const path = `${dir}/${entry.name}`
    const answer = JSON.parse(await $.fs.read(path).catch(() => 'null')) as Answer | null
    await $.process.run(['rm', '-f', path])
    if (!answer) continue
    const spec = await readSpec($, found[1]!)
    const block = spec?.blocks[answer.block]
    if (spec && block) {
      spec.blocks[answer.block] = { ...block, answered: answer.label }
      await writeSpec($, found[1]!, spec)
    }
    const asked = (block?.prompt ?? block?.summary ?? block?.title ?? '') as string
    const text = answer.type === 'confirm'
      ? `In the ${found[1]} pane I ${answer.value ? 'confirmed' : 'declined'}${asked ? `: ${asked.split('\n')[0]}` : ''}.`
      : `In the ${found[1]} pane I picked "${answer.label}"${asked ? ` for: ${asked.replace(/[.?!:]$/, '')}` : ''}.`
    void $.prompt.submit({ text })
  }
}
