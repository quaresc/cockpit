---
name: side-browser
description: Use the browser pane beside this conversation (terminal-browser in herdr) to check a front change end to end against the local dev server, on mobile, tablet and desktop sizes, reproduce a bug on a page, read the page the person is looking at, or prepare a review on GitHub for the person to submit. Use it whenever a task needs a real browser and this session runs in herdr; it falls back to Claude in Chrome when terminal-browser is missing or the person set front checks to Chrome.
---

The person sees this browser beside the conversation and it keeps their sign-ins, so they watch what you do and the pages
you open are already logged in.

**Which browser?** Run `command -v terminal-browser; echo "$HERDR_PANE_ID"; jq -r '.checksBrowser // "terminal"'
~/.config/cockpit/config.json`. Use Claude in Chrome (the `claude-in-chrome` skill) instead, and skip the rest of this
skill, when terminal-browser is missing, there is no pane id, the setting says `chrome`, or the person asks for Chrome.
The person flips the setting with `/cockpit checks chrome` or `/cockpit checks terminal`.

**Open a page** with `sh ~/.config/cockpit/herdr/browse.sh <url>`, never with `terminal-browser open`. A URL, a
`localhost:<port>` or an absolute path to an HTML file all work. It opens the page as a tab of the browser already beside
you, or brings the tab to the front when that page is already open, or splits a new pane right of yours sized so herdr
draws it fast. The keyboard stays with the conversation. `--size 0.6` asks for a wider pane, `--reload` reloads the page
when it is already open.

**Drive it** with `terminal-browser action -- <command>`, which targets the active tab of the browser beside you (the
`terminal-browser` skill has the full command list): `snapshot -i` lists the elements with refs, then `click @e12`,
`fill @e3 "text"`, `press Enter`, `get text @e5`, `eval "document.title"`, `wait --load networkidle`,
`screenshot <path>` to look at the result. Run `terminal-browser action done` after your last action, which clears the
"agent in control" banner the person sees.

**Checking a front change**: make sure the dev server runs, open its local URL, walk the flow the change touches, take
a screenshot at each state that matters and compare it with the Figma frame when the ticket has one. Say what you checked
and what you saw, with the screenshot paths.

**Mobile, tablet and desktop**: emulation sticks to the tab it is set on, so give it a tab of its own:
`terminal-browser new-tab <url>`, then `terminal-browser action -- set device "iPhone 16"` (also `iPad`, `iPad Pro`,
`Pixel 9`, `Galaxy S25`) or `set viewport 1440 900` for a desktop width, `reload`, and `screenshot` at each size. Close
that tab with `terminal-browser action -- tab close` when done, so the
person's tabs keep their own size.

**Before and after, for a PR description**: `sh ~/.config/cockpit/herdr/shots.sh <url> <dir>` screenshots a page at
phone, tablet and desktop sizes in throwaway tabs (`--sizes mobile,desktop`, `--full` for the whole page) and prints the
paths. Take it once for before, into `~/.cache/cockpit/shots/<repo>-<branch>/before`: on `main` with the dev server
still running, or on the production page when the branch must stay checked out. Then once for after, on the branch,
into `…/after`. Dismiss cookie banners first so both sides match. Show them to the person with the show tool,
`placement: "browser"`, one `{type: "compare", title: "Mobile", before, after}` block per size. In the PR description,
GitHub only takes pictures uploaded through its own editor: open the PR's edit form (or the new PR page) beside you,
`upload` the screenshots into the description field, check their links appeared, arrange them in a small Before | After
table, and leave Save or Create to the person.

**Anything that publishes in the person's name stays theirs**: a GitHub review, a comment, a message, a form that
sends. Fill it in (the review's inline comments, the summary), leave the page on the Submit button and tell the person
it is ready for them to read and submit. Never press Submit, Send, Approve, Merge or Delete yourself.
