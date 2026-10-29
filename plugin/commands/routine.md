---
description: A routine the cockpit hub runs on its own, on a schedule or when something happens, shown in the routines space
argument-hint: "<when and what, in your words>"
---

Write the routine $ARGUMENTS describes (when empty: ask me what it should do and when) as a file in `~/.config/cockpit/routines/<slug>.md`, slug being a few lowercase words of its name joined by dashes, not already taken. To change an existing routine, read its file first and edit it in place.

The file is a front matter of `key: value` lines between two `---` lines, then the prompt Claude runs (or the script, with `run: shell`):

```
---
name: Morning sweep
when: weekdays 09:00
cwd: ~/code/api
model: sonnet
tools: Read, Grep, Bash(gh pr list:*)
notify: failed
---

Sum up my open PRs: which wait for review, which fail CI, which can merge.
```

- `when`: `manual`, `every 30m` (or `2h`, `1d`; a minute at least), `hourly`, `daily 09:00`, `weekdays 9:00,17:30`, `weekends 10:00`, `mon,thu 14:00`, `tue-fri 08:15`, `cron <minute hour day month weekday>`, or `on <trigger> [words]`, which runs on each event of that kind whose line holds every word. Triggers: `review`, `approved`, `changes`, `opened`, `merged`, `ci-failed`, `ci-passed`, `ticket`, `assigned`, `live`, `release`, `release-ok`, `release-moved`, `release-failed`, `alert`, `error`, `any`. Times are local.
- `cwd`: where it runs. Left out, a Claude routine runs in a folder of its own, where no project's CLAUDE.md or memory is read, which keeps each run small; set it only when the routine works in a repo. A script runs in the configured reposDir. `run: shell` runs the body with `sh -c` instead of Claude; it gets `$COCKPIT_EVENT`, `$COCKPIT_EVENT_KEY` and `$COCKPIT_EVENT_KIND` when an event set it off.
- For Claude: `model`: `lightest` for a routine that only calls a tool or two (haiku at low effort on a one-line system prompt, nothing loaded but the tools it lists, every other MCP server left unconnected: the cheapest run, so keep its prompt self-contained), else `haiku`, `sonnet` or `opus`, or left out for my default model; `tools` (comma-separated allow rules, MCP tools by their full `mcp__…` name; a routine that lists them gets those built-in tools alone and no skills unless it lists `Skill`, which keeps each run small; without any it may only read), `mode` (a permission mode such as `acceptEdits`). Nobody answers its questions: give it the tools it needs and nothing more, and write the prompt so it finishes on its own, its reply's first line a summary.
- `budget`: only when I ask for one, dollars a week (`budget: 2`), as Claude estimates its runs; on a Claude plan runs draw on the plan, not dollars. Runs wait once the week's runs reach it, and a run stops at what is left.
- `timeout` (minutes, 30 by default), `notify` (`always`, `failed` or `never`), `paused: true`.

Keep the prompt short and concrete. Never give a routine tools that deploy, delete or write to a production system unless I asked for exactly that. Reply with the file's path, when it runs next in words, and the tools it may use.
