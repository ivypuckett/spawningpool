# CLI reference

The binary is `spawningpool`. Install it with `cargo install spawningpool-cli`
(see the [Quickstart](README.md#quickstart)).

Every command operates on one JSON registry on disk. A missing registry is
treated as empty, so the first `define` creates it. See
[Configuration](configuration.md) for where it lives.

- [`spawningpool`](#spawningpool-bare) — show the next step
- [`spawningpool define`](#spawningpool-define) — create entities
- [`spawningpool list`](#spawningpool-list) — list names
- [`spawningpool show`](#spawningpool-show) — print one definition
- [`spawningpool delete`](#spawningpool-delete) — remove an entity
- [`spawningpool run`](#spawningpool-run) — run a specialist

---

## `spawningpool` (bare)

State-aware onboarding. Reads the registry and prints exactly which command to
run next in the provider → model → specialist → run progression, plus a warning
for any provider whose API-key env var isn't set.

```sh
spawningpool
```

---

## `spawningpool define`

### provider

A wire protocol (`--api`) + endpoint (`--base-url`) + optional key env var.

```sh
spawningpool define provider <name> \
  --api <anthropic|openai> \
  --base-url <url> \
  [--api-key-env <ENV_VAR>] \
  [--constrained-decoding]
```

`--api` accepts the protocol or its brand alias:
- `anthropic-messages` (alias `anthropic`) — Claude.
- `openai-completions` (alias `openai`) — LM Studio / any OpenAI-compatible endpoint.

`--api-key-env` names the environment variable the key is read from *at run time*
(it stores the variable name, not the key). Omit it for keyless servers like LM Studio.

`--constrained-decoding` declares that this endpoint supports grammar-constrained
`response_format` (true constrained decoding). It's a capability you assert, not
something inferred — two `openai-completions` endpoints can differ. It only
affects how a constrained specialist forces its tool call; see
[`--constraint`](#specialist) below.

```sh
# Hosted Claude
spawningpool define provider anthropic --api anthropic \
  --base-url https://api.anthropic.com --api-key-env ANTHROPIC_API_KEY

# Local LM Studio (keyless)
spawningpool define provider lmstudio --api openai --base-url http://localhost:1234/v1
```

### model

Keyed by its API `id`, defined under a provider, with its token limits. There is
**no built-in catalog** — you define the models you call. A model inherits its
`api`/`base-url` from its provider.

```sh
spawningpool define model <id> \
  --provider <provider> \
  --max-tokens <n> \
  --context-window <n> \
  [--name <display-name>]      # defaults to <id>
```

```sh
spawningpool define model claude-opus-4-8 --provider anthropic \
  --max-tokens 4096 --context-window 200000 --name "Claude Opus 4.8"
```

Defining a model whose `--provider` doesn't exist is rejected with the command
to create it.

### specialist

A system prompt on a (provider, model), with tools it may call.

```sh
spawningpool define specialist <name> \
  --provider <provider> \
  --model <model> \
  --system-prompt '<prompt>' \
  [--tools 'a,b,c']            # comma-separated; the model freely calls these
  [--constraint 'tool']        # OR: force exactly one call to this tool
  [--reasoning off|low|medium|high]   # default off
  [--stream]                   # stream output token-by-token
```

`--tools` and `--constraint` are **mutually exclusive**:

- `--tools` runs the specialist **agentically**: the model decides which tools to
  call, each backing script runs, its output is fed back, and the loop repeats
  until the model stops calling tools (cap: 16 turns).
- `--constraint` **forces** one call to the named tool — for a specialist whose
  sole job is to figure something out and hand it to a tool. The call runs once
  and the run ends. A forced call is incompatible with reasoning, so a
  constrained specialist must keep `--reasoning off` (enforced at define time).

  How the forced call is realized depends on the provider:
  - If the provider was defined `--constrained-decoding`, the call uses true
    grammar-constrained decoding (`response_format` built from the tool's
    parameter schema) — the model's output is guaranteed to match the schema.
  - Otherwise it uses the portable `tool_choice: "required"`, which works across
    OpenAI-compatible servers and Anthropic. (Streaming is disabled for a
    constrained-decoding run, since its single output is the tool arguments.)

```sh
# Agentic: the model picks tools
spawningpool define specialist netop --provider anthropic --model claude-opus-4-8 \
  --system-prompt 'You diagnose network issues.' --tools 'ping,dig' --reasoning low

# Constrained: always classify via one tool
spawningpool define specialist classifier --provider anthropic --model claude-opus-4-8 \
  --system-prompt 'Classify the message, then call classify.' --constraint classify

# Streaming, no tools
spawningpool define specialist writer --provider anthropic --model claude-opus-4-8 \
  --system-prompt 'You write concise release notes.' --stream
```

References to a missing provider, model, or tool are rejected with the exact
command to create the missing piece.

### tool

Defines a tool from an executable script. Its `# desc:` and `# params:` header
comments become the description and parameters the model sees. See
[Writing tools](tools.md) for the script format.

```sh
spawningpool define tool <name> --script <path>
```

```sh
spawningpool define tool ping --script ./scripts/ping.sh
```

The script is checked at define time: it must exist and be executable
(`chmod +x`), failing here with a fix rather than mid-run. A missing `# desc:`
header warns but succeeds. Rather than recording the tool in `registry.json`,
this symlinks the script into the tools folder (`tools/` beside the registry),
which is the source of truth — so you can equally drop an executable script into
that folder by hand, edit one in place (its header is re-read on every run), or
`rm` one. Re-defining a tool replaces whatever backed that name.

---

## `spawningpool list`

Prints names, sorted, one per line.

```sh
spawningpool list providers
spawningpool list models
spawningpool list specialists
spawningpool list tools          # reads the tools folder, not the registry

# Special: query a running LM Studio server for the model ids it has loaded
# (at $LMSTUDIO_BASE_URL, default http://localhost:1234), not the registry.
spawningpool list models --remote
```

---

## `spawningpool show`

Prints one definition as pretty JSON. Errors if it doesn't exist.

```sh
spawningpool show provider anthropic
spawningpool show model claude-opus-4-8
spawningpool show specialist netop
spawningpool show tool ping
```

---

## `spawningpool delete`

Removes one entity. Deleting a provider, model, or tool that specialists still
reference warns about each dangling reference (the delete still happens).
Deleting a tool removes its script from the tools folder.

```sh
spawningpool delete specialist netop
spawningpool delete model claude-opus-4-8
spawningpool delete provider anthropic
spawningpool delete tool ping
```

---

## `spawningpool run`

Instantiates a specialist with a prompt and runs it. Alias: `spawningpool spawn`.

```sh
spawningpool run --specialist <name> --prompt '<prompt>'
```

```sh
spawningpool run --specialist netop --prompt 'Why can I not reach example.com?'
```

Output streams:
- **stdout** — assistant text (streamed live if the specialist was defined
  `--stream`) and each tool's output (`[tool <name>]`).
- **stderr** — token usage (`[usage] N in / N out`) and tool failures.

The API key is sourced from the provider's `--api-key-env` variable at run time;
if it isn't set you'll get an auth error (bare `spawningpool` warns about this in advance).
An agentic specialist that never stops calling tools fails after 16 turns.
