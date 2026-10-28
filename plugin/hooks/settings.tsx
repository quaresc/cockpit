import { atom, read, update } from 'claude-code'
import type { EngineInterface, On } from 'claude-code'

import { forgetConfig, home, loadConfig } from './config'
import type { Config } from './config'

type $ = EngineInterface

// /cockpit: a pane of the settings worth flipping day to day, written to ~/.config/cockpit/config.json.
// `/cockpit e2e chrome` and the like set one without opening it.
const PANE = 'cockpit-settings'
const shown = atom({ plugin: 'cockpit', key: 'settings' } as const, null)

type Setting = {
  key: string
  path: string[]
  label: string
  values: readonly (string | boolean | number)[]
  names: Record<string, string>
}

const SETTINGS: Setting[] = [
  { key: 'e2e', path: ['e2eBrowser'], label: 'E2E tests run in', values: ['terminal-browser', 'chrome'], names: { 'terminal-browser': 'terminal-browser beside the agent', chrome: 'Claude in Chrome' } },
  { key: 'idle', path: ['browserIdleMinutes'], label: 'Close browsers out of view', values: [60, 180, 0], names: { 60: 'after 1 h', 180: 'after 3 h', 0: 'never' } },
  { key: 'notifications', path: ['notifications'], label: 'Notifications', values: [true, false], names: { true: 'on', false: 'off' } },
  { key: 'brief', path: ['brief', 'enabled'], label: 'Morning brief', values: [true, false], names: { true: 'on', false: 'off' } },
  { key: 'recap', path: ['recap', 'enabled'], label: 'Recap when back in a space', values: [true, false], names: { true: 'on', false: 'off' } },
  { key: 'deploys', path: ['deployWatch', 'enabled'], label: 'Check errors after a deploy', values: [true, false], names: { true: 'on', false: 'off' } },
  { key: 'meetings', path: ['meetingBrief', 'enabled'], label: 'Meeting briefs', values: [true, false], names: { true: 'on', false: 'off' } },
  { key: 'density', path: ['density'], label: 'Density', values: ['comfortable', 'compact'], names: { comfortable: 'comfortable: a row\'s actions in a panel under the list', compact: 'compact: dense rows, their buttons on the row' } },
]

const ALIASES: Record<string, string | boolean | number> = {
  on: true, off: false, true: true, false: false, terminal: 'terminal', chrome: 'chrome', '1h': 60, '3h': 180, never: 0, comfortable: 'comfortable', compact: 'compact',
}

function valueOf(config: Config, setting: Setting) {
  return setting.path.reduce<unknown>((node, key) => (node as Record<string, unknown> | undefined)?.[key], config)
}

async function configPath($: $) {
  return `${await home($)}/.config/cockpit/config.json`
}

// Writes one value into the file as it is, keeping every other key.
async function write($: $, setting: Setting, value: string | boolean | number) {
  const path = await configPath($)
  const raw = JSON.parse(await $.fs.read(path).catch(() => '{}')) as Record<string, unknown>
  let node = raw
  for (const key of setting.path.slice(0, -1)) node = (node[key] ??= {}) as Record<string, unknown>
  node[setting.path.at(-1)!] = value
  await $.fs.write(path, `${JSON.stringify(raw, null, 2)}\n`)
  forgetConfig()
  // The space apps stop, and their loops start them again in the new density; an open show pane keeps its own until it reopens.
  if (setting.key === 'density') await $.process.run(['pkill', '-f', 'cockpit (inbox|todo|runbook|meetings) cockpit-space']).catch(() => undefined)
  await update($, shown, () => Date.now())
}

async function toggle($: $, setting: Setting) {
  const current = valueOf(await loadConfig($), setting)
  const next = setting.values[(setting.values.indexOf(current as string | boolean | number) + 1) % setting.values.length]!
  await write($, setting, next)
}

async function terminalBrowserVersion($: $) {
  const done = await $.process.run(['sh', '-c', 'PATH="/opt/homebrew/bin:/usr/local/bin:$HOME/.local/bin:$PATH" terminal-browser --version'], { timeoutMs: 5_000 }).catch(() => undefined)
  return done?.exitCode === 0 ? done.stdout.trim().split(/\s+/).at(-1) : undefined
}

// Whether the apps are there: the release's binary in bin/, or a dev build.
async function hasApps($: $) {
  const root = (await loadConfig($)).herdrPluginRoot
  for (const dir of ['bin', 'tui/target/release']) if (await $.fs.exists(`${root}/${dir}/cockpit`)) return true
  return false
}

let version: string | null | undefined
let installed = true

// Called from the mod's session.start: one hook per event and matcher.
export async function registerSettingsCommand($: $) {
  await $.command.register({ name: 'cockpit', description: 'Cockpit settings; `/cockpit e2e chrome` and the like set one directly', argumentHint: '[setting value]' })
}

export function registerSettings(on: On) {
  on('command.run', { command: 'cockpit' }, async ($, e) => {
    const [name, word] = e.args.trim().toLowerCase().split(/\s+/)
    if (!name) {
      version = (await terminalBrowserVersion($)) ?? null
      installed = await hasApps($)
      await $.ui.open({ id: PANE, title: 'Cockpit' })
      return { text: 'Cockpit settings opened.' }
    }
    const setting = SETTINGS.find(s => s.key === name)
    const value = word === undefined ? undefined : ALIASES[word]
    if (!setting || value === undefined || !setting.values.includes(value)) {
      const usage = SETTINGS.map(s => `${s.key} ${s.values.map(v => Object.entries(ALIASES).find(([, a]) => a === v)?.[0] ?? String(v)).join('|')}`)
      return { text: `Usage: /cockpit [setting value]\n${usage.join('\n')}` }
    }
    await write($, setting, value)
    return { text: `${setting.label}: ${setting.names[String(value)]}.` }
  })

  on('ui.render', { component: 'Pane', requestId: PANE }, async ($, e) => {
    const { Box, Text, Button } = $.ui.resolve(e)
    await read($, shown)
    const config = await loadConfig($)
    const width = Math.max(...SETTINGS.map(s => s.label.length)) + 2
    const missing = version === null
    return (
      <Box flexDirection="column">
        <Text bold>Cockpit settings</Text>
        <Text> </Text>
        {SETTINGS.map(s => {
          const value = valueOf(config, s)
          const greyed = missing && ['e2e', 'idle'].includes(s.key)
          return (
            <Box key={s.key} flexDirection="row">
              <Text dimColor={greyed}>{s.label.padEnd(width)}</Text>
              <Button key={`set-${s.key}`} label={s.names[String(value)] ?? String(value)} onPress={() => toggle($, s)} />
            </Box>
          )
        })}
        <Text> </Text>
        <Text dimColor>
          {missing
            ? 'terminal-browser is not installed: E2E tests run in Claude in Chrome.'
            : `terminal-browser ${version ?? ''} · press a value to change it · /cockpit <setting> <value> from any session`}
        </Text>
        {installed ? null : <Text color="yellow">{`The space apps are not installed for this machine yet: sh ${config.herdrPluginRoot}/fetch-apps.sh`}</Text>}
      </Box>
    )
  })
}
