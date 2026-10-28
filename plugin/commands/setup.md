---
description: Set up cockpit - pick the features, write ~/.config/cockpit/config.json, wire herdr
---

Set up cockpit for me. Work through the steps in order, say in one line what each found, and ask before changing any file outside `~/.config/cockpit`.

1. **Prerequisites.** Check `herdr --version` (0.9 or later), `gh auth status`, `jq`, and a Nerd Font in `~/Library/Fonts` or `/Library/Fonts` (a name containing "Nerd"). Report what is missing with its install command and stop only when herdr, gh or jq is missing.

2. **herdr plugin.** Read `~/.config/herdr/plugins.json`. When no entry has `plugin_id` "cockpit", run `herdr plugin install quaresc/cockpit/herdr --yes` and read the file again. Keep that entry's `plugin_root`. Then run `sh <plugin_root>/fetch-apps.sh`: it puts this version's `cockpit` binary, which runs the space apps and the hub, built for this machine, in `<plugin_root>/bin` (for a version whose release has no binary yet it keeps the one an earlier install had, or does nothing on a first install).

3. **Connectors.** Run `claude mcp list` and note the exact names of the connected servers for tickets (Linear, Jira, Trello, Asana, GitHub Issues…), CI (CircleCI, GitLab, Buildkite…; GitHub Actions needs none) and a calendar (Google Calendar, Outlook…), and of a platform connector whose tools report deploys or run per-branch preview environments (an Argo, Vercel or in-house connector), if any.

