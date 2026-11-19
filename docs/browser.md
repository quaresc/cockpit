# The Chromium side browser

cockpit's side browser is terminal-browser by default: a browser drawn inside a terminal pane. In the Herdr app on
macOS you can have a real Chromium window instead, docked in the side panel. It has an address bar, tabs, history and
DevTools, each agent session has its own browser, and you can point at elements of a page and hand them to that agent.

herdr's own panes and Linux keep terminal-browser: turning the option on there changes nothing, and `cockpit browse`
says so.

## Turning it on

```sh
cockpit options set sideBrowser chromium
cockpit browser install          # Electron and agent-browser, pinned and checked, about 130 MB to download
```

`cockpit setup` asks the same two things. In Cockpit settings the option is **Side browser** under Apps, and
`/cockpit browser chromium` sets it from Claude Code. `cockpit browser status` says what is installed and what runs.

Nothing else changes for an agent: `sh ~/.config/cockpit/herdr/browse.sh <url>` opens a page, and
`browse.sh action -- snapshot -i`, `click @e3`, `fill`, `screenshot` and the rest drive it, as with terminal-browser.

## What you see

The browser shows in the side panel (⌘⌥B), under the globe on the activity bar, in both layouts. It opens by itself
when the agent in front opens a page. It is a separate window that cockpit lays exactly over the panel and keeps there:
it follows the Herdr window when it moves or changes size, and steps aside when the palette or a dialog of cockpit's
opens over it, leaving a picture of its last frame in its place.

- **One browser per session.** A browser belongs to the agent session it was opened for, and shows with that session
  only. Put another session in front and you see its own tabs, where you left them (the active tab, each page's scroll
  and its size); one that never opened a page has no browser, and the panel closes until you come back. A page an agent
  opens while you are elsewhere waits for you: the panel opens on it when its session comes in front. When a session's
  terminal closes, its browser goes with it.
- **The address bar** takes an address, a `localhost:3000`, a file's path, or words to search for. It suggests pages
  you visited. `⌘L` goes to it.
- **A new tab** (`⌘T`) lists the servers listening in the session's worktree, so the dev server is one click away.
- **Page size** (the button with two screens) shows the page as a phone, a tablet or a laptop, scaled to fit the panel.
- **DevTools** (`⌥⌘I`) opens under the page, or at its right from the ⋯ menu. An agent can keep driving the page while
  it is open.
- **Keys**: `⌘R` reload, `⇧⌘R` hard reload, `⌘W` close the tab, `⇧⌘T` reopen it, `⌘[` and `⌘]` back and forward,
  `⌘F` find, `⌘1`…`⌘9` a tab, `⌘+` `⌘-` `⌘0` zoom.
- **Pop-ups** open as tabs of the same browser and keep their link to the page that opened them, so a sign-in window
  works. `browser.popups: block` refuses them.
- **Downloads** go to `browser.downloads` (your Downloads folder by default); what an agent downloads goes to
  `~/.cache/cockpit/browser/downloads/` instead. The ⋯ menu lists them.
- **Permissions.** A page gets full screen and the clipboard's write side without asking. Location, notifications,
  camera, microphone, the clipboard's read side and devices on your network are asked once per site and profile, in a
  system dialog. Screen sharing is never offered.

With `browser.dock` off, the same browser is an ordinary window of its own.

## Pointing at the page for the agent

What you point at goes to the session the browser belongs to. There is nobody to choose: it is that agent's page.

- **Pick** (the arrow button, `⇧⌘C`): hover an element, click it, write one line about it (a question, or what should
  change) and **Send**. While picking, `C` copies the element under the pointer and `S` its picture; `esc` stops.
- **Notes** (the speech bubble, `⇧⌘A`): click an element, write what should change, choose an intent (fix, change,
  question, approve) and a priority, and go on to the next one, up to 20 a page. Each element keeps a numbered badge;
  rest the pointer on a badge to read its note. Once there are notes, the bubble shows their count and opens their
  list: remove one, add another, copy them as Markdown, clear them, or **Send** them all.

In both, `enter` saves what you wrote and `shift+enter` starts a new line.

The agent gets one prompt headed `## Design Feedback`: for each element its selector, its source file and line when
the page is a React development build, its text, your note and the path of a cropped picture. With **Include page
details** it also gets the element's box, classes, computed styles and HTML. Values that look like secrets are left out.

## Profiles

A profile is a separate store of cookies, local storage and cache. The chip left of the address names the one in use
and opens its menu.

- `browser.profileScope` says which profile a session's browser starts with, from the worktree it works in: `shared`
  (one for all, the default), `repo` (one per repo, made by cockpit) or `worktree` (one per worktree, deleted with
  it). Picking another from the menu sticks for that browser.
- **Import cookies** copies them from Chrome, Brave, Arc, Edge, Comet, Helium, Vivaldi, Firefox or Safari, or from a
  JSON cookie file. The cookies the profile already had for the same sites are replaced. Google's sign-in cookies are
  left out, since Google ties them to the browser that made them: sign in to Google in the profile itself. For a
  Chromium-family browser, macOS asks for your password so that `security` can read that browser's "Safe Storage" key;
  Safari needs Full Disk Access.
- From a terminal: `cockpit browser profiles`, and
  `cockpit browser import-cookies <profile> --from chrome[:Default]` or `--file cookies.json`.

Creating, deleting, clearing and importing happen in system menus and dialogs, which only you can answer.

## What it runs, and what can reach it

The hub starts one Electron process for every session and profile, from `~/.cache/cockpit/browser/electron/`. Tabs you
are not looking at are paused, and one out of view for `browserIdleMinutes` is put to sleep: its page is closed, and it
loads again where it was when you come back. A tab that plays sound, has DevTools open or holds text you typed stays
awake. A page covered by another app's window keeps drawing, so an agent's screenshot never waits for you to uncover it. `cockpit browser quit` ends the process; the next page opened starts it again.

Agents drive pages through Chromium's debugging port, bound to this machine only, as terminal-browser's is. Any
program running as you can connect to that port and read or drive any tab of any profile. Do not import cookies you
would not trust every program on your account with.

Sign-ins, tabs and history are under `~/.config/cockpit/browser/`; picked elements' pictures and the log under
`~/.cache/cockpit/browser/`.
