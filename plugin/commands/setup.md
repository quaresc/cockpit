---
description: Set up cockpit - pick the features, write ~/.config/cockpit/config.json, wire herdr
---

Set up cockpit for me. Work through the steps in order, say in one line what each found, and ask before changing any file outside `~/.config/cockpit`.

1. **Prerequisites.** Check `herdr --version` (0.9 or later), `gh auth status`, `jq`, `python3`, and a Nerd Font in `~/Library/Fonts` or `/Library/Fonts` (a name containing "Nerd"). Report what is missing with its install command and stop only when herdr, gh or jq is missing.

2. **herdr plugin.** Read `~/.config/herdr/plugins.json`. When no entry has `plugin_id` "cockpit", run `herdr plugin install quaresc/cockpit/herdr --yes` and read the file again. Keep that entry's `plugin_root`.

3. **Connectors.** Run `claude mcp list` and note the exact names of the connected servers for Linear, CircleCI, Google Calendar and Backstage, if any.

4. **Features.** Ask with AskUserQuestion, multi-select, preselecting nothing: GitHub PRs inbox, Linear tickets, CircleCI runs, Google Calendar meetings, Backstage deploys and sandboxes, a Chrome window per agent beside the terminal (macOS, iTerm2 and Chrome only). Mark the ones whose connector is missing as such in their description.

5. **Values.** Propose each from what you can detect, and ask only what you cannot settle:
   - `reposDir`: the folder holding the git checkouts; look for one with several `*/.git` among `~/code`, `~/src`, `~/dev`, `~/Work/*`, `~/projects`.
   - `github.org`: limits my own PRs to one organization; offer the orgs from `gh api user/orgs --jq '.[].login'` and "none".
   - `linear.teams`: team keys a ticket mention may use (tickets assigned to me add theirs).
   - each enabled connector's `server`: the exact name from step 3.
   - Backstage only: `stagePrefix`, `sandboxArgoUrl` (with `{stage}`) and `workflowUrl` (with `{namespace}` and `{name}`).
   - CircleCI only: `imageJob`, the job that pushes the image a sandbox runs, or empty.
   - `deployWatch`: on only when an observability connector (Datadog or similar) is connected; `tools` lists its `mcp__<server>` prefix.

6. **Config.** Write `~/.config/cockpit/config.json` in this shape, with the values above, `herdrPluginRoot` left empty (the mod finds the herdr plugin in herdr's registry, which follows its updates), and `enabled: false` for every feature I did not pick. Drop `meetings` from `spaces` when the calendar is off. `runbookTools` lists extra tools the background runbook refresh may use, e.g. a read-only database MCP. Merge into an existing file instead of replacing it, and show the result.

```json
{
  "reposDir": "~/code",
  "github": { "enabled": true, "org": "my-org" },
  "linear": { "enabled": true, "server": "claude.ai Linear", "teams": ["ENG"] },
  "circleci": { "enabled": false, "server": "circleci", "imageJob": "docker_build" },
  "backstage": {
    "enabled": false,
    "server": "backstage",
    "stagePrefix": "preview-",
    "sandboxArgoUrl": "https://argocd.example.com/applications/{stage}",
    "workflowUrl": "https://argo-workflows.example.com/workflows/{namespace}/{name}",
    "sandboxDefaults": { "monitoringEnabled": false }
  },
  "deployWatch": { "enabled": false, "afterMinutes": 30, "tools": ["mcp__datadog"] },
  "recap": { "enabled": true, "awayMinutes": 10 },
  "meetingBrief": { "enabled": true, "model": "sonnet", "skip": ["lunch", "focus", "break"], "tools": ["mcp__notion__notion-fetch"] },
  "calendar": { "enabled": true, "server": "claude.ai Google Calendar" },
  "brief": { "enabled": true, "at": "08:30", "weekends": false },
  "notifications": true,
  "chromeSideWindow": false,
  "herdrPluginRoot": "",
  "spaces": ["inbox", "terminal", "to-do", "runbook", "meetings"],
  "runbookTools": []
}
```

7. **herdr config.** Read `<plugin_root>/config.example.toml` and merge it into `~/.config/herdr/config.toml`: keep every key I already set, add the missing ones, and never duplicate a `[[keys.command]]` with the same `key`. The one key to replace is `ui.toast.delivery`: set it to `herdr` so herdr's system popups do not double cockpit's, and say so. Back the file up as `config.toml.bak-<date>`, show me the diff, write it once I agree, then run `herdr server reload-config`.

8. **Spaces.** Run `sh <plugin_root>/ensure-spaces.sh` then `sh <plugin_root>/pin-spaces.sh`, and check with `herdr workspace list` that the pinned spaces exist.

9. **Notifications (macOS).** Run `sh <plugin_root>/notifier/build.sh` (it needs `swiftc`, from `xcode-select --install`), then `sh <plugin_root>/notify.sh "Cockpit" "Notifications are on" "space:inbox"`, and ask me to click Allow when macOS asks about Cockpit's notifications.

10. **Permissions.** Background refreshes call MCP tools outside any turn, so auto mode needs allow rules for them. Print the block to add to `permissions` in `~/.claude/settings.json`: `allow` with one `mcp__<server>` entry per enabled connector (the server name with every character outside `A-Za-z0-9_-` turned into `_`), and `deny` with the destructive tools of those servers the cockpit never uses, such as `cancel_workflow`, `rollback_deploy_component`, `delete_*`, `create_event`, `update_event`, `delete_event` and `respond_to_event`. Edit the file only if I say so.

11. **Finish.** Tell me that `/cockpit:doctor` checks the whole setup at any time, that Claude sessions started before now load the footer and the background refreshes once restarted, that the inbox space fills within a minute of the first session starting, and list the herdr keys `prefix+i/t/d/u/m`.
