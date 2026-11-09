// The agent tools' names, descriptions and input schemas, and the drawing guidance ({show} stands for the show tool),
// shared with `cockpit mcp`, which parses the object below as JSON: keep it strict JSON, double quotes and no trailing commas.
export const MCP = {
  "tools": {
    "show": {
      "name": "show",
      "description": [
        "Draws charts, figures, tables, diagrams, pictures, diffs, logs or a question in a pane beside this session, natively ",
        "(in the Herdr app or herdr). Reach for it on your own, unasked, whenever an answer holds a trend, a breakdown, a ",
        "distribution, a comparison, a share, a ranking, a funnel, a process, an architecture or an exchange: draw it here ",
        "instead of a markdown table or ASCII art, and keep the text answer to what it means. It replaces hand-written HTML ",
        "pages and claude.ai artifacts: use those only for what no block can draw, or when the person asks for a page or a ",
        "link to share. Blocks: {type:\"stats\", items:[{label, value, note?, delta?, good?, spark?:[numbers]}]} for headline ",
        "figures as tiles (good: whether the delta is good news); {type:\"chart\", kind:\"line\"|\"area\"|\"bar\"|\"donut\", ",
        "columns, rows, x?, y?:[columns], stacked?, percent?, horizontal?, show_values?, unit?} (rows as arrays; line or area over ",
        "time, bar for categories, horizontal:true for rankings and long names, stacked:true or percent:true (each row to ",
        "100%) for parts of a whole per category, donut for one whole split in a few parts (first y column; past six ",
        "slices fold into Other), show_values:true to write the values on the marks, unit:\"ms\"|\"s\"|\"bytes\"|<your own> so a ",
        "measured quantity keeps its unit rather than counting in thousands: the whole chart reads in the one its largest ",
        "value picks, so a ms series peaking at 96000 draws 96s, not 96k); ",
        "{type:\"table\", columns, rows}; a chart or table can instead take source:{metabase:{question:<id>}|{query:<MBQL>}} or ",
        "source:{datadog:{queries:[...], from?:\"now-1h\"}} or source:{metrics:{queries:[...], from?}} (queries in the language of ",
        "the metrics tool cockpit's setup connected) to refresh itself every `every` seconds, or command:<shell printing JSON> ",
        "with every; {type:\"flow\", nodes:[{id, label, kind?:\"box\"|\"round\"|\"decision\"}], edges:[{from, to, label?}], ",
        "direction?:\"down\"|\"right\"}; {type:\"sequence\", actors?:[...], messages:[{from, to, label, reply?} | {note, over}]}; ",
        "{type:\"mermaid\", text} for a flowchart or sequenceDiagram; {type:\"blueprint\", text}, the living plan of a task of ",
        "several steps: a Mermaid flowchart (`flowchart TD`, a `---\\ntitle: …\\n---` header for its title), one box per step, ",
        "`class A,B done` and `class C active` (or `A:::done`) marking progress; show it once in its own view (pane ",
        "\"blueprint\", one per session), then keep it current as the work moves by rewriting the whole flowchart into the .mmd ",
        "file the result names (with Write), which redraws the pane with no further show call; a box the person clicks ",
        "reaches you as their next message, a change request about that step; {type:\"image\", path|url, caption?} (an ",
        "animated GIF plays, so an E2E recording shows as one); {type:\"compare\", before, after} for two screenshots (a before/after of a front change); ",
        "{type:\"diff\", repo:<absolute path>, base?} following the working tree, or {type:\"diff\", patch}, whose lines the person can ",
        "comment on, each comment reaching you as their next message; {type:\"log\", command, every?, lines?} tailing a command; ",
        "{type:\"ci\", repo, branch} for the CI run of a branch; {type:\"choice\", prompt?, options:[{label, value?, preview?}]} or ",
        "{type:\"confirm\", summary, consequence?, confirm_label?}, whose answer reaches you as the person's next message; ",
        "{type:\"markdown\", text}. Any block takes a title. The same pane name updates the same pane. placement:\"browser\" ",
        "draws the view as a web page in the browser beside the session instead, at full resolution: use it for tables too ",
        "wide for a pane, big diagrams, pictures and side-by-side diffs; it takes markdown, stats, table, chart, flow, ",
        "sequence, mermaid, image, compare and diff blocks, with rows given in the call."
      ],
      "inputSchema": {
        "type": "object",
        "properties": {
          "pane": {
            "type": "string",
            "description": "A short name such as quota-chart; the same name updates the same pane"
          },
          "title": {
            "type": "string"
          },
          "url": {
            "type": "string",
            "description": "Optional: what `o` in the pane opens in the browser"
          },
          "placement": {
            "type": "string",
            "enum": [
              "right",
              "down",
              "zoomed",
              "browser"
            ],
            "description": "Where a new pane opens: right of this session (default), below it, zoomed, or as a page in the browser beside it"
          },
          "blocks": {
            "type": "array",
            "items": {
              "type": "object",
              "properties": {
                "type": {
                  "type": "string",
                  "enum": [
                    "markdown",
                    "stats",
                    "table",
                    "chart",
                    "flow",
                    "sequence",
                    "mermaid",
                    "blueprint",
                    "image",
                    "compare",
                    "diff",
                    "log",
                    "ci",
                    "choice",
                    "confirm"
                  ]
                }
              },
              "required": [
                "type"
              ]
            }
          }
        },
        "required": [
          "pane",
          "title",
          "blocks"
        ]
      }
    },
    "cockpit_search": {
      "name": "cockpit_search",
      "description": [
        "Searches everything cockpit knows, as its palette does: the person's PRs and the reviews asked of them, their tickets, ",
        "runbooks and their steps, show views, to-dos, meetings, monitored apps and their releases, the week's events (a review ",
        "requested, CI turning red, a ticket moving, a release judged) and past agent sessions by what was said in them. Use it ",
        "to find a PR, ticket, runbook, meeting, release or past session the person names loosely (\"the quota PR\", \"yesterday's ",
        "session on dark mode\") before asking them for a link or an id. Returns JSON {entries: [{app, kind, title, help, target}]}, ",
        "best first; a PR's target key is its URL, a ticket's its id."
      ],
      "annotations": {
        "readOnlyHint": true
      },
      "inputSchema": {
        "type": "object",
        "properties": {
          "query": {
            "type": "string",
            "description": "Words to match, fuzzily: a ticket id, a repo, part of a title"
          }
        },
        "required": [
          "query"
        ]
      }
    },
    "cockpit_changes": {
      "name": "cockpit_changes",
      "description": [
        "What changed in the person's work, newest first, from the week of history cockpit keeps: reviews requested, PRs opened, ",
        "approved, merged or asked for changes, CI turning red or green, tickets moving status, releases judged or moving their ",
        "numbers. Use it for \"what changed since this morning\", \"what happened while I was away\", \"anything new on my PRs\". ",
        "Returns JSON {events: [{at, when, app, doc, key, title}]}."
      ],
      "annotations": {
        "readOnlyHint": true
      },
      "inputSchema": {
        "type": "object",
        "properties": {
          "since": {
            "type": "string",
            "description": "How far back: 30m, 2h, 1d (the default) up to 7d, or epoch milliseconds"
          },
          "app": {
            "type": "string",
            "enum": [
              "inbox",
              "monitor"
            ],
            "description": "Only the inbox (PRs, reviews, CI, tickets) or only the monitor (releases)"
          }
        }
      }
    },
    "cockpit_read": {
      "name": "cockpit_read",
      "description": [
        "Reads from the person's tools through the MCP servers cockpit itself is connected to, whatever they are (Linear, Jira, ",
        "GitLab, CircleCI, a calendar, a deploy platform, Grafana, Sentry, a mail server…), as cockpit's setup mapped them: ",
        "tickets (listMine, get, details, comments, listStates; values {ticket}), ci (listRuns, listWorkflows, listJobs, getJob, ",
        "listTests, logs; values {run}, {workflow}, {job}), calendar (listEvents, getEvent; {event}), platform (deployStatus, ",
        "deployHistory, listPreviews, previewStatus; {appId}, {login}, {preview}), observability (metric, monitors, issues; ",
        "{query}, {from}, {to}, {service}, {env}), mail (listInbox, get; {message}), code (listMine, listReviews, listMerged, ",
        "details). Read-only. Returns the tool's answer as JSON; a source cockpit does not reach itself says so."
      ],
      "annotations": {
        "readOnlyHint": true
      },
      "inputSchema": {
        "type": "object",
        "properties": {
          "source": {
            "type": "string",
            "enum": [
              "tickets",
              "ci",
              "calendar",
              "platform",
              "observability",
              "mail",
              "code"
            ]
          },
          "verb": {
            "type": "string",
            "description": "One of the source's read verbs listed above"
          },
          "values": {
            "type": "object",
            "description": "The verb's placeholders, e.g. {\"ticket\": \"ENG-412\"}"
          }
        },
        "required": [
          "source",
          "verb"
        ]
      }
    },
    "cockpit_health": {
      "name": "cockpit_health",
      "description": [
        "A service's health from the metrics and errors tool cockpit is connected to (Datadog, Grafana, Sentry or any other ",
        "it mapped): errors, hits and p95 latency over a window, its alerting monitors and its open errors of the day. Use it ",
        "to compare before and after a deploy. Read-only. Returns JSON {service, from, to, numbers, monitors, issues}."
      ],
      "annotations": {
        "readOnlyHint": true
      },
      "inputSchema": {
        "type": "object",
        "properties": {
          "service": {
            "type": "string",
            "description": "The service's name as the metrics tool knows it"
          },
          "from": {
            "type": "string",
            "description": "Where the window starts: now-1h (the default), now-30m, now-2d, or an ISO time"
          },
          "to": {
            "type": "string",
            "description": "Where it ends: now (the default) or an ISO time"
          }
        },
        "required": [
          "service"
        ]
      }
    },
    "report_step": {
      "name": "report_step",
      "description": [
        "Records how a runbook step handed to this session went, and updates that step's status cell in the runbook. ",
        "Call it once the step has run (done, failed or blocked), or with waiting when the step needs the person's go before it runs."
      ],
      "inputSchema": {
        "type": "object",
        "properties": {
          "handoff": {
            "type": "string",
            "description": "The handoff id the request named, e.g. eng-410-3"
          },
          "outcome": {
            "type": "string",
            "enum": [
              "done",
              "failed",
              "blocked",
              "waiting"
            ],
            "description": "done, failed, blocked, or waiting for the person's go"
          },
          "message": {
            "type": "string",
            "description": "One line on what happened or what it waits for"
          }
        },
        "required": [
          "handoff",
          "outcome",
          "message"
        ]
      }
    }
  },
  "drawing": [
    "# Drawing results",
    "Whenever an answer holds a trend, a breakdown, a distribution, a comparison, a share, a ranking, a process, an architecture or an exchange between parts, draw it with {show} in the pane beside the session, without being asked: figures as a stats block, numbers as charts, structure as flow, Mermaid or sequence blocks. Keep the written answer to what the picture means.",
    "Do not write an HTML page to open in a browser, nor publish a page or an artifact, to show results: those are the fallback for what {show} cannot draw (call it first with what it can), or for when the person asks for a page or a link to share."
  ]
} as const
