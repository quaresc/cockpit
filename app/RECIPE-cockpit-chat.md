# Cockpit Chat

Read your agents instead of watching their terminals. The window opens on the conversation with the agent in front —
its words as words, everything it did folded into one line you can open — with the question it is waiting on as a card
above the box you answer in. Its real terminal is one key away.

The same cockpit mod and hub as the Cockpit layout, so the inbox, runbooks, meetings and the rest are still a key
away; only the shell around them changes.

## What you see

Left to right, then top to bottom. Colours come from the layout's own theme (`cockpit-chat`, dark, and
`cockpit-chat-light`): near-monochrome greys, colour only for what an agent is doing and the providers' own logos.

- **Sidebar**: cockpit's own — Herdr's brand row with the search field and the panel button that closes
  it, the quota bars, the app strip with its counts (Needs you second, with how many wait; keys in the tooltips only), the NEEDS YOU band whose questions are answered in place, the
  agents grouped by whose move it is with their ticket, PR, diff and age, the spaces with their terminals, a quiet
  Cockpit settings row (every option, the health checks, the versions) and the Settings footer. ⌘B and the panel button pin it open and let it go; it remembers.
- **Drawers**, one per edge, each standing in for its pinned half while that half is away: a 10px bar of `selected`
  is all that shows, and the pointer slides out the rest — the agents alone on the left (whose move it is, the title,
  the ticket or branch, the wait), the agent's `show` view on the right. Herdr draws one layout and never changes it
  on hover, so the bar is simply the first child on the edge side; nothing of the content peeks through. Both wear the
  same head — what it is, a count, the button that pins it open; the agents' head also leads to Needs you, as does its
  Needs you heading — and both are only as tall as what they hold, up to 620px, after which the list scrolls.
- **Top bar**, 56px, draggable, over the window's traffic lights: one rounded capsule holding a pill per tab of the
  space in front, the active one filled, each with its agent's mark (a turning arc working, a check done, an amber dot
  waiting, a ring idle). A tab whose agent waits says so in the blocked colour and carries how long it has waited.
  A pill reads the title of its agent's work while the tab's label is Herdr's number or the agent's own name. What
  does not fit waits behind a "+N" pill (⌘⇧A opens its menu). The active pill's × closes the tab, a second click
  confirming it while its agent works or waits. A second capsule on the right holds the hub's dot (filled
  while it runs, a ring with "Hub stopped" or "Hub not running" when it is down, a click starts it), Herdr's own new menu (a terminal, an
  agent, a space, a worktree — ⌘T) and the agent's terminal.
- **Chat**, the screen the window opens on, in a 760px reading column:
  - what you said, in a bubble against the right on `raised`, 20px corners;
  - what the agent said, full width, as Markdown; fenced code in a box of its own (its language and a copy button
    over the lines, 1px border, JetBrains Mono, scrolled sideways rather than wrapped) because Herdr's Markdown draws
    it square, and a fence still streaming is a box already;
  - everything the agent did for one turn as a single line ("Worked for 2m 14s"), which opens into what it said on
    the way and one line per call: the tool, what it was given, and whether it worked; a rule under it separates the
    work from the answer, and a turn still running counts up to now and shows only the step it is on, its terminal's
    words for it, with "N more ⌄" beside it opening the rest;
  - the newest message stays in view only while you are there: reading back is never pulled down, and a "N new ↓"
    pill counts what came since; reaching the top, or "Earlier messages", reads further back with the first message
    kept where it was;
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
  the review screen, the focused agent's runbook, files and changes, the Chromium side browser when it is
  turned on, and the activity rail that switches between them (⌃⌘1–5). Files and Changes are cockpit's own
  (`project-files`, `project-changes`): they follow the repositories the chat's agent works in, even from a folder of
  several, the one it changed last first; `chat.projectPanels` off gives Herdr's back.

## How it behaves

- The chat follows one named agent: the one in front, or the one opened from the palette, the drawer, Needs you or a
  session pill in the top bar, until the focus moves. Its header names it (status, title, place, branch, PR), each
  agent keeps its own draft, and with nobody to follow it says so rather than showing someone else.
- What you send leaves the box once the hub took it ("Sending…" until then); refused, it stays with "Not sent · Retry".
  The queued prompts sit above the box, a paused queue says why with Resume, and Stop ends a turn. With the hub down
  it says so with Start it, and comes back by itself.
