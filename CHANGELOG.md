# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.1] - 2026-06-11

First public release on crates.io. spawningpool lets you create hyper-specific,
0-waste agents — "specialists" with minimal system prompts that do one thing
well — and call them from the CLI or manage them in an interactive terminal UI.

Published as two crates: the `spawningpool` library and the `spawningpool-cli`
binary (installed as `spawningpool`).

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

### Changed

- The CLI is presented as `spawningpool` throughout (docs and help), and both
  crates use the singular `agent` keyword for crates.io.
- Per-crate READMEs and crates.io metadata added in preparation for publishing.

### Removed

- The Tauri desktop app was removed; the interactive experience is now the
  built-in `spawningpool tui`.

[0.1.1]: https://github.com/ivypuckett/spawningpool/releases/tag/v0.1.1
