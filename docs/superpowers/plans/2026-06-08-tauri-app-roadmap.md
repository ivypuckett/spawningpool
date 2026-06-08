# Tauri Desktop App (`app` crate) — Implementation Roadmap

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this roadmap task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a management-first desktop GUI for spawningpool — the `app` workspace crate — that lets a user manage the registry (providers, models, specialists, tools) and run a specialist, on top of the existing front-end-agnostic `spawningpool` library.

**Architecture:** A new Tauri v2 crate (`app/src-tauri`, package name `app`) depends on the `spawningpool` library and exposes thin `#[tauri::command]` wrappers around `store`, `domain`, `tools`, `run`, and `ai`. A Svelte frontend renders a three-pane shell and forwards run progress over a `tauri::ipc::Channel<RunEvent>`. The registry on disk is the single source of truth, shared with the CLI.

**Tech Stack:** Rust + Tauri v2, Svelte + Vite + TypeScript frontend, Vitest for frontend unit tests, the existing `spawningpool` library for all domain logic.

**Source spec:** `docs/superpowers/specs/2026-06-08-tauri-user-flows-design.md`

---

## How this roadmap is structured

This is the **full direction** across five sequential plans. Each plan produces working, testable software on its own and is the prerequisite for the next.

- **Plan 1 (Foundation)** is written to execution-ready, bite-sized TDD granularity.
- **Plans 2–5** are documented as ordered task breakdowns — exact files, responsibilities, the library calls each wraps, key type signatures, and the test for each task. Their per-line frontend code is finalized against the real scaffolding at execution time (Plan 1 locks the patterns they copy), per the project rule of not guessing interfaces that don't exist yet.

Build order and what each plan ships:

| Plan | Ships | Depends on |
|---|---|---|
| 1. Foundation | Launchable app: three-pane shell, live registry load + watcher, read-only lists/detail for all four entity types | — |
| 2. Providers & Models | Create/edit/delete, key badge, Test connection, model Discover, reference badges, referrer-aware delete | 1 |
| 3. Specialists | Mode-toggle editor, reasoning auto-lock, constrained-decoding preview, live references | 1, 2 |
| 4. Tools | Author-new + register-existing editor, header compose/parse, model-view preview | 1 |
| 5. Run panel | `RunEvent` streaming transcript, re-run, pre-run validation | 1, 2, 3 |

---

## Shared foundations (conventions every plan follows)

These are introduced concretely in Plan 1 and reused verbatim afterward.

### Workspace & directory layout

```
spawningpool/            # existing library crate (unchanged except small extractions)
cli/                     # existing binary crate (unchanged)
app/                     # NEW — Tauri app
  package.json           # frontend (Vite + Svelte + TS, Vitest)
  vite.config.ts
  index.html
  src/                   # Svelte frontend
    lib/
      api.ts             # typed wrappers over `invoke(...)`
      types.ts           # TS mirrors of the serialized domain types
      stores.ts          # Svelte stores: registry snapshot, selection
    components/
      Shell.svelte       # three-pane frame
      EntityList.svelte  # middle pane list
      ...one editor component per entity (added in later plans)
    App.svelte
    main.ts
  src-tauri/             # NEW workspace member, Cargo package `app`
    Cargo.toml
    tauri.conf.json
    build.rs
    src/
      main.rs            # Tauri builder + generate_handler!
      commands/
        mod.rs
        registry.rs      # list/show/define/delete commands
        run.rs           # run_specialist command (Plan 5)
        tools.rs         # tool commands (Plan 4)
        connection.rs    # test_connection / discover (Plan 2)
      dto.rs             # serializable request/response shapes
      watch.rs           # filesystem watcher -> emits "registry-changed"
```

The root `Cargo.toml` gains `"app/src-tauri"` as a workspace member.

### Command-layer pattern

- Every command is a thin `#[tauri::command]` fn that calls the library and maps its `Result<_, String>` straight through (Tauri serializes `Err(String)` to a rejected promise the frontend catches).
- The domain types (`ProviderDef`, `ModelDef`, `Specialist`, `ToolDef`, `Registry`) already derive `Serialize`/`Deserialize`, so they cross the IPC boundary directly. Where a command needs an input shape the domain doesn't have (e.g. a specialist-editor payload that carries the mode), define it in `dto.rs`.
- Commands resolve the registry path through `spawningpool::store` so the app and CLI always agree on location.

