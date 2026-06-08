#!/bin/bash
set -euo pipefail

if [ "${CLAUDE_CODE_REMOTE:-}" != "true" ]; then
  exit 0
fi

cd "$CLAUDE_PROJECT_DIR"

cargo fetch
cargo build --all

# Install the `task` runner (go-task) that specialists shell out to when
# running tool-backed Taskfile tasks. taskfile.dev is blocked by the default
# network policy, so fetch the release binary straight from GitHub. Idempotent:
# skipped if `task` is already on PATH.
TASK_VERSION="v3.51.1"
if ! command -v task >/dev/null 2>&1; then
  tmp="$(mktemp -d)"
  curl -sSL --retry 3 -o "$tmp/task.tar.gz" \
    "https://github.com/go-task/task/releases/download/${TASK_VERSION}/task_linux_amd64.tar.gz"
  tar -xzf "$tmp/task.tar.gz" -C "$tmp" task
  install -m 0755 "$tmp/task" /usr/local/bin/task
  rm -rf "$tmp"
fi
