# Getting started

Ten minutes from nothing to a cockpit showing your own work.

## 1. What you need

- macOS or Linux (x86_64 or arm64).
- [herdr](https://herdr.dev) 0.9 or later with a Nerd Font, the Herdr desktop app, or both. cockpit draws its
  spaces inside one of them, so this is the one thing you cannot skip.
- `gh`, signed in (`gh auth login`), and `jq`.

Everything else — tickets, CI, meetings, mail, deploys, the release monitor — is optional and comes from MCP
servers you already use. The first setup can leave all of them out.

## 2. Install

In Claude Code:

```
/plugin install cockpit --marketplace quaresc/cockpit
/cockpit:setup
```

With any other agent, in a shell:

```sh
curl -fsSL https://raw.githubusercontent.com/quaresc/cockpit/main/install.sh | sh
cockpit setup
```

The shell installer checks the three things above before it downloads anything, and says what is missing.

## 3. Connect your work

```sh
cockpit setup
```

Setup asks how far to go. **Just my pull requests, to-do and runbooks** needs nothing connected — GitHub comes
through `gh` — and is done in about a minute. The other answer walks through tickets, CI, meetings, mail, deploys
and the monitor, connecting each to a server you already use. Run `cockpit setup` again whenever you want to add
a source.

Your work fills the spaces within a minute.

## 4. Move around

| Key | Space |
| --- | --- |
| `prefix+i` | Inbox — PRs, reviews, tickets, CI, previews |
| `prefix+d` | To-do |
| `prefix+u` | Runbook — the steps of the work in hand |
| `prefix+m` | Meetings |
| `prefix+o` | Monitor — what each release did to production |
| `prefix+y` | Routines |
| `prefix+space` | The palette: everything, searchable |

In every space: `j`/`k` move, `Enter` does the main action, `/` filters, `q` leaves. The row you are on shows its
actions with their keys, and the palette lists each space with the key that opens it.

In the Herdr app the same spaces are `⌘⌥1` to `⌘⌥7`, and Home shows which agents are waiting for you.

## 5. Day to day

- `cockpit doctor` checks every part and names the fix for anything wrong.
- `/cockpit` in Claude Code flips the settings worth changing often; everything else is in
  [configuration.md](configuration.md).
- cockpit tells you when a new version is out; `cockpit update` updates every part.
- Something not working? [troubleshooting.md](troubleshooting.md).
