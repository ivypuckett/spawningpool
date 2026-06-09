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
