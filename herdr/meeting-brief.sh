#!/bin/sh
# Writes the brief of one meeting with a headless Claude: why it exists, where things stand, what to bring.
# It reads the event, the Notion pages and tickets it links, and local runbooks; nothing else.   meeting-brief.sh <event id>
PATH="/opt/homebrew/bin:/usr/local/bin:$HOME/.local/bin:$HOME/.claude/local:$PATH"
. "$(dirname "$0")/lib.sh"
id=$1
CACHE="$HOME/.cache/cockpit"
DIR="$CACHE/meeting-briefs"
name=$(printf '%s' "$id" | sed 's/[^A-Za-z0-9_-]/_/g')
mkdir -p "$DIR"
[ -f "$DIR/$name.pending" ] && exit 0
touch "$DIR/$name.pending"
trap 'rm -f "$DIR/$name.pending"' EXIT

meeting=$(jq -c --arg id "$id" '.meetings[] | select(.id == $id)' "$CACHE/meetings.json")
[ -n "$meeting" ] || exit 1
calendar=$(jq -r '.calendar.server // "claude.ai Google Calendar"' "$CONFIG")
linear=$(jq -r 'if .linear.enabled then .linear.server else empty end' "$CONFIG")
prefix() { printf 'mcp__%s' "$(printf '%s' "$1" | sed 's/[^A-Za-z0-9_-]/_/g')"; }

set -- Read Write "$(prefix "$calendar")__get_event"
[ -n "$linear" ] && set -- "$@" "$(prefix "$linear")__get_issue" "$(prefix "$linear")__list_comments"
for tool in $(jq -r '.meetingBrief.tools // [] | .[]' "$CONFIG"); do set -- "$@" "$tool"; done

# The live state of all my work, to check older notes against.
live=$(jq -c '{tickets: [.tickets[] | {id, status}],
  open_prs: [.mine[] | {pr: "\(.repo)#\(.number)", branch, review, ci}],
  merged_prs: [(.merged // [])[] | {pr: "\(.repo)#\(.number)", branch, merged: .mergedAt[0:10]}],
  live_in_prod: (.deploys // {})}' "$CACHE/inbox.json" 2>/dev/null)

prompt="Write the brief for this meeting to $DIR/$name.md, so I come prepared. Meeting, from my calendar:
$meeting

Read the full event with get_event (id $id) for its description and attachments. Open the Notion pages and tickets it
links or names, with the tools you have; my assigned tickets and PRs are in $CACHE/inbox.json, and a ticket's runbook,
when it has one, in ~/.config/cockpit/runbooks/<ticket lowercased>.md. Use nothing else: no Slack, no web.

Live state of my work, fetched minutes ago: ${live:-{}}
This live state and the runbooks are current. Memory notes and older documents can be stale: when they disagree
with the live state (a ticket's status, a PR merged or not, something live in prod), trust the live state and drop
the stale claim.

Format, plain Markdown, at most 15 lines in all, headings included, no title heading. Every bullet fits on one line
under 90 characters, never wrapped onto a second line; links aside. One point per bullet, ticket keys rather than
titles, no filler.
- first line: why the meeting exists, in one short sentence
- ## Where it stands: up to 3 bullets
- ## Open questions: up to 2 bullets
- ## Bring: up to 2 points or questions for me to raise
- ## Links: up to 4 '- [short title](url)' lines; attachments you could not read go here by name
Leave out a section with nothing to say. Write the file, then reply with nothing."
model=$(jq -r '.meetingBrief.model // "sonnet"' "$CONFIG")
cd "$REPOS" && claude -p "$prompt" --model "$model" --permission-mode acceptEdits --add-dir "$DIR" \
  --add-dir "$HOME/.config/cockpit/runbooks" --add-dir "$CACHE" --allowedTools "$@" </dev/null >"$CACHE/meeting-brief-$name.log" 2>&1

# Whatever came back: wrapped bullets joined, prose lines cut at 90 characters (link lines keep their URL),
# 15 non-empty lines at most.
brief="$DIR/$name.md"
[ -f "$brief" ] && awk '
  /^[ \t]+[^ \t-]/ && n > 0 { sub(/^[ \t]+/, ""); lines[n] = lines[n] " " $0; next }
  NF { lines[++n] = $0 }
  END {
    for (i = 1; i <= n && i <= 15; i++) {
      line = lines[i]
      if (line !~ /\]\(http/ && length(line) > 90) line = substr(line, 1, 89) "…"
      print line
    }
  }' "$brief" > "$brief.tmp" && mv "$brief.tmp" "$brief"
