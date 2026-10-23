import type { EngineInterface } from 'claude-code'

import { loadConfig } from './config'
import type { Block, Spec } from './show'
import { isViewOpen, present, toggleView, writeSpec } from './show'
import { mcpText } from './sources'

type $ = EngineInterface
type Here = { pane: string } | undefined

// A footer chip's press: the ticket, PR, page, thread or design it links to, read into a pane beside the session.
// The pane opens at once on a loading line, and `o` in it still opens the link in the browser.
export type ReaderKind = 'linear' | 'pr' | 'notion' | 'slack' | 'figma'

const slug = (text: string) => text.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '').slice(0, 30)

export function readerName(kind: ReaderKind, ref: string) {
  if (kind === 'linear') return `linear-${slug(ref)}`
  if (kind === 'pr') return `pr-${slug(ref.replace(/^https:\/\/github\.com\/[^/]+\//, '').replace('/pull/', '-'))}`
  if (kind === 'figma') return `figma-${slug(new URL(ref).searchParams.get('node-id') ?? ref.split('/')[4] ?? 'frame')}`
  if (kind === 'notion') return `notion-${(ref.match(/[0-9a-f]{32}/)?.[0] ?? slug(ref)).slice(-12)}`
  return `slack-${ref.match(/p(\d{16})/)?.[1] ?? slug(ref).slice(-16)}`
}

const markdown = (text: string, title?: string): Block => ({ type: 'markdown', text, ...(title ? { title } : {}) })

export async function openReader($: $, kind: ReaderKind, ref: string, url: string, here: Here) {
  const name = readerName(kind, ref)
  if (await isViewOpen($, name)) {
    await toggleView($, name, here)
    return
  }
  const base: Spec = { title: ref, blocks: [markdown(`Reading ${url}…`)], at: Date.now(), session: await $.session.id(), placement: 'right', url, isReader: true }
  await present($, name, base, here)
  let blocks: Block[]
  let title = ref
  try {
    ;({ title, blocks } = await read($, kind, ref, url))
  } catch (err) {
    blocks = [markdown(`Could not read it: ${(err as Error).message.slice(0, 300)}\n\n\`o\` opens it in the browser.`)]
  }
  await writeSpec($, name, { ...base, title, blocks, at: Date.now() })
}

async function read($: $, kind: ReaderKind, ref: string, url: string): Promise<{ title: string; blocks: Block[] }> {
  if (kind === 'linear') return linear($, ref)
  if (kind === 'pr') return pr($, url)
  if (kind === 'notion') return notion($, url)
  if (kind === 'slack') return slack($, url)
  return figma($, url)
}

// ---------------------------------------------------------------------------
// Linear: the ticket, its links and its comments

type Comment = { body?: string; createdAt?: string; author?: { name?: string } | string | null; user?: { name?: string } }

async function linear($: $, id: string) {
  const server = (await loadConfig($)).linear.server
  const fields = ['title', 'description', 'url', 'status', 'assignee', 'labels', 'project', 'attachments', 'parentId']
  const issue = JSON.parse(await mcpText($, server, 'get_issue', { id, fields })) as Record<string, unknown>
  const { comments = [] } = JSON.parse(await mcpText($, server, 'list_comments', { issueId: id, limit: 50, orderBy: 'createdAt' })) as { comments?: Comment[] }
  const facts = [issue.status, issue.assignee, issue.project, (issue.labels as string[] | undefined)?.join(', ')].filter(Boolean).join(' · ')
  const description = String(issue.description ?? '_No description._').replace(/\\([[\]_*])/g, '$1')
  const links = ((issue.attachments ?? []) as { title?: string; url?: string }[]).map(a => `- [${(a.title ?? a.url ?? "").replace(/[[\]]/g, "")}](${a.url})`).join('\n')
  const said = comments.map(c => {
    const who = typeof c.author === 'string' ? c.author : (c.author?.name ?? c.user?.name ?? 'someone')
    return `**${who}** · ${(c.createdAt ?? '').slice(0, 10)}\n${c.body ?? ''}`
  }).join('\n\n')
  return {
    title: `${id} · ${issue.title}`,
    blocks: [
      markdown(`${facts}\n\n${description}`),
      ...(links ? [markdown(links, 'Linked')] : []),
      ...(said ? [markdown(said, `Comments (${comments.length})`)] : []),
    ],
  }
}

