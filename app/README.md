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

## Demo video

`npm --prefix app run render-video` records a short walkthrough of the UI to
`app/media/spawningpool-demo.webm` so it can be attached to (and reviewed in) a
PR. It drives the real Svelte frontend in headless Chromium and mocks the Tauri
IPC backend with deterministic seed data (`e2e/`), so no display server or Rust
build is needed.

First run, once per machine, download the browser:

```sh
cd app && npx playwright install chromium
```

Then render (re-run any time the UI changes):

```sh
npm --prefix app run render-video
```

The pre-commit hook re-renders and re-stages the video automatically, but only
when a commit touches the frontend (`app/src/`, `app/index.html`, `app/e2e/`,
or the Playwright/Vite/Svelte config). Rust-only commits skip it. If the
Playwright browser isn't installed it warns and skips rather than blocking the
commit; a genuine walkthrough failure (e.g. a renamed control) fails the
commit so the video can't silently go stale.

Tweak what the recording shows by editing the walkthrough in `e2e/demo.spec.ts`
and the registry data in `e2e/seed.ts`.
