import type { EngineInterface } from 'claude-code'

import { loadConfig } from './config'
import type { Block, Spec } from './show'
import { isViewOpen, present, toggleView, writeSpec } from './show'
import { mcpText } from './sources'

type $ = EngineInterface
type Here = { pane: string } | undefined

// A footer chip's press: the ticket or page it links to, read into a pane beside the session.
// The pane opens at once on a loading line, and `o` in it still opens the link in the browser.
export type ReaderKind = 'linear' | 'notion'

const slug = (text: string) => text.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '').slice(0, 30)

export function readerName(kind: ReaderKind, ref: string) {
  return kind === 'linear' ? `linear-${slug(ref)}` : `notion-${(ref.match(/[0-9a-f]{32}/)?.[0] ?? slug(ref)).slice(-12)}`
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
  return kind === 'linear' ? linear($, ref) : notion($, url)
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