// ---------------------------------------------------------------------------
// GitHub: the PR, its checks, its review comments and its diff

const MAX_DIFF = 400_000

async function gh($: $, args: string[]) {
  const done = await $.process.run(['gh', ...args], { timeoutMs: 30_000 })
  if (done.exitCode !== 0) throw new Error(`gh ${args[0]} ${args[1] ?? ''}: ${done.stderr.trim().split('\n')[0]}`)
  return done.stdout
}

type Check = { name?: string; context?: string; conclusion?: string | null; state?: string; status?: string }
type ReviewComment = { path: string; line?: number | null; body: string; user?: { login?: string } }

async function pr($: $, url: string) {
  const fields = 'title,body,author,state,isDraft,reviewDecision,headRefName,baseRefName,additions,deletions,changedFiles,statusCheckRollup,reviews,comments,number'
  const view = JSON.parse(await gh($, ['pr', 'view', url, '--json', fields])) as Record<string, unknown>
  const repo = url.match(/github\.com\/([^/]+\/[^/]+)\/pull\/(\d+)/)
  const inline = repo ? (JSON.parse(await gh($, ['api', `repos/${repo[1]}/pulls/${repo[2]}/comments`, '--paginate'])) as ReviewComment[]) : []
  const diff = await gh($, ['pr', 'diff', url]).catch(() => '')
  const checks = ((view.statusCheckRollup ?? []) as Check[]).map(c => [c.name ?? c.context ?? '?', c.conclusion ?? c.state ?? c.status ?? '?'])
  const author = (view.author as { login?: string } | undefined)?.login ?? '?'
  const facts = [`${view.state}${view.isDraft ? ' (draft)' : ''}`, `by ${author}`, `${view.headRefName} → ${view.baseRefName}`,
    `+${view.additions} −${view.deletions} in ${view.changedFiles} files`, view.reviewDecision ? String(view.reviewDecision).toLowerCase().replace('_', ' ') : ''].filter(Boolean).join(' · ')
  const reviews = [...((view.reviews ?? []) as { author?: { login?: string }; state?: string; body?: string }[]).filter(r => r.body || r.state !== 'COMMENTED')
    .map(r => `**${r.author?.login ?? '?'}** ${String(r.state ?? '').toLowerCase().replace('_', ' ')}${r.body ? `\n${r.body}` : ''}`),
  ...((view.comments ?? []) as { author?: { login?: string }; body?: string }[]).map(c => `**${c.author?.login ?? '?'}**\n${c.body ?? ''}`)]
  const byFile = new Map<string, string[]>()
  for (const c of inline) byFile.set(c.path, [...(byFile.get(c.path) ?? []), `- **${c.user?.login ?? '?'}**${c.line ? ` line ${c.line}` : ''}: ${c.body.split('\n')[0]}`])
  const onCode = [...byFile].map(([path, lines]) => `\`${path}\`\n${lines.join('\n')}`).join('\n\n')
  return {
    title: `#${view.number} ${view.title}`,
    blocks: [
      markdown(`${facts}\n\n${String(view.body || '_No description._').replace(/<!--[\s\S]*?-->/g, '').replace(/<\/?(details|summary|p|br|sub|sup|div|img)[^>]*>/g, '')}`),
      ...(checks.length ? [{ type: 'table', title: 'Checks', columns: ['check', 'result'], rows: checks }] : []),
      ...(reviews.length ? [markdown(reviews.join('\n\n'), 'Reviews and comments')] : []),
      ...(onCode ? [markdown(onCode, 'On the code')] : []),
      ...(diff && diff.length <= MAX_DIFF ? [{ type: 'diff', title: 'Diff', patch: diff }] : []),
    ],
  }
}

// ---------------------------------------------------------------------------
// Notion: the page as Markdown

const ENTITIES: Record<string, string> = { '&lt;': '<', '&gt;': '>', '&amp;': '&', '&quot;': '"', '&#39;': "'" }

