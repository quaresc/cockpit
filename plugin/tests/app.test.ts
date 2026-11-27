import { expect, test } from 'claude-code/testing'

import { forgetFacts, reportFacts, transcriptPath } from '../hooks/facts'
import { exportAppPane, findTerminal, frontendOf, herdrHere } from '../hooks/herdr'
import type { AppSpace } from '../hooks/herdr'
import { isBrowserTerminal } from '../hooks/sidebar'

const HOME = '/home/me'
const ran = (exitCode: number, stdout = '') => ({ exitCode, stdout, stderr: '', isStdoutTruncated: false, isStderrTruncated: false })

const SPACES: AppSpace[] = [
  { id: 'sp1', name: 'api', folder: '/repos/api', machine: null, active: true, tabs: [
    { id: 'tb1', name: 'quotas', active: true, terminals: [
      { terminal: 't3', pane: 'p3', status: 'idle', name: null, app: 'claude-code' },
      { terminal: 't4', pane: 'p4', status: null, name: 'terminal-browser', app: null },
    ] },
  ] },
]

// Just what the frontend and the facts reach for: the environment, herdr-app, cockpit store and the plugin manifest.
function fake(env: Record<string, string>, ancestry = 1) {
  const patches: Record<string, unknown>[] = []
  const exported: Record<string, string | undefined> = {}
  const files = new Map<string, string>([['/plugin/.claude-plugin/plugin.json', '{"version": "0.26.0"}'], [`${HOME}/.config/cockpit/herdr/cockpit.sh`, '']])
  const $ = {
    env: {
      get: async (name: string) => (name === 'HOME' ? HOME : name in exported ? exported[name] : env[name]),
      set: async (name: string, value: string | undefined) => {
        exported[name] = value
        return {}
      },
    },
    plugin: { root: '/plugin' },
    session: { id: async () => 's1', cwd: async () => '/repos/api' },
    fs: {
      read: async (path: string) => {
        const content = files.get(path)
        if (content === undefined) throw new Error('ENOENT')
        return content
      },
      write: async (path: string, body: string) => void files.set(path, body),
      stat: async (path: string) => {
        if (!files.has(path)) throw new Error('ENOENT')
        return { size: 1 }
      },
    },
    process: {
      run: async (argv: string[]) => {
        const line = argv.join(' ')
        if (line.includes('" ls')) return ran(0, JSON.stringify({ spaces: SPACES }))
        if (line.includes('pid=$PPID')) return ran(ancestry)
        if (argv[2] === 'store') {
          patches.push(JSON.parse(files.get(argv[5]!) ?? 'null') as Record<string, unknown>)
          return ran(0, '{}')
        }
        return ran(0)
      },
    },
  } as unknown as Parameters<typeof reportFacts>[0]
  return { $, patches, exported }
}

test('a terminal is found with its pane, tab and space, and a browser terminal by its program', async () => {
  const found = findTerminal(SPACES, 't3')
  expect(found?.space.id).toBe('sp1')
  expect(found?.tab.id).toBe('tb1')
  expect(found?.terminal.pane).toBe('p3')
  expect(findTerminal(SPACES, 't9')).toBe(undefined)
  expect(isBrowserTerminal(SPACES[0]!.tabs[0]!.terminals[1]!)).toBe(true)
  expect(isBrowserTerminal(SPACES[0]!.tabs[0]!.terminals[0]!)).toBe(false)
})

test('a Herdr app terminal is the app, a herdr pane the TUI, and with both the process tree decides', async () => {
  expect(await frontendOf(fake({ HC_TERMINAL_ID: 't3' }).$)).toBe('app')
  expect(await frontendOf(fake({ HERDR_ENV: '1' }).$)).toBe('tui')
  expect(await frontendOf(fake({}).$)).toBe(undefined)
  expect(await frontendOf(fake({ HERDR_ENV: '1', HC_TERMINAL_ID: 't5' }, 0).$)).toBe('app')
  expect(await frontendOf(fake({ HERDR_ENV: '1', HC_TERMINAL_ID: 't6' }, 1).$)).toBe('tui')
})