### Error convention

Commands return `Result<T, String>`. The frontend `api.ts` wrappers let the rejection propagate; components catch it and render it **in context** (on the field/panel that triggered it), never as a raw alert. This reuses the library's already-actionable messages.

### Backend test harness

Backend command tests follow the CLI's existing pattern exactly: a `std::sync::Mutex` env-lock, point `$SPAWNINGPOOL_REGISTRY` at a temp file, exercise the command, assert against `store::load_from`. (See `cli/src/main.rs` tests for the template.)

### Frontend test harness

Vitest + `@testing-library/svelte`. The `invoke`/`Channel` calls from `@tauri-apps/api/core` are mocked, so component tests assert interaction logic (mode toggle locking reasoning, badge rendering) without a running backend.

### Commit discipline

Every task ends in a commit. Branch already in use: `docs/tauri-user-flows` holds the specs/plans; implementation happens on a fresh `feat/tauri-app` branch created at execution start.

---

# Plan 1 — Foundation

**Goal:** A launchable Tauri app showing a three-pane shell that lists all four entity types from the live registry, with read-only detail and a filesystem watcher that refreshes on external change.

### Task 1.1: Scaffold the `app` crate into the workspace

**Files:**
- Create: `app/**` (via scaffolder, then adjusted)
- Modify: `Cargo.toml` (root, add workspace member)

- [ ] **Step 1: Scaffold with the official tool**

Run from the repo root:
```bash
npm create tauri-app@latest app -- --template svelte-ts --manager npm
```
Choose Svelte + TypeScript when prompted (the flags preselect; confirm). This creates `app/` with `src/` (Svelte) and `src-tauri/` (Rust).

- [ ] **Step 2: Name the package and join the workspace**

Edit `app/src-tauri/Cargo.toml` so the package is named `app` and depends on the library:
```toml
[package]
name = "app"
version = "0.1.0"
edition = "2021"

[lib]
name = "app_lib"
crate-type = ["staticlib", "cdylib", "rlib"]

[build-dependencies]
tauri-build = { version = "2", features = [] }

[dependencies]
tauri = { version = "2", features = [] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["full"] }
notify = "6"
spawningpool = { path = "../../spawningpool" }
```

Add the member to the root `Cargo.toml`:
```toml
[workspace]
members = [
    "spawningpool",
    "cli",
    "app/src-tauri",
]
resolver = "2"
```

- [ ] **Step 3: Verify it builds and runs**

Run: `cargo build -p app`
Expected: compiles. Then `cargo test` from the root still passes the existing suite.

- [ ] **Step 4: Commit**

```bash
git add app Cargo.toml Cargo.lock
git commit -m "feat(app): scaffold Tauri v2 + Svelte crate into workspace"
```

### Task 1.2: A `ping` command end-to-end (proves the IPC wiring)

**Files:**
- Create: `app/src-tauri/src/commands/mod.rs`, `app/src-tauri/src/commands/registry.rs`
- Modify: `app/src-tauri/src/lib.rs` (or `main.rs`) to register handlers
- Test: `app/src-tauri/src/commands/registry.rs` (`#[cfg(test)]`)

- [ ] **Step 1: Write the failing test**

In `commands/registry.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_entities_reads_an_empty_registry() {
        let _g = test_support::env_lock();
        let _tmp = test_support::point_registry_at_temp();
        let snapshot = load_snapshot().unwrap();
        assert!(snapshot.providers.is_empty());
        assert!(snapshot.models.is_empty());
        assert!(snapshot.specialists.is_empty());
        assert!(snapshot.tools.is_empty());
    }
}
```

- [ ] **Step 2: Run it to confirm it fails**

Run: `cargo test -p app list_entities_reads_an_empty_registry`
Expected: FAIL (`load_snapshot`, `test_support` undefined).

- [ ] **Step 3: Implement `load_snapshot` and a `RegistrySnapshot` DTO**

