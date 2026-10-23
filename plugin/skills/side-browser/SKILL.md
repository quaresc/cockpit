---
name: side-browser
description: Use the browser pane beside this conversation (terminal-browser in herdr) to check a front change end to end against the local dev server, reproduce a bug on a page, read the page the person is looking at, or prepare a review on GitHub for the person to submit. Use it whenever a task needs a real browser and this session runs in herdr; it falls back to Claude in Chrome when terminal-browser is missing.
---

The person sees this browser beside the conversation and it keeps their sign-ins, so they watch what you do and the pages
you open are already logged in.

**Is it there?** Run `command -v terminal-browser && echo "$HERDR_PANE_ID"`. With no path or no pane id, use Claude in
Chrome (the `claude-in-chrome` skill) for the same work instead, and skip the rest of this skill.

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

**Anything that publishes in the person's name stays theirs**: a GitHub review, a comment, a message, a form that
sends. Fill it in (the review's inline comments, the summary), leave the page on the Submit button and tell the person
it is ready for them to read and submit. Never press Submit, Send, Approve, Merge or Delete yourself.
