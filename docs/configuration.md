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
| `spaces` | inbox, terminal, actions, to-do, runbook, meetings | The pinned herdr spaces, in order. A string, or `{name, cwd, icon, key}` for one of your own. |
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
| `mail` | Unread mail: `name`, `server`. A tool that sends mail is never mapped. |
| `platform` | Deploys and preview environments: `previewPrefix`, `previewDashboardUrl`, `workflowUrl`, `previewDefaults`. |
| `metabase`, `datadog` | `{server}` — the connectors `show` panes refresh from. |
| `servers` | The MCP servers cockpit calls itself, with their URL or command. Written by setup for agents other than Claude Code. |
| `refresh` | Seconds between fetches, per source (`github`, `tickets`, `ci`, `mail`, `calendar`, `platform`, `monitor`). |

## What cockpit does on its own

| Key | Default | What it does |
| --- | --- | --- |
| `agents.default` | the first installed | The agent cockpit starts, by id (`claude`, `codex`, `opencode`…). |
| `agents.commands.<id>` | — | Overrides an agent's command line, field by field; a new id adds an agent. |
| `agents.recap` | `auto` | `manual` stops the recap written a minute after a turn ends. |
| `agents.pace` | `week` | `weekdays` moves the 7d quota bar's pace tick 20% per weekday and holds it over the weekend. |
| `recap` | `{enabled: true, awayMinutes: 10}` | The recap shown when you come back to a space. |
| `brief` | `{enabled, at: "08:30", weekends: false}` | The morning brief. |
| `meetingBrief` | `{enabled, model, skip, tools}` | What to read before a meeting. `skip` drops events by name. |
| `deployWatch` | `{enabled, afterMinutes, tools}` | Checks errors after a deploy, through an observability connector. |
| `monitor` | Datadog APM queries, `env: production` | The release monitor: `queries` ({service}, {env}), `moved` thresholds, `notify` (`mine`/`all`/`none`), `serviceReplace` when Datadog names a service differently from the platform. |
| `runbookTools` | `[]` | Extra tools a runbook's background refresh may use. |
| `names` | `true` | Short names given to agents; `false` turns them off. |
| `fetcher` | `true` | The Claude session the hub keeps for sources it cannot reach itself; `false` turns it off. |
| `notifications` | `true` | Notifications at all. |
| `e2eBrowser` | `terminal-browser` | Where an agent runs E2E tests: beside it, or Claude in Chrome. |
| `browserIdleMinutes` | `60` | A browser whose space stayed out of view this long closes; `0` keeps it. |
| `footer.links` | on in Claude Code | The links under the prompt; off in the Herdr app, whose status bar shows them. |

`/cockpit` in Claude Code flips the eight settings worth changing day to day without editing the file.
