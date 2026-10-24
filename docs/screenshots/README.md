# Screenshots

The SVGs in `docs/` are the real herdr apps rendered from fictional data, plus a composed overview and notifications.

```sh
python3 -m venv .venv && .venv/bin/pip install pyte fonttools
python3 demo_data.py home          # a fake HOME with caches, to-dos and runbooks
.venv/bin/python shoot.py          # inbox, todo, runbook, meetings
.venv/bin/python overall.py        # the overview
.venv/bin/python runbook_anim.py   # a refresh landing in the runbook, animated (run demo_data.py again first)
.venv/bin/python features.py       # the recap popup, and the inbox with its preview and stop buttons
".venv/bin/python show_anim.py      # the show tool: a chart hiding to its chip, flow, sequence, live blocks, a diff
.venv/bin/python settings.py       # the /cockpit settings pane
python3 notifications.py           # the notifications and their buttons, animated
```

Needs FiraCode Nerd Font Mono in `~/Library/Fonts`; a subset of it is embedded in each SVG.

`compare.png` is the real E2E flow on the fake app in `webapp/`, run from a Claude session in herdr:

```sh
python3 -m http.server 5173 --directory webapp &
sh ~/.config/cockpit/herdr/shots.sh http://localhost:5173/main/ /tmp/shots/before --sizes mobile
sh ~/.config/cockpit/herdr/shots.sh http://localhost:5173/branch/ /tmp/shots/after --sizes mobile
```

Then ask the session to `show` the two as a `compare` block in the browser, and screenshot that page.
