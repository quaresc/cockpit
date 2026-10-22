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
linear=$(jq -r 'if .linear.enabled then .linear.server else empty end' "$CONFIG")
prefix() { printf 'mcp__%s' "$(printf '%s' "$1" | sed 's/[^A-Za-z0-9_-]/_/g')"; }

set -- Write
[ -n "$linear" ] && set -- "$@" "$(prefix "$linear")__get_issue" "$(prefix "$linear")__list_comments"
for tool in $(jq -r '.meetingBrief.tools // [] | .[]' "$CONFIG"); do set -- "$@" "$tool"; done

# Everything known locally goes in the prompt, so the model writes instead of searching: the event, my tickets the
# meeting names (by key or project), their runbooks, and my PRs it names.
text=$(printf '%s' "$meeting" | jq -r '"\(.title)\n\(.description // "")"' | sed 's/<[^>]*>/ /g')
lower=$(printf '%s' "$text" | tr '[:upper:]' '[:lower:]')
tickets=$(jq -c --arg t "$lower" '
  [.tickets[] | select((.id | ascii_downcase) as $id | $t | contains($id))]
  + [.tickets[] | select(.project != null and (.project | length) >= 4 and ($t | contains(.project | ascii_downcase)))]
  | unique_by(.id) | map({id, title, status, project})' "$CACHE/inbox.json" 2>/dev/null)
prs=$(jq -c --arg t "$text" '[.mine[] | select(($t | contains(.url)) or ($t | contains("\(.repo)#\(.number)"))) | {repo, number, title, review, ci}]' "$CACHE/inbox.json" 2>/dev/null)
books=""
for id in $(printf '%s' "$tickets" | jq -r '.[]?.id' 2>/dev/null); do
  book="$HOME/.config/cockpit/runbooks/$(printf '%s' "$id" | tr '[:upper:]' '[:lower:]').md"
  [ -f "$book" ] && books="$books
--- runbook $id
$(head -30 "$book")"
done
event=$(printf '%s' "$meeting" | jq -c '{title, start, end, people, attachments, description: ((.description // "") | gsub("<[^>]*>"; " ") | .[0:1500])}')

prompt="Write the brief for this meeting to $DIR/$name.md, so I come prepared. Work from what is below; open a linked
Notion page or ticket only when it is essential and not covered here, at most two. No Slack, no web.

Meeting: $event
My tickets it relates to: ${tickets:-[]}
My PRs it names: ${prs:-[]}$books

Format, plain Markdown, at most 15 lines in all, headings included, no title heading. Every bullet fits on one line
under 90 characters, never wrapped onto a second line; links aside. One point per bullet, ticket keys rather than
titles, no filler.
- first line: why the meeting exists, in one short sentence
- ## Where it stands: up to 3 bullets
- ## Open questions: up to 2 bullets
- ## Bring: up to 2 points or questions for me to raise
- ## Links: up to 4 '- [short title](url)' lines; attachments go here by name
Leave out a section with nothing to say. Write the file, then reply with nothing."
model=$(jq -r '.meetingBrief.model // "sonnet"' "$CONFIG")
cd "$REPOS" && claude -p "$prompt" --model "$model" --effort medium --permission-mode acceptEdits --add-dir "$DIR" \
  --allowedTools "$@" </dev/null >"$CACHE/meeting-brief-$name.log" 2>&1

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
