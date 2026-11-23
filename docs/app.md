# cockpit in the Herdr app

The Herdr desktop app half of cockpit: two layouts (`app/layouts/`) and one mod (`app/mods/cockpit/`), drawn
natively by Herdr from JavaScript, on the same hub as the herdr TUI half. `cockpit app install` copies them into the
Herdr home (`HERDR_APP_HOME`, else `~/.config/herdr-app`) with the themes in `app/themes/` (a theme already there is
left alone), writes `cockpit.json` there (where the binary and cache are), merges `keys.json` (the person's own keys stay;
the exact bindings cockpit replaced, `RETIRED` in `cli/src/gui.rs`, are taken out), and switches the window
to the cockpit layout the first time.

`cockpit` is the default: the agents' terminals, with Home, the spaces and the side panel around them.
`cockpit-chat` reads them instead — see [The reading layout](#the-reading-layout-cockpit-chat). ⌘⇧L switches.

## How it fits

```
Herdr window ── mods/cockpit (JS) ── TCP 127.0.0.1 (hub.json: port, token) ──┐
herdr TUI ───── cockpit apps (Rust) ── unix socket ────────────────────────────┤── cockpit hub ── store, views, actions
Claude Code ─── plugin hooks ─────────── cockpit CLI / socket ─────────────────┘
```

- **One mod, many views.** Herdr refuses imports outside a mod's folder, so every place uses the `cockpit` mod with
  `props.app`: `sidebar`, `todo`, `inbox`, ..., `palette`, `menu`, `kit`. `main.js` maps names to view classes.
- **Rust keeps the logic, JS draws.** A view's model is built by its app crate and served by the hub (`view`, and
  pushed on `subscribe`); edits are hub actions (`act`). TUI and GUI never drift.
- **Frontend.** `cockpit_core::frontend` picks the herdr TUI (`herdr` CLI) or the Herdr app (`herdr-app do`, `hc`)
  per request: GUI requests carry `"frontend": "app"`, sessions in an app terminal have `HC_TERMINAL_ID`. Handles of
  app terminals are `app:t3`; use `frontend::{actions_tab, new_space, run_in, focus, alive, tab_of, show_app}`, never
  the `herdr` CLI directly, in anything both halves reach.

## Adding an app's view

1. **Rust**: in the app crate, a `gui.rs` with `view(args, doc) -> Value`, `act(action, args) ->
   Option<Result<note, error>>` (None for actions the hub's `act` already has) and `dirs() -> Vec<PathBuf>`. Register
   it in `herdr/tui/cli/src/gui.rs` `register()`:
   ```rust
   views::register(Def { name: "inbox", docs: &["inbox", "ci"], dirs: cockpit_inbox::gui::dirs, build: cockpit_inbox::gui::view, act: cockpit_inbox::gui::act });
   ```
   The hub rebuilds the view when one of `docs` changes, a file in `dirs` changes, or after its own action, and pushes
   it only when it differs. `cockpit_todo::gui` is the reference, with its test.
2. **JS**: `app/mods/cockpit/<app>.js`, a class with `init(props, cx)`, `render(cx)`, optional `key(e, cx)` (returns true
   when it used the key), `command(c, cx)` and `close()`; add it to `VIEWS` in `main.js`. The layout already has a
   block `{ "mod": "cockpit", "props": { "app": "<app>" } }` in the `center` stack. `todo.js` is the reference.
3. Every TUI key and action exists, as a visible button with its key as a `Kbd` hint, plus the pointer patterns below.

## The hub from JS (`hub.js`)

```js
import * as hub from "./hub.js";
this.sub = hub.subscribe(cx, { views: ["todo"], docs: ["inbox"], focus: "todo" }, (m, cx) => { /* {view,args,value} | {doc,value} | {focus,target} */ }, (state, cx) => {});
await hub.act("todo", "add", ["text"]);   // {note, pending, refreshing, error}
await hub.view("todo");  await hub.search("quota");  await hub.go(target);  await hub.fetchDetails("ci_log", args);
await hub.events("inbox", sinceMs);  await hub.history("ci_duration", key);
await hub.cockpit(["focus", "space:inbox"]);  // the cockpit binary, on the install's HOME, driving this window
await hub.openUrl(url);  // open (macOS), xdg-open (Linux), start (Windows): never exec("open") directly
hub.isNotInstalled(state);  // a subscription's "error: …" when the Herdr home has no cockpit.json
```

**One subscription per view and window** (`feed.js`): the agents, Home and queue views are followed through
`sharing(hub.subscribe)`, a drop-in for `hub.subscribe`. Each of those views, with its arguments, is one feed of the
window on `globalThis`, the only thing the mod's copies share: the first copy that asks opens it on its own async
context, the others join and get each new value on theirs, and the last one to leave closes it. A copy that leaves
while holding a feed hands it to the copy that beat last, within a beat, and the old subscription serves until the new
one sends its first value, so nobody sees "connecting" in between. Each member beats on its own 3 s timer, which stops
with its copy (a layout switch, a reload): a holder silent for two beats loses its feed within 6–9 s, a member silent
for three drops out until its next beat brings it back (and the feed with it, if it closed meanwhile), and only members
that beat lately take a feed over or are handed values. A beat that comes late itself (the machine slept, the window
was throttled) judges nobody. A subscription the window's watchdog stopped (`sub.stopped`) is opened again on the next
beat, and so is one that said nothing for 12 s short of live (`STALL_MS`: a task Herdr spawned in the middle of a
layout switch can wait for good, while a retry says "connecting" again every few seconds). A hub that does not answer
moves nothing else, and a callback that throws is skipped. `null` and `{}` follow every
frontend's agents and `{frontend: "app"}` the app's only, so a window has two agents feeds and the hub builds each once
per change. The other views of the same call, and any documents, notifications or focus, stay one subscription of the
copy. AgentsModel (sidebar, chat, needs, recap) follows it this way, and so do Home, the status bar, the bell, the agent
heads, the queue, the side peek, the review panel, the inbox and the runbook panel.

**One clock per copy** (`main.js`): the 1 s timer each copy beats on also calls its view's `tick(cx, now)`, where Home,
the bell, the agent heads, the review panel, `show` (and the drawer holding one), the inbox, mail, meetings and the
monitor redraw their ages and poll, instead of a timer each.

`subscribe` reconnects by itself: a connection reads hub.json and pings the hub it names, and when there is no
hub.json or the ping fails it asks for a hub (`startHub`: one start at a time for every view, a failed one tried again
after 30 s); stop it in `close()`. A hub stopped with `cockpit hub stop` (it writes `hub.stopped`, which only `cockpit
hub start` takes away) and a remote runner's hub are started by no window on its own, only by Start it
(`startHub({explicit: true})`, in the chat, Needs, Home, the inbox, Usage, the Start work forms, the sidebar and the
status bar, each saying "The cockpit hub was stopped" or "The cockpit hub isn't running" through `hub.downWords`, ui.js's
`HubDown` card where content goes; a remote runner's offers no Start it). The window keeps why it last failed to reach
the hub, in the error's own words (`hub.lastFailure()`: `{message, at}`, set when a connection and the start after it
both fail, or a followed link breaks; cleared once the window reaches the hub, and said for ten minutes at most): the status bar's item and Cockpit Chat's hub dot say it in their tooltip
(`hub.downTip(state)`: "The cockpit hub isn't running: <the error>"), and Cockpit settings says it under Health. The CLI, MCP and notifications,
which go through `core::hub::request`, say "the cockpit hub was stopped: run cockpit hub start" instead of starting
one, and the doctor warns so, `cockpit hub start` its fix. `cockpit hub start` never stops a hub whose socket takes a connection: it waits for its answer and hub.json.
Every step of a connection is bounded (`within(cx, promise, ms, what)`): a task Herdr spawns while layouts switch can
die with its read pending. A connection waiting on the window's shared `cockpit hub start` reads hub.json again every
half second and takes a hub started another way at once (Start it, the CLI, another window), so a slow start (Herdr
stops a command after 30 s) never keeps a view down once a hub answers. Each copy beats once a second for all its subscriptions, and a task following the hub beats
on its own timer: one that went quiet (its context died) or took longer than every bounded step and the slowest hub
start (`STUCK_MS`, 40 s) is replaced, and so is one the hub never answered (live or an error) whose step outlasts 12 s
(`FIRST_MS`): a first task made mid-switch can hang on a read that has no bound while its timer still beats. One watchdog per window (`globalThis.cockpitHubSubscriptions`, every 12 s while
all is live, 3 s otherwise) pings the hub, closes the sockets of subscriptions on another hub's port it knows, parks
those whose copy stopped beating (their own beat follows again; none is parked just after the machine slept, when
every copy looks behind), and asks for a hub itself only after two of its pings failed in a row. A one-off request is
bounded on its caller's `cx` when it gives one, closes a connection that comes after it gave up, and one sent but
never answered throws with `isUnconfirmed(e)`: it may have been done. Every socket belongs to the evaluation of hub.js
that opened it (`globalThis.cockpitHubOwned`). Herdr 0.1.12 evaluates the mod once per copy, and Herdr may evaluate a
build it never runs, so a newer load takes nothing from an older one: a load is gone only once none of its copies beats
or asks the hub for 10 s (`liveLoad`), and the watchdog's `sweep` then closes its sockets past every bound of a request
(90 s), a save or an update having left its tasks waiting on their reads; its subscriptions are parked as any silent
copy's are.
`{focus, target}` arrives when the palette or a notification asks the view to bring `target.key` into view
(`cockpit focus`, `hub.go`). Without `cockpit.json` Home draws the install panel and `AgentsModel` (agentsui.js) falls
back to Herdr's `agents()`. Only the app's main view sets `focus`; count
subscriptions leave it out, so they cannot consume a pending remote-desk navigation.

## A remote desk (preview)

The Herdr app can read a Linux or macOS runner's hub, including from Windows. Both ends need this version of the
cockpit mod/hub. Connect the runner through Herdr's machine settings first, then put the cockpit layout and mod in
the desktop's Herdr home. The desktop needs no cockpit binary. The runner needs no window of its own: with neither the
Herdr app nor the herdr TUI running there, its hub lists the agents from herdr-core, in the spaces and tabs the
desktop placed them in.

On the runner, start the hub with `COCKPIT_HUB_LISTEN` set to its Tailscale IP, for example
`COCKPIT_HUB_LISTEN=100.64.0.10 cockpit hub start`. If a hub already runs, restart it deliberately with that variable;
`hub start` reuses an existing process and cannot change its interface. The default is loopback. Wildcard and
multicast addresses are refused, as is an address no interface holds: the hub says why on stderr and leaves the
running one alone.

In the desktop's Herdr home, write `cockpit.json` using the runner's paths and its name in Herdr:

```json
{
  "machine": "runner",
  "host": "100.64.0.10",
  "bin": "/home/sam/.local/bin/cockpit",
  "home": "/home/sam",
  "cache": "/home/sam/.cache/cockpit"
}
```

The mod reads the port and token through Herdr's remote file API, authenticates before requesting data, and follows
updates over TCP. Subscriptions reconnect when the hub restarts or sends nothing for 65 seconds; the hub sends an empty
line every 10 s, and a read Herdr gives up on with nothing to read ("Resource temporarily unavailable") reads again on
the same connection. Each subscription answers the hub at least every 30 seconds: the hub resets one that falls quiet
for 90 seconds or stops reading, such as the sockets a reloaded copy of the mod leaves open in Herdr. Links and item
navigation open on the desktop. Notifications appear as in-app toasts. Go, Peek and Open beside reach a runner agent
through its machine-scoped address (`runner/t7`), the terminal Herdr shows for that machine; they write nothing to the
runner's hub. The machine must connect to the hc session the runner's hub reads (its default session unless the hub
runs with `HC_SESSION`), or these reach a different terminal with the same id. A runner's agent stays done and unread
only while the desktop's Herdr still shows it done: the runner never learns which agents you opened on the desktop, and
the desktop cannot tell it. Runner agents are not joined to local
terminals or tabs with matching IDs.

The review panel works on a runner agent: focus it (Go from the agents list) and the panel reads its folder, a runner
path, over TCP. Its three actions, `review send`, `review walkthrough` and `review stop_walkthrough`, travel as
`cockpit hub act review <action> <JSON args>` run on the runner through Herdr's machine connection, the way `hub start`
does; that command checks the arguments and hands the action to the runner's hub on its unix socket. Comments go only to
an agent of the runner (`runner/t7` reaches the runner as `t7`).

The runbook's step actions travel the same way, as `cockpit hub act runbook <action> <JSON args> [armed]`: `done`, `set`
(edit a cell or the status), `add`, `undo` and `run_step`. Run still asks first, and its confirm key goes along as
`armed`. Copy writes the desk's clipboard and never calls the hub. Every other action (refresh, talk, delete, open the
session or the PR) and Archive stay unavailable in this preview.

TCP has no TLS: use an encrypted private network such as Tailscale, and keep `hub.json` private. A hub listening
off loopback answers reads only, whoever holds the token; the mod keeps the same list so a remote desk says so
without asking. That holds for the runner's own window too, so leave `COCKPIT_HUB_LISTEN` unset on a machine whose
desktop writes. On loopback, the token is a file the same user can read, so any process of theirs that reads it can
act as the window: the hub tells processes apart on its unix socket only, where it refuses permission answers from an
agent's own process tree (see `answer` under View `agents`).

Desktop checks: `node --experimental-vm-modules --test app/tests/*.test.mjs`.

## Window-wide plumbing (`commands.js`)

- Keys (`app/keys.json`, sections `layout:cockpit` and `layout:cockpit-chat`): `cmd-shift-p` → `cockpit.palette`
  (cockpit's palette, a superset of Herdr's), `cmd-/` → `cockpit.keys` (the keys sheet), `cmd-alt-1..7` →
  `cockpit.go:<inbox|runbook|todo|meetings|monitor|routines|mail>`. A `cockpit.<command>` reaches every shown copy's
  `command()` (a hidden one, such as a sidebar ⌘B closed or a screen behind another, hears nothing); prefix your own
  with your view's name (`todo:delete:<key>`).