// Notion's Markdown flavour, its tags turned into plain Markdown and its markers dropped.
export function notionToMarkdown(text: string) {
  return text
    .replace(/<b>([\s\S]*?)<\/b>/g, '**$1**')
    .replace(/<code>([\s\S]*?)<\/code>/g, '`$1`')
    .replace(/<br\s*\/?>/g, '\n')
    .replace(/<date start="([^"]+)"[^>]*\/>/g, '$1')
    .replace(/<page url="([^"]+)"[^>]*>([\s\S]*?)<\/page>/g, '[$2]($1)')
    .replace(/<callout[^>]*>/g, '').replace(/<\/callout>/g, '')
    .replace(/<\/?p\s*\/?>/g, '\n')
    .replace(/<[^>]+>/g, '')
    .replace(/&(lt|gt|amp|quot|#39);/g, m => ENTITIES[m] ?? m)
    .replace(/\n{3,}/g, '\n\n')
    .trim()
}

async function notion($: $, url: string) {
  const page = JSON.parse(await mcpText($, (await loadConfig($)).notion.server, 'notion-fetch', { id: url })) as { title?: string; text?: string }
  const content = page.text?.match(/<content>([\s\S]*?)<\/content>/)?.[1] ?? page.text ?? ''
  return { title: page.title ?? 'Notion page', blocks: [markdown(notionToMarkdown(content))] }
}

// ---------------------------------------------------------------------------
// Slack: the thread

export function slackToMarkdown(text: string) {
  return text
    .replace(/<@[A-Z0-9]+\|([^>]+)>/g, '@$1')
    .replace(/<(https?:[^|>]+)\|([^>]+)>/g, '[$2]($1)')
    .replace(/<(https?:[^>]+)>/g, '$1')
    .replace(/(^|\s)\*([^*\n]+)\*(?=\s|$|[.,!?])/g, '$1**$2**')
    .replace(/^> ([^:\n]+?) <[^>]+>: /gm, '- **$1**: ')
    .replace(/\\\//g, '/')
}

async function slack($: $, url: string) {
  const found = url.match(/archives\/([A-Z0-9]+)\/p(\d{10})(\d{6})/)
  if (!found) throw new Error('not a Slack message link')
  const thread = new URL(url).searchParams.get('thread_ts') ?? `${found[2]}.${found[3]}`
  const answer = JSON.parse(await mcpText($, (await loadConfig($)).slack.server, 'slack_read_thread', {
    channel_id: found[1], message_ts: thread, limit: 100, response_format: 'concise',
  })) as { messages?: string }
  return { title: 'Slack thread', blocks: [markdown(slackToMarkdown(answer.messages ?? ''))] }
}

// ---------------------------------------------------------------------------
// Figma: the frame, drawn in the pane

async function figma($: $, url: string) {
  const parsed = new URL(url)
  const parts = parsed.pathname.split('/').filter(Boolean)
  const fileKey = parts[2] === 'branch' ? parts[3] : parts[1]
  // An instance's id is a path of ids; its last one is the frame itself.
  const nodeId = parsed.searchParams.get('node-id')?.split(';').at(-1)?.replace(/^I/, '')
  if (!fileKey || !nodeId) throw new Error('the Figma link names no frame (node-id)')
  const server = (await loadConfig($)).figma.server
  const session = JSON.parse(await mcpText($, server, 'create_session', { fileKey, description: 'Preview a frame in a terminal pane beside a Claude Code session' })) as { sessionId?: string }
  const shot = await mcpText($, server, 'get_screenshot', { fileKey, nodeId: nodeId.replace(':', '-'), maxDimension: 1024, ...(session.sessionId ? { sessionId: session.sessionId } : {}) })
  const image = shot.match(/"image_url"\s*:\s*"([^"]+)"/)?.[1]
  if (!image) throw new Error(`Figma gave no picture: ${shot.slice(0, 160)}`)
  const name = decodeURIComponent(parts[2] === 'branch' ? (parts[5] ?? '') : (parts[2] ?? '')).replace(/-{2,}/g, ' – ').replace(/-/g, ' ')
  return { title: `Figma · ${name || fileKey}`, blocks: [{ type: 'image', url: image, caption: `node ${nodeId}` }] }
}
