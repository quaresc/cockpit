# Changelog

Each version is tagged `v<version>` and has a GitHub release with these notes, both made by the release workflow once
`plugin/.claude-plugin/plugin.json` reaches main with a new version. Both halves, the Claude Code plugin and the herdr
plugin, share the version with the apps (`herdr/tui/Cargo.toml`), whose binaries the release builds; the section for it
must be here first.

## 0.19.0 (2026-10-28)

- New opt-in monitor space (`monitor` in `spaces`, `prefix+o`): every production release from the platform connector,
  judged on Datadog's errors, p95 and new errors before and after, with alerting monitors and a day of numbers per app.
  From a release or an error: investigate, fix in a draft PR, file a bug or roll back, each in the actions space (#8).
- The hub is the only writer of the shared data, kept with a week of history in `store.redb` and pushed to the apps;
  sessions take leases in the hub instead of lock folders, which left stale files behind (#6).
- `doctor`, `notify`, `focus` and the space loops are `cockpit` commands instead of shell scripts (#6).
- The same keys in every app: `g`/`G` the ends, `[` `]` the groups, Enter the main action, `x` done, `r` run; the inbox
  groups on `w`, the runbook marks done on `x` (#7).

## 0.18.0 (2026-10-28)

- One `cockpit` binary instead of seven, a smaller download. It keeps itself current: the hub installs a new release
  as soon as it is published, and an update that arrives before its binary keeps the previous one running (#5).
- The hub fetches GitHub itself every minute, so the inbox stays fresh without a Claude session; with `tickets.token`
  or `ci.token` in the config it fetches Linear or CircleCI too.
- ctrl+p finds past Claude sessions in an index built into the hub, nothing to install, and opens a fork of the one you
  pick in the actions space.
- Code is coloured with tree-sitter in 22 languages, TypeScript and Elixir included, and markdown renders as on GitHub:
  alerts, footnotes, autolinks.
- The palette never waits on the hub while you type, and the hub stays up while a space app runs.

## 0.17.0 (2026-10-28)

- The space and show apps are rewritten in Rust: about 11 MB each, idle CPU about 0, nothing that cockpit runs needs
  Python any more. Each release carries them built for macOS and Linux, and setup and updates install them (#4).
- A background hub runs every action, and ctrl+p in any app searches all of them: a ticket, its runbook, its PRs, a view.
- Inbox: `/` filters, several rows picked share their actions, review threads and failing CI show beside a PR, and a row
  flashes when its CI turns or an agent finishes; ticket costs get a sparkline.
- Runbook: filter, pick several steps, edit a step's command or status and add steps in place.
- Show: CommonMark with syntax colours, charts with values on hover and zoom, diffs with word highlights, side by side and
  line comments sent to the session, before/after compares in the pane, animated GIFs, a log that scrolls and follows.
- To-do edits in place with undo; meetings draw the week as a picture.
- `/cockpit` gets a Density setting: compact puts a row's buttons on the row itself.

## 0.16.3 (2026-10-27)

- Brief lines about several PRs open the GitHub search that lists them all, not just the first PR (#2).
- The calendar connector retries after a missed startup race, and meetings read Google's `Z` timestamps (#1).

## 0.16.2 (2026-10-27)

- The sidebar lists the actions space's Claude sessions under its row, each with its state; one waiting for you shows in
  the accent colour. Setup adds the six `$act_*` rows to the herdr config.

## 0.16.1 (2026-10-27)

- Each runbook in the runbook space gets a **talk** button: it forks the session that wrote the runbook, else the one that
  worked most on its ticket, in a tab of the actions space.

## 0.16.0 (2026-10-27)

- New pinned **actions** space (`prefix+a`): the inbox's review and plan, and a runbook step whose session is gone, open
  as its tabs instead of new spaces. Pressing an action again while it runs brings its tab back.
- **Talk** on every inbox ticket (`t`): forks the session that worked most on the ticket, with what the inbox knows of
  its PRs, CI, deploys, previews and runbook, and changes nothing unless asked.
- **Continue** on a ticket with a live plan tab moves that tab out to a space of its own.
- Notifications, the recap popup, the same-branch warning and closing a session on a merged branch follow the session's
  tab when it sits in a pinned space; closing no longer takes a whole pinned space with it.

## 0.15.6 (2026-10-26)

- A /bg job draws its own rows but leaves fetching to the sessions at a prompt: it has no REPL for the mod's MCP calls, so its inbox fetch recorded Linear and previews as failing
- Runbook space spans the full width like the inbox: status, step and where share it, refresh sits at the right edge and stays among a hovered runbook's buttons, notes wrap; padding cut by a row's buttons stays blank

## 0.15.5 (2026-10-26)

- Runbook space: a step's command shows under it, wrapped over the full width; a click copies it

## 0.15.4 (2026-10-26)

- Sidebar rows fit herdr's sidebar width instead of ending in an ellipsis: the work row keeps a running CI's progress, tightening its spacing, then shortening the repo to its initials, then keeping the ticket alone; the topic drops whole words to leave the context gauge its room

## 0.15.3 (2026-10-26)

- Runbook space: a step with a command gets a copy button, which puts it on the clipboard

## 0.15.2 (2026-10-26)

- Runbook space: no command column, the step takes its room; a step's command still goes to the session that runs it

## 0.15.1 (2026-10-26)

- README: a space's icon escape as JSON reads it
- terminal-browser's agent-browser daemons stop with their browser: prune-browsers runs at start and on each closed pane, and doctor counts the ones left

## 0.15.0 (2026-10-26)

- Pinned spaces of your own: `spaces` takes a name or { name, cwd, icon, key }, kept on top, named and shown in the sidebar like the built-in ones. A space already opened under that name becomes the pinned one; setup offers them and binds their key, doctor checks the sidebar shows them

## 0.14.2 (2026-10-26)

- A /bg job is told by its CLAUDE_JOB_DIR, since job folders are named by the short session id

## 0.14.1 (2026-10-26)

- A background job started with /bg keeps its pane's rows and heartbeat; only helper sessions without a job of their own stay out of it

## 0.14.0 (2026-10-26)

- Worktrees: a session follows the checkout git says it is in, herdr's worktrees and Claude Code's included, with the repo named after its origin, and knows it from its own folder at start. The inbox opens a PR's branch where it is checked out, focusing the space already on it, or with worktrees on in a new herdr worktree; a merged branch's worktree space offers to remove the worktree

## 0.13.0 (2026-10-25)

- Background and headless Claude sessions stay out of their parent pane's sidebar row and heartbeat; old heartbeats go on /clear and after ten minutes, and doctor tells a leftover instance from the session. The inbox shows what each ticket cost, recorded per session. The mod shares its resolved config in ~/.cache/cockpit/config.json, which the apps and scripts read instead of their own defaults and migrations. Activity lines are appended, the inbox parses its caches only when they change, pin-spaces lists the sidebar once per event, and Linux notifications get their buttons

## 0.12.1 (2026-10-25)

- A session's timers stop when it ends, so the conversation before a /clear no longer keeps fetching and beating beside the new one

## 0.12.0 (2026-10-25)

- README: one animated screenshot per feature in a table, and every supported tool listed. New animations of the sidebar, the footer filling with links and an E2E test opening terminal-browser beside the agent, on real screenshots of the fake app; inbox, to-do and meetings step through their rows; all screenshots regenerated from the current code
- README opens its features with the whole cockpit at once, animated: an agent on a ticket, the page it tests in terminal-browser, CI on the sidebar and links filling the footer
- README: overview, then install, then features
- One tagline everywhere: one space with everything, for Claude Code users who switch context all day. README, marketplace and plugin descriptions match the GitHub description
- README says what cockpit is, a Claude Code plugin and a herdr plugin working as one, and what it adds to herdr
- Tickets match their branch as a whole key and only known team keys name a ticket; shared files are written atomically or one file per entry, the cross-session lock stays fresh while held, and stale requests no longer run hours later. The Claude Code /inbox pane goes (herdr's inbox app does it all), register.tsx splits into herdr, inbox, sidebar, footer and work modules, views poll without re-reading every spec, and the herdr inbox reuses a space already on the same work and offers to close one whose branch merged. Tests and a check workflow for both halves

## 0.11.0 (2026-10-24)

- README rewritten short: install, what it works with, one paragraph per feature. Front checks become E2E tests, set by e2eBrowser (terminal-browser or chrome); the before/after screenshot is the real flow on a fake app

## 0.10.0 (2026-10-24)

- Tickets, CI and calendar through any connector: config maps each capability to a tool and the fields to read, like the platform; Linear, Google Calendar and CircleCI map by themselves and CI defaults to GitHub checks. Footer chips for 31 sites from one table: Jira, GitLab, Teams, Trello, Sentry, Confluence and more

## 0.9.0 (2026-10-24)

- Sandboxes become previews and Argo links dashboards, named after their site; works out of the box with any platform connector setup maps

## 0.8.1 (2026-10-24)

- Deploys and sandboxes through any platform connector: config maps each capability to a tool and the fields to read, setup writes the mapping

## 0.8.0 (2026-10-24)

- Links and footer chips open in the default browser; terminal-browser is for an agent's front checks. README rewritten as a feature showcase, with fresh screenshots, the split agent rows, notification buttons, settings and before/after

## 0.7.8 (2026-10-24)

- Notifications wait for their buttons to register before posting

## 0.7.7 (2026-10-24)

- Notification buttons: Review, Why? and Rerun, Plan, Join, ArgoCD

## 0.7.6 (2026-10-24)

- Ticket and Notion chips open their page in the browser beside the agent, like the other links

## 0.7.5 (2026-10-24)

- Open the page beside the agent in the default browser: a footer chip and prefix+f

## 0.7.4 (2026-10-24)

- Before/after screenshots at phone, tablet and desktop sizes for PR descriptions, shown side by side; browsers beside an agent out of view for an hour close

## 0.7.3 (2026-10-24)

- Agent rows split like a browser's split view when a pane sits beside the agent: topic │ page +N

## 0.7.2 (2026-10-24)

- Pinned spaces matched by exact name, so a space named terminal-browser stays an agent space; agent rows show the browser tabs and panes beside them

## 0.7.1 (2026-10-24)

- /cockpit settings pane, front checks in the pane or Chrome, device sizes in side-browser; doctor and setup recommend auto transport, 60 fps and Ghostty tip

## 0.7.0 (2026-10-23)

- terminal-browser is optional and every link goes through it beside the agent, keyboard kept, pages reused; the agent drives it with side-browser, sees its page and follows sandboxes on ArgoCD; show opens browser pages; doctor checks what makes it slow

## 0.6.9 (2026-10-23)

- Links open in terminal-browser beside the agent instead of a Chrome window per space; readers keep to Linear and Notion

## 0.6.8 (2026-10-23)

- Footer chips read tickets, PRs, pages, threads and frames into a pane; show gains pictures, sequence and Mermaid diagrams, live Metabase, Datadog, log and CI blocks, and questions answered back to the agent

## 0.6.7 (2026-10-23)

- Refresh the README and screenshots, with the show tool animated
- Show draws flow charts: nodes and edges laid out, routed and looped back in the pane

## 0.6.6 (2026-10-23)

- Show panes hide to a footer chip that brings them back; the footer drops the meeting and context use

## 0.6.5 (2026-10-23)

- Add show: agents open charts, tables and diffs in a pane beside them, with a reviewr button on diffs

## 0.6.4 (2026-10-23)

- Meeting briefs trust the live inbox over older notes
- Apps reach the scripts through the stable link, so they survive an update
- Thorough meeting briefs again; a done button on runbook steps
- Apps restore the terminal when an update stops them
- Start the space apps on a clean prompt
- Apps share one view layer; inbox requests are checked, and runbook steps report how they went

## 0.6.3 (2026-10-23)

- Faster meeting briefs: context gathered up front, medium effort

## 0.6.2 (2026-10-23)

- Keep meeting briefs to 15 short lines
- Write meeting briefs only on request

## 0.6.0 (2026-10-23)

- Screenshots of the recap, sandbox buttons and same-branch warning; shorter README
- Only report a published version newer than the installed one
- Add meeting briefs

## 0.5.0 (2026-10-23)

- Add deploy watch, sandbox create and stop, same-branch warning, recap popup and meeting prep

## 0.4.1 (2026-10-23)

- Keep brief lines short and skip CI failures already listed

## 0.4.0 (2026-10-23)

- Add the morning brief

## 0.3.0 (2026-10-22)

- Say why a notification was not shown
- Show the notifications and the runbook done animation in the README
- Add /cockpit:doctor, update notices, runbook refresh on deploy, install pruning and Linux support

## 0.2.0 (2026-10-22)

- Notifications with the cockpit icon and a click that leads to the PR, ticket, space or runbook

## 0.1.3 (2026-10-22)

- Add a logo, plugin icon and screenshots of each space

## 0.1.2 (2026-10-22)

- Reach the herdr plugin through a stable link that follows its updates

## 0.1.1 (2026-10-22)

- Leave herdrPluginRoot empty in setup, report space counts only from the space loops

## 0.1.0 (2026-10-22)

- Initial release
