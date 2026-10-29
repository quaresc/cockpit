# Screenshots

The SVGs in `docs/` are the Rust apps and composed screens on fictional data; the E2E ones show real screenshots
of the fake app in `webapp/`.

```sh
python3 -m venv .venv && .venv/bin/pip install pyte fonttools pillow
(cd ../../herdr/tui && cargo build --release)  # the cockpit binary whose apps the screenshots run
python3 demo_data.py home          # a fake HOME with caches, to-dos, runbooks, releases and past sessions; run it again before each script
.venv/bin/python history.py        # the hub's history from an earlier inbox and monitor, and "last looked" before it
.venv/bin/python apps_anim.py      # inbox, to-do, meetings, monitor and routines: the cursor stepping, the rows marked as changed
.venv/bin/python palette_anim.py   # the palette popup over a Claude session: a ticket and its history, a past session
.venv/bin/python runbook_anim.py   # a refresh landing in the runbook
.venv/bin/python show_anim.py      # show: charts beside the conversation, laid over as the images it saves
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
