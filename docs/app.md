# cockpit in the Herdr app

The Herdr desktop app half of cockpit: a layout (`app/layouts/cockpit.json`) and one mod (`app/mods/cockpit/`), drawn
natively by Herdr from JavaScript, on the same hub as the herdr TUI half. `cockpit app install` copies them into the
Herdr home (`HERDR_APP_HOME`, else `~/.config/herdr-app`), writes `cockpit.json` there (where the binary and cache are),
merges `keys.json`, and switches the window to the cockpit layout the first time.

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

`subscribe` reconnects by itself and starts the hub when none runs (a failed `cockpit hub start` is tried again after
30 s, for every view at once); stop it in `close()`. `{focus, target}` arrives when the palette or a notification asks
the view to bring `target.key` into view (`cockpit focus`, `hub.go`). Without `cockpit.json` Home draws the install
panel and `AgentsModel` (agentsui.js) falls back to Herdr's `agents()`. Only the app's main view sets `focus`; count
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
updates over TCP. Subscriptions reconnect when the hub restarts or sends nothing for 65 seconds. Links and item
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
an agent of the runner (`runner/t7` reaches the runner as `t7`). Every other action and Archive stay unavailable in this
preview.

TCP has no TLS: use an encrypted private network such as Tailscale, and keep `hub.json` private. A hub listening
off loopback answers reads only, whoever holds the token; the mod keeps the same list so a remote desk says so
without asking. That holds for the runner's own window too, so leave `COCKPIT_HUB_LISTEN` unset on a machine whose
desktop writes.

Desktop checks: `node --experimental-vm-modules --test app/tests/*.test.mjs`.

## Window-wide plumbing (the sidebar copy owns it)

- Keys (`app/keys.json`, section `layout:cockpit`): `cmd-shift-p` → `cockpit.palette` (cockpit's palette, a superset of
  Herdr's), `cmd-alt-1..7` → `cockpit.go:<inbox|runbook|todo|meetings|monitor|routines|mail>`. A `cockpit.<command>`
  reaches every shown copy's `command()`; prefix your own with your view's name (`todo:delete:<key>`).
- `show(child)` from `sidebar.js` switches the `center` stack and gives that view the keyboard (`show:<app>`).
- Notifications (`notify.js`): everything goes through `cockpit_core::notify::post` (or `cockpit notify`). With the app
  frontend the hub hands `{title, body, action, row, buttons}` to the sidebar (`subscribe {notify: true}`), which posts
  Herdr's own toast while the window is in front, else the system notification. `row` is where the one click leads:
  the row the notification is about (`inbox:<PR url>`, `inbox:<ticket id>`, `runbook:<slug>`, `routine:<slug>`, ...),
  where its other buttons are; the toast runs `cockpit.target:<row>` → `cockpit focus <row>` → the hub's `go`, so the
  view gets `{focus, target: {app, key}}`. A system notification posted for the app prefixes its click and buttons
  with `herdr-app:`, so `cockpit focus` drives the app whatever environment the notifier runs it in.
- Palette (`palette.js`): a full-window frameless popup drawing Herdr's palette geometry (620 wide, 60 from the top,
  10 rows of 32). `cockpit.palette` again closes it; `cockpit.palette:type:<text>` types into it (tests). Over a Claude
  agent, a found item goes into it through the hub: `act palette insert|ask|fork [over, found]`, over = `{terminal,
  cwd, title, session?}`; forking the session itself is `act agents fork {terminal: "app:t3", place}`. Open in
  editor is `editor.js`'s `openInEditor(folder, machine)` (the config's `editor`, else VS Code; Remote-SSH for a
  remote space).
- `cockpit.key:<keystroke>` presses a key in the shown views (how the sandbox drives them).

## The shell: layout, Home, status bar, bell, side panel, queue

`app/layouts/cockpit.json`: sidebar · a top bar (`tabs` + `bell`) · the `center` stack, `home` first and shown at start
(the board, with a hidden `home-agent` column: `agent-head` over `{agent: "focused"}`), `work` (a `view` stack: `main` =
the tab, `agent` = the focused agent alone), the apps · the `status` bar; in `work`, the right `panel` (stack
`panel-body`: `show`, `review`, `runbook-panel` (`runbook {scope: "focused"}`), `files`, `changes`, then the `activity`
bar); floating on the root, the `queue` drawer (right edge, 44px peek, shown only while prompts are queued).

