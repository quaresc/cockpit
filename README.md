<p align="center"><img src="docs/logo.svg" width="112" alt="cockpit logo"></p>

<h1 align="center">cockpit</h1>

<p align="center">A cockpit for running many Claude Code agents side by side in <a href="https://herdr.dev">herdr</a>:<br>
one agent per space, pinned spaces for what surrounds the work, and a footer in every session that says where it stands.</p>

![The cockpit: herdr sidebar with pinned spaces and agents, a Claude session and its footer](docs/overview.svg)

- **Sidebar**: context percentage and repo · ticket under each agent, coloured by its PR's CI; review, CI, ticket and
  plan-limit counts on the inbox space.
- **Session footer**: the ticket, PR, Figma, Notion, Slack and Argo links of what the conversation works on, the next
  meeting, plan limits when tight, and the context window in use.
- **Notifications** (macOS): with the cockpit icon, and a click that leads somewhere useful: the PR's checks when CI
  ends, the ticket or PR when something changes, the agent's space when it finishes or waits for you, the meeting's
  video link, and the runbook, playing its newly done steps, when a refresh or a deploy completes.
- **Chrome per agent** (optional): an agent using Claude in Chrome gets its own window beside the terminal, shown
  only while its space is focused.

Everything is optional except herdr and GitHub: `/cockpit:setup` asks which parts to turn on.

### Inbox

Review requests, then your work: Linear tickets grouped by project (or status) with their stacked PRs, CI progress
and failures, deploys and sandboxes. A hovered or selected row shows its buttons: review, fix CI, rerun, redeploy,
continue, done.

![Inbox space](docs/inbox.svg)

### Runbook

`/runbook` writes where a feature stands as a table: merges, deploys, migrations and production commands, in order.
The space refreshes it in the background and hands any step back to the session that wrote it.

![Runbook space](docs/runbook.svg)

When a refresh lands, the steps it turned done play in, one after the other. A ticket going live in production
refreshes its runbook by itself, and the notification that follows opens it there:

![A runbook refresh landing: three steps turn done](docs/runbook-done.svg)

### Notifications

macOS notifications carry the cockpit icon, and a click leads where the news is: the PR's checks, the agent's space,
the runbook playing its done steps, the meeting's video call. herdr's own popups stay in-app so none arrives twice.

![Cockpit notifications sliding in, each with where its click leads](docs/notifications.svg)

### To-do and meetings

A Markdown to-do list (also `/todo <text>` from any session) with clickable tickets and links, and the next meetings
from Google Calendar, with a footer pill and a notification five minutes before each.

![To-do space](docs/todo.svg)

![Meetings space](docs/meetings.svg)

## Install

In a Claude Code session inside herdr:

```
/plugin install cockpit --marketplace quaresc/cockpit
```

Answer `y` to add the marketplace and pick a scope, then run `/cockpit:setup`. The setup installs the herdr half
(`herdr plugin install quaresc/cockpit/herdr`) when it is missing, checks the prerequisites, asks which features
and connectors to use, writes `~/.config/cockpit/config.json`, merges the sidebar and key settings into your herdr
config (showing the diff first), creates the pinned spaces and prints the permission rules to add.

## Keeping it healthy

- `/cockpit:doctor` checks every part: tools, both installs and their versions, which Claude sessions run cockpit,
  how fresh the inbox, CI and meetings data is, the space apps, notifications and the permission rules. It names
  the fix for each problem and applies the safe ones once you agree.
- A new version is announced by a notification; clicking it updates both halves, restarts the space apps and reloads
  idle Claude sessions (`sh ~/.config/cockpit/herdr/update.sh` does the same).
- herdr keeps one folder per installed version; cockpit removes the old ones at startup.

## Requirements

- macOS, or Linux: there links open with `xdg-open`, commands copy with `wl-copy` or `xclip`, notifications go
  through `notify-send` (a click works where the notification daemon supports actions), and the Chrome window per
  agent is off
- herdr 0.9 or later, Claude Code with plugin hooks modules
- Xcode command line tools (`swiftc`) for the notifications; without them cockpit falls back to herdr's own
- `gh` signed in, `jq`, Python 3
- A Nerd Font in the terminal (icons and pill buttons); FiraCode Nerd Font Mono is what it was drawn with
- Optional connectors in Claude Code: Linear, CircleCI, Google Calendar, Backstage

## How it fits together

```
plugin/   Claude Code plugin: a hooks module (hooks/*.ts) and the /runbook, /todo, /inbox and /cockpit:setup commands
herdr/    herdr plugin: the space apps (inbox.py, todo.py, runbook.py, meetings.py) and the scripts herdr runs on events
```

The Claude Code side does every network call, from whichever session gets there first (a lock keeps it to one),
and writes the results to `~/.cache/cockpit`. The herdr apps only read those files and drop action requests
(rerun, redeploy, done…) in `~/.cache/cockpit/requests`, which the next session picks up. Your own lists live in
`~/.config/cockpit`: `todo.md`, `runbooks/`, `muted.json`.

## Configuration

`~/.config/cockpit/config.json`, read again every 30 seconds. See [`config.example.json`](config.example.json).

| Key | What it does |
| --- | --- |
| `reposDir` | Folder holding your git checkouts: new agent spaces start there, and the footer follows the repo a session works in |
| `github.org` | Limits your own PRs to one organization; empty for all |
| `linear.teams` | Team keys a ticket mention may use, besides those of the tickets assigned to you |
| `<connector>.server` | The connector's name as `claude mcp list` shows it |
| `circleci.imageJob` | The job that pushes the image a sandbox runs; enables the redeploy button |
| `backstage.*` | Deploy status and sandboxes through Backstage's ship and sandbox-manager plugins, with Argo links built from the URL templates |
| `chromeSideWindow` | One Chrome window per agent, beside the terminal (macOS, iTerm2, Chrome) |
| `spaces` | Which pinned spaces exist, in sidebar order |
| `runbookTools` | Extra tools the background runbook refresh may call |

## Keys

`prefix+i` inbox, `prefix+t` terminal, `prefix+d` to-do, `prefix+u` runbook, `prefix+m` meetings. In the apps:
`j`/`k` move between rows, `h`/`l` or Tab between buttons, Enter presses, `R` refreshes, `g` switches the inbox's
ticket grouping between project and status.

Screenshots are the real apps on fictional data; [`docs/screenshots`](docs/screenshots) regenerates them.

## License

MIT
