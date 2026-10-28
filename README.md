<p align="center"><img src="docs/logo.svg" width="112" alt="cockpit logo"></p>

<h1 align="center">cockpit</h1>

<p align="center"><b>One space with everything, for Claude Code users who switch context all day.</b><br>
cockpit is a Claude Code plugin and a <a href="https://herdr.dev">herdr</a> plugin that work as one
(<a href="#how-it-works">how the two halves fit</a>): your agents run side by side in herdr, and each sits beside its
ticket, PR, CI and the page it tests, with pinned spaces for your inbox, to-dos, runbooks and meetings.</p>

![herdr with the cockpit sidebar, an agent testing its change in terminal-browser beside it, and its footer](docs/overview.svg)

## herdr runs your agents, cockpit tells you what they're working on

herdr is the workspace: spaces, panes and tabs for many agents at once, with their status. It doesn't know about your
work. cockpit adds that context: each agent shows its ticket, branch, PR, CI and deploy, pinned spaces gather your
review requests, tickets, meetings and to-dos, notifications come with buttons that act, and inside Claude Code you get
a footer of links, runbooks and briefs.

## Install

In a Claude Code session inside herdr:

```
/plugin install cockpit --marketplace quaresc/cockpit
/cockpit:setup
```

Setup installs the herdr half with the apps built for your machine (macOS or Linux, ARM or Intel),
asks which features you want and writes your config. `/cockpit:doctor` checks everything and says how to fix what is
off. Updates arrive as a notification.

**Needs** macOS (Linux untested), herdr 0.9+, `gh` signed in, `jq`, Xcode command line tools and a Nerd Font.

## Features

<table>
<tr><td width="24%"><b>🧭 Sidebar</b></td><td>Counts on the pinned spaces; per agent its task, branch, ticket, CI and the page beside it. Red on failing CI or two agents on one branch.</td></tr>
<tr><td colspan="2" align="center"><img src="docs/sidebar.svg" width="238" alt="Sidebar: CI runs on an agent's branch, the agent finishes, a review request arrives"></td></tr>
<tr><td width="24%"><b>🔗 Footer</b></td><td>One click to the ticket, PR, deploy dashboard, preview and each page the conversation cites.</td></tr>
<tr><td colspan="2" align="center"><img src="docs/footer.svg" width="614" alt="Footer: the chips follow what the conversation names"></td></tr>
<tr><td width="24%"><b>📥 Inbox</b></td><td>Review requests, then your tickets by project with their stacked PRs, CI, deploys, previews and what each cost in Claude usage. Each row has its actions, and several picked rows share them; <code>/</code> filters, review threads and failing CI show beside a PR, and a row flashes when its CI turns or an agent finishes. A review, a plan or a talk about a ticket (forked from the session that worked on it) opens in a tab of the actions space.</td></tr>
<tr><td colspan="2" align="center"><img src="docs/inbox.svg" width="668" alt="Inbox: the tree of projects, tickets and stacked PRs, a filter typed and kept, then the palette searching every app"></td></tr>
<tr><td width="24%"><b>🧪 E2E tests</b></td><td>The agent opens its page in terminal-browser beside it, on phone and desktop, main against the branch, and puts the before/after in the PR.</td></tr>
<tr><td colspan="2" align="center"><img src="docs/e2e.svg" width="829" alt="E2E test: terminal-browser opens beside the agent, phone then desktop, then the before and after"></td></tr>
<tr><td width="24%"><b>📋 Runbook</b></td><td><code>/runbook</code> writes the steps left to ship a ticket and keeps them current; steps light up as they get done, and you can filter, edit and add them in place.</td></tr>
<tr><td colspan="2" align="center"><img src="docs/runbook-done.svg" width="776" alt="Runbook: a refresh lands and three steps spin, flash and fade as they turn done"></td></tr>
<tr><td width="24%"><b>📊 Show</b></td><td>Charts (hover for values), tables, flow charts, sequence diagrams, pictures and before/after compares, diffs you comment line by line for the agent, live Metabase or Datadog data and questions, in a pane beside the agent.</td></tr>
<tr><td colspan="2" align="center"><img src="docs/show.svg" width="829" alt="Show: charts drawn as images open beside the conversation, hide to their chip and come back"></td></tr>
<tr><td width="24%"><b>🔔 Notifications</b></td><td>Each leads somewhere useful, with buttons that act: Review, Why?, Rerun, Plan, Join.</td></tr>
<tr><td colspan="2" align="center"><img src="docs/notifications.svg" width="627" alt="Notifications sliding in with their buttons"></td></tr>
<tr><td width="24%"><b>☀️ Briefs</b></td><td>A morning brief, meeting briefs, a recap when you come back to a space, a check after each deploy.</td></tr>
<tr><td colspan="2" align="center"><img src="docs/recap.svg" width="475" alt="Recap of what an agent did while you were away"></td></tr>
<tr><td width="24%"><b>📝 To-do</b></td><td>A Markdown list you edit in place; <code>/todo &lt;text&gt;</code> adds to it from any session, ticket keys and links clickable.</td></tr>
<tr><td colspan="2" align="center"><img src="docs/todo.svg" width="561" alt="To-do space"></td></tr>
<tr><td width="24%"><b>📅 Meetings</b></td><td>Your week, as a list or drawn day by day, with join and prepare on each meeting.</td></tr>
<tr><td colspan="2" align="center"><img src="docs/meetings.svg" width="539" alt="Meetings space"></td></tr>
<tr><td width="24%"><b>🩺 Monitor</b></td><td>Every new release of the environment, from the platform connector, with a verdict once it has run a while: errors, p95 and new Datadog errors before and after, alerting monitors and a day of numbers per app. From a release or an error: investigate, fix it in a draft PR, file a bug or roll back, each in a tab of the actions space. Opt in by adding <code>monitor</code> to <code>spaces</code>.</td></tr>
<tr><td width="24%"><b>⚙️ Settings</b></td><td><code>/cockpit</code> flips what you change day to day. In every app, ctrl+p searches all of them: a ticket, its runbook, its PRs, a view.</td></tr>
<tr><td colspan="2" align="center"><img src="docs/settings.svg" width="546" alt="The /cockpit settings pane"></td></tr>
<tr><td width="24%"><b>🕰️ Past sessions</b></td><td>ctrl+p also finds your past Claude sessions by what was said in them, from an index the hub keeps on its own; choosing one opens a fork of it in the actions space that sums up where it stood.</td></tr>
</table>

