import { atom, read, update } from 'claude-code'
import type { Register } from 'claude-code'

import { activityOf, logActivity } from './activity'
import { maybeBrief } from './brief'
import { processRequests, refreshCi } from './ci'
import { home } from './config'
import { refreshLinks, refreshViews, renderFooter } from './footer'
import { REPORT_SPEC, REPORT_TOOL_FULL, bindHandoff, recordRunbookOwner, reportStep, settleTurn } from './handoffs'
import { herdrHere, hereSoon, pageContext } from './herdr'
import { refresh } from './inbox'
import { refreshMeetings } from './meetings'
import { registerSettings, registerSettingsCommand } from './settings'
import { SHOW_SPEC, SHOW_TOOL_FULL, collectAnswers, commandsOf, refreshSources, show } from './show'
import { clearRows, closeIdleBrowsers, notifyFinished, reportSide, reportUsage } from './sidebar'
import { checkUpdate, forget, heartbeat } from './upkeep'
import { branchOf, trackWork } from './work'

const links = atom({ plugin: 'cockpit', key: 'links' } as const, null)
const sidePage = atom({ plugin: 'cockpit', key: 'sidePage' } as const, null)
const ticket = atom({ plugin: 'cockpit', key: 'ticket' } as const, null)
const usage = atom({ plugin: 'cockpit', key: 'usage' } as const, null)
const views = atom({ plugin: 'cockpit', key: 'views' } as const, [])
const work = atom({ plugin: 'cockpit', key: 'work' } as const, null)

// The session's timers: a /clear starts a new session in the same process, and the old one's must not tick on beside it.
let timers: { cancel: () => void }[] = []

const stopTimers = () => {
  for (const timer of timers) timer.cancel()
  timers = []
}

