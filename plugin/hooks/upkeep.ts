import type { EngineInterface } from 'claude-code'

import { notify, withLock } from './ci'
import { cacheDir, home } from './config'

type $ = EngineInterface

const REPO = 'quaresc/cockpit'
const UPDATE_EVERY_MS = 6 * 3600_000

export async function ownVersion($: $) {
  const manifest = JSON.parse(await $.fs.read(`${$.plugin.root}/.claude-plugin/plugin.json`)) as { version: string }
  return manifest.version
}

// One file per running session, so /cockpit:doctor can tell which sessions run which version, and which run none.
export async function heartbeat($: $, here: { pane: string; space: string } | undefined, work?: { repo: string; branch?: string }) {
  const dir = `${await cacheDir($)}/sessions`
  await $.process.run(['mkdir', '-p', dir])
  const beat = { version: await ownVersion($), at: Date.now(), pane: here?.pane ?? null, space: here?.space ?? null, repo: work?.repo, branch: work?.branch }
  await $.fs.write(`${dir}/${await $.session.id()}.json`, JSON.stringify(beat))
}

export async function forget($: $) {
  await $.process.run(['rm', '-f', `${await cacheDir($)}/sessions/${await $.session.id()}.json`])
}

function isNewer(latest: string, current: string) {
  const a = latest.split('.').map(Number)
  const b = current.split('.').map(Number)
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    if ((a[i] ?? 0) !== (b[i] ?? 0)) return (a[i] ?? 0) > (b[i] ?? 0)
  }
  return false
}

type UpdateState = { at: number; latest?: string; notified?: string }

// Every six hours one session asks GitHub for the published version; a newer one is announced once, its click updating.
export async function checkUpdate($: $) {
  const path = `${await cacheDir($)}/update.json`
  const state = JSON.parse(await $.fs.read(path).catch(() => '{"at":0}')) as UpdateState
  if (Date.now() - state.at < UPDATE_EVERY_MS) return
  await withLock($, 'update', async () => {
    const { exitCode, stdout } = await $.process.run(
      ['gh', 'api', `repos/${REPO}/contents/plugin/.claude-plugin/plugin.json`, '-H', 'Accept: application/vnd.github.raw'],
      { timeoutMs: 20_000 },
    )
    const latest = exitCode === 0 ? (JSON.parse(stdout) as { version?: string }).version : undefined
    const next: UpdateState = { ...state, at: Date.now(), latest }
    const current = await ownVersion($)
    const hasLink = await $.fs.stat(`${await home($)}/.config/cockpit/herdr/update.sh`).then(() => true, () => false)
    if (latest && isNewer(latest, current) && state.notified !== latest) {
      const how = hasLink ? 'Click to update both halves and reload idle sessions.' : 'Run /plugin update cockpit@cockpit.'
      await notify($, `⬆️ Cockpit ${latest}`, `You run ${current}. ${how}`, hasLink ? 'update' : `https://github.com/${REPO}`)
      next.notified = latest
    }
    await $.fs.write(path, JSON.stringify(next))
  })
}
