import { atom, read, update } from 'claude-code'
import type { EngineInterface, RenderInput } from 'claude-code'

import type { TicketLinks } from '../types'

import { openUrl } from './browser'
import { loadConfig } from './config'
import { hereSoon } from './herdr'
import { dashboardFor, linksFor } from './links'
import { ticketKey, ticketsIn } from './match'
import { dashboardName } from './platform'
import { toggleView, viewsOf } from './show'
import { reportSide } from './sidebar'
import { chipsFor, citedIn } from './sites'
import { ticketUrl } from './tickets'
import { ICON } from './icons'
import { branchOf } from './work'

type $ = EngineInterface

const inbox = atom({ plugin: 'cockpit', key: 'inbox' } as const, null)
const links = atom({ plugin: 'cockpit', key: 'links' } as const, null)
const sidePage = atom({ plugin: 'cockpit', key: 'sidePage' } as const, null)
const ticket = atom({ plugin: 'cockpit', key: 'ticket' } as const, null)
const usage = atom({ plugin: 'cockpit', key: 'usage' } as const, null)
const views = atom({ plugin: 'cockpit', key: 'views' } as const, [])
const work = atom({ plugin: 'cockpit', key: 'work' } as const, null)

// The footer under the prompt: chips for what this conversation works on, its views and the page beside it.

const GITHUB_PR = /https:\/\/github\.com\/[^/\s]+\/[^/\s]+\/pull\/\d+/g

const lastMatch = (text: string, pattern: RegExp) => [...text.matchAll(pattern)].at(-1)?.[0]

async function conversationText($: $) {
  return (await $.session.messages())
    .slice(-40)
    .map(m => `${m.text} ${m.toolUses.map(t => `${JSON.stringify(t.input)} ${(t.text ?? '').slice(0, 2000)}`).join(' ')}`)
    .join('\n')
}

// This session's views for the footer, redrawn only when one appears, opens or hides.
export async function refreshViews($: $) {
  const next = await viewsOf($, await $.session.id())
  if (JSON.stringify(next) !== JSON.stringify(await read($, views))) await update($, views, () => next)
}

// What this conversation works on: the ticket it names last (else its branch's), and the links it cites.
export async function refreshLinks($: $, prompt = '') {
  const text = `${await conversationText($)}\n${prompt}`
  const data = await read($, inbox)
  const current = await read($, work)
  const branch = current ? await branchOf($, current) : undefined
  const { tickets } = await loadConfig($)
  const id = tickets.enabled ? (ticketsIn(text, data, tickets.keys).at(-1) ?? (branch ? ticketKey(branch, data, tickets.keys) : undefined)) : undefined
  // Without the ticket itself, its URL comes from the template or an assigned ticket's.
  const guessed = id ? (ticketUrl(tickets, id, data?.tickets) ?? '') : ''
  const fromTicket = id ? ((await linksFor($, id, data).catch(() => null)) ?? { id, title: '', url: guessed, at: 0 }) : null
  const pr = lastMatch(text, GITHUB_PR)
  const found: TicketLinks = {
    id: id ?? '',
    title: fromTicket?.title ?? '',
    url: fromTicket?.url ?? '',
    pr: pr ? { url: pr, number: Number(pr.split('/').pop()) } : fromTicket?.pr,
    // The newest page of each site the conversation names wins over the ticket's own.
    cited: { ...fromTicket?.cited, ...citedIn([text], 'last') },
    at: Date.now(),
  }
  const repo = current?.repo ?? found.pr?.url.match(/github\.com\/[^/]+\/([^/]+)\/pull/)?.[1]
  const prBranch = data?.mine.find(p => p.url === found.pr?.url)?.branch
  Object.assign(found, await dashboardFor($, repo, branch ?? prBranch, text, data).catch(() => ({})))
  const isEmpty = !found.id && Object.keys(found.cited ?? {}).length === 0 && !found.pr && !found.dashboard && !found.preview
  await update($, ticket, () => id ?? null)
  await update($, links, () => (isEmpty ? null : found))
}

// A rounded pill: the caps take the fill colour as their foreground.
const PILL = { main: { fill: '#FFC799', ink: '#101010' }, quiet: { fill: '#2C2C30', ink: '#D2D2D2' } }

// A dashboard link's mark: Argo's for an ArgoCD page, a gauge for any other.
const dashboardIcon = (url: string) => (dashboardName(url) === 'ArgoCD' ? ICON.argo : ICON.dashboard)

export async function renderFooter($: $, e: RenderInput<'SessionMode'>) {
  const { Box, Text, Button } = $.ui.resolve(e)
  const current = await read($, usage)
  const found = await read($, links)
  const shown = await read($, views)
  const beside = await read($, sidePage)
  const tight = (current?.limits ?? []).filter(l => l.percent >= 50)
  const pill = (key: string, label: string, onPress: () => unknown, tone: keyof typeof PILL = 'quiet') => (
    <Button key={key} plain onPress={onPress}>
      <Text color={PILL[tone].fill}>{''}</Text>
      <Text backgroundColor={PILL[tone].fill} color={PILL[tone].ink}>{label}</Text>
      <Text color={PILL[tone].fill}>{''}</Text>
    </Button>
  )
  const chip = (key: string, label: string, url: string, tone: keyof typeof PILL = 'quiet') => pill(key, label, () => openUrl($, url), tone)
  return (
    <Box flexDirection="row" gap={1}>
      {e.props.modes.length > 0 && <Text dimColor>{e.props.modes.join(' & ')}</Text>}
      {found?.id && found.url && chip('lk-ticket', `${ICON.ticket} ${found.id}`, found.url, 'main')}
      {found?.pr && chip('lk-pr', `${ICON.pr} ${found.pr.number}`, found.pr.url)}
      {chipsFor(found?.cited ?? {}).map(site => chip(`lk-${site.kind}`, `${site.icon} ${site.label}`, site.url))}
      {found?.dashboard && chip('lk-dashboard', `${dashboardIcon(found.dashboard.url)} ${found.dashboard.label}`, found.dashboard.url)}
      {found?.workflow && chip('lk-workflow', `${ICON.workflow} ${found.workflow.label}`, found.workflow.url)}
      {found?.preview && chip('lk-preview', `${ICON.app} ${found.preview.label}`, found.preview.url)}
      {beside && pill('lk-external', `${ICON.external} browser`, () => openUrl($, beside.url))}
      {shown.map(v => pill(`lk-view-${v.name}`, `${ICON.view} ${v.name}`, () => hereSoon($).then(here => toggleView($, v.name, here)).then(() => refreshViews($)).then(() => reportSide($)), v.isOpen ? 'main' : 'quiet'))}
      {tight.map(l => (
        <Text key={`lim-${l.name}`} color={l.percent >= 80 ? 'error' : 'warning'}>
          {l.name} {Math.round(l.percent)}%
        </Text>
      ))}
    </Box>
  )
}