test('in the app a session sits at app:<terminal>, with the pane to split', async () => {
  const here = await herdrHere(fake({ HC_TERMINAL_ID: 't3' }).$)
  expect(here).toEqual({ pane: 'app:t3', tab: 'tb1', space: 'sp1', frontend: 'app', terminal: 't3', paneId: 'p3' })
})

test('in the app the session exports its pane and the app socket for terminal-browser, elsewhere nothing', async () => {
  const app = fake({ HC_TERMINAL_ID: 't3' })
  await exportAppPane(app.$)
  expect(app.exported).toEqual({ HERDR_PANE_ID: 'p3', HERDR_SOCKET_PATH: `${HOME}/.config/herdr-app/herdr-app.sock` })
  const own = fake({ HC_TERMINAL_ID: 't3', HERDR_APP_HOME: '/tmp/box/herdr-home' })
  await exportAppPane(own.$)
  expect(own.exported.HERDR_SOCKET_PATH).toBe('/tmp/box/herdr-home/herdr-app.sock')
  const tui = fake({ HERDR_ENV: '1', HERDR_PANE_ID: 'w1:p2' })
  await exportAppPane(tui.$)
  expect(tui.exported).toEqual({})
  const gone = fake({ HC_TERMINAL_ID: 't9' })
  await exportAppPane(gone.$)
  expect(gone.exported).toEqual({})
})

test('facts reach the agents document under the handle, once per change, and go with the session', async () => {
  const { $, patches } = fake({ HC_TERMINAL_ID: 't3' })
  const here = { pane: 'app:t3', tab: 'tb1', space: 'sp1', frontend: 'app' as const, terminal: 't3', paneId: 'p3' }
  await reportFacts($, here, { context: 42, branch: 'eng-12-login', page: 'http://localhost:3000/' })
  await reportFacts($, here, { context: 42 })
  expect(patches.length).toBe(1)
  const facts = patches[0]!['app:t3'] as Record<string, unknown>
  expect(facts.session).toBe('s1')
  expect(facts.terminal).toBe('t3')
  expect(facts.context).toBe(42)
  expect(facts.page).toBe('http://localhost:3000/')
  expect(facts.transcript).toBe(transcriptPath(`${HOME}/.claude`, '/repos/api', 's1'))
  expect(facts.configDir).toBe(`${HOME}/.claude`)
  expect(facts.version).toBe('0.26.0')
  // A session whose terminal moved leaves its old row.
  await reportFacts($, { ...here, pane: 'app:t7', terminal: 't7' }, { context: 50 })
  expect(patches[1]!['app:t3']).toBe(null)
  await forgetFacts($, 'app:t7')
  expect(patches[2]).toEqual({ 'app:t7': null })
})

test("a conversation's transcript is in Claude's folder for its working directory", async () => {
  expect(transcriptPath('/home/me/.claude', '/repos/my_api.v2', 'abc')).toBe('/home/me/.claude/projects/-repos-my-api-v2/abc.jsonl')
})

test('a session on another account names its config folder, and its transcript is under it', async () => {
  const { $, patches } = fake({ HC_TERMINAL_ID: 't4', CLAUDE_CONFIG_DIR: '/home/me/.claude-work/' })
  const here = { pane: 'app:t4', tab: 'tb1', space: 'sp1', frontend: 'app' as const, terminal: 't4', paneId: 'p4' }
  await reportFacts($, here, { cwd: '/repos/api' })
  const facts = patches.at(-1)!['app:t4'] as Record<string, unknown>
  expect(facts.configDir).toBe('/home/me/.claude-work')
  expect(facts.transcript).toBe('/home/me/.claude-work/projects/-repos-api/s1.jsonl')
})
