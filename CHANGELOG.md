# Changelog

Each version is tagged `v<version>` and has a GitHub release with these notes. Both halves, the Claude Code plugin and
the herdr plugin, share the version.

## 0.16.3

- Brief lines about several PRs open the GitHub search that lists them all, not just the first PR (#2).
- The calendar connector retries after a missed startup race, and meetings read Google's `Z` timestamps (#1).

## 0.16.2

- The sidebar lists the actions space's Claude sessions under its row, each with its state; one waiting for you shows in
  the accent colour. Setup adds the six `$act_*` rows to the herdr config.

## 0.16.1

- Each runbook in the runbook space gets a **talk** button: it forks the session that wrote the runbook, else the one that
  worked most on its ticket, in a tab of the actions space.

## 0.16.0

- New pinned **actions** space (`prefix+a`): the inbox's review and plan, and a runbook step whose session is gone, open
  as its tabs instead of new spaces. Pressing an action again while it runs brings its tab back.
- **Talk** on every inbox ticket (`t`): forks the session that worked most on the ticket, with what the inbox knows of
  its PRs, CI, deploys, previews and runbook, and changes nothing unless asked.
- **Continue** on a ticket with a live plan tab moves that tab out to a space of its own.
- Notifications, the recap popup, the same-branch warning and closing a session on a merged branch follow the session's
  tab when it sits in a pinned space; closing no longer takes a whole pinned space with it.
