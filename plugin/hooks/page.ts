import type { Block, Spec } from './show'

// A show view as a web page, for terminal-browser beside the session: what a terminal draws poorly (wide tables, big
// diagrams, pictures, side-by-side diffs) at full resolution. The page renders its blocks itself from the embedded spec.
export const PAGE_BLOCKS = ['markdown', 'stats', 'table', 'chart', 'flow', 'sequence', 'mermaid', 'image', 'compare', 'diff']

const LIBS = {
  marked: 'https://cdn.jsdelivr.net/npm/marked@12.0.2/marked.min.js',
  mermaid: 'https://cdn.jsdelivr.net/npm/mermaid@10.9.1/dist/mermaid.min.js',
  chart: 'https://cdn.jsdelivr.net/npm/chart.js@4.4.3/dist/chart.umd.min.js',
  diff: 'https://cdn.jsdelivr.net/npm/diff2html@3.4.48/bundles/js/diff2html-ui.min.js',
  diffCss: 'https://cdn.jsdelivr.net/npm/diff2html@3.4.48/bundles/css/diff2html.min.css',
}

const picture = (ref: string) => (ref.startsWith('/') ? `file://${ref}` : ref)

const quoted = (text: string) => `"${text.replace(/"/g, '#quot;').replace(/\n/g, ' ')}"`

// flow and sequence blocks are drawn by Mermaid in the page.
function flowText(block: Block) {
  const shape = (kind: unknown, label: string) => (kind === 'round' ? `(${quoted(label)})` : kind === 'decision' ? `{${quoted(label)}}` : `[${quoted(label)}]`)
  const nodes = (block.nodes as Block[]).map((n, i) => `  n${i}${shape(n.kind, n.label as string)}`)
  const ids = new Map((block.nodes as Block[]).map((n, i) => [n.id as string, `n${i}`]))
  const edges = ((block.edges ?? []) as Block[]).map(e => `  ${ids.get(e.from as string)} -->${e.label ? `|${quoted(e.label as string)}|` : ''} ${ids.get(e.to as string)}`)
  return [`flowchart ${block.direction === 'right' ? 'LR' : 'TD'}`, ...nodes, ...edges].join('\n')
}

function sequenceText(block: Block) {
  const names = new Map<string, string>()
  const id = (name: string) => {
    if (!names.has(name)) names.set(name, `a${names.size}`)
    return names.get(name)!
  }
  const actors = ((block.actors ?? []) as (string | Block)[]).map(a => (typeof a === 'string' ? { id: a, label: a } : { id: a.id as string, label: (a.label ?? a.id) as string }))
  const lines = actors.map(a => `  participant ${id(a.id)} as ${a.label.replace(/[:;\n]/g, ' ')}`)
  for (const m of block.messages as Block[]) {
    const text = String(m.label ?? m.note ?? '').replace(/[;\n]/g, ' ')
    if (m.note !== undefined) lines.push(`  Note over ${id(m.over as string)}: ${text}`)
    else lines.push(`  ${id(m.from as string)}${m.reply ? '-->>' : '->>'}${id(m.to as string)}: ${text || ' '}`)
  }
  const late = [...names].filter(([name]) => !actors.some(a => a.id === name)).map(([name, alias]) => `  participant ${alias} as ${name.replace(/[:;\n]/g, ' ')}`)
  return ['sequenceDiagram', ...late, ...lines].join('\n')
}

// Blocks with diagrams turned into Mermaid text and a repo diff into its patch, ready for the page's script.
export function pageBlocks(blocks: Block[], patches: Map<number, string>) {
  return blocks.map((b, n) => {
    if (b.type === 'flow') return { type: 'mermaid', title: b.title, text: flowText(b) }
    if (b.type === 'sequence') return { type: 'mermaid', title: b.title, text: sequenceText(b) }
    if (b.type === 'image' && typeof b.path === 'string') return { ...b, url: `file://${b.path}` }
    if (b.type === 'compare') return { ...b, before: picture(b.before as string), after: picture(b.after as string) }
    if (b.type === 'diff' && patches.has(n)) return { ...b, patch: patches.get(n) }
    return b
  })
}

