# Cockpit Chat

Read your agents instead of watching their terminals. The window opens on the conversation with the agent in front —
its words as words, everything it did folded into one line you can open — with the question it is waiting on as a card
above the box you answer in. Its real terminal is one key away.

The same cockpit mod and hub as the Cockpit layout, so the inbox, runbooks, meetings and the rest are still a key
away; only the shell around them changes.

## What you see

Left to right, then top to bottom. Colours come from the layout's own theme (`cockpit-chat`, dark, and
`cockpit-chat-light`): near-monochrome greys, colour only for what an agent is doing.

- **Sidebar**: cockpit's own, unchanged — Herdr's brand row with the search field and the panel button that closes
  it, the quota bars, the app strip with its counts, the NEEDS YOU band whose questions are answered in place, the
  agents grouped by whose move it is with their ticket, PR, diff and age, the spaces with their terminals, and the
  Settings footer. ⌘B and the panel button pin it open and let it go; it remembers.
- **Drawers**, one per edge, each standing in for its pinned half while that half is away: a 10px bar of `selected`
  is all that shows, and the pointer slides out the rest — the agents alone on the left (whose move it is, the title,
  the ticket or branch, the wait), the agent's `show` view on the right. Herdr draws one layout and never changes it
  on hover, so the bar is simply the first child on the edge side; nothing of the content peeks through. Both wear the
  same head — what it is, a count, the button that pins it open — and both are only as tall as what they hold, up to
  620px, after which the list scrolls.
- **Top bar**, 56px, draggable, over the window's traffic lights: one rounded capsule holding a pill per tab of the
  space in front, the active one filled, each with its agent's mark (a turning arc working, a check done, an amber dot
  waiting, a ring idle). A tab whose agent waits says so in the blocked colour and carries how long it has waited; a
  second capsule on the right opens Herdr's own new menu (a terminal, an agent, a space, a worktree — ⌘T) and the
  agent's terminal.
- **Chat**, the screen the window opens on, in a 760px reading column:
  - what you said, in a bubble against the right on `raised`, 20px corners;
  - what the agent said, full width, as Markdown; fenced code in a box of its own (12px corners, 1px border,
    JetBrains Mono) because Herdr's Markdown draws it square;
  - everything the agent did for one turn as a single line ("Worked for 2m 14s"), which opens into what it said on
    the way and one line per call: the tool, what it was given, and whether it worked; a rule under it separates the
    work from the answer, and a turn still running counts up to now with its fold open;
  - a button back to the newest message once the wheel has moved (End does the same), and "Show earlier" at the top
    when the session runs past what the hub sent;
  - the prompts waiting their turn, under the conversation, each with a cross that takes it back;
  - the question it waits on as the cockpit question card, wide, right above the box;
  - the box, 22px corners, what you type above and what it is working with below: `+` for a file, what the agent may
    do without asking (amber once it is more than "asks first"), the folder, then the model and a round send button
    that lights up once there is something to send. Where its work lives sits above the box as icons (the views it
    showed beside itself, then its ticket, PR, CI run and the pages it cites — their names repeat, so words would
    read as soup). ⏎ answers a question, sends a prompt to an idle agent, and queues one for an agent still working.
    A file dropped on the box rides with the next prompt as a path, and `+` chooses one or takes the image off the
    clipboard.
- **Needs you**: every waiting agent as a question card, the longest-waiting first, each with a button to read it.
- **Home, Inbox, Runbook, To-do, Meetings, Monitor, Routines, Mail**: the cockpit apps, unchanged.
- **Tab** (⌘⌥T): the space's real panes, for when the terminal is what you want — a `show` pane, a diff, a page in
  terminal-browser.
- **Side panel** (⌘⌥B), 420px on the right of both: the `show` tool's views with their charts, tables and diagrams,
  the review screen, the focused agent's runbook, Herdr's files and changes, and the activity rail that switches
  between them (⌃⌘1–5).

## How it behaves

- The chat follows whoever is in front: focus another agent and its conversation replaces this one.
- ⌘B, or the panel button in the brand row, pins the sidebar open and lets it go; ⌘⌥B does the same for the side
  panel. Whichever is away peeks at its edge and slides out over the conversation on hover.
