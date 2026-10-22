# cockpit

A cockpit for running many Claude Code agents side by side in [herdr](https://herdr.dev): one agent per space,
pinned spaces for what surrounds the work, and a footer in every session that says where it stands.

- **Inbox space**: review requests, my PRs as stacked trees with CI and review state, Linear tickets grouped by
  project or status, CircleCI runs, deploys and sandboxes, with hover and keyboard buttons (review, fix CI, rerun,
  open, done…).
- **Terminal, to-do, runbook and meetings spaces**: a shell in your repos folder, a Markdown to-do list, the
  `/runbook` tables of what is left to ship, and today's meetings from Google Calendar.
- **Session footer**: the ticket, PR, Figma, Notion, Slack and Argo links of what the conversation works on, the
  next meeting, plan limits when tight, and the context window in use.
- **Sidebar**: context percentage and repo · ticket under each agent, review / CI / ticket counts on the inbox.
- **Chrome per agent** (optional): an agent using Claude in Chrome gets its own window beside the terminal, shown
  only while its space is focused.

Everything is optional except herdr and GitHub: `/cockpit:setup` asks which parts to turn on.

## Install

In a Claude Code session inside herdr:

```
/plugin install cockpit --marketplace quaresc/cockpit
```

Answer `y` to add the marketplace and pick a scope, then run `/cockpit:setup`. The setup installs the herdr half
(`herdr plugin install quaresc/cockpit/herdr`) when it is missing, checks the prerequisites, asks which features
and connectors to use, writes `~/.config/cockpit/config.json`, merges the sidebar and key settings into your herdr
config (showing the diff first), creates the pinned spaces and prints the permission rules to add.

## Requirements

- macOS (the apps open links with `open` and copy with `pbcopy`; Linux is untested)
- herdr 0.9 or later, Claude Code with plugin hooks modules
- `gh` signed in, `jq`, Python 3
- A Nerd Font in the terminal (icons and pill buttons); FiraCode Nerd Font Mono is what it was drawn with
- Optional connectors in Claude Code: Linear, CircleCI, Google Calendar, Platform

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
| `platform.*` | Deploy status and sandboxes through Platform's deploy and sandbox tools, with Argo links built from the URL templates |
| `chromeSideWindow` | One Chrome window per agent, beside the terminal (macOS, iTerm2, Chrome) |
| `spaces` | Which pinned spaces exist, in sidebar order |
| `runbookTools` | Extra tools the background runbook refresh may call |

## Keys

`prefix+i` inbox, `prefix+t` terminal, `prefix+d` to-do, `prefix+u` runbook, `prefix+m` meetings. In the apps:
`j`/`k` move between rows, `h`/`l` or Tab between buttons, Enter presses, `R` refreshes, `g` switches the inbox's
ticket grouping between project and status.

## License

MIT
