# shellcheck shell=sh
# Removes the worktrees of your repos whose branch merged: commits of its own reached the default branch, or GitHub says
# its pull request merged at the commit it has. Kept, said as cockpit's cleanup says it: work not committed or pushed,
# ignored files other than builds, a program still in it, a lock, submodules, or a change in the last 3 days.
failed=0
unsaved=0
list=$(mktemp) || exit 1
errs=$(mktemp) || exit 1
# A command run 15 s at most: its output, and its status (above 128 once its time ran out).
bounded() {
  "$@" & pid=$!
  ( sleep 15; kill "$pid" 2>/dev/null ) >/dev/null 2>&1 & watch=$!
  wait "$pid"; code=$?
  kill "$watch" 2>/dev/null
  return "$code"
}
# What runs where: lsof (failing with nothing said, it found nothing), else Linux's /proc; else nothing can be said idle.
if command -v lsof >/dev/null; then
  cwds=$(bounded lsof -a -d cwd -Fcn 2>"$errs"); code=$?
  { [ "$code" -gt 128 ] || { [ "$code" -ne 0 ] && [ -s "$errs" ]; }; } && cwds=unknown
elif [ -d /proc/self ]; then
  cwds=$(for p in /proc/[0-9]*; do d=$(readlink "$p/cwd" 2>/dev/null) && printf 'c%s\nn%s\n' "$(cat "$p/comm" 2>/dev/null)" "$d"; done)
else
  cwds=unknown