- **Home** (`home.js`, hub view `home` from `cli/src/home.rs`): a dashboard. A greeting with one line of counts, KPI
  tiles (agents needing you, reviews for you, your PRs, merged this week, agent spend), the NEXT MOVE callout (only an
  agent waiting on you: its question with its choices as buttons and a reply field), the agents by stage as wrapped
  cards (empty stages on one line), then your PRs no agent works on (to fix, to land). On the right: Up next (meetings),
  Apps (each pinned app in a few words, a click away) and Activity (the hub's events of the last days). "Needs you" means
  agents only. With no card picked, the keys act on the NEXT MOVE agent.
  Keys: `j k h l` move, `⏎` go, `1–9` answer, `t` reply, `f` the agent beside the board, `m` merge, `a` archive (tab
  settled, Undo on the toast), `o` PR, `r` recap, `s` snooze, `q` queue, `n` new agent; right-click for the rest.
- **Status bar** (`status.js`): Home's counts (`fleet.counts`, each a click) on the left; on the right the focused
  agent's branch, ↑↓ and files, its links (the agents view's `links`: PR, ticket, CI, deploy dashboard and run, then the
  pages its transcript cites, each named after its service, always in that order), queued prompts, agents to resume,
  the hub only while it is offline, and its `documents` at the right end. Its copy runs the shell's keys: `cmd-alt-0` →
  `cockpit.home` (Home ↔ work), `cmd-alt-a` `agent-view`, `cmd-alt-b` `panel`, `ctrl-cmd-1..5` `panel:<show|review|runbook|files|changes>`, `cmd-j`
  `next-waiting`, `cmd-shift-j`/`cmd-shift-u` `next:blocked`/`next:done`, `cmd-g` `pr`, `cmd-l` `ticket`, `cmd-alt-r`
  `resume`. `cmd-alt-q` → `cockpit.queue` (the queue dialog), `cmd-r` → `cockpit.recap` (view `agents`' popup).
- **Code** (`code.js`, `{app: "code", file, line?, terminal?, tab?}`): a file in a pane beside the agent, to read and edit:
  syntax highlighting, ⌘S saves (refused when the file changed on disk since it was opened), the agent's edits show
  as it makes them, and Markdown gets a live preview beside it (⌘⇧V). "Ask the agent" turns a note on the file, or on
  some lines (`12-18: …`), into a one-comment review. `openCode(file, {terminal, line})` keeps one per tab; `e` in
  Review opens the file under the cursor, and `cockpit.code:open:<path>` any file. Ported from yama-lei/herdr-editor-
  layout's editor (Apache-2.0); no LSP.
- **Reader** (`reader.js`): a window that reads a file or a text as rendered Markdown, selectable and copied in one click;
  `openReader({path | text, title, meta})`, and `Markdown(id, text)` for the same rendering inside a panel (a routine
  run's output uses both).
- **Bell** (`bell.js`): agents waiting, breathing; click = the longest waiting, hover = their questions, right-click =
  pick one. **Activity bar** (`activity.js`): the panel's children with badges from shared `badge.<name>` =
  `{count, tone}` (write it only when it changes); it adds its panels to `workspace.panels` so a space switch keeps them.
- **Prompt queue** (`queue.js`, hub doc `queue`, view and actions `queue` from `cli/src/queue.rs`): `add [handle, text]`,
  `edit [handle, id, text]`, `remove`, `move [handle, id, to]`, `clear`, `send [handle, id | "all"]` (all = one numbered
  prompt; `hc send` in the app, `herdr pane run` in the TUI; refused while the agent waits on a question). Handles are
  `app:t3` or herdr pane ids. Shared `cockpit.queue.for` picks the agent.
- **Resume after a restart** (`core/src/resume.rs`, both halves): every minute the hub notes which Claude session each
  terminal runs (`~/.claude/sessions/<pid>.json` for app terminals, herdr's `agent_session` for TUI panes) in
  `~/.cache/cockpit/resume/agents.json`. A session not running after a reboot or a herdr-core restart is lost: one
  notification (its click resumes them all), Home's banner, the status bar, `cockpit resume [look|all|<handle>|dismiss
  <handle>]` and `cockpit focus resume`. Resuming types `cd '<cwd>' && claude --resume <id>` into its terminal when it
  sits at a shell, opens a space in its folder when it is gone, and refuses when another program runs there.

## The hub on its own

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
- **Names** (`hub/src/names.rs`): every 20 s a tab still at its default name (its number, a shell's) takes its agent's
  task, the title Claude Code gives the session (else a few words from haiku with no tools, once per session), and a
  space at its folder's name takes its agents' ticket ("ENG-412 · Store and enforce") or repo. A name the person typed
  never changes; `names: false` turns it off.

## View `agents` (every agent terminal, both halves)

Built by `cockpit_agents::gui` in the hub, pushed again when an agent's status changes (the hub polls the frontends
every 2 s), when the `inbox`/`ci`/`agents` documents change, or after its own action. Subscribe with
`{name: "agents", args: {frontend: "app"}}` (the Herdr app's terminals; `"tui"` for herdr's panes, no args for both).

```jsonc
{
  "agents": [{
    "terminal": "app:t3",                    // the handle every action takes (a herdr TUI pane id in the TUI: "w9:p1")
    "address": "t3", "frontend": "app",      // the frontend's own id; "app" | "tui"
    "space": {"id": "sp…", "name": "api"}, "tab": {"id": "tb…", "name": "quotas"},
    "title": "Daily quotas: enforce limits", // the agent's task (facts.task), else its terminal title / tab name
    "program": "claude",
    "status": "blocked",                     // working | blocked | done | idle
    "since": 1793611400000,                  // ms, this status began (the hub's own clock: survives restarts)
    "waiting_since": 1793611400000,          // ms when blocked, else null
    "question": {                            // parsed from the screen when blocked, else null
      "text": "Which date should old invoices keep?",
      "kind": "single",                      // single | multi | text (no choices: answer with free text)
      "choices": [{"n": 1, "label": "Issue date", "detail": "Matches the PDFs", "checked": null, "free": false}],
      "cursor": 1,                           // n under the cursor, "submit", or null
      "submit": false,                       // multi: a Submit item follows the choices
      "preview": "",                         // the preview box Claude draws beside a choice, if any
      "permission": false                    // a tool permission prompt (Yes / Yes always / No)
    },
    "said": "Old invoices store two dates…", // its last message on screen, or ""
    "recap": {"text": "…", "at": 1793611000000, "fresh": true, "running": false, "error": null},  // or null
    "stage": "needs_you",                    // whose move: needs_you | done | fixing | idle | working | review | merged | snoozed
                                             // done is an attention queue: opening the agent drops it to what its PR says
    "repo": "api", "branch": "me/eng-412-daily-quotas", "cwd": "/…/api",
    "ticket": {"key": "ENG-412", "title": "…", "url": "…", "status": "In Progress"},          // or null
    "pr": {"number": 2198, "url": "…", "title": "…", "state": "open", "draft": false, "review": "APPROVED", "ci": "SUCCESS"},  // or null
    "ci": {"state": "fail", "label": "lint failing", "done": 9, "total": 9},                 // running | pass | fail, or null
    "changes": {"files": 3, "add": 214, "del": 38, "ahead": 2, "behind": 0},                 // or null
    "context": 41, "cost": 1.84,             // facts the plugin reports (percent, USD), or null
    "page": "https://…",                     // the page beside it, or null
    "time": {"agent_today": 3120, "ticket_today": 4800, "ticket_week": 18000},             // seconds, from the ledger
    "snoozed": {"until": 1793640000000, "label": "until 18:00"},                          // or null ("till done": until null)
    "seen": false,                           // looked at (`agents seen`) since this status began
    "unread": true,                          // needs_you or done, not seen, not snoozed: what the bell counts
    "session": "4f1c…",                      // its Claude session id, when known (the recap and fork use it)
    "load": {"cpu": 12, "mem": 340, "history": [3, 12, …]}  // its process tree: % of a core, MB, the last samples; or null
  }],
  "stages": [{"id": "needs_you", "label": "Needs you", "count": 2}, …],  // whose-move order, every stage
  "counts": {"needs_you": 2, "done": 1, "working": 3, "unread": 3, "total": 8, …},
  "next": "app:t3",                          // the longest-waiting unsnoozed agent that needs you (NEXT MOVE), or null
  "quota": [{"name": "5h", "percent": 34, "resets_at": "…", "elapsed": 0.42, "ahead": false}],  // pace tick at elapsed
  "tickets": [{"key": "ENG-412", "title": "…", "url": "…", "agents": ["app:t3"], "today": 4800, "week": 18000}],
  "week": {"days": ["2026-10-27", …], "rows": [{"key": "ENG-412", "secs": [0, 3600, …], "total": 18000}]}
}
```

Actions: `hub.act("agents", action, [{…}])`, one object argument.

| Action | Argument | Does |
| --- | --- | --- |
| `answer` | `{terminal, choice: n}` · `{terminal, choices: [n…]}` (multi, then Submit) · `{terminal, text, choice?}` | Reads the screen again, presses the keys (`hc keys` / `herdr pane send-keys`), pastes text |
| `recap` | `{terminal, force?}` | Claude's own `/recap` in an unsaved fork of the session (cached per transcript); fills the row's `recap`. Asked by itself on haiku a minute after a turn ends when the agent was not seen meanwhile (config `agents.recap`: `auto`, or `manual`) |
| `snooze` | `{terminal, until}` | `until`: ms, `"1h"`, `"18:00"`, `"tomorrow"`, `"done"` (till its status changes), or null to wake |
| `seen` | `{terminal}` | Looked at in its current status (`unread` goes) |
| `fork` | `{terminal, place}` | The session forked (`claude --resume <id> --fork-session`) in a new `tab`, or a split `right`/`down` |
| `focus` | `{terminal}` | Brings the agent's tab into view, and marks it seen |

Facts per agent come from the plugin: store document `agents`, patched by terminal handle,
`{"app:t3": {session, transcript, cwd, branch, repo, ticket, task, page, context, cost, account, at}}`; the hub fills
what is missing from git, the inbox and `ci`. Quota reads `~/.cache/cockpit/limits.json` (`{at, limits: [{name,
percent, resetsAt}]}`), which the plugin writes from Claude's rate limits.

Drawn by `sidebar.js` (Needs you band answered in place, agents by ticket / space / whose move, Herdr's spaces list,
quota bars) and `recap.js` (the cmd-r dialog, its Time tab the week's ledger), from `agentsui.js`: `AgentsModel`
(follows the view, runs its actions, menu commands `cockpit.agents|<what>|<terminal>|<arg>` run by the sidebar copy),
`QuestionCard`, `Chips`, `QuotaBars`, `StageMark`. In a shell or herdr's popup: `cockpit agents [pick|answer|recap|
snooze|seen|fork|week]`. `COCKPIT_AGENTS_DEMO=herdr/tui/agents/demo.json` (set before `sandbox.sh start`) replaces
the terminals with the acme demo's made-up agents, answered in memory.

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
  agent's tab, else splits the agent's pane right with `terminal-browser open <url>`.
- **Open in editor** (`editor.js`, `openInEditor(folder, machine)`) and **fork a session** (`agents fork`) are in the
  palette and on the pane header (`{app: "pane"}`, hover: editor, fork to a tab, split, zoom, close).

## View `inbox` (the inbox tree, both halves)

Built by `cockpit_inbox::gui` from the same rows the TUI inbox draws (`model::model`), pushed again when the `inbox`,
`ci` or `brief` documents change, a request's outcome lands, `muted.json` or the costs change, or after its own action.
Subscribe with `{name: "inbox", args: {group: "feature" | "status"}}` (feature by default).

```jsonc
{
  "at": 1793611507779, "updated": "14:05",     // when the cache was written; "" before any data
  "state": "ok",                                // ok | empty (no data yet) | stale (another cockpit version wrote it)
  "message": "",                                // what to say for empty/stale
  "errors": ["github: HTTP 502"],               // the feed's and CI's error lines
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
               "project": "Quotas", "prs": ["<url>"], "cost": 1.84, "secs": 4800 /* time on it this week, or null */}],
  "worktrees": [{"path": "/…/api-eng-412", "repo": "api", "branch": "me/eng-412-quotas", "merged": true,
                 "changes": 0, "pr": "<url>" /* or null */}],
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
| `review` `fix_ci` `rerun` `redeploy` `create_preview` `plan` `continue_ticket` `talk` `mark_done` `extend` `stop_preview` `close_space` `set_muted` `dismiss_brief` `request_refresh` `open_url` `details` | as the rows' `acts` give them | The TUI inbox's actions |
| `merge` | `[pr, method?]` (`squash` / `merge` / `rebase`) | `gh pr merge` pinned to the head commit; adds to the merge queue where the base has one |
| `automerge` | `[pr, method \| "off"]` | Auto-merge on (by method) or off |
| `ready` / `draft` | `[pr]` | Draft ↔ ready for review |
| `dequeue` | `[pr]` | Out of the merge queue |
| `start` | `[ticket \| pr]` (armed `start-<id or url>`) | Checks first (open, not someone else's, no space on it yet), then a worktree of the branch (`~/.herdr/worktrees/<repo>/<branch>`) with Claude in a new space; a ticket also goes In Progress, assigned to you |
| `set_state` | `[ticket, state name]` | The ticket's Linear state (by name, case-insensitive) |
| `assign` | `[ticket, "me" \| login]` | Assigns the ticket |
| `file_issue` | `[{team, title, description}]` | A new Linear issue, assigned to you; its URL is the note |
| `remove_worktree` | `[path]` (armed `remove-<path>`) | `git worktree remove` and `git branch -d` (never forced: refuses with changes), closing its space |
| `open_worktree` | `[path]` | The space open on it in view, else a new space there |
| `forget_result` | `[file]` | Drops a request's outcome once shown |

Without a Linear token (`tickets.token`), `set_state`, `assign`, `file_issue` and the ticket side of `start` hand the
change to a Claude session in the actions space. Details on demand (`hub.fetchDetails(kind, args)`): `pr_merge`
`{repo, number}` → `{ready: {state, text, tone}, choices: [{do, method, label}], why, card: {body, labels, additions,
deletions, files, reviewers, checks}}`; `ticket_card` `{id}` → `{description, labels, priority, assignee, states:
[{name, type}]}`; `start_plan` `{ticket | pr}` → `{ok, steps: [text], blockers: [text]}`.

## Views `mail` and `mail-thread` (the mail space, both halves)

Built by `cockpit_mail::gui` from the hub's `mail` document (`{at, messages, notified, by?, error?}`, written by the hub
through its own mail server or by a Claude Code session through the mail connector). The TUI app (`cockpit mail`, run in
the `mail` space) draws the same model: the latest 15 unread messages, then the latest 15 read ones, each newest first.

```jsonc
{
  "waiting": false, "enabled": true, "updated": "14:05", "error": null,
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

## Views `show` and `review` (the right panel and the show tool's panes)

- **`show`**, in two places. In the right panel (`{app: "show"}`) it follows the focused agent: its views as tabs, newest
  first (all recent ones when that terminal showed none). As a pane beside an agent (`{app: "show", name}`) it shows that
  view. Hub views: `show {name}` or `show {terminal, pick}` (pushed on change: `~/.cache/cockpit/show` and the CI caches),
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
  $HC_TERMINAL_ID` into the spec. The view is open while the hub's `shown show` lists its name (see Claude sessions);
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
- **`review`** (`{app: "review"}`, or `{repo, terminal, scope}`): the focused agent's repo, scope Uncommitted | Branch
  (against `origin/HEAD`, main or master, from the merge base) | PR (`gh pr view` + `gh pr diff`, kept a minute). Hub view
  `review {repo, scope, since}` (polled every 3 s while shown) gives the files, `about` (branch, base, commits, PR) and
  `walkthrough`. Actions `review send [terminal, {scope, repo, comments}]` (one numbered line typed into the agent with
  `hc send`), `review walkthrough [repo, scope, model]`, `review stop_walkthrough [repo, scope]`. Command
  `cockpit.review:open:<path>` reviews that folder; `review:scope:<s>`, `review:mode:diff|walk`, `review:submit`.
- **Walkthrough**: one detached `claude -p` (no tools, MCP servers, settings or skills, JSON schema) over a digest of the
  hunks (`h1`…), in `~/.cache/cockpit/walkthrough/{jobs,entries,pointers}`, keyed by the digest and the model. Hunk ids are
  the file and a hash of the hunk's lines, so a step whose code moved shows Outdated and new hunks land in Not covered.
- `diff.js` (`DiffView`) is the shared diff: folding files, unified or side by side, a cursor, comments as rows under
  their line, the composer floating over the list (docked under it when narrow). `diagram.js` lays out and draws
  flowcharts and sequence charts natively; `plot.js` draws the charts.

## Design kit (`ui.js`)

shadcn/ui's components and proportions on Herdr's `theme()` (light and dark follow the person's theme). Build every
screen from these; add what is missing to the kit, not inline. `sh scripts/sandbox.sh kit [charts]` shows them all.

| Area | API |
| --- | --- |
| Tokens | `tokens()` (background, foreground, card, popover, muted, mutedFg, dim, accent, selected, border, input, ring, primary, primaryFg, destructive, success, warning, info, working, blocked, done, idle), `alpha(color, a)`, `RADIUS`, `TEXT`, `CAPTION` (10px, for axes and corner counts) |
| Primitives | `Icon(name)` (lucide, `icons/`), `Spinner`, `Separator`, `Kbd` (writes "ctrl p" as ⌃P: `keyLabel(key)`), `CornerCount(n, {tone})` (a count on an icon's corner), `Badge(text, default/secondary/outline/destructive/success/warning/info, {dot, icon})`, `Button(id, {label, icon, variant, size, kbd, tooltip, onClick})`, `Checkbox`, `Skeleton`, `Avatar(name)`, `Progress(v)` |
| Layout | `PageHeader({icon, title, description, aside, actions})`, `Card({title, description, action, children, footer})`, `Section(id, {title, count, open, onToggle})` (collapsible), `SidePanel(id, {title, onClose, children})` (details beside a list), `Tabs(id, {items, value, onChange})`, `KeyHints([[key, what]])`, `Empty`, `ErrorState`, `SectionHeader` |
| Lists | `ListRow(id, {selected, marked, flash, leading, title, meta, actions, hover, grip, dropAbove, dimmed, onClick, onDoubleClick, onRightClick})`, `Scroller` (keyboard-followed list), `Table(id, {columns, rows, sort, onSort})` (sortable) |
| Inputs | `SearchField(state, {active, kbd})`, `TextField(state)` (an `InputState` made in `init`), `Command(id, {state, groups, selected})` |
| Feedback | `Toaster` (`push(cx, title, {kind, description, action: {label, onClick}})`, Sonner-style, with Undo), `Flashes` (rows that changed), `AlertDialog` |
| Pointer | `Hover` (actions on hover), `HoverCard(id, trigger, content)`, `openMenu(items)` (right-click menu at the pointer, items run `cockpit.<command>`), `Reorder` (drag a grip to reorder: `grip(key, i)`, `track(listEl, rowHeight, count)`, `onDrop(key, to, cx)`) |
| Charts | `Sparkline(values, {w, h, area})`, `LineChart(id, {series, xLabels})`, `BarChart(id, {bars})`, `Ring(v, {label})`, `StatTile(id, {label, value, delta, deltaGood, spark})` — native paths (`window.paint_path` with `%` coordinates; strokes × zoom) |
| Dashboards | `Meter(parts, {total, segW})` (segmented LED meter), `Strip(values, {levels, h})` (history strip, a column per sample colored by level), `SegBar(id, parts)` (a bar split in proportion), `Pills(id, options, value, onPick)` (one choice among a few), `Toggle(id, on, onToggle)` (switch), `Tiles(tiles, {narrow})` (a row of tiles, rows of two when narrow) |
| Boards | `Mono(text)`, `dur(ms)` (`<1m`/`12m`/`3h`/`2d`), `Chip(text, {color, mono, icon, onClick})` (a fact tinted with its state), `CountPill(n)`, `StageHeading(id, label, {color, count, pulse, spin, action})`, `Overline(text)` (the heading inside a card or side panel), `MiniBars(values)`, `Metric(id, {label, value, tone, tint, hint, bars, spin})`, `Callout(id, {tone, label, icon, title, meta, body, actions, stack})`, `ToneButton(id, {label, tone, kbd})` |

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
- The recap (Claude's `/recap` in an unsaved fork), the stages by whose move it is and the recap popup's layout follow
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