- ⌃` opens the agent's real terminal as a window over the chat, with its own head bar.
- 1–9 answer the question on the card while the box is empty; ⏎ sends what is in the box.
- ⌘⌥0 Chat, ⌘⌥9 Needs you, ⌘⌥8 Home, ⌘⌥1–7 the apps, ⌘⌥T the panes, ⌘⇧P the palette, ⌘J the next waiting agent.
- ⌘⇧L switches back to the Cockpit layout; everything there is where it was.

## Driven, not only read

A conversation the chat shows comes from one of two places. Any agent Herdr runs is **read**: its transcript gives the
words and the tool calls, its screen gives the question. Claude Code can also be **driven**: cockpit starts it under
the streaming protocol and owns its stdin and stdout, so the same words arrive as events, and with them the things a
reader had to guess — the agent's own slash commands, its rate-limit window, the cost of the turn. ⌘⌥N starts one in
the folder of the space in front; the chat prefers it when there is one.

A driven session gives the box what a reader could never have: `/` completes from the commands the agent itself
publishes and `@` from the folder's files, the permission word and the model are menus that set them on the running
agent, ⇧⇥ cycles what it may do without asking (asks first → edits freely → plan only), ⎋ stops the turn, and a
permission it wants arrives as the same question card the rest of cockpit uses.

## Built from

- `layouts/cockpit-chat.json`: a row of `sidebar` (268px: Herdr's `workspace-chrome` brand row, the cockpit
  `sidebar` view, its footer) and `work`; `work` is `topbar` (56px,
  `chat-top`) over the `screen` stack. Its first child is a row also called `work`, holding the `view` stack
  (`chat` and `tab`, Herdr's `{tab: "active"}` with a pane frame) beside the hidden `panel`; then come `needs`,
  `home` and the seven app blocks.
- `mods/cockpit/`, the one cockpit mod, through `props.app`: `chat`, `chat-top`, `needs`, and every view the Cockpit
  layout already uses, `sidebar` included. `screens.js` says which stack each layout uses and which screen it
  deals with an agent on (`chat` here, the panes in the Cockpit layout), so `show()` and `goToAgent()` work in both.
- `themes/cockpit-chat.json` and `cockpit-chat-light.json`: the twelve panel colours plus the ansi set the design kit
  derives its badges from.
- The hub's `chat` view (`herdr/tui/agents/src/chat.rs`): one agent's transcript read with
  `transcripts::read_detail(.., Detail::Tools)`, its messages and tool calls in order, with the agents view's row and
  question beside them. It answers, snoozes and forks through the `agents` view's actions, and sends a prompt through
  the `queue` view's.
- Herdr: `spaces`, `focused`, `props`, `shared`, `setShared`, `run`, `openOverlay`, `theme`, `window`, `layout`.

## Watch out

- The sidebar column's id must stay `sidebar`: Herdr's own ⌘B is `layout.toggle:sidebar`.
- Its `min` is 220, not 44: the cockpit sidebar folds into an icon rail below 160px, and here the small state is the
  drawer, not a rail.
- A drawer sizes to what it holds, so nothing inside it may be `flex_1` with `min_h(0)` — there is no height to take,
  and the drawer collapses to its head. The body caps the height; the list keeps its own.
- One copy at a time, never two. A drawer and the thing it stands in for both follow the hub, so each drawer draws
  nothing and drops its subscription while its pinned half is open (`DOCKED` in `screens.js`, `PANELED` in
  `status.js`). A second live copy doubles every subscription it holds, and a hub that blinks then has twice as many
  copies racing to start one — which is what exhausts the window's file descriptors.
- The layout file cannot say whether the panel is open: `layout()` is the file, where it is always `hidden`. The
  shared flags are what the drawers read — so every way of closing a panel has to set them, the activity rail's own
  close button included, or its drawer keeps standing down for something that is no longer there.
- `TextView.markdown` takes no styling and draws fences square: the chat splits them out itself.
- The chat's rows are a plain scroll area with a `ScrollHandle` (Herdr 0.1.15), not a `list`: a list cannot be
  scrolled from code, and its renderer refuses handlers, which the tool chips and the markdown links need. A scroll
  request is consumed after the next layout, so the jump to the newest message is asked for again a few frames on.
- This runtime has no `Intl`: write a clock by hand rather than with `toLocaleTimeString`.
- A mod has no file picker, and `read_from_clipboard` gives text alone. The `+` goes through `osascript` — macOS's own
  open panel, and the clipboard written out as a PNG — so it is macOS only; a drop works anywhere.
- The hub's `chat` view sends the latest 400 messages unless `limit` asks for more, and says `more` when it held
  some back.
- The hub reads tool calls only when asked (`Detail::Tools`): the session index and recaps still read words alone, and
  Codex's reader still skips its tool records unless they are wanted.

## Done when

- A real Claude session reads the same as its terminal, a turn's work folded into one line that opens.
- A permission prompt shows as a card under the conversation, and `1` answers it.
- A reply typed in the box reaches the agent; one typed while it works is queued.
- The pointer at the left edge slides the sidebar out, ⌘B docks it, ⌃` opens the terminal.
- Both themes are right at 1280 and at 760.
