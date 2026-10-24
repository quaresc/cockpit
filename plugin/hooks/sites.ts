// The tools a conversation or a ticket links to, each a footer chip: its hosts and the path of a page worth a chip, as
// regex sources, and a label built from the `ref` the path captures. Marks are FiraCode Nerd Font 3.0 glyphs, a generic
// one where it has no logo (Figma's palette, Notion's boxed N, a pencil for whiteboards, a gauge for monitoring).
type Site = { kind: string; icon: string; host: string; path: string; label?: (ref: string) => string }

const GOOGLE_FILES: Record<string, string> = { document: 'doc', spreadsheets: 'sheet', presentation: 'slides', forms: 'form' }

const SITES: Site[] = [
  // Code review, beside the GitHub PR chip
  { kind: 'mr', icon: '\u{f0ba0}', host: 'gitlab\\.[\\w.-]+', path: '/[^\\s?#]+?/-/merge_requests/(?<ref>\\d+)', label: ref => `!${ref}` },
  { kind: 'bitbucket', icon: '\u{f00a8}', host: 'bitbucket\\.org', path: '/[\\w.-]+/[\\w.-]+/pull-requests/(?<ref>\\d+)', label: ref => `#${ref}` },
  { kind: 'azure-pr', icon: '\u{f0fd5}', host: 'dev\\.azure\\.com|[\\w-]+\\.visualstudio\\.com', path: '/[^\\s?#]+?/_git/[^\\s?#]+?/pullrequest/(?<ref>\\d+)', label: ref => `!${ref}` },
  // Tickets
  { kind: 'jira', icon: '\u{f0303}', host: '[\\w-]+\\.atlassian\\.net|jira\\.[\\w.-]+', path: '/(?:browse/|[^\\s"\'`]*?selectedIssue=)(?<ref>[A-Z][A-Z0-9]+-\\d+)', label: ref => ref },
  { kind: 'issue', icon: '', host: 'github\\.com', path: '/[\\w.-]+/[\\w.-]+/issues/(?<ref>\\d+)', label: ref => `#${ref}` },
  { kind: 'gitlab-issue', icon: '\u{f0ba0}', host: 'gitlab\\.[\\w.-]+', path: '/[^\\s?#]+?/-/(?:issues|work_items)/(?<ref>\\d+)', label: ref => `#${ref}` },
  { kind: 'azure-item', icon: '\u{f0fd5}', host: 'dev\\.azure\\.com|[\\w-]+\\.visualstudio\\.com', path: '/[^\\s?#]+?/_workitems/edit/(?<ref>\\d+)', label: ref => `#${ref}` },
  { kind: 'trello', icon: '\u{f0532}', host: 'trello\\.com', path: '/[cb]/' },
  { kind: 'asana', icon: '\u{f0756}', host: 'app\\.asana\\.com', path: '/\\d+/' },
  { kind: 'clickup', icon: '\u{f0756}', host: 'app\\.clickup\\.com', path: '/t/' },
  { kind: 'shortcut', icon: '\u{f0913}', host: 'app\\.shortcut\\.com', path: '/[\\w-]+/story/(?<ref>\\d+)', label: ref => `sc-${ref}` },
  // Design
  { kind: 'figma', icon: '\u{f03d8}', host: '(?:www\\.)?figma\\.com', path: '/(?:design|file|proto|board|slides|make)/' },
  { kind: 'miro', icon: '\u{f0f49}', host: 'miro\\.com', path: '/app/board/' },
  { kind: 'excalidraw', icon: '\u{f0f49}', host: '(?:app\\.|link\\.)?excalidraw\\.com', path: '/(?:#json=|#room=|l/|s/)' },
  // Docs
  { kind: 'notion', icon: '\u{f0b15}', host: '(?:[\\w-]+\\.)?notion\\.(?:so|site)|app\\.notion\\.com', path: '/[\\w-]' },
  { kind: 'confluence', icon: '\u{f0804}', host: '[\\w-]+\\.atlassian\\.net|confluence\\.[\\w.-]+', path: '/(?:wiki|display|pages)/' },
  { kind: 'google', icon: '\u{f02b6}', host: 'docs\\.google\\.com', path: '/(?<ref>document|spreadsheets|presentation|forms)/d/', label: ref => GOOGLE_FILES[ref] ?? 'doc' },
  { kind: 'coda', icon: '\u{f09ee}', host: 'coda\\.io', path: '/d/' },
  { kind: 'loom', icon: '\u{f0bdc}', host: '(?:www\\.)?loom\\.com', path: '/(?:share|embed)/' },
  // Chat
  { kind: 'slack', icon: '', host: '[a-z0-9-]+\\.slack\\.com', path: '/archives/' },
  { kind: 'teams', icon: '\u{f02bb}', host: 'teams\\.microsoft\\.com|teams\\.live\\.com|teams\\.cloud\\.microsoft', path: '/l/' },
  { kind: 'discord', icon: '\u{f066f}', host: '(?:ptb\\.|canary\\.)?discord(?:app)?\\.com', path: '/channels/' },
  // Monitoring and incidents
  { kind: 'sentry', icon: '\u{f0a30}', host: '(?:[\\w-]+\\.)?sentry\\.io|sentry\\.[\\w.-]+', path: '/[^\\s"\'`]*?issues/\\d+' },
  { kind: 'datadog', icon: '\u{f0e93}', host: '(?:[\\w-]+\\.)?datadoghq\\.(?:com|eu)|(?:[\\w-]+\\.)?ddog-gov\\.com', path: '/[\\w-]' },
  { kind: 'grafana', icon: '\u{f0a07}', host: '[\\w-]+\\.grafana\\.net|grafana\\.[\\w.-]+', path: '/(?:d/|explore|alerting/)' },
  { kind: 'pagerduty', icon: '\u{f078f}', host: '[\\w-]+\\.pagerduty\\.com', path: '/incidents/' },
  { kind: 'incident', icon: '\u{f078f}', host: 'app\\.incident\\.io', path: '/[^\\s"\'`]*?incidents/' },
  // Product analytics
  { kind: 'amplitude', icon: '\u{f154d}', host: '(?:[\\w-]+\\.)*amplitude\\.com', path: '/analytics/' },
  { kind: 'mixpanel', icon: '\u{f154d}', host: '(?:eu\\.)?mixpanel\\.com', path: '/project/' },
  { kind: 'posthog', icon: '\u{f154d}', host: '(?:us|eu|app)\\.posthog\\.com', path: '/project/' },
  { kind: 'metabase', icon: '\u{f154d}', host: 'metabase\\.[\\w.-]+|[\\w-]+\\.metabaseapp\\.com', path: '/(?:question|dashboard)/\\d+' },
]

