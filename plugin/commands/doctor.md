---
description: Check every part of cockpit and fix what is broken
---

Run `sh ~/.config/cockpit/herdr/doctor.sh` (when that link is missing, the herdr plugin is not set up: say so and suggest `/cockpit:setup`).

Reply with the `fail` and `warn` lines only, each as one short line: what is wrong and the fix the script names. When everything is `ok`, say so in one line.

Then offer to apply the fixes that only touch cockpit's own pieces: running `ensure-spaces.sh`, `prune-browsers.sh`, `update.sh`, `notifier/build.sh`, or sending `/reload-plugins` to an idle Claude pane with `herdr agent prompt <pane> /reload-plugins`. Apply them only once I agree. Never edit `~/.claude/settings.json` or the herdr config yourself: print the change for me to make.
