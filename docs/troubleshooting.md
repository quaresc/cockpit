# Troubleshooting

`cockpit doctor` checks every part and names the fix for whatever is wrong, so run it first:

```sh
cockpit doctor          # only what needs doing
cockpit doctor --all    # every check, passing ones included
```

In Claude Code, `/cockpit:doctor` runs it and offers to apply the safe fixes.

What it reports most often:

## The spaces are empty

The hub fills them within a minute of the first app opening. If they stay empty:

- **Nothing is set up yet.** Each space says which of the two it is: *"no mail server is connected · cockpit setup
  adds one"* means waiting will not help, while *"the hub fills this within a minute"* means it is on its way.
- **No binary for this machine.** `cockpit doctor` says so; `sh ~/.config/cockpit/herdr/fetch-apps.sh` fetches it.
  A release that has no binary for your platform yet leaves the spaces empty until the next one.
- **A feature is off.** Tickets, CI, meetings, mail, deploys and the monitor each need a connector picked in
  `cockpit setup`. A space whose feature is off is not pinned at all.

## "The cockpit hub did not start"

The hub runs detached, and what it said on its way down is in `~/.cache/cockpit/hub.log`. Read that first. Then:

```sh
pkill -f '(^|/)cockpit hub$'   # the next app that needs it starts a new one
cockpit hub ping               # exits 0 when one answers
```

## A key does nothing

`prefix+i`, `prefix+u` and the rest jump to a pinned space. A key whose space is not pinned now says so in a
notification. `cockpit setup` pins the spaces for the features you picked; `cockpit doctor` lists the ones missing.

## Notifications arrive twice, or not at all

- Twice: herdr is delivering them as well as cockpit → set `ui.toast.delivery` to `"herdr"` in
  `~/.config/herdr/config.toml`, which doctor also tells you.
- Not at all: on macOS, System Settings → Notifications → Cockpit → Allow. Doctor reports the current state.

## A session behaves like an older cockpit

Each agent session loads the plugin when it starts, so one that was already running keeps the old code after an
update. `/reload-plugins` in that session picks up the new one; doctor lists every session that is behind.

## Still stuck

Open an issue with the output of `cockpit doctor --all`:
<https://github.com/quaresc/cockpit/issues/new/choose>.