In `commands/registry.rs`:
```rust
use serde::Serialize;
use spawningpool::{store, tools, Registry};

/// A flat, name-sorted view of everything in the registry plus the tools folder,
/// shaped for the frontend lists.
#[derive(Serialize)]
pub struct RegistrySnapshot {
    pub providers: Vec<String>,
    pub models: Vec<String>,
    pub specialists: Vec<String>,
    pub tools: Vec<String>,
    pub registry_path: String,
}

pub fn load_snapshot() -> Result<RegistrySnapshot, String> {
    let registry: Registry = store::load()?;
    let tool_names = tools::list(&store::tools_dir())?;
    Ok(RegistrySnapshot {
        providers: sorted(registry.providers.keys()),
        models: sorted(registry.models.keys()),
        specialists: sorted(registry.specialists.keys()),
        tools: tool_names,
        registry_path: store::registry_path().display().to_string(),
    })
}

fn sorted<'a>(keys: impl Iterator<Item = &'a String>) -> Vec<String> {
    let mut v: Vec<String> = keys.cloned().collect();
    v.sort();
    v
}

#[tauri::command]
pub fn list_entities() -> Result<RegistrySnapshot, String> {
    load_snapshot()
}
```

Create `app/src-tauri/src/test_support.rs` mirroring the CLI's env-lock + temp-registry helpers (`env_lock()` returns a `MutexGuard`; `point_registry_at_temp()` returns a guard that sets `$SPAWNINGPOOL_REGISTRY` to a unique temp path and restores it on drop).

- [ ] **Step 4: Register the command**

In `lib.rs`:
```rust
mod commands;
mod test_support;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![commands::registry::list_entities])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

- [ ] **Step 5: Run the test — expect PASS**

Run: `cargo test -p app list_entities_reads_an_empty_registry`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add app/src-tauri/src
git commit -m "feat(app): list_entities command returns a registry snapshot"
```

### Task 1.3: Frontend types, api wrapper, and registry store

**Files:**
- Create: `app/src/lib/types.ts`, `app/src/lib/api.ts`, `app/src/lib/stores.ts`
- Test: `app/src/lib/api.test.ts`

- [ ] **Step 1: Write the failing test** — `api.test.ts` mocks `invoke` to return a snapshot and asserts `listEntities()` resolves to it.
- [ ] **Step 2: Run** `npm --prefix app test -- api` → FAIL.
- [ ] **Step 3: Implement** `types.ts` (TS interfaces mirroring `RegistrySnapshot` and the domain types), `api.ts` (`listEntities()` calling `invoke('list_entities')`), `stores.ts` (a writable `registry` store plus a `selection` store of `{kind, name}`).
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** `feat(app): typed api wrapper and registry store`.

### Task 1.4: The three-pane shell

**Files:**
- Create: `app/src/components/Shell.svelte`, `EntityList.svelte`, `EntityDetail.svelte`
- Modify: `app/src/App.svelte`
- Test: `app/src/components/EntityList.test.ts`

- [ ] **Step 1: Failing test** — `EntityList.test.ts` renders the list given a snapshot and asserts each entity name appears and a `⚠` is absent for clean entities; selecting an item updates the `selection` store.
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement** the three panes: left rail (four types + counts + registry-path footer), middle filterable `EntityList`, right `EntityDetail` rendering the selected item's JSON read-only (via a `show_*` command added next task, stubbed until then). Load the snapshot on mount.
- [ ] **Step 4: Run** → PASS. Manual check: `npm --prefix app run tauri dev` shows the shell.
- [ ] **Step 5: Commit** `feat(app): three-pane shell with entity lists`.

### Task 1.5: Read-only detail via `show_*` commands

**Files:**
- Modify: `app/src-tauri/src/commands/registry.rs`, `lib.rs`, `app/src/components/EntityDetail.svelte`, `app/src/lib/api.ts`
- Test: backend `show_provider_returns_definition`; frontend detail render test

- [ ] **Step 1: Failing backend test** — define a provider into a temp registry via `store::save`, assert `show_entity(EntityKind::Provider, name)` returns its serialized def; an absent name errors with "no such".
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement** a single `show_entity(kind, name) -> Result<serde_json::Value, String>` command covering all four kinds (tools via `tools::resolve`), mirroring the CLI's `show`. Wire `EntityDetail` to call it and pretty-print.
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** `feat(app): read-only entity detail`.

### Task 1.6: Filesystem watcher → live refresh

