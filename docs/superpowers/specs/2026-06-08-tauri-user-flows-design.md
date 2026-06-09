# Tauri Desktop App — User Flows Design

**Date:** 2026-06-08
**Status:** Approved design (pre-implementation)
**Scope:** The user flows and UI structure for the planned Tauri desktop app, the
`app` workspace crate. This document defines *what the experience is*; the
implementation plan is a separate document.

## Purpose

spawningpool today is a CLI (`sp`) over a front-end-agnostic library. The Tauri
app makes the same capabilities usable by humans through a graphical interface.

The app's center of gravity is **managing the ecosystem** — the registry of
providers, models, specialists, and tools, and the web of references between
them. Running a specialist is a secondary convenience, intentionally kept simple
in v1.

**Audience:** technically capable users who are *new to spawningpool*. The UI
favors guided forms, inline explanations of the domain's constraints, and live
validation over terminal-style density.

## Guiding principles

1. **Mirror the CLI's capabilities, not its ergonomics.** Everything `sp` can
   define, list, run, and delete is reachable; the *delight* comes from things a
   terminal does poorly — live reference validation, structurally-prevented
   invalid states, an at-define-time connection check, and a streamed run view.
2. **Make invalid states unreachable, not just rejected.** Where the library's
   `validate()` would error, the UI is shaped so the error can't be produced.
3. **The library owns the logic; the app is a thin shell.** No business rules are
   re-implemented in the frontend that the library already enforces.
4. **The registry is the single source of truth**, shared with the CLI. The app
   stores nothing of its own.

## Architecture

### Integration: link the library (chosen)

A new workspace crate, **`app`**, depends on the `spawningpool` library and
exposes Tauri commands that call it directly:

- `list` / `show` / `define` / `delete` commands → `store::load` / `store::save`,
  plus the `domain` reference and validation functions (`missing_model_ref`,
  `missing_specialist_ref`, `referrers`, `Specialist::validate`).
- `run_specialist` command → drives `run::run_specialist`, forwarding each
  `RunEvent` over a Tauri channel to the webview for live rendering.
- `tools::*` for the tool editor (list, resolve/`summarize`, install, remove).
- `test_connection` for the provider connection check.
- `list_remote_models` for OpenAI-compatible model discovery.

The library is already front-end agnostic — `RunEvent` exists specifically so a
non-CLI front-end can render a run — so this path is both less work and more
capable than the alternative.

**Rejected alternative:** shelling out to the `sp` binary and parsing stdout.
That re-derives structured data the library already returns, loses streaming
fidelity, and discards the structured `MissingRef`/`Referrer` information. Not
pursued.

The CLI and the app become sibling consumers of one core library.

### Stack

- **Tauri v2**, with the `app` crate as the Rust backend.
- **Svelte** frontend — small footprint, simple form handling, minimal ceremony,
  consistent with the project's simplicity ethos.

### Window layout — three panes

```
┌──────────┬───────────────┬──────────────────────────┐
│ Entity   │ Item list     │ Detail / guided editor    │
│ rail     │ (filterable)  │                           │
│          │               │  [Run ▸] on specialists   │
│ Providers│ summarizer    │  ...fields...             │
│ Models   │ netop      ◀  │  Used by: ...referrers... │
│ Spec…  ◀ │ classifier    │                           │
│ Tools    │ router  ⚠     │                           │
│ ─────────│ + New …       │                           │
│ registry │               │                           │
│  path    │               │                           │
└──────────┴───────────────┴──────────────────────────┘
```

- **Left rail:** the four entity types with live counts; footer shows the active
  `registry.json` path.
- **Middle:** filterable list of the selected type; a `⚠` marks items whose
  references don't resolve; a "+ New" action at the bottom.
- **Right:** the detail / guided editor for the selected item, a "Run ▸"
  affordance on specialists, and a "Used by" line driven by `referrers`.

## User flows

