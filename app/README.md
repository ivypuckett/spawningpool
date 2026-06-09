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

This captures each UI state to `app/media/screens/*.png` (gitignored). An agent
can read these back to verify its own frontend changes, and the pre-commit hook
publishes them into the PR (below).

It drives the exact Svelte frontend the app ships, mocking only the Tauri IPC
backend with fixed seed data (`e2e/`), so the output is reproducible and needs
no display server, Rust build, or running Tauri webview. Edit the captured
states in `e2e/demo.spec.ts` and the registry data in `e2e/seed.ts`.

### How it runs in a locked-down environment

Cloud agent sandboxes often block Playwright's browser CDN. To stay portable,
the harness fetches **Chrome for Testing** (pinned in `e2e/browser.ts`) from
the reachable Google Cloud Storage bucket on first run, caching it under
`app/.browser` (gitignored). Nothing else is required — no `playwright install`,
no system Chrome, no apt packages. The SessionStart hook
(`.claude/hooks/session-start.sh`) installs the frontend deps and provisions
this Chrome up front, so a fresh agent session can render immediately.

### Screenshots in the PR

The screenshots aren't committed to the branch (they'd churn the diff and
outlive the PR). Instead the pre-commit hook publishes them, on **every commit**,
as a single parentless commit on a disposable side branch `screenshots-<branch>`,
force-pushed so the previous set is discarded:

- The PR description embeds those images by **stable URL**
  (`https://raw.githubusercontent.com/<owner>/<repo>/screenshots-<branch>/01-overview.png`).
  Because the URL is stable and the hook force-pushes fresh images, the pictures
  refresh on every commit with no GitHub token — just the push credentials git
  already has. (Public repo, so plain `raw.githubusercontent.com` URLs render
  inline.)
- Set the PR body once (it references the stable URLs); delete the
  `screenshots-<branch>` branch when the PR closes.

This runs on every commit — the screenshots double as living proof of what the
system renders, so they're kept current regardless of what changed. It's strict:
because the SessionStart hook guarantees the deps and browser are in place, a
failure here is a real problem (a broken walkthrough, a UI regression, no
network to push) and **fails the commit** rather than letting the screenshots
silently go stale. The one place a commit now requires the network is the
force-push of the screenshots branch.
