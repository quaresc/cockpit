#!/bin/sh
# Checks every part of cockpit and prints one line per check: ok, warn or fail, what it found, and the fix.
PATH="/opt/homebrew/bin:/usr/local/bin:$HOME/.local/bin:$HOME/.claude/local:$PATH"
. "$(dirname "$0")/lib.sh"
CACHE="$HOME/.cache/cockpit"
now=$(date +%s)

ok() { printf 'ok    %s\n' "$*"; }
warn() { printf 'warn  %s\n' "$*"; }
fail() { printf 'fail  %s\n' "$*"; }
age() { echo $(( now - $(stat -f %m "$1" 2>/dev/null || stat -c %Y "$1" 2>/dev/null || echo 0) )); }
enabled() { [ "$(jq -r ".$1.enabled // false" "$SETTINGS" 2>/dev/null)" = true ]; }

# Tools and config
v=$("$H" --version 2>/dev/null | awk '{print $2}')
case "$v" in 0.[0-8].*|"") fail "herdr ${v:-missing}: cockpit needs 0.9 or later → herdr update" ;; *) ok "herdr $v" ;; esac
gh auth status >/dev/null 2>&1 && ok "gh signed in" || fail "gh not signed in → gh auth login"
command -v python3 >/dev/null && ok "python3" || fail "python3 missing"
if [ "$(jq -r '.apps // "full"' "$CONFIG" 2>/dev/null)" = full ]; then
  command -v uv >/dev/null && ok "uv $(uv --version | awk '{print $2}') runs the full apps" ||
    warn "uv is missing, so the apps run light → brew install uv, or /cockpit apps light to say so"
else
  ok "light apps"
fi
if [ ! -f "$CONFIG" ]; then fail "no ~/.config/cockpit/config.json → /cockpit:setup"
elif jq empty "$CONFIG" 2>/dev/null; then ok "config.json"
else fail "config.json is not valid JSON → fix it or rerun /cockpit:setup"; fi

# Both halves installed, and the link on the current herdr install
[ -n "$REGISTERED" ] && [ -d "$REGISTERED" ] && ok "herdr plugin at $REGISTERED" ||
  fail "herdr plugin not installed → herdr plugin install quaresc/cockpit/herdr --yes"
[ "$(readlink "$LINK")" = "$REGISTERED" ] && ok "link ~/.config/cockpit/herdr is current" ||
  fail "link ~/.config/cockpit/herdr points at $(readlink "$LINK") → sh $REGISTERED/ensure-spaces.sh"
