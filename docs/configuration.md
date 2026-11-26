# Configuration

Everything cockpit knows lives in one file, `~/.config/cockpit/config.json`, written by `cockpit setup`
(`/cockpit:setup` in Claude Code) and safe to edit by hand. `config.example.json` at the repo root is a filled-in
example of the same shape.

Nothing in it is required: a missing key falls back to the default below, and a missing file means a cockpit that
is not set up yet. The file is written `0600` because it may hold tokens.

After editing it by hand, run `cockpit doctor` — it reports a file that does not parse, and any connector whose
tools are not mapped.

## Where the work is

| Key | Default | What it does |
| --- | --- | --- |
| `reposDir` | `~` | The folder holding your git checkouts. |
| `worktrees` | `false` | The inbox starts work on a branch checked out nowhere in a new herdr worktree. |
| `start.branch` | `{prefix}/{key}-{slug}` | The branch Start work makes for a ticket or a prompt: `{key}` is the ticket's key, `{slug}` a few words of its title or the prompt, and an empty part drops out. A branch name the tracker gives wins over it. |
| `start.prefix` | `""` | What `{prefix}` stands for, such as `me`. |
| `start.setup` | `{}` | A command per repo, by its folder name (`{"api": "npm ci"}`), that Start work runs in a new worktree before the agent. A new or changed command is asked about once, and a fork's PR runs it only when asked. |
| `spaces` | inbox, terminal, actions, to-do, runbook, meetings | The pinned herdr spaces, in order. A string, or `{name, cwd, icon, key}` for one of your own. `monitor`, `routines`, `review` and `mail` are the other built-in ones. |
| `editor` | `code` | The command the palette's "open in your editor" runs. |
| `density` | `comfortable` | `compact` puts a row's buttons on the row itself and fits more rows. |
| `herdrPluginRoot` | `""` | The herdr plugin folder; empty lets the mod find it in herdr's registry. |
| `frontend` | unset | Forces `tui` or `app` when cockpit cannot tell which it runs in. |

## The sources

Each source is `{"enabled": true, "name": …, "server": …}` plus a `tools` and `fields` mapping, which setup writes
by reading the server's own tool schemas. A source counts as connected when its block is on **and** it has
something to reach — a `server`, the calendar's `ics`, or `gh` for GitHub and its checks. A space whose sources
are none of those says so in its empty state rather than waiting for data that will never come. Linear, CircleCI, Gmail and Google Calendar are mapped by cockpit
itself and need no `tools`.

