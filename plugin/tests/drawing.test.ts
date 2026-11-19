import { expect, test } from 'claude-code/testing'

import { artifactRefusal, freshTurn, openedPage, pageRefusal, subagentTurn } from '../hooks/drawing'
import { pageHtml } from '../hooks/page'
import { viewError } from '../hooks/show'

const S = '/tmp/scratchpad'

test('a hand-made HTML page opened beside the session is found, a dev server or a web page is not', async () => {
  expect(openedPage(`sh ~/.config/cockpit/herdr/browse.sh ${S}/signups.html; echo "exit=$?"`)).toBe(`${S}/signups.html`)
  expect(openedPage(`terminal-browser action --browser 45188-1 -- open "file://$S/signups-last-week.html"`)).toBe('file://$S/signups-last-week.html')
  expect(openedPage('sh ~/.config/cockpit/herdr/browse.sh localhost:3000')).toBe(undefined)
  expect(openedPage('sh ~/.config/cockpit/herdr/browse.sh https://example.com/docs/index.html')).toBe(undefined)
  expect(openedPage(`cat ${S}/signups.html | head`)).toBe(undefined)
})

test('the page waits for show, the person asking for one, or a stated reason', async () => {
  const command = `sh ~/.config/cockpit/herdr/browse.sh ${S}/signups.html`
  expect(pageRefusal(command, freshTurn('Show me the sign-ups last week in graphs'))).toContain('mcp__cockpit__show')
  expect(pageRefusal(command, { ...freshTurn('Show me the sign-ups'), shown: true })).toBe(undefined)
  expect(pageRefusal(command, freshTurn('Build me an HTML page of the sign-ups'))).toBe(undefined)
  expect(pageRefusal(`COCKPIT_SHOW_FALLBACK="a 3D globe" ${command}`, freshTurn('Show me the sign-ups'))).toBe(undefined)
})

test('an artifact is published only when asked for or after show', async () => {
  const turn = freshTurn('Show me the sign-ups last week in graphs')
  expect(artifactRefusal({ file_path: `${S}/signups.html` }, turn)).toContain('mcp__cockpit__show')
  expect(artifactRefusal({ action: 'list' }, turn)).toBe(undefined)
  expect(artifactRefusal({ file_path: `${S}/signups.html` }, freshTurn('Make it an artifact I can share with the team'))).toBe(undefined)
  expect(artifactRefusal({ file_path: `${S}/signups.html` }, { ...turn, shown: true })).toBe(undefined)
})

test('a subagent whose brief asks for an artifact may publish one, any other still waits for show', async () => {
  const parent = freshTurn('Run the CI report')
  const input = { file_path: `${S}/report.html` }
  expect(artifactRefusal(input, subagentTurn(parent, 'Collect the numbers and publish the report artifact'))).toBe(undefined)
  expect(artifactRefusal(input, subagentTurn(parent, 'Collect the numbers and summarise them'))).toContain('mcp__cockpit__show')
})

test('charts take their new kinds and options, figures their items', async () => {
  const columns = ['lang', 'confirmed', 'not confirmed']
  const rows = [['fr', 9332, 1359], ['en', 7758, 932]]
  const view = (block: Record<string, unknown>) => viewError({ pane: 'p', title: 'T', blocks: [block] })
  expect(view({ type: 'chart', kind: 'bar', horizontal: true, percent: true, columns, rows })).toBe(undefined)
  expect(view({ type: 'chart', kind: 'area', stacked: true, columns, rows })).toBe(undefined)
  expect(view({ type: 'chart', kind: 'donut', columns: ['method', 'n'], rows: [['google', 11683], ['linkedin', 4302]] })).toBe(undefined)
  expect(view({ type: 'chart', kind: 'pie', columns, rows })).toContain('"donut"')
  expect(view({ type: 'chart', kind: 'line', horizontal: true, columns, rows })).toContain('only bars')
  expect(view({ type: 'chart', kind: 'bar', stacked: 'yes', columns, rows })).toContain('true or false')
  expect(view({ type: 'stats', items: [{ label: 'Sign-ups', value: 21234, delta: '-0.8%', good: false, spark: [1, 2, 3] }] })).toBe(undefined)
  expect(view({ type: 'stats', items: [{ label: 'Sign-ups' }] })).toContain('a figure is {label, value}')
  expect(viewError({ pane: 'p', title: 'T', placement: 'browser', blocks: [{ type: 'stats', items: [{ label: 'a', value: 1 }] }] })).toBe(undefined)
})

test('the browser page draws figures and the new chart kinds', async () => {
  const blocks = [
    { type: 'stats', items: [{ label: 'Sign-ups', value: 21234, delta: '-0.8%', good: false }] },
    { type: 'chart', kind: 'donut', columns: ['m', 'n'], rows: [['google', 3], ['email', 1]] },
  ]
  const html = pageHtml({ title: 'Sign-ups', blocks, at: 0 }, blocks)
  expect(html).toContain("stats: b => el('div', { className: 'stats' }")
  expect(html).toContain("'doughnut'")
})
