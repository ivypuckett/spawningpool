#!/usr/bin/env bash
set -euo pipefail

# Single source of truth for where the rendered screenshots live and how to
# embed them. The pre-commit hook publishes the PNGs to the ref this prints
# (`--ref`), and the PR description embeds the markdown this prints (no args).
# Deriving both from one place is what keeps the push target and the image URLs
# from drifting apart — change the naming here and both follow.
#
#   app/e2e/screenshot-urls.sh --ref   # screenshots-<branch>  (the push target)
#   app/e2e/screenshot-urls.sh         # markdown block for the PR body

root=$(git rev-parse --show-toplevel)
branch=$(git rev-parse --abbrev-ref HEAD)
media_branch="screenshots-${branch//\//-}"

if [ "${1:-}" = "--ref" ]; then
  echo "$media_branch"
  exit 0
fi

# owner/repo from the origin URL: strip a trailing .git, take the last two path
# segments. Works for git@github.com:owner/repo.git and https URLs alike.
origin=$(git remote get-url origin)
slug=$(echo "${origin%.git}" | tr ':' '/' | awk -F/ '{print $(NF-1)"/"$NF}')

shopt -s nullglob
pngs=("$root"/app/media/screens/*.png)
shopt -u nullglob
if [ ${#pngs[@]} -eq 0 ]; then
  echo "no screenshots in app/media/screens/ — run 'npm --prefix app run render'" >&2
  exit 1
fi

for f in "${pngs[@]}"; do
  name=$(basename "$f")
  echo "![${name%.png}](https://raw.githubusercontent.com/${slug}/${media_branch}/${name})"
done
