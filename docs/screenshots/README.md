# Screenshots

The SVGs in `docs/` are the real herdr apps and composed screens on fictional data; the E2E ones show real screenshots
of the fake app in `webapp/`.

```sh
python3 -m venv .venv && .venv/bin/pip install pyte fonttools pillow
python3 demo_data.py home          # a fake HOME with caches, to-dos and runbooks; run it again before each script
.venv/bin/python apps_anim.py      # inbox, to-do and meetings, the cursor stepping through the rows
.venv/bin/python runbook_anim.py   # a refresh landing in the runbook
.venv/bin/python show_anim.py      # the show tool: a chart hiding to its chip and coming back
.venv/bin/python features.py       # the recap popup
.venv/bin/python settings.py       # the /cockpit settings pane
python3 notifications.py           # the notifications and their buttons
.venv/bin/python compose_anim.py   # the sidebar, the footer, and an E2E test in terminal-browser
.venv/bin/python overall.py        # the whole cockpit at once, animated
```

`compose_anim.py` and `overall.py` read the fake app's screenshots, which a Claude session in herdr takes:

```sh
python3 -m http.server 5173 --directory webapp &
sh ~/.config/cockpit/herdr/shots.sh http://localhost:5173/main/ ~/.cache/cockpit/shots/acme-web-quota/before --sizes mobile,desktop
sh ~/.config/cockpit/herdr/shots.sh http://localhost:5173/branch/ ~/.cache/cockpit/shots/acme-web-quota/after --sizes mobile,desktop
```

`docs/compare.png` is the session's `show` compare page of the mobile pair, screenshotted.

Needs FiraCode Nerd Font Mono in `~/Library/Fonts`; a subset of it is embedded in each SVG.
