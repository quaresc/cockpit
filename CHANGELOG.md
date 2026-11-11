# Changelog

Each version is tagged `v<version>` and has a GitHub release with these notes, both made by the publish workflow once
`plugin/.claude-plugin/plugin.json` reaches main with a new version. Both halves, the Claude Code plugin and the herdr
plugin, share the version with the apps (`herdr/tui/Cargo.toml`), whose binaries the release builds; the section for it
must be here first.

## 0.40.3 (2026-11-11)

- Remote desk: the review panel sends comments and runs walkthroughs on the runner (#44)
- Cockpit's sidebar lists every machine, not only those with spaces (#28)

## 0.40.2 (2026-11-10)

- Update tag: what the published cockpit changes, and a click that installs it (#39)
- Agents: a finished Claude session lands in Done until you open it (#41)

## 0.40.1 (2026-11-10)

- A ticket mapping that holds its own `states` is no longer asked for a `listStates` tool, so `cockpit doctor`
  stops warning about one the GitHub Issues preset does not need (#37).
- `cockpit` on PATH is linked through `~/.config/cockpit/herdr`, not an install folder the next update prunes: a
  link left pointing nowhere goes back on it, and `cockpit doctor` says when one does (#37).

## 0.40.0 (2026-11-10)

- Inbox: do not duplicate stacked PRs at the section root (#29)
- Dashboard tiles wrap instead of overflowing their card (#31)
- Remote: connect Herdr to a Cockpit server (#30)
- Inbox: the age test freezes the clock before reading it (#33)
- Remote: a hub listening off loopback answers reads only (#35)
- Remote desk: Go, Peek and Open beside reach the runner's terminal (#34)

## 0.39.3 (2026-11-09)

- A chart's unit keeps a measured quantity in its unit (#25)

## 0.39.2 (2026-11-09)

- The GitHub Issues preset maps itself, so naming GitHub's server is enough for your open issues to reach the inbox,
  with reading, closing, assigning and filing an issue all mapped (#23).
- A tickets mapping can name an issue by several values: `idPattern` reads owner, repo and number out of a ticket's URL,
  `idTemplate` writes its id as `owner/repo#number`, and `teamPattern` splits the repo a new issue is filed in (#23).
- A mapping with no tool to list the states a ticket can move to holds its own `states`, which is how a GitHub issue
  closes from the inbox (#23).
- A shorter README, with the same picture in `app/` for the Herdr gallery (#21).

## 0.39.1 (2026-11-09)

- The Google Calendar and Gmail presets connect: cockpit opens with the newest MCP protocol version that still has a
  handshake, which Google's servers accept, and setup mentions Google's Workspace Developer Preview Program (#18).
- `calendar.calendars` lists the calendars to read, not only the primary one: each meeting keeps its calendar, so
  meeting briefs can read the full event (#19).

## 0.39.0 (2026-11-06)

- A UX pass over the first ten minutes: `install.sh` checks herdr, `gh` and `jq` before downloading, `cockpit setup`
  offers a one-minute path with nothing to connect, and `cockpit` is on PATH after `/cockpit:setup` (#15).
- `?` opens a keys overlay in every space: the row's actions, how to move, and the `prefix+…` chords, built from each
  app's own actions so it cannot drift; `m` there gives the mouse to the terminal (#15).
- New docs: getting started, configuration, the `show` blocks and troubleshooting (#15).
- The main action, toasts and CI tone carry a mark, not only a colour, and empty states name your agent rather than
  Claude Code (#15).
- `monitor.serviceReplace` renames a platform's service to the name Datadog knows, so its queries, monitors and APM
  link resolve (#12).
- The hub reads the last line of `gh auth token`, so a mise or asdf shim's tool line no longer breaks every GitHub call
  (#14).

## 0.38.5 (2026-11-05)

- The status bar shows as many of the agent's links as fit, the rest in "+N", and GitHub's own checks read as CI beside
  the GitHub PR.
- A new gallery preview with every space.

## 0.38.4 (2026-11-05)

- The mail space lists your inbox as a mail client does: the latest 15 unread above the latest 15 read, "N unread"
  everywhere instead of "need you". An opened mail stays in place until you open another or close it, then the latest
  unread is selected; See is now Open.
- Web page (`h`) shows a mail in the browser with its layout and images (terminal-browser beside it in herdr); the
  reading panel splits a mail's layout tables into paragraphs and leaves out its hidden preview text.
- Snippets and subjects read as text (`haven't`, not `haven&#39;t`), and a longer listing no longer announces older mail
  as new.

## 0.38.3 (2026-11-05)

- An idle hub exits again on a machine with cockpit's spaces open: only the space apps under its own HOME keep it alive,
  and a hub whose socket is gone or taken over exits at once, so test and sandbox hubs no longer pile up.

## 0.38.2 (2026-11-05)

- Mail actions are instant: archive and mark read change the list at once while the mail server or a Claude session's
  connector does it in the background, and a failure puts the mail back; no toast about a Claude session any more.
- Opening a mail marks it read.

## 0.38.1 (2026-11-04)

- Mail opens at once: each refresh also reads the threads of the listed mail ahead (a few at a time, again when a reply
  lands), through the hub's mail server or Claude Code's connector.
- Long threads load: an answer too long for Claude Code, which it saves to a file, is read from that file.
- Claude sessions take requests within a second, so a mail opened before it was read ahead loads in a couple of seconds.

## 0.38.0 (2026-11-04)

- Every agent's links in the Herdr app's status bar, named after their tool and always in the same order: GitHub (its
  PR), Linear (its ticket), CircleCI, Argo, Argo Workflows, then the pages its conversation cites (Slack, Datadog,
  Figma, Notion…), read from any agent's transcript; a "+N" menu past the room. Its documents close the bar.
- The side panel's Show is now Documents; the status bar is taller, its links buttons apart from the branch.
- herdr's palette lists the agent's links and documents first when opened over it.
- Claude Code's footer names its links the same way and leaves them to the status bar in the Herdr app
  (`"footer": {"links": true}` brings them back).

## 0.37.1 (2026-11-04)

- Opening a mail no longer loads forever when a Claude Code session on an older plugin takes the request: requests are
  `.req` files older plugins never read, a session leaves a type it does not know to a newer one, and the hub restarts
  its fetcher session when cockpit updates.

## 0.37.0 (2026-11-04)

- A Mail space: unread mail split into what needs you and updates, each thread rendered as HTML beside the list (a
  double click or See), with reply, archive, mark read and open in mail; new mail notifies as before, and the hub reads
  it through its own MCP server when it reaches one (`prefix+shift+e` in herdr, `cmd-alt-7` in the Herdr app).
- Gmail and Google Calendar through Google's own MCP servers with an OAuth client you create (`cockpit setup` walks
  you through it), so neither needs Claude Code.
- Fresher everywhere: GitHub and CI every 30 s, tickets, mail and deploys every minute, each tunable in
  `config.refresh`; moving or assigning a ticket shows at once, and any action refreshes what it touched.
- Update notifications for every agent, from the hub; `cockpit update` updates every part you have.
- Pinned apps reorder by drag and drop in the Herdr app's sidebar; herdr's pinned spaces follow.
- Mute an app in the monitor (`m`): it is neither watched nor announced until you unmute it.

## 0.36.1 (2026-11-04)

- The Herdr app's Spaces fold like the agent groups: a chevron on each space hides what runs in it, and a header button
  hides or shows them all.
- Mail through Claude Code's Gmail connector (or another mail connector), for a mail server cockpit does not reach
  itself: `mail: {"enabled": true, "server": "claude.ai Gmail"}` maps Gmail on its own; archive and mark read go through
  a Claude session, and `cockpit doctor` names the allow rules mail needs.

## 0.36.0 (2026-11-04)

- Every source and action works with any tool and any agent through cockpit's own MCP servers:
  - pull requests from GitLab, Bitbucket or any code host (`code`), merged, readied and drafted through it;
  - mail: unread messages in the inbox and on Home, archived, marked read, or answered with a reply an agent drafts
    for you to send (`mail`, for Gmail, Outlook, Fastmail or any mail server your agent maps);
  - the CI logs panel reads another CI's logs through its server.
- `cockpit_read` and `cockpit_health`, two read-only tools for every agent (through `cockpit mcp` and the Claude
  plugin), read any source and a service's health; meeting briefs, deploy watches, runbook refreshes and CI
  investigations use them, so they need no connector of the agent's own.
- The sidebar lists what runs in each space, agent or terminal, with its state, in the Herdr app and in herdr
  (`$term_1`…`$term_4` rows); the inbox space's counts, unread mail included, show in herdr without Claude Code.
- The platform's own name for you (`platform.login`) lists your previews without a GitHub login.
- Tests run the hub against a fake MCP server for every source, request and ticket action.

## 0.35.0 (2026-11-04)

- The release monitor and live chart panes work with any metrics and errors tool, not only Datadog: an `observability`
  mapping (`metric`, `monitors` and `issues` verbs, the errors, hits and p95 queries in the tool's own language) feeds
  each release's verdict, its monitors and open errors, and show blocks take `source: {metrics: {queries, from?}}`.
  `cockpit setup` offers Datadog and Sentry, and has your agent map Grafana, Prometheus, New Relic or any other server.
- Tickets gain `assign`, `create` and `details` verbs: assigning a ticket, filing one and its card go through the
  hub's ticket server, for any tracker, without a Linear token or an agent session. Linear's mapping has them built in.
- `cockpit connector tools <server> --json` lists each tool's input schema, `cockpit connector ticket <server> <id>`
  shows a ticket's card.

## 0.34.1 (2026-11-04)

- Show panes and the doctor call Metabase's MCP tools by their current names (`run_saved_question`, `execute_query`)
  and read its answers' columns and rows.

## 0.34.0 (2026-11-04)

- cockpit's hub is an MCP client: tickets, CI runs, meetings, deploys and previews, the release monitor and the show
  panes' Metabase and Datadog sources come from your MCP servers with any agent, no Claude Code needed. A `servers`
  block in `config.json` names each one (a URL, a URL with headers or `bearerCommand`, or a local command), read with
  the same mappings as the Claude plugin's connectors.
- `cockpit setup` asks where each source comes from: servers your agents' configs already hold, the ones cockpit
  knows (Linear, Jira, GitHub Issues, GitLab, CircleCI, Datadog), or Claude Code's connectors. It logs cockpit in once
  through your browser (OAuth, kept in the macOS keychain) and has your default agent map a server it has no mapping
  for, trying the mapping on the real server.
- `cockpit connector login|logout|tools|call|tickets|map|rows` to log in, explore a server and map it by hand.
- The hub carries out the inbox's requests (CI reruns, preview create, redeploy, stop and extend, marking a ticket
  done) and ticket moves through those servers, announces tickets going live in production, and warns of meetings
  five minutes ahead where no Claude Code runs.
- The hidden Claude Code session only starts for sources the hub cannot reach itself; `cockpit doctor` says which
  server each source uses and whether cockpit is logged in.

## 0.33.1 (2026-11-04)

- After the shell install, `cockpit app install` finds the Herdr app layout and mod: paths follow the
  `~/.local/bin/cockpit` link to the install folder.
- `cockpit setup` merges cockpit's keys and sidebar into herdr's config.toml itself, keeping your own settings and a
  dated backup, then reloads herdr.
- `cockpit doctor` speaks to every agent, and asks for Datadog or Metabase permission rules only of people who set
  them up.

## 0.33.0 (2026-11-04)

- Every agent Herdr supports, not only Claude Code: cockpit starts, resumes, forks and runs headless Codex, opencode,
  Copilot, Cursor and the rest from one table (`agents.default`, `agents.commands` in the config), and reads Claude,
  Codex, pi, omp, Qwen and Copilot sessions for search, recaps and context.
- `cockpit mcp` gives any MCP agent the show, search, changes and report tools; `cockpit agents connect` adds it to
  each installed agent's config, and a Codex plugin ships in `codex/`.
- Install without Claude Code: `curl -fsSL https://raw.githubusercontent.com/quaresc/cockpit/main/install.sh | sh`,
  then `cockpit setup`.
- The Herdr app layout says how to install cockpit when it isn't, and keeps listing Herdr's agents meanwhile.
- GitHub Actions reruns go through `gh`; actions only the Claude Code plugin can carry out are offered only with it.
- Links, pictures and the clipboard work on Linux.
- Third-party notices ship with every release; CONTRIBUTING, SECURITY and a code of conduct for contributors.

## 0.32.0 (2026-11-03)

- An editor beside the agent in the Herdr app: syntax highlighting, ⌘S saves (refused when the file changed on disk),
  the agent's edits show live, Markdown with a preview. "Ask the agent" sends a note about the file or some lines.
  `e` in Review opens the file under the cursor. Ported from yama-lei/herdr-editor-layout.
- Review: typing a comment no longer runs the panel's shortcuts.
- The site shows the editor, the diff, comments sent back and the walkthrough.

## 0.31.0 (2026-11-03)

- Agents draw with the show tool on their own: a trend, a breakdown, a share or a process becomes a chart or a diagram
  beside the session, unasked. Opening a hand-written HTML page beside it or publishing an artifact is the fallback,
  refused until show was tried, the person asked for one, or a `COCKPIT_SHOW_FALLBACK` reason is given.
- More to draw: `stats` figure tiles (delta, sparkline), area and donut charts, and bars stacked, to 100% or lying
  down with their values written; native in the Herdr app, on the browser page too.

## 0.30.1 (2026-11-03)

- A branch whose PR already merged, still checked out, no longer gives a session its ticket, PR or CI.
- The space renamer never sets an empty name, which Herdr reads as "after its folder".

## 0.30.0 (2026-11-03)

- Home is a dashboard: KPI tiles, the next move (an agent waiting on you), agents by stage, your PRs, then Up next,
  Apps and Activity on the side. Needs you, working, ready and done count agents only, everywhere.
- The work view and every app sit as floating cards; agent cards show their processes' CPU and memory with a sparkline.
- Live blueprint: an agent keeps a Mermaid plan in a show pane (done and active boxes), and a click on a box sends a
  change request about it back to the agent.
- Tabs and spaces name themselves: a tab after its agent's task, a space after its ticket or repo; names you typed stay.
- The hub keeps a hidden Claude session for what only MCP reaches, and reads the calendar itself from `calendar.ics`
  (a secret iCal address); "$.mcp.call is not available" never reaches the inbox, CI or meetings again.
- Routines: a run's output reads as Markdown, with a window to read and copy all of it.
- Pages open in the browser in front: they open from the Herdr window, not the hub.
- Home lists the agents only the hub sees too, such as a Claude session attached from the background.

## 0.29.0 (2026-11-02)

- Inbox in the Herdr app, by kind: alerts, reviews, your PRs, tickets and what to clean up, each its own section.
- Home: a stage's cards side by side, the empty stages on one line instead of blank columns.
- A Claude Code session attached from the background (a job) shows as an agent, working or idle from its title.
- The routines view flags a Claude routine with no tools and no mode: unattended, its gh, git and file calls are refused.
- `cockpit app install` finds the mod again when run through the `~/.config/cockpit/herdr` link.

## 0.28.0 (2026-11-02)

- Reviewing a PR keeps to that PR: its session's branch, PR and ticket are the reviewed PR's, not those of the checkout it
  runs in.
- In the Herdr app, an action (review, plan, talk, fix CI) opens a tab in the space of its repo, else a space named
  after it, instead of an `actions` space.
- Home: one short line per pinned app (inbox, next meeting, runbooks, to-do, prod, routines), each a click away.
- The sidebar drops the work tile: a space or an agent brings the work area back.

## 0.27.1 (2026-11-02)

- A background job (it has a job folder) never fetches: its MCP calls cannot run there, and it filled the inbox with
  "$.mcp.call is not available" errors.
- MCP calls that cannot run yet no longer show as errors in the inbox (tickets, previews, deploy status); they are
  retried within a minute.

## 0.27.0 (2026-11-02)

- Inbox, quieter: no tiles, one line per PR and review like the TUI, each person in their own color, the reference a
  link that opens it, and the details beside the list only on demand (tab).
- Home keeps the greeting, the next move and the board; the stat tiles, project chips and weekly delivery are gone.
- Status bar: the focused agent's links on the right (branch, ticket, CI, PR); no quota, clock or hub unless it is
  offline.
- Every pinned app shows a bubble in the sidebar: inbox (needs you), meetings (left today), monitor (alerts), routines
  (runs to look at), to-do.
- A website, quaresc.github.io/cockpit, holds the features, shortcuts, configuration and changelog; the README is short.

## 0.26.2 (2026-11-02)

- An update stops the hub only once the link names the new folder: a Herdr window reconnecting at once restarted it on
  the old binary.
- What cockpit types into an app terminal (`/reload-plugins` on update, resume lines, answers, queued prompts) goes
  untagged: run from a terminal, `hc send` prefixed it with its sender, so commands did not run.

## 0.26.1 (2026-11-02)

- Forks, recaps, routines, runbook refreshes and walkthroughs run the Claude Code you run (`COCKPIT_CLAUDE`, else the
  local install, else the native one, else PATH): a stale `claude` on PATH ran them on an old version and model.
- A session that cannot call MCP yet (still starting) no longer writes an error over CI and meetings, nor wipes the CI
  runs: it leaves the fetch to the next tick or another session.
- Home: `1–9`, `t` and `⏎` act on the next move's agent while no card is picked, as its hints say.
- Release notes keep each changelog bullet on one line.

## 0.26.0 (2026-11-02)

- cockpit runs in the Herdr desktop app: a `cockpit` layout and mod drawn natively by Herdr, on the same hub as the
  herdr half, both at once. `cockpit app install` puts them in place (setup and update do it), merges the keys and
  switches the window to the layout once.
- The hub also listens on 127.0.0.1 with a token (`~/.cache/cockpit/hub.json`) and serves each app's view model, built
  by the app's own Rust code, so both halves show the same rows; requests name their frontend, and everything that
  opens, focuses or types into a window goes to the right one.
- Sidebar in place of Herdr's lists: agents that need you, answered in place (single, multi or free text), agents by
  ticket with PR, CI and changes, Claude quota bars with a pace tick, spaces, and the apps with their counts.
- Home: today's numbers, the next move, the agents and PRs by whose move it is (Needs you, Working, Ready to land,
  Done), the week's delivery. A status bar, a bell, a side panel (Show, Review, Runbook, Files, Changes) and the
  focused agent alone (⌘⌥A).
- Every app as a GUI view with all of its TUI keys and actions, plus buttons, hover cards, right-click menus, drag to
  reorder, sortable tables and side panels; Monitor and Routines as dashboards.
- Inbox: merge readiness and merge, auto-merge, ready and draft, a ticket or PR started in a worktree with an agent
  after safety checks, Linear status and assignee, PR alerts by diff, uncommitted work and merged worktrees to clean.
- Agents: the hub's `agents` view (question, recap, whose move, ticket, PR, CI, cost) with answer, recap, snooze, seen
  and fork; an agent-time ledger per ticket; a recap popup (⌘R) by stage; a prompt queue per agent; agents resumed
  after a restart, in both halves.
- Review: the focused agent's changes (uncommitted, branch or PR) commented line by line and sent back as one
  message, or read as a guided walkthrough.
- The palette in the app (⌘⇧P) is Herdr's plus cockpit's, on Herdr's own geometry: every command with its key,
  layouts, spaces, tabs and agents, cockpit's commands, the doctor's report and today's brief, and the hub's search.
  Over a Claude agent a found item goes into its prompt, is asked about or looked into by a fork; the session itself
  forks to a tab or the right, and its folder opens in your editor (`editor` in the config; Remote-SSH for a remote
  space).
- Notifications in the app are Herdr's toast while the window is in front and the system's otherwise; either click
  opens the row the notification is about (a PR or ticket in the inbox, a runbook, a routine), where its buttons are.
- `cockpit focus inbox:<PR url|ticket id>` selects that row in the inbox, in both halves.
- An agent's recap is asked of haiku a minute after its turn ends when nobody looked at it (`agents.recap: "manual"`
  turns that off).
- herdr: `prefix+shift+a` opens the agents popup, `prefix+shift+q` the prompt queue of the agent in view and
  `prefix+shift+u` resumes the agents a restart stopped; a diff in a show pane sends several comments as one review
  (`S`).
- With both halves running, a notification about no session goes to the one in front.
- `docs/screenshots/app_shots.sh` shoots the app's screens in a sandbox window, dark and light, normal and narrow.

## 0.25.0 (2026-10-29)

- Lightest routines no longer give up when their MCP server is still connecting at the start: their system prompt names
  the tools to load and says to try again, and a run that calls none of its listed tools runs once more, then fails
  as "called none of its tools" instead of passing.
- The routines space flags a lightest routine whose file lists no tools, or a tool no connected MCP server has.
- `cockpit routine run <slug>` runs a routine now and waits: how it went, the tools it called, its output. `/routine`
  resolves every id and exact tool name before writing a lightest routine, then tries it (asking first when it sends
  or changes something).

## 0.24.0 (2026-10-29)

- `model: lightest` routines run on a one-line system prompt instead of Claude Code's, and the MCP servers they do not
  call are not connected at all (`deniedMcpServers`), so neither their tools nor their instructions are read: a
  routine calling one MCP tool reads 8 to 14k tokens instead of about 33k.
- The servers the lightest routines leave out are learnt from every run and never forgotten by one that connected
  fewer.

## 0.23.1 (2026-10-29)

- Routines show what their latest run cost, not the average of all their runs, which a change of model or setup made
  stale.

## 0.23.0 (2026-10-29)

- Routines pick their model: `m` on a routine steps through default, lightest, haiku, sonnet and opus, and the form
  that adds one has the same switch.
- `model: lightest` is the cheapest run: haiku at low effort, its listed tools alone, no skills, and every MCP server
  it does not call denied, from the servers the last run reported. A routine calling one MCP tool reads about 30k
  tokens instead of 80k.
- Claude routines run with `--output-format stream-json`, which tells the hub the MCP servers Claude Code connected.

## 0.22.0 (2026-10-29)

- Routines: each one shows what a run costs on average, Claude's estimate (on a Claude plan runs draw on the plan,
  not dollars), and the panel what its runs cost this week.
- `budget` is now dollars a week: runs wait once the last seven days reach it (one notification says so), and a run
  stops at what is left of it.
- Claude routines run lighter: in a folder of their own unless `cwd` says otherwise, so no project's CLAUDE.md or memory
  is read, and a routine listing its tools gets those built-in tools alone and no skills unless it lists `Skill`.
  A run calling one MCP tool reads about 80k tokens instead of 110 to 150k.
- A run stopped by its budget or turn limit says so in words.

## 0.21.0 (2026-10-29)

- A routines space (opt-in: `routines` in `spaces`, `prefix+y`): Claude prompts and shell scripts in
  `~/.config/cockpit/routines/` that the hub runs on a schedule, every so often or on an event (CI failing, a review
  requested, a release that moved…), three at a time, a missed run caught up once within 12 hours.
- Each run keeps its answer, cost and Claude session: run, stop, pause, edit or delete a routine, read a run's output
  or resume its session in the actions space; a notification when a run ends, with Open and Talk.
- `a` in the space adds a routine in a form that reads its schedule back in words; `/routine` asks Claude to write one.
- The hub's events carry a kind, the palette finds routines and their runs, and the hub stays up while a routine waits
  for its time.

## 0.20.1 (2026-10-29)

- A notification click brings forward the very Ghostty window and tab where herdr runs, not just Ghostty (#11).
- Doctor and setup ask for Ghostty 1.4.0 or newer rather than its tip builds, now that a stable release reads herdr's
  frame files (#11).

## 0.20.0 (2026-10-28)

- `prefix+space` opens the palette as a popup over any pane: search everything, jump to a space, check cockpit, read
  the brief; over a Claude session, put a reference in its prompt, ask it about an item or fork it (#10).
- ctrl+p searches a week of what changed (reviews, CI, tickets, merges, deploys, releases, alerts, errors) and finds
  past sessions by meaning as well as words, each with the passage that matched (#10).
- The inbox and the monitor mark the rows that changed since you last left them, with a line saying what (#10).
- `cockpit search`, `cockpit events` and the cockpit_search and cockpit_changes tools let Claude ask the same (#10).

## 0.19.1 (2026-10-28)

- Monitor: Datadog's empty answers no longer show as "JSON Parse error", services with many monitors load in pages
  under Claude Code's answer limit, and p95 only flags a release when it grows by 100 ms as well as 1.5x (#9).
- Monitor app: full app names, fetch errors on one line, "monitors ?" when a watched app's monitors failed (#9).

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
