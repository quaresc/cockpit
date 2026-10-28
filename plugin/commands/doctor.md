---
description: Check every part of cockpit and fix what is broken
---

Run `COCKPIT_APP_WAIT=0 sh ~/.config/cockpit/herdr/app.sh doctor` (when that link is missing, the herdr plugin is not set up: say so and suggest `/cockpit:setup`). When it prints `no cockpit binary for this machine yet` instead of checks, the `cockpit` binary is missing and could not be fetched: say so, and suggest `sh ~/.config/cockpit/herdr/fetch-apps.sh` (its error says why) or `cargo build --release` in `~/.config/cockpit/herdr/tui`.

Reply with the `fail` and `warn` lines only, each as one short line: what is wrong and the fix the line names. When everything is `ok`, say so in one line.

Then offer to apply the fixes that only touch cockpit's own pieces: running `ensure-spaces.sh`, `fetch-apps.sh`, `prune-browsers.sh`, `update.sh`, `notifier/build.sh`, `pkill -f '(^|/)cockpit hub$'`, or sending `/reload-plugins` to an idle Claude pane with `herdr agent prompt <pane> /reload-plugins`. Apply them only once I agree. Never edit `~/.claude/settings.json` or the herdr config yourself: print the change for me to make.