- ⌘B, or the panel button in the brand row, pins the sidebar open and lets it go; ⌘⌥B does the same for the side
  panel. Whichever is away peeks at its edge and slides out over the conversation on hover.
- ⌃` opens the agent's real terminal as a window over the chat, with its own head bar. A session in chat is handed
  over instead, once you say so and only at rest, with nothing queued for it: its terminal opens, it stops here, and
  it goes on there, so one process ever writes it.
- 1–9 answer a question on the card while the box is empty; ⏎ sends what is in the box. A permission is answered
  on purpose: Tab puts the keyboard on its card, then 1–9 answer; a digit typed into the empty box allows nothing, and
  no chord answers one (⌥ types a character, ⌘1–9 stay Herdr's tabs). The card always shows what the permission
  would allow, its first lines even folded, or says it could not read it.
- ⌘⌥0 Chat, ⌘⌥9 Needs you, ⌘⌥8 Home, ⌘⌥1–7 the apps, ⌘⌥T the panes, ⌘⇧P the palette, ⌘J the next waiting agent
  (⌘⇧J the next blocked, ⌘⇧U the next done), ⌘R the recap. Key hints live in tooltips; ⌘/ lists every key of the layout,
  grouped and searchable, the one place they are all shown.
- The palette lists this layout's screens and commands (Chat, Needs you, Tab…), nothing that does nothing here, and with
  nothing typed the agents by stage, those waiting on you first.
- ⌘⇧L switches back to the Cockpit layout; everything there is where it was.

## In chat, not only read

A conversation the chat shows comes from one of two places. Any agent Herdr runs is **read**: its transcript gives the
words and the tool calls, its screen gives the question. Claude Code can also run **in chat**: cockpit starts it under
the streaming protocol and owns its stdin and stdout, so the same words arrive as events, and with them the things a
reader had to guess — the agent's own slash commands, its rate-limit window, the cost of the turn. ⌘⌥N starts one in
the folder of the space in front and opens it in the chat; each session in chat has a pill in the top bar.

A session in chat gives the box what a reader could never have: `/` completes from the commands the agent itself
publishes and `@` from the folder's files, the permission word and the model are menus that set them on the running
agent, ⇧⇥ cycles what it may do without asking (asks first → edits freely → plan only) and ⎋ stops the turn, both
only while the keyboard is in the chat (a menu or the palette keeps its own), and a permission it wants arrives as the
same question card the rest of cockpit uses.
Full access and auto, which run tools without asking, are set only after a confirm that names the session; Cancel
holds the keyboard. Edits freely is set at once from the mode menu, whose items carry a key made when it opened, or by ⇧⇥
pressed in the chat, and through the same confirm any other way (the palette's entry, a window command).

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
- The hub's `chat` view (`herdr/tui/agents/src/chat.rs`): one agent's transcript followed with
  `transcripts::Follow` (only what was written since the last build is read), its messages and tool calls in order,
  with the agents view's row and question beside them. It answers, snoozes and forks through the `agents` view's actions, and sends a prompt through
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
  The handle cannot say where the view is: the chat works it out from the wheel and each row's `on_size_change`, and
  a revealed row lands at the edge it came from, which is why keeping a row at the top goes to the end first.
- Text views are retained by id: keyed by the message, every message ever drawn would keep one. Their ids come from a
  pool as big as the window, kept by row, so a row keeps its views while drawn; folds and the reading position are
  kept by message id, which survives a row growing.
- This runtime has no `Intl`: write a clock by hand rather than with `toLocaleTimeString`.
- A mod has no file picker, and `read_from_clipboard` gives text alone. The `+` goes through `osascript` — macOS's own
  open panel, and the clipboard written out as a PNG — so it is macOS only; a drop works anywhere.
- The hub's `chat` view sends the latest 400 messages unless `limit` asks for more, and says `more` when it held
  some back.
- The hub reads tool calls only when asked (`Detail::Tools`): the session index and recaps still read words alone, and
  Codex's reader still skips its tool records unless they are wanted.

## Done when

- A real Claude session reads the same as its terminal, a turn's work folded into one line that opens.
- A permission prompt shows as a card under the conversation with what it would allow, and Tab then `1` answers it.
- A reply typed in the box reaches the agent; one typed while it works is queued.
- The pointer at the left edge slides the sidebar out, ⌘B docks it, ⌃` opens the terminal.
- Both themes are right at 1280 and at 760.
