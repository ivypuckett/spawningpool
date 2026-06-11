# spawningpool docs

`spawningpool` creates **hyper-specific specialists**: a saved template of
(provider, model, system prompt, tools) you instantiate with a prompt and run.
Everything you define lives in one JSON registry on disk; the `spawningpool` CLI is the
whole interface.

- **[Quickstart](#quickstart)** — go from nothing to a running specialist below.
- **[CLI reference](cli.md)** — every command and flag, with copyable examples.
- **[Writing tools](tools.md)** — turn a script into a tool a specialist can call.
- **[Configuration](configuration.md)** — registry location, API keys, env vars.

## The model

Four entity kinds, defined in order — each references the previous by name:

```
provider   a wire protocol + endpoint + key  (e.g. Anthropic, or a local LM Studio)
  └─ model       an API id + its token limits, under a provider
       └─ specialist   a system prompt + tools, on a model
tool       an executable script a specialist may call (referenced by specialists)
```

Run bare `spawningpool` at any time — it reads where you are in this progression and prints
the exact next command.

## Quickstart

### 0. Install `spawningpool`

Requires the Rust toolchain (`cargo`); the binary is compiled on install.

```sh
cargo install spawningpool-cli
# Installs the `spawningpool` binary into ~/.cargo/bin — make sure it's on your PATH.
```

Or build from source:

```sh
git clone https://github.com/ivypuckett/spawningpool spawningpool && cd spawningpool
cargo build --release
# The binary is target/release/spawningpool — put it on your PATH.
```

### 1. Define a provider

Hosted Claude:

```sh
spawningpool define provider anthropic --api anthropic \
  --base-url https://api.anthropic.com --api-key-env ANTHROPIC_API_KEY
export ANTHROPIC_API_KEY=sk-ant-...
```

…or a local, OpenAI-compatible LM Studio (no key needed):

```sh
spawningpool define provider lmstudio --api openai --base-url http://localhost:1234/v1
```

### 2. Define a model under it

```sh
spawningpool define model claude-opus-4-8 --provider anthropic \
  --max-tokens 4096 --context-window 200000
```

For LM Studio you can discover what's currently loaded instead of guessing ids:

```sh
spawningpool list models --remote        # prints ids the running server has loaded
```

### 3. Define a specialist

```sh
spawningpool define specialist haiku-namer --provider anthropic --model claude-opus-4-8 \
  --system-prompt 'You suggest one short, memorable name. Reply with only the name.'
```

### 4. Run it

```sh
spawningpool run --specialist haiku-namer --prompt 'A CLI that spawns AI specialists'
```

Assistant text prints to stdout; token usage and any tool failures go to stderr.

That's the whole loop. Add tools next — see **[Writing tools](tools.md)** — or
read the full **[CLI reference](cli.md)**.
