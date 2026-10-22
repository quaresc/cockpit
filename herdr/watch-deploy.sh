#!/bin/sh
# After a ticket goes live: waits, has a headless Claude compare errors and latency before and after the deploy,
# adds the verdict to the ticket's runbook, and notifies once.   watch-deploy.sh <ticket> <repo,repo> [version]
PATH="/opt/homebrew/bin:/usr/local/bin:$HOME/.local/bin:$HOME/.claude/local:$PATH"
. "$(dirname "$0")/lib.sh"
ticket=$1 repos=$2 version=${3:-}
slug=$(printf '%s' "$ticket" | tr '[:upper:]' '[:lower:]')
mark="$HOME/.cache/cockpit/watched/$slug"
[ -e "$mark" ] && exit 0
mkdir -p "$(dirname "$mark")" && touch "$mark"

after=$(jq -r '.deployWatch.afterMinutes // 30' "$CONFIG")
deployed=$(date -u +%Y-%m-%dT%H:%M:%SZ)
sleep $((after * 60))

book="$HOME/.config/cockpit/runbooks/$slug.md"
row=""
[ -f "$book" ] && row="Then add one row at the end of the runbook table in $book, numbered after the last one:
| <n> | Watch after deploy | prod | ✅ <three words> or ❌ <three words> | |
keeping the file's format."
prompt="$ticket went live in production around $deployed (repos: $repos${version:+, version $version}).
Compare the 30 minutes before that time with the $after minutes after it, for the services these repos run in production:
error rate, new error logs, p95 latency. Use the observability tools you have, read-only. $row
Finish with exactly one line of JSON and nothing after it:
{\"ok\": true or false, \"summary\": \"<at most 12 words>\", \"url\": \"<a link to the logs or dashboard you looked at, or empty>\"}"
# Each entry of deployWatch.tools is one --allowedTools value, e.g. an observability MCP server.
set -- Read Edit Write
for tool in $(jq -r '.deployWatch.tools // [] | .[]' "$CONFIG"); do set -- "$@" "$tool"; done
out=$(cd "$REPOS" && claude -p "$prompt" --model sonnet --permission-mode acceptEdits --add-dir "$HOME/.config/cockpit/runbooks" \
  --allowedTools "$@" 2>/dev/null | grep '^{' | tail -1)

ok=$(printf '%s' "$out" | jq -r '.ok // empty' 2>/dev/null)
summary=$(printf '%s' "$out" | jq -r '.summary // empty' 2>/dev/null)
url=$(printf '%s' "$out" | jq -r '.url // empty' 2>/dev/null)
if [ -f "$book" ]; then action="runbook:$slug"; elif [ -n "$url" ]; then action=$url; else action="space:inbox"; fi
case "$ok" in
  true) sh "$ROOT/notify.sh" "✅ $ticket after deploy" "${summary:-no change in errors or latency}" "$action" ;;
  false) sh "$ROOT/notify.sh" "⚠️ $ticket after deploy" "${summary:-errors or latency moved, have a look}" "$action" ;;
  *) sh "$ROOT/notify.sh" "❔ $ticket after deploy" "The check did not conclude; see ~/.cache/cockpit/watch-$slug.log" "$action" ;;
esac