### Empty state / onboarding

When the registry is empty, the main pane mirrors the CLI's onboarding ladder: a
**provider → model → specialist → run** progress strip with the current rung
highlighted and a "Define your first provider" call to action. Each rung checks
off as it is satisfied. A fresh install is never a blank dead-end.

### Provider flow

Create / edit form fields: **name · wire protocol (dropdown) · base URL · API key
env var · constrained-decoding toggle**.

- The constrained-decoding toggle is disabled when the protocol is
  `anthropic-messages`, with a tooltip explaining that Anthropic uses native
  forced tool choice and ignores the flag.
- **Live key badge:** reads whether the configured env var is set in the app's
  environment. If not, an amber warning with a copyable `export NAME=…` hint.
  This is the CLI's `unset_key_warnings`, made live.
- **Test connection** button: a cheap call to the endpoint that reports a green
  ✓ or a red ✗ with the error inline — turning a runtime surprise into a
  define-time check. It never stores the secret value; it reads the key from the
  environment for the test only.
- **Delete:** warns about referrers (models defined under the provider,
  specialists pointing at it) before confirming, using `referrers`.

### Model flow

Create / edit:

- **Pick provider first.** For OpenAI-compatible providers, a **Discover** button
  lists the models the live server currently has loaded (`list_models`) so the id
  can be filled in one click. Anthropic providers have no discovery endpoint, so
  the button is not offered for them.
- Fields: **model id · optional display name (defaults to id) · max tokens ·
  context window.**
- Live ✓/⚠ that the chosen provider resolves (`missing_model_ref`).
- **Delete:** warns about specialists referencing the model.

### Specialist flow

The richest editor. Fields: **name · provider · model · system prompt**, plus a
**tool-access mode** and run options.

Two domain constraints from `Specialist::validate()` are made *structurally
impossible* rather than merely rejected:

- **Tool access is a mode toggle, not two independent fields.** The user picks
  exactly one of:
  - **Agentic** — selects any number of tools the model may freely call; the run
    loops until the model stops calling tools.
  - **Forced single tool** — selects one tool the model is forced to call once
    (the `constraint`).

  Because these are mutually-exclusive modes, the "both tools and a constraint"
  error cannot be produced.

- **Reasoning auto-locks to `off` in forced mode**, with the reason shown inline
  ("a forced tool call can't use reasoning"). Switching back to agentic
  re-enables the reasoning selector. This prevents the second `validate()` error
  structurally.

Additional behavior:

- **Forced mode previews how the call will be realized:** if the chosen provider
  declares constrained decoding, it shows "uses constrained decoding"; otherwise
  "forced via `tool_choice`."
- **The stream toggle greys out when constrained decoding applies**, matching the
  runtime (`run.rs` forces a non-streaming turn for constrained decoding).
- **Live references:** provider, model, and every selected/forced tool show ✓/⚠
  as the user edits (`missing_specialist_ref`); a `⚠` tool links directly to
  creating it.
- `validate()` remains the hard gate on save; the live checks make a failed save
  rare.
- **"Run ▸"** opens the run panel pre-targeted at this specialist.
- **Delete:** specialists are referenced by nothing, so no orphan warning.

### Tool flow

A tool is an executable script on disk. The editor supports two entry modes:

