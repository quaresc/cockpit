import type { EngineInterface } from 'claude-code'

import type { CiCapability } from './ci'
import type { CalendarCapability } from './meetings'
import type { Capability } from './platform'
import type { TicketCapability } from './tickets'

type $ = EngineInterface

// A platform tool for one capability, its arguments with "{name}" placeholders, and the answer's fields to send back
// as they were (`keep`, for a tool that replaces a whole setup).
export type Recipe = { tool: string; args?: Record<string, unknown>; keep?: string[] }

// Where each value cockpit reads sits in the platform tools' answers, as dotted paths ("status.health"; "a|b" for
// the first that has one).
export type PlatformFields = Partial<Record<
  | 'apps' | 'appId' | 'appRepo' | 'appLink' | 'appTag' | 'appSha' | 'appKind'
  | 'deployments' | 'deployWorkflow' | 'deployNamespace' | 'deployTag' | 'deployBranch'
  | 'previews' | 'previewId' | 'previewExpires' | 'previewActive' | 'previewHealth' | 'previewApps'
  | 'appName' | 'appEnabled' | 'appBranch' | 'statusApps' | 'statusUrl' | 'createdId' | 'healthy',
  string
>> & { skipKinds?: string[] }

// The connector holding my tickets, mapped like the platform: a tool per capability, where each value sits in the answers,
// and how its status categories read as Linear's (`statusTypes`).
export type TicketFields = Partial<Record<
  | 'tickets' | 'id' | 'title' | 'url' | 'status' | 'statusType' | 'parentId' | 'project'
  | 'description' | 'attachments' | 'attachmentUrl' | 'states' | 'stateId' | 'stateName' | 'stateType',
  string
>>

export type Tickets = {
  enabled: boolean
  // What the inbox and notifications call the tool: Linear, Jira…
  name: string
  server: string
  // Key prefixes a ticket mention may use, on top of my tickets' own.
  keys: string[]
  // A ticket's page ({id}); empty, it follows the URL of an assigned ticket.
  urlTemplate: string
  tools: Partial<Record<TicketCapability, Recipe>>
  fields: TicketFields
  statusTypes: Record<string, string>
}

// The CI behind my PRs: GitHub's checks when no connector is named, else a CI connector mapped like the platform.
export type CiFields = Partial<Record<
  | 'runs' | 'runId' | 'runRepo' | 'runBranch' | 'runTag' | 'runCreated' | 'runPhase' | 'runOutcome'
  | 'workflows' | 'workflowId' | 'workflowName' | 'workflowCreated' | 'workflowOutcome'
  | 'jobs' | 'jobId' | 'jobName' | 'jobPhase' | 'jobOutcome'
  | 'steps' | 'stepName' | 'stepOutcome' | 'stepExit' | 'tests' | 'testName',
  string
>>

export type Ci = {
  enabled: boolean
  // What notifications and prompts call it: GitHub, CircleCI, GitLab…
  name: string
  // The CI connector; empty reads GitHub's checks through gh.
  server: string
  // The job or check that builds the image a preview runs; enables `preview` and `redeploy`.
  imageJob: string
  tools: Partial<Record<CiCapability, Recipe>>
  fields: CiFields
  // How the connector's outcomes read as cockpit's (succeeded, failed, canceled), and the phases that mean ended.
  outcomes: Record<string, string>
  endedPhases: string[]
}

// The calendar connector, mapped like the others; Google Calendar's mapping is the default.
export type CalendarFields = Partial<Record<
  | 'events' | 'nextPage' | 'id' | 'title' | 'start' | 'end' | 'url' | 'join' | 'location' | 'description' | 'series' | 'myResponse'
  | 'attendees' | 'attendeeSelf' | 'attendeeResponse' | 'attendeeResource' | 'attendeeName' | 'attachments' | 'attachmentTitle' | 'attachmentUrl',
  string
>>

export type Calendar = {
  enabled: boolean
  name: string
  server: string
  tools: Partial<Record<CalendarCapability, Recipe>>
  fields: CalendarFields
  // Events left out when a field holds one of these values (cancelled, free), and kept only when a field holds one of
  // those (regular meetings, not out of office or focus time); a missing field reads as "".
  skip: Record<string, string[]>
  only: Record<string, string[]>
  // How the connector's answers to an invite read as cockpit's: accepted, tentative, declined, needsAction.
  responses: Record<string, string>
}

export type Platform = {
  enabled: boolean
  server: string
  // How a preview's id starts, to spot one named in a conversation.
  previewPrefix: string
  // The page following a preview's rollout ({preview}), and a deploy run's ({namespace}, {name}).
  previewDashboardUrl: string
  workflowUrl: string
  previewDefaults: Record<string, unknown>
  tools: Partial<Record<Capability, Recipe>>
  fields: PlatformFields
}

