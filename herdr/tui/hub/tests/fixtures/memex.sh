#!/bin/sh
# A fake memex: logs its arguments to memex.log beside it, waits the seconds a `delay` file beside it says, then prints
# acme's recorded search hits holding the query (the last argument), one of them a subagent's, or the session rows: the
# one --session-id asks for, else all.
here=$(dirname "$0")
echo "$*" >>"$here/memex.log"
sleep "$(cat "$here/delay" 2>/dev/null || echo 0)"
for arg do last=$arg; done
case "$1" in
  search) grep -iF -- "$last" <<'EOF'
{"ts":"2026-10-27T16:02:11Z","project":"api","session_id":"s-login","source_path":"/h/.claude/projects/-acme-api/s-login.jsonl","snippet":"…the Google sign-in callback now checks the state"}
{"ts":"2026-10-27T15:40:00Z","project":"api","session_id":"agent-a1","source_path":"/h/.claude/projects/-acme-api/s-login/subagents/agent-a1.jsonl","snippet":"a subagent looked at the sign-in callback"}
{"ts":"2026-09-02T09:00:00Z","project":"web","session_id":"s-dark","source_path":"/h/.claude/projects/-acme-web/s-dark.jsonl","snippet":"…dark mode tokens for the sign-in page"}
EOF
    ;;
  sessions)
    id='' prev=''
    for arg do
      [ "$prev" = --session-id ] && id=$arg
      prev=$arg
    done
    grep -F -- "\"session_id\":\"$id" <<'EOF'
{"source":"claude","session_id":"s-login","project":"api","cwd":"@BOX@/repos/api","last_at":"2026-10-27T16:05:00Z","label":"Google sign-in for ENG-401"}
{"source":"claude","session_id":"s-dark","project":"web","cwd":"@BOX@/repos/web","last_at":"2026-09-02T09:30:00Z","label":""}
EOF
    ;;
esac
