#!/bin/bash
set -euo pipefail

if [ "${CLAUDE_CODE_REMOTE:-}" != "true" ]; then
  exit 0
fi

cd "$CLAUDE_PROJECT_DIR"

# Rust workspace. The Tauri 'app' backend needs GTK/WebKit; lightweight sandboxes
# don't have them (and can't apt-install), so build it only where it can compile.
# The frontend render path below mocks the backend and doesn't need it.
cargo fetch
cargo_scope=(--workspace)
if ! pkg-config --exists gdk-3.0 2>/dev/null; then
  echo "note: GTK not found — building without the Tauri 'app' backend." >&2
  cargo_scope=(--workspace --exclude app)
fi
cargo build "${cargo_scope[@]}"

# Desktop app (Tauri + Svelte): install deps and provision the pinned Chrome the
# screenshot harness drives, so `npm --prefix app run render` and the pre-commit
# screenshot step work immediately. The first render downloads Chrome for Testing
# (cached for the session); we do it now and tolerate a transient network failure
# so a session still starts.
npm --prefix app install
npm --prefix app run render \
  || echo "warning: initial UI render failed (check network to storage.googleapis.com)" >&2

# Activate the project git hooks so commits run fmt/clippy/test and publish the
# UI screenshots. .git/config isn't committed, so this must be set per clone.
git config core.hooksPath .githooks