## Supported tools

**Built in** works with no mapping and is tested. **Mapped by setup**: `/cockpit:setup` reads that connector's tools and
writes which tool does what, so it works as far as the connector's tools go.

| Area | Built in | Mapped by setup |
| --- | --- | --- |
| Code, PRs and reviews | GitHub | |
| Tickets | Linear | Jira, Trello, Asana, ClickUp, Shortcut, GitHub Issues, GitLab Issues, Azure DevOps Boards |
| CI | GitHub Actions and every GitHub check on your PRs (CircleCI, Buildkite and Jenkins statuses included), CircleCI | GitLab CI, Buildkite, Jenkins |
| Calendar | Google Calendar | Outlook |
| Deploys and previews | | Argo CD, Vercel, Netlify, Render, Railway, Fly.io, Backstage |
| Live data in show panes | Metabase, Datadog | |
| Check after a deploy | Datadog | Grafana, Sentry |
| Meeting briefs read | Notion | Confluence, Google Docs |
| E2E browser | terminal-browser, Claude in Chrome | |
| Line comments on a diff | herdr-reviewr | |

**Footer links** recognised, one chip each:

| | |
| --- | --- |
| Code review | GitHub pull requests, GitLab merge requests, Bitbucket pull requests, Azure DevOps pull requests |
| Tickets | Linear, Jira, GitHub issues, GitLab issues, Azure DevOps work items, Trello, Asana, ClickUp, Shortcut |
| Design | Figma (design, FigJam, Slides, Make), Miro, Excalidraw |
| Docs | Notion, Confluence, Google Docs, Google Sheets, Google Slides, Google Forms, Coda, Loom |
| Chat | Slack, Microsoft Teams, Discord |
| Monitoring | Sentry, Datadog, Grafana, PagerDuty, incident.io |
| Analytics | Amplitude, Mixpanel, PostHog, Metabase |
| Deploys | Argo CD, Vercel, Netlify, Render, Railway, Fly.io dashboards, and your preview environments |

