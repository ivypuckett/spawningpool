# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **`run specialist` reads its prompt from a positional argument or stdin.** The
  prompt may now be given positionally (`spawningpool run specialist netop 'why?'`)
  or piped on stdin (`cat issue.txt | spawningpool run specialist triager`), in
  addition to `--prompt`. The positional and `--prompt` forms are mutually
  exclusive; stdin is used only when neither is present.

### Changed

- **`run specialist` output now defaults to the terminal.** With no `--output`,
  it streams `plaintext` when stdout is a terminal and prints the `json` envelope
  when stdout is piped, so interactive runs are readable and scripted runs stay
  parseable. Pass `--output json|plaintext` to force either.

### Fixed

- **README quickstart commands.** The `run --specialist <name>` examples in
  `README.md` and `cli/README.md` were stale; the command is `run specialist
  <name>` (name positional). Corrected.

## [0.3.0] - 2026-06-20

The Workflow DSL release. spawningpool gains a typed orchestration language for
composing tools, specialists, and other workflows — parser, type-checker, and
evaluator — wired into the CLI through restructured `run` subcommands, with
NDJSON run logging, Mermaid diagrams, and an `ask` channel back to a human
mid-run. Also adds Apple Foundation Models support and a batch of TUI fixes.

### Added

- **The Workflow DSL** (`spawningpool::workflow`,
  [docs/workflow-dsl.md](docs/workflow-dsl.md)). A typed orchestration language
  with a tokenizer + recursive-descent `parse`, a static `check` type-checker
  that infers variable types from literals, tool `# output:` declarations, and
  the fixed specialist envelope, and an async `eval` evaluator. Statements run in
  sequence and the last statement's value is the workflow result. Supports
  literals, arithmetic and string `+`, `if`/`for`/`foreach`, object/array access,
  and the run, loop, comparison, and `ask` constructs below.
- **`run <kind> <name>` invocation, unified across the language and CLI.** One
  verb selects the namespace explicitly — `run tool get_weather { CITY: city }`,
  `run specialist reporter ("Summarize: " + weather.summary)`,
  `run workflow deploy { ENV: env }` — so a tool and a workflow may share a file
  name without ambiguity. Workflows compose: a workflow can `run workflow`
  another, inputs flow as typed JSON (not stringified env vars), the result type
  is inferred from the callee's last statement, and cycles are rejected at
  type-check time. The verb/kind keywords accept the CLI aliases (`run`↔`spawn`,
  `workflow`↔`overseer`, `specialist`↔`lenny`/`ling`).
- **CLI `run` subcommands.** `run` is now `run specialist <name> --prompt …`
  (name positional, matching `show`/`delete`), `run workflow <name>` (reads a DSL
  source from a `workflows/` folder beside the registry, parses, type-checks,
  evaluates, and prints the result JSON), and `run tool <name> --arg K=V …` (runs
  a tool script directly and prints its structured output). `spawn` aliases the
  `run` parent.
- **Workflow inputs and tool-style output.** A workflow declares `# inputs:`
  using the same typed-param notation as a tool's `# params:`; values are supplied
  with `run workflow --arg KEY=VALUE`, coerced to the declared type and validated
  before the run. When invoked with `$SP_OUTPUT_PATH` set, the result is written
  there as well as to stdout, so a workflow obeys the same I/O contract as a tool
  and is composable as one.
- **Workflow run logging (NDJSON).** A run emits structured events —
  `workflow.start`/`done`/`error`, `tool.call`/`done`, `specialist.start`/`done`,
  and `ask.prompt`/`answer` — through an injected `LogSink` (mirroring
  `AskHandler`, so the library stays front-end-agnostic). The CLI writes one file
  per invocation, `logs/<datestamp>-<root>-<run>.ndjson`, with an RFC 3339
  timestamp and a per-run id (no date or random dependency added). The `logs/`
  folder lives beside the registry (`~/.spawningpool/logs/` by default, tracking
  `$SPAWNINGPOOL_HOME`/`$SPAWNINGPOOL_REGISTRY`). Format:
  [docs/workflow-logging.md](docs/workflow-logging.md).
- **Mermaid rendering for workflows.** `spawningpool::workflow::mermaid` renders a
  workflow's implicit data flow as a Mermaid `flowchart` — one node per input and
  statement, an edge wherever one statement references another's variable, and
  node shapes per run kind. Exposed as `show workflow <name> --format mermaid`
  (source is the default), filling in the previously missing `Show::Workflow`.
- **Apple Foundation Models support.** macOS 27's `fm serve` exposes the
  on-device Foundation Model as an OpenAI-compatible server, so spawningpool
  drives it through the existing `openai-completions` adapter with no
  Apple-specific code. Setup recipe in
  [docs/configuration.md](docs/configuration.md).
