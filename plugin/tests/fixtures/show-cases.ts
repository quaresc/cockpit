// The show tool's checks, run on the same views by the plugin's tests and by `cockpit mcp`'s (cli/src/mcp/show.rs), which
// parses the list below as JSON. `error` is the plugin's answer (null: fine), `mcp` the CLI's where it differs on purpose.
export const SHOW_CASES: { name: string; spec: Record<string, unknown>; error: string | null; mcp?: string | null }[] = [
 {
  "name": "a stats view is fine",
  "spec": {
   "pane": "quota",
   "title": "Quota",
   "blocks": [
    {
     "type": "stats",
     "items": [
      {
       "label": "Open PRs",
       "value": 4,
       "delta": "+1",
       "good": false,
       "spark": [
        2,
        3,
        4
       ]
      }
     ]
    }
   ]
  },
  "error": null
 },
 {
  "name": "a chart drawn from its rows is fine",
  "spec": {
   "pane": "quota",
   "title": "Quota",
   "blocks": [
    {
     "type": "chart",
     "kind": "bar",
     "columns": [
      "team",
      "prs"
     ],
     "rows": [
      [
       "api",
       3
      ],
      [
       "web",
       5
      ]
     ],
     "horizontal": true
    }
   ]
  },
  "error": null
 },
 {
  "name": "a pane name has no capitals or spaces",
  "spec": {
   "pane": "Quota Chart",
   "title": "Quota",
   "blocks": [
    {
     "type": "markdown",
     "text": "x"
    }
   ]
  },
  "error": "at /pane: a short name of lowercase letters, digits and dashes, such as quota-chart"
 },
 {
  "name": "a view has a title",
  "spec": {
   "pane": "quota",
   "blocks": [
    {
     "type": "markdown",
     "text": "x"
    }
   ]
  },
  "error": "at /title: a string"
 },
 {
  "name": "a url is http or https",
  "spec": {
   "pane": "quota",
   "title": "Quota",
   "url": "ftp://acme.example/x",
   "blocks": [
    {
     "type": "markdown",
     "text": "x"
    }
   ]
  },
  "error": "at /url: an http or https URL"
 },
 {
  "name": "a placement is one of four",
  "spec": {
   "pane": "quota",
   "title": "Quota",
   "placement": "left",
   "blocks": [
    {
     "type": "markdown",
     "text": "x"
    }
   ]
  },
  "error": "at /placement: one of \"right\", \"down\", \"zoomed\", \"browser\""
 },
 {
  "name": "a view holds blocks",
  "spec": {
   "pane": "quota",
   "title": "Quota",
   "blocks": []
  },
  "error": "at /blocks: a non-empty list of blocks"
 },
 {
  "name": "one blueprint per view",
  "spec": {
   "pane": "quota",
   "title": "Quota",
   "blocks": [
    {
     "type": "blueprint",
     "text": "flowchart TD\n  a --> b"
    },
    {
     "type": "blueprint",
     "text": "flowchart TD\n  c --> d"
    }
   ]
  },
  "error": "at /blocks: one blueprint per view"
 },
 {
  "name": "a block is an object",
  "spec": {
   "pane": "quota",
   "title": "Quota",
   "blocks": [
    "text"
   ]
  },
  "error": "at /blocks/0: a block is an object with a type"
 },
 {
  "name": "an unknown block type lists the known ones",
  "spec": {
   "pane": "quota",
   "title": "Quota",
   "blocks": [
    {
     "type": "pie"
    }
   ]
  },
  "error": "at /blocks/0: unknown block type \"pie\"; allowed: \"markdown\", \"stats\", \"table\", \"chart\", \"flow\", \"sequence\", \"mermaid\", \"blueprint\", \"image\", \"compare\", \"diff\", \"log\", \"ci\", \"choice\", \"confirm\""
 },
 {
  "name": "a block title is a string",
  "spec": {
   "pane": "quota",
   "title": "Quota",
   "blocks": [
    {
     "type": "markdown",
     "text": "x",
     "title": 3
    }
   ]
  },
  "error": "at /blocks/0/title: a title is a string"
 },
 {
  "name": "a blueprint is a flowchart",
  "spec": {
   "pane": "quota",
   "title": "Quota",
   "blocks": [
    {
     "type": "blueprint",
     "text": "sequenceDiagram\n  a->>b: hi"
    }
   ]
  },
  "error": "at /blocks/0/text: a blueprint is a Mermaid flowchart: flowchart TD, then its boxes and edges (a --- title header may come first)"
 },
 {
  "name": "a blueprint may open with a title header",
  "spec": {
   "pane": "quota",
   "title": "Quota",
   "blocks": [
    {
     "type": "blueprint",
     "text": "---\ntitle: ENG-412\n---\nflowchart TD\n  a --> b\n  class a done"
    }
   ]
  },
  "error": null
 },
 {
  "name": "markdown needs its text",
  "spec": {
   "pane": "quota",
   "title": "Quota",
   "blocks": [
    {
     "type": "markdown"
    }
   ]
  },
  "error": "at /blocks/0/text: markdown needs its text as a string"
 },
 {
  "name": "two nodes never share an id",
  "spec": {
   "pane": "quota",
   "title": "Quota",
   "blocks": [
    {
     "type": "flow",
     "nodes": [
      {
       "id": "a",
       "label": "A"
      },
      {
       "id": "a",
       "label": "B"
      }
     ]
    }
   ]
  },
  "error": "at /blocks/0/nodes/1/id: \"a\" is already the id of another node"
 },
 {
  "name": "an edge joins known nodes",
  "spec": {
   "pane": "quota",
   "title": "Quota",
   "blocks": [
    {
     "type": "flow",
     "nodes": [
      {
       "id": "a",
       "label": "A"
      },
      {
       "id": "b",
       "label": "B"
      }
     ],
     "edges": [
      {
       "from": "a",
       "to": "c"
      }
     ]
    }
   ]
  },
  "error": "at /blocks/0/edges/0/to: \"c\" is not a node id; ids: \"a\", \"b\""
 },
 {
  "name": "a node kind is one of three",
  "spec": {
   "pane": "quota",
   "title": "Quota",
   "blocks": [
    {
     "type": "flow",
     "nodes": [
      {
       "id": "a",
       "label": "A",
       "kind": "hexagon"
      }
     ]
    }
   ]
  },
  "error": "at /blocks/0/nodes/0/kind: one of \"box\", \"round\", \"decision\""
 },
 {
  "name": "a sequence has messages",
  "spec": {
   "pane": "quota",
   "title": "Quota",
   "blocks": [
    {
     "type": "sequence",
     "actors": [
      "Maya",
      "Sam"
     ]
    }
   ]
  },
  "error": "at /blocks/0/messages: a non-empty list of {from, to, label} or {note, over}"
 },
 {
  "name": "a sequence message or note",
  "spec": {
   "pane": "quota",
   "title": "Quota",
   "blocks": [
    {
     "type": "sequence",
     "messages": [
      {
       "from": "Maya",
       "to": "Sam",
       "label": "review?"
      },
      {
       "note": "waits",
       "over": "Sam"
      }
     ]
    }
   ]
  },
  "error": null
 },
 {
  "name": "a choice offers two options at least",
  "spec": {
   "pane": "quota",
   "title": "Quota",
   "blocks": [
    {
     "type": "choice",
     "options": [
      {
       "label": "Free"
      }
     ]
    }
   ]
  },
  "error": "at /blocks/0/options: at least two {label, value?, preview?}"
 },
 {
  "name": "a confirm says what it confirms",
  "spec": {
   "pane": "quota",
   "title": "Quota",
   "blocks": [
    {
     "type": "confirm"
    }
   ]
  },
  "error": "at /blocks/0/summary: what is being confirmed, as Markdown"
 },
 {
  "name": "a ci block names its branch",
  "spec": {
   "pane": "quota",
   "title": "Quota",
   "blocks": [
    {
     "type": "ci",
     "repo": "acme/api"
    }
   ]
  },
  "error": "at /blocks/0: a ci block is {repo, branch}"
 },
 {
  "name": "a log tails a command",
  "spec": {
   "pane": "quota",
   "title": "Quota",
   "blocks": [
    {
     "type": "log",
     "command": ""
    }
   ]
  },
  "error": "at /blocks/0/command: the shell command to tail"
 },
 {
  "name": "a log's lines are at least 1",
  "spec": {
   "pane": "quota",
   "title": "Quota",
   "blocks": [
    {
     "type": "log",
     "command": "tail -f api.log",
     "lines": 0
    }
   ]
  },
  "error": "at /blocks/0: every and lines are numbers of at least 1"
 },
 {
  "name": "a compare takes absolute pictures",
  "spec": {
   "pane": "quota",
   "title": "Quota",
   "blocks": [
    {
     "type": "compare",
     "before": "before.png",
     "after": "/tmp/after.png"
    }
   ]
  },
  "error": "at /blocks/0: a compare block is {before, after}, each an absolute path or an http(s) URL"
 },
 {
  "name": "an image takes a path or a url, not both",
  "spec": {
   "pane": "quota",
   "title": "Quota",
   "blocks": [
    {
     "type": "image",
     "path": "/tmp/a.png",
     "url": "https://acme.example/a.png"
    }
   ]
  },
  "error": "at /blocks/0: an image takes either path (an absolute path) or url (http or https), not both"
 },
 {
  "name": "a diff takes a patch or a repo",
  "spec": {
   "pane": "quota",
   "title": "Quota",
   "blocks": [
    {
     "type": "diff"
    }
   ]
  },
  "error": "at /blocks/0: a diff takes either patch (a unified diff) or repo (an absolute path), not neither"
 },
 {
  "name": "a diff base is a git ref",
  "spec": {
   "pane": "quota",
   "title": "Quota",
   "blocks": [
    {
     "type": "diff",
     "repo": "/home/me/repos/api",
     "base": "main branch"
    }
   ]
  },
  "error": "at /blocks/0/base: a git ref such as main or HEAD~3"
 },
 {
  "name": "a figure's good is a boolean",
  "spec": {
   "pane": "quota",
   "title": "Quota",
   "blocks": [
    {
     "type": "stats",
     "items": [
      {
       "label": "p95",
       "value": "180 ms",
       "good": "yes"
      }
     ]
    }
   ]
  },
  "error": "at /blocks/0/items/0/good: true when the delta is good news, false when bad"
 },
 {
  "name": "a stats block holds at most eight figures",
  "spec": {
   "pane": "quota",
   "title": "Quota",
   "blocks": [
    {
     "type": "stats",
     "items": [
      {
       "label": "f0",
       "value": 0
      },
      {
       "label": "f1",
       "value": 1
      },
      {
       "label": "f2",
       "value": 2
      },
      {
       "label": "f3",
       "value": 3
      },
      {
       "label": "f4",
       "value": 4
      },
      {
       "label": "f5",
       "value": 5
      },
      {
       "label": "f6",
       "value": 6
      },
      {
       "label": "f7",
       "value": 7
      },
      {
       "label": "f8",
       "value": 8
      }
     ]
    }
   ]
  },
  "error": "at /blocks/0/items: a list of 1 to 8 figures, each {label, value, note?, delta?, good?, spark?}"
 },
 {
  "name": "a chart kind is one of four",
  "spec": {
   "pane": "quota",
   "title": "Quota",
   "blocks": [
    {
     "type": "chart",
     "kind": "pie",
     "columns": [
      "a",
      "b"
     ],
     "rows": []
    }
   ]
  },
  "error": "at /blocks/0/kind: a chart is one of \"line\", \"area\", \"bar\", \"donut\""
 },
 {
  "name": "a chart flag is a boolean",
  "spec": {
   "pane": "quota",
   "title": "Quota",
   "blocks": [
    {
     "type": "chart",
     "kind": "area",
     "stacked": "yes",
     "columns": [
      "a",
      "b"
     ],
     "rows": []
    }
   ]
  },
  "error": "at /blocks/0/stacked: true or false"
 },
 {
  "name": "only bars lie down",
  "spec": {
   "pane": "quota",
   "title": "Quota",
   "blocks": [
    {
     "type": "chart",
     "kind": "line",
     "horizontal": true,
     "columns": [
      "a",
      "b"
     ],
     "rows": []
    }
   ]
  },
  "error": "at /blocks/0/horizontal: only bars lie down; use kind \"bar\""
 },
 {
  "name": "each row has a value per column",
  "spec": {
   "pane": "quota",
   "title": "Quota",
   "blocks": [
    {
     "type": "table",
     "columns": [
      "repo",
      "prs"
     ],
     "rows": [
      [
       "acme/api",
       3
      ],
      [
       "acme/web"
      ]
     ]
    }
   ]
  },
  "error": "at /blocks/0/rows/1: each row is a list of 2 plain values (\"repo\", \"prs\")"
 },
 {
  "name": "a table needs columns",
  "spec": {
   "pane": "quota",
   "title": "Quota",
   "blocks": [
    {
     "type": "table",
     "rows": []
    }
   ]
  },
  "error": "at /blocks/0/columns: a non-empty list of column names, or a command or source to get them from"
 },
 {
  "name": "x is one of the columns",
  "spec": {
   "pane": "quota",
   "title": "Quota",
   "blocks": [
    {
     "type": "chart",
     "kind": "line",
     "columns": [
      "day",
      "prs"
     ],
     "rows": [
      [
       "mon",
       2
      ]
     ],
     "x": "week"
    }
   ]
  },
  "error": "at /blocks/0/x: one of the columns: \"day\", \"prs\""
 },
 {
  "name": "a drawn column holds numbers",
  "spec": {
   "pane": "quota",
   "title": "Quota",
   "blocks": [
    {
     "type": "chart",
     "kind": "bar",
     "columns": [
      "team",
      "lead"
     ],
     "rows": [
      [
       "api",
       "Maya"
      ]
     ]
    }
   ]
  },
  "error": "at /blocks/0/y: column \"lead\" holds values that are not numbers; cast them in the query"
 },
 {
  "name": "a command refreshes every so many seconds",
  "spec": {
   "pane": "quota",
   "title": "Quota",
   "blocks": [
    {
     "type": "table",
     "command": "gh pr list --json number",
     "every": 0
    }
   ]
  },
  "error": "at /blocks/0/every: a number of seconds"
 },
 {
  "name": "a drawn column may hold gaps",
  "spec": {
   "pane": "quota",
   "title": "Quota",
   "blocks": [
    {
     "type": "chart",
     "kind": "line",
     "columns": [
      "day",
      "prs"
     ],
     "rows": [
      [
       "mon",
       3
      ],
      [
       "tue",
       null
      ]
     ],
     "y": [
      "prs"
     ]
    }
   ]
  },
  "error": null
 },
 {
  "name": "a command's rows refresh every minute",
  "spec": {
   "pane": "quota",
   "title": "Quota",
   "blocks": [
    {
     "type": "table",
     "command": "gh pr list --json number,title",
     "every": 60
    }
   ]
  },
  "error": null
 },
 {
  "name": "a Datadog source refreshes the chart",
  "spec": {
   "pane": "quota",
   "title": "Quota",
   "blocks": [
    {
     "type": "chart",
     "kind": "line",
     "source": {
      "datadog": {
       "queries": [
        "avg:api.latency{*}"
       ],
       "from": "now-1h"
      }
     },
     "every": 60
    }
   ]
  },
  "error": null,
  "mcp": "at /blocks/0/source: no server of cockpit's own for this source (servers in config.json, set up by cockpit setup); give rows, or a command printing JSON"
 },
 {
  "name": "a chart refreshed from any metrics tool",
  "spec": {"pane": "latency", "title": "Latency", "blocks": [{"type": "chart", "kind": "line", "source": {"metrics": {"queries": ["histogram_quantile(0.95, rate(http_seconds_bucket[5m]))"], "from": "now-6h"}}}]},
  "error": null,
  "mcp": "at /blocks/0/source: no server of cockpit's own for this source (servers in config.json, set up by cockpit setup); give rows, or a command printing JSON"
 },
 {
  "name": "a metrics source needs its queries",
  "spec": {"pane": "latency", "title": "Latency", "blocks": [{"type": "chart", "kind": "line", "source": {"metrics": {"queries": []}}}]},
  "error": "at /blocks/0/source/metrics/queries: a list of metric queries in your metrics tool's language",
  "mcp": "at /blocks/0/source: no server of cockpit's own for this source (servers in config.json, set up by cockpit setup); give rows, or a command printing JSON"
 },
 {
  "name": "a browser view takes no log",
  "spec": {
   "pane": "quota",
   "title": "Quota",
   "placement": "browser",
   "blocks": [
    {
     "type": "log",
     "command": "tail -f api.log"
    }
   ]
  },
  "error": "at /blocks/0: a browser view takes \"markdown\", \"stats\", \"table\", \"chart\", \"flow\", \"sequence\", \"mermaid\", \"image\", \"compare\", \"diff\" blocks with their data in the call; show this one in a pane",
  "mcp": null
 }
]
