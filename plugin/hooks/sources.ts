import type { EngineInterface } from 'claude-code'

import { loadConfig } from './config'

type $ = EngineInterface
type Block = Record<string, unknown>
type Rows = { columns: string[]; rows: unknown[][] }

// One MCP call, its text blocks joined; an error result throws with the text.
export async function mcpText($: $, server: string, tool: string, args: Record<string, unknown>) {
  const result = await $.mcp.call(server, tool, args)
  const text = result.content.map(c => ('text' in c ? String(c.text) : '')).join('\n')
  if (result.isError) throw new Error(`${server} ${tool}: ${text.slice(0, 200)}`)
  return text
}

// A Metabase result, a saved question or an MBQL query, as columns and rows.
async function metabase($: $, source: Block): Promise<Rows> {
  const server = (await loadConfig($)).metabase.server
  const text = typeof source.question === 'number'
    ? await mcpText($, server, 'execute_question', { id: source.question })
    : await mcpText($, server, 'query', { query: source.query, query_handle: null, continuation_token: null })
  const answer = JSON.parse(text) as { data?: { rows?: unknown[][]; cols?: { display_name?: string; name?: string }[] }; status?: string; error?: string }
  if (answer.status === 'failed' || !answer.data) throw new Error(`Metabase: ${answer.error ?? 'no data'}`)
  return { columns: (answer.data.cols ?? []).map(c => c.display_name ?? c.name ?? '?'), rows: answer.data.rows ?? [] }
}

const POINTS = 60

// Datadog timeseries as rows of time and one column per series, about sixty points across the window.
async function datadog($: $, source: Block): Promise<Rows> {
  const server = (await loadConfig($)).datadog.server
  const text = await mcpText($, server, 'get_datadog_metric', {
    queries: source.queries,
    from: source.from ?? 'now-1h',
    raw_data: true,
    max_tokens: 20000,
    telemetry: { intent: 'A live chart in a cockpit show pane beside a Claude Code session' },
  })
  const json = text.match(/<JSON_DATA>([\s\S]*?)<\/JSON_DATA>/)?.[1]?.trim()
  if (!json) throw new Error(text.includes('NO_DATA') ? 'Datadog: no data for these queries in this window' : `Datadog: ${text.slice(0, 160)}`)
  const series = JSON.parse(json) as { expression: string; scope?: string; values: string; start_time: string; interval_ms: number }[]
  const length = Math.max(...series.map(s => s.values.split(',').length))
  const step = Math.max(1, Math.ceil(length / POINTS))
  const start = Date.parse(series[0]!.start_time)
  const rows: unknown[][] = []
  for (let i = 0; i < length; i += step) {
    const at = new Date(start + i * series[0]!.interval_ms)
    const time = `${String(at.getHours()).padStart(2, '0')}:${String(at.getMinutes()).padStart(2, '0')}`
    rows.push([time, ...series.map(s => {
      const window = s.values.split(',').slice(i, i + step).map(Number).filter(v => !Number.isNaN(v))
      return window.length ? Math.round((window.reduce((a, b) => a + b, 0) / window.length) * 100) / 100 : null
    })])
  }
  return { columns: ['time', ...series.map(s => (series.length > 1 && s.scope ? s.scope : s.expression))], rows }
}

export async function fetchSource($: $, source: Block): Promise<Rows> {
  if (source.metabase) return metabase($, source.metabase as Block)
  return datadog($, source.datadog as Block)
}