4. **Features.** Ask with AskUserQuestion, multi-select, preselecting nothing: GitHub PRs inbox, tickets (naming the ticket connector found), CI runs (GitHub checks, or the CI connector found), meetings (naming the calendar connector found), deploys and preview environments through the platform connector, E2E tests in terminal-browser beside the agent, for an agent testing a front change in a browser you watch (installs it with `brew install --cask terminal-browser` when missing, then runs `terminal-browser config set render.transport auto` and `terminal-browser config set render.fps 60`; on macOS with Ghostty, also adds `auto-update-channel = tip` to Ghostty's config and asks me to update Ghostty, since only its tip builds read the frame files herdr hands it). Without it, E2E tests run in Claude in Chrome. Links and footer chips always open in the default browser. Mark the ones whose connector is missing as such in their description.

5. **Values.** Propose each from what you can detect, and ask only what you cannot settle:
   - `reposDir`: the folder holding the git checkouts; look for one with several `*/.git` among `~/code`, `~/src`, `~/dev`, `~/Work/*`, `~/projects`.
   - `github.org`: limits my own PRs to one organization; offer the orgs from `gh api user/orgs --jq '.[].login'` and "none".
   - Tickets: `name` (what the inbox calls the tool: Linear, Jira…), `keys` (the key prefixes a ticket mention may use, such as `ENG` in ENG-123; tickets assigned to me add theirs), and `urlTemplate` (a ticket's page with `{id}`, e.g. `https://acme.atlassian.net/browse/{id}`; leave it empty when the connector's answers carry each ticket's URL).
   - Tickets mapping: with the Linear connector, leave `tools`, `fields` and `statusTypes` out: cockpit maps Linear by itself. With any other, list its tools with their descriptions and input schemas, and map each capability: `listMine` (my open tickets, e.g. a search for `assignee = currentUser() AND statusCategory != Done`), `get` (one ticket with its description and attachments), `comments` (a ticket's comments, read by meeting briefs), `listStates` (the states a ticket can move to: a team's statuses, or an issue's transitions) and `setState` (moves a ticket to one). Write each as `{ "tool", "args" }` with the placeholders `{ticket}` (its key or id), `{team}` (the key's prefix) and `{state}` (the id `listStates` gives). Then call `listMine` and `get` once each (read-only) and fill `fields` with the dotted path of every value in their answers: `tickets` (the list in `listMine`'s answer), `id`, `title`, `url`, `status` (the state's name), `statusType` (its category), `parentId`, `project`, `description`, `attachments` and `attachmentUrl`, and in `listStates`' answer `states` (empty when the answer is the list), `stateId`, `stateName` and `stateType`. Cockpit reads Linear's categories (`triage`, `backlog`, `unstarted`, `started`, `completed`, `canceled`): map the tool's own ones to them in `statusTypes`, e.g. Jira's `{ "new": "unstarted", "indeterminate": "started", "done": "completed" }`. Leave out a capability no tool covers; cockpit skips what it cannot do.
   - each enabled connector's `server`: the exact name from step 3.
   - Platform only: `previewPrefix` (how a preview's id starts, to spot one named in a conversation), `previewDashboardUrl` (the page following a preview's rollout, with `{preview}`), `workflowUrl` (a deploy run's page, with `{namespace}` and `{name}`), and the mapping below. Leave a URL empty when the platform has no such page.
   - Platform mapping: list the platform server's tools with their descriptions and input schemas, and map each capability cockpit uses to one: `deployStatus` (the deployed apps and their versions), `deployHistory` (an app's recent deploys), `listPreviews`, `previewStatus` (one preview's app URLs), `createPreview`, `redeployPreview`, `stopPreview`, `startPreview` and `extendPreview`. Write each as `{ "tool", "args" }`, the arguments from the tool's schema with the placeholders `{appId}` (a deployed app), `{login}` (my GitHub login), `{preview}`, `{app}` (the app a preview runs), `{branch}` and `{days}`. When updating a preview replaces its whole setup, list the settings to send back as they were in `extendPreview.keep`. Then call `deployStatus` and `listPreviews` once each (read-only) and fill `fields` with the dotted path of every value in their answers (`status.health`, `urls.0.url`; `a|b` takes the first that has one), and `skipKinds` with app kinds that never build from a repo. Leave out a capability no tool covers; cockpit skips what it cannot do.
   - CI: `imageJob`, the job or check that pushes the image a preview runs (a GitHub check named `ci/circleci: docker_build` matches `docker_build`), or empty. Without a CI connector, leave `server` empty: cockpit reads the checks of my PRs from GitHub and reruns GitHub Actions with `gh`.
   - CI mapping, with a CI connector only: set `name` and `server`. List its tools with their descriptions and input schemas, and map each capability: `listRuns` (my recent pipelines or runs, newest first), `listWorkflows` (a run's workflows, for a CI that nests them like CircleCI; leave it out otherwise), `listJobs` (a workflow's or run's jobs), `getJob` (one job with its steps), `listTests` (a job's failed tests), `logs` (a job's logs, named in prompts that investigate a failure) and `rerun` (reruns the failed jobs). Write each as `{ "tool", "args" }` with the placeholders `{run}`, `{workflow}` (a run's id when the CI has no workflows) and `{job}`. Then call `listRuns`, `listJobs` and `getJob` once each (read-only) and fill `fields` with the dotted path of every value: `runs` (the list), `runId`, `runRepo` (a repo name or URL), `runBranch`, `runTag` (set on tag builds, which are skipped), `runCreated`, `runPhase`, `runOutcome`; `workflows`, `workflowId`, `workflowName`, `workflowCreated`, `workflowOutcome`; `jobs`, `jobId`, `jobName`, `jobPhase`, `jobOutcome`; `steps` (a `[]` segment flattens nested lists, e.g. `attributes.parallel_executions[].steps`), `stepName`, `stepOutcome`, `stepExit`; `tests`, `testName`. Map the CI's outcomes to cockpit's `succeeded`, `failed` and `canceled` in `outcomes` (e.g. GitLab's `{ "success": "succeeded" }`), and list in `endedPhases` the phase values that mean a run or job ended; with no phase field, a final outcome means ended. The CircleCI connector maps by itself: a `ci` block with `"name": "CircleCI"`, `"server": "circleci"` and no `tools` gets CircleCI's mapping.
   - Calendar mapping: with the Google Calendar connector, leave `tools`, `fields`, `skip`, `only` and `responses` out: cockpit maps it by itself. With any other, set `name` and `server`, list its tools with their schemas, and map `listEvents` (my coming events, with `{pageToken}` for the next page when it pages) and `getEvent` (one event, `{event}`, read by meeting briefs). Call `listEvents` once (read-only) and fill `fields`: `events` (the list), `nextPage`, `id`, `title`, `start` and `end` (date-times; all-day events must have none at that path), `url`, `join` (the video call), `location`, `description`, `series` (the recurring series' id), `myResponse` (my answer to the invite, when the event carries it), `attendees`, `attendeeSelf`, `attendeeResponse`, `attendeeResource`, `attendeeName`, `attachments`, `attachmentTitle`, `attachmentUrl`. List in `skip` the values that leave an event out, by field path (Outlook: `{ "isCancelled": ["true"], "showAs": ["free"], "isAllDay": ["true"] }`), in `only` the values an event must hold to be kept (`""` stands for a missing field; Google: `{ "eventType": ["", "DEFAULT"] }`), and map invite answers to `accepted`, `tentative`, `declined` and `needsAction` in `responses` (Outlook: `{ "tentativelyAccepted": "tentative", "notResponded": "needsAction", "organizer": "accepted" }`).
   - `deployWatch`: on only when an observability connector (Datadog or similar) is connected; `tools` lists its `mcp__<server>` prefix.
   - `density`: ask with AskUserQuestion. `comfortable`, the default, keeps a row's actions in a panel under the list; `compact` fits more rows on screen, with a row's buttons on the row itself.

6. **Config.** Write `~/.config/cockpit/config.json` in this shape, with the values above, `herdrPluginRoot` left empty (the mod finds the herdr plugin in herdr's registry, which follows its updates), `enabled: false` for every feature I did not pick, and `e2eBrowser: "chrome"` when I did not pick terminal-browser. Drop `meetings` from `spaces` when the calendar is off. Ask whether I want pinned spaces of my own (a notes repo, a scratch shell…), offering the spaces `herdr workspace list` shows that are not agent sessions; add each as `{ "name", "cwd", "icon", "key" }`, with `cwd` its folder (reposDir when left out), `icon` a Nerd Font glyph (a folder when left out) and `key` a free `prefix+<letter>` (or none). `runbookTools` lists extra tools the background runbook refresh may use, e.g. a read-only database MCP. `worktrees: true` makes the inbox open a branch checked out nowhere in a new herdr worktree instead of the main checkout. Optionally, for data that stays fresh without a Claude session: with Linear, ask for a personal API key (Linear → Settings → Security & access) and set it as `tickets.token`; with CircleCI, a personal API token (CircleCI → User settings → Personal API tokens) as `ci.token`; the hub then fetches tickets or runs itself. Ask for each in a plain question, say it is optional, write it straight into the file and never repeat, print or log a token (the shown result masks it as `"…"`). GitHub needs none: the hub uses `gh`'s login. Merge into an existing file instead of replacing it, and show the result.

```json
{
  "reposDir": "~/code",
  "github": { "enabled": true, "org": "my-org" },
  "tickets": { "enabled": true, "name": "Linear", "server": "claude.ai Linear", "keys": ["ENG"], "urlTemplate": "" },
  "ci": { "enabled": true, "name": "GitHub", "server": "", "imageJob": "docker_build" },
  "platform": {
    "enabled": false,
    "server": "platform",
    "previewPrefix": "preview-",
    "previewDashboardUrl": "https://dashboard.example.com/previews/{preview}",
    "workflowUrl": "https://dashboard.example.com/runs/{namespace}/{name}",
    "previewDefaults": {},
    "tools": {
      "deployStatus": { "tool": "deploys_status", "args": { "env": "production" } },
      "deployHistory": { "tool": "deploys_history", "args": { "env": "production", "app": "{appId}" } },
      "listPreviews": { "tool": "previews_list", "args": { "owner": "{login}" } },
      "previewStatus": { "tool": "previews_get", "args": { "id": "{preview}" } },
      "createPreview": { "tool": "previews_create", "args": { "app": "{app}", "branch": "{branch}" } },
      "redeployPreview": { "tool": "previews_redeploy", "args": { "id": "{preview}", "app": "{app}" } },
      "stopPreview": { "tool": "previews_stop", "args": { "id": "{preview}" } },
      "startPreview": { "tool": "previews_start", "args": { "id": "{preview}" } },
      "extendPreview": { "tool": "previews_extend", "args": { "id": "{preview}", "days": "{days}" } }
    },
    "fields": {
      "apps": "apps", "appId": "id", "appRepo": "repository", "appLink": "dashboardUrl", "appTag": "version", "appSha": "commit",
      "deployments": "deployments", "deployWorkflow": "workflow", "deployNamespace": "namespace", "deployTag": "version", "deployBranch": "branch",
      "previews": "previews", "previewId": "id", "previewExpires": "expiresAt", "previewActive": "running", "previewHealth": "health",
      "previewApps": "apps", "appName": "name", "appBranch": "branch",
      "statusApps": "apps", "statusUrl": "url", "createdId": "id", "healthy": "Healthy"
    }
  },
  "deployWatch": { "enabled": false, "afterMinutes": 30, "tools": ["mcp__datadog"] },
  "recap": { "enabled": true, "awayMinutes": 10 },
  "meetingBrief": { "enabled": true, "model": "sonnet", "skip": ["lunch", "focus", "break"], "tools": ["mcp__notion__notion-fetch"] },
  "calendar": { "enabled": true, "name": "Google Calendar", "server": "claude.ai Google Calendar" },
  "brief": { "enabled": true, "at": "08:30", "weekends": false },
  "notifications": true,
  "e2eBrowser": "terminal-browser",
  "browserIdleMinutes": 60,
  "herdrPluginRoot": "",
  "spaces": ["inbox", "terminal", "actions", "to-do", "runbook", "meetings"],
  "runbookTools": [],
  "worktrees": false,
  "density": "comfortable"
}
```

7. **herdr config.** Read `<plugin_root>/config.example.toml` and merge it into `~/.config/herdr/config.toml`: keep every key I already set, add the missing ones, and never duplicate a `[[keys.command]]` with the same `key`. The one key to replace is `ui.toast.delivery`: set it to `herdr` so herdr's system popups do not double cockpit's, and say so. In `ui.sidebar.spaces`, the `workspace` token's rules must show every pinned space: add `{ contains = "  " }` before the hiding rule when it is missing. For each space of my own with a `key`, add a `[[keys.command]]` running `sh ~/.config/cockpit/herdr/focus-space.sh <name>`, like the built-in ones. Back the file up as `config.toml.bak-<date>`, show me the diff, write it once I agree, then run `herdr server reload-config`.

8. **Spaces.** Run `sh <plugin_root>/ensure-spaces.sh` then `sh <plugin_root>/pin-spaces.sh`, and check with `herdr workspace list` that the pinned spaces exist.

9. **Notifications (macOS).** Run `sh <plugin_root>/notifier/build.sh` (it needs `swiftc`, from `xcode-select --install`), then `COCKPIT_APP_WAIT=0 sh <plugin_root>/app.sh notify "Cockpit" "Notifications are on" "space:inbox"`, and ask me to click Allow when macOS asks about Cockpit's notifications.

10. **Permissions.** Background refreshes call MCP tools outside any turn, so auto mode needs allow rules for them. Print the block to add to `permissions` in `~/.claude/settings.json`: `allow` with one `mcp__<server>` entry per enabled connector (the server name with every character outside `A-Za-z0-9_-` turned into `_`), plus the read-only tools live panes call: Datadog's `get_datadog_metric` and Metabase's `execute_question` and `query`, each as `mcp__<server>__<tool>` for the connectors I have; and `deny` with the destructive tools of those servers the cockpit never uses, such as `cancel_workflow`, `rollback_deploy_component`, `delete_*`, `create_event`, `update_event`, `delete_event` and `respond_to_event`. Edit the file only if I say so.

11. **Finish.** Tell me that `/cockpit:doctor` checks the whole setup at any time, that Claude sessions started before now load the footer and the background refreshes once restarted, that the inbox space fills its PRs within a minute of opening and the rest within a minute of the first session starting, and list the herdr keys `prefix+i/t/a/d/u/m` and those of my own spaces.
