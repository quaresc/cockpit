#!/bin/sh
# Writes app/README.md, the page the Herdr gallery links to, as a copy of the root README with its relative links
# pointing one folder up.   app-readme.sh [--check]
set -eu
cd "$(dirname "$0")/.."

{
  echo '<!-- A copy of ../README.md, written by scripts/app-readme.sh: edit that one. -->'
  echo
  sed -E 's@(\]\(|src="|srcset="|href=")([^)"#/][^):"]*)([)"])@\1../\2\3@g' README.md
} >app/README.md.new

if [ "${1:-}" = "--check" ]; then
  if ! cmp -s app/README.md app/README.md.new; then
    rm app/README.md.new
    echo "app/README.md is not the root README's copy: run sh scripts/app-readme.sh" >&2
    exit 1
  fi
  rm app/README.md.new
  exit 0
fi
mv app/README.md.new app/README.md
