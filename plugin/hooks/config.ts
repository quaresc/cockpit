import type { EngineInterface } from 'claude-code'

type $ = EngineInterface

// ~/.config/cockpit/config.json, written by /cockpit:setup and shared with the herdr apps.
export type Config = {
  reposDir: string
  github: { enabled: boolean; org: string }
  linear: { enabled: boolean; server: string; teams: string[] }
  circleci: { enabled: boolean; server: string; imageJob: string }
  backstage: {
    enabled: boolean
    server: string
    stagePrefix: string
    sandboxArgoUrl: string
    workflowUrl: string
    sandboxDefaults: Record<string, unknown>
  }
  deployWatch: { enabled: boolean; afterMinutes: number; tools: string[] }
  recap: { enabled: boolean; awayMinutes: number }
  meetingBrief: { enabled: boolean; model: string; skip: string[]; tools: string[] }
  calendar: { enabled: boolean; server: string }
  // Connectors the footer chips read from and that show panes refresh from.
  notion: { server: string }
  metabase: { server: string }
  datadog: { server: string }
  brief: { enabled: boolean; at: string; weekends: boolean }
  notifications: boolean
  // Links open in terminal-browser beside the agent when it is installed; false keeps them in the default browser.
  terminalBrowser: boolean
  // Where an agent checks a front change: the terminal-browser pane beside it, or Claude in Chrome.
  checksBrowser: 'terminal' | 'chrome'
  // A browser beside an agent whose space stayed out of view this long closes, to stop it drawing; 0 keeps it.
  browserIdleMinutes: number
  herdrPluginRoot: string
}

const DEFAULTS: Config = {
  reposDir: '~',
  github: { enabled: true, org: '' },
  linear: { enabled: false, server: 'claude.ai Linear', teams: [] },
  circleci: { enabled: false, server: 'circleci', imageJob: '' },
  backstage: { enabled: false, server: 'backstage', stagePrefix: '', sandboxArgoUrl: '', workflowUrl: '', sandboxDefaults: {} },
  deployWatch: { enabled: false, afterMinutes: 30, tools: [] },
  recap: { enabled: true, awayMinutes: 10 },
  meetingBrief: { enabled: true, model: 'sonnet', skip: ['lunch', 'focus', 'break', 'wday', 'out of office'], tools: [] },
  calendar: { enabled: false, server: 'claude.ai Google Calendar' },
  notion: { server: 'claude.ai Notion' },
  metabase: { server: 'metabase' },
  datadog: { server: 'claude.ai Datadog' },
  brief: { enabled: true, at: '08:30', weekends: false },
  notifications: true,
  terminalBrowser: true,
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
  let raw: Partial<Config> = {}
  try {
    raw = JSON.parse(await $.fs.read(`${await home($)}/.config/cockpit/config.json`)) as Partial<Config>
  } catch {
    // Not set up yet: defaults only.
  }
  const config: Config = {
    ...DEFAULTS,
    ...raw,
    github: { ...DEFAULTS.github, ...raw.github },
    linear: { ...DEFAULTS.linear, ...raw.linear },
    circleci: { ...DEFAULTS.circleci, ...raw.circleci },
    backstage: { ...DEFAULTS.backstage, ...raw.backstage },
    calendar: { ...DEFAULTS.calendar, ...raw.calendar },
    notion: { ...DEFAULTS.notion, ...raw.notion },
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
