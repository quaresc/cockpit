import type { EngineInterface } from 'claude-code'

import type { Inbox, TicketLinks } from '../types'

import { mcp } from './ci'
import { cacheDir, fill, loadConfig } from './config'
import { appFor, deployedApps, latestDeploy, sandboxUrl } from './platform'
import type { Deployment } from './platform'

type $ = EngineInterface

const LINKS_TTL_MS = 15 * 60_000

// A ticket mentioned in a prompt must use a team key from the config or from an assigned ticket.
export function ticketsIn(text: string, inbox: Inbox | null, known: string[]) {
  const teams = new Set([...known.map(t => t.toUpperCase()), ...(inbox?.tickets ?? []).map(t => t.id.split('-')[0] ?? '')])
  return [...text.matchAll(/\b([A-Z][A-Z0-9]{1,9})-(\d{1,6})\b/gi)]
    .map(m => `${(m[1] ?? '').toUpperCase()}-${m[2]}`)
    .filter(id => teams.has(id.split('-')[0] ?? ''))
}

type Issue = {
  id: string
  title: string
  url: string
  description?: string | null
  attachments?: { url?: string | null; title?: string | null }[] | null
}

const firstUrl = (texts: string[], pattern: RegExp) => {
  for (const text of texts) {
    const found = text.match(pattern)?.[0]
    if (found) return found.replace(/[>)\]]+$/, '')
  }
  return undefined
}

// The ticket's Linear page, its first Figma frame, its PR, and any Notion page or Slack thread it cites.
export async function linksFor($: $, id: string, inbox: Inbox | null): Promise<TicketLinks | null> {
  const path = `${await cacheDir($)}/tickets/${id}.json`
  try {
    const cached = JSON.parse(await $.fs.read(path)) as TicketLinks
    if (Date.now() - cached.at < LINKS_TTL_MS) return cached
  } catch {
    // Not cached yet.
  }
  if (!(await loadConfig($)).linear.enabled) return null
  const issue = await mcp<Issue>($, 'linear', 'get_issue', { id, fields: ['id', 'title', 'url', 'description', 'attachments'] }).catch(() => null)
  if (!issue) return null
  const attached = (issue.attachments ?? []).map(a => a.url ?? '')
  const texts = [issue.description ?? '', ...attached]

  const key = id.toLowerCase()
  const open = inbox?.mine.find(pr => pr.branch?.toLowerCase().includes(key))
  const merged = inbox?.merged?.find(pr => pr.branch.toLowerCase().includes(key))
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
    figma: firstUrl(texts, /https:\/\/(?:www\.)?figma\.com\/(?:design|file|proto|board)\/[^\s)>\]]+/),
    pr,
    notion: firstUrl(texts, /https:\/\/(?:www\.)?(?:notion\.so|notion\.site|app\.notion\.com)\/[^\s)>\]]+/),
    slack: firstUrl(texts, /https:\/\/[a-z0-9-]+\.slack\.com\/archives\/[^\s)>\]]+/),
    at: Date.now(),
  }
  await $.fs.write(path, JSON.stringify(links))
  return links
}

// ---------------------------------------------------------------------------
// ArgoCD and Argo Workflows for the repo a session works on: its sandbox when it runs on one,
// else the production app and that app's latest deploy workflow.

async function workflowLink($: $, latest: Deployment) {
  const url = fill((await loadConfig($)).platform.workflowUrl, { namespace: latest.namespace, name: latest.workflow })
  return url ? { url, label: latest.tag ?? latest.branch ?? 'deploy' } : undefined
}

// Each repo's deployed app link (its ArgoCD page) and latest deploy workflow, for the inbox app to link.
export async function deployLinksFor($: $, repos: string[]) {
  const apps = await deployedApps($)
  const out: Record<string, { argo?: string; workflow?: { url: string; label: string } }> = {}
  for (const repo of [...new Set(repos)]) {
    const app = appFor(apps, repo)
    if (!app) continue
    const latest = await latestDeploy($, app.id).catch(() => null)
    out[repo] = {
      argo: app.link,
      workflow: latest ? await workflowLink($, latest) : undefined,
    }
  }
  return out
}

export async function argoFor($: $, repo: string | undefined, branch: string | undefined, text: string, inbox: Inbox | null) {
  const { stagePrefix, sandboxArgoUrl } = (await loadConfig($)).platform
  const prefix = stagePrefix.replace(/[^a-z0-9-]/gi, '')
  const mentioned = prefix ? [...text.matchAll(new RegExp(`\\b${prefix}[a-z0-9]+(?:-[a-z0-9]+)*\\b`, 'g'))].map(m => m[0]).at(-1) : undefined
  const boxes = inbox?.sandboxes ?? []
  const box = boxes.find(b => b.product === repo && b.branch === branch) ?? boxes.find(b => b.product === repo)
  const stage = mentioned ?? box?.stage
  const out: { argo?: { url: string; label: string }; workflow?: { url: string; label: string }; app?: { url: string; label: string } } = {}
  const argoUrl = stage ? fill(sandboxArgoUrl, { stage }) : undefined
  if (stage && argoUrl) out.argo = { url: argoUrl, label: prefix ? stage.replace(prefix, '') : stage }
  // The app itself on the sandbox running this repo, beside its ArgoCD page.
  const appUrl = box && repo ? await sandboxUrl($, box.stage, repo).catch(() => undefined) : undefined
  if (appUrl) out.app = { url: appUrl, label: 'app' }
  if (!repo) return out
  const app = appFor(await deployedApps($), repo)
  if (!app) return out
  if (!out.argo && app.link) out.argo = { url: app.link, label: 'prod' }
  const latest = await latestDeploy($, app.id).catch(() => null)
  if (latest) out.workflow = await workflowLink($, latest)
  return out
}