- The window's commands live in `commands.js` and run in the one copy each layout always shows, its *runner*
  (`RUNNER`): `status` in Cockpit, `chat-top` in Cockpit Chat, each through `runCommand(c, cx, layout)`, which acts only
  while its layout (or a copy of it, such as `cockpit-chat-copy`) is shown. Both layouts: `palette`, `keys`,
  `recap[:<tab>]`, `next-waiting`, `next:blocked|done`, `panel[:<child>]`, `doc:<name>`, `go:<screen>` (a screen the
  layout has, or `work`: where you deal with an agent), `via:<screen>|<command>` (shows that screen, then hands its
  view the command), `pr`, `ticket`, `resume`, `link:<url>`, `target:<row>`, `update`, `kit[:<tab>]`,
  `code:open:<path>`; Cockpit only `home` and `agent-view`; Cockpit Chat only `sidebar`, `new`, `terminal` (⌃`: the
  chat's subject in a terminal; for a driven session, the chat's question before it is handed over to one) and
  `session` (⌘⌥N: a session cockpit drives, opened in the chat). `via:chat|chat:open:<key>` pins the chat's subject
  at once (`pinChat`), and going to an agent on purpose (`goToAgent`, a row, a tab pill) unpins it. The queue drawer (`queue`, Cockpit) and the chat (`chat:open:<key>`, and from the
  palette `chat:mode` and `chat:stop`, Cockpit Chat) handle their own; the chat's keys (⇧⇥, ⎋, Tab, the digits) are its
  own while the keyboard is in it, never window bindings, so a menu or the palette keeps them (`CHAT_KEYS` lists them
  in the keys sheet). Tab, ⇧⇥, ⎋ and ⏎ are bindings scoped to the chat's key context (`chatBindings`, `CockpitChat`
  and `CockpitChat > Input`), whose flags say which of them the chat has a use for now (`chatContext`): GPUI gives a key
  to the box's own bindings and the window's focus moves before any `on_key_down`. The keymap is the app's, so a binding
  stays bound until Herdr restarts; one with no handler on the focus path falls to Herdr's commands. A question's digits
  stay `on_key_down`, and the next steps' 1, 2, 3 and 0 are bindings flagged only while the strip shows; a held ⏎ never
  sends. The palette, the drawer and Needs open a chat as `cockpit.via:chat|chat:open:<key>`, and the
  Cockpit layout reads a driven session in Cockpit Chat (`readInChat`, its row and menu). `app/tests/keys.test.mjs` checks every binding and
  every palette entry reaches exactly one handler in its layout.
- `CATALOGUE` names every command (title, icon, group, the layouts and screen whose view handles it, whether it is a
  driven session's), for the palette and the keys sheet; `available(layout, c, {driven})` says whether one does
  something in a layout. `hint(label, action)` writes a tooltip with the key the shown layout binds ("Home  ⌘⌥0" in
  Cockpit, "Home  ⌘⌥8" in Cockpit Chat): the shell's views write no window key by hand.
- The runners and the agents drawer lend the agents view they follow (`lend`); ⌘J and the palette read it from there
  (`lent()`) and only ask the hub when no copy follows it. ⌘J passes an agent with no running terminal (Herdr's
  `terminals()`), and one the last press could not bring up, and says which it passed. Each lender notes the load of hub.js it runs, the newest
  load's asked first, and a load that stopped running lends nothing (`globalThis.cockpitAgentLent`). A runner starting puts `cockpit.app` back on one of its
  layout's screens (`settle`).
- `screens.js` lists each layout's screens (`screens().list`: child, label, icon, the command its key is bound to);
  `show(child)` switches the layout's stack and gives that view the keyboard (`show:<app>`), `showWork()` brings up
  where you deal with an agent, `showPanes()` the tab's panes.
- Notifications (`notify.js`): everything goes through `cockpit_core::notify::post` (or `cockpit notify`). With the app
  frontend the hub hands `{title, body, action, row, buttons}` to the sidebar (`subscribe {notify: true}`), which posts
  Herdr's own toast while the window is in front, else the system notification. `row` is where the one click leads:
  the row the notification is about (`inbox:<PR url>`, `inbox:<ticket id>`, `runbook:<slug>`, `routine:<slug>`, ...),
  where its other buttons are; the toast runs `cockpit.target:<row>` → `cockpit focus <row>` → the hub's `go`, so the
  view gets `{focus, target: {app, key}}`. A system notification posted for the app prefixes its click and buttons
  with `herdr-app:`, so `cockpit focus` drives the app whatever environment the notifier runs it in. A click or a
  button only ever navigates: `act:<review|ci|rerun|merge>:<PR>` leads to the PR's row, `act:plan:<ticket>` to the
  ticket's and `act:routine:<slug>` to the routine, where the person acts (`notify.js` `rowOf`, `cockpit focus act:…`),
  and `update` opens the update dialog; only `resume` resumes, as `cockpit focus resume` does. Act buttons
  (`act:<review|ci|rerun|plan|merge|routine>:<ref>`) are the hub's own threads' alone: a notification that comes over the
  socket (any client, `cockpit notify` included) keeps only targets that bring something into view (`inbox:`,
  `runbook:`, `space:`, a tab, a URL; `notify::navigation`), an act turned into the row it is about, and reaches the
  window as `{title: "From <sender>: <title>", body, action, row, buttons, from: <sender>}`, its words plain text
  (`fetch::untrusted`) that name who sent it (`a terminal`, `an agent's tools (<handle>)`, `a process cockpit cannot
  place`). `cockpit notify` itself turns an act into its row before it posts.
- Palette (`palette.js`): a full-window frameless popup drawing Herdr's palette geometry (620 wide, 60 from the top,
  10 rows of 32). Its entries are the shown layout's: its screens, the commands that do something there, each with the
  key it has there as a Kbd chip in Cockpit (Cockpit Chat keeps keys to tooltips and the sheet). With nothing typed it
  lists the agents in the hub's order (`view palette {agents: "app", rows}`, below), under "Waiting on you" and
  "Agents", then what was picked lately; picking one brings its terminal into view and marks it seen. When no command
  matches what was typed, every palette ends with one entry, id `start-work`, that starts work on those words, after
  what the hub found: the desktop's "Start work: <query>" opens the Start work dialog on them (`openStart(query)`) in
  Cockpit and the chat's own start box in Cockpit Chat (`cockpit.via:chat|chat:start:new:<text>`, keyed), started by
  the person from there; the herdr TUI popup's "start work: <query>" runs `cockpit start-work <query>` in its popup.
  `cockpit.palette`
  again closes it; `cockpit.palette:type:<text>` types into it in a sandbox only (tests). Opened with `{sheet: "keys"}`
  (`cockpit.keys`, ⌘/) it is the keys sheet: the layout's bindings by group (Screens, Agents, This agent, Side panel, Window, In the
  apps, then Herdr's own keys the layout leaves alone), searchable by name or caps, ⏎ running the row's command;
  ⌘/ and ⌘⇧P turn one into the other in place. A popup the mod opens passes `popup: true` in its props (Herdr's own
  `overlay` is the overlay's id), so `front.js` treats it as one. Over a Claude
  agent, a found item goes into it through the hub: `act palette insert|ask|fork [over, found]`, over = `{terminal,
  cwd, title, session?}`; forking the session itself is `act agents fork {terminal: "app:t3", place}`. Open in
  editor is `editor.js`'s `openInEditor(folder, machine)` (the config's `editor`, else VS Code; Remote-SSH for a
  remote space).
- **Window commands** (`person.js`). `herdr-app do 'cockpit.<command>'` reaches the views as a menu click or a key binding
  does, from any process of the user, so a command alone only navigates: it opens a view, a menu or a dialog, selects,
  scrolls, fills a box. What acts (an agent's recap, snooze, seen or fork, a row's action, a merge, a ticket's state, a
  mail reply, a brief, a step run, a session's model or stop, Start work's picks, who gets a review) takes a one-time key
  minted in the window: `openMenu` mints one per opening for its items, the palette and the views' clicks one per pick,
  and the command arrives as `<command>~<key>`; `main.js` takes the key off and hands it to `command(c, cx, key)`,
  whose handler uses it once (`take`). Without it the handler only brings the row into view, or opens its menu
  (`review:to`, a session's start), and such a menu takes a click only (`clickOnly`): `cockpit.session` from its key
  binding opens a one-item menu to click. A selection or a focus a command made never arms a key: Home's digits answer
  only the card the person moved to or clicked, the recap's keys only the row moved to, the chat's digits wait for
  another key or a click after `chat:open` or `reply`, and a `show` block brought into view by `show:go` or a hub focus
  answers only a click or a move to it; a row a hub focus selected in an app takes no key that acts until the person
  moves to it or clicks (`holds`, core's `pointed::Pointed` in the TUI). What a command typed (`start:type:`,
  `chat:start:type:`) starts from a click on Start or ↑ only, never from ⏎, until the box is emptied;
  `chat:start:send` starts only with a key, or in a sandbox.
- `cockpit.key:<keystroke>` presses a key in the view in front only, and only in a sandbox: a window whose own process
  was started with `COCKPIT_SANDBOX` and `COCKPIT_TEST_KEYS=1` (`scripts/sandbox.sh` starts it so; `cockpit app install`
  never writes it, and no file says so), else it is dropped, so nothing outside the window answers or confirms.
  In front is the newest popup that takes keys, else the screen the shown layout's stack shows (`front.js` reads
  `layout()`), else the panes beside an agent's terminal; a copy a layout switch dropped stops counting once it stops
  beating (every second). The key arrives marked `injected`, and the chat's digits ignore it. A hidden Home or chat
  answers nothing; a permission's number answers only the card the person moved to (Home, the recap) or focused (the
  chat's Tab), never a key held down, and no chord answers one. `cockpit.chat:<command>` reaches the chat only when it is in front.

## The shell: layout, Home, status bar, bell, side panel, queue

`app/layouts/cockpit.json`: sidebar · a top bar (`tabs` + `bell`) · the `center` stack, `home` first and shown at start
(the board, with a hidden `home-agent` column: `agent-head` over `{agent: "focused"}`), `work` (a `view` stack: `main` =
the tab, `agent` = the focused agent alone), the apps · the `status` bar; in `work`, the right `panel` (stack
`panel-body`: `show`, `review`, `runbook-panel` (`runbook {scope: "focused"}`), `files`, `changes`, then the `activity`
bar); floating on the root, the `queue` drawer (right edge, 44px peek, shown only while prompts are queued).

- **Home** (`home.js`, hub view `home` from `cli/src/home.rs`): a dashboard. A greeting with one line of counts, KPI
  tiles (agents needing you, reviews for you, your PRs, merged this week, agent spend, estimated), the NEXT MOVE callout
  (only an agent waiting on you: its question with its choices as buttons and a reply field), the agents by stage as wrapped
  cards (empty stages on one line; the agents pinned here or in the sidebar first, in a Pinned row, out of their stage's
  row but still in its counts; pins live in the shared `cockpit.pins`), then your PRs no agent works on (to fix, to
  land). On the right: Up next (meetings),
  Apps (each pinned app in a few words, a click away) and Activity (the hub's events of the last days). "Needs you" means
  agents only. With no card picked, the keys act on the NEXT MOVE agent, except `1–9`, which answer only a card moved to.
  Keys: `j k h l` move, `⏎` go, `1–9` answer the card moved to or clicked, `t` reply, `f` the agent beside the board, `m` merge, `a` archive (tab
  settled, Undo on the toast), `o` PR, `p` pin, `r` recap, `s` snooze, `q` queue, `x` dismiss the card moved to (it leaves Needs you until it asks
  something new, Undo on the toast; on a dismissed card it brings it back), `x` twice stop a working agent's turn, `n`
  new agent (the Start work dialog); right-click for the rest.
- **Status bar** (`status.js`): Home's counts (`fleet.counts`, each a click) on the left; on the right the agent in
  view's (`agentInView`: the agent block's while one is on screen, else the focused pane's, else one in the tab in
  front; never the first agent Herdr names for its blocks when that tab runs none) branch, ↑↓ and files, its links
  (the agents view's `links`: PR, ticket, CI, deploy dashboard and run, then the
  pages its transcript cites, each named after its service, always in that order), queued prompts, agents to resume,
  the hub only while it does not answer, and its `documents` at the right end. It is this layout's runner (`commands.js`):
  `cmd-alt-0` → `cockpit.home` (Home ↔ work), `cmd-alt-a` `agent-view`, `cmd-alt-b` `panel`, `ctrl-cmd-1..5`
  `panel:<show|review|runbook|files|changes>`, `cmd-j` `next-waiting`, `cmd-shift-j`/`cmd-shift-u` `next:blocked`/
  `next:done`, `cmd-g` `pr`, `cmd-l` `ticket`, `cmd-alt-r` `resume`, `cmd-r` → `cockpit.recap` (view `agents`' popup),
  `cmd-/` the keys sheet. `cmd-alt-q` → `cockpit.queue` (the queue dialog) is the queue drawer's own.
- **Code** (`code.js`, `{app: "code", file, line?, terminal?, tab?}`): a file in a pane beside the agent, to read and edit:
  syntax highlighting, ⌘S saves (refused when the file changed on disk since it was opened), the agent's edits show
  as it makes them, and Markdown gets a live preview beside it (⌘⇧V). "Ask the agent" turns a note on the file, or on
  some lines (`12-18: …`), into a one-comment review. `openCode(file, {terminal, line})` keeps one per tab; `e` in
  Review opens the file under the cursor, and `cockpit.code:open:<path>` any file. In Cockpit Chat the file opens in
  the side panel beside the chat instead (`{app: "code", panel: true}` in `panel-body`, reading shared
  `cockpit.code.panel` = {file, line, terminal, at, back}); its Close gives the panel back to what it showed, or closes
  it, and the activity bar gets a button for the file. The chat's messages link their bare paths and URLs, and a code
  span holding only one (`textlinks.js`): a path opens here (`path:12` on its line), a folder or an image in the system's
  own app, a URL in the browser; the local images an answer names show under it, each opened full size with a click.
  Ported from yama-lei/herdr-editor-layout's editor (Apache-2.0);
  no LSP.
- **Reader** (`reader.js`): a window that reads a file or a text as rendered Markdown, selectable and copied in one click;
  `openReader({path | text, title, meta})`, and `Markdown(id, text)` for the same rendering inside a panel (a routine
  run's output uses both).
- **Bell** (`bell.js`): agents waiting, breathing; click = the longest waiting, hover = their questions, right-click =
  pick one. **Activity bar** (`activity.js`): the panel's children with badges from shared `badge.<name>` =
  `{count, tone}` (write it only when it changes); it adds its panels to `workspace.panels` so a space switch keeps them.
- **Prompt queue** (`queue.js`, hub doc `queue`, view and actions `queue` from `cli/src/queue.rs`): the view is
  `{queues: {handle: [{id, text, at}]}, paused: {handle: {why, at}}, asleep: {handle: true}, drain, count}`, `asleep`
  naming the driven sessions that are paused or whose agent ended. On every look the poller offers
  each agent at rest to its queue: the next item goes through `deliver` once the agent has rested 1.5 s since the last
  one went, shows no menu (its screen read again; one it cannot read is a no) and, in a herdr pane, has nothing typed in
  its box, all looked at again under the answer lock before it types; a screen it cannot read or a draft there leave the
  item for the next look, without pausing the queue, and an item whose session was ended meanwhile is dropped. A driven
  session that is paused or whose agent ended gets nothing on its own; `tell`, `send` and `resume` pick it up. The
  document notes the last item that went (`sent`, retired once a turn ends after it), so none goes twice, and who each
  queue was written for (`owners`: a driven session's id, else agent and folder); a queue whose terminal now runs
  another agent is parked, shown and never sent on its own. A failed turn or one the person stopped pauses the queue
  (`why`; a stop within 10 s of an item going, before any turn ended, puts it back first), a turn that ends well lifts a
  failure's pause, an ended driven session takes its queue with it, and `drain` is false when the config's `queue.drain`
  is. Actions: `add [handle, text]`, `tell [handle, text]` (what a box's Enter calls: sent now when the agent is ready
  and no queue of its own runs ahead, older items left queued when the queue is paused or `drain` is off; else queued,
  the note saying when it goes; once queued, always a "queued: …" note, even when the oldest it asked to send did not
  go, so the words are never offered to send again), `edit [handle, id, text]`, `remove`, `move [handle, id, to]`, `clear`, `send [handle,
  id | "all"]` (now, paused or not; all = one numbered prompt; refused while the agent waits on a question, and put back
  where they stood), `resume [handle]` (the queue is this agent's now). An agent's own tools write only to its own
  queue. Handles are `app:t3`, herdr pane ids, or a driven session's key. Shared `cockpit.queue.for` picks the agent.
  herdr's popup (prefix+shift+q) tells a typed line to the agent; its commands start with a colon (`:s`, `:s <n>`, `:e
  <n> <text>`, `:m <n> <to>`, `:d <n>`, `:c`, `:r`, `:q`).
- **Resume after a restart** (`core/src/resume.rs`, both halves): every minute the hub notes which Claude session each
  terminal runs (`~/.claude/sessions/<pid>.json` for app terminals, herdr's `agent_session` for TUI panes) in
  `~/.cache/cockpit/resume/agents.json`. A session not running after a reboot or a herdr-core restart is lost: one
  notification (its click resumes them all), Home's banner, the status bar, `cockpit resume [look|all|<handle>|dismiss
  <handle>]` and `cockpit focus resume`. Resuming types `cd '<cwd>' && claude --resume <id>` into its terminal when it
  sits at a shell, opens a space in its folder when it is gone, and refuses when another program runs there.


**Links, badge and drags.** Both layouts list the mod in `link_handlers` (Herdr 0.1.17): `link.js` takes a Cmd-click on
a web link in a terminal, passes it back to Herdr unless `links.open` is `side`, and then runs `cockpit browse` for the
clicked terminal. The sidebar puts the count of agents that need you on the Dock icon (`setBadge`, `dockBadge`), Files
and Changes rows drag their paths into a terminal (`drag_paths`), and the mod starts, asks and follows nothing while
`herdr-app check` loads it (`hub.checking()`). In Cockpit Chat the boundary between the conversation and the side panel
drags. The pane header shows a name given with Rename (`pane.rename`), and ctrl-alt-arrows move a pane's edge
(`pane.resize`).
## The reading layout (`cockpit-chat`)

`app/layouts/cockpit-chat.json`, its brief in `app/RECIPE-cockpit-chat.md`. It draws the conversation instead of
showing the terminal: a `screen` stack over a 56px `chat-top` bar, beside the cockpit layout's own sidebar (the same
`workspace-chrome` brand row, `sidebar` view and footer), which ⌘B opens and closes. Its agent screen is a row of the
`view` stack (`chat` and `tab`) beside the same `panel` the cockpit layout has, so `show`'s charts, the review screen,
files and changes are ⌘⌥B away in both, and so is `web`, the Chromium side browser's slot. Its panel also has `browser`, a block of the tab in front: a terminal-browser
split beside the agent opens the panel there, at least 600px wide, with its tab zoomed on the browser (`browser.js`,
on the top bar's clock), and the chat's box keeps the keyboard. The panel put on another child or closed, the Tab
screen, or another layout gives the tab back unzoomed; the Tab screen hands the browser back to the panel on the way
back to the chat, and a browser closing closes its panel. Its own theme pair,
`themes/cockpit-chat.json` and `cockpit-chat-light.json`: near-monochrome greys with colour kept for what an agent is
doing, and for the providers' own logos.

- `chat` (`chat.js`) follows one named agent, its *subject*: the agent in view (`frontAgent`: the focused pane's, else
  the one in the tab in front; none on a tab no agent runs in, where the chat is empty, never the first agent Herdr
  names for its blocks), or the one `chat:open:<key>` named until another agent comes in front (shared as
  `cockpit.chat.subject`). The focus landing on a shell, or on nothing, keeps that one, and so does a hub restart: a pinned agent goes only once the answering hub has not listed it for a while. It subscribes to the hub's `chat` view for that terminal,
  the `queue` view, and the `session` view `{key}` only for a session cockpit drives, and draws a 760px reading column:
  a one-line header (status, title, place, branch, PR; the PR chip's menu says how ready it is from the row's CI and
  reviews and the hub's `pr_merge`, and offers the inbox's `merge`/`automerge` acts that apply, each asked once more
  under the header), what you said as a bubble, what the agent said as Markdown
  with fences split into their own boxes, and everything it did between two answers as one row of chips. The question
  card (`QuestionCard`, wide) and the box sit under it; each subject keeps its own draft. Files ride with the prompt as
  chips: dropped on the box, chosen from its +, or pasted with ⌘V when the clipboard holds an image (`paste.js`: the
  system's own tool, osascript or wl-paste or xclip, writes it to a new file only its owner reads, at most 20 MB, and
  deletes this person's pastes older than a day); a held ⌘V pastes once, a paste with no image does nothing, and one
  that fails says so above the box. The box holds several lines (⇧⏎ starts one, up
  to eight show); ⏎ answers a question, else sends through `queue tell` (now when the agent is free, queued after the others while it works; a driven session
  through `session deliver`); the words leave the box once the hub took them, refused they stay with "Not sent ·
  Retry". The queued prompts sit above the box (edit, move up, send now, take back), a paused queue says why with
  Resume, and Stop interrupts a turn (`session interrupt`, or `agents interrupt` for a terminal that is `working` or
  `stoppable`). The hub down, it says so inline with Start it, and comes back by itself.
  **Model, effort, speed.** The box's chips (`picks.js`) show the model, the effort level and fast mode its agent
  offers (the row's `picks`; an agent without effort levels or fast mode shows no such chip), each a menu whose pick is
  a `pick:<setting>:<value>` command keyed by that opening of the menu (D9): a window command or an injected key changes
  nothing. A driven session takes it over its stream (`session model|effort|fast`), or "From your next message" when
  it is paused or its agent takes that setting only at a start. A terminal gets its agent's own command typed through
  `agents pick` while it rests; working, the chip says to wait, and one its agent changes only by hand (Codex's `/fast`)
  is shown, not offered, with "Change it in Codex's terminal" in its tooltip. A terminal's values come from its
  transcript (`chat` view `session.effort`, `session.fast`), what was typed showing until the transcript says so.
  **Reading.** Each row lists the messages it holds (`msgId`: an id, else role, time and words), and what must hold
  while rows change (folds, the message kept at the top, the drawn window, the last message the reader saw) is kept by
  message, not by row, since a turn's row grows when its earlier messages come in. The view stays on the newest row
  only while the reader is there; a wheel up freezes the drawn window where it is, so new rows never take rows away
  above the reader, and a "N new ↓" pill counts the messages with words that came since. Reading back, a conversation
  keeps the messages that slide out of the hub's latest page (`joinPages`), and a driven session moves what slid out
  of its live page after its older pages, fetching only a gap it never held. Where the reader is comes from the wheel
  and each row's measured height (Herdr's `ScrollHandle` only scrolls to a child), so a drag on the scrollbar counts
  as away until the pill; ticks that come while a slide or a load settles move nothing. Reaching the top, or "Earlier
  messages", reads further back once: a conversation raises `limit` up to the hub's 20,000, a driven session asks the
  `session` view for the page before `cursor.from`. The message at the top stays put: the view goes a view's height
  below it, then back up to it, revealed from below (`chat:earlier` and `chat:end` do the same from a command). At
  most `DRAW` (300) rows are drawn, a window that slides over the rows read already before anything is asked of the
  hub; their text views come from a pool kept by row, the least recently drawn given up first, so a row keeps its
  views while the window moves. Rows are made again only when the messages change. A driven session can be ended
  (`session stop`, asked first while it works) and says "Paused · picks up when you write" when it came back paused.
  **Starting work.** With no subject (no agent in front), the empty chat is Start work (`chatstart.js`): what the box
  holds is a prompt, a ticket's key or a PR's URL, planned by `start_work` as it is typed and drawn in place of the
  conversation with its blockers and their fixes, and the getting-started checklist under it. The folder chip turns
  into the repo and worktree picker, and an agent chip picks the agent: one cockpit drives (`drives`: its agent table
  row has a `stream` column, Claude Code today) starts as a session in this chat, any other in a terminal the chat
  follows; model, effort and speed chips follow it where its agent takes them at start. ⏎ runs `start work` and the
  steps show as they go;
  once the run made its terminal or its session (`made.pane`) the chat follows it there, the run on show until its
  agent has a conversation to draw, and what is typed next is a new form with the same picks.
  For checks driven from a shell, `cockpit.chat:start:type:<text>` types in the empty box of the chat in front (⏎ then
  leaves it to a click on ↑) and `cockpit.chat:start:send` starts it, in a sandbox or with a key only.
  `cockpit.chat:start:new:<text>` gives the chat a start box of its own, whatever agent is in front, holding the text:
  with the palette's key ⏎ starts it, without one only a click on ↑ does.
- `chat-top` draws the capsule tab bar, with a pill per session cockpit drives (the agents view's driven rows, a paused
  one marked so), the active tab's pill lit only while the chat is not pinned elsewhere. A tab's pill and its "+N" menu
  item bring in front what the tab holds (`showTab`): its agent's chat, the agent named to Herdr with
  `agent.focus`, or its panes on the Tab screen when no agent runs there (a shell, a tab Herdr numbers "3"). Pills fit the bar from the
  widths they were measured at (`fitPills`), the active one always; the rest wait behind a
  "+N" pill whose menu ⌘⇧A (`cockpit.tabs`) opens from the keyboard, under the pill, with its first item picked (a click
  opens it there with nothing picked). A pill names the tab by its agent's title while the
  label is Herdr's number or the agent's own name (`tabName`). The active pill's × closes the tab (`tab.close`, the
  next tab brought in front first), asking for a second click while its agent works or waits. A monochrome dot in the right capsule says whether the hub runs; down on this machine
  it reads "Hub stopped" or "Hub not running" and starts it on a click. Its last button shows the chat agent's terminal
  (`cockpit.terminal`), drawn with that agent's logo and named in its tooltip ("Show Claude Code's terminal"; a session
  cockpit drives asks before it moves to one), the terminal glyph only while no agent is in front. Each pill wears its
  agent's logo too. Being the view always shown, it is this layout's
  runner (`commands.js`): ⌘B,
  ⌘T, ⌘⌥B, ⌘J and ⌘⇧J/⌘⇧U, ⌘R, ⌘/, `cockpit.doc:<name>` and `cockpit.go:<screen>` (⌘⌥0 Chat, ⌘⌥9 Needs you,
  ⌘⌥8 Home, ⌘⌥T Tab). The sidebar's app strip and the left drawer's head lead to Needs you too.
- `side-peek` and `panel-peek` are the drawers on the two edges, floating blocks with `peek: 10`. Each draws nothing
  and drops its subscription while its pinned half is open, so the sidebar and its drawer — and the panel and its
  drawer — are never both following the hub. Whether a half is open is the layout's to say (`sidebarShown`,
  `panelOpen`), never a flag carried over from the other layout: `cockpit.chat.sidebar` and `cockpit.panel.open` follow
  it, so each drawer is there from the first frame and ⌘⌥B acts at the first press. The sidebar is open or put away, never between: the brand row's
  fold to a rail (`sidebar` under 160 px) puts it away at its kept width (`settleSidebar`, on the top bar's clock), as
  ⌘B does. The left drawer's head pins it back; under it, the palette and each app's screen as a row of icons, then the
  agents by stage, in the agents view's order and words, then Places: the window's spaces (six, then a link to the palette, which lists every one for the keyboard), a
  click bringing one in front; they stay under the list, outside its scroll. It carries the sidebar's update tag (`UpdateTag`, without colour) when a newer cockpit
  is out. `panel-peek` holds a `Show` of its own for exactly as long as it draws.
- `needs` lists every waiting agent as a question card, then the agents stopped with an error.
- `screens.js` holds each layout's stack and the screen it deals with an agent on (`chat` here, the panes in the
  cockpit layout), so `show()` and `goToAgent()` land in the right place.
- The rows are drawn with `list`, which measures each as it draws it; a list renderer takes no handlers, so a chip
  opens through the list's own `on_item_click` and the scroll is driven with a `VirtualListScrollHandle`.

### View `chat` (one agent's conversation)

```json
{"agent": {…the agents view's row…},
 "session": {"id": "…", "cwd": "/r/api", "branch": "me/eng-412", "model": "claude-opus-4-5", "context": 31.4, "cost": 1.84,
             "effort": "high", "fast": false},  // what its transcript says its last turn ran with (null where it does not say)
 "messages": [{"role": "user", "at": "…", "text": "…", "tool": null},
              {"role": "assistant", "at": "…", "text": "", "tool": {"name": "Bash", "summary": "mix test", "ok": true}}],
 "question": {…the same shape the agents view emits…},
 "more": false,
 "next": {"turn": "b5999e0a90930fb0", "state": "ready",
          "items": [{"label": "Fix the lint failure", "prompt": "Fix the lint failure on the unused import, then run lint again."}]}}
```

`args`: `{terminal, frontend, limit?}`, where `terminal` is the chat's subject (a handle, an address or a driven
session's key); without one, or for one the agents view does not list, `agent` is null and nothing else is shown.
`limit` is how many of the latest messages to send (400 by default, 20 000 at most); reading further back raises it.
Built again when the agents view moves and when a transcript it follows grows (a long turn writes for minutes while
its agent's status stays the same), in `herdr/tui/agents/src/chat.rs` from a `transcripts::Follow` per transcript (the
eight last read are kept):
a Claude transcript folds only the lines written since the last build, any other format is read again whole only when
its file changed, always with tool calls (`Detail::Tools`), which the session index and recaps never pay for. On 5,000
messages a build takes 0.8 ms with nothing new and 1 ms after a turn, against 38 ms reading it whole (debug build,
`chat::tests::measure_a_5000_message_chat`). The last 400 messages, `more` true when there were more. It has no
actions: a chat answers through `agents` and sends through `queue`.

**How a turn draws** (`app/mods/cockpit/chat.js`, `rowsOf`). A turn is what you said, then one row for everything
the agent did and thought, then the one thing it answers with — its last words. The work row says how long it took
("Worked for 2m 14s") and opens on a click to every call it made and everything it said on the way; a rule under it
separates the work from the answer. A turn still running keeps all its words in the fold and counts up to now; under
it shows only the step the agent is on, as its terminal words it ("Reading the config" over `$ cat config.json`,
read off the screen by the chat view's `task` and `detail`), else the transcript's last call, with "N more" opening
the rest. The box's bar shows the mode a terminal agent runs in and its focus mode, from its footer and transcript;
where its agent has them (`cycle`, `focus` in its row of the table), the mode is a menu the hub sets by pressing the
agent's own ⇧⇥ until its footer shows the mode picked (one that asks less going through the confirm), and Focus a click
that types `/focus` once it rests.

**Files and Changes** (`files.js`, `changes.js`, `projects.js`). In Cockpit Chat the side panel's Files and Changes are
cockpit's, unless `chat.projectPanels` is off. The chat view's `session.repos` names the git repositories its agent
works in, the latest first: its folder's own and those its calls changed or went into (a file written, a `cd`, a path
under its folder in a command), never one only read. The chat shares them with its folder (`cockpit.panel.where`);
Files lists the folder as a tree opened down to each of them, marked, with a repository outside the folder listed above
it and every changed file coloured by its git status, and Changes shows their uncommitted changes one at a time, a
switcher on top, with each file's diff and a commit of the one shown. Review follows the latest of them too.

**The box** carries what the agent may do and which model answers it, both read from the session and set on it:
`+` for a file, the permission mode, where it is, the model, and send. `/` completes the agent's own commands and
`@` the folder's files (`git ls-files`, else everything not hidden), by `hintsFor`.

**Next steps** (`chat.nextSteps: true` in the config, off by default; `herdr/tui/agents/src/next.rs`,
`app/mods/cockpit/next-steps.js`). When the subject's turn ends with an answer of at least 80 characters (at rest,
not stopped, not failed, no question, its last message its answer), the chat view's `next` is `{turn, state:
"loading", by}` while that agent is asked for up to three prompts the person is likely to send next, then `{turn, state:
"ready", by, commands, items: [{label, prompt}]}`, else null; `by` names the agent that writes them. Any provider's conversation gets them: the agent that answers is the
followed one where its row has `aside` switches (it forks its own session), else `chat.nextStepsAgent`, else the first
installed agent that has them, given the conversation as cockpit read it; with none installed the view says
`{turn, state: "unavailable", reason}` and starts nothing. The `aside` switches exist for Claude Code only: safe mode
disables plugins, hooks, MCP servers and other customizations, user and project settings stay unread, an empty tool set
removes read, write, shell and network tools, strict MCP configuration starts no servers, and session persistence is
off. The person's own sign-in (claude.ai or an API key) serves the run. Modified
command templates are unsupported too. A run forks its session where its row can (`run_fork`), else reads a fresh bounded digest of the
conversation's last 12 KB, including a suffix of an oversized answer. It runs at the agent's lowest effort level, on
`chat.nextStepsModel` when it names one (a name, or one per agent id such as `{"claude": "haiku"}`), else the agent's
lightest model (`light`: Haiku for Claude Code), else the session's own. Each run is billed to the
agent that writes them, the option's line says so (Claude Code writes them for an agent that cannot answer with no
tools, billed to the person's Claude account); a fork reads the whole conversation again. Only
the person's own look starts a run: the hub builds a view as the person's when a subscriber placed as theirs follows it
(`peer::with`), and an agent's tools reading `chat` get only what is kept. One run at most per turn (a turn is keyed by its answer,
kept in `~/.cache/cockpit/agents/next/<session>.json` across restarts), and a run for a turn the person has moved on
from is stopped with its entire process group and its answer dropped; a failure shows nothing and goes to the hub's log. The answer is untrusted
(D9): only the JSON array between its first `[` and last `]` is read, at most three entries, a prompt of 600
characters and a label of 48, text holding Unicode tag characters refused, escape sequences and control, format,
private-use and bidi characters removed, a prompt that runs something on send dropped (`!` a shell line, `#` a memory
note, in Claude Code, Codex and Gemini CLI), and a slash command dropped unless the session has it (a driven session's
commands are known and the view says `commands: true`; a terminal's are not, so its `/` prompts go). The chat cleans it
again and applies the same rules, and draws it above the box as a list, one line per suggestion as the agent's own
terminal shows them: the key that picks it in a small tag, then its words (the whole prompt in the tooltip), and "0
Dismiss" last; while the agent is asked, "Next steps…". It
is hidden while the agent works, during a question card, under the Full access confirm or a question under the header,
and when the box holds text. 1, 2 and 3 (bindings of the chat's key context, active only while suggestions show and
the chat is the person's, not pointed by a window command, so its digits type then) or a click put that prompt in the
box, focused, for the person to edit and send; 0 or ⎋ put them away for that turn, ⎋ only once no confirm or question
under the header is up. Nothing is ever sent or run from a suggestion, and a key held down or sent from outside the
window picks nothing.

### View `session` (an agent cockpit drives itself)

`herdr/tui/agents/src/session.rs`. The agent runs under `claude -p --input-format stream-json --output-format
stream-json --verbose`, so cockpit owns its stdin and stdout: what it says, the tools it calls and what they returned
arrive as events, already folded into the `{role, at, text, tool}` shape the chat draws. Its actions are `start {agent,
cwd, mode, model?, effort?, fast?}`, `model {key, model}`, `effort {key, effort}`, `fast {key, fast}` (fast mode on only
from a person), `send {key, text}`, `deliver {key, text}`, `stop {key, confirm?}` (End: its stdin closes and its process
group, which it runs in, gets SIGTERM, then SIGKILL after 3 s, and its prompt queue goes with it; while it works it
ends only with `confirm: true`, else "it is working: ending it stops this turn too", looked at and done under the
session's launch lock so no turn starts between) and `terminal {key, confirm, frontend?}` (the hand-over: refused
while it works, while an interrupt waits for its turn's result, while it waits on an answer, while its queue holds
prompts and before it said anything; else its new space first, then the session ended here and its process gone, so one
process writes it, then the resume line run through a script file whatever the shell; a failure after the end says
the line to run), and a session tells the hub to rebuild the view whenever it moves. It runs with `--replay-user-messages`: what was sent carries `echo: true` until
its own replay, or the end of its turn (words sent during a turn wait for the end of the next) or of the process,
and the chat draws "Sending…" under it meanwhile.

The view, with args `{key?, limit?, before?}`, serves `{sessions, session}`; with a `key`, `{session}` alone (a chat's
pills come from the agents view's driven rows):

```jsonc
{
  "sessions": [{                            // every session, the most recently started first, without its messages
    "key": "claude:1794…-0", "session": "00000000-acme-…", "agent": "claude", "cwd": "/…/api", "mode": "default",
    "model": "claude-haiku-5-5", "working": false, "pending": [], "ended": null, "failed": null, "interrupted": false,
    "paused": true,                         // brought back after a restart: the next prompt picks it up
    "title": "Make the daily quota configurable", // its first prompt's first line
    "count": 5012, "since": 1794…, "started": 1794…, "cost": 1.2, "usage": {…}, "rate_limit": {…}, "rate_limit_at": 1794…,
    "commands": […],
    "effort": "high",                       // the level its last init reported, else the one set; "" when unknown
    "fast": true, "fast_on": false,         // fast mode as set (null: never), and as it reports serving (null: not yet)
    "fast_why": "model_not_allowed",        // why it does not serve, in the agent's words, while it says so
    "models": [{"id": "opus", "label": "Opus", "resolved": "claude-opus-5-5", "efforts": ["low", …], "fast": true}], // its `list_models`
    "restart": false                        // a setting it takes only at a start waits: its next message at rest resumes it
  }],
  "session": {                              // the one `key` names, else the first: the same fields, and a page
    "messages": [{"role": "user", "at": "…", "text": "…", "tool": null}, …],
    "cursor": {"from": 4612, "to": 5012, "total": 5012, "more": true}
  }
}
```

The page is the `limit` latest messages (400 unless asked) before message `before` (else the latest); `cursor` says
where it sits, `more` that older ones come before it. To read further back, ask the view once with `before:
cursor.from` and put that page in front.

**Surviving a restart.** The command line comes from the agent table: its `stream` switches, the permission mode, the
model, effort level (`effort`) and fast mode (`fast`) when set, then a new session under the id cockpit picks (`new_session`: `--session-id <id>`) or the table's
`resume` (`--resume <id>`). `~/.cache/cockpit/driven/registry.json` (`{sessions: [{key, agent, session_id, cwd, mode,
model, effort, fast, pid, hub, prompted, started}]}`, `hub` the pid of the hub its process hangs off, written atomically when one
changes and never after the hub handed over) knows it from the start. A hub that starts (after a restart or an update's
hand-over) reads it and puts every session back `paused` first, its history read from its transcript and a mode that
runs tools or edits unasked set back to `default`; then a process its last hub left running is ended (its group, or
alone when an older hub left it leading none, its command line looked at again before each signal) and waited for, only
when its command line opens the agent's stream for that session's id and no live hub that started it holds it (a hub
still holding it gets 5 s to go), and the history is read again. A session never prompted comes back empty; one whose
transcript is gone or older than its log is kept (14 days) is dropped, and a notification says how many. The first
`send` picks it up under the same id, one launch at a time per session, and is refused while another process may run
that session (one naming it with `--resume`, `-r`, `--session-id`, with or without `=`, or the agent with
`--continue`/`-c` in its folder) or wrote its transcript in the last 5 s after our own process of it ended (`/resume`
inside a running agent); a session whose agent ended on its own, or just exited, is picked up the same way, and an End
during the start wins. A paused session takes a mode or a model the agent knows, for its next start, set under the
launch lock. Only a session in a turn or waiting on an answer keeps the hub up and puts an update off; one at rest comes
back paused in the next hub. What the agent writes on stderr goes to the session's log. `recap {terminal: key}` recaps
it from that transcript, and its row carries the recap. A restored session keeps its first start's `started`, so it
keeps its place in `sessions`.

Beyond the words, the stream carries what the reader had to guess at: `system/init` names the session, its tools and
its slash commands, `system/commands_changed` republishes those commands with their descriptions when they change —
together they are what the box completes `/` from, so the list is the agent's own and never goes stale —
`rate_limit_event` carries the five-hour window, and `result` carries the cost and the token use. Some sessions publish
no `commands_changed` at all, so `init` is the only list they get.

**Permissions.** A driven session is started with `--permission-prompt-tool stdio`, so the agent asks before a tool
runs on the same pipe: a `control_request` of subtype `can_use_tool` (its `request_id`, `tool_name`, `input`,
`tool_use_id`), answered by a `control_response` naming that `request_id` with `{behavior: allow, updatedInput}` or
`{behavior: deny, message}`. Each ask is pending under an id of its own, its tool call's (`toolu_…`), never reused; the
session view lists them in `pending` (`{id, request_id, at, tool_name, input, tool_use_id, description}`). A session
waiting on one appears in the agents view as a row whose `question` is a permission carrying that id as `ask`, so the
sidebar's NEEDS YOU band, Home, Needs, the recap and the chat's card draw it with no new UI, and answering routes to
the session instead of pressing keys at a terminal. The answer names its ask (`session answer {key?, ask, allow,
source}`, or the agents view's `answer` with `ask`): an ask that is answered, taken back or unknown is refused with
"The question changed — look again", and the views are built again so the card redraws. Only an explicit `1` (allow
once) or `2` (deny) answers it — words, ticks or a missing choice are refused rather than read as an allow, and no
choice is drawn as picked. An allow carries the arguments back unchanged, taken from the ask rather than from the
caller. An ask whose tool call came back, or that the agent took back (`control_cancel_request`, after an interrupt),
leaves `pending`: no card may stay up offering an answer nothing is waiting for. The card shows what the ask would do,
in full (`preview`: the command, the diff, the file, the plan, an MCP tool's input). The modes that run tools without
asking, `bypassPermissions` and `auto`, are set by `start` or `mode` only with `confirm: true`; the chat asks first,
naming the session, and ⇧⇥'s cycle never reaches them. Each session writes what it asked, what it was answered and
from where to `~/.cache/cockpit/driven/<key>.log` (kept 14 days). `cockpit session answer <ask> allow|deny` answers one
from a shell, at a terminal only: without a verdict, or with no terminal on its input, it refuses. The session runs
without the hub's terminal variables (`HC_*`, `HERDR_*`, `CLAUDECODE`, …), the hub stays up while one runs (idle exit
and an update's hand-over both wait), and its process is reaped when it ends.

**What the answer checks stop (D9).** Cockpit stops the person's own windows from answering the wrong thing, and stops
an agent's own tools, and any process cockpit started, from answering for the person, loosening a mode, acting on
another agent, starting agents, running commands with effects or bringing the window to an agent through cockpit. It
does not claim to stop a determined process of the same user that drives terminals or windows itself; the routes that
remain are listed below, and nothing else is claimed.

*The person's windows.* A card answers only the question it drew, by its `ask` or `asked`; a permission takes a number
picked on purpose, from a card moved to or focused, never a held or injected key. A choice in the question's `unasked`
(every tool runs unasked: Claude's bypass, Codex's full access) takes a step of its own, Cancel first, whose item carries a
one-time key made when that menu opened: a window command from outside (`herdr-app do 'cockpit.home:unasked:…'`) names
no key and answers nothing. The chat's mode menu is keyed the same way; a mode that asks less (`acceptEdits`,
`bypassPermissions`, `auto`) set any other way, the palette's ⇧⇥ entry included, goes through the chat's confirm (a real
⇧⇥ pressed in the chat cycles at once; an injected key does nothing). A permission the
screen does not show in full (`cut`: no frame tells where it starts, or nothing of it was read) is answered in the agent's
terminal: every surface says so in the hub's one wording (`terminal_only`) and offers no choice, and the hub refuses it.
A window command alone only navigates (see Window commands): what acts takes a one-time key the window minted for a
menu's opening, a palette pick or a click (`person.js`), and a test sends every registered command bare to hold that.
A menu a command opened (a session's start, `review:to`) takes a click only. A selection, a focus or text a command made
arms nothing: a digit answers only a card or row the person moved to or clicked (Home, the recap), the chat's digits and
a `show` block a command put in view wait for the person's own key or click, and typed text starts from a click only. A
row a hub focus selected (a notification's click, `cockpit focus inbox:<PR>`) is pointed at, in the inbox, monitor,
routines, mail, meetings, runbook and to-do of the desktop and the TUI: ⏎, its letters and digits (and the to-do's
space, which ticks an item there) say "Move to it first" until the person moves to it or clicks.
Keys and typing from a shell (`cockpit.key:`, `palette:type:`, `chat:start:send`) work only in a sandbox's own window,
started with `COCKPIT_SANDBOX` and `COCKPIT_TEST_KEYS=1`, which an install never writes. A notification's click or
button, `cockpit.target:` and `cockpit focus` only navigate: an act target leads to its row, where the person acts,
`link:` opens web pages only, and `update` opens the update dialog (`cockpit focus update` run by a process updates,
as `cockpit update` would).

*Who sent a request.* The hub names the sender (the unix socket's peer; over TCP, the process holding the source port,
looked up once per connection) and reads its whole line of parents, to the system's root:
- an agent's root anywhere on it (a watched terminal's own process or its foreground one, a driven session's process)
  makes it that agent's; the agents are looked at again when the line passes an agent's program none of them is known
  by, so a new agent's tools act on it at once;
- an agent's program anywhere on it (a name from the agent table, by the process's name, its executable or, under node,
  bun, deno, python or ruby, its script) makes it an agent cockpit does not watch, which may act on no agent;
- cockpit's own work is refused: the hub itself and what it runs (a routine, a driven start's setup), what a `cockpit
  show` pane runs for a block, and `cockpit focus`, `cockpit act` and `cockpit setup-run`, the sender itself as well as
  what they start. `focus` and `act` run what a click or a window command names (the window's `cockpit()` helper, the
  notifier), never the person's hand; `setup-run` runs start work's setup command, the repo's code, in the person's
  terminal;
- else the first window or terminal the person uses makes it theirs. A front is known by its executable's real path,
  never by the name a process gives itself nor by the PATH the hub runs on: the bundles Terminal (`Terminal`), iTerm2
  (`iTerm2`, `iTermServer` and the copy it keeps in `~/Library/Application Support/iTerm2/`), Ghostty (`ghostty`),
  WezTerm (`wezterm-gui`), kitty, Alacritty, Warp (`stable`), Herdr (`Herdr`), Herdr Core (`herdr-core`) and cockpit's
  notifier (`Cockpit.app`'s `cockpit-notify`, which runs a click only once clicked); and herdr, herdr-core, tmux,
  mosh-server, sshd and the common Linux terminals where they are installed: the system's own folders (`/usr` but
  `/usr/local`, `/bin`, `/sbin`), MacPorts, Nix, snaps, Homebrew's Cellars, and the account's `~/.local/bin` and
  `~/.cargo/bin` (its home from the user database), so a copy under that name elsewhere is none. An ssh login counts
  from another machine only: one from this machine (`ssh localhost`) is refused unless the config says
  `"trust_loopback_ssh": true`, and one whose client cockpit cannot read is refused. VS Code and Cursor are no fronts:
  their own agents run commands in their terminals, so a request from them is refused. An agent run in the background
  (`claude --bg`) sits under no front either; its tools are an agent's by its program.
- Nothing of these (a pid the table does not list, a process detached to the system's root) is refused.

Work the hub starts for a request carries where its sender stood, not a pid; a thread that carries nothing fails closed.

*What needs a person.* Every answer and every mode that asks less (`acceptEdits` included; Start work's
`bypassPermissions` and `auto` also take `confirm: true`, and a mode it does not offer is refused); every act a confirm
armed, whatever key it brings; starting, resuming or forking an agent (a session's `start` and `terminal`, `agents
fork`, Home's `resume`, Start work, the inbox's, monitor's, runbook's and routines' talks, fixes, plans and reviews, a
runbook's refresh, which forks its writer's session with edits allowed, the palette's fork, the review's walkthrough);
bringing the window to an agent (`agents focus`, the palette's insert, which also waits out a menu); taking a card out
of Needs you (`agents dismiss`, its own agent's tools included); a `show` view's
answer (an agent neither answers its own confirm nor comments on another's view); and what acts on the world for the
person: merge, auto-merge, ready, draft, dequeue, rerun, redeploy, roll back, previews, closing spaces, removing
worktrees, ticket and mail changes, deleting, editing or changing routines (`routines update`) and runbooks (a step
marked done, the runbook view's `done`, `set`, `add` and `undo`), dismissing a start's record (`start dismiss`, the
inbox's `start_dismiss`), sign-in, `doctor fix` and `doctor fix_all`, and `inbox run`, which runs nothing but a herdr or cockpit focus by
the hub's own binaries. `routines add` is open to an agent's tools only for a routine whose mode is absent, `default`
or `plan`; any other mode, one that asks less, needs a person, and a mode Claude Code does not name is refused. A runbook action names its runbook by slug alone (`{"$book": slug}`): the path its
prompt quotes and the session its owner file names come from the runbooks folder, never from the caller. An agent's
own tools may act on their own agent only: words for it (`deliver`, the queue, the palette's ask, a review's comments),
a stop, its recap, snooze and seen, its driven session's model and the modes that ask as much. No client writes the
`queue` document: the hub hands its items to agents, so they come in through the queue's actions alone.

*What reaches the person or an agent from a client.* A notification's act buttons and act click are the hub's own
threads' alone: one that comes over the socket (any client, `cockpit notify` included) only brings something into view,
an act turned into the row it is about, and its words are plain text that say who sent it ("From an agent's tools
(app:t3): …"), never cockpit's own; the plugin's notifications, posted from hooks that run inside agents, read so too.
`cockpit notify` turns an act into its row before it posts a system notification itself. CI news reaches an agent's queue only from events the hub's own fetches made (`by: "hub"`): a red run a client
writes into the inbox or ci document is never news. Text from outside (a mail, a monitor's error, a ticket, a PR's
branch, an event's title) never reaches an interactive shell as typed text: every agent cockpit starts reads its prompt
from a file of its own, made safe (`fetch::untrusted`), and the lines cockpit still types (a script's path, a resume, a
fork, an editor) drop the control characters a line editor acts on even inside quotes (`value::quote`,
`agent::shell_line`). A routine's prompt quotes the event that set it off; a runbook step goes to its session through
`deliver`, which refuses while the agent waits on a question.

*What stays open to agents, and why.* The documents other than `queue` (the plugin's hooks run in agents and keep them),
reads, search and the `show` tool. `routines run` and `routines add`: the routine skill has the agent that wrote a
routine try it, and a routine runs only what its file says, with the tools that file lists, as a headless run under the
hub, which answers and starts nothing; an agent can write that file anyway. So a routine set to run on events also runs
on an event a client's write made, and the hub's own news and briefs read documents a client may have written between
two fetches (quoted as data, never words to obey).

*The hub's environment.* The person's own hub (its cache the account's, however HOME spells the home: `/Users/me/../me`,
a link to it, a home whose `.cache` is a link to the account's) runs on what their login shell sets, plus the
starter's ssh agent, temp folder and language: a PATH, a herdr or hc binary (`HERDR_BIN_PATH`, `HC_*`), a demo world or a
test's stand-ins the starter carried are dropped. A hub whose cache is another (the sandbox, a test) keeps its
environment: it serves its own socket, not the person's.

*What remains, by design.* Each of these is a process of the same user acting by itself; cockpit sees none of it:
- `herdr pane run|send-keys|send-text`, `herdr agent prompt`, `hc keys|send`, and AppleScript or System Events keys:
  they type into any terminal, an agent's menu included, as a person's keys would. They do not make the hub answer.
- `herdr-app do <command>`: any window command that navigates (focus, open a view, a menu, a dialog); what acts takes a
  one-time key or a click the window draws. Herdr's own commands (`agent.focus`, `tab.select`, the pane commands) are
  Herdr's, which cockpit cannot gate: one that moves the chat to another agent leaves a digit typed into its focused,
  empty box able to answer that agent's plain question (a permission still takes its card).
- What stays a window command by design: `cockpit.resume` (⌘⌥R) and `target:resume` resume the agents a restart
  stopped, and a mode's `mode:<id>` without its menu's key opens the chat's confirm, which takes a click only.
- A Herdr window started with a sandbox's environment (`COCKPIT_SANDBOX`, `COCKPIT_TEST_KEYS=1`), or a login shell that
  sets them for the person's own window: its keys and typing from a shell are taken.
- A command started in a front: `open -a Terminal|iTerm|Ghostty …`, AppleScript's `do script`, a new herdr or Herdr
  window, `tmux new -d` or `send-keys`, `mosh-server`. It runs under a front past no agent, so the hub takes it for the
  person.
- `ssh localhost`, once the config trusts it.
- Files the person's processes may write: the config, routines, runbooks, a show view's `.answer` file, the notifier,
  and the install folders a front is known by that the user owns (Homebrew's Cellars, `~/.local/bin`, `~/.cargo/bin`);
  cockpit's own CLI run by the agent (`cockpit focus resume`, `cockpit update`) does in that process what the agent could
  do itself.
- The notifier run directly (`Cockpit.app/Contents/MacOS/cockpit-notify post …`): a system notification with any words
  and cockpit's icon. Its click goes through `cockpit focus`, so it only navigates, except `resume` and `update`, which
  resume the agents a restart stopped and update cockpit as `cockpit focus resume` or `cockpit update` run by the agent
  itself would.
- `cockpit hub stop`, or `kill`, from an agent's tools: the person's hub stops, and nothing starts it again until
  `cockpit hub start`.
- Pid recycling: the hub names a sender by its pid and reads its parents when the request is served; a sender that
  exits first leaves its pid to whatever process the system gives it next, which is placed in its stead.

**How a turn ends.** A driven turn whose `result` is an error leaves the session `failed` (its row's status and stage,
`error` its message); one you interrupt ends `done` with `interrupted`, drawn "stopped by you"; the next prompt clears
both. A terminal agent is read the same way: Herdr's `error` is failed, and a transcript whose last turn is Claude's
"[Request interrupted by user…]" or Codex's `turn_aborted` is done and interrupted, never working. The chat draws that
line as a quiet "Stopped by you" mark, never as your own bubble. The hub notifies once when an app agent turns failed,
or a driven session's turn fails or its process ends on its own (✗, once until its next prompt; a failure the hub
finds as it starts was told before it), and when a driven session asks before a tool runs (⏸, at most once a minute
per session).

**Control.** `control_request` on the same pipe takes `set_permission_mode`, `set_model` and `interrupt`, answered by a
`control_response` the session believes — so ⇧⇥ cycles default → acceptEdits → plan and ⎋ stops the turn, without the
model being asked anything, while the chat is the screen in front and holds the keyboard (its own key bindings, which
no injected key reaches). The palette's `cockpit.chat:mode` and `cockpit.chat:stop` do the same, except that the palette's
cycle moves into acceptEdits only through the chat's confirm, since a window command may come from outside the window.
The mode cycle uses the names the agent reports back, which is why the resting one is `default` rather than `manual`. An
interrupted turn reports the same `is_error` result a real failure does; the
session knows it asked for the interrupt and stays open for the next prompt. `set_model` takes an alias (`opus`,
`sonnet`, `haiku`, `fable`) and answers with the full name the picker shows. A pick (`session::pick`) goes over the
stream for the settings the agent table's `live` lists (Claude Code 2.1.325: `set_model`, and `apply_flag_settings` with
`effortLevel` or `fastMode`, the layer `--settings` writes at start); each init and result reports the effort level and
`fast_mode_state` back. A setting the agent takes only at a start, or any while the session is paused, is kept and
said to apply "from your next message", which ends the process at rest and resumes the conversation with the new
switches. At each start the session asks `list_models` for its catalog (`models`).
The `applies` facts distinguish a setter accepted while a turn runs from when its choice takes effect: the next model
call for a model, by the next turn for effort and speed. Each dropdown choice names its originating subject and is
refused after navigation to another agent.

Only Claude Code has a protocol cockpit speaks; `session::start` refuses any other agent, which keeps reading its
transcript as before. `cockpit session run <agent> <cwd> <prompt>` drives one from a shell, without the window.

## The hub on its own

- **Words for an agent** (`core/src/deliver.rs`): `deliver(handle, text)` is the one way cockpit sends an agent words
  (the queue, a review's comments, the palette's Ask, the `show` tool's answers, the chat): typed as its next prompt
  (`hc send`, which waits out a person's unfinished draft; `herdr agent prompt` in the TUI), or handed to a session cockpit
  drives through the handler `cli` registers. It refuses while the agent waits on a question or a permission, "<agent>
  is waiting on a question: answer it first" (a driven session's `pending`, or a terminal the poller saw blocked or
  whose screen shows a menu, read or not), looking and typing under the lock answers to that terminal take. Outside
  the hub it asks the hub (`agents deliver {terminal, text}`), so every caller meets the same refusals.
- **The agent table** (`core/src/agent.rs`) says, where it was measured, how a permission mode is passed at start
  (`mode`: Claude Code's `--permission-mode {mode}`), which keys stop a turn in the agent's own terminal (`interrupt`:
  `esc` for Claude Code and Codex) and how its permission prompts read (`permission`). An agent with none of them does
  without what needs them; `agents.commands.<id>.mode|interrupt|permission` in the config overrides them.

- **Servers** (`connectors/`, `hub/src/feed/mcp.rs`): the hub is an MCP client. A source whose `server` names an entry
  of `servers` in `config.json` (a URL cockpit logged in to with `cockpit connector login`, a URL with headers or a
  `bearerCommand`, or a local command) is fetched by the hub itself through the section's mapping, the same `tools` and
  `fields` the Claude plugin reads: tickets (with their assign, create and details verbs), CI runs, meetings, deploys and
  previews, the monitor's numbers (`observability`: Datadog's own tools, or any metrics and errors tool through `metric`,
  `monitors` and `issues` verbs and `queries` in its own language), and the show panes' Metabase, Datadog and `metrics`
  sources (`cockpit connector rows`), pull requests from a code host other than GitHub (`code`, with merge, ready and
  draft), and unread mail (`mail`, with archive and mark read; a reply is drafted by an agent, never sent). Agents read
  every source through the `cockpit_read` and `cockpit_health` tools (`cockpit mcp`, and the Claude plugin), which
  meeting briefs, deploy watches, runbook refreshes and CI investigations point to. Logins live in the macOS keychain
  (`cockpit-mcp`), else in `~/.config/cockpit/credentials/`. The hub also carries out the apps' requests (reruns, preview
  actions, ticket releases) for those sources, and warns of meetings where no Claude Code runs. `cockpit setup` finds the
  servers the agents' own configs hold, logs cockpit in, and has the default agent map a server cockpit has no mapping
  for (`cockpit connector map`).
- **Fetcher** (`hub/src/fetcher.rs`): while a source needs Claude Code's connectors (no server of the hub's own and no
  token), the hub keeps one Claude Code session idle at its prompt in a hidden herdr-core terminal named `cockpit-fetch`
  (no space, no pane, haiku, no tokens spent while idle); its plugin refreshes the inbox, CI, meetings and monitor like
  any session. `fetcher: false` turns it off. The store also refuses Claude Code's "$.mcp.call is not available in this
  mode" errors, so a session that cannot call MCP yet never writes over good data.
- **Names** (`core/src/names.rs`, `hub/src/names.rs`): every agent and driven session, whatever its agent, is called by
  one name everywhere (the agents view's `title`): the name the person gave it (its tab's label, or the session's own
  rename such as Claude Code's `/rename`), else the agent's own title for its work (the transcript's, such as Claude
  Code's `ai-title`, else its terminal's title without spinner glyphs or brand, never the agent's name, its folder or a
  path), else its first prompt in a few words (a slash command as "name args", its leading verb and filler words
  dropped), else the agent's name; automatic names are capped at 28 characters, cut at a word. No model is asked. Every
  20 s a tab takes its agent's name only while its label is Herdr's placeholder (empty or a number) or the name cockpit
  gave it last (`~/.cache/cockpit/names/given.json`): a label the person typed, or another tool such as
  herdr-automatic-rename gave, is never written over, and cockpit stops naming that tab until its label is cleared. A
  label that only names what runs there (`names::default_label`: a shell, or the program or agent by one of its names,
  as "Claude", "zsh" or "[2] claude") stays in the tab but is never shown as the agent's name. A
  space at its folder's name takes its agents' ticket ("ENG-412 · Store and enforce") or repo. `names: false` turns
  it off: no tab or space is renamed, nothing is read from transcripts, and an agent is called by its tab's label,
  else its terminal's title, else its agent's name.

## View `agents` (every agent terminal, both halves)

Built by `cockpit_agents::gui` in the hub, pushed again when an agent's status changes (the hub follows `hc events`
and looks when a terminal is made, ends or changes status, else every 10 s; every 2 s while an agent waits on a
question, the herdr TUI runs or the events are not followed), when the `inbox`/`ci`/`agents` documents change, when a driven session moves, or after its own action. Subscribe with
`{name: "agents", args: {frontend: "app"}}` (the Herdr app's terminals; `"tui"` for herdr's panes, no args for both).

```jsonc
{
  "agents": [{
    "terminal": "app:t3",                    // the handle every action takes (a herdr TUI pane id in the TUI: "w9:p1")
    "address": "t3", "frontend": "app",      // the frontend's own id; "app" | "tui"
    "space": {"id": "sp…", "name": "api"}, "tab": {"id": "tb…", "name": "quotas"},
    "title": "Daily quotas: enforce limits", // its name (see Names): the person's, the agent's title, its first prompt or the agent's name
    "title_source": "agent",                 // where that name comes from: person | agent | prompt | default
    "program": "claude",
    "icon": "claude",                        // Herdr's icon for its agent (`<herdr-app home>/icons/<icon>.svg`), from
                                             // `agent::icon` (the table's id, an alias or Herdr's kind: `antigravity-cli`
                                             // and `agy` are `antigravity`, `qodercli` is `qoder`); `agent` when it has none
    "reports_limits": true,                // this agent has a supported limit source; false means not reported
    "agent_name": "Claude Code",             // what people read for it: the table's name, else Herdr's, else its id
    "status": "blocked",                     // working | blocked | failed | done | idle (Herdr's `error` is failed;
                                             // views read it through `fleet.agentStatus`, an unknown word as blocked)
    "interrupted": false,                    // done because you stopped its turn (drawn "stopped by you")
    "error": null,                           // failed: what it stopped on, or null
    "since": 1793611400000,                  // ms, this status began (the hub's own clock: survives restarts)
    "waiting_since": 1793611400000,          // ms when blocked, else null
    "question": {                            // parsed from the screen when blocked and it asks something, else null
      "text": "Which date should old invoices keep?",
      "kind": "single",                      // single | multi | text (no choices: answer with free text)
      "choices": [{"n": 1, "label": "Issue date", "detail": "Matches the PDFs", "checked": null, "free": false}],
      "cursor": 1,                           // n under the cursor, "submit", or null (always null on a permission)
      "submit": false,                       // multi: a Submit item follows the choices
      "preview": "",                         // a permission: what it would allow, in full; else the box Claude draws beside a choice
      "permission": false,                   // asks before a tool runs, edits go in, a plan is carried out or a folder trusted
      "cut": false,                          // a permission cockpit cannot show in full: no frame on the screen tells where it
      "terminal_only": null,                 // starts, or nothing of it was read; then the one wording every surface shows,
                                             // and no surface offers its choices: it is answered in the agent's terminal
      "unasked": [],                         // the choices that turn on a mode running every tool unasked (Claude's bypass,
                                             // Codex's full access): each surface asks a step of its own before one goes
      "markdown": false,                     // the preview is Markdown (a plan): the card renders it instead of showing it as typed
      "view": null,                          // a plan kept as a view of its session (`plan-<session>-<n>`): cards show one line
                                             // that opens it in the side panel
      "asked": "9f3c0a12b4e6d7f8",           // a terminal's question: its fingerprint (its own block of lines, its choices,
                                             // and the count of prompts seen), sent back with its answer
      "ask": null                            // a driven session's permission: its ask id (toolu_…), sent back instead
    },
    "waits": null,                           // blocked with no question to read: {"kind": "limit", "text": "Reached its usage
                                             // limit · resets 08:00", "resets": "08:00"} once its last output says it reached a
                                             // limit (`resets` from its usage windows, "08:00" or "12 Oct 08:00", none once they
                                             // say nothing is full; else its screen's, with a time zone not this machine's; never
                                             // a guess), or {"kind": "terminal", "text": "Waiting at its terminal", "resets": null}
    "wait": "1793611400000:limit",           // what a dismissal names (`agents dismiss`): since when and what it waits on, or null
    "said": "Old invoices store two dates…", // its last message on screen, or ""
    "recap": {"text": "…", "at": 1793611000000, "fresh": true, "running": false, "error": null},  // or null
    "stage": "needs_you",                    // whose move: needs_you | failed | done | fixing | idle | working | review | merged | snoozed
                                             // | dismissed. done is an attention queue: opening the agent drops it to what its PR says
    "repo": "api", "branch": "me/eng-412-daily-quotas", "cwd": "/…/api",
    "ticket": {"key": "ENG-412", "title": "…", "url": "…", "status": "In Progress"},          // or null
    "pr": {"number": 2198, "url": "…", "title": "…", "state": "open", "draft": false, "review": "APPROVED", "ci": "SUCCESS"},  // or null
    "ci": {"state": "fail", "label": "lint failing", "done": 9, "total": 9},                 // running | pass | fail, or null
    "changes": {"files": 3, "add": 214, "del": 38, "ahead": 2, "behind": 0},                 // or null
    "context": 41,                           // the plugin's figure (percent), or null
    "cost": 1.84,                            // the session's estimate from its transcript (USD, view `usage`), or null
    "page": "https://…",                     // the page beside it, or null
    "time": {"agent_today": 3120, "ticket_today": 4800, "ticket_week": 18000},             // seconds, from the ledger
    "snoozed": {"until": 1793640000000, "label": "until 18:00"},                          // or null ("till done": until null)
    "dismissed": null,                       // {"at": ms} once the person dismissed what it waits on: `stage` is then
                                             // dismissed and `unread` false, `status`, `question` and `waits` staying what they are
    "seen": false,                           // looked at (`agents seen`) since this status began
    "unread": true,                          // needs_you, failed or done, not seen, not snoozed: what the bell counts
    "session": "4f1c…",                      // its Claude session id, when known (the recap and fork use it)
    "stoppable": false,                      // working, and its agent has a measured key to stop a turn (`interrupt`)
    "interrupt": ["esc"],                    // the keys the agent table says stop its turn (driven rows too), or null where none is measured
    "paused": false,                         // a driven session brought back after a restart: the next prompt picks it up
    "picks": {                               // what a person picks for its agent and how (`Agent::picks_json`), or null
      "agent": "Claude Code", "models": [{"id": "opus", "label": "Opus"}, …],  // the one catalog (`core::models::of`), which Start work and the routines read too: the
                                                                               // agent's own listing (`Agent::model_listing`) once it answered, else its table's; a
                                                                               // driven session shows the models it says it can switch to
      "efforts": ["low", "medium", "high", "xhigh", "max"], "fast": true,      // [] and false: no such picker
      "loading": false,                                                    // true while the agent's own listing is being read; forms refresh until it finishes
      "start": {"model": true, "effort": true, "fast": true},                  // taken at start (the Start work forms)
      "live": ["model", "effort", "fast"],                                      // a driven session takes them over its stream
      "applies": {"model": "call", "effort": "turn", "fast": "turn"},           // next model call, or by the next turn; shown in the chips' tooltips
      "typed": {"model": {"command": "/model {model}"}, "effort": {"command": "/effort {effort}"}, "fast": {"byHand": "/fast"}}
    },
    "load": {"cpu": 12, "mem": 340, "history": [3, 12, …]}  // its process tree: % of a core, MB, the last samples; or null
  }],
  "stages": [{"id": "needs_you", "label": "Needs you", "count": 2}, …],  // whose-move order, every stage
  "counts": {"needs_you": 2, "done": 1, "working": 3, "unread": 3, "total": 8, …},
  "next": "app:t3",                          // the longest-waiting unsnoozed agent that needs you (NEXT MOVE), or null
  "quota": [{                                // every agent's limits as bars (see view `usage`): none for an agent reporting nothing
    "agent": "claude", "provider": "Claude", "icon": "claude", "name": "5h", "label": "Claude 5h",
    "percent": 34,                           // or null: a source may give only a verdict
    "figure": "34%",                         // what it reads as everywhere: "full" once rejected, "≥34%" while `floor`, "near", "ok"
    "floor": false,                          // a stale percent before its known reset: use only grows, so it is a minimum
    "status": null, "status_words": null,    // allowed | allowed_warning | rejected, "near its limit"; the newest verdict
                                             // no older than the percent, a rejected one making the window full
    "resets_at": 1793620000000, "resets_words": "resets in 2 h", "length": 18000000,
    "elapsed": 0.42, "ahead": false,         // the pace tick, and use running ahead of it
    "warn": false,                           // past 80 %, or a warning verdict
    "as_of": 1793611400000, "as_of_words": "as of 12:04", "source": "claude-statusline", "source_words": "Claude Code's status line",
    "state": "fresh",                        // fresh | stale (dimmed, kept until its window resets)
    "fresh": {"source": "claude-statusline", "at": 1793611400000, "next": null, "every": 600000, "state": "fresh", "error": null},
    "tooltip": "34% used · resets in 2 h · as of 12:04 · from Claude Code's status line"   // "limit reached · …" once rejected
  }],
  "tickets": [{"key": "ENG-412", "title": "…", "url": "…", "agents": ["app:t3"], "today": 4800, "week": 18000}],
  "week": {"days": ["2026-10-27", …], "rows": [{"key": "ENG-412", "secs": [0, 3600, …], "total": 18000}]}
}
```

Actions: `hub.act("agents", action, [{…}])`, one object argument.

| Action | Argument | Does |
| --- | --- | --- |
| `answer` | `{terminal, choice: n}` · `{terminal, choices: [n…]}` (multi, then Submit) · `{terminal, text, choice?}`, each with the question's `ask` or `asked` and `source` (`"<view>:<click\|key\|command>"`) | Reads the screen again and checks it still asks what `asked` names (after every look, so a question that changes midway stops it), then presses the keys (`hc keys` / `herdr pane send-keys`), pastes text; a driven session's ask goes to the session. A missing or different `ask`/`asked` is refused: "The question changed — look again". Nothing is allowed by default: a terminal's permission takes only the numbered `choice` the person picked (Claude's "Yes, and don't ask again" included), a driven session's takes `1` (allow) or `2` (deny), and words, ticks or a number its screen does not show are refused. A permission is told by its agent's prompt shapes (the `permission` column of `core/src/agent.rs`: Claude Code's tool, plan and folder-trust prompts, Codex's command and edit approvals) and by its structure (no free-text field). Answers to one terminal take turns. Every answer, and a mode that runs tools unasked, comes from a person: the hub refuses one from an agent's process tree (its tools), which it names by the unix socket's peer or, over TCP, by the process holding the source port, and one whose process it cannot name. What that does and does not stop is D9's (see the session view). Detection fails closed: a menu whose choices read like a permission's ("Yes…", "don't ask again", "No, and tell…") is one, whatever the agent; its block runs from the full-width rule at column 0 above it; with no such rule on the screen nothing tells where it starts, so it is `cut` and refused with `terminal_only`'s wording, as is one with nothing of what it would do read. Its card always shows the first lines of what it would do. A plan approval (Claude Code scrolls the plan in a box, so its screen holds part of it) takes the whole plan from the transcript instead, where the agent's table (`PLANS` in `core/src/agent.rs`) says it is kept: the unanswered plan call's argument (Claude Code's `ExitPlanMode`, Qwen Code's `exit_plan_mode`), the plan file that call names (Gemini CLI), or the last message of a turn in plan mode (Codex); its `preview` is then the plan, `markdown` true, nothing cut, and the menu a permission whatever the agent's shapes say. The approval is told by its question or a choice naming the plan, or by Qwen Code's documented choices. Each plan of an app terminal or a driven session is also kept once as a show view of its session, `plan-<session>-<n>` titled "Plan n · <its first heading>", so it stays in the session's documents and side panel after it is carried out, and a revised plan is the next number; the question's `view` names it. Every card then shows the plan as one line that opens it in the side panel, and Cockpit Chat opens the panel on it by itself the first time it asks. A choice in the question's `unasked` ("Yes, and bypass permissions", Codex's "Full Access") is picked only with `confirm: true`. Every answer, taken or refused, is a line in `~/.cache/cockpit/agents/answers.log` with its `source` |
| `recap` | `{terminal, force?}` | A digest of the session's transcript (its task and latest exchanges) handed to an agent run kept aside: no tools, no slash commands, nothing saved; the session's own agent where it has that isolation, else one that does (cached per transcript); fills the row's `recap`. Asked by itself on haiku a minute after a turn ends when the agent was not seen meanwhile (config `agents.recap`: `auto`, or `manual`) |
| `snooze` | `{terminal, until}` | `until`: ms, `"1h"`, `"18:00"`, `"tomorrow"`, `"done"` (till its status changes), or null to wake |
| `seen` | `{terminal}` | Looked at in its current status (`unread` goes) |
| `dismiss` | `{terminal, wait?, undo?}` | `wait` is the row's `wait` the card showed (`"<since>:<what it asks>"`); one that no longer matches is refused: "The question changed — look again". The person sets aside what a blocked or failed agent waits on: its `stage` reads `dismissed`, out of `needs_you`, `failed`, `unread`, `next` and their counts, until it waits on something else: another question (a terminal's read twice in a row as another one, a driven session's next ask), a new wait (it worked again and came back), or a restart of the agent. The hub's state file keeps it across hub restarts. `undo: true` brings it back. It is neither a snooze (which ends at a time) nor seen (which is about finished work), and both stay as they were. A person's act alone: refused to an agent's process tree, its own card included, and to a sender the hub cannot place |
| `fork` | `{terminal, place}` | The session forked (`claude --resume <id> --fork-session`) in a new `tab`, or a split `right`/`down` |
| `focus` | `{terminal}` | Brings the agent's tab into view, and marks it seen |
| `pick` | `{terminal, setting: "model" \| "effort" \| "fast", value, source}` | A model, an effort level or fast mode (`value` a bool): a driven session takes it itself (`session::pick`); a terminal gets its agent's own command (`picks.typed`) typed through `deliver` while it rests (idle, done or failed), refused while it works or waits on a question, and for a setting its agent changes only by hand ("change it in Codex's terminal: /fast") or a value it does not take. Fast mode on only from a person; logged in `answers.log` |
| `interrupt` | `{terminal}` | Stops a working agent's turn: a driven session's interrupt request, else its agent's `interrupt` keys from `core/src/agent.rs` (Claude Code and Codex: esc), pressed between answers once its screen, read again, shows no menu; refused for an agent not working or with no measured key (`stoppable` false), while a stop already waits (5 s, or until it leaves its turn), and to another agent's process tree. The note is "asked it to stop", since the turn ends when the agent says so: then done and interrupted (told by its transcript first; a pane still shown working 5 s after a stop rests when its transcript says so), and its prompt queue pauses |

What a blocked terminal asks is read off its screen (`agents/src/prompt.rs`): a menu's choices, else the agent's last
message (from its last "⏺"/"•" line, its last four lines) when it asks something or an empty input box waits for words.
Its input box and everything under it, a status line ("7d 100%", "C 64%·5h"), a mode line ("auto mode on (shift+tab to
cycle)"), key hints, spinners and a list above the box are never a question or a choice; each agent's own shapes are
`agent::screen` in `core/src/agent.rs` (its footer, its spinners, the lines that say a limit was reached or is over),
every agent's footer for a program the table does not know. A limit is read only from a line that asks nothing, in the
agent's own words (any agent's for one with none of its own, never in a tool's output). With nothing to read the row has
`question: null` and `waits`, and no surface offers a reply. A screen that could not be read changes nothing. A terminal
its frontend still calls blocked once its screen shows no prompt takes its status from what the hub reads instead:
`done` when its transcript closed the turn, `working` or `idle` when the person's own words are the last thing above its
box; a menu on its screen, read or not, or a limit it reached keeps it blocked.

Dismissing is on every surface that lists what needs you, never as a snooze or seen: the Cockpit sidebar's Needs you
band, Home's NEXT MOVE and its cards (`x` on the card moved to), the recap dialog (`x`), Cockpit Chat's Needs screen,
the chat's question card; each card's ✕ button, the agents' right-click menus ("Dismiss until it asks again", "Bring back
to Needs you"), and the palette's This agent entries `cockpit.dismiss` and `cockpit.dismiss:back[:<terminal>]` (a bare
window command opens a menu naming the agent in front; only its keyed pick acts). Undo shows on the view's toast, as a row
above the cards in the sidebar and the chat, and as the click of Herdr's toast for the palette's. A dismissed agent stays
in the Agents lists with its real status, its mark a still ring and the word "dismissed", with Bring back on its row.

Facts per agent come from the plugin: store document `agents`, patched by terminal handle,
`{"app:t3": {session, transcript, cwd, branch, repo, ticket, task, page, context, cost, account, at}}`; the hub fills
what is missing from git, the inbox and `ci`. `quota` comes from `cockpit_core::usage`, as view `usage` below.

Drawn by `sidebar.js` (Needs you band answered in place, agents by ticket / space / whose move, Herdr's spaces list,
quota bars) and `recap.js` (the cmd-r dialog, its Time tab the week's ledger), from `agentsui.js`: `AgentsModel`
(follows the view, runs its actions, menu commands `cockpit.agents|<what>|<terminal>|<arg>` run by the sidebar copy),
`QuestionCard`, `Chips`, `QuotaBars`, `StageMark`; every agent named on the desktop has its logo beside it (`AgentLogo`).
The TUI has no pictures: there an agent's mark is one lowercase word, Herdr's own (`claude`, `codex`, `qoder`:
`agent::mark`), on a line that lists agents without naming them (the popup below), and a line that names the agent
keeps its name. In a shell or herdr's popup: `cockpit agents [pick|answer|recap|
snooze|seen|dismiss|stop|fork|week]` (`pick` and `answer` send the question they printed; a permission is answered there only at
a terminal; `pick` snoozes with `z 1 <when>`, dismisses with `x 1` (the apps' shared dismiss key), lists the dismissed
ones as `[d1]`, brought back with `x d1`, prints its keys with `?`, lists the working agents it can stop as `[s1]`, stopped with `s 1` (the
review and routines apps' stop key), and the sessions cockpit drives
as `[o1]` with their title and ask, ended with `end 1` (asking first while one works) or handed over to a terminal of
this herdr with `o 1`). `COCKPIT_AGENTS_DEMO=herdr/tui/agents/demo.json` (set before `sandbox.sh start`) replaces the
terminals with the acme demo's made-up agents, answered in memory, and adds its `driven` sessions, one waiting on a permission;
a session started there runs for real and comes back after a restart like any other.
`herdr/tui/agents/tests/fixtures/stub-claude` speaks Claude Code's stream-json and control protocol without an account:
point `agents.commands.claude.bin` (or `COCKPIT_CLAUDE`) at it to drive sessions that ask ("ask", "ask twice", "ask
both" in the prompt).

Recaps run through an agent that can answer with no tools, currently Claude Code with its built-in command templates
and the same `aside` switches as next steps: no tools, MCP servers, plugins, hooks or browser integration, and no
session written. The person's own sign-in serves it; other transcript formats use such an agent when one is installed,
and without one the recap says it is unavailable.

## View `palette` (its pages and the agents' order, both halves)

`cockpit_palette::gui` in the hub. `{report: "doctor" | "brief"}` is that page (`{title, rows: [{mark, tone, text,
url}]}`). Otherwise `{pinned}`, and with `{agents: "app" | "tui" | "all", rows?}` that frontend's agents in the order
every palette lists them, the herdr TUI popup included (`cockpit_palette::jumps`): the `rows` given (the fields of the
agents view's rows the order reads, from a copy of the window that follows it), else the agents view built for it.

```jsonc
{"pinned": ["inbox", …],
 "agents": [{"terminal": "app:t41", "address": "t41", "title": "Enforce daily quotas", "status": "blocked",
             "stage": "needs_you", "waiting": true, "driven": false, "help": "needs you · 6m · api › quotas", "icon": "claude"}],
 "listed": 8}   // how many an empty query shows, from the top: every waiting one, and the next ones up to 8
```

The order is by stage: waiting on you (`needs_you`, `failed`, `fixing`), the longest waiting first; then `done`,
`working`, `review`, `idle`, `merged`, `snoozed`, each the latest change first. A `driven` row is a session cockpit
drives: the herdr TUI leaves it out (and lists its own panes only), Cockpit Chat opens its chat. Its repo, branch,
ticket, PR and CI come from git in its folder and the inbox, as a terminal's do. Actions: `insert`,
`ask`, `fork` with `[over, found]` (above).

## View `usage` (every agent's limits and tokens, both halves)

Built by `cockpit_core::usage` in the hub from what already exists on the machine: Claude Code's status line
(`~/.cache/cockpit/limits.json`, `{at, limits: [{name, percent, resetsAt}]}`, which the plugin writes), the
`rate_limit_event`s of the sessions cockpit drives (`claude-stream`, as of the session's `rate_limit_at`, which may carry
a verdict and no percent), and the last `token_count` of each limit bucket (`limit_id`) in the newest Codex rollout of
the last 8 days' folders (`codex-rollout`: `rate_limits.primary/secondary`, plan and credits), Copilot's quotas as
GitHub keeps them while Copilot's CLI is installed (`copilot-gh`: `gh api /copilot_internal/user` with the sign-in `gh`
already has, at most every 10 minutes: premium, chat and completions, each limited one used up to its percent until the
reset date), and for any other agent the command `usage.commands.<agent id>` in the config names (`usage-command`: a JSON
list, or `{windows}`, of `{name, percent, resets_at}`, run at most every 10 minutes). Which agent reports
through which source, and in which words, is `agent::reports` in `core/src/agent.rs`. A file is read again only when
its size or time changed; limits.json caught half written keeps the last good reading.

The hub's usage thread reads them every 30 s, so a new reading reaches every surface within 30 s, and counts the
transcripts' tokens every minute, only what each file gained (each Claude message, Codex token count, pi or Qwen
Code assistant message, Copilot `assistant.usage` event and each Gemini CLI answer's latest token snapshot, so a fork
or a resume does not count its parent again;
`usage::counts_tokens` names the formats). A call is priced from the Claude price table when its model is there, else at
the price its own agent wrote beside it (pi's `cost.total`), else counted in tokens only; another agent's session gets
an estimate once one of its calls had a price. It rebuilds `agents` when a bar moved, `home` when the spend
did, and `usage` each time; the views only read what that thread keeps.

```jsonc
{
  "at": 1793611400000,
  "providers": [{                            // installed agents and any with a reading: readings first, the table's order
    "agent": "codex", "name": "Codex", "icon": "codex", "letter": "X", "reports": true,
    "state": "stale",                        // fresh | stale | none ("No recent reading") | unreported | unavailable
    "words": "as of 11:52 · stale",          // "Not reported by this agent", "Usage unavailable: …"
    "identity": {"signed_in": true, "account": null, "org": null, "plan": "Plus", "words": "ChatGPT Plus plan"},
                                             // this machine's account, shown beside the windows and never as theirs:
                                             // Claude's is oauthAccount's emailAddress and organizationName of
                                             // ~/.claude.json, nothing else ("This machine's Claude Code: …"); or null
    "credits": {"has_credits": false, "unlimited": false, "balance": "0"},   // Codex only, else null
    "error": null, "windows": [/* quota rows, shortest window first */], "tightest": {/* a quota row */}
  }],
  "windows": [/* every quota row, tightest first */], "tightest": {/* the first of them */},
  "token": "󰊚  C 64%·5h 38%·7d  X ≥12%·7d", // the herdr sidebar's $limits, grouped by agent: its letter, then each
                                             // window's `figure` (spaces in its token name become hyphens); each agent's tightest alone past 28 columns
  "sources": [Source],                       // each agent's windows as one header chip ("Claude 5h 7d · 2 min ago", as fresh
                                             // as its oldest reading), ids `usage-<agent>`;
                                             // a failed read as a failing one; `sources refresh` on one reads the files again now
  "loading": false,                          // true until the first count of tokens ends ("Loading usage…")
  "tokens": {                                // the last 7 days, estimated at API prices; null while loading
    "at": 1793611400000, "as_of_words": "as of 12:04", "estimated": true,
    "days": [{"date": "2026-11-11", "label": "Wed 11", "total": {…},
              "agents": {"claude": {"input": 1200, "output": 300, "cache_read": 62000, "cache_write": 0, "total": 63500, "cost": 0.42, "unpriced": 0}}}],
    "week": {"agents": {"claude": {…}, "codex": {…}}, "total": {…}}, "today": {…},
    "names": {"claude": "Claude", "pi": "Pi"}, // each counted agent's name, for one that is not installed or reports no limits
    "unpriced": ["gpt-5-codex"],             // models of these 7 days with no known price: tokens only, never dollars
    "covers": "Claude only"                  // what the estimate leaves out ("Claude, some models without a price"), or null
  }
}
```

The hub alone writes the herdr TUI's `$limits` token, source `cockpit` on the inbox space's row: on its first pass
whatever it is, then when it changes and every 5 minutes, each write lasting 11 minutes (`--ttl-ms`), so a hub that
dies leaves no reading behind for long. When the hub stops idle or on SIGTERM (`cockpit hub stop`), SIGINT or SIGHUP,
it stops serving, removes its socket, and clears the token (herdr given 2 s) only while hub.json still names it, so a
newer hub's token stays. herdr keeps a token per name, the last write winning whatever its source, and a clear from any source
clears it, so the plugin only writes limits.json (to a temporary file moved over it) and never clears the token.

Home's `spend` is this count's today and week, `{today, week, estimated, unpriced, covers}`, or null when no token of
the week has a price (never a made-up $0); its tile adds `covers` ("· Claude only"). The same count prices each Claude
session, every message once: to the session that started first (its first line's time, then when its file was made,
never when it was last written), and never to another once a count charged it, its transcript kept or deleted. The
result goes to `~/.cache/cockpit/estimates.json` `{at, sessions: {id: usd}, owned: {id: [message hashes]}, gone: {id:
since}, tickets: {id: usd}, archived: {ticket: usd}}`, one process at a time (`estimates.lock`) and never over a newer
count; the count itself is kept in `usage-scan.json`, so a new hub reads only what the transcripts gained. A booked
session whose transcript is gone keeps its last estimate for 30 days, then its tickets' share goes to `archived` and
its book is deleted, so no ticket total drops. A ticket's estimate is each session's,
split over the tickets its cost book (`~/.cache/cockpit/costs/<session>.json`, the plugin's) names in the book's
proportions; spend made on no ticket is booked under `(no ticket)`, which weighs in and is never shown. Every cost cockpit
shows is this one, marked estimated: the inbox's ticket costs and their `ticket_estimate` history, Home's spend and each
agent's `cost` in the agents view. Books of sessions that never got an estimate go after 30 days. Drawn by `usage.js` (the Usage
sheet, each agent under its logo with its state, account and windows, from a quota bar, Home's spend tile or the
palette's "Usage and limits"; ⇧R reads every agent's files again), `QuotaGroups` (as `QuotaBars`) in the sidebar and the
recap, each provider's logo and name over its windows, a provider with no source saying "not reported", one awaiting its source saying "no reading yet", and each provider's tightest window beside its logo in the Chat drawer, tinted with its text. In a shell:
`cockpit usage [--json]` (without a hub, the windows at once and the tokens once counted), also the TUI palette's
"Usage and limits".

## Claude sessions (the plugin)

The Claude Code plugin finds its frontend like the CLI: `HERDR_ENV=1` is the herdr TUI, `HC_TERMINAL_ID` without it the
Herdr app. In the app it writes no herdr tokens: what the TUI's sidebar rows show reaches the hub instead.

- **Agent facts** (both frontends): each interactive session patches the store document `agents` with
  `{"<handle>": facts}` when a fact changes, and `{"<handle>": null}` when it ends. `facts`: `session`, `transcript`,
  `cwd`, `repo`, `branch`, `ticket`, `task` (the TUI pane's topic, else null), `page` (URL beside it), `pageTitle`,
  `context` (%), `cost` (USD, this session), `account`, `at`, plus `frontend`, `terminal` (`t3`, null in the TUI),
  `space`, `tab`, `version`, `pr` (`{number, url, failing}`), `ci` (`{running, done, total}`), `clash` (`{label,
  space, tab}`: another session on the same branch), `views` (`[{name, open}]`, its show panes) and `last` (`{at,
  durationMs, text}`: its last answer's first line). Unknown is null.
- **Show panes**: the show tool splits the agent's pane with `pane.split {direction, pane, mod: "cockpit", props:
  {app: "show", name, session}}` (then `pane.zoom` for `zoomed`); the view reads `~/.cache/cockpit/show/<name>.json`
  (its `terminal` is the session's app terminal, `t3`)
  and leaves each answer as `<name>.<block>.answer` (`{block, type, value, label}`), as the TUI pane does. A show view
  subscribes with `{views: [{name: "show", args: {name}}]}`: the hub's `shown {view}` op (`cockpit hub shown show`)
  lists the args of live subscriptions, which is how the plugin knows a view's pane is open.
- **Browser beside an agent**: `cockpit browse <url> [session]` opens a tab in the terminal-browser already in the
  agent's tab, else splits the agent's pane right with `terminal-browser open <url>` and returns once it lists. App
  terminals carry Ghostty's variables and none of herdr's, so every terminal-browser call gets the session's pane
  (`HERDR_PANE_ID`, from `herdr-app ls`: the terminal `HC_TERMINAL_ID` names, else the one the caller descends from) and
  the app's socket (`HERDR_SOCKET_PATH`); `cockpit browse --env` prints them as exports, and the plugin sets them for a
  Claude Code session at its start. A session the window shows nowhere gets the reason and the default browser.
  Cockpit Chat shows that browser in its side panel (below).
- **The Chromium side browser** (`sideBrowser: chromium`, macOS; [browser.md](browser.md) is its guide): cockpit's own
  Electron shell (`browser/`), one process the hub starts (`cli/src/browser/hub.rs`) and every command reaches on
  `~/.cache/cockpit/browser.sock`, one JSON request per line (`browser/main/protocol.js` lists the ops). A browser
  belongs to a session and is named by its owner, the session's app terminal (`app:t3`). `cockpit browse` goes there
  instead of terminal-browser (`core/src/browser.rs` `engine_for`): `act browser open {url, owner, session, cwd}` starts
  the shell and opens the page in that session's browser, and `cockpit browse action` pins agent-browser to its active
  tab on Chromium's debugging port. The window is docked from three sides. The `web` view (`web.js`, the `web` child of
  `panel-body` in both layouts) only draws the slot and writes its size to the shared store (`cockpit.web.size`). The
  sidebar, which is in every layout, follows it (`followBrowser`): ten times a second it tells the hub the slot with
  `act browser slot` when it changed (its rectangle inside the window, whether the panel shows it, whether something of
  cockpit's covers it, the terminal in front), one request at a time, and it moves the panel with the session in front
  (`panelSteps`). The hub adds the Herdr app's process, the owner, the worktree of the folder in focus and its servers;
  the shell follows the Herdr window on screen through `cockpit browser track` and lays its window on the slot, one
  step above the Herdr window, when the session in front has a browser. The hub's `browser` view (`{enabled, installed,
  docked, running, run, shown, frame, current, tabs: {owner: count}, opened: {owner, at}}`) is what the slot draws: the
  shell's last frame while its window is hidden, and `opened` is what brings the panel to a page an agent opens. Notes
  a person sends from the page arrive as the shell's `grab.send {owner, markdown}` event, and the hub types them to
  that owner through `deliver`. The hub closes the browsers of sessions whose terminal is gone.
- **Open in editor** (`editor.js`, `openInEditor(folder, machine)`) and **fork a session** (`agents fork`) are in the
  palette and on the pane header (`{app: "pane"}`, hover: editor, fork to a tab, split, zoom, close).

## View `inbox` (the inbox tree, both halves)

Built by `cockpit_inbox::gui` from the same rows the TUI inbox draws (`model::model`), pushed again when the `inbox`,
`ci` or `brief` documents change, a request's outcome lands, `muted.json` or the costs change, or after its own action.
Subscribe with `{name: "inbox", args: {group: "feature" | "status"}}` (feature by default).

```jsonc
{
  "at": 1793611507779, "updated": "14:05",     // when the cache was written; "" before any data
  "state": "ok",                                // ok | empty (no data yet) | other_version (another cockpit version wrote it)
  "message": "",                                // what to say for empty/other_version
  "errors": ["Previews failed"],                // error lines no source chip tells: the mod's, a CI the hub does not read
  "sources": [Source],                          // GitHub, tickets, CI, deploys: see "How fresh each source is"
  "group": "feature",
  "counts": {"needs_you": 2, "reviews": 5, "direct": 2, "tickets": 7, "in_progress": 3, "prs": 6, "ready": 1,
             "failing": 1, "alerts": 2, "worktrees": 3, "cleanup": 1},
  "sections": [Row],                           // today, alerts, review, work, other: the TUI's top rows, in its order
  "prs": [{                                    // my open PRs, flat, for the home screen and hover cards
    "url": "…", "repo": "api", "number": 2198, "title": "…", "branch": "me/eng-412-quotas", "base": "main",
    "draft": false, "ci": "SUCCESS", "review": "APPROVED", "updated": "2026-11-02T10:00:00Z", "age": "3h",
    "ticket": "ENG-412", "failing": ["lint"], "running": {"done": 3, "total": 9} /* or null */,
    "ready": {"state": "ready", "text": "Ready to merge", "tone": "good"}   // ready | blocked | waiting | draft
  }],
  "tickets": [{"id": "ENG-412", "title": "…", "url": "…", "status": "In Progress", "statusType": "started",
               "project": "Quotas", "prs": ["<url>"], "cost": 1.84 /* USD, estimated, see view `usage` */,
               "secs": 4800 /* time on it this week, or null */}],
  "worktrees": [{"path": "/…/api-eng-412", "repo": "api", "branch": "me/eng-412-quotas", "merged": true,
                 "changes": 0, "pr": "<url>" /* or null */,
                 "blockers": [{"id": "uncommitted", "words": "1 uncommitted change"}]}],  // what keeps it, below
  "results": {"1793611507779-rerun.json": {"ok": true, "state": "done", "message": "rerun started"}}
}
```

`Row` = `{key, path, kind, section, name, expand, text | cells, info, acts, primary, status, item, children}`:
- `kind`: `plain` (headings, groups, detail lines) | `pr` | `review` | `ticket` | `alert` | `brief` | `muted` | `worktree`.
- `text`: `[{text, tone, bold}]` spans (tone `bad|warn|good|dim|accent|""`), plus `icon` (a lucide name) and `extra` on
  headings; `cells`: `{lead: [{icon, tone}], ref, title, title_tone, notes, notes_tone, age}` for PR, review and ticket rows.
- `info`: the details panel's lines, `[[span]]`. `item`: the PR (pr/review rows) or ticket (ticket rows) it stands for.
- `acts`: `[{label, key, main, confirm, call: {fn, args}, choices: [{label, call}]}]`: run `call` as `act("inbox", fn,
  args)` with `armed: confirm` once confirmed (the hub does nothing for a `confirm` act without it); `choices` asks first.
- `primary`: the call a click/Enter makes when no act is `main`. `status`: the key its requests' `pending` go by.

Actions (`hub.request("act", {app: "inbox", action, args, armed})`), positional args:

| Action | Args | Does |
| --- | --- | --- |
| `review` `rerun` `redeploy` `create_preview` `plan` `continue_ticket` `talk` `mark_done` `extend` `stop_preview` `close_space` `set_muted` `dismiss_brief` `request_refresh` `open_url` `details` | as the rows' `acts` give them | The TUI inbox's actions |
| `fix_ci` | `[pr, ci run, "here" \| "new"]` (or `[pr url]`) | What to do first, then, fenced and labelled as data, the PR, its branch, the failing job, checks and tests (`failed_tests`) and the end of the failing jobs' logs of the PR's head commit (`ci_log`, at most 12 000 characters); a log it could not read is said so, the reason in the data. "here" goes to the agent working on the PR through its queue's `tell`, else to a new one on the branch (a new tab in a space open there); "new" to a new one off main. A second press while the log is fetched is refused |
| `address_review` | `[pr]` | The PR's unresolved threads and its reviews' own words (`review_threads`), quoted as data, one JSON line per comment saying who wrote it and whether they collaborate on the repo, people's threads first and bots' comments left out unless `github.reviewBots` is true (a person's reply in a bot's thread stays). It goes to the agent working on the PR (the agents view's row on its URL, else on its branch, in its repo by owner and name) through its queue's `tell`; with none there, the first press asks and the second (or `address-<url>` armed) starts one on the branch. The TUI and Cockpit show it as `address review` (`F`), never the main action |
| `merge` | `[pr, method?]` (`squash` / `merge` / `rebase`) | `gh pr merge` pinned to the head commit, the merge written to the inbox's `merged` (with `headSha`) for its worktree's cleanup; adds to the merge queue where the base has one |
| `automerge` | `[pr, method \| "off"]` | Auto-merge on (by method) or off |
| `ready` / `draft` | `[pr]` | Draft ↔ ready for review |
| `dequeue` | `[pr]` | Out of the merge queue |
| `start` | `[ticket \| pr]` (armed `start-<id or url>`) | Start work on it with the last choices (`act start work`, below); the inbox's own rows open the Start work dialog instead |
| `set_state` | `[ticket, state name]` | The ticket's Linear state (by name, case-insensitive) |
| `assign` | `[ticket, "me" \| login]` | Assigns the ticket |
| `file_issue` | `[{team, title, description}]` | A new Linear issue, assigned to you; its URL is the note |
| `close_space` on a merged session's worktree | `[space, label, true]` (armed `close-<space>`) | Refused with "cannot tell which worktree this is" when herdr does not name its folder, and with what keeps it, leaving out only what ends with the space (its panes' processes by pid, from `herdr pane process-info`, with all they started, and the hub's agents in those panes); else `herdr worktree remove` |
| `remove_worktree` | `[path]` (armed `remove-<path>`) | Refused with `kept <path>: <blockers' words>` while anything keeps it, fetching (`--prune`, never asking for a password, 15 s at most, once a minute per repo) only when its merge is the last question; else `git worktree remove`, then its space closed and `git branch -d`, never forced |
| `open_worktree` | `[path]` | The space open on it in view, else a new space there |
| `forget_result` | `[file]` | Drops a request's outcome once shown |

**What keeps a worktree** (`cockpit_core::cleanup`, one rule set for the inbox, `remove_worktree`, the routines' run
worktrees and their cleanup template). Each blocker is `{id, words}`, said alike on every surface: `uncommitted` "N
uncommitted change(s)" (untracked files counted whatever the config hides), `unpushed` "N commit(s) not pushed" (past its
upstream, else past the commit the code host merged or a prune last saw, else past every remote and other local
branch), `ignored` "N ignored file(s) other than builds" (a removal deletes them; `BUILDS` are spared), `running`
"<program> still runs in it" (an agent the hub knows works there, else by `lsof` or /proc; shells and herdr's own are
idle), `locked` "locked: <reason>", `submodules` "it holds submodules",
`not_merged` "not merged into <base>", `unknown_base` "its default branch is unknown (git remote set-head origin
--auto)", and `unchecked` "cannot tell <what>" when a git probe or lsof failed or ran out of time (a probe never reads as
nothing there). Merged means commits of its own reached `origin/HEAD`'s branch, or the code host merged it at the commit the
worktree has (`merged[].headSha`); a branch merely gone is no merge, and a branch with nothing of its own is not held by
it. The **To clean up** section lists my worktrees that merged with no open PR, or hold changes, a live session's
excepted (the Alerts say those): `merged, nothing keeps it` with Remove worktree (`D`), or `kept by <words>` with Go
(`o`). Picked together (`v`/`V`), `D` asks once ("Goes: …. Stays: … (why).", long lists shortened) and removes only
the ones nothing keeps, one after the other, then says how many went and what refused the rest; any other shared action
runs on every picked row. A scan is redone at most every 30 s or once any process says a worktree changed
(`~/.cache/cockpit/worktrees.stamp`), off the TUI's keys thread, every git call timed out.

Without a Linear token (`tickets.token`), `set_state`, `assign`, `file_issue` and the ticket side of `start` hand the
change to a Claude session in the actions space. Details on demand (`hub.fetchDetails(kind, args)`): `pr_merge`
`{repo, number}` → `{ready: {state, text, tone}, choices: [{do, method, label}], why, card: {body, labels, additions,
deletions, files, reviewers, checks}}`; `ci_log` `{repo, branch, sha?}` → `{run, runs, sha, lines, jobs: [{job, step,
lines}]}` (every failed, timed-out or unstarted GitHub Actions run of the head commit, `sha` else the branch's newest; the
last 200 lines, and the first five failing jobs' last 150 lines each; another CI's failing jobs through its server);
`review_threads` `{repo, number}` → `{threads: [{path, line, outdated, bot, comments: [{author, body, bot, member}],
more}], reviews: [{state, comment}], cut}` (paged, `cut` when more were left). Text from outside reaches every surface
through one sanitiser (`fetch::untrusted`): terminal colours and commands, control, line-separator and bidi characters
out, a line rewritten by carriage returns kept as it ended, unusual spaces shown as `\u{a0}`; `ticket_card` `{id}` → `{description, labels, priority, assignee, states:
[{name, type}]}`; `start_plan` `{ticket | pr}` → `{ok, steps: [text], blockers: [text]}`, start work's plan in lines.

When a CI run ends (the events `ci_passed`/`ci_failed`, or `ci_passing`/`ci_failing` without a CI document) and the
hub's own fetch made the event (`by: "hub"`; a `store.put`/`patch` a client sent makes `by: "client"` events, which
never become news), the agent working on that PR or branch gets it added to its prompt queue (`hub/src/ci_news.rs`), never sent at once: the queue's
own drain sends it once the agent rests, unless the queue is paused or a draft waits. It counts as told once the queue
took it; an add refused is tried again the next batches. Red goes once per head commit (else run, else event),
asked to say what it would do and change nothing yet (or to carry on, when fix ci handed it the fix), the failing job
and tests quoted as data; green only after a red was told, which `~/.cache/cockpit/ci-told.json` keeps across hub
restarts. Newer news replaces older news of the same PR still queued,
runs on a default branch (a PR's head included) or on a PR's base are left out, and `ci.tellAgents: false` turns it off.

Everything handed to an agent asks first, naming the PR as `#<n>` only, then quotes what came from outside (the PR's
URL, its branch, the checkout, error text, logs, comments) in a fence labelled as data. A delivery never goes to a
terminal with no agent in it (`frontend::alive`, herdr's pane or `hc ls`'s `agent`). Every agent cockpit starts reads
its prompt from a file: `~/.cache/cockpit/prompts/<label>-<random>.prompt` (folder 0700, files 0600, written whole),
made safe and run by a script beside it; when it cannot be written, the action fails and nothing is typed.

## Start work (`start.js`, fetch `start_work`, view `start`, both halves)

A prompt, a ticket or a PR becomes a worktree, a space and an agent. Home's New agent (`n`), the inbox's Start on a
row, the palette's "Start work…" and its `start-work` entry when nothing matches open the dialog (`openStart(source)`, or `openStart({run})` on a start that runs
or failed); Cockpit Chat's empty box is the same form (see the reading layout); the TUI opens it from the inbox's `n`
and from `prefix+shift+s` (`cockpit start-work [source]`). The hub plans it again as the form changes, without side
effects, and only on this machine (refused off loopback):

```jsonc
// hub.fetchDetails("start_work", {request})
{"id": "g1793611507779", "text": "ENG-412 keep the API side only",  // a prompt, or a ticket's key or URL or a PR's URL first
 "source": "",                                                      // "prompt" reads the text as a prompt whatever it holds
 "repo": "api" /* a checkout of the repos folder */, "base": "main", "worktree": "new" /* or "here" */, "branch": "",
 "agent": "claude", "model": "", "effort": "high", "fast": true /* left out: the agent's own */, "mode": "plan",
 "existing": true, "dirty": true, "trust": "<setup hash>", "setup": false, "confirm": true, "path": "<folder>",
 "forkSetup": true, "driven": false, "lastRepo": false}             // set by fixes and options; lastRepo: no fallback repo
// → the plan
{"ok": false, "kind": "ticket" /* prompt | pr */, "title": "ENG-412 Store and enforce daily quotas", "label": "eng-412",
 "place": "worktree" /* checkout | here | space | focus */, "repo": "api", "remote": "origin", "base": "main",
 "branch": "me/eng-412-…", "branchFrom": "template" /* typed | tracker | pr | current */, "path": "/…/.herdr/worktrees/api/me-eng-412-…",
 "cross": false, "setup": {"command": "npm ci", "hash": "…"}, "agent": "claude", "agentName": "Claude Code", "model": "",
 "effort": "high", "fast": true, "mode": "plan" /* "" for an agent whose table row takes none */, "prompt": "…",
 "steps": [{"id": "fetch" /* worktree | space | setup | agent | ticket, in the order they run */, "text": "fetch origin/main"}],
 "blockers": [{"step": "branch", "text": "The branch me/eng-412-… exists already",
               "fixes": [{"label": "Use it", "set": {"existing": true}}, {"label": "Rename", "set": {"branch": "me/eng-412-…-2"}}]}],
 "options": [{"label": "Use as a prompt", "set": {"source": "prompt"}}],
 "notes": ["Your checkout of api has 2 uncommitted changes: the worktree starts from origin without them"],
 "choices": {"repos": ["api", "web"], "agents": [{"id", "name", "icon", "installed", "models", "modes": [{"id", "label"}], "drives", "picks"}], "agent", "repo", "mode"}}
```

- **Source.** A PR by its URL; a ticket the inbox or `tickets.keys` knows by its key or URL, or one the tracker
  confirms; anything else is a prompt. A ticket or a PR offers "Use as a prompt". A PR the inbox does not say is a
  fork's or not is asked of GitHub.
- **Blockers** (`step` names the step): `empty` (no source yet: start in the checkout with no prompt, or type one),
  `source` (a closed or merged PR or ticket, a ticket someone else holds), `repo`, `github`, `base` (unknown default
  branch, a typed base the remote lacks), `branch` (exists: Use it / Rename; not a branch name; nothing to name it
  after), `worktree` (a folder in the way: Use another folder; uncommitted changes when working in the checkout),
  `setup` (new or changed: Run it / Skip it this time), `agent` (not installed; Full access not confirmed).
- **Forks.** A fork's PR is fetched into `<remote>/pr/<n>` and checked out as the branch `pr/<n>`, never onto a branch
  of yours; a local `pr/<n>` that is not its head blocks. Setup stays off for a fork's code unless asked
  ("Run setup from the fork's tree"), and your checkout's git hooks run on it when its worktree is added.
- **Remote and base.** The remote whose URL names the PR's repo, else origin; the base is the remote's HEAD, else its
  only main or master, else asked.
- **Mode and model** go only to an agent whose table row takes them (the `mode` column, `models`), and so do the effort
  level and fast mode (`effort`, `fast`; a value it does not take is left out with a note). The forms offer them as its
  `picks` say: pills in the dialog (Model ⌃M, Effort ⌃L, Speed ⌃S cycle them), chips in Cockpit Chat's empty box, ‹ ›
  fields in the TUI; each starts as the agent's own default, a model it lists none of is typed, and another agent or a
  model without them puts them back to the default.
- **Pressing again.** A space of the start's label on the same repo plans `focus`; on another repo the new space gets
  `<label> (<repo>)`. A branch already checked out with a space open on it, or a live session cockpit drives in it
  (`session: true`), plans `space`: the run brings it up ("is open already: brought it up"), its key in `made.pane`
  for the chat that asked, and makes nothing.

Actions (`hub.act("start", …)`): `work [request]`, `cancel [id]`, `retry [id]`, `dismiss [id]`; the inbox's rows use
`start_retry`, `start_cancel` (armed `cancel-start-<id>`) and `start_dismiss`, and open the form on the run itself
(`start_open`). `work` runs fetch → `git worktree add` (`--no-track -b <branch> <path> <remote>/<base>`, or the existing
branch) → the space (its id from the create answer) → the setup command and the agent: the terminal runs `sh
<cache>/start/<id>-<random>.sh`, which runs the setup as `cockpit setup-run /bin/sh -c '<command>'` (cockpit's own work:
the hub refuses what only a person does to it and to what it starts), writes its exit status, then `exec`s the agent
with its prompt read from `<id>-<random>.prompt`, so nothing from a title or a prompt is typed into a shell. Both files
are new ones of the start's own (0600, never written through a link). A mode is one of the form's (`default`,
`acceptEdits`, `plan`, `bypassPermissions`) or `auto`, else the plan blocks; `bypassPermissions` and `auto` run tools
without asking and block until `confirm: true`. A ticket moves to In Progress (Linear token)
once setup succeeded, else its agent is asked to. A refused start is recorded as failed (`error.step: "plan"`).

Cancel and failures remove only what the start made: the space it opened, the worktree it added (forced only before
the space was handed over; after, a worktree with changes is kept and said so) and its new branch while it has no
commits of its own. A failed setup keeps its space and worktree until Retry types the line again or Cancel removes
them; a setup silent for an hour fails. Every surface asks before removing: Esc leaves a start going, or keeps a
failed one, which the inbox and Home list until it is retried, cancelled or dismissed. A driven start (`driven: true`,
Claude) runs setup in a login shell in the hub, its output in `<id>.log`, then `act session start` and `send`; Cancel
ends that setup's process group before it removes the worktree, and a cancelled run stays cancelled. The hub neither
exits idle nor hands over while it runs a start itself; a run it was running when it stopped anyway comes back failed
("the hub restarted during it") with Retry, which runs a driven setup again, while a terminal's setup is followed again.

On a remote desk the dialog says Start work runs on that machine's own cockpit and offers nothing: `act start …` is
not among the reads a remote desk sends, and the runner's hub, listening off loopback, refuses `fetch start_work`
too. An optional `label` in the request names the new space (`"review · api"`) in place of the source's.

View `start` (`{id}` for one run): `{run: {id, label, title, state: running|done|failed|cancelled, setup, steps: [{id,
text, state: todo|running|done|failed|skipped, note}], error: {step, text} | null, retry, cancel, request, made, at}}`,
without `id` `{runs: [...]}`, newest first; it is pushed as each step moves, and kept in `~/.cache/cockpit/start-runs.json`
across a hub restart (a run cut short reads as failed). The last repo, agent and mode are in `start.json`, the approved
setup commands in `start-trust.json`. Keys in the dialog: ↓ from a field reaches the options and fixes, in the order
they are drawn, ↑/↓ move between them, ⏎ applies one and Esc goes back to the source; `1`–`9` pick one once ↓ gave the
dialog the keyboard, never while a field types. None of these act while the plan is being checked, and a new plan keeps
a lit choice on its label (the source takes the keyboard back when it is gone). ⌃R ⌃T ⌃G ⌃O turn the repo, worktree,
agent and mode; ⏎ in a field starts, `r` retries, `c` cancels and Esc leaves. The TUI form numbers the same choices
the same way, its digits working away from a text field. Cockpit Chat shows the keys in tooltips.

## How fresh each source is (`hub.fresh`, `sources`, both halves)

The hub's feed records each source it reads in the document that source fills, as `hub.fresh.<source>`: `github`,
`tickets` and `platform` in `inbox`, `ci` in `ci`, `calendar` in `meetings`, `mail` in `mail`, `monitor` in `monitor`.
The `inbox`, `meetings`, `mail` and `monitor` views serve them as `sources`, in that order:

```jsonc
// Source: cockpit_core::fresh::Freshness's shape, with what a chip needs.
{"source": "tickets", "label": "Linear", "at": 1793611507779 /* last good read */, "next": 1793611567779, "every": 60000,
 "state": "failing" /* never | fresh | stale (two intervals, five minutes at least, past the next read) | failing, as of the build */,
 "error": {"kind": "auth" /* auth | rate | network | not_found | server | other */, "message": "HTTP 401: token refused", "since": 1793610007779} /* or null */,
 "login": "cockpit connector login Linear" /* how it signs in again, as typed, or what to do instead */,
 "via": null /* the server that failed, for a source read through two */,
 "elsewhere": 1793611500000 /* when a Claude session's plugin last brought this source in (the inbox's
                              modFetched.<source>, else a plugin-written document's at), or null */,
 "now": 1793611507900 /* the build's time: a remote desk says ages by the hub's clock */}
```

Words and state are made again at drawing time (`fresh.js` in the mod, `cockpit_core::header` in the TUI), so an age keeps
moving while nothing new arrives: "GitHub · 2 min ago", "Linear · sign in again", "CI · offline since 12:04", "Calendar
· stale, 2 h ago", "Mail · limited until 12:30". A failing source whose data a Claude session brought in since reads
quiet, its age that data's. A source the hub records says its errors in its chip, so the views leave its error lines
out; the record survives a hub restart and a whole write by another writer (`store put`, which keeps the hub's own
record, never one the writer carried), and leaves when the config stops reading the source, with its document's error
and its inbox error line. A remote desk shows the chips without their actions. Actions (`act("sources", action, [source])`): `sign_in` types the source's login (`gh auth
login`, `cockpit connector login <server>`, else `cockpit setup`) into a new tab of the actions space; `refresh` reads
every source again at once (a rate-limited one at its reset). Keys: `L` (shift+L) signs in again, `R` reads again,
listed in the TUI's `?` under Sources.

## Cockpit settings and getting started (`cockpit doctor --json`)

`cockpit doctor --json` runs the doctor's checks and prints them as data, and exits 0 whatever they found; it keeps the
report in `~/.cache/cockpit/doctor.json`, which `--cached` gives back while it is under ten minutes old, newer than
`config.json`, from the past and not marked `stale` by a fix. A `--json` run never builds the notifier: it says
`notifier not built` with the build as its fix.

```jsonc
{"version": 1, "at": 1793611507779, "cockpit": "0.40.3", "summary": {"pass": 21, "warn": 3, "fail": 1},
 "checks": [{"id": "tools.gh" /* <category>.<what>, stable */, "category": "tools", "status": "fail" /* pass | warn | fail */,
             "message": "gh not signed in", "fix": {"label": "gh auth login", "command": "gh auth login" /* or */, "act": {"app", "action", "args"}} /* or null */}],
 "getting_started": [{"id": "github", "label": "Sign in to GitHub for your pull requests", "done": false, "status": "fail", "check": "tools.gh", "fix": {…}}],
 "agents": [{"id": "codex", "name": "Codex", "installed": true, "default": false, "path": "…",
             "can": {"start", "resume", "fork", "headless", "edits", "model", "driven"} /* booleans; driven reads "in chat" */,
             "sessions": true, "usage": "its session files: tokens and context",
             "limits": "Codex's session log" /* where usage reads its plan limits (agent::reports), or null */, "tools": "mcp" /* plugin | mcp | null */, "connected": false}]}
```

The steps of getting started are checks: setup (`tools.config`), the default agent, its tools, GitHub (`tools.gh`, or
`sources.server.code` when a code server reads it), tickets (`sources.tickets`: done once they came in lately, read by
the hub or brought by a Claude session, not once a token is set), the Herdr app's layout and notifications; a step
whose check did not run is left out. Categories: `tools`, `install`,
`app`, `agents`, `claude`, `sessions`, `sources`, `connectors`, `rules`, `spaces`, `notifications`, `browser`.

- **Fix** (`act("doctor", "fix", [check id, report at, fix as shown])`), asked first in both layouts: the hub reads the
  kept report and types the check's `command` into a tab of the actions space (named `fix · <check id>`), or runs its
  `act`, then marks that report `stale` (under `doctor.lock`, never a newer one). It refuses a report another cockpit
  version wrote, one other than the `at` the person saw, a fix other than the one shown (its command, or `app action
  [args]`: "The checks changed: look again"), an action other than `sources refresh` and `sources sign_in`, one that did
  nothing, and a fix said in words only. A confirm asked from a report closes when another lands; after a command the
  sheet waits for Check again, after an action it checks again at once. A fix runs only the report's command or action, never one the caller
  sends; the hub does run a caller's argv elsewhere: the inbox's `run` action (a row's own command, such as a focus).
  In a sandbox (`COCKPIT_HC_BIN` set) a fix's command is shown, not run, since its terminals run on the real HOME.
  The question is answered by a click on its button only, in both layouts, and not by one landing in its first 400 ms.
- **Fix all** (`act("doctor", "fix_all", [report at, [[check id, fix as shown], …]])`): Health's top row shows "Fix all
  (N)" (`F`) when two or more checks that need a look have a fix the hub can run (`fixAll(report)`); one confirm, in
  each layout's own style, lists each fix on a line (a command in mono with "runs in a tab of the actions space", an
  action in `fixQuestion`'s words) and how many others need the person ("2 more need you: see below"). The hub checks
  the whole list against the kept report first, as `fix` does, and any check missing or changed refuses the batch with
  "The checks changed: look again", nothing run; then it runs each fix as `fix` would (a command in its own tab), notes
  a failure and goes on, and marks the report `stale` once. Its note is a line of counts ("Fixed 3 · 1 failed: <why>",
  "· N shown, not run" in a sandbox), then a line per fix, which the toast shows under it; nothing done at all is an
  error. After a batch with a command the sheet waits for Check again, else it checks again at once.
- **The sheet** (`doctor.js`, view `settings`; `diagnostics` names the same view): `openSettings({tab})` opens Cockpit
  settings as a window (860 px, narrower in a narrower window), from the palette ("Cockpit settings", and "Cockpit settings: Health", ": Agents" and ": Layout" for a tab),
  `cockpit.settings`, `cockpit.settings:<options|health|agents|layout>`, and `SettingsLink` at the foot of the sidebar and of
  Cockpit Chat's drawer, which opens it on Health while a check fails. `cockpit.diagnostics`, the name it had, still
  opens it, on Health (`ALIASES` in commands.js, so the keys sheet names a key bound to it as what it opens).
  - **Header**: the installed version, the published one and when it was last checked, from `cockpit.json` and
    `update.json` (`versions()` and `versionLine()` in update.js: "Cockpit 0.40.4 · up to date · checked 4 min ago",
    "· 0.41.0 is out", "· updates not checked yet", "· the check failed: <why>"; "your own build" on an install that
    builds itself). **Check for updates** (`U`) asks the hub to look now (`act("update", "check")`, below) and shows
    "checking for updates…" meanwhile; when a newer version is out, **Update to <version>** opens the update dialog as
    the person's click (`openUpdate({person: true})`), where Install is. Opening the sheet asks nobody.
  - **Options** (first, view `options` below, followed for as long as the sheet is open): every option section by
    section with its line, its default and its key, "new" on one an update brought until its tab shows it, and what
    sets it: a switch, a choice's values as pills, a short field or a number field with its unit. A switch or a pill
    saves at once through `act options set`; a field saves on `⏎` or its Save button, which shows once what is typed
    differs. What is saved shows at once and its toast takes it back (Undo); a save the hub refuses goes back to the
    value before and says why under the option ("Not saved: …", an invalid value in the option's own words, what was
    typed left to fix). An option config.json holds as something no field shows (`by_hand`, a model per agent) reads
    "set in config.json". Under the sections, **In config.json** lists what no switch or field sets (the view's
    `in_file`: each key with one line) and **Open config.json** (`O`) opens the file in the person's editor
    (`openInEditor`, the config's `editor`).
  - **Health**: what the checks found, Fix all (`F`, above) and Check again (`R`), the window's link to the hub when it is not live (what it
    is doing, or "The cockpit hub isn't running: this window cannot reach it" with why and Start it), getting started,
    then the checks that need a look, or every one (`A`, or the two pills).
  - **Agents**: every agent of the table, installed first, with what each can do: start, resume, fork, headless, and
    in chat (`can.driven`, its tooltip "Cockpit Chat runs it itself, without a terminal").
  - **Layout**: a card per cockpit layout, Cockpit and Cockpit Chat, side by side (stacked under 640 px): its picture
    in the window's theme (`app/mods/cockpit/layouts/<layout>-<dark|light>.png`, 800×512, the acme demo world; dark or
    light by the theme's background, `isDark`), a few lines on what it is for (`LAYOUT_CHOICES`), "In use" on the one
    shown (a copy counts as the layout it copies) and **Use this layout** on the other. Its click closes the sheet and
    runs `layout.switch` as the update dialog's Try does, with a toast saying how to come back; no key or command does it.
  - **Keys**: `1`–`4` tabs, `j k` pick an option or a check, `space`/`⏎` flip the switch picked or take a choice's
    next value, `← →` (`h l`) turn a choice, `⏎` puts the keyboard in the picked field (`esc` gives it back, what was
    typed dropped), `⏎` on a check asks about its fix (a click answers), `F` asks about Fix all, `R` checks again, `U` checks for updates, `O` opens config.json, `esc`
    closes. Any process can open the sheet (`cockpit.settings`), so the keys that set an option, turn a choice, reach
    a field, open the file or ask about a fix or Fix all act only from the person's own key, never held, once they moved in that
    tab (`j k`) or clicked one of its rows; showing a tab, or every check, asks for that again. Before that they say
    "Move to it first". A bare window command opens the sheet and changes nothing, and an injected key sets nothing
    and reaches no field. The options it shows count as seen once the person opened the sheet (`{person: true}`: the
    sidebar's click, a keyed command) or pressed a key of theirs or clicked in it.
  - **The hub down**: the sheet still opens. Options says so with why and Start it, and still opens config.json;
    Health runs its checks without the hub; the header's versions are read from the files.
- **Getting started in an empty state**: `new Started()` in `init`, `this.started.want(cx)` in `render` while the empty
  state shows (the doctor is asked only then), and `this.started.el({compact})` (null when every step is done), as
  Home's empty board does; a step's fix asks inline first and the list reads the checks again after it. `StartedList(report,
  {onFix})` draws a report already read; the TUI inbox draws `cockpit_core::started::lines` while it has no row, and
  asks the doctor again every ten minutes.
- **The sidebar's and the drawer's link** ("Cockpit settings") reads the failing count from
  `shared("cockpit.doctor.failing")`, which every report read sets; a remote desk shows no link, and its chips and
  `⇧L`/`⇧R` do nothing.

## View `options` and the update's steps (`update.js`, both halves)

The options a person sets are listed once, in `core/src/options.rs` (`ALL`), section by section (`SECTIONS`): key, section,
the version that brought it (`since`: `NEXT`, the one number to change at release, or none for an option cockpit always
read), title, one plain line, default and kind. A kind is `toggle`, `choice` (its values with their names; the
installed agents for `agents.default`), `text` (one line, one word, a time of day, a folder that exists or a branch
template; `blank` says what an empty one means) or `number` (a whole number between two bounds, with its unit).
`checked(setting, value)` gives the value as it is saved or why it is refused, in the option's own words ("Morning brief
time takes a time of day, such as 08:30", "GitHub takes 15 to 86400 s"); `IN_FILE` lists what config.json holds that no
switch or field sets. What was shown lives in `~/.cache/cockpit/options.json`: `{ran, seen: [keys], update: {from, to,
at, notes, shown}}`. `note_run` runs at the hub's start, before its first look at what is published, and with `cockpit
options`: a version later than `ran` (update.json's `current` before anything was noted) is an update, kept with
update.json's notes between the two.

```jsonc
// view options
{"next": "1.0.0", "layout": "cockpit-chat" /* the layout the update offers, or null */,
 "update": {"from": "0.40.3", "to": "1.0.0", "notes": [{"version", "date", "changes"}], "shown": false} /* or null */,
 "sections": ["Agents", "Queue and CI", "Chat", "Usage", "Start work", "Sources", "Refresh", "Notifications", "Briefs and recaps", "Apps"],
 "options": [{"key": "queue.drain", "section": "Queue and CI", "since": "1.0.0" /* or null */, "title", "description",
              "kind": "toggle" /* toggle | choice | text | number */, "default": true,
              "value": true /* config.json's when it is of the kind, else the default */,
              "by_hand": false /* config.json holds something there no field shows */,
              "choices": [{"value", "label"}] /* choice */, "blank": "the conversation's own model" /* text: what empty means, or null */,
              "min": 15, "max": 86400, "unit": "s" /* number */,
              "seen": false /* always true without `since` */, "new": true /* brought by the update */}],
 "in_file": [{"key": "usage.commands", "description"}], "path": "/Users/me/.config/cockpit/config.json"}
```

- **Acts**, the view's own, a person's only (`peer::person`, so an agent's tools are refused): `set [{key: value…}]`
  checks each value as its option takes it (a boolean, one of a choice's values, a field's text, a whole number within
  its bounds) and refuses the whole save, nothing written, when one is not or names no option; then it writes
  `~/.config/cockpit/config.json` in place (`core::jsonedit`, mode 0600, every other byte and every unknown key kept; a
  part of a key holding something other than an object is refused) and the resolved copy in the cache (a folder with
  its `~` spelled out), and marks them seen; `shown [{keys?, update?}]` marks options seen and the update shown. The
  update dialog asks about the switches an update brought; an option of another kind is set in Cockpit settings.
- **`act("update", "check")`**, anyone's (it reads what is published and installs nothing): the hub looks at the
  published version now and writes `update.json` (`{at, looked_at, current, latest, dev, notes, notified, error?:
  {message, at}}`: `at` stays the six-hourly look's, `looked_at` is this one's, and a failed look keeps the last version
  read and says why until one works). It answers "Up to date" or "0.41.0 is out" as its note, or why it failed as its
  error, and gives the same answer again for 45 s rather than asking GitHub. It announces nothing; the version counts
  as announced (`notified`) only when a person asked. `versions()` in update.js reads the file: `{current, latest, at
  (the later of the two looks), dev, newer, error}`.
- **The dialog** (`update.js`): after Install, or opened with `{after: true}`, it shows what changed (`news`, when there
  are notes), the options the update brought (`options`: a switch each; Save sends only the changed ones, Skip keeps
  them) and the layout to try (`layout`, when the window shows another: `layout.switch`, then a toast saying ⌘⇧L comes
  back). The dialog marks the update shown, and the options step its options seen, once the person opened it (the
  update tag's click, a keyed `cockpit.update`) or acted in it (a key of theirs, a click); one opened on its own or by
  a bare command marks nothing. Save, Skip and Try take a click or the person's own key (`⏎`, `space` flips the picked
  option, `j k` pick), never held, and `⏎` and `space` only once they moved in the dialog or clicked in it; an injected
  key only moves or closes, and `cockpit.update:<save|skip|try|next>` acts only with a key the window minted (`take`). An install this
  window started is kept in `shared("cockpit.update.flow")`, so the dialog the mod's reload makes anew follows it on.
- **At start** (`askAfterUpdate`, from `main.js`, once per load of the mod): when the update is not shown, says
  something (notes, an option it brought, or a layout to offer) and no update dialog beats, the dialog opens on it.
- **Elsewhere**: `cockpit update` at an interactive terminal ends with `cockpit options ask`: the notes, each new option
  asked in turn, and a default-No offer to try Cockpit Chat in Herdr desktop. Without terminal input it prints
  `cockpit options new`, including the command to try the layout; the desktop frontend keeps its own update dialog.
  Herdr's `whats-new` popup (`whats-new.sh`, after update.sh and at herdr's start, while
  `cockpit options pending`) runs `cockpit options ask`; `cockpit setup` and /cockpit:setup ask about the options not
  seen yet.

## Views `mail` and `mail-thread` (the mail space, both halves)

Built by `cockpit_mail::gui` from the hub's `mail` document (`{at, messages, notified, by?, error?}`, written by the hub
through its own mail server or by a Claude Code session through the mail connector). The TUI app (`cockpit mail`, run in
the `mail` space) draws the same model: the latest 15 unread messages, then the latest 15 read ones, each newest first.

```jsonc
{
  "waiting": false, "enabled": true, "updated": "14:05", "error": null, "sources": [Source],
  "counts": {"unread": 6, "read": 1, "total": 7},
  "groups": [{"key": "unread", "label": "Unread", "ids": ["t1"]}, {"key": "read", "label": "Read", "ids": ["t2"]}],
  "messages": [{"id": "t1", "from": "maya@example.com", "fromName": "Maya", "subject": "…", "snippet": "…", "url": "…",
                "date": "…", "age": "3h", "unread": true, "raw": {/* as the document holds it */},
                "acts": [{"id": "see" | "reply" | "archive" | "read" | "open", "label": "…", "key": "c", "main": false}]}]
}
```

`mail-thread {id}` is one message's thread for the reading panel: `{state: "unread" | "loading" | "ready" | "failed",
since?, error?, messages: [{from, fromName, date, when, html, text}]}`, its `html` sanitized (no scripts, styles, event
handlers, frames or forms; links and images kept) and drawn with `TextView.html`, `text` the fallback. It follows
`~/.cache/cockpit/mail/`, where threads are kept a week as `<id>.json`.

Actions (`act("mail", action, [message])`, the message as `raw` gives it): `mail_thread` reads the thread into the cache,
through the hub's mail server at once (the mapping's `get` verb, `thread`/`html`/`body` fields), else files a `mail-get`
request a Claude Code session carries out (`plugin/hooks/mail.ts`); `mail_reply` starts an agent in the actions space
that drafts a reply and sends nothing; `mail_archive`, `mail_read`, `mail_open`; `request_refresh`. A notification about a
new message leads to `mail:<id>`: the mail space, that message open.

## Views `routines` and `routines-when` (the routines space, both halves)

The hub keeps the `routines` document (`hub/src/routines.rs`); the view (`routines/src/gui.rs`) is what `routines.js`,
Home and Needs read. Times are epoch milliseconds; `when`, `ago` and `text` fields are already in words.

The TUI routine form cycles through the installed agents and the shell at its Runs field. Model discovery updates an
open form without discarding its typed prompt or selected model; longer model lists show one choice at a time.
Agent capability coverage and source references are listed in [the agent inventory](dev/agent-capabilities.md).

```jsonc
{
  "ready": true, "at": 1793611507779, "since": 1793600000000, "settling": false,   // settling: the machine just woke
  "agents": {"default": "claude", "installed": [{"id": "claude", "name": "Claude Code", "icon": "claude", "models": ["", "lightest", "opus", "sonnet", "haiku", "fable"]}]},
                                   // `models`: what a routine of it may run on, its default ("") first, then what only a
                                   // routine takes (Claude Code's `lightest`), then the models the agent offers everywhere
                                   // (`core::models::for_routine`: its own listing, asked at most once a day, half an hour
                                   // after one that failed, and kept in ~/.cache/cockpit/models.json, else its table's; a
                                   // failed listing keeps the last good one; none is asked in the demo world or with
                                   // COCKPIT_HUB_MODELS=off); each Routine has its agent's
  "tiles": {"routines": 7, "active": 6, "paused": 1, "running": 0, "waiting": 0, "failing": 2, "look": 2, "unread": 4,
            "weekRuns": 10, "weekOk": 8, "weekFailed": 1, "weekMissed": 1, "weekSkipped": 1, "weekCost": 0.66,
            "spent": "~$0.66", "budget": 0.5, "overBudget": true, "worktrees": 1,
            "next": {"slug": "notes-pull", "name": "Pull my notes", "at": 1793612000000, "text": "in 18 min"}},
  "days": [{"label": "Wed", "date": "11 Nov", "today": true, "ok": 3, "failed": 1, "missed": 0, "skipped": 1, "cost": 0.18}],
  "look": [Look],                  // what needs a look, newest first (also Needs and Home)
  "inbox": [Run],                  // runs not read yet, newest first
  "routines": [Routine],           // running and failing first
  "templates": [Template],
  "triggers": [{"word": "ci-failed", "what": "CI failing"}]
}
```

- `Look` = `{key, slug, id, name, word, reason, summary, at, when, ago}`: a routine whose latest run (skips aside)
  `failed` or was `missed` and is not read, or one whose file is `broken` (`id` null, `key` its slug). Open it with
  `hub.go({app: "routines", key})`; `act("routines", "read", [slug, id])` takes it off.
- `Routine` = `{key, slug, name, state, when, whenText, trigger, error, problems, run, agent, agentName, agentSet, model,
  mode, tools, cwd, budget, spentWeek, spent, lastCost, over, paused, prompt, path, notify, worktree, catchUp, precheck,
  tz, zone, settling, next, nextText, nextRuns, settings, running: {id, started, for, why, stopping, checking} | null,
  queued, history, runs: [Run], unread, acts}`. `state`: `running | waiting | broken | over | paused | failed | stopped
  | missed | skipped | ok | never`; `error` names a broken `when` or `tz` (`tz: …`); `checking`: its precheck runs.
- `Run` = `{key, slug, id, name, started, ended, when, ago, took, cost, ok, word, why, summary, reason, kind, code,
  session, late, missed, skipped, repeats, checked, timedOut, unread, worktree, output, acts}`. `word`: `ok | skipped |
  failed | missed | stopped`; a skipped run has `ok: true` and `skipped: "precheck"`, and `repeats` ("6 times since
  18:29") when it stands for several skips in a row. `kind` and `reason` say how it went wrong in one sentence:
  `no_agent no_headless no_folder worktree unstarted precheck_slow precheck_broken skipped logged_out timed_out stopped
  budget turn_limit missed no_tools exit agent_error no_status`. `checked`: its precheck passed. `output` is the
  `<id>.txt` path (what the precheck said, for a skip).
- `Template` = `{id, name, about, when, shell, model, tools, mode, precheck, worktree, notify, body}`, as the default
  agent takes it (a model only its own, tools and a mode only for Claude Code): fills the form, nothing is saved until
  `add`. The Herdr command `cockpit.routines:template:<id>` (`blank` for none) picks one in an open new-routine form.
- `acts` on routines and runs: `[{id, label, key, main, confirm, question, action, args}]`, run as `act("routines",
  action, args)` with `armed: confirm` once confirmed.

`routines-when {when, tz}` → `{ok, when, kind: manual|every|schedule|event, describe, zone, next: [{at, text}]}` or
`{ok: false, error}`: what a schedule means read on `tz`'s clock (empty: the machine's), its next runs on yours.

Actions (`act("routines", action, args)`), positional args:

| Action | Args | Does |
| --- | --- | --- |
| `add` | `[name, when, body, cwd, kind ("shell" or an agent id, "" the default), model, worktree, notify, precheck, tz, tools, mode]` | A new routine file; every field after `body` optional |
| `update` | `[slug, …add's fields up to tz]` | Rewrites those fields; a missing `precheck` or `tz` keeps the file's |
| `run` | `[slug]` | Runs it now, a run of it waiting in the queue included; the note says it started, waits for a free slot or already runs; refused, with the sentence its run was recorded with, when its file cannot run, its week's budget is spent or it could not start |
| `stop` `edit` `output` `talk` `delete` (armed `delete-routine-<slug>`) `request_refresh` | as the `acts` give them | The TUI's actions |
| `pause` | `[slug, paused]` | Pauses, or resumes from now on |
| `model` | `[slug, model]` | Its model |
| `read` / `read_all` | `[slug, id?]` / `[]` | Runs marked read |
| `open_worktree` / `cleanup` | `[slug, id]` / `[slug?, id?]` (armed `remove-run-<slug>-<id>`, else `cleanup-runs`) | A run's worktree in a tab / removed (never forced) when nothing keeps it by `cockpit_core::cleanup`'s rules: a given run's (refused with what keeps it), else each, the note naming what keeps the others |

Store events of the `routines` document: `routine_ok`, `routine_failed`, `routine_missed`, `routine_skipped` (once per
stretch of skips: it keeps the first skip's id).

## Views `show` and `review` (the right panel and the show tool's panes)

- **`show`**, in two places. In the right panel (`{app: "show"}`) it follows the session in view (`followed`: the chat's
  subject in Cockpit Chat, else the agent in view, else the focused pane's terminal, so a plain shell lists its own): its
  views as tabs, newest first, and none when it showed none or nothing is in view. As a pane beside an agent (`{app: "show", name}`) it shows that
  view. Hub views: `show {name}` or `show {terminal, pick}` (an empty terminal lists nothing; an omitted terminal lists recent views), pushed on change from `~/.cache/cockpit/show` and the CI caches,
  and `show-block {name, n, since}` asked on the block's own clock for what runs something (a repo diff every 3 s, a log
  or a command's rows every `every` s; `{same: true}` when a diff's hash is `since`). Actions `show answer [name, n, type,
  value, label]` (type `choice`, `confirm`, `comment`, `blueprint` (a change request about a box), or `review` for
  several diff comments at once: `value.comments`, `label` = "1) a.rs:12: text 2) …"). Pictures are copied into
  `<app home>/data/cockpit/show/` (the app draws images only from its folder). Commands: `cockpit.show:go:<name>[#block]`,
  `cockpit.show:hide:<name>` (the pane closes itself), `show:copy:<n>`, `show:first:<n>`, `show:page:`.
- **Drawing** (`plot.js`): a chart block's model is `{kind: line|area|bar|donut, labels, series, stacked, percent,
  horizontal, show_values}`; `Chart` picks the plot (axes, stacks, areas), `Rows` (horizontal bars with their values
  written) or `Donut` (six slices, then "Other"). A `stats` block draws its items as `StatTile`s with `Figures`.
- **The show tool in app mode** (the plugin's side): split the agent's pane with `pane.split {direction, pane, mod:
  "cockpit", props: {app: "show", name}}`, keep `{pane, terminal}` in `<name>.app` and write `terminal:
  $HC_TERMINAL_ID` into the spec. A session cockpit drives has no terminal: it runs with `COCKPIT_SESSION_KEY` (its
  key, `claude:…`), which the plugin and `cockpit mcp` write as the spec's `terminal` instead, so the chat's side panel
  and its row's `documents` list its views as a terminal's. The view is open while the hub's `shown show` lists its name (see Claude sessions);
  the footer chip closes it with `pane.close {pane}`. Answers are the same `<name>.<n>.answer` files.
- **Blueprint** (a `{type: "blueprint", text}` block): the session's living plan, a Mermaid flowchart whose boxes are
  the steps, `class A,B done` / `class C active` (or `A:::done`) marking progress and a `---\ntitle: …\n---` header
  its title. A view holding one is named `<pane>-<first 8 of the session id>` (one per session) and its text lives in
  `~/.cache/cockpit/show/<name>.mmd`: the show call removes that file so the call's text shows, then the agent keeps
  the plan current by writing the whole flowchart there (the plugin allows Write/Edit on those files without a
  prompt). The hub watches the folder, so each save rebuilds the view: its model carries `text` (the file's, else the
  call's), `file` and `updated` (the file's mtime, ms). The app draws a header (title, "updated 2m ago", done count,
  Fit / − / zoom% / +), a done / in progress / to do legend with a progress bar and the diagram on a dot grid; a click
  on a box (or the palette's "Ask for a change", or `cockpit.show:ask:<block>:<box id>`) opens a composer, sent as
  `show answer [name, n, "blueprint", {node, label, text}, label]`, which reaches the session as "In the blueprint,
  about «label» (box id): text". The herdr TUI pane draws the file's flowchart like a mermaid block, without clicks.
- **`review`** (`{app: "review"}`, or `{repo, terminal, scope}`): the followed agent's repo (the focused terminal's, or
  the one a view hands over with `followInReview(handle, cwd)`, kept in shared `review.subject`), scope Uncommitted |
  Branch (against `origin/HEAD`, main or master, from the merge base) | PR (`gh pr view` + `gh pr diff`, kept a minute).
  Hub view `review {repo, scope, since, load}` (polled every 3 s while shown) gives the files, `about` (branch, base,
  commits, PR) and `walkthrough`; a file also carries `old_path` (a rename), `binary` with `size` (bytes, from the working
  tree), and `large: true` with `total` when it has more than 1500 lines and `load` does not name it (its lines then
  come empty: "Large diff · N lines · Load it"). `review {prompt: {scope, comments}}` gives `{prompt}`, the review as an
  agent reads it. Actions `review send [handle, {scope, repo, comments, to}]` (one numbered message through `deliver`,
  to a terminal or a driven session, refused while it waits on a question; the note names `to`), `review walkthrough
  [repo, scope, model]`, `review stop_walkthrough [repo, scope]`. The footer's To button (⇧T) picks who gets it among the
  agents view's rows working in the repo (driven ones included) or a new agent, which `start work` opens in the repo's
  checkout (`{source: "prompt", text: <prompt>, repo, worktree: "here", dirty: true}`); the choice is kept per repo in
  shared `review.to`. Commands `cockpit.review:open:<path>` reviews that folder; `review:scope:<s>`,
  `review:mode:diff|walk`, `review:submit`, `review:to` (the menu), `review:to:<handle|new>`, `review:follow:<handle>`
  (empty: the focus again).
  The herdr TUI's review space (`cockpit review`, `prefix+shift+c`) reads the same scopes with `review::read` and
  sends through the same actions, to the agent it follows or the one `T` picks for the worktree (a driven session, or a
  new agent through `start work`).
- **Walkthrough**: one detached `claude -p` (no tools, MCP servers, settings or skills, JSON schema) over a digest of the
  hunks (`h1`…), in `~/.cache/cockpit/walkthrough/{jobs,entries,pointers}`, keyed by the digest and the model. Hunk ids are
  the file and a hash of the hunk's lines, so a step whose code moved shows Outdated and new hunks land in Not covered.
  Only the hub's review view takes a finished job in; the TUI reads with `walkthrough::peek` and asks that view when one waits.
- `diff.js` (`DiffView`) is the shared diff: folding files, unified or side by side, a cursor, comments as rows under
  their line, the composer floating over the list (docked under it when narrow). `diagram.js` lays out and draws
  flowcharts and sequence charts natively; `plot.js` draws the charts.
- A `table` block sorts on a header click or `1-9`. Dragging a header's right edge gives every column a width of its own,
  and a table wider than its card then scrolls sideways; a double-click on an edge fits the columns to the card again.

## Design kit (`ui.js`)

shadcn/ui's components and proportions on Herdr's `theme()` (light and dark follow the person's theme). Build every
screen from these; add what is missing to the kit, not inline. `sh scripts/sandbox.sh kit [charts]` shows them all.

| Area | API |
| --- | --- |
| Tokens | `tokens()` (background, foreground, card, popover, muted, mutedFg, dim, accent, selected, border, input, ring, primary, primaryFg, destructive, success, warning, info, working, blocked, done, idle), `alpha(color, a)`, `RADIUS`, `TEXT`, `CAPTION` (10px, for axes and corner counts) |
| Primitives | `Icon(name)` (lucide, `icons/`), `AgentLogo(x, {size, color, tooltip, id})` (an agent's logo: Herdr's own icon from its home's `icons/`, LobeHub's, MIT, for a row's `icon`, Herdr's program or an icon name, a picture in its colours or a glyph tinted like the text from Herdr's program table, with `CHAT_LOGO_STYLE` in `ui.js` (`"brand"`, as shipped, keeps brand colours; `"tinted"` draws Chat's logos tinted); `iconOf(x)`, `providersOf(rows)`), `Spinner`, `Separator`, `Kbd` (writes "ctrl p" as ⌃P: `keyLabel(key)`; nothing in Cockpit Chat, where keys live in tooltips), `ChoiceKey(n)` (a choice's number: a Kbd in Cockpit, quiet text in Cockpit Chat), `CornerCount(n, {tone})` (a count on an icon's corner), `Badge(text, default/secondary/outline/destructive/success/warning/info, {dot, icon})`, `Button(id, {label, icon, variant, size, kbd, tooltip, onClick})` (`kbd` joins the tooltip in Cockpit Chat), `Checkbox`, `Skeleton`, `Avatar(name)`, `Progress(v)` |
| Layout | `PageHeader({icon, title, description, aside, actions, sources, view, narrow})` (a view model's `sources` as chips beside the title, a click runs their fix, `view.toasts` says how it went; `narrow` keeps the loud ones), `SourceChips(sources, {view, narrow})`, `sourceKey(e, sources, cx, view, {refresh})` (shift+L, and shift+R for a view with no refresh of its own), `Card({title, description, action, children, footer})`, `Section(id, {title, count, open, onToggle})` (collapsible), `SidePanel(id, {title, onClose, children})` (details beside a list), `Tabs(id, {items, value, onChange})`, `KeyHints([[key, what]])` (nothing in Cockpit Chat), `Empty`, `ErrorState`, `SectionHeader` |
| Lists | `ListRow(id, {selected, marked, flash, leading, title, meta, actions, hover, grip, dropAbove, dimmed, onClick, onDoubleClick, onRightClick})`, `Scroller` (keyboard-followed list), `Table(id, {columns, rows, sort, onSort})` (sortable) |
| Inputs | `SearchField(state, {active, kbd})`, `TextField(state)` (an `InputState` made in `init`), `Command(id, {state, groups, selected})` |
| Feedback | `Toaster` (`push(cx, title, {kind, description, action: {label, onClick}})`, Sonner-style, with Undo), `Flashes` (rows that changed), `AlertDialog` |
| Pointer | `Hover` (actions on hover), `HoverCard(id, trigger, content)`, `openMenu(items, {width, at, byKey})` (a menu at the pointer, or at `at` = {x, y} under its control, kept inside the window; with the pointer outside the window it opens at the top middle; `byKey`, a key opened it, picks its first item; items run `cockpit.<command>`, an `info` item only says something), `Reorder` (drag a grip to reorder: `grip(key, i)`, `track(listEl, rowHeight, count)`, `onDrop(key, to, cx)`) |
| Charts | `Sparkline(values, {w, h, area})`, `LineChart(id, {series, xLabels})`, `BarChart(id, {bars})`, `Ring(v, {label})`, `StatTile(id, {label, value, delta, deltaGood, spark})` — native paths (`window.paint_path` with `%` coordinates; strokes × zoom) |
| Dashboards | `Meter(parts, {total, segW})` (segmented LED meter), `Strip(values, {levels, h})` (history strip, a column per sample colored by level), `SegBar(id, parts)` (a bar split in proportion), `Pills(id, options, value, onPick)` (one choice among a few), `Toggle(id, on, onToggle)` (switch), `Tiles(tiles, {narrow})` (a row of tiles, rows of two when narrow) |
| Boards | `Mono(text)`, `dur(ms)` (`<1m`/`12m`/`3h`/`2d`), `Chip(text, {color, mono, icon, logo, onClick})` (a fact tinted with its state; `logo` an agent's), `CountPill(n)`, `StageHeading(id, label, {color, count, pulse, spin, action})`, `Overline(text)` (the heading inside a card or side panel), `MiniBars(values)`, `Metric(id, {label, value, tone, tint, hint, bars, spin})`, `Callout(id, {tone, label, icon, title, meta, body, actions, stack})`, `ToneButton(id, {label, tone, kbd})` |

Agent logos use Herdr's program-table `tint` metadata, cached once per window, and keep their brand colours in both layouts. The single `CHAT_LOGO_STYLE` switch in `ui.js` (`"brand"`, or `"tinted"` for the LobeHub MIT monochrome marks in `app/mods/cockpit/icons/agents/`) sets Cockpit Chat's.

The GUI is the floor-plus: visible buttons with key hints instead of hidden keys, hover actions, right-click menus,
drag to reorder, double-click to edit, Undo on toasts instead of confirms for reversible actions, details beside lists
instead of popups, sortable tables, inline sparklines and progress. No TUI-only workarounds (modal key modes, cramped
abbreviations).

## Testing

```sh
COCKPIT_SANDBOX=$TMPDIR/cockpit-gui-inbox sh scripts/sandbox.sh start   # one sandbox per person or agent working at once
sh scripts/sandbox.sh show todo; sh scripts/sandbox.sh key j; sh scripts/sandbox.sh shot /tmp/x.png; sh scripts/sandbox.sh check
sh scripts/sandbox.sh notify "🔴 CI failed: api main" "lint" "space:inbox"; sh scripts/sandbox.sh light; sh scripts/sandbox.sh stop
```

The sandbox is a separate Herdr window (its own `HERDR_APP_HOME`) on a fake HOME with the acme demo world, its own hub,
and the mod copied again on every save in `app/`. It shares herdr-core with the person's window: the spaces and
terminals there are theirs, so never create, close or type into them. Herdr loads no mod through a symlink (its icons
would not show), hence the copying. `herdr-app check` must pass and a screenshot must look right in both themes before a
view is done; `cargo test` for the crates touched.

- Keep `COCKPIT_SANDBOX` short: macOS caps a socket's path at 104 bytes, and a longer one gives no `herdr-app.sock`.
- A window behind others does not repaint, so `herdr-app shot` returns its last frame: bring it to the front first.
- `hc` runs on the real HOME (`$box/hc.sh`, through `COCKPIT_HC_BIN`): under the fake one it would start a herdr-core
  of its own.
- After many reloads the mod can stop reloading ("out of memory" in `sandbox.sh status`): quit the window and start
  again.

`sh docs/screenshots/app_shots.sh [view ...]` shoots each view in the sandbox window, dark and light, at 1280 and 760
wide, into `docs/app/` (`OUT`, `WIDTHS`, `THEMES` change that). It brings the window to the front for each shot.

## Credits

- The question parser and its key sequences (`herdr/tui/agents/src/prompt.rs`) are ported from
  [herdr-relevo](https://github.com/johnny4young/herdr-relevo)'s `lib/prompt.js` (Apache-2.0).
- The Chromium side browser's element picker, its notes' Markdown, its cookie import, its viewport sizes and its rules
  for permissions and sleeping tabs are ported from [Orca](https://github.com/stablyai/orca) (MIT); see
  `THIRD_PARTY_NOTICES.md`. Its per-worktree tabs, its profiles and its design mode follow Orca's browser.
- The recap, the stages by whose move it is and the recap popup's layout follow
  [herdr-recap](https://github.com/mrolafsson/herdr-recap) (MIT).
- The monitor's segmented meters and history strips follow [herdr-vitals](https://github.com/mrolafsson/herdr-vitals)
  (MIT); the routines' run inbox, catch-up and missed runs, worktree per run, preset pills and next-runs preview follow
  [herdr-automations](https://github.com/DnzzL/herdr-automations) (MIT).
- Home is after [jkuepker's Board](https://github.com/jkuepker/herdr-layouts) (MIT): today's tiles, the NEXT MOVE
  banner, the four columns and Focus. Resume after a restart follows
  [herdr-claude-resume](https://github.com/jordanlyall/herdr-claude-resume) and
  [herdr-revive](https://github.com/dmazlum/herdr-revive) (MIT); the per-agent prompt queue follows
  [herdr-todo](https://github.com/karanpatel1993/herdr-todo) (MIT); the bell is an idea from
  [herdr-whitby](https://github.com/shellscape/herdr-whitby) (MPL-2.0, no code taken).
- The inbox's merge readiness line and merge menu (`hub/src/act/land.rs`) are ported from
  [herdr-github](https://github.com/mrolafsson/herdr-github)'s `detail.go` (MIT, Copyright (c) 2026 Hjortur Stefan
  Olafsson); starting a ticket or PR in a worktree with its checks first follows it and
  [herdr-linear](https://github.com/mrolafsson/herdr-linear) (MIT); PR alerts by diff, the agent-time ledger read on
  ticket rows and the merged-worktree cleanup follow herdr-relevo (Apache-2.0, ideas); hover cards fetched on hover
  follow [herdr-gh-board](https://github.com/meviusisback/herdr-gh-board) (MIT).
- The code view (`app/mods/cockpit/code.js`) is ported from [yama-lei/herdr-editor-layout](https://github.com/yama-lei/herdr-editor-layout)'s
  editor pane (Apache-2.0).
- The show view's Mermaid flowchart parser and layered layout (`app/mods/cockpit/diagram.js`) are ported from
  [herdr-blueprint-app](https://github.com/chappse6/herdr-blueprint-app)'s `mermaid.js` and `layout.js` (MIT, Copyright
  (c) 2026 chappse6), its node drawing and done/active marks after its `main.js`; the living blueprint (a plan the
  agent keeps current in a `.mmd` file, a click on a box asking it for a change, Fit / − / % / +, the dot grid) is its
  idea. The review's walkthrough (prompt, JSON
  schema, the checks on the answer, hunk ids that survive line shifts, Outdated and Not covered) follows
  [herdr-walkthrough](https://github.com/dmazlum/herdr-walkthrough) (MIT, Copyright (c) 2026 Devrim Mazlum); comments as
  rows under their line with a floating composer and "Submit review · N" are ideas from herdr-reviewer (no license, not
  copied).
