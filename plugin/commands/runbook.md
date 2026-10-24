---
description: Where the work stands (PRs, deploys, prod commands) as a table, also shown in the herdr runbook space
argument-hint: "[ticket or feature]"
---

Build the runbook for $ARGUMENTS (when empty: the ticket this conversation is about): where it stands and what is left, in execution order.

The runbook lives in `~/.config/cockpit/runbooks/<slug>.md`, slug being the ticket key lowercased, else a short feature slug. When that file already exists, read it first and update it in place: keep its step order and numbering, refresh every status from live facts, add the steps that appeared and mark the ones finished.

Gather the facts before writing, never from memory alone: every PR across repos (gh: state, review, CI), whether merged ones are live in production (the deploy status your connected tools give, then `gh api repos/<owner>/<repo>/compare/<merge sha>...<deployed ref>`), the ticket status, and the deploy order, migrations and post-deploy commands from the PRs, the memory notes and the code.

Format:
- first line `# <TICKET> · <feature>`, second line `updated: <YYYY-MM-DD HH:MM>`, the local time from `date`, never a guess
- then one table `| # | Step | Where | Status | Command |`, one row per step: merges, deploys, migrations, prod commands, checks
- Where: `repo#123` for a PR, else the environment (prod, staging, a preview's name) or the repo
- Status: exactly one of ✅ done, 🚀 live, 🟡 in progress, ⏳ waiting, ⬜ to do, ❌ blocked, followed by two or three words
- Command: the exact command in backticks when the step has one, else empty
- optionally `## Notes` with at most three one-line bullets for traps (deploy order, flags, data checks)

Do not run any production command or deploy: list them. Reply with the table only.
