# Spawning Pool

Goal: create hyper-specific specialists with minimal system prompts that do one thing and one thing well.

## Requirements

- Creating/updating specialists needs to be trivial.
- Need to be able to give specialists tools easily.
- Need to be able to evaluate specialists against each other quickly.
- Needs to be written such that a specialist *could* define and refine other specialists dynamically.

## Terms

- **Tool:** deterministic, callable thing which a model can utilize.
- **System prompt:** standard definition.
- **Specialist:** the template of model provider, model, system prompt, and tools which can be instantiated and called with a user prompt.

## CLI

The binary is named `spawningpool`; its CLI name is `sp`. Definitions are
persisted to a single JSON registry at `$SPAWNINGPOOL_HOME/registry.json`
(default `~/.spawningpool/registry.json`); set `$SPAWNINGPOOL_REGISTRY` to
override the exact path. A missing file loads as an empty registry, so the
first `define` creates it.

```bash
# run (alias: spawn)
sp run --specialist name --prompt 'prompt'

# define a provider — a wire protocol (--api) + endpoint (--base-url) + key env var
sp define provider name \
  --api anthropic|openai \
  --base-url URL \
  --api-key-env ENV          # optional

# define a model — keyed by its API id, against a provider, with its limits
sp define model id \
  --provider provider \
  --max-tokens N \
  --context-window N \
  --name NAME                # optional; defaults to the id

# define a specialist
sp define specialist name \
  --provider provider \
  --model model \
  --system-prompt 'prompt' \
  --tools 'tool,tool2' \      # optional, comma-separated
  --constraint 'tool' \       # optional, forces this tool call
  --reasoning off|low|medium|high \   # optional, defaults to off
  --stream                    # optional, stream the response

# define a tool from a Taskfile task — its desc and {{.VARS}} become
# the tool's description and parameters
sp define tool name --taskfile PATH --task TASK

# delete
sp delete specialist name
sp delete provider name
sp delete model name
sp delete tool name

# list
sp list specialists
sp list providers
sp list models
sp list tools
```

### Providers

Two wire protocols are implemented, selected at runtime from a model's `api`:

- `anthropic` (`anthropic-messages`) — Claude.
- `openai` (`openai-completions`) — LM Studio and any OpenAI-compatible endpoint.

spawningpool deliberately does **not** embed a catalog of hosted models or their
limits — those facts go stale and being their arbiter is a liability. Models you
call are defined in your own registry via `sp define model`.

### Constrained decoding

A specialist's `--constraint <tool>` forces the model to call that tool — the
core feature for a specialist whose sole job is to figure something out and call
a tool with the result. There is also an opt-in validator that checks a model's
tool-call arguments against the tool's JSON Schema and feeds violations back to
the model to retry.

## Technology choices

1. **Rust:** Catches the most bugs at compile time.
2. **Taskfiles:** tools are defined from Taskfile tasks, so we don't reinvent the wheel for tool definitions. Deterministic parsing of a task's `{{.VARS}}` gives us a tool's parameters for free.
3. **Clap:** CLI parsing. Necessary for the first version, complete integration testing, and model calling.
4. **Tauri:** Visual app — **not yet implemented.** Planned past version one, once it's ready to be used by humans.

## Inspirations

- **task-keeper:** already parses Taskfiles; inspired our own Taskfile parsing (hand-rolled here on `serde_yaml` + `regex`).
- **pi SDK:** an excellent SDK we drew from, but not used as a dependency — it does not support constrained decoding, which is a core feature here.

## Phases

1. Install the above technologies into a hello-world project.
2. `sp list providers`, `models`
3. `sp define provider`
4. `sp list specialists`
5. `sp define model`
6. `sp define specialist`
7. `sp run specialist`
8. `sp list tools`
9. `sp define tool`
10. `sp delete *`

## Simplifications

- A chain is a sequence/selection/iteration of specialists. Bash and scripting allows for this and provides an existing abstraction/entry point. The project does not prioritize shared memory. Therefore, bash is sufficient. If better tooling is desired, LangGraph and Mastra have both invested in this space and could be used alongside it.

## Build & test

```sh
cargo build      # build the workspace
cargo test       # run all tests
cargo clippy     # lint
cargo fmt        # format
```

The binary is built to `target/debug/spawningpool` (or `target/release/spawningpool`).
See `CLAUDE.md` for contributor rules and git-hook setup.