| Key | What it covers |
| --- | --- |
| `github` | `{enabled, org}` — your pull requests, through `gh`. `org` limits them to one organization. |
| `tickets` | Your tickets: `name`, `server`, `keys` (prefixes such as `ENG`), `urlTemplate`, and `token` for refreshes with no agent session. Linear and GitHub Issues are mapped by cockpit; naming their server is enough. |
| `ci` | CI runs: `name`, `server`, `imageJob` (the job that pushes a preview's image), `token`. Empty `server` reads GitHub checks through `gh`. |
| `calendar` | Meetings: `server`, and `calendars` (ids such as `primary` or an address) to read more than its default calendar; or `ics` (the calendar's secret iCal address) and `email` so the hub fetches them itself. |
| `mail` | Unread mail: `name`, `server`. `mail.tools.listUnread` lists the unread threads when the listing carries no unread flag, `mail.tools.peek` reads one thread's metadata for its date when it carries none. A tool that sends mail is never mapped. |
| `platform` | Deploys and preview environments: `previewPrefix`, `previewDashboardUrl`, `workflowUrl`, `previewDefaults`. |
| `metabase`, `datadog` | `{server}` — the connectors `show` panes refresh from. |
| `servers` | The MCP servers cockpit calls itself, with their URL or command. Written by setup for agents other than Claude Code. |
| `refresh` | Seconds between fetches, per source (`github`, `tickets`, `ci`, `mail`, `calendar`, `platform`, `monitor`). |

## What cockpit does on its own

| Key | Default | What it does |
| --- | --- | --- |
| `agents.default` | the first installed | The agent cockpit starts, by id (`claude`, `codex`, `opencode`…). |
| `agents.commands.<id>` | — | Overrides an agent's command line, field by field; a new id adds an agent. `light` names its lightest model, which errands like next steps run on. |
| `agents.recap` | `auto` | `manual` stops the recap written a minute after a turn ends. |
| `usage.commands` | none | Limits of a provider cockpit has no source for: `{"<agent id>": "<command>"}`, each command printing a JSON list of `{name, percent, resets_at}` (run at most every 10 minutes); its windows show under that agent's logo. |
| `queue.drain` | `true` | Each prompt queued for an agent goes when its turn ends; `false` sends them only by hand. |
| `chat.nextSteps` | `false` | After a finished answer, up to three suggested prompts appear above the box; 1, 2, 3 or a click fill it, never send. Works for any agent's conversation: Claude Code answers, with no tools, billed to your own Claude account; `chat.nextStepsAgent` names another agent that runs with no tools once one exists. |
| `chat.projectPanels` | `true` | In Cockpit Chat, the side panel's Files and Changes are cockpit's: they show the repositories the agent works in, even from a folder of several, the latest first. Off, Herdr's own follow the focused terminal's folder. |
| `chat.nextStepsModel` | the agent's lightest model | The model next steps run on, at the agent's lowest effort: a name, or one per agent id (`{"claude": "haiku"}`). By default the agent's lightest (`agents.commands.<id>.light`, Haiku for Claude Code), else the session's own. |
| `ci.tellAgents` | `true` | A CI run that ends red is told to the agent working on its PR, and green after it; `false` tells no agent. |
| `github.reviewBots` | `false` | `address review` hands bots' review threads over too, after people's. |
| `agents.pace` | `week` | `weekdays` moves the 7d quota bar's pace tick 20% per weekday and holds it over the weekend. |
| `recap` | `{enabled: true, awayMinutes: 10}` | The recap shown when you come back to a space. |
| `brief` | `{enabled, at: "08:30", weekends: false}` | The morning brief. |
| `meetingBrief` | `{enabled, model, skip, tools}` | What to read before a meeting. `skip` drops events by name. |
| `deployWatch` | `{enabled, afterMinutes, tools}` | Checks errors after a deploy, through an observability connector. |
| `monitor` | Datadog APM queries, `env: production` | The release monitor: `queries` ({service}, {env}), `moved` thresholds, `notify` (`mine`/`all`/`none`), `serviceReplace` when Datadog names a service differently from the platform. |
| `runbookTools` | `[]` | Extra tools a runbook's background refresh may use. |
| `names` | `true` | Agents, tabs and spaces named after their work: the name you gave, else the agent's own title, else its first prompt in a few words, never over a tab name you or another tool typed; `false` turns it off. |
| `fetcher` | `true` | The Claude session the hub keeps for sources it cannot reach itself; `false` turns it off. |
| `notifications` | `true` | Notifications at all. |
| `dockBadge` | `true` | On macOS, the Herdr app's Dock icon shows how many agents need you. |
| `e2eBrowser` | `terminal-browser` | Where an agent runs E2E tests: beside it, or Claude in Chrome. |
| `browserIdleMinutes` | `60` | A browser whose space stayed out of view this long closes, and a Chromium tab out of view this long sleeps; `0` keeps them. |
| `sideBrowser` | `terminal-browser` | The browser beside an agent: terminal-browser in a pane, or `chromium`, cockpit's own window docked in the Herdr app on macOS (see [browser.md](browser.md)). |
| `links.open` | `browser` | Where a Cmd-click (Ctrl-click) on a web link in a Herdr app terminal opens it: `browser`, your own, or `side`, the side browser beside that terminal. |
| `links.fromBranch` | `false` | An agent's ticket, PR and CI also come from its branch name and the pull requests on it; off, only what its conversation cites or creates counts, everywhere cockpit shows them. |
| `browser` | `{dock: true, profileScope: "shared", popups: "tab", downloads: ""}` | The Chromium side browser: docked or a window of its own, the profile a worktree starts with (`shared`, `repo`, `worktree`), pop-ups as tabs or blocked, and the downloads folder (yours when empty). |
| `artifactGuard` | `true` | In Claude Code, a new claude.ai artifact waits until `show` was tried or you asked for an artifact or a link; `false` lets every publish through (see [show.md](show.md)). |
| `footer.links` | on in Claude Code | The links under the prompt; off in the Herdr app, whose status bar shows them. |

`/cockpit` in Claude Code flips the eight settings worth changing day to day without editing the file. In the Herdr
app, **Cockpit settings** (at the foot of the sidebar, or ⌘⇧P then "Cockpit settings") shows every option that takes a
switch, a choice or a short field, section by section, each with its default; under them it lists what only this file
holds and opens it in your editor. `cockpit options` prints the same list in a terminal, and
`cockpit options set <key> <value>` sets one there: `on` or `off`, one of a choice's values, a field's text or a
number, checked before anything is written (a script or an agent's tools are told to ask you). The options an update
brings (`links.fromBranch`, `queue.drain`, `ci.tellAgents`, `chat.nextSteps`) are asked about once after that update.
