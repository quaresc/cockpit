# Screenshots

The SVGs in `docs/` are the real herdr apps rendered from fictional data, plus a composed overview and notifications.

```sh
python3 -m venv .venv && .venv/bin/pip install pyte fonttools
python3 demo_data.py home          # a fake HOME with caches, to-dos and runbooks
.venv/bin/python shoot.py          # inbox, todo, runbook, meetings
.venv/bin/python overall.py        # the overview
.venv/bin/python runbook_anim.py   # a refresh landing in the runbook, animated (run demo_data.py again first)
.venv/bin/python features.py       # the recap popup, and the inbox with its sandbox and stop buttons
python3 notifications.py           # the notifications, animated
```

Needs FiraCode Nerd Font Mono in `~/Library/Fonts`; a subset of it is embedded in each SVG.