- **Register existing** — point the app at an existing executable script; it is
  symlinked into the tools folder (the CLI's behavior), so a script that lives in
  its own repository stays intact and editable in place. Its `# desc:` /
  `# params:` header is parsed for display.
- **Author new** — an in-app editor with fields **name · description · params
  (chips) · script body**. On save, the file is composed with the `#!` shebang
  and the `# desc:` / `# params:` header derived from the fields, written into the
  tools folder, and marked executable. Editing an authored tool reloads these
  fields by parsing the header (`summarize`).

Shared behavior:

- A **live "what the model sees" preview** rendered from `ToolDef::to_tool` (the
  tool name and its required string parameters).
- Inline **valid-name** and **missing-description** warnings (the CLI's existing
  checks).
- **Delete:** warns about specialists referencing the tool (including a forced
  `constraint` tool).

### Run flow (single run, done well)

Deliberately minimal in v1 — running is a convenience, not the app's purpose.

- Opens pre-targeted from a specialist's "Run ▸", or the user picks a specialist
  in the panel. Shows the resolved provider/model and a prompt box.
- **Live transcript** maps the `RunEvent` stream 1:1:
  - streamed assistant text (`TextDelta`) or a complete block (`Text`),
  - each tool call as an expandable block showing its inputs, combined output,
    and exit status (`ToolRan`),
  - tool execution failures in amber (`ToolFailed`),
  - per-turn token usage (`Usage`).
- **Re-run** in one click. A key-unset or unresolved-reference problem is caught
  before the run starts and surfaced in the panel.
- **No run history or persistence in v1.** The transcript is ephemeral.

## Cross-cutting concerns

### Shared registry and live reload

The app and CLI both read and write `~/.spawningpool/registry.json` and the
`tools/` folder; the app persists nothing of its own.

- The app reloads the registry **after every mutation** and **on window focus**,
  and runs a **filesystem watcher** on `registry.json` and `tools/` so changes
  made by the CLI or a second window appear live.
- `store::save` is **last-writer-wins.** For a single-user desktop app this is
  acceptable. To shrink the clobber window, each mutation re-reads, applies, and
  writes within one command rather than acting on state cached in the webview.
  Locking is intentionally out of scope.

### Error handling

The library returns rich, actionable `Result<_, String>` messages. The app
surfaces them **in context, never as raw dialogs**:

- Reference and validation errors attach to the specific field (a missing tool
  highlights that tool's row with a "define it" link).
- `validate()` is the hard gate on save; live checks (`missing_*_ref`, the
  mode/reasoning rules) provide as-you-edit feedback so save rarely fails.
- Run errors and tool failures render inside the run transcript; test-connection
  errors render inline on the provider form.

### Testing strategy

- **Keep logic in the library** (already well-tested) so the Tauri command layer
  stays thin. New commands are unit-tested against a temporary registry via the
  `$SPAWNINGPOOL_REGISTRY` env var — the pattern the CLI tests already use.
- **Frontend component tests** for the genuinely stateful pieces: the specialist
  editor's mode toggle locking reasoning, the live reference badges, and the tool
  editor composing/parsing the header. These encode the "impossible states"
  guarantees.
- A small **smoke test** that a run streams `RunEvent`s end-to-end through the
  command channel. Broad end-to-end UI automation is out of scope for v1.

## Explicitly out of scope for v1

- Side-by-side specialist comparison and persisted run history (the run surface
  is single-run only).
- A visual dependency-graph view (reference-awareness is delivered inline via
  per-field ✓/⚠ badges and "Used by" lists instead).
- Storing secret key values (e.g., OS keychain integration); the app only reads
  the environment to test a connection and never persists a secret.
- Concurrent-write locking on the registry.

## Mapping to the existing library

| UI element | Library surface |
| --- | --- |
| Entity lists, detail | `store::load`, `Registry` fields |
| Save provider/model/specialist | `store::save`, `Specialist::validate` |
| Live reference badges | `missing_model_ref`, `missing_specialist_ref` |
| "Used by" / delete warnings | `referrers`, `Referrer` |
| Model discovery | `Client::list_models` / `list_remote_models` |
| Tool editor preview | `ToolDef::to_tool`, `summarize` |
| Tool register/author/delete | `tools::*`, `prepare_script` |
| Run transcript | `run::run_specialist`, `RunEvent` |
| Onboarding ladder | re-derived from `Registry` emptiness (providers → models → specialists); the same logic the CLI keeps in `onboarding_message`, which is not part of the shared library |
