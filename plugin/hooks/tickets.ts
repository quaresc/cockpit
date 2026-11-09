import type { EngineInterface } from 'claude-code'

import type { Ticket } from '../types'

import { fill, loadConfig } from './config'
import type { Tickets } from './config'
import { callMapped, capture, fillArgs, listAt, pick, text } from './platform'

type $ = EngineInterface

// My tickets through whatever MCP tools the ticket connector offers (Linear, Jira, Trello…). The config maps each
// capability to a tool and each value cockpit reads to a path in its answers; /cockpit:setup writes both.

// assign, create and details are the hub's, through a ticket server of its own.
export type TicketCapability = 'listMine' | 'get' | 'comments' | 'listStates' | 'setState' | 'assign' | 'create' | 'details'

// Cockpit's status types are Linear's categories; the open ones, in the order the inbox lists them.
const ACTIVE = ['started', 'unstarted', 'triage', 'backlog']

export async function ticketSystem($: $): Promise<Tickets | undefined> {
  const config = (await loadConfig($)).tickets
  return config.enabled ? config : undefined
}

// Text to search for links: a rich-text description (Jira's, say) is searched as its JSON.
const searchable = (value: unknown) => (typeof value === 'string' ? value : value == null ? '' : JSON.stringify(value))

const statusType = (config: Tickets, value: unknown) => {
  const type = text(value) ?? ''
  return config.statusTypes[type] ?? type
}

// A ticket's page: the URL template's, else an assigned ticket's URL cut where its id is a path segment, with the id swapped.
export function ticketUrl(config: Tickets, ticket: string, known: Ticket[] = []) {
  const id = ticket.toUpperCase()
  const filled = fill(config.urlTemplate, { id })
  if (filled) return filled
  const sample = known.find(t => t.id && t.url.includes(`/${t.id}`))
  return sample ? `${sample.url.slice(0, sample.url.lastIndexOf(`/${sample.id}`) + 1)}${id}` : undefined
}

// A ticket's id as `idTemplate` writes it from the parts `idPattern` reads in its URL, for a tool whose issues are not
// named by one value (GitHub's owner/repo#number).
export function composedId(config: Tickets, url: string) {
  const { idPattern, idTemplate } = config
  if (!idPattern || !idTemplate) return undefined
  const parts = capture(idPattern, url)
  return parts && String(fillArgs(idTemplate, parts))
}

// What a mapped tool's arguments are filled from: the ticket, its team, and the parts `idTemplate` reads in its id.
function values(config: Tickets, ticket: string) {
  const parts = config.idTemplate ? capture(config.idTemplate, ticket) : undefined
  return { ticket, team: ticket.split('-')[0] ?? '', ...parts }
}

// The states a ticket can move to: the ones a tool lists, else the ones the mapping holds itself.
async function ticketStates($: $, config: Tickets, ticket: string) {
  if (!config.tools.listStates?.tool) return config.states ?? []
  const f = config.fields
  return listAt(await callMapped($, config, 'listStates', values(config, ticket)), f.states).map(s => ({
    id: text(pick(s, f.stateId)) ?? '',
    name: text(pick(s, f.stateName)) ?? '',
    type: statusType(config, pick(s, f.stateType)),
  }))
}

// My open tickets, in progress first; null when tickets are off.
export async function myTickets($: $): Promise<Ticket[] | null> {
  const config = await ticketSystem($)
  if (!config) return null
  const f = config.fields
  const tickets = listAt(await callMapped($, config, 'listMine'), f.tickets).map((raw): Ticket => {
    const read = text(pick(raw, f.id)) ?? ''
    const url = text(pick(raw, f.url)) ?? fill(config.urlTemplate, { id: read }) ?? ''
    const id = composedId(config, url) ?? read
    return {
      id,
      title: text(pick(raw, f.title)) ?? '',
      url,
      status: text(pick(raw, f.status)) ?? '',
      statusType: statusType(config, pick(raw, f.statusType)),
      parentId: text(pick(raw, f.parentId)) ?? null,
      project: text(pick(raw, f.project)) ?? null,
    }
  })
  const rank = (t: Ticket) => ACTIVE.indexOf(t.statusType)
  return tickets.filter(t => rank(t) >= 0).sort((a, b) => rank(a) - rank(b))
}

// One ticket with the texts its links hide in: its description and attachments.
export async function ticketDetail($: $, id: string) {
  const config = await ticketSystem($)
  if (!config?.tools.get?.tool) return null
  const f = config.fields
  const raw = await callMapped($, config, 'get', values(config, id))
  return {
    id: text(pick(raw, f.id)) ?? id,
    title: text(pick(raw, f.title)) ?? '',
    url: text(pick(raw, f.url)) ?? ticketUrl(config, id) ?? '',
    description: searchable(pick(raw, f.description)),
    attachments: listAt(raw, f.attachments).map(a => text(pick(a, f.attachmentUrl)) ?? ''),
  }
}

// Moves a ticket to its "Done" state, else the first completed one it can reach, and says which.
export async function completeTicket($: $, ticket: string) {
  const config = await ticketSystem($)
  if (!config) throw new Error('tickets are off: run /cockpit:setup')
  const states = await ticketStates($, config, ticket)
  const done = states.find(s => s.type === 'completed' && /^done$/i.test(s.name)) ?? states.find(s => s.type === 'completed')
  if (!done) throw new Error(`no completed state for ${ticket}`)
  await callMapped($, config, 'setState', { ...values(config, ticket), state: done.id })
  return done.name
}
