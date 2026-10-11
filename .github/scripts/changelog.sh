#!/usr/bin/env bash
# Release notes for the build of HEAD: every pull request merged since the previous release tag
# (v*), with its commits as bullets, then direct commits. Usage: changelog.sh [<previous tag>]
set -euo pipefail

prev="${1:-$(git describe --tags --abbrev=0 --match 'v*' HEAD^ 2>/dev/null || true)}"
range="${prev:+$prev..}HEAD"
repo_url="${GITHUB_SERVER_URL:-https://github.com}/${GITHUB_REPOSITORY:-andreafunicelli/Handrail}"

echo "## What's new"
echo
found=0
while read -r sha parents; do
  set -- $parents
  if [ $# -ge 2 ]; then
    subject=$(git log -1 --format=%s "$sha")
    title=$(git log -1 --format=%b "$sha" | sed '/^[[:space:]]*$/d' | head -n 1)
    num=$(printf '%s' "$subject" | sed -n 's/^Merge pull request #\([0-9]*\).*/\1/p')
    if [ -n "$num" ]; then
      echo "### ${title:-Pull request #$num} ([#$num]($repo_url/pull/$num))"
    else
      echo "### ${title:-$subject}"
    fi
    git log --no-merges --reverse --format='- %s' "$1..$2"
  else
    git log -1 --format='- %s' "$sha"
  fi
  echo
  found=1
done < <(git log --first-parent --format='%H %P' "$range")
[ "$found" = 1 ] || echo "No changes since ${prev:-the first release}."
if [ -n "$prev" ]; then
  echo "**Full changes:** $repo_url/compare/$prev...$(git rev-parse --short HEAD)"
fi