// ~/.config/cockpit/config.json, written by /cockpit:setup and shared with the herdr apps.
export type Config = {
  reposDir: string
  github: { enabled: boolean; org: string }
  tickets: Tickets
  ci: Ci
  // The connector whose tools deploy apps and run preview environments, mapped by /cockpit:setup.
  platform: Platform
  deployWatch: { enabled: boolean; afterMinutes: number; tools: string[] }
  recap: { enabled: boolean; awayMinutes: number }
  meetingBrief: { enabled: boolean; model: string; skip: string[]; tools: string[] }
  calendar: Calendar
  // Connectors that show panes refresh from.
  metabase: { server: string }
  datadog: { server: string }
  brief: { enabled: boolean; at: string; weekends: boolean }
  notifications: boolean
  // Where an agent checks a front change: the terminal-browser pane beside it, or Claude in Chrome.
  checksBrowser: 'terminal' | 'chrome'
  // A browser beside an agent whose space stayed out of view this long closes, to stop it drawing; 0 keeps it.
  browserIdleMinutes: number
  herdrPluginRoot: string
}

// The Linear connector's mapping, the default for a config that names no other.
const LINEAR: Tickets = {
  enabled: false,
  name: 'Linear',
  server: 'claude.ai Linear',
  keys: [],
  urlTemplate: '',
  tools: {
    listMine: { tool: 'list_issues', args: { assignee: 'me', limit: 100, fields: ['id', 'title', 'url', 'status', 'statusType', 'parentId', 'project'] } },
    get: { tool: 'get_issue', args: { id: '{ticket}', fields: ['id', 'title', 'url', 'description', 'attachments'] } },
    comments: { tool: 'list_comments', args: { issueId: '{ticket}' } },
    listStates: { tool: 'list_issue_statuses', args: { team: '{team}' } },
    setState: { tool: 'save_issue', args: { id: '{ticket}', state: '{state}' } },
  },
  fields: {
    tickets: 'issues', id: 'id', title: 'title', url: 'url', status: 'status', statusType: 'statusType', parentId: 'parentId', project: 'project',
    description: 'description', attachments: 'attachments', attachmentUrl: 'url', states: '', stateId: 'id', stateName: 'name', stateType: 'type',
  },
  statusTypes: {},
}

// The CircleCI connector's mapping, for a config from before CI connectors were mapped.
const CIRCLECI: Ci = {
  enabled: true,
  name: 'CircleCI',
  server: 'circleci',
  imageJob: '',
  tools: {
    listRuns: { tool: 'list_runs', args: { mine: true, limit: 30 } },
    listWorkflows: { tool: 'list_run_workflows', args: { run: '{run}' } },
    listJobs: { tool: 'list_workflow_jobs', args: { workflow: '{workflow}' } },
    getJob: { tool: 'get_job', args: { job: '{job}' } },
    listTests: { tool: 'list_job_tests', args: { job: '{job}', limit: 3 } },
    logs: { tool: 'get_job_logs', args: { job: '{job}' } },
    rerun: { tool: 'rerun_workflow', args: { workflow: '{workflow}', from_failed: true } },
  },
  fields: {
    runs: 'runs', runId: 'id', runRepo: 'references.event.attributes.vcs.origin_repository_url', runBranch: 'references.event.attributes.vcs.branch',
    runTag: 'references.event.attributes.vcs.tag', runCreated: 'attributes.created_at', runPhase: 'attributes.phase', runOutcome: 'attributes.current_outcome',
    workflows: 'workflows', workflowId: 'id', workflowName: 'attributes.name', workflowCreated: 'attributes.created_at', workflowOutcome: 'attributes.outcome',
    jobs: 'jobs', jobId: 'id', jobName: 'attributes.name', jobPhase: 'attributes.phase', jobOutcome: 'attributes.outcome',
    steps: 'attributes.parallel_executions[].steps', stepName: 'name', stepOutcome: 'outcome', stepExit: 'exit_code', tests: 'tests', testName: 'name|classname',
  },
  outcomes: {},
  endedPhases: ['ended'],
}

// The Google Calendar connector's mapping, the default for a config that names no other.
const GOOGLE_CALENDAR: Calendar = {
  enabled: false,
  name: 'Google Calendar',
  server: 'claude.ai Google Calendar',
  tools: {
    listEvents: { tool: 'list_events', args: { orderBy: 'startTime', pageSize: 12, pageToken: '{pageToken}' } },
    getEvent: { tool: 'get_event', args: { eventId: '{event}' } },
  },
  fields: {
    events: 'events', nextPage: 'nextPageToken', id: 'id', title: 'summary', start: 'start.dateTime', end: 'end.dateTime', url: 'htmlLink',
    join: 'conferenceUrl|hangoutLink', location: 'location', description: 'description', series: 'recurringEventId',
    attendees: 'attendees', attendeeSelf: 'self', attendeeResponse: 'responseStatus', attendeeResource: 'resource', attendeeName: 'displayName|email',
    attachments: 'attachments', attachmentTitle: 'title', attachmentUrl: 'fileUrl',
  },
  skip: { status: ['cancelled'], transparency: ['transparent'] },
  only: { eventType: ['', 'DEFAULT'] },
  responses: {},
}

