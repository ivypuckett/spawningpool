# app

A desktop GUI for the `spawningpool` library. Browse providers, models, specialists, and tools defined in your local registry, view their full definitions, and get live refresh when the registry changes on disk.

Read-only so far: navigation and display only.

## Develop

```sh
npm --prefix app install
npm --prefix app run tauri dev
```

Requires Rust, Node, and the [Tauri Linux system dependencies](https://tauri.app/start/prerequisites/).

## Test

```sh
# Frontend
npm --prefix app test

# Backend
cargo test -p app
```

## Build

```sh
npm --prefix app run tauri build
```

## Visual rendering

```sh
npm --prefix app run render
```

This produces a deterministic visual representation of the UI so frontend work
can be seen and reviewed:

- **Screenshots** of each UI state in `app/media/screens/*.png` — the primary
  artifact. An agent can read these back to verify its own changes, and they
  show up inline in a PR.
- A short **walkthrough video** at `app/media/spawningpool-demo.webm`.

It drives the exact Svelte frontend the app ships, mocking only the Tauri IPC
backend with fixed seed data (`e2e/`), so the output is reproducible and needs
no display server, Rust build, or running Tauri webview.

### How it runs in a locked-down environment

Cloud agent sandboxes often block Playwright's browser CDN. To stay portable,
the harness fetches **Chrome for Testing** (pinned in `e2e/browser.ts`) from
the Google Cloud Storage bucket on first run, caching it under `app/.browser`
(gitignored). Nothing else is required — no `playwright install`, no system
Chrome, no apt packages.

### Pre-commit

The pre-commit hook re-renders and re-stages `app/media` automatically, but
only when a commit touches the frontend (`app/src/`, `app/index.html`,
`app/e2e/`, or the Playwright/Vite/Svelte config). Rust-only commits skip it.
If the browser can't be provisioned (no network) it warns and skips rather than
blocking; a genuine render failure (e.g. a renamed control breaking the
walkthrough) fails the commit so the visuals can't silently go stale.

Edit the captured states in `e2e/demo.spec.ts` and the registry data in
`e2e/seed.ts` to change what's shown.
