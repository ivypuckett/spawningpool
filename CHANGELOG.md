# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

Foundations for the [Workflow DSL](docs/workflow-dsl.md): the typed tool headers
and structured-tool-output plumbing the DSL builds on. The orchestration language
itself (parser, type-checker, evaluator) is not included yet.

### Added

- **A type system for tool headers** (`spawningpool::types`). The notation
  `string`/`number`/`bool`/`[T]`/`{ "k": T, ... }` parses into a `Type` and
  lowers to JSON Schema, reusing the existing tool-call validator and schema
  builder rather than duplicating them.
- **Typed `# params:` and a new `# output:` header directive.** A param may carry
  an optional `:type` suffix (a bare param still means `string`, so existing
  headers are unchanged); `# output:` declares the type of a tool's structured
  result. `ToolDef`/`ScriptSummary` carry these, and `ToolDef::to_tool` lowers
  each param to its declared type.
- **Structured tool output via `$SP_OUTPUT_PATH`.** Before running a tool the
  runner sets `SP_OUTPUT_PATH` to a fresh temp file; a tool's JSON written there
  is read back as `ScriptRun::structured_output`. stdout/stderr remain ordinary
  logs and are not parsed.
- **A `# exits:` header directive declaring exit codes and their meanings.** Each
  entry is `<code> <name>` with an optional quoted description; the `<name>` is a
  workflow identifier so a later stage can branch on it. `ScriptSummary`/`ToolDef`
  carry the parsed `ExitCode`s, and `ToolDef::to_tool` appends them to the
  description the model reads so an agentic specialist can tell why a call failed.
- **An `else` recovery block on `run tool`** (workflow-dsl §6.6/§7). A non-zero
  exit aborts the workflow by default; an `else { name: expr, ..., _: expr }`
  block instead recovers it into a value, keyed by the tool's `# exits:` name
  (with `_` as the default). Each arm must produce the tool's `# output:` type,
  and the block must cover every declared non-zero exit or supply `_`.

## [0.2.0] - 2026-06-12

First public release on crates.io, published as two crates: the `spawningpool`
library and the `spawningpool-cli` binary (installed as `spawningpool`). On top
of packaging the 0.1.0 feature set, this release reworks `run`'s output into a
structured, machine-readable envelope by default and tightens up the TUI.

### Added

- **`run --output <format>`.** `run` now emits a structured envelope. The
  default is `json`; pass `--output plaintext` for the old live terminal
  rendering.
- **Expanded JSON envelope.** The `json` output is a single object with nine
  fields: `output`, `thinking`, `inputTokens`, `outputTokens`, `stopReason`,
  `model`, `specialist`, `turns`, and `toolCalls` (each tool call records its
  `name`, `success`, and `output`).
- **Library run events for reasoning and stop reasons.** `RunEvent` gained
  `ThinkingDelta`, `Thinking`, and `TurnDone { stop_reason }` variants, so
  callers can observe a model's reasoning content and per-turn stop reasons.
- Per-crate READMEs and crates.io metadata, and packaging for the
  `cargo install` path.

### Changed

- **`run` defaults to JSON output.** *(Breaking.)* Previously `run` streamed
  plain text to the terminal by default and `--output json` was opt-in. JSON is
  now the default; callers who relied on streamed terminal output must pass
  `--output plaintext`.
- **TUI: `o` on a provider drills into its models.** A provider's `base_url` is
  an API endpoint, not a web page, so pressing `o` on one now drills into its
  models (matching Enter/→) instead of trying to open the URL in a browser. The
  browser-opening path is gone, which also removes a class of hangs and crashes
  on systems whose URL opener misbehaves against a dead endpoint.
- The CLI is presented as `spawningpool` throughout (docs and help) rather than
  `sp`.
- Both crates use the singular `agent` keyword for crates.io.

### Fixed

- TUI: editing a registry entity inside a multiplexer (Zellij/tmux/Kitty) no
  longer opens a blank editor.
- TUI: the cursor is no longer lost when popping back out of a drilled provider,
  or when adding an entity while the view is filtered.