const DEFAULTS: Config = {
  reposDir: '~',
  github: { enabled: true, org: '' },
  tickets: LINEAR,
  ci: { enabled: true, name: 'GitHub', server: '', imageJob: '', tools: {}, fields: {}, outcomes: {}, endedPhases: [] },
  platform: { enabled: false, server: '', previewPrefix: '', previewDashboardUrl: '', workflowUrl: '', previewDefaults: {}, tools: {}, fields: {} },
  deployWatch: { enabled: false, afterMinutes: 30, tools: [] },
  recap: { enabled: true, awayMinutes: 10 },
  meetingBrief: { enabled: true, model: 'sonnet', skip: ['lunch', 'focus', 'break', 'wday', 'out of office'], tools: [] },
  calendar: GOOGLE_CALENDAR,
  metabase: { server: 'metabase' },
  datadog: { server: 'claude.ai Datadog' },
  brief: { enabled: true, at: '08:30', weekends: false },
  notifications: true,
  checksBrowser: 'terminal',
  browserIdleMinutes: 60,
  herdrPluginRoot: '',
}

let cached: { at: number; config: Config } | undefined

// After a write, the next read takes the file again instead of the 30 s copy.
export function forgetConfig() {
  cached = undefined
}

export async function home($: $) {
  return (await $.env.get('HOME')) ?? ''
}

export async function cacheDir($: $) {
  return `${await home($)}/.cache/cockpit`
}

// Read at most every 30 s, so a setup change reaches running sessions without a restart.
export async function loadConfig($: $): Promise<Config> {
  if (cached && Date.now() - cached.at < 30_000) return cached.config
  let raw: Partial<Config> & {
    linear?: { enabled?: boolean; server?: string; teams?: string[] }
    circleci?: { enabled?: boolean; server?: string; imageJob?: string }
  } = {}
  try {
    raw = JSON.parse(await $.fs.read(`${await home($)}/.config/cockpit/config.json`)) as typeof raw
  } catch {
    // Not set up yet: defaults only.
  }
  // A config from before ticket connectors were mapped has a `linear` block: Linear's mapping with its values.
  if (!raw.tickets && raw.linear) raw.tickets = { ...LINEAR, enabled: raw.linear.enabled ?? false, server: raw.linear.server ?? LINEAR.server, keys: raw.linear.teams ?? [] }
  // Likewise a `circleci` block: CircleCI's mapping when it is on, else GitHub's checks.
  if (!raw.ci && raw.circleci) {
    const { enabled, server, imageJob = '' } = raw.circleci
    raw.ci = enabled ? { ...CIRCLECI, server: server ?? CIRCLECI.server, imageJob } : { ...DEFAULTS.ci, imageJob }
  }
  // A CircleCI connector named without a mapping gets CircleCI's.
  if (raw.ci?.server && !raw.ci.tools && /circleci/i.test(raw.ci.server)) raw.ci = { ...CIRCLECI, ...raw.ci }
  const config: Config = {
    ...DEFAULTS,
    ...raw,
    github: { ...DEFAULTS.github, ...raw.github },
    tickets: { ...DEFAULTS.tickets, ...raw.tickets },
    ci: { ...DEFAULTS.ci, ...raw.ci },
    platform: { ...DEFAULTS.platform, ...raw.platform },
    calendar: { ...DEFAULTS.calendar, ...raw.calendar },
    metabase: { ...DEFAULTS.metabase, ...raw.metabase },
    datadog: { ...DEFAULTS.datadog, ...raw.datadog },
    brief: { ...DEFAULTS.brief, ...raw.brief },
    deployWatch: { ...DEFAULTS.deployWatch, ...raw.deployWatch },
    recap: { ...DEFAULTS.recap, ...raw.recap },
    meetingBrief: { ...DEFAULTS.meetingBrief, ...raw.meetingBrief },
  }
  const dir = await home($)
  const expand = (path: string) => path.replace(/^~(?=\/|$)/, dir).replace(/\/$/, '')
  config.reposDir = expand(config.reposDir)
  config.herdrPluginRoot = expand(config.herdrPluginRoot) || (await herdrPluginRoot($))
  cached = { at: Date.now(), config }
  return config
}

// Where herdr keeps the cockpit herdr plugin, as its plugin registry says.
async function herdrPluginRoot($: $) {
  try {
    const plugins = JSON.parse(await $.fs.read(`${await home($)}/.config/herdr/plugins.json`)) as { plugin_id: string; plugin_root: string }[]
    return plugins.find(p => p.plugin_id === 'cockpit')?.plugin_root ?? ''
  } catch {
    return ''
  }
}

// Fills `{name}` placeholders of a URL template; an empty template gives no URL.
export function fill(template: string, values: Record<string, string>) {
  return template ? template.replace(/\{(\w+)\}/g, (_, key: string) => encodeURIComponent(values[key] ?? '')) : undefined
}
