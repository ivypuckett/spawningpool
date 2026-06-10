#!/bin/bash
set -euo pipefail

if [ "${CLAUDE_CODE_REMOTE:-}" != "true" ]; then
  exit 0
fi

cd "$CLAUDE_PROJECT_DIR"

cargo fetch
cargo build --workspace

# Activate the project git hooks so commits run fmt/clippy/test.
# .git/config isn't committed, so this must be set per clone.
git config core.hooksPath .githooks
