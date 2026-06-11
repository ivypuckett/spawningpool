# spawningpool-cli

[![crates.io](https://img.shields.io/crates/v/spawningpool-cli.svg)](https://crates.io/crates/spawningpool-cli)
[![license](https://img.shields.io/crates/l/spawningpool-cli.svg)](https://github.com/ivypuckett/spawningpool/blob/main/LICENSE)

`sp` — create hyper-specific, 0-waste agents from the command line.

This crate installs the `spawningpool` binary (CLI name: `sp`), the whole
interface to [spawningpool](https://crates.io/crates/spawningpool). A
**specialist** is a saved template of `(provider, model, system prompt, tools)`
you instantiate with a prompt and run. Everything you define lives in one JSON
registry on disk.

## Install

```sh
cargo install spawningpool-cli
```

This installs a `spawningpool` binary into `~/.cargo/bin`. Its CLI name is `sp`,
so alias it:

```sh
alias sp="spawningpool"
```

## The model

Four entity kinds, defined in order — each references the previous by name:

```text
provider   a wire protocol + endpoint + key  (e.g. Anthropic, or a local LM Studio)
  └─ model       an API id + its token limits, under a provider
       └─ specialist   a system prompt + tools, on a model
tool       an executable script a specialist may call (referenced by specialists)
```

Run bare `sp` at any time — it reads where you are in this progression and prints
the exact next command.

## Quickstart

```sh
# 1. Define a provider (hosted Claude)
sp define provider anthropic --api anthropic \
  --base-url https://api.anthropic.com --api-key-env ANTHROPIC_API_KEY
export ANTHROPIC_API_KEY=sk-ant-...

# 2. Define a model under it
sp define model claude-opus-4-8 --provider anthropic \
  --max-tokens 4096 --context-window 200000

# 3. Define a specialist
sp define specialist haiku-namer --provider anthropic --model claude-opus-4-8 \
  --system-prompt 'You suggest one short, memorable name. Reply with only the name.'

# 4. Run it
sp run --specialist haiku-namer --prompt 'A CLI that spawns AI specialists'
```

Assistant text prints to stdout; token usage and any tool failures go to stderr.

Browse and manage everything in an interactive terminal UI with `sp tui`.

## Documentation

Full guides — quickstart, CLI reference, writing tools, and configuration — live
in [the `docs/` folder](https://github.com/ivypuckett/spawningpool/tree/main/docs).

## License

Licensed under the [MIT license](https://github.com/ivypuckett/spawningpool/blob/main/LICENSE).