## Configuration

`~/.config/cockpit/config.json`, written by `/cockpit:setup` and re-read every 30 seconds. See
[`config.example.json`](config.example.json).

While a cockpit app runs, its hub fetches GitHub every minute with `gh`'s token (or `GH_TOKEN`), so PRs, reviews and
GitHub checks stay fresh without a Claude session; tickets, deploys, previews and other CIs come from a Claude session's
connectors, unless a token below lets the hub fetch them too.

<details>
<summary><b>Keys</b></summary>

| Key | |
| --- | --- |
| `reposDir` | Where your checkouts live; new agent spaces start there. |
| `github.org` | Only list your PRs from this org. |
| `tickets` | Ticket connector: `server`, `keys` (ticket key prefixes such as `ENG`), `urlTemplate`, and for anything but Linear the mapping setup writes. |
| `ci` | CI: GitHub checks when `server` is empty, else a CI connector and its mapping. `imageJob`: the job that builds a preview's image. |
| `tickets.token` | Optional, for Linear: a personal API key (Linear → Settings → Security & access). The hub then fetches your tickets itself, fresh without a Claude session. Stays in this file only. |
| `ci.token` | Optional, for CircleCI: a personal API token (CircleCI → User settings → Personal API tokens). The hub then fetches your runs itself, fresh without a Claude session. Stays in this file only. |
| `calendar` | Calendar connector: Google Calendar, or another and its mapping. |
| `platform` | Deploy and preview connector, and its mapping. |
| `metabase`, `datadog` | Connectors for live panes; Datadog's also feeds the monitor space. |
| `monitor` | The monitor space: `env`, the Datadog `queries` of a service (`errors`, `hits`, `p95`, with `{service}` and `{env}`), when a release counts as moved (`moved`: `errorPoints`, `p95Ratio`, `minHits`) and whose releases notify (`notify`: `mine`, `all` or `none`). |
| `brief`, `meetingBrief`, `recap`, `deployWatch` | When briefs, recaps and deploy checks run, and the tools they may use. |
| `e2eBrowser` | Where agents run E2E tests: `terminal-browser` (beside the agent) or `chrome` (Claude in Chrome). |
| `spaces` | Pinned spaces, in sidebar order: the built-in `inbox`, `terminal`, `actions` (a tab per review, plan or talk the inbox and runbook start), `to-do`, `runbook`, `meetings`, the opt-in `monitor`, and your own as a name, or as `{ "name": "notes", "cwd": "~/notes", "icon": "\uf413", "key": "prefix+n" }`. A space you already opened under that name becomes the pinned one. |
| `density` | `comfortable` (default): a row's actions in a panel under the list. `compact`: denser rows with their buttons on the row itself. `/cockpit` switches it. |

</details>

<details>
<summary><b>Shortcuts</b></summary>

`prefix+i` inbox, `prefix+t` terminal, `prefix+a` actions, `prefix+d` to-do, `prefix+u` runbook, `prefix+m` meetings,
`prefix+o` monitor, the `key` of each space of your own, `prefix+f` the page beside the agent in your browser. The apps
share their keys: `j`/`k` rows, `g`/`G` the ends, `[` `]` the groups, Enter the row's main action, `o` open, `/` filter,
`v` pick, `R` refresh, ctrl+p everything.

</details>

## How it works

`plugin/` is the Claude Code half: it fetches in your sessions, one at a time, and hands what it fetched to the hub.
`herdr/` is the herdr half: the space apps and the hub, one `cockpit` binary. The hub is the only writer of the shared
data, kept with a week of its history in `~/.cache/cockpit/store.redb` and mirrored as JSON beside it; it pushes each
change to the apps, whose buttons leave requests the next session carries out. The
screenshots are the real apps on made-up data; [`docs/screenshots`](docs/screenshots) regenerates them.

## License

MIT
