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

prompt="Write the brief for this meeting to $DIR/$name.md, so I come prepared. Meeting, from my calendar:
$meeting

Read the full event with get_event (id $id) for its description and attachments. Open the Notion pages and tickets it
links or names, with the tools you have; my assigned tickets and PRs are in $CACHE/inbox.json, and a ticket's runbook,
when it has one, in ~/.config/cockpit/runbooks/<ticket lowercased>.md. Use nothing else: no Slack, no web.

Format, plain Markdown, at most 15 lines, factual, no filler:
- first line: why the meeting exists, in one sentence
- ## Where it stands: up to 3 bullets
- ## Open questions: up to 3 bullets
- ## Bring: up to 3 points or questions for me to raise
- ## Links: up to 5 '- [title](url)' lines; attachments you could not read go here by name
Leave out a section with nothing to say. Write the file, then reply with nothing."
cd "$REPOS" && claude -p "$prompt" --model sonnet --permission-mode acceptEdits --add-dir "$DIR" --add-dir "$HOME/.config/cockpit/runbooks" \
  --add-dir "$CACHE" --allowedTools "$@" >"$CACHE/meeting-brief-$name.log" 2>&1
