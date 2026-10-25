import type { EngineInterface } from 'claude-code'

import type { Inbox, TicketLinks } from '../types'

import { cacheDir, fill, loadConfig } from './config'
import { hasTicket } from './match'
import { appFor, deployedApps, latestDeploy, previewUrl } from './platform'
import type { Deployment } from './platform'
import { citedIn } from './sites'
import { ticketDetail } from './tickets'

type $ = EngineInterface

const LINKS_TTL_MS = 15 * 60_000

// The ticket's page, its PR, and the first page of each site it cites.
export async function linksFor($: $, id: string, inbox: Inbox | null): Promise<TicketLinks | null> {
  const path = `${await cacheDir($)}/tickets/${id}.json`
  try {
    const cached = JSON.parse(await $.fs.read(path)) as TicketLinks
    if (Date.now() - cached.at < LINKS_TTL_MS) return cached
  } catch {
    // Not cached yet.
  }
  const issue = await ticketDetail($, id).catch(() => null)
  if (!issue) return null
  const attached = issue.attachments
  const texts = [issue.description, ...attached]

  const open = inbox?.mine.find(pr => hasTicket(pr.branch, id))
  const merged = inbox?.merged?.find(pr => hasTicket(pr.branch, id))
  const attachedPr = attached.find(url => /github\.com\/[^/]+\/[^/]+\/pull\/\d+/.test(url))
  const pr = open
    ? { url: open.url, number: open.number }
    : attachedPr
      ? { url: attachedPr, number: Number(attachedPr.split('/').pop()) }
      : merged
        ? { url: merged.url, number: merged.number }
        : undefined

  const links: TicketLinks = {
    id: issue.id,
    title: issue.title,
    url: issue.url,
    pr,
    cited: citedIn(texts, 'first'),
    at: Date.now(),
  }
  await $.fs.write(path, JSON.stringify(links))
  return links
}

// ---------------------------------------------------------------------------
// Deploy dashboards: a repo's preview or production app, and its latest deploy run.

async function workflowLink($: $, latest: Deployment) {
  const url = fill((await loadConfig($)).platform.workflowUrl, { namespace: latest.namespace, name: latest.workflow })
  return url ? { url, label: latest.tag ?? latest.branch ?? 'deploy' } : undefined
}

// Each repo's production dashboard and latest deploy run, for the inbox app to link.
export async function deployLinksFor($: $, repos: string[]) {
  const apps = await deployedApps($)
  const out: Record<string, { dashboard?: string; workflow?: { url: string; label: string } }> = {}
  // One deploy history call per repo, side by side: in a row they could outlast the inbox lock.
  await Promise.all([...new Set(repos)].map(async repo => {
    const app = appFor(apps, repo)
    if (!app) return
    const latest = await latestDeploy($, app.id).catch(() => null)
    out[repo] = { dashboard: app.link, workflow: latest ? await workflowLink($, latest) : undefined }
  }))
  return out
}

// The dashboards of the repo a session works on: its preview's when one runs it (and the app there), else production's
// with its latest deploy run.
export async function dashboardFor($: $, repo: string | undefined, branch: string | undefined, text: string, inbox: Inbox | null) {
  const { previewPrefix, previewDashboardUrl } = (await loadConfig($)).platform
  const prefix = previewPrefix.replace(/[^a-z0-9-]/gi, '')
  const mentioned = prefix ? [...text.matchAll(new RegExp(`\\b${prefix}[a-z0-9]+(?:-[a-z0-9]+)*\\b`, 'g'))].map(m => m[0]).at(-1) : undefined
  const envs = inbox?.previews ?? []
  const env = envs.find(e => e.app === repo && e.branch === branch) ?? envs.find(e => e.app === repo)
  const preview = mentioned ?? env?.id
  const out: { dashboard?: { url: string; label: string }; workflow?: { url: string; label: string }; preview?: { url: string; label: string } } = {}
  const dashboardUrl = preview ? fill(previewDashboardUrl, { preview }) : undefined
  if (preview && dashboardUrl) out.dashboard = { url: dashboardUrl, label: prefix ? preview.replace(prefix, '') : preview }
  const appUrl = env && repo ? await previewUrl($, env.id, repo).catch(() => undefined) : undefined
  if (appUrl) out.preview = { url: appUrl, label: 'preview' }
  if (!repo) return out
  const app = appFor(await deployedApps($), repo)
  if (!app) return out
  if (!out.dashboard && app.link) out.dashboard = { url: app.link, label: 'prod' }
  const latest = await latestDeploy($, app.id).catch(() => null)
  if (latest) out.workflow = await workflowLink($, latest)
  return out
}