- **Workflows can mix providers.** The evaluator resolves each specialist's API
  key and constrained-decoding flag from its own provider in the registry (the
  CLI passes a provider→key map), instead of stamping one shared key onto every
  call. A workflow whose specialists span providers now runs; `run workflow`
  warns up front about any referenced specialist whose provider API key is unset.
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
- **A `do (body) while (cond) max (n)` loop** (workflow-dsl §6.5.1). Re-runs a
  body expression — no accumulator — until the `while` condition goes `false`.
  The condition decides done-ness and sees the body's latest value bound to the
  assigned variable, so the body just computes itself. The body runs at least
  once; `max` is a required iteration cap (≥ 1) so the loop can't spin forever.
  The loop's value is the body's final value. Because the condition refers to the
  assigned variable, `do` is only valid as a statement's whole right-hand side.
- **Equality and comparison operators** (workflow-dsl §6.3). `==`/`!=` compare
  any two operands of the same type (numbers by value, arrays and objects
  structurally) and `<`/`<=`/`>`/`>=` order two numbers or two strings; all
  yield `bool`. These are the first operators to produce a `bool` from non-`bool`
  operands, so `if`/`while` conditions can now test computed values rather than
  only pre-existing booleans.
- **An `ask <prompt> [else <fallback>]` expression** (workflow-dsl §6.8,
  [docs/ask.md](docs/ask.md)). Pauses the run, puts a question to the human
  operating the workflow, and resolves to their reply as a `string` — the one
  point where control returns to a person mid-run. It's a built-in keyword (no
  named on-disk entity to resolve) and exists only in workflows. An answer
  (including the empty string for a bare enter) is in-band data to branch on with
  `if`; when the question can't be answered (a headless run with no front-end, or
  the user cancelling), the optional `else` supplies a single fallback string, and
  without one the workflow aborts. The CLI prompts on stderr and reads stdin when
  interactive, treating a run with `$SP_OUTPUT_PATH` set or a non-TTY stdin as
  headless.
- **`delete` confirmation.** The CLI `delete` previews the references it would
  orphan before removing anything and prompts for confirmation; `--yes`/`-y`
  skips the prompt. Existence is checked without mutating the registry, so a
  declined or absent delete leaves it untouched.

### Changed

- **`run` is now subcommand-based.** *(Breaking.)* `run --specialist <name>
  --prompt …` becomes `run specialist <name> --prompt …`. Scripts that called
  `run --specialist` must move the name to a positional argument under the
  `specialist` subcommand.

### Fixed

- **TUI: the terminal is restored on a panic.** A panic unwinding out of the
  event loop skipped `teardown()`, leaving the shell in raw mode on the alternate
  screen with the backtrace stair-stepping across it. A wrapped panic hook now
  runs `teardown()` before the default handler prints.
- **TUI: a failed tool call reports its exit status.** `ScriptRun` carries the
  exit `code` (or `None` when signalled), and `run_tool` reports "exited with
  status N — see its output above" (or "(no output)") instead of a bare "exited
  non-zero".
- **TUI: a failed rename no longer diverges from disk.** Rename reloads from the
  file when `persist()` fails, so in-memory state matches what was saved.
- **OpenAI adapter robustness.** `stream` is always serialized (even when
  `false`), since `fm serve` defaults to streaming when the field is absent,
  which had broken the non-streaming `complete` path. Streaming is now suppressed
  only when constrained decoding is actually active (a forced tool call present),
  not merely because the provider declares the capability — previously any
  specialist on a `--constrained-decoding` provider silently lost streaming.
- The workflow evaluator's `+` arm uses `num_val()` like its sibling operators
  instead of an `unwrap()`, and a `cargo doc` intra-doc link warning in
  `domain.rs` is resolved.

### Documentation

- The **Workflow DSL v1 spec** ([docs/workflow-dsl.md](docs/workflow-dsl.md)) and
  the orchestration stance it reconciles.
- **[docs/data-flow.md](docs/data-flow.md)** — how input/output crosses every
  boundary (specialist loop, tool channels, the three run kinds), as contract
  cards with an overview diagram.
- **[docs/channels.md](docs/channels.md)** — names the three information channels
  (data, ask, log) and how they relate, differ, and fail, cross-linked from each
  deep-dive doc.
- **[docs/workflow-logging.md](docs/workflow-logging.md)** — the NDJSON event
  format for workflow observability.
- A **showcase workflow** exercising every DSL feature, the `# exits:` directive
  in [docs/tools.md](docs/tools.md), the `fm serve` recipe in
  [docs/configuration.md](docs/configuration.md), the `delete` confirmation /
  `--yes` and the `tui` command and keys in [docs/cli.md](docs/cli.md), and a
  note on the registry's single-writer assumption.

### Internal

- Inline `#[cfg(test)]` modules extracted into sibling `*_tests.rs` files via
  `#[path]`, and the `openai`/`anthropic` providers, the workflow lexer, and
  `cli/src/main.rs` split into submodules — so reading a production module no
  longer pulls in its tests or unrelated code. No behavior change.
- End-to-end tests for the specialist run loop (agentic path, the `MAX_TURNS`
  cap, and a constrained single-call run) against a local mock server.

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

[0.3.0]: https://github.com/ivypuckett/spawningpool/releases/tag/v0.3.0
[0.2.0]: https://github.com/ivypuckett/spawningpool/releases/tag/v0.2.0
[0.1.0]: https://github.com/ivypuckett/spawningpool/releases/tag/v0.1.0