### Documentation

- Documented the `run --output` flag and corrected a default-output mismatch in
  the docs.
- Documented the tool-call trick and corrected the Anthropic
  constrained-decoding claim.

### Removed

- The provider-console browser action (and its supporting `OpenProvider`
  action, `open_provider`/`open_url_command`/`provider_base_url` helpers, and
  the detached-spawn path) — superseded by drilling into a provider's models.

## [0.1.0] - 2026-06-10

Initial feature-complete build. spawningpool lets you create hyper-specific,
0-waste agents — "specialists" with minimal system prompts that do one thing
well — and call them from the CLI or manage them in an interactive terminal UI.

### Added

- **Specialists.** Define a specialist as a template of provider, model, system
  prompt, and tools, then instantiate and run it against a user prompt with
  `spawningpool run --specialist <name> --prompt '…'` (alias: `spawn`).
- **Persisted registry.** Providers, models, and specialists live in a single
  JSON registry at `$SPAWNINGPOOL_HOME/registry.json` (default
  `~/.spawningpool/registry.json`; override the exact path with
  `$SPAWNINGPOOL_REGISTRY`). A missing file loads as an empty registry, so the
  first `define` creates it.
- **CLI verbs** over the registry: `define`, `list`, `show`, and `delete` for
  providers, models, specialists, and tools, plus `run`.
- **Multi-provider LLM support.** Two wire protocols selected at runtime from a
  model's `api`: `anthropic` (Claude) and `openai` (LM Studio and any
  OpenAI-compatible endpoint). spawningpool deliberately ships no embedded
  catalog of hosted models or their limits — you define the models you call in
  your own registry.
- **Tools as plain executable scripts.** A tool is a single executable script in
  the `tools/` folder beside the registry; its `# desc:` and `# params:` header
  comments become the tool's description and parameters. Arguments are passed to
  the script as environment variables (never interpolated into a command line),
  so there is no shell-injection surface. `spawningpool define tool <name>
  --script PATH` symlinks one in.
- **Agentic and constrained execution.** A specialist gets tools one of two
  mutually exclusive ways: `--tools <a,b,…>` runs it agentically (the model
  picks tools, each backing script runs, output is fed back, loop continues
  until the model stops or hits a turn cap), or `--constraint <tool>` forces a
  single tool call and ends the run.
- **Constrained decoding.** A provider declared with `--constrained-decoding`
  uses true grammar-constrained decoding (via `response_format` built from the
  tool's parameter schema) so output is guaranteed schema-valid; otherwise a
  forced call uses the portable `tool_choice: "required"`. Anthropic uses its
  native forced tool choice either way.
- **Define-time validation** with actionable errors: references are validated
  when you define, and incompatible combinations (e.g. `--constraint` with
  reasoning, or `--tools` with `--constraint`) are rejected up front.
- **Opt-in tool-call validator** that checks a model's tool-call arguments
  against the tool's JSON Schema and feeds violations back to the model to retry.
- **Reasoning and streaming** controls per specialist: `--reasoning
  off|low|medium|high` and `--stream`.
- **Remote model discovery.** `spawningpool list models --remote` lists the
  models a running LM Studio server currently has loaded (at
  `$LMSTUDIO_BASE_URL`, default `http://localhost:1234`).
- **State-aware onboarding** on bare `spawningpool`, guiding you from an empty
  registry to a first run.
- **Interactive terminal UI.** `spawningpool tui` opens a Ratatui interface over
  the same registry — vim- and mouse-navigable tabs for Providers, Specialists,
  and Tools, with add/edit/rename/delete, in-place `$EDITOR` editing (opening in
  a new pane under Zellij, tmux, or Kitty), search, and chat/run actions.
- **`lenny` and `ling` aliases** for the `specialist` subcommand.

### Removed

- The Tauri desktop app was removed; the interactive experience is the built-in
  `spawningpool tui`.

[0.2.0]: https://github.com/ivypuckett/spawningpool/releases/tag/v0.2.0
[0.1.0]: https://github.com/ivypuckett/spawningpool/releases/tag/v0.1.0
