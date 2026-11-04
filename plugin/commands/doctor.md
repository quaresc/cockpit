---
description: Check every part of cockpit and fix what is broken
---

Run `COCKPIT_APP_WAIT=0 sh ~/.config/cockpit/herdr/app.sh doctor` (when that link is missing, neither the herdr plugin nor the Herdr app half is set up: say so and suggest `/cockpit:setup`). When it prints `no cockpit binary for this machine yet` instead of checks, the `cockpit` binary is missing and could not be fetched: say so, and suggest `sh ~/.config/cockpit/herdr/fetch-apps.sh` (its error says why) or `cargo build --release` in `~/.config/cockpit/herdr/tui`.

Reply with the `fail` and `warn` lines only, each as one short line: what is wrong and the fix the line names. When everything is `ok`, say so in one line.

Then offer to apply the fixes that only touch cockpit's own pieces: running `ensure-spaces.sh`, `fetch-apps.sh`, `prune-browsers.sh`, `update.sh`, `notifier/build.sh`, `cockpit.sh app install` (the Herdr app's layout and mod), `cockpit.sh agents connect <agent>` (cockpit's tools in an agent's MCP config, backed up first), `pkill -f '(^|/)cockpit hub$'`, or sending `/reload-plugins` to an idle Claude session: `herdr agent prompt <pane> /reload-plugins` in herdr, `hc send <terminal> /reload-plugins` in the Herdr app. Apply them only once I agree. Never edit `~/.claude/settings.json`, the herdr config or the Herdr app's own files (`keys.json`, `settings.json`) yourself: print the change for me to make.
