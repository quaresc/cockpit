# The `show` tool

An agent draws results in a pane beside the session with `show`: charts, tables, diagrams, diffs, logs, pictures
and questions, rendered natively by herdr and the Herdr app. It is the reason cockpit holds back hand-written HTML
pages and claude.ai artifacts — those are the fallback, for what no block can draw.

This is the reference. The tool's own description carries the same ground, and its errors name the exact rule a
call broke, so you can also learn it by calling it.

## A call

```json
{
  "pane": "quota-chart",
  "title": "Daily quotas",
  "placement": "right",
  "blocks": [{ "type": "chart", "kind": "line", "columns": ["day", "rejected"], "rows": [["Mon", 3], ["Tue", 11]] }]
}
```

- **`pane`** — a short name of lowercase letters, digits and dashes. The same name updates the same pane, so a
  view can be redrawn as the work moves.
- **`title`** — optional, above the blocks.
- **`placement`** — `right` (the default), `down`, `zoomed`, or `browser`. `browser` draws the view as a web page
  beside the session at full resolution: use it for tables too wide for a pane, big diagrams, pictures and
  side-by-side diffs. It takes only the blocks marked *page* below, with their data given in the call.
- **`blocks`** — one or more of the following. Every block also takes a `title`.

## The blocks

| Block | Fields | For |
| --- | --- | --- |
| `stats` *(page)* | `items: [{label, value, note?, delta?, good?, spark?: [numbers]}]`, 1–8 | Headline figures as tiles. `good` says whether the delta is good news. |
| `chart` *(page)* | `kind: line\|area\|bar\|donut`, `columns`, `rows`, `x?`, `y?: [columns]`, `stacked?`, `percent?`, `horizontal?`, `show_values?` | Line or area over time, bar for categories, donut for one whole in a few parts. `horizontal` for rankings and long names; `stacked`/`percent` for parts of a whole per category. |
| `table` *(page)* | `columns`, `rows` | Rows as arrays, up to 5000. |
| `flow` *(page)* | `nodes: [{id, label, kind?: box\|round\|decision}]`, `edges: [{from, to, label?}]`, `direction?: down\|right` | A process or an architecture, up to 60 nodes. |
| `sequence` *(page)* | `actors?`, `messages: [{from, to, label, reply?} \| {note, over}]` | An exchange between parts. |
| `mermaid` *(page)* | `text` | A `flowchart` or `sequenceDiagram`; other Mermaid types are not drawn. |
| `blueprint` | `text` | The living plan of a task of several steps. See below. |
| `image` *(page)* | `path` (absolute) or `url`, `caption?` | A picture; an animated GIF plays, so an E2E recording shows as one. |
| `compare` *(page)* | `before`, `after` | Two screenshots side by side, for a before/after of a front change. |
| `diff` *(page)* | `repo` (absolute) and `base?`, or `patch` | The working tree as it changes, or a patch. The person can comment on a line, and each comment reaches the agent as their next message. |
| `log` | `command`, `every?`, `lines?` | A command tailed in the pane. |
| `ci` | `repo`, `branch` | The CI run of a branch. |
| `choice` | `prompt?`, `options: [{label, value?, preview?}]` | A question; the answer reaches the agent as the person's next message. |
| `confirm` | `summary`, `consequence?`, `confirm_label?` | Same, for a yes or no. |
| `markdown` *(page)* | `text` | Prose. |

## Data that refreshes itself

A `chart` or a `table` can take its rows from a source instead of the call, and redraw every `every` seconds
(30 at the least):

```json
{ "type": "chart", "kind": "line", "every": 60,
  "source": { "datadog": { "queries": ["sum:trace.http.server.request.errors{service:api}.as_count()"], "from": "now-1h" } } }
```

- `source.metabase: {question: <id>}` or `{query: <MBQL>}`
- `source.datadog: {queries: [...], from?: "now-1h"}`
- `source.metrics: {queries: [...], from?}` — in the language of whichever metrics tool `cockpit setup` connected
- `command: <shell printing JSON>` with `every` (1 second at the least)

A pane that runs a command asks the person the first time.

## Blueprints

A `blueprint` is the plan of a task of several steps, kept current as the work moves: a Mermaid flowchart
(`flowchart TD`, with a `---\ntitle: …\n---` header for its title), one box per step, and `class A,B done` /
`class C active` (or `A:::done`) marking progress.

Show it once in its own view — pane `blueprint`, one per session — then keep it current by rewriting the whole
flowchart into the `.mmd` file the result names, with Write. That redraws the pane with no further `show` call.
A box the person clicks reaches the agent as their next message, a change request about that step.

## When a call is refused

Each error names one rule and what would be accepted, so the next call is usually right:

```
at /blocks/0: unknown block type "pie"; allowed: "markdown", "stats", "table", …
at /blocks/0/horizontal: only bars lie down; use kind "bar"
at /blocks/0/edges/0/to: "c" is not a node id; ids: "a", "b"
at /blocks/0/y: column "lead" holds values that are not numbers; cast them in the query
at /pane: a short name of lowercase letters, digits and dashes, such as quota-chart
```

Only the first broken rule is reported, so a call with several mistakes takes a few turns.

In Claude Code, the plugin also refuses a hand-made HTML page beside the session, or a claude.ai artifact, when `show`
could draw it. Asking for a page or a link in so many words lets it through, as does prefixing the command with
`COCKPIT_SHOW_FALLBACK="<what show cannot draw>"`.

## Where it draws

In herdr and the Herdr app the pane opens beside the session; `q` hides it and its footer chip brings it back.
In a session running in neither, the view is written to `~/.cache/cockpit/show/<pane>.json` and nothing opens —
the result says so.