fi
# The agents the hub knows of, from its agents document, one row a line: the folder and the agent's id of each row
# refreshed in the last quarter of an hour, whatever order its fields come in.
agents=$(tr -d '\n' < "$HOME/.cache/cockpit/agents.json" 2>/dev/null | tr '}' '\n' | awk -v now="$(date +%s)" '
  function field(name, number,    at) {
    if (!match($0, "\"" name "\":" (number ? "[0-9]+" : "\"[^\"]*\""))) return ""
    at = length(name) + 3 + (number ? 0 : 1)
    return substr($0, RSTART + at, RLENGTH - at - (number ? 0 : 1))
  }
  { c = field("cwd", 0); t = field("at", 1); if (c != "" && t != "" && now * 1000 - t < 900000) print c "\t" field("agent", 0) }')
# A program as cockpit names it: an agent by the agent table's name, anything else by its own.
name_of() {
  case $1 in
    claude|claude-code) echo "Claude Code" ;;
    codex) echo "Codex" ;;
    pi) echo "Pi" ;;
    omp) echo "Oh My Pi" ;;
    copilot|github-copilot|ghcs) echo "GitHub Copilot CLI" ;;
    devin|devin-cli) echo "Devin" ;;
    droid) echo "Droid" ;;
    kimi|kimi-code) echo "Kimi Code" ;;
    opencode|opencode2|open-code) echo "OpenCode" ;;
    kilo|kilo-code) echo "Kilo Code" ;;
    hermes|hermes-agent) echo "Hermes" ;;
    qodercli|qoder|qoderclicn|qodercn) echo "Qoder CLI" ;;
    qwen|qwen-code) echo "Qwen Code" ;;
    letta|letta-code) echo "Letta Code" ;;
    cursor|cursor-agent) echo "Cursor Agent" ;;
    mastracode|mastra-code) echo "Mastra Code" ;;
    antigravity-cli|agy|antigravity) echo "Antigravity" ;;
    grok|grok-build) echo "Grok" ;;
    gemini|gemini-cli) echo "Gemini CLI" ;;
    cline) echo "Cline" ;;
    kiro|kiro-cli) echo "Kiro CLI" ;;
    amp) echo "Amp" ;;
    maki) echo "Maki" ;;
    muse|muse-code|muse-cli) echo "Muse Code" ;;
    *) echo "$1" ;;
  esac
}
plural() { if [ "$1" -eq 1 ]; then echo "1 $2"; else echo "$1 $3"; fi; }
for repo in */; do
  repo=${repo%/}
  [ -e "$repo/.git" ] || continue
  top=$(git -C "$repo" rev-parse --show-toplevel) || { failed=1; continue; }
  base=$(git -C "$repo" symbolic-ref --quiet refs/remotes/origin/HEAD) || {
    echo "kept the worktrees of $repo: its default branch is unknown (git -C $repo remote set-head origin --auto)"; failed=1; continue; }
  git -C "$repo" worktree prune
  git -C "$repo" worktree list --porcelain -z | tr '\0' '\n' |
    awk '/^worktree /{p=substr($0, 10)} /^branch /{print p "\t" substr($0, 8)}' > "$list"
  while IFS="$(printf '\t')" read -r path ref; do
    [ "$path" = "$top" ] && continue
    branch=${ref#refs/heads/}
    tip=$(git -C "$repo" rev-parse "$ref")
    born=$(git -C "$repo" reflog show --format=%H "$ref" -- 2>/dev/null | tail -n 1)
    host=""
    if [ -n "$born" ] && [ "$tip" != "$born" ] && git -C "$repo" merge-base --is-ancestor "$ref" "$base"; then :
    else
      # GitHub's word counts for the commit it merged, or one before it; a branch merely gone is no merge.
      host=$(cd "$repo" && command -v gh >/dev/null && GH_PROMPT_DISABLED=1 bounded \
        gh pr list --head "$branch" --state merged --json headRefOid --jq '.[0].headRefOid // ""' 2>/dev/null)
      [ -n "$host" ] && { [ "$host" = "$tip" ] || git -C "$repo" merge-base --is-ancestor "$tip" "$host" 2>/dev/null; } || continue
    fi
    why=""
    add() { why="${why:+$why, }$1"; }
    changes=$(git -C "$path" status --porcelain --untracked-files=normal --ignore-submodules=none | grep -c .)
    [ "$changes" -gt 0 ] && add "$(plural "$changes" "uncommitted change" "uncommitted changes")"
    up=$(git -C "$repo" for-each-ref --format='%(upstream)|%(upstream:track)' "$ref")
    if [ -n "${up%%|*}" ] && [ "${up#*|}" != "[gone]" ]; then ahead=$(git -C "$path" rev-list --count "${up%%|*}..HEAD")
    elif [ -n "$host" ] && git -C "$repo" cat-file -e "$host^{commit}" 2>/dev/null; then ahead=$(git -C "$path" rev-list --count "$host..HEAD")
    else ahead=$(git -C "$path" rev-list --count HEAD --not --exclude="$branch" --branches --remotes); fi
    [ "$ahead" -gt 0 ] && add "$(plural "$ahead" commit commits) not pushed"
    ignored=$(git -C "$path" status --porcelain --ignored --untracked-files=normal | grep '^!! ' |
      grep -cvE '^!! (.*/)?(node_modules|target|_build|deps|dist|build|\.next|\.venv|__pycache__)/?$')
    [ "$ignored" -gt 0 ] && add "$(plural "$ignored" "ignored file" "ignored files") other than builds"
    [ -n "$why" ] && unsaved=1
    real=$(cd "$path" 2>/dev/null && pwd -P) || real=$path
    # The agent the hub knows in it, by its table name (Claude Code for a row that names none); else what lsof finds.
    known=$(printf '%s\n' "$agents" | awk -F '\t' -v p="$real" -v q="$path" '$1 == p || $1 == q || index($1, p "/") == 1 || index($1, q "/") == 1 { print ($2 == "" ? "claude" : $2); exit }')
    if [ -n "$known" ]; then
      add "$(name_of "$known") still runs in it"
    elif [ "$cwds" = unknown ]; then add "cannot tell what runs in it"
    else
      who=$(printf '%s\n' "$cwds" | awk -v p="$real" '/^c/ { c = substr($0, 2) }
        /^n/ { d = substr($0, 2); if ((d == p || index(d, p "/") == 1) && c !~ /^-?(sh|bash|zsh|fish|dash|lsof|cockpit|git|herdr|herdr-core|herdr-app|hc)$/) { print c; exit } }')
      [ -n "$who" ] && add "$(name_of "$who") still runs in it"
    fi
    lock=$(git -C "$repo" worktree list --porcelain | awk -v p="$path" -v r="$real" '/^worktree /{w=substr($0, 10)}
      /^locked/ && (w == p || w == r) { s = substr($0, 8); print (s == "" ? "locked" : "locked: " s); exit }')
    [ -n "$lock" ] && add "$lock"
    [ -f "$path/.gitmodules" ] && add "it holds submodules"
    [ -n "$(find "$path" -name .git -prune -o -type f -mtime -3 -print 2>/dev/null | head -n 1)" ] && add "changed in the last 3 days"
    if [ -n "$why" ]; then echo "kept $path: $why"; continue; fi
    if git -C "$repo" worktree remove "$path"; then echo "removed $path"; else failed=1; fi
  done < "$list"
done
rm -f "$list" "$errs"
# Merged work kept because it is not saved anywhere else needs a look: the run fails, so a "failed" notify says it.
[ "$failed" = 0 ] && [ "$unsaved" = 0 ]