export const register: Register = on => {
  registerSettings(on)

  on('session.start', async ($, e, next) => {
    stopTimers()
    const every = (ms: number, fn: () => void) => void timers.push($.clock.every(ms, fn))
    await $.command.register({ name: 'todo', description: 'Add to the to-do list in the herdr to-do space; alone, list it', argumentHint: '[text]' })
    await registerSettingsCommand($)
    await $.tool.register(REPORT_SPEC).catch(() => undefined)
    await $.tool.register(SHOW_SPEC).catch(() => undefined)
    every(60_000, () => void refresh($, false).then(() => maybeBrief($)).catch(() => undefined))
    every(60_000, () => void refreshCi($).catch(() => undefined))
    every(60_000, () => void refreshMeetings($).catch(() => undefined))
    void refreshMeetings($).catch(() => undefined)
    // A view hidden with q in its pane, or closed with herdr, turns its chip quiet within a few seconds.
    every(3_000, () => void refreshViews($).catch(() => undefined))
    every(10_000, () => void reportSide($).catch(() => undefined))
    every(60_000, () => void closeIdleBrowsers($).catch(() => undefined))
    // An answer given in a pane reaches the session as the person's next message.
    every(2_000, () => void $.session.id().then(id => collectAnswers($, id)).catch(() => undefined))
    // A chart or table with a Metabase or Datadog source refreshes while its pane is open.
    every(15_000, () => void $.session.id().then(id => refreshSources($, id)).catch(() => undefined))
    every(5_000, () => void processRequests($, () => void refreshCi($, true).catch(() => undefined)))
    const beat = async () => {
      const current = await read($, work)
      await heartbeat($, await hereSoon($), current ? { repo: current.repo, branch: await branchOf($, current) } : undefined)
    }
    every(60_000, () => void beat().catch(() => undefined))
    void beat().catch(() => undefined)
    every(3600_000, () => void checkUpdate($).catch(() => undefined))
    void checkUpdate($).catch(() => undefined)
    void refreshCi($).catch(() => undefined)
    void refresh($, false).catch(() => undefined)
    $.ui.status(undefined)
    void refreshLinks($).catch(() => undefined)
    void reportSide($).catch(() => undefined)
    // Draw the agent row now, not after the next turn. A resumed session has no
    // figure until its first response, so the window is estimated locally until then.
    void (async () => {
      const u = await $.session.usage()
      const percent = u.context.percent ?? (await $.session.usage({ breakdown: 'summary' })).context.breakdown?.percentage
      await reportUsage($, { ...u, context: { ...u.context, percent }, changed: [] })
    })().catch(() => undefined)

    return next(e)
  })

  // The to-do list is ~/.config/cockpit/todo.md, Markdown checkboxes the herdr to-do app also edits.
  on('command.run', { command: 'todo' }, async ($, e) => {
    const path = `${await home($)}/.config/cockpit/todo.md`
    const body = await $.fs.read(path).catch(() => '# To-do\n\n')
    const items = body.split('\n').filter(l => /^- \[( |x)\] /.test(l))
    const text = e.args.trim()
    if (!text) {
      const open = items.filter(l => l.startsWith('- [ ] ')).map(l => l.slice(6))
      return { text: open.length > 0 ? open.map(t => `• ${t}`).join('\n') : 'Nothing on the to-do list.' }
    }
    await $.fs.write(path, ['# To-do', '', `- [ ] ${text}`, ...items, ''].join('\n'))
    return { text: `Added to the to-do list: ${text}` }
  })

  // /runbook is the plugin's markdown command (commands/runbook.md), a prompt the engine runs itself.
  // This hook only records which session ran it, the one the runbook app hands steps to.
  on('command.run', { command: ['runbook', 'cockpit:runbook'] }, async ($, e, next) => {
    const target = e.args.trim() || (await read($, ticket)) || ''
    const here = await herdrHere($).catch(() => undefined)
    if (/^[A-Za-z]+-\d+$/.test(target) && here) await recordRunbookOwner($, target, here.pane)
    return next(e)
  })
    .catch(($, e, next) => next(e))

  on('tool.call', { tool: REPORT_TOOL_FULL }, async ($, e) => reportStep($, e as unknown as Record<string, unknown>))
  // It writes cockpit's own files and one runbook status cell, so it needs no permission prompt.
  on('tool.check', { tool: REPORT_TOOL_FULL }, async () => ({ decision: 'allow' }))
  on('tool.call', { tool: SHOW_TOOL_FULL }, async ($, e) => {
    const answer = await show($, e as unknown as Record<string, unknown>, await herdrHere($), await $.session.cwd())
    await refreshViews($).catch(() => undefined)
    void reportSide($).catch(() => undefined)
    return answer
  })
  // It writes the view to cockpit's cache and splits a pane beside this session; a view that runs commands asks first.
  on('tool.check', { tool: SHOW_TOOL_FULL }, async ($, e) => {
    const commands = commandsOf((e.input ?? {}) as Record<string, unknown>)
    return commands.length > 0 ? { decision: 'ask', reason: `The pane will run, again and again: ${commands.join(' ; ')}` } : { decision: 'allow' }
  })

  on('tool.call', async ($, e, next) => {
    await trackWork($, e as unknown as Record<string, unknown>).catch(() => undefined)
    const done = activityOf(String(e.tool), e as unknown as Record<string, unknown>, (await read($, work))?.dir ?? '')
    if (done && e.agentId === undefined) void hereSoon($).then(here => logActivity($, here?.space, done)).catch(() => undefined)
    return next(e)
  })

  on('turn.complete', async ($, e, next) => {
    if (e.agentId === undefined) void refreshLinks($).catch(() => undefined)
    if (e.agentId === undefined && e.reason === 'answer') void notifyFinished($, e.durationMs, e.answer).catch(() => undefined)
    if (e.agentId === undefined && e.reason === 'answer') void settleTurn($).catch(() => undefined)
    if (e.agentId === undefined && e.reason === 'answer') {
      const first = e.answer.split('\n').map(l => l.replace(/[*`#>_]/g, '').trim()).find(l => l.length > 0) ?? ''
      void hereSoon($).then(here => logActivity($, here?.space, { kind: 'turn', text: first.slice(0, 200) })).catch(() => undefined)
    }
    return next(e)
  })

  on('prompt.submit', async ($, e, next) => {
    void refreshLinks($, e.text).catch(() => undefined)
    void hereSoon($).then(here => bindHandoff($, e.text, here?.pane, e.origin?.kind === 'plugin')).catch(() => undefined)
    const page = e.command || e.origin?.kind !== 'composer' ? [] : await pageContext($)
    return next(page.length > 0 ? { ...e, context: [...(e.context ?? []), ...page] } : e)
  })

  on('ui.render', { component: 'SessionMode' }, async ($, e) => renderFooter($, e))

  on('session.measure', async ($, e, next) => {
    await reportUsage($, e).catch(() => undefined)
    return next(e)
  })

  on('session.end', async ($, e, next) => {
    stopTimers()
    if (e.reason !== 'clear') await forget($).catch(() => undefined)
    // A /clear keeps the process: what the old conversation worked on must not follow the new one.
    if (e.reason === 'clear') {
      await update($, work, () => null)
      await update($, usage, () => null)
      await update($, ticket, () => null)
      await update($, links, () => null)
      await update($, views, () => [])
      await update($, sidePage, () => null)
    }
    const here = await herdrHere($).catch(() => undefined)
    if (here) void clearRows($, here.pane)
    return next(e)
  })
}
