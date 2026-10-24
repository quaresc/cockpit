import type { EngineInterface } from 'claude-code'

import { loadConfig } from './config'

type $ = EngineInterface

let platform: string | undefined

export async function openUrl($: $, url: string) {
  platform ??= (await $.process.run(['uname'])).stdout.trim()
  return $.process.run([platform === 'Darwin' ? 'open' : 'xdg-open', url])
}

// A page beside the conversation, in terminal-browser right of this session's pane: an agent's browser views. browse.sh
// falls back to the default browser without terminal-browser. `size` is the share of the pane a new split takes.
export async function browse($: $, url: string, options: { size?: number; reload?: boolean } = {}) {
  const { herdrPluginRoot } = await loadConfig($)
  if (!herdrPluginRoot) return openUrl($, url)
  const flags = [...(options.size ? ['--size', String(options.size)] : []), ...(options.reload ? ['--reload'] : [])]
  return $.process.run(['sh', `${herdrPluginRoot}/browse.sh`, ...flags, url, await $.session.id()], { timeoutMs: 20_000 })
}