export function pageHtml(spec: Spec, blocks: Block[]) {
  const data = JSON.stringify({ title: spec.title, url: spec.url, blocks }).replace(/</g, '\\u003c')
  const title = spec.title.replace(/[<&]/g, c => (c === '<' ? '&lt;' : '&amp;'))
  return `<!doctype html>
<html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>${title}</title>
<link rel="stylesheet" href="${LIBS.diffCss}">
<style>
:root { --bg: #101010; --panel: #1A1A1D; --line: #2C2C30; --text: #D2D2D2; --dim: #8B8B8B; --accent: #FFC799; --green: #99FFE4; --red: #FF8080; }
* { box-sizing: border-box; }
body { margin: 0; padding: 24px 28px 48px; background: var(--bg); color: var(--text); font: 15px/1.55 -apple-system, BlinkMacSystemFont, "Inter", sans-serif; }
h1 { font-size: 22px; margin: 0 0 20px; color: var(--accent); }
h2.block { font-size: 15px; margin: 32px 0 10px; color: var(--dim); text-transform: uppercase; letter-spacing: .06em; }
a { color: var(--accent); }
code, pre { font-family: "FiraCode Nerd Font Mono", ui-monospace, monospace; font-size: 13px; }
pre { background: var(--panel); padding: 12px 14px; border-radius: 8px; overflow-x: auto; }
table { border-collapse: collapse; font-variant-numeric: tabular-nums; }
.scroll { overflow-x: auto; }
th, td { padding: 6px 12px; border-bottom: 1px solid var(--line); text-align: left; white-space: nowrap; }
th { color: var(--dim); font-weight: 600; position: sticky; top: 0; background: var(--bg); }
td.num { text-align: right; }
figure { margin: 0; } figcaption { color: var(--dim); font-size: 13px; margin-top: 6px; }
img { max-width: 100%; border-radius: 6px; }
.chart { position: relative; height: 360px; }
.chart.donut { height: 280px; max-width: 520px; }
.stats { display: grid; grid-template-columns: repeat(auto-fit, minmax(170px, 1fr)); gap: 12px; }
.stat { background: var(--panel); border: 1px solid var(--line); border-radius: 10px; padding: 14px 16px; }
.stat .label { color: var(--dim); font-size: 13px; } .stat .value { font-size: 26px; font-weight: 600; margin-top: 2px; }
.stat .note { color: var(--dim); font-size: 12px; } .good { color: var(--green); } .bad { color: var(--red); }
.compare { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 12px; align-items: start; }
.compare figcaption { text-transform: uppercase; letter-spacing: .06em; margin: 0 0 6px; }
@media (max-width: 420px) { .compare { grid-template-columns: 1fr; } }
.mermaid { background: var(--panel); border-radius: 8px; padding: 16px; }
.d2h-wrapper { --d2h-bg-color: var(--bg); }
.error { color: var(--red); }
</style></head>
<body>
<h1>${title}</h1>
<main id="view"></main>
<script type="application/json" id="spec">${data}</script>
<script src="${LIBS.marked}"></script>
<script src="${LIBS.mermaid}"></script>
<script src="${LIBS.chart}"></script>
<script src="${LIBS.diff}"></script>
<script>
const spec = JSON.parse(document.getElementById('spec').textContent)
const SERIES = ['#FFC799', '#99FFE4', '#A0A0FF', '#FF99CC', '#FF8080']
const view = document.getElementById('view')
const el = (tag, props = {}, children = []) => { const e = Object.assign(document.createElement(tag), props); e.append(...children); return e }
// A chart measures its box, so it is drawn once the page holds it.
const later = []
const render = {
  markdown: b => el('div', { innerHTML: marked.parse(b.text) }),
  table: b => el('div', { className: 'scroll' }, [el('table', {}, [
    el('thead', {}, [el('tr', {}, b.columns.map(c => el('th', { textContent: c })))]),
    el('tbody', {}, b.rows.map(r => el('tr', {}, r.map(v => el('td', { textContent: v ?? '', className: typeof v === 'number' ? 'num' : '' }))))),
  ])]),
  stats: b => el('div', { className: 'stats' }, b.items.map(t => el('div', { className: 'stat' }, [
    el('div', { className: 'label', textContent: t.label }),
    el('div', { className: 'value', textContent: typeof t.value === 'number' ? t.value.toLocaleString('en-US') : t.value }),
    ...(t.delta !== undefined ? [el('div', { className: t.good === true ? 'good' : t.good === false ? 'bad' : 'note', textContent: String(t.delta) })] : []),
    ...(t.note ? [el('div', { className: 'note', textContent: t.note })] : []),
  ]))),
  chart: b => {
    const canvas = el('canvas')
    const x = b.x ?? b.columns[0], xi = b.columns.indexOf(x)
    const ys = b.y ?? b.columns.filter(c => c !== x)
    const donut = b.kind === 'donut', stacked = Boolean(b.stacked || b.percent)
    const totals = b.rows.map(r => ys.reduce((sum, c) => sum + (r[b.columns.indexOf(c)] ?? 0), 0))
    const value = (r, i, c) => { const v = r[b.columns.indexOf(c)]; return b.percent && v !== null && totals[i] ? (v / totals[i]) * 100 : v }
    Chart.defaults.color = '#8B8B8B'; Chart.defaults.borderColor = '#2C2C30'
    const datasets = donut
      ? [{ label: ys[0], data: b.rows.map(r => r[b.columns.indexOf(ys[0])]), backgroundColor: b.rows.map((_, i) => SERIES[i % 5]), borderColor: '#101010', borderWidth: 2 }]
      : ys.map((c, i) => ({ label: c, data: b.rows.map((r, j) => value(r, j, c)), borderColor: SERIES[i % 5], backgroundColor: b.kind === 'area' ? SERIES[i % 5] + (stacked ? '99' : '33') : SERIES[i % 5],
          fill: b.kind === 'area' ? (stacked && i > 0 ? '-1' : 'origin') : false, pointRadius: 0, tension: .2 }))
    const scale = { stacked, ...(b.percent ? { max: 100, ticks: { callback: v => v + '%' } } : {}) }
    const options = donut ? { maintainAspectRatio: false, animation: false, cutout: '62%', plugins: { legend: { position: 'right' } } }
      : { maintainAspectRatio: false, animation: false, indexAxis: b.horizontal || (b.kind === 'bar' && b.rows.length > 12) ? 'y' : 'x', scales: { x: { stacked }, y: { stacked }, ...(b.horizontal ? { x: scale } : { y: scale }) } }
    later.push(() => new Chart(canvas, { type: donut ? 'doughnut' : b.kind === 'area' ? 'line' : b.kind, data: { labels: b.rows.map(r => r[xi]), datasets }, options }))
    return el('div', { className: donut ? 'chart donut' : 'chart' }, [canvas])
  },
  mermaid: b => el('pre', { className: 'mermaid', textContent: b.text }),
  // Before and after side by side.
  compare: b => el('div', { className: 'compare' }, [
    el('figure', {}, [el('figcaption', { textContent: 'Before' }), el('img', { src: b.before })]),
    el('figure', {}, [el('figcaption', { textContent: 'After' }), el('img', { src: b.after })]),
  ]),
  image: b => el('figure', {}, [el('img', { src: b.url }), ...(b.caption ? [el('figcaption', { textContent: b.caption })] : [])]),
  diff: b => {
    const box = el('div')
    if (b.error) box.append(el('p', { className: 'error', textContent: b.error }))
    else new Diff2HtmlUI(box, b.patch, { outputFormat: 'side-by-side', drawFileList: true, matching: 'lines', colorScheme: 'dark' }).draw()
    return box
  },
}
for (const b of spec.blocks) {
  if (b.title) view.append(el('h2', { className: 'block', textContent: b.title }))
  try { view.append(render[b.type](b)) } catch (err) { view.append(el('p', { className: 'error', textContent: String(err) })) }
}
for (const draw of later) draw()
mermaid.initialize({ startOnLoad: false, theme: 'dark', themeVariables: { primaryColor: '#1A1A1D', primaryBorderColor: '#FFC799', lineColor: '#8B8B8B', fontFamily: 'Inter, sans-serif' } })
mermaid.run({ querySelector: '.mermaid' })
</script>
</body></html>
`
}