**Files:**
- Create: `app/src-tauri/src/watch.rs`
- Modify: `lib.rs` (spawn watcher in `setup`), `app/src/lib/stores.ts` (listen for `registry-changed`, reload)
- Test: `watch.rs` unit test that a change to the watched file emits within a timeout

- [ ] **Step 1: Failing test** — `watch.rs` watches a temp file with `notify`, writes to it, asserts the debounced callback fires.
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement** a `notify` watcher on `store::registry_path()` and `store::tools_dir()` that, debounced (~200ms), calls `app_handle.emit("registry-changed", ())`. In `stores.ts`, `listen('registry-changed', reload)`. Also reload on window focus.
- [ ] **Step 4: Run** → PASS. Manual: edit the registry with `sp define` while the app is open; the list updates.
- [ ] **Step 5: Commit** `feat(app): live registry refresh via fs watcher`.

**Plan 1 done:** launchable app, live lists + read-only detail for all four types, auto-refresh. No mutation yet.

---

# Plan 2 — Providers & Models

**Goal:** Full create/edit/delete for providers and models, with the live key badge, Test connection, model Discover, reference validation, and referrer-aware delete.

**Library note — `list_models` generalization (do first):** `Client::list_models` is hardcoded to the provider name `"lmstudio"` and a fixed base URL. Generalize it (in the `spawningpool` library, TDD) to take the `ProviderDef` (api + base_url) and discover against any `openai-completions` endpoint, returning the same `Vec<Model>`. Keep the existing `"lmstudio"` convenience working. This is a small, well-bounded library change with its own tests.

