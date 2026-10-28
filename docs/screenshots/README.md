# Screenshots

The SVGs in `docs/` are the real herdr apps and composed screens on fictional data; the E2E ones show real screenshots
of the fake app in `webapp/`.

```sh
python3 -m venv .venv && .venv/bin/pip install pyte fonttools pillow "textual>=8,<9" "textual-image>=0.14,<0.15" "resvg-py>=0.5,<0.6"
python3 demo_data.py home          # a fake HOME with caches, to-dos and runbooks; run it again before each script
.venv/bin/python apps_anim.py      # inbox, to-do and meetings, the cursor stepping through the rows
.venv/bin/python runbook_anim.py   # a refresh landing in the runbook
.venv/bin/python show_anim.py      # the show tool: a chart hiding to its chip and coming back
(cd ../../herdr/tui && cargo build --release)  # the Rust apps, then the same drawn by them:
.venv/bin/python apps_anim.py --rust         # inbox-rust.svg, todo-rust.svg, meetings-rust.svg
.venv/bin/python runbook_anim.py --rust      # runbook-done-rust.svg
.venv/bin/python show_anim.py --rust         # show-rust.svg, its charts laid over as images (COCKPIT_SHOT_DIR)
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
