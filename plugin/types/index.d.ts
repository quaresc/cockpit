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

export type Sandbox = { stage: string; product: string; branch: string; health: string; expiresAt: string; activated?: boolean }

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
  figma?: string
  pr?: { url: string; number: number }
  notion?: string
  slack?: string
  argo?: { url: string; label: string }
  workflow?: { url: string; label: string }
  app?: { url: string; label: string }
  at: number
}

export type Usage = { percent?: number; limits: { name: string; percent: number }[] }

export type Inbox = {
  at: number
  direct: Pr[]
  team: Pr[]
  botRepos: string[]
  mine: Pr[]
  tickets: Ticket[]
  errors: string[]
  merged?: MergedPr[]
  deploys?: Record<string, DeployedPr[]>
  sandboxes?: Sandbox[]
  deployLinks?: Record<string, { argo?: string; workflow?: { url: string; label: string } }>
}

export type CiFailure = { job: string; jobId: string; step?: string; tests: string[] }

export type CiRun = {
  repo: string
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

export type CiState = { at: number; runs: CiRun[]; error?: string; version?: number }

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

export type MeetingsState ={ at: number; meetings: Meeting[]; notified: string[]; error?: string }

declare module 'claude-code' {
  interface PluginState {
    cockpit: {
      inbox: Inbox | null
      isBusy: boolean
      expanded: string[]
      note: string
      muted: string[]
      work: Work | null
      usage: Usage | null
      ticket: string | null
      links: TicketLinks | null
      meeting: { label: string; url: string; isSoon: boolean } | null
    }
  }
}
