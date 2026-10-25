#!/bin/sh
# Popup summary of a PR: description, review state and checks. q closes it.
url="${PEEK_URL:?}"
GH_FORCE_TTY="$(tput cols 2>/dev/null || echo 120)"
export GH_FORCE_TTY GH_PAGER=cat
{
  gh pr checks "$url" 2>&1
  printf '\n'
  gh pr view "$url" 2>&1
} | less -R -K -P "q to close"
