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
  --tools 'tool,tool2' \      # optional, comma-separated (mutually exclusive with --constraint)
  --constraint 'tool' \       # optional, forces this one tool call (mutually exclusive with --tools)
  --reasoning off|low|medium|high \   # optional, defaults to off
  --stream                    # optional, stream the response

# define a tool from an executable script — its `# desc:` and `# params:`
# header comments become the tool's description and parameters
sp define tool name --script PATH

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

### Tools

A tool is a single executable script. `sp define tool name --script PATH` reads
two header comments to build the tool the model sees:

```sh
#!/bin/sh
# desc: Greet a person by name
# params: NAME, GREETING
echo "$GREETING, $NAME!"
```

`# desc:` becomes the description; `# params:` (whitespace/comma separated)
becomes the parameters. When the tool is called, each argument is passed to the
script as an **environment variable** of the same name — values are never
interpolated into a command line, so there's no shell-injection surface. The
script must be executable and carry a shebang. A non-zero exit is reported as a
failed tool call.

### Constraints and the agent loop

A specialist gets tools one of two ways, and the two are mutually exclusive:

- `--tools <a,b,…>` runs the specialist **agentically**: the model decides which
  tools to call, each call's backing script is executed and its output is fed
  back, and the loop continues until the model stops calling tools (or hits a
  turn cap).
- `--constraint <tool>` forces the model to call that one tool — the core feature
  for a specialist whose sole job is to figure something out and call a tool with
  the result. The forced call runs once, its script executes, and the run ends.

A script's non-zero exit is fed back to the model as a tool error (agentic) or
surfaced (constrained).

There is also an opt-in validator that checks a model's tool-call arguments
against the tool's JSON Schema and feeds violations back to the model to retry.

## Technology choices

1. **Rust:** Catches the most bugs at compile time.
2. **Plain scripts as tools:** a tool is just an executable script with a small `# desc:`/`# params:` header. No external task runner, no YAML, no extra dependency — and the script stays runnable and testable on its own. Inputs arrive as environment variables, keeping execution injection-free.
3. **Clap:** CLI parsing. Necessary for the first version, complete integration testing, and model calling.
4. **Tauri:** Visual app — **not yet implemented.** Planned past version one, once it's ready to be used by humans.

## Inspirations

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