### Task 2.1: `define_provider` / `delete_provider` commands
- **Files:** `commands/registry.rs`, `dto.rs`, `lib.rs`; backend tests.
- Wraps: `ProviderDef` construction, `store::save`, `registry.referrers(EntityKind::Provider, name)` for the delete warning.
- **Test:** define persists and round-trips through `store::load_from`; delete returns the referrer list (models + specialists) so the frontend can warn.
- DTO: `ProviderInput { name, api, base_url, api_key_env: Option<String>, constrained_decoding: bool }`. `api` parsed via `Api::from_str` (reuse the library's parser).

### Task 2.2: `key_status` command (live key badge)
- **Files:** `commands/connection.rs`.
- Returns, for a provider's `api_key_env`, whether it is set in the app's environment — the live form of the CLI's `unset_key_warnings`.
- **Test:** with a known env var set/unset, returns the right boolean and the env var name for the `export` hint.

### Task 2.3: `test_connection` command
- **Files:** `commands/connection.rs`.
- Makes a cheap call: for `anthropic-messages` and `openai-completions`, the smallest valid request (e.g. a 1-token completion against a caller-supplied model id, or a models-list GET for openai). Sources the key from `api_key_env`. Returns `Ok(())` or the provider `Error` string.
- **Test:** uses a stub/mocked endpoint (the library's provider tests show the pattern) — assert a 200 yields `Ok`, a 401 yields a readable error. Network calls are not made in unit tests; a thin seam lets the test inject the HTTP result.

### Task 2.4: Provider editor component
- **Files:** `app/src/components/ProviderEditor.svelte`, `api.ts`.
- Fields per spec; constrained-decoding toggle disabled for `anthropic-messages` with tooltip; amber key badge from `key_status` with copyable `export`; Test connection button rendering ✓/✗ inline; delete confirms with the referrer list.
- **Frontend test:** toggle disabled state by protocol; badge renders when key unset; delete shows referrers.

### Task 2.5: `define_model` / `delete_model` + discovery commands
- **Files:** `commands/registry.rs`, `commands/connection.rs`.
- Wraps: `ModelDef` construction, `registry.missing_model_ref` for the live ✓/⚠, `store::save`, `referrers(EntityKind::Model, …)`, and `discover_models(provider_name)` over the generalized `list_models`.
- **Test:** missing-provider model is rejected with the library message; discovery returns ids for an openai provider; delete reports referring specialists.

### Task 2.6: Model editor component
- **Files:** `app/src/components/ModelEditor.svelte`.
- Provider dropdown first; Discover button (only for openai providers) filling the id; id/name/max-tokens/context fields; live provider-resolves badge; referrer-aware delete.
- **Frontend test:** Discover hidden for anthropic providers; resolves-badge reflects a known/unknown provider.

**Plan 2 done:** providers and models are fully manageable end-to-end with validation and the connection check.

---

# Plan 3 — Specialists

**Goal:** The specialist editor with the agentic/forced mode toggle, reasoning auto-lock, constrained-decoding preview, and live reference badges — making both `validate()` errors structurally unreachable.

### Task 3.1: `define_specialist` / `delete_specialist` commands
- **Files:** `commands/registry.rs`, `dto.rs`.
- DTO `SpecialistInput { name, provider, model, system_prompt, mode, reasoning, stream }` where `mode` is an enum `{ Agentic { tools: Vec<String> }, Forced { constraint: String } }`. The command lowers `mode` into the domain `Specialist`'s `tools`/`constraint` fields, so the wire shape *cannot* express both — the structural guarantee lives at the boundary.
- Wraps: `Specialist::validate` (defense in depth), `registry.missing_specialist_ref(spec, |n| tools::exists(dir, n))`, `store::save`.
- **Test:** an agentic input yields a spec with tools and no constraint; a forced input yields a constraint and no tools; forced + non-off reasoning is rejected by `validate` (the DTO can still carry reasoning, so the command must reject it — assert the error). Missing provider/model/tool reported via `missing_specialist_ref`.

### Task 3.2: `check_specialist_refs` command (live badges)
- **Files:** `commands/registry.rs`.
- Returns, for an in-progress `SpecialistInput`, the first `MissingRef` (or none) so the editor can badge provider/model/each tool as ✓/⚠ live without saving.
- **Test:** mirrors the library's `missing_specialist_ref` ordering (provider → model → tool).

### Task 3.3: Specialist editor component
- **Files:** `app/src/components/SpecialistEditor.svelte`.
- Radio mode toggle (Agentic multi-select tools vs Forced single tool); reasoning selector auto-locks to `off` and disables in Forced mode with the reason shown; forced mode previews "uses constrained decoding" vs "forced via tool_choice" by reading the selected provider's `constrained_decoding`; stream toggle greys out when constrained decoding applies; per-reference ✓/⚠ via `check_specialist_refs`; a ⚠ tool links to the tool editor; "Run ▸" routes to the run panel (Plan 5) pre-targeted.
- **Frontend tests (the heart of the "impossible states" guarantee):**
  - switching to Forced mode disables the reasoning control and forces it to `off`;
  - switching back to Agentic re-enables reasoning;
  - selecting a constrained-decoding provider greys the stream toggle;
  - the payload sent to `define_specialist` carries `mode` and never both tools and a constraint.

**Plan 3 done:** specialists authored through a UI where the invalid combinations cannot be expressed.

---

# Plan 4 — Tool editor

**Goal:** In-app authoring of tool scripts plus registering existing ones, with header compose/parse and the model-view preview.

**Library extraction (do first):** the tool-install logic currently lives inline in `cli/src/main.rs::define_tool` (validate name, `prepare_script`, `summarize`, missing-desc warning, `tools::remove` then symlink). Extract two functions into the `spawningpool::tools` module so the CLI and app share them (TDD, with the CLI refactored to call them):
- `tools::install_existing(dir, name, script_path) -> Result<InstallReport, String>` — the symlink path (register existing).
- `tools::author(dir, name, description, params, body) -> Result<InstallReport, String>` — compose `#!`+`# desc:`+`# params:`+body, write into `dir`, set the executable bit, return a report (including a "no description" flag).
`InstallReport` carries any non-fatal warning (e.g. empty description) for the UI to surface.

### Task 4.1: tool commands
- **Files:** `commands/tools.rs`.
- `register_tool(name, path)` → `tools::install_existing`; `author_tool(name, description, params, body)` → `tools::author`; `delete_tool(name)` → `tools::remove` + `referrers(EntityKind::Tool, name)`; `read_tool(name)` → `tools::resolve` plus the raw script body (for the editor) and `ToolDef::to_tool` (the model-view preview).
- **Test:** author writes an executable script whose header round-trips through `summarize`; an invalid name is rejected; register symlinks an existing script; delete reports referring specialists; read returns body + parsed desc/params + the model-facing schema.

### Task 4.2: tool editor component
- **Files:** `app/src/components/ToolEditor.svelte`.
- Two entry modes (Author new / Register existing — the latter via the Tauri dialog plugin file picker); fields name/description/params(chips)/body; live valid-name + missing-desc warnings; "what the model sees" preview from the returned schema; referrer-aware delete.
- **Frontend test:** authoring composes the expected payload; invalid name disables save with the message; preview lists the params as required strings.

**Plan 4 done:** tools authored and registered entirely in-app; CLI and app share one install path.

---

# Plan 5 — Run panel

**Goal:** Run a specialist and watch it stream, mapping `RunEvent` 1:1, with re-run and pre-run validation. Single-run only.

### Task 5.1: `run_specialist` streaming command
- **Files:** `commands/run.rs`, a `RunUpdate` serializable DTO in `dto.rs`.
- Signature: `async fn run_specialist(app, specialist: String, prompt: String, channel: tauri::ipc::Channel<RunUpdate>) -> Result<(), String>`.
- Body mirrors `cli/src/main.rs::run_specialist`: load registry, `tools::resolve_all(tools_dir, spec.tool_names())` up front, build `CompleteOptions` (source the API key from the provider's `api_key_env`, set `constrained_decoding` from the provider), then call `spawningpool::run::run_specialist` with an observer closure that maps each `RunEvent` to a `RunUpdate` and `channel.send`s it.
- `RunUpdate` enum mirrors `RunEvent`: `TextDelta{delta}`, `Text{text}`, `Usage{input,output}`, `ToolRan{name,output,success}`, `ToolFailed{name,message}`. (Owned `String`s — the `RunEvent` borrows are only valid during the observer call, so the mapping clones.)
- **Test:** with a stubbed client/provider (the library's test seams) a canned turn produces the expected ordered `RunUpdate`s through a collecting channel; a missing-reference specialist errors before any send (pre-run validation).

### Task 5.2: Run panel component
- **Files:** `app/src/components/RunPanel.svelte`, `api.ts` (a `runSpecialist(name, prompt, onUpdate)` wrapper that constructs a `Channel`).
- Shows resolved provider/model + prompt box; streams assistant text; renders each `ToolRan` as an expandable block (inputs/output/exit status); `ToolFailed` in amber; per-turn usage; Re-run button; surfaces a pre-run error in the panel. Ephemeral — no persistence.
- **Frontend test:** feeding a sequence of `RunUpdate`s renders text, a tool block, and the usage line in order; a rejected run shows the error in-panel.

### Task 5.3: Wire "Run ▸"
- **Files:** `SpecialistEditor.svelte`, `Shell.svelte`.
- The specialist editor's "Run ▸" opens the run panel pre-targeted at that specialist.
- **Frontend test:** clicking Run ▸ mounts the panel with the specialist preselected.

**Plan 5 done:** a specialist can be run from the app with a live transcript. The app now mirrors the CLI's full capability set.

---

## Self-review — spec coverage map

Each spec requirement maps to a task:

| Spec section | Covered by |
|---|---|
| Integration: link the library | Plan 1.1–1.2 (crate, command pattern) |
| Three-pane shell | 1.4 |
| Stack: Tauri v2 + Svelte | 1.1 |
| Empty-state onboarding ladder | 1.4 (rail/empty state); refined as entities become creatable in 2–3 |
| Provider flow (form, key badge, test connection, delete) | 2.1–2.4 |
| Model flow (discover, validation, delete) | 2.1(note), 2.5–2.6 |
| Specialist mode toggle + reasoning lock + CD preview + live refs | 3.1–3.3 |
| Tool flow (author + register + preview + delete) | 4 (extraction + 4.1–4.2) |
| Run flow (RunEvent transcript, re-run, pre-run validation) | 5.1–5.3 |
| Shared registry + live reload + watcher | 1.6; reload-on-mutation in every define/delete |
| Error handling in-context | command error convention (foundations) + each editor task |
| Testing strategy (thin commands, component tests, run smoke test) | backend tests per command task; frontend tests per component; 5.1 smoke |
| Out of scope (compare/history, graph, keychain, locking) | not planned — explicitly excluded |

**Open follow-ups noted, not planned here:**
- The onboarding-ladder *text/derivation* lives in the CLI today; Plan 1 re-derives the minimal "what's empty" state in the frontend. If richer guidance is wanted, extracting the ladder into the library is a later, separate task.
- `notify` debounce tuning and window-focus reload are implemented in 1.6 but may want adjustment once real usage exists.