const REST = '[^\\s)>\\]"\'`]*'

const COMPILED = SITES.map(site => {
  const source = `https://(?:${site.host})${site.path}${REST}`
  return { ...site, all: new RegExp(source, 'g'), one: new RegExp(`^${source}`), host: new RegExp(`^(?:${site.host})$`) }
})

// A URL without the punctuation that closes the sentence or the markup around it.
const clean = (url: string) => url.replace(/[.,;:!?>)\]]+$/, '')

// The URL each kind of site gets in these texts: the first one cited, or the last.
export function citedIn(texts: string[], pick: 'first' | 'last') {
  const cited: Record<string, string> = {}
  for (const site of COMPILED) {
    const urls = texts.flatMap(text => [...text.matchAll(site.all)].map(m => clean(m[0])))
    const url = pick === 'first' ? urls[0] : urls.at(-1)
    if (url) cited[site.kind] = url
  }
  return cited
}

// The footer chips for the cited URLs, in the table's order.
export function chipsFor(cited: Record<string, string>) {
  return COMPILED.filter(site => cited[site.kind]).map(site => {
    const url = cited[site.kind]!
    const ref = url.match(site.one)?.groups?.ref
    return { kind: site.kind, url, icon: site.icon, label: ref && site.label ? site.label(ref) : site.kind }
  })
}

// A page's site mark: the kind whose page it is, else the kind whose host it is on.
export function siteMark(url: string) {
  const host = url.match(/^https?:\/\/([^/]+)/)?.[1] ?? ''
  return (COMPILED.find(site => site.one.test(url)) ?? COMPILED.find(site => site.host.test(host)))?.icon
}
