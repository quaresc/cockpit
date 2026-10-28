export type Pr = {
  repo: string
  number: number
  title: string
  url: string
  author: string
  authorName?: string
  updatedAt: string
  isDraft: boolean
  isDirect: boolean
  review: string | null
  ci: string | null
  branch?: string
  base?: string
  failedChecks?: string[]
}

export type MergedPr = { repo: string; slug: string; url: string; number: number; branch: string; mergeSha: string; mergedAt: string }

export type DeployedPr = { repo: string; number: number; isLive: boolean; tag: string | null }

// A preview environment of mine and the branch one of its apps runs.
export type Preview = { id: string; app: string; branch: string; health: string; expiresAt: string; activated?: boolean }

export type Ticket = {
  id: string
  title: string
  url: string
  status: string
  statusType: string
  parentId?: string | null
  project?: string | null
}

export type TicketLinks = {
  id: string
  title: string
  url: string
  pr?: { url: string; number: number }
  // The page each kind of site it cites, by the kind's name in hooks/sites.ts.
  cited?: Record<string, string>
  dashboard?: { url: string; label: string }
  workflow?: { url: string; label: string }
  preview?: { url: string; label: string }
  at: number
}

export type Usage = { limits: { name: string; percent: number }[] }

// A view the show tool opened, as its footer chip: open in a pane, or hidden and one press away.
export type ViewChip = { name: string; isOpen: boolean }

// What the hub's own feed wrote last: when it tried, when its GitHub and ticket fields last came in, and its errors.
export type HubPart = { at: number; github?: number; tickets?: number; errors: string[] }

export type Inbox = {
  version?: number
  // The last write, by the mod or the hub; `modAt` is the mod's own last fetch.
  at: number
  modAt?: number
  hub?: HubPart
  direct: Pr[]
  team: Pr[]
  botRepos: string[]
  mine: Pr[]
  tickets: Ticket[]
  errors: string[]
  merged?: MergedPr[]
  deploys?: Record<string, DeployedPr[]>
  previews?: Preview[]
  deployLinks?: Record<string, { dashboard?: string; workflow?: { url: string; label: string } }>
}

// The failing job of a run, and how a session reads its logs.
export type CiFailure = { job: string; jobId: string; step?: string; tests: string[]; logs: string }

export type CiRun = {
  repo: string
  // owner/repo, for GitHub runs.
  slug?: string
  branch: string
  runId: string
  createdAt: string
  isRunning: boolean
  outcome: string
  workflowId?: string
  done: number
  total: number
  failed?: CiFailure
  dockerTag?: string
}

// `by: 'hub'` when the hub's feed wrote it.
export type CiState = { at: number; runs: CiRun[]; error?: string; version?: number; by?: 'hub' }

export type Work = { repo: string; dir: string }

export type Meeting = {
  id: string
  title: string
  start: string
  end: string
  url: string
  join?: string
  response: string
  attendees: number
  description?: string
  series?: string
  people?: string[]
  attachments?: { title: string; url: string }[]
  // Whether the meetings app offers a brief: not a daily, not declined, someone else invited, not in meetingBrief.skip.
  briefable?: boolean
}

// A runbook step the runbook app handed to a Claude session, in ~/.cache/cockpit/handoffs/<id>.json.
// sent → running → (waiting → running)* → done | failed | blocked, or unreported when the session never says.
export type Handoff = {
  id: string
  slug: string
  step: string
  title: string
  path: string
  state: 'sent' | 'running' | 'waiting' | 'done' | 'failed' | 'blocked' | 'unreported'
  sentAt: number
  session?: string
  pane?: string
  message?: string
  reportedAt?: number
  isNudged?: boolean
}

// The monitor space's document: each app of the watched environment, and for those with a recent release (or pinned)
// their releases with a before/after verdict, alerting monitors, open errors and a day of errors and p95.
export type Health = { errors: number; hits: number; p95: number | null }
export type Verdict = {
  state: 'watching' | 'ok' | 'moved' | 'failed' | 'unknown'
  summary: string
  before: Health | null
  after: Health | null
  newIssues: string[]
  until: string
}
export type Release = {
  id: number
  tag: string
  actor: string
  isMine: boolean
  phase: string
  startedAt: string
  finishedAt: string | null
  tickets: { id: string; url: string }[]
  prs: { repo: string; number: number; url: string }[]
  verdict: Verdict | null
}
export type Issue = { id: string; type: string; message: string; count: number; firstSeen: string; firstVersion: string; url: string; isNew: boolean }
export type Monitors = { ok: number; warn: number; alert: number; nodata: number; alerting: { id: number; name: string; status: string; url: string }[] }
export type MonitoredApp = {
  id: string
  name: string
  service: string
  repo: string
  health: string
  tag: string
  deployedAt: string
  behind: number
  argoUrl: string
  datadogUrl: string
  watched: boolean
  pinned: boolean
  monitors: Monitors | null
  series: { errors: number[]; p95: number[] } | null
  issues: Issue[]
  releases: Release[]
}
export type MonitorState = { version: 1; at: number; env: string; errors: string[]; apps: MonitoredApp[] }

export type MeetingsState ={ at: number; meetings: Meeting[]; notified: string[]; error?: string }

declare module 'claude-code' {
  interface PluginState {
    cockpit: {
      inbox: Inbox | null
      muted: string[]
      work: Work | null
      usage: Usage | null
      ticket: string | null
      links: TicketLinks | null
      views: ViewChip[]
      settings: number | null
      sidePage: { url: string; title: string } | null
    }
  }
}