case "$REGISTERED" in */github-installations/*/checkout/herdr)
  old=$(find "$(dirname "$(dirname "$(dirname "$REGISTERED")")")" -mindepth 1 -maxdepth 1 -type d | grep -vc "$(dirname "$(dirname "$REGISTERED")")$")
  [ "$old" -eq 0 ] && ok "no old herdr installs" || warn "$old old herdr installs → sh $REGISTERED/ensure-spaces.sh prunes them" ;;
esac
installed=$(claude plugin list 2>/dev/null | awk '/cockpit@cockpit/ {found=1} found && /Version:/ {print $2; exit}')
[ -n "$installed" ] && ok "Claude plugin cockpit $installed" || fail "Claude plugin not installed → /plugin install cockpit --marketplace quaresc/cockpit"
latest=$(jq -r '.latest // empty' "$CACHE/update.json" 2>/dev/null)
newest=$(printf '%s\n%s\n' "$latest" "$installed" | sort -t. -k1,1n -k2,2n -k3,3n | tail -1)
[ -n "$latest" ] && [ -n "$installed" ] && [ "$latest" != "$installed" ] && [ "$newest" = "$latest" ] &&
  warn "cockpit $latest is published → sh $LINK/update.sh"

# Claude sessions: each one in herdr should beat with the installed version
find "$CACHE/sessions" -name '*.json' -mmin +5 -delete 2>/dev/null
"$H" pane list 2>/dev/null | jq -r '.result.panes[] | select(.agent == "claude") | "\(.pane_id) \(.workspace_id)"' |
while read -r pane ws; do
  label=$("$H" workspace list | jq -r --arg w "$ws" '.result.workspaces[] | select(.workspace_id == $w) | .label' | sed 's/^\[[0-9]*\] *//')
  # A pane can hold beats from more than one instance (the conversation before a /clear on old code): the one on the
  # installed version is the session; any other still beating is a leftover.
  beats=$(cat "$CACHE"/sessions/*.json 2>/dev/null | jq -s -c --arg p "$pane" 'map(select(.pane == $p))')
  beat=$(printf '%s' "$beats" | jq -c --arg v "$installed" '(map(select(.version == $v)) | max_by(.at)) // max_by(.at) // empty')
  version=$(printf '%s' "$beat" | jq -r '.version // empty')
  others=$(printf '%s' "$beats" | jq -r --arg v "$version" 'map(select(.version != $v)) | length')
  if [ -z "$beat" ]; then warn "session \"$label\" ($pane) runs no cockpit → /reload-plugins in it, or restart it"
  elif [ -n "$installed" ] && [ "$version" != "$installed" ]; then warn "session \"$label\" runs cockpit $version, $installed is installed → /reload-plugins in it"
  elif [ "$others" -gt 0 ]; then warn "session \"$label\" on cockpit $version, but an older cockpit still runs in its pane → restart Claude there (exit, then claude --resume)"
  else ok "session \"$label\" on cockpit $version"; fi
done

# Shared caches: fresh, and without errors
check_cache() {
  file=$1 name=$2 limit=$3
  if [ ! -f "$file" ]; then warn "$name: no data yet → start a Claude session in herdr; it fills within a minute"; return; fi
  a=$(( now - $(jq -r '(.at // 0) / 1000 | floor' "$file") ))
  errors=$(jq -r '[.errors[]?, .error // empty] | join("; ")' "$file")
  if [ -n "$errors" ]; then fail "$name: $errors"
  elif [ "$a" -gt "$limit" ]; then warn "$name: last refresh $((a / 60)) min ago → no Claude session with cockpit is running"
  else ok "$name: refreshed $((a / 60)) min ago"; fi
}
check_cache "$CACHE/inbox.json" inbox 900
[ "$(jq -r '.ci.enabled != false' "$SETTINGS" 2>/dev/null)" = true ] && check_cache "$CACHE/ci.json" CI 600
enabled calendar && check_cache "$CACHE/meetings.json" meetings 900

# Pinned spaces and their apps
spaces=$("$H" workspace list 2>/dev/null | jq -r '.result.workspaces[].label')
while IFS= read -r space; do
  [ -n "$space" ] || continue
  printf '%s\n' "$spaces" | grep -q -- "  $space\$" || { fail "space $space missing → sh $LINK/ensure-spaces.sh"; continue; }
  grep -Eq "contains = \"  ($space)?\"" "$HOME/.config/herdr/config.toml" 2>/dev/null ||
    warn "space $space is hidden in the sidebar → add { contains = \"  \" } before the hiding rule of ui.sidebar.spaces in ~/.config/herdr/config.toml"
  case "$space" in inbox | to-do | runbook | meetings) ;; *) ok "space $space"; continue ;; esac
  app=$(echo "$space" | sed 's/to-do/todo/')
  pgrep -f "herdr/$app-loop\.sh" >/dev/null && ok "space $space, app running" || fail "space $space has no app running → sh $LINK/ensure-spaces.sh"
done <<EOF
$SPACES
EOF

# Notifications
if [ "$(jq -r '.notifications // true' "$SETTINGS" 2>/dev/null)" = true ]; then
  if [ "$OS" = Darwin ]; then
    if sh "$ROOT/notifier/build.sh" >/dev/null 2>&1; then
      status=$("$HOME/.config/cockpit/Cockpit.app/Contents/MacOS/cockpit-notify" status 2>/dev/null)
      [ "$status" = allowed ] && ok "notifications allowed" ||
        fail "notifications ${status:-unknown} → System Settings → Notifications → Cockpit → Allow"
    else fail "notifier not built → xcode-select --install, then sh $ROOT/notifier/build.sh"; fi
  else
    command -v notify-send >/dev/null && ok "notify-send" || warn "notify-send missing: herdr's toasts are used instead"
  fi
  delivery=$(sed -n '/^\[ui\.toast\]/,/^\[/p' "$HOME/.config/herdr/config.toml" 2>/dev/null | sed -n 's/^delivery *= *"\(.*\)"/\1/p')
  [ "$delivery" = herdr ] && ok "herdr toasts in-app" || warn "herdr ui.toast.delivery is \"${delivery:-system}\": notifications arrive twice → set it to \"herdr\""
fi

# terminal-browser for an agent's E2E tests beside it: optional, they run in Claude in Chrome without it
if ! command -v terminal-browser >/dev/null; then
  ok "terminal-browser not installed: E2E tests run in Claude in Chrome (brew install --cask terminal-browser runs them beside the agent)"
else
  ok "terminal-browser $(terminal-browser --version 2>/dev/null | awk '{print $2}')"
  installs=$(which -a terminal-browser | while read -r bin; do realpath "$bin" 2>/dev/null; done | sort -u | wc -l | tr -d ' ')
  [ "$installs" -le 1 ] || warn "$installs terminal-browser installs, each with its own sign-ins → keep the brew one: rm ~/.local/bin/terminal-browser && rm -rf ~/.local/share/terminal-browser"
  grep -Eq '^[[:space:]]*kitty_graphics[[:space:]]*=[[:space:]]*false' "$HOME/.config/herdr/config.toml" 2>/dev/null &&
    fail "herdr kitty_graphics is off: terminal-browser panes stay blank → set it to true under [experimental]" ||
    ok "herdr kitty graphics on"
  left=$(sh "$ROOT/prune-browsers.sh" --count)
  [ "$left" -eq 0 ] && ok "no agent-browser daemons left by closed browsers" ||
    warn "$left agent-browser daemons left by closed browsers → sh $LINK/prune-browsers.sh"
  transport=$(terminal-browser config get render.transport 2>/dev/null)
  [ "$transport" = auto ] && ok "terminal-browser frame transport auto" ||
    warn "terminal-browser render.transport is ${transport:-unset}: herdr cannot hand its frames to the terminal as files → terminal-browser config set render.transport auto"
  fps=$(terminal-browser config get render.fps 2>/dev/null)
  case "$fps" in 60) ok "terminal-browser at 60 frames per second" ;;
    *) warn "terminal-browser render.fps is ${fps:-display}: 60 scrolls smoothly and leaves herdr room → terminal-browser config set render.fps 60" ;; esac
  # herdr hands frames to Ghostty as files in /tmp, which Ghostty reads on macOS from its tip builds (stable 1.3.1 and older refuse them).
  if [ "$OS" = Darwin ] && [ -x /Applications/Ghostty.app/Contents/MacOS/ghostty ]; then
    info=$(/Applications/Ghostty.app/Contents/MacOS/ghostty --version 2>/dev/null)
    version=$(printf '%s\n' "$info" | sed -n 's/^ *- version: //p')
    channel=$(printf '%s\n' "$info" | sed -n 's/^ *- channel: //p')
    case "$channel:$version" in
      tip:*|*:1.3.[2-9]*|*:1.[4-9].*|*:[2-9].*) ok "Ghostty $version ($channel) reads herdr's frame files" ;;
      *) warn "Ghostty $version ($channel) refuses herdr's frame files, so terminal-browser panes are slow → add auto-update-channel = tip to Ghostty's config, then Ghostty → Check for Updates…, or download the tip build from ghostty.org/download" ;;
    esac
  fi
fi

# The platform connector's tools, mapped by /cockpit:setup to what cockpit does with deploys and previews
if enabled platform; then
  mapped=$(jq -r '.platform.tools // {} | to_entries | map(select(.value.tool // "" | length > 0) | .key) | join(", ")' "$SETTINGS")
  [ -n "$mapped" ] && ok "platform tools mapped: $mapped" ||
    warn "platform is on but no tool is mapped: no deploys or previews → /cockpit:setup maps them"
fi

# The ticket connector's tools, mapped by /cockpit:setup (Linear's by default)
if jq -e '.linear and (.tickets | not)' "$CONFIG" >/dev/null 2>&1; then
  warn "config has the old linear block: it still works as Linear's mapping → /cockpit:setup rewrites it as tickets"
elif enabled tickets; then
  missing=$(jq -r '.tickets.tools as $t | if $t then ["listMine", "get", "listStates", "setState"] | map(select(($t[.].tool // "") == "")) | join(", ") else "" end' "$SETTINGS")
  [ -z "$missing" ] && ok "ticket tools mapped" || warn "ticket tools not mapped: $missing; cockpit skips what they do → /cockpit:setup maps them"
fi

# The CI: GitHub's checks through gh, or a CI connector mapped by /cockpit:setup
if jq -e '.circleci and (.ci | not)' "$CONFIG" >/dev/null 2>&1; then
  warn "config has the old circleci block: it still works as CircleCI's mapping → /cockpit:setup rewrites it as ci"
elif [ -n "$(jq -r '.ci.server // empty' "$SETTINGS")" ]; then
  missing=$(jq -r '.ci.tools as $t | if $t then ["listRuns", "listJobs"] | map(select(($t[.].tool // "") == "")) | join(", ") else "" end' "$SETTINGS")
  [ -z "$missing" ] && ok "CI tools mapped" || warn "CI tools not mapped: $missing; no CI runs → /cockpit:setup maps them"
else
  ok "CI from GitHub checks"
fi

# Permission rules for the background MCP calls
for source in tickets ci platform calendar; do
  block=$(jq -c --arg s "$source" '.[$s] // {}' "$SETTINGS" 2>/dev/null)
  [ "$(printf '%s' "$block" | jq -r '.enabled // false')" = true ] || continue
  server=$(printf '%s' "$block" | jq -r --arg s "$source" '.server // (if $s == "tickets" then "claude.ai Linear" else "" end)')
  [ -n "$server" ] || continue
  rule="mcp__$(printf '%s' "$server" | sed 's/[^A-Za-z0-9_-]/_/g; s/__*/_/g; s/_$//')"
  jq -e --arg r "$rule" '.permissions.allow // [] | any(. == $r or startswith($r + "__"))' "$HOME/.claude/settings.json" >/dev/null 2>&1 &&
    ok "allow rule for $server" || fail "no allow rule for $server: background refreshes are refused → add \"$rule\" to permissions.allow"
done

# The read-only tools live show panes call
allowed() { jq -e --arg r "$1" '.permissions.allow // [] | any(. as $a | $r == $a or ($r | startswith($a + "__")))' "$HOME/.claude/settings.json" >/dev/null 2>&1; }
for pair in "datadog:get_datadog_metric" "metabase:query"; do
  source=${pair%%:*} tool=${pair#*:}
  server=$(jq -r --arg s "$source" '.[$s].server // empty' "$SETTINGS" 2>/dev/null)
  [ -n "$server" ] || server=$(printf '%s' "$source" | sed 's/datadog/claude.ai Datadog/')
  rule="mcp__$(printf '%s' "$server" | sed 's/[^A-Za-z0-9_-]/_/g; s/__*/_/g; s/_$//')__$tool"
  allowed "$rule" && ok "allow rule for $rule" || warn "no allow rule for $rule: its live pane asks or is refused → add it to permissions.allow"
done
